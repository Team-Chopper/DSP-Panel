//! Login state and the WebSocket to the panel server. The web client is always
//! served by the panel server itself, so all URLs are relative to the page's
//! own origin.

use std::cell::RefCell;
use std::time::Duration;

use gloo_net::http::Request;
use leptos::prelude::*;
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{MessageEvent, WebSocket};

use crate::store::{ServerState, app};

thread_local! {
    static SOCKET: RefCell<Option<WebSocket>> = const { RefCell::new(None) };
    static RETRY: RefCell<(u32, Option<TimeoutHandle>)> = const { RefCell::new((0, None)) };
}

/// Send a message; silently dropped while disconnected.
pub fn send(message: Value) {
    if app().server.get_untracked() != ServerState::Connected {
        return;
    }
    SOCKET.with(|s| {
        if let Some(ws) = s.borrow().as_ref() {
            let _ = ws.send_with_str(&message.to_string());
        }
    });
}

pub fn start() {
    leptos::task::spawn_local(check_session());
}

async fn check_session() {
    let app = app();
    app.server.set(ServerState::Checking);
    let response = Request::get("/api/session").send().await;
    let json = match response {
        Ok(r) => r.json::<Value>().await.ok(),
        Err(_) => None,
    };
    let Some(json) = json else {
        app.server_error.set(Some("Server not reachable".into()));
        schedule_retry();
        return;
    };
    let auth = json.get("authEnabled").and_then(Value::as_bool).unwrap_or(false);
    app.auth_enabled.set(auth);
    app.password_set.set(json.get("passwordSet").and_then(Value::as_bool).unwrap_or(true));
    if auth && json.get("authenticated") != Some(&Value::Bool(true)) {
        app.server_error.set(None);
        app.server.set(ServerState::LoginRequired);
    } else {
        connect();
    }
}

/// Log in with the panel password. Returns an error message.
pub async fn login(password: String) -> Option<String> {
    let request = Request::post("/api/login").header("content-type", "application/json");
    let response = match request.body(json!({"password": password}).to_string()) {
        Ok(r) => r.send().await,
        Err(_) => return Some("Server not reachable".into()),
    };
    let Ok(response) = response else { return Some("Server not reachable".into()) };
    if response.status() != 200 {
        let json = response.json::<Value>().await.ok();
        return Some(
            json.and_then(|j| j.get("error").and_then(Value::as_str).map(String::from))
                .unwrap_or_else(|| "Login failed".into()),
        );
    }
    connect();
    None
}

pub async fn logout() {
    cancel_retry();
    let _ = Request::post("/api/logout").send().await;
    close_socket();
    app().server.set(ServerState::LoginRequired);
}

fn socket_url() -> String {
    let location = web_sys::window().map(|w| w.location());
    let protocol = location.as_ref().and_then(|l| l.protocol().ok()).unwrap_or_default();
    let host = location.as_ref().and_then(|l| l.host().ok()).unwrap_or_default();
    format!("{}//{host}/ws", if protocol == "https:" { "wss:" } else { "ws:" })
}

fn close_socket() {
    SOCKET.with(|s| {
        if let Some(ws) = s.borrow_mut().take() {
            ws.set_onopen(None);
            ws.set_onmessage(None);
            ws.set_onclose(None);
            ws.set_onerror(None);
            let _ = ws.close();
        }
    });
}

fn is_current(ws: &WebSocket) -> bool {
    SOCKET.with(|s| s.borrow().as_ref().is_some_and(|current| current == ws))
}

fn connect() {
    close_socket();
    app().server.set(ServerState::Connecting);
    let Ok(ws) = WebSocket::new(&socket_url()) else {
        app().server_error.set(Some("Could not connect to the server".into()));
        schedule_retry();
        return;
    };

    let opened = ws.clone();
    let on_open = Closure::<dyn FnMut()>::new(move || {
        if !is_current(&opened) {
            return;
        }
        RETRY.with(|r| r.borrow_mut().0 = 0);
        let app = app();
        app.server_error.set(None);
        app.server.set(ServerState::Connected);
    });
    ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    on_open.forget();

    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let Some(text) = event.data().as_string() else { return };
        match serde_json::from_str::<Value>(&text) {
            Ok(message) => app().on_message(&message),
            Err(e) => leptos::logging::warn!("Invalid server message: {e}"),
        }
    });
    ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    on_message.forget();

    let closed = ws.clone();
    let on_close = Closure::<dyn FnMut()>::new(move || {
        if !is_current(&closed) {
            return;
        }
        SOCKET.with(|s| s.borrow_mut().take());
        let app = app();
        // The upgrade fails with 401 when the login expired; the retry
        // checks the session again.
        let was_open = app.server.get_untracked() == ServerState::Connected;
        app.server_error.set(Some(
            if was_open { "Connection to the server lost" } else { "Could not connect to the server" }.into(),
        ));
        schedule_retry();
    });
    ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    on_close.forget();

    SOCKET.with(|s| *s.borrow_mut() = Some(ws));
}

fn cancel_retry() {
    RETRY.with(|r| {
        if let Some(handle) = r.borrow_mut().1.take() {
            handle.clear();
        }
    });
}

fn schedule_retry() {
    app().server.set(ServerState::Disconnected);
    cancel_retry();
    const DELAYS: [u64; 4] = [1, 2, 5, 10];
    let attempt = RETRY.with(|r| {
        let mut r = r.borrow_mut();
        let attempt = r.0;
        r.0 += 1;
        attempt
    });
    let delay = DELAYS[(attempt as usize).min(DELAYS.len() - 1)];
    let handle = set_timeout_with_handle(
        || leptos::task::spawn_local(check_session()),
        Duration::from_secs(delay),
    )
    .ok();
    RETRY.with(|r| r.borrow_mut().1 = handle);
}
