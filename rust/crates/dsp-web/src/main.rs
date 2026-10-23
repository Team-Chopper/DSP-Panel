//! Web client of the DSP panel (Leptos, client-side rendered, WebAssembly).
//!
//! Talks to the panel server over one WebSocket (same API as the Flutter
//! client) and shares the protocol and op logic with the server (`dsp-core`).

mod app;
mod components;
mod dialogs;
mod fmt;
mod icons;
mod local_settings;
mod meters;
mod net;
mod pages;
mod rta;
mod store;
mod taper;
mod tone;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(app::Root);
}
