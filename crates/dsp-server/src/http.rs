//! HTTP(S) server.
//!
//! ```text
//! GET  /api/status   {version, devices: [{name, kind, state, detail?}]} for a monitoring hub
//! GET  /ws           WebSocket (see hub.rs)
//! GET  /limit        limit light page, only when switched on (light.rs)
//! GET  /limit/events {on, ready} as server-sent events
//! GET  /*            web client (static files)
//! ```
//!
//! There is no login: everyone who can reach the server controls the DSPs.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, watch};
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;

use crate::config::ServerConfig;
use crate::hub::{HubCommand, MAX_MESSAGE_LENGTH};
use crate::light::{self, LightSnapshot};
use crate::log;

/// Interval of the WebSocket ping; a client that does not answer before the
/// next ping is disconnected.
pub const PING_INTERVAL: Duration = Duration::from_secs(20);

pub struct AppState {
    pub config: ServerConfig,
    pub hub: mpsc::UnboundedSender<HubCommand>,
    /// State of the limit light, published by the hub.
    pub light: watch::Receiver<LightSnapshot>,
    next_client: AtomicU64,
}

impl AppState {
    pub fn new(
        config: ServerConfig,
        hub: mpsc::UnboundedSender<HubCommand>,
        light: watch::Receiver<LightSnapshot>,
    ) -> Arc<Self> {
        Arc::new(AppState { config, hub, light, next_client: AtomicU64::new(1) })
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    let files = Router::new().fallback_service(static_files(&state.config)).layer(CompressionLayer::new());
    // The limit light stream must not go through the compression layer.
    Router::new()
        .route("/ws", get(web_socket))
        .route("/api/status", get(status))
        .route("/limit", get(light::page))
        .route("/limit/events", get(light::events))
        .merge(files)
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

async fn status(State(state): State<Arc<AppState>>) -> Response {
    let (reply, answer) = tokio::sync::oneshot::channel();
    if state.hub.send(HubCommand::Status { reply }).is_err() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    match tokio::time::timeout(Duration::from_secs(1), answer).await {
        Ok(Ok(body)) => axum::Json(body).into_response(),
        _ => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
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
/// we were addressed with, otherwise any website open in a browser on the
/// network could drive the panel (cross-site WebSocket hijacking).
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
