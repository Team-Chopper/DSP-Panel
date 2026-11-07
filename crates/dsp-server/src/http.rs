//! HTTP(S) server.
//!
//! ```text
//! GET  /api/session  {authEnabled, authenticated, passwordSet}
//! POST /api/login    {password} → sets the session cookie
//! POST /api/logout   clears the session cookie
//! GET  /ws           WebSocket (see hub.rs)
//! GET  /limit        limit light page, no login, only when switched on (light.rs)
//! GET  /limit/events {on, ready} as server-sent events
//! GET  /*            web client (static files)
//! ```

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;

use crate::auth::{AuthService, COOKIE_NAME, SESSION_LIFETIME};
use crate::config::ServerConfig;
use crate::hub::{HubCommand, MAX_MESSAGE_LENGTH};
use crate::light::{self, LightSnapshot};
use crate::log;

/// Interval of the WebSocket ping; a client that does not answer before the
/// next ping is disconnected.
pub const PING_INTERVAL: Duration = Duration::from_secs(20);

pub struct AppState {
    pub config: ServerConfig,
    pub auth: Mutex<AuthService>,
    pub hub: mpsc::UnboundedSender<HubCommand>,
    /// State of the limit light, published by the hub.
    pub light: watch::Receiver<LightSnapshot>,
    next_client: AtomicU64,
}

impl AppState {
    pub fn new(
        config: ServerConfig,
        auth: AuthService,
        hub: mpsc::UnboundedSender<HubCommand>,
        light: watch::Receiver<LightSnapshot>,
    ) -> Arc<Self> {
        Arc::new(AppState { config, auth: Mutex::new(auth), hub, light, next_client: AtomicU64::new(1) })
    }

    fn auth(&self) -> MutexGuard<'_, AuthService> {
        self.auth.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn is_authenticated(&self, headers: &HeaderMap) -> bool {
        let auth = self.auth();
        !auth.enabled || auth.verify_token(cookie(headers, COOKIE_NAME).as_deref())
    }

    fn session_cookie(&self, value: &str, max_age: u64) -> String {
        format!(
            "{COOKIE_NAME}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Strict{}",
            if self.config.tls_enabled { "; Secure" } else { "" }
        )
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/api/session", get(session))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .fallback_service(static_files(&state.config))
        .layer(CompressionLayer::new());
    // The limit light stream must not go through the compression layer.
    Router::new()
        .route("/ws", get(web_socket))
        .route("/limit", get(light::page))
        .route("/limit/events", get(light::events))
        .merge(api)
        .layer(axum::middleware::map_response(security_headers))
        .with_state(state)
}

/// The web client from the web root, or the one built into the server.
fn static_files(config: &ServerConfig) -> Router {
    let root = config.web_root.clone();
    if root.is_dir() {
        return Router::new().fallback_service(ServeDir::new(root).append_index_html_on_directories(true));
    }
    log::warn(format!("Web root {} not found – only the API is served", root.display()));
    let text = format!("Web client not installed ({})", root.display());
    Router::new().fallback(move || {
        let text = text.clone();
        async move { (StatusCode::NOT_FOUND, text) }
    })
}

async fn security_headers(mut response: Response) -> Response {
    let headers = response.headers_mut();
    let set = |headers: &mut HeaderMap, name: &'static str, value: &'static str| {
        headers.insert(name, HeaderValue::from_static(value));
    };
    set(headers, "x-content-type-options", "nosniff");
    set(headers, "referrer-policy", "no-referrer");
    set(headers, "content-security-policy", "frame-ancestors 'none'");
    set(headers, "x-frame-options", "DENY");
    // Cross-origin isolation: everything is served from this server, and
    // SharedArrayBuffer (audio worklets, WebAssembly threads) needs it.
    set(headers, "cross-origin-opener-policy", "same-origin");
    set(headers, "cross-origin-embedder-policy", "require-corp");
    // Always revalidate so a server update is picked up immediately.
    if !headers.contains_key(header::CACHE_CONTROL) {
        set(headers, "cache-control", "no-cache");
    }
    response
}

fn json_response(status: StatusCode, body: Value, cookie: Option<String>) -> Response {
    let mut response = (status, body.to_string()).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Some(cookie) = cookie.and_then(|c| HeaderValue::from_str(&c).ok()) {
        headers.insert(header::SET_COOKIE, cookie);
    }
    response
}

async fn session(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let authenticated = state.is_authenticated(&headers);
    let auth = state.auth();
    let body = json!({"authEnabled": auth.enabled, "authenticated": authenticated, "passwordSet": auth.has_password()});
    json_response(StatusCode::OK, body, None)
}

async fn login(State(state): State<Arc<AppState>>, ConnectInfo(remote): ConnectInfo<SocketAddr>, body: Body) -> Response {
    let client = remote.ip().to_string();
    {
        let mut auth = state.auth();
        if !auth.enabled {
            return json_response(StatusCode::OK, json!({"ok": true}), None);
        }
        if !auth.allow_attempt(&client) {
            let body = json!({"error": "Too many failed attempts – please wait a minute."});
            return json_response(StatusCode::TOO_MANY_REQUESTS, body, None);
        }
        if !auth.has_password() {
            let body = json!({"error": "No password has been set yet (run: dsp-server passwd)."});
            return json_response(StatusCode::SERVICE_UNAVAILABLE, body, None);
        }
    }

    let bytes = tokio::time::timeout(Duration::from_secs(5), axum::body::to_bytes(body, 4096)).await;
    let password = match bytes {
        Ok(Ok(bytes)) => serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|v| v.get("password").and_then(Value::as_str).map(String::from)),
        _ => None,
    };
    let Some(password) = password else {
        return json_response(StatusCode::BAD_REQUEST, json!({"error": "Password missing"}), None);
    };

