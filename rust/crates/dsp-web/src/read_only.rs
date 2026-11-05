//! View-only mode of a browser tab: on phones and tablets the panel starts in
//! it, so a finger on the screen moves nothing by accident; a switch at the
//! top allows changes. It is a safety catch of this tab only (not stored, the
//! server does not know it).
//!
//! One capture-phase filter on the window swallows the events that change
//! something inside the page content and the DSP actions (presets, connect)
//! before any handler sees them. Allowed are navigation (sections, channel
//! chips), whatever carries `data-view-ok`, the header (DSP tabs, menu, mute
//! all) and dialogs.

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{Element, Event, KeyboardEvent};

use crate::store::app;

/// Where events are swallowed in view-only mode …
const GUARDED: &str = ".page, .section-bar .btn";
/// … except here.
const ALLOWED: &str = "[data-view-ok], .channel-selector, .section-button";

/// Touch devices (coarse pointer) start in view-only mode.
pub fn starts_read_only() -> bool {
    web_sys::window()
        .and_then(|w| w.match_media("(pointer: coarse)").ok().flatten())
        .is_some_and(|m| m.matches())
}

fn guarded(event: &Event) -> bool {
    let Some(el) = event.target().and_then(|t| t.dyn_into::<Element>().ok()) else { return false };
    el.closest(GUARDED).ok().flatten().is_some() && el.closest(ALLOWED).ok().flatten().is_none()
}

/// Install the filter (once, at start).
pub fn install() {
    let Some(window) = web_sys::window() else { return };
    let filter = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
        if !app().read_only.get_untracked() || !guarded(&event) {
            return;
        }
        let kind = event.type_();
        if kind == "keydown"
            && let Some(key) = event.dyn_ref::<KeyboardEvent>().map(KeyboardEvent::key)
            && matches!(key.as_str(), "Tab" | "Escape")
        {
            return;
        }
        event.stop_immediate_propagation();
        // The wheel still scrolls the page; everything else does nothing.
        if kind != "wheel" {
            event.prevent_default();
        }
        if kind == "click" {
            app().toast("View only – switch to \"Edit\" at the top to make changes", false);
        }
    });
    for kind in ["pointerdown", "mousedown", "click", "dblclick", "keydown", "wheel", "beforeinput", "change"] {
        let _ = window.add_event_listener_with_callback_and_bool(kind, filter.as_ref().unchecked_ref(), true);
    }
    filter.forget();
}
