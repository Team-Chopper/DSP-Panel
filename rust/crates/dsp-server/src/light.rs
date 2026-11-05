//! Limit light: a page for the DJ booth or the stage (`/limit`, no login,
//! off by default) with one big lamp that is on while the limiter of a
//! watched output acts, and for one more second. It learns nothing but
//! `{on, ready}` from `/limit/events` (server-sent events) and changes nothing.
//!
//! An output is watched unless it is muted (nobody hears it) or left out in
//! the settings. The limiter bits come from the meter answers, so while the
//! light is on the server polls the meters of every DSP. On a DSP 204 the
//! limiter bit was seen to stay on for up to 4 s.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use dsp_core::DeviceState;
use dsp_core::protocol::limiter_active;
use futures_util::stream::{self, Stream};
use serde_json::json;
use tokio::sync::watch;
use tokio::time::Instant;

use crate::http::AppState;

/// How long the lamp stays on after a limiter acted.
pub const HOLD: Duration = Duration::from_secs(1);
/// Open event streams at most.
const MAX_STREAMS: usize = 20;
static STREAMS: AtomicUsize = AtomicUsize::new(0);

/// What the page needs, published by the hub.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LightSnapshot {
    pub enabled: bool,
    /// Last time a watched limiter acted.
    pub last_hit: Option<Instant>,
    /// A ready DSP has at least one watched output.
    pub ready: bool,
}

impl LightSnapshot {
    pub fn on(&self, now: Instant) -> bool {
        self.last_hit.is_some_and(|t| now.duration_since(t) < HOLD)
    }
}

/// Limiter activity per DSP (kept by the hub).
#[derive(Debug, Default)]
pub struct LightTracker {
    /// Per DSP: has a watched output, last limiter hit.
    dsps: HashMap<String, (bool, Option<Instant>)>,
}

/// The outputs of [state] the light watches: not muted, not in [ignore].
pub fn watched_outputs(state: &DeviceState, ignore: &[String]) -> Vec<usize> {
    state
        .model
        .outputs
        .iter()
        .enumerate()
        .filter(|(_, ch)| !state.mutes.get(**ch).copied().unwrap_or(false) && !ignore.iter().any(|i| i == *ch))
        .map(|(i, _)| i)
        .collect()
}

impl LightTracker {
    /// A meter answer of [dsp]: [watched] outputs and the dynamics word.
    pub fn note(&mut self, dsp: &str, watched: &[usize], dynamics: u16, now: Instant) {
        let entry = self.dsps.entry(dsp.to_string()).or_default();
        entry.0 = !watched.is_empty();
        if watched.iter().any(|o| limiter_active(dynamics, *o)) {
            entry.1 = Some(now);
        }
    }

    /// The DSP is not ready (any more): it no longer counts.
    pub fn forget(&mut self, dsp: &str) {
        self.dsps.remove(dsp);
    }

    pub fn snapshot(&self, enabled: bool) -> LightSnapshot {
        LightSnapshot {
            enabled,
            last_hit: self.dsps.values().filter_map(|(_, hit)| *hit).max(),
            ready: self.dsps.values().any(|(watched, _)| *watched),
        }
    }
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "Not found").into_response()
}

/// GET /limit
pub async fn page(State(state): State<Arc<AppState>>) -> Response {
    if !state.light.borrow().enabled {
        return not_found();
    }
    let mut response = include_str!("limit.html").into_response();
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    response
}

/// GET /limit/events: `{on, ready}` whenever it changes.
pub async fn events(State(state): State<Arc<AppState>>) -> Response {
    if !state.light.borrow().enabled {
        return not_found();
    }
    if STREAMS.fetch_add(1, Ordering::SeqCst) >= MAX_STREAMS {
        STREAMS.fetch_sub(1, Ordering::SeqCst);
        return (StatusCode::TOO_MANY_REQUESTS, "Too many open pages").into_response();
    }
    let stream = light_stream(state.light.clone(), StreamGuard);
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))).into_response()
}

/// Decrements the stream count when the stream is dropped.
struct StreamGuard;

impl Drop for StreamGuard {
    fn drop(&mut self) {
        STREAMS.fetch_sub(1, Ordering::SeqCst);
    }
}

fn light_stream(
    light: watch::Receiver<LightSnapshot>,
    guard: StreamGuard,
) -> impl Stream<Item = Result<Event, Infallible>> {
    stream::unfold((light, None::<(bool, bool)>, guard), |(light, last, guard)| async move {
        loop {
            let snapshot = *light.borrow();
            if !snapshot.enabled {
                return None;
            }
            let now = (snapshot.on(Instant::now()), snapshot.ready);
            if last != Some(now) {
                let event = Event::default().data(json!({"on": now.0, "ready": now.1}).to_string());
                return Some((Ok(event), (light, Some(now), guard)));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dsp_core::DeviceModel;
    use dsp_core::ops::{self, Op};

    #[test]
    fn muted_and_ignored_outputs_do_not_count() {
        let mut state = DeviceState::new(DeviceModel::DSP206);
        ops::apply(&mut state, &Op::mute("Out 2", true));
        assert_eq!(watched_outputs(&state, &["Out 6".to_string()]), [0, 2, 3, 4]);
    }

    #[test]
    fn the_lamp_holds_one_second() {
        let start = Instant::now();
        let mut t = LightTracker::default();
        assert!(!t.snapshot(true).ready);
        t.note("a", &[0, 1], 0, start);
        assert!(t.snapshot(true).ready && !t.snapshot(true).on(start));
        // Out 2 limits (bit 8 + 1), Out 3 is not watched.
        t.note("a", &[0, 1], 0x0200, start);
        assert!(t.snapshot(true).on(start + Duration::from_millis(900)));
        assert!(!t.snapshot(true).on(start + Duration::from_millis(1100)));
        t.note("b", &[0], 0x0400, start + Duration::from_secs(2));
        assert!(!t.snapshot(true).on(start + Duration::from_millis(2100)), "Out 3 of b is not watched");
        t.forget("a");
        assert!(t.snapshot(true).last_hit.is_none());
    }
}