    // PBKDF2 takes a while: keep it off the runtime thread.
    let check_state = state.clone();
    let ok = tokio::task::spawn_blocking(move || check_state.auth().verify_password(&password)).await.unwrap_or(false);
    let mut auth = state.auth();
    if !ok {
        auth.record_failure(&client);
        log::warn(format!("Failed login from {client}"));
        return json_response(StatusCode::UNAUTHORIZED, json!({"error": "Wrong password"}), None);
    }
    log::info(format!("Login from {client}"));
    let cookie = state.session_cookie(&auth.issue_token(), SESSION_LIFETIME.as_secs());
    json_response(StatusCode::OK, json!({"ok": true}), Some(cookie))
}

async fn logout(State(state): State<Arc<AppState>>) -> Response {
    json_response(StatusCode::OK, json!({"ok": true}), Some(state.session_cookie("", 0)))
}

async fn web_socket(
    State(state): State<Arc<AppState>>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    if !is_same_origin(&headers) {
        let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()).unwrap_or_default();
        log::warn(format!("Rejected WebSocket from foreign origin {origin}"));
        return (StatusCode::FORBIDDEN, "Origin not allowed").into_response();
    }
    if !state.is_authenticated(&headers) {
        return (StatusCode::UNAUTHORIZED, "Not logged in").into_response();
    }
    let id = state.next_client.fetch_add(1, Ordering::Relaxed);
    let hub = state.hub.clone();
    let remote = remote.ip().to_string();
    upgrade
        .max_message_size(4 * MAX_MESSAGE_LENGTH)
        .on_upgrade(move |socket| client_loop(socket, hub, id, remote))
}

async fn client_loop(socket: WebSocket, hub: mpsc::UnboundedSender<HubCommand>, id: u64, remote: String) {
    let (mut sink, mut stream) = socket.split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    if hub.send(HubCommand::Connected { id, remote: remote.clone(), out: out_tx }).is_err() {
        return;
    }
    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + PING_INTERVAL, PING_INTERVAL);
    let mut pong_pending = false;
    loop {
        tokio::select! {
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    let _ = hub.send(HubCommand::Text { id, text: text.to_string() });
                }
                Some(Ok(Message::Binary(_))) => {
                    let _ = hub.send(HubCommand::Binary { id });
                }
                Some(Ok(Message::Pong(_))) => pong_pending = false,
                Some(Ok(Message::Ping(_))) => {}
                Some(Ok(Message::Close(_))) | None => break,
                Some(Err(e)) => {
                    log::warn(format!("WebSocket error ({remote}): {e}"));
                    break;
                }
            },
            outgoing = out_rx.recv() => match outgoing {
                Some(text) => {
                    if sink.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                // The hub dropped the client (shutdown).
                None => {
                    let _ = sink.send(Message::Close(None)).await;
                    break;
                }
            },
            _ = ping.tick() => {
                if pong_pending {
                    break; // no answer to the last ping
                }
                pong_pending = true;
                if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
        }
    }
    let _ = hub.send(HubCommand::Disconnected { id });
}

/// Browsers always send Origin on WebSocket requests. It must match the Host
/// we were addressed with, otherwise another website could drive the panel
/// with the user's cookie (cross-site WebSocket hijacking).
fn is_same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true; // non-browser client
    };
    let (Ok(origin), Some(Ok(host))) = (origin.to_str(), headers.get(header::HOST).map(|h| h.to_str())) else {
        return false;
    };
    let Some(rest) = origin.split_once("://").map(|(_, rest)| rest) else { return false };
    let authority = rest.split('/').next().unwrap_or_default();
    fn normalize(h: &str) -> String {
        let h = h.to_lowercase();
        for port in [":443", ":80"] {
            if let Some(stripped) = h.strip_suffix(port) {
                return stripped.to_string();
            }
        }
        h
    }
    !authority.is_empty() && normalize(authority) == normalize(host)
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let header = headers.get(header::COOKIE)?.to_str().ok()?;
    header.split(';').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        (key.trim() == name).then(|| value.trim().to_string())
    })
}
