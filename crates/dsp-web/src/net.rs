//! The WebSocket to the panel server. The web client is always served by the
//! panel server itself, so the URL is relative to the page's own origin.

use std::cell::RefCell;
use std::time::Duration;

use leptos::prelude::*;
use serde_json::Value;
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
    connect();
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
    let handle = set_timeout_with_handle(connect, Duration::from_secs(delay)).ok();
    RETRY.with(|r| r.borrow_mut().1 = handle);
}
