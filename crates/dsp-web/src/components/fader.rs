//! Mixing-console style fader: a slot with scale lines across it (labels
//! beside), a ridged cap and an optional level meter. Used vertically for
//! gain, EQ, filter, GEQ and delay, horizontally for the dynamics.
//!
//! Drag the cap (or anywhere on the track) to move it, scroll to step,
//! double-click to reset. While dragging, Shift moves the cap at a quarter of
//! the speed, and the cap snaps to the reset value within a few pixels of it
//! (detent, with a short vibration where the browser supports it). A clicked
//! fader takes the keyboard focus: the arrow keys step it (Shift: 10 steps),
//! Page Up/Down move 10 % of the travel. Horizontal faders have − / + buttons
//! at their ends: a press steps once, holding repeats faster and faster.
//!
//! Plain HTML/CSS: the cap moves with a CSS position, the meter is clipped
//! from the meter bank without any re-render (see `meters`).

use std::time::Duration;

use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{HtmlElement, KeyboardEvent, PointerEvent, WheelEvent};

use crate::meters::Led;
use crate::store::Device;
use crate::taper::Taper;

/// Meter beside a vertical fader: level of [channel] on the fader's taper.
#[derive(Clone)]
pub struct MeterSpec {
    pub device: Device,
    pub channel: String,
    pub clip: bool,
    pub limit: bool,
}

const LABEL_SPACE: f64 = 24.0;
const METER_SPACE: f64 = 26.0;
const INDICATOR_SPACE: f64 = 2.0 * (20.0 + 3.0);
/// Width of a − / + button plus its gap to the travel.
const STEPPER_SPACE: f64 = 20.0;
/// The detent catches the cap while the pointer is this close to the reset value.
const DETENT_PX: f64 = 8.0;
/// Speed of a drag while Shift is held.
const FINE_SPEED: f64 = 0.25;
/// Stepper hold: first repeat, then the repeat interval.
const REPEAT_DELAY: Duration = Duration::from_millis(380);
const REPEAT_INTERVAL: Duration = Duration::from_millis(70);

/// The travel in client pixels along the fader's axis.
#[derive(Clone, Copy)]
struct Track {
    /// Top (vertical) or left edge …
    start: f64,
    /// … and length.
    len: f64,
    vertical: bool,
}

impl Track {
    /// Pixel of a fader position.
    fn px(self, position: f64) -> f64 {
        if self.vertical { self.start + (1.0 - position) * self.len } else { self.start + position * self.len }
    }

    /// Fader position at a pixel (clamped to the travel).
    fn position(self, px: f64) -> f64 {
        let t = (px - self.start) / self.len;
        (if self.vertical { 1.0 - t } else { t }).clamp(0.0, 1.0)
    }

    fn clamp(self, px: f64) -> f64 {
        px.clamp(self.start, self.start + self.len)
    }
}

/// A running pointer drag. The cap follows the pointer relative to an anchor,
/// so changing the speed (Shift) continues smoothly from where it happens.
#[derive(Clone, Copy, Default)]
struct Drag {
    /// Pointer coordinate at the anchor …
    pointer: f64,
    /// … and the cap's pixel there (before snapping to the detent).
    cap: f64,
    /// Fine mode (Shift held).
    fine: bool,
    /// The last value relative to the reset value: −1 below, 0 on it, 1 above.
    side: i8,
}

impl Drag {
    fn speed(self) -> f64 {
        if self.fine { FINE_SPEED } else { 1.0 }
    }

    /// The cap's pixel for the pointer at [along].
    fn cap_at(self, along: f64) -> f64 {
        self.cap + (along - self.pointer) * self.speed()
    }
}

/// A short haptic tick (phones); silently nothing where the browser has no
/// vibration support.
fn haptic_tick() {
    let Some(navigator) = web_sys::window().map(|w| w.navigator()) else { return };
    let supported = js_sys::Reflect::get(&navigator, &JsValue::from_str("vibrate")).is_ok_and(|f| f.is_function());
    if supported {
        let _ = navigator.vibrate_with_duration(8);
    }
}

#[component]
pub fn Fader(
    #[prop(into)] value: Signal<f64>,
    taper: Taper,
    on_change: Callback<f64>,
    /// Value set by a double-click; a drag snaps to it (detent).
    #[prop(optional)]
    reset: Option<f64>,
    /// Values with a scale line; [major] ones are drawn brighter.
    #[prop(optional)]
    marks: Vec<f64>,
    #[prop(optional)] major: Vec<f64>,
    /// Scale label of a mark (none: no labels).
    #[prop(optional, into)]
    label: Option<Callback<f64, String>>,
    #[prop(into, default = "var(--accent)".to_string())] color: String,
    #[prop(default = true)] vertical: bool,
    #[prop(optional)] meter: Option<MeterSpec>,
    /// Wheel, arrow key and stepper step in value units (default: 1 % of the travel).
    #[prop(optional, into)]
    step: Option<Signal<f64>>,
    #[prop(default = 30.0)] cap: f64,
    /// Max level: the range above it is drawn hatched.
    #[prop(optional, into)]
    limit: Option<Signal<Option<f64>>>,
    /// − / + buttons at the ends (horizontal faders only; default: shown on
    /// horizontal faders).
    #[prop(optional)]
    steppers: Option<bool>,
) -> impl IntoView {
    let root = NodeRef::<html::Div>::new();
    let travel = NodeRef::<html::Div>::new();
    let dragging = RwSignal::new(false);
    let fine = RwSignal::new(false);
    let drag = StoredValue::new(Drag::default());
    let position = Memo::new(move |_| taper.to_position(value.get()));

    let has_labels = label.is_some();
    let has_meter = meter.is_some() && vertical;
    let indicators = meter.as_ref().is_some_and(|m| m.clip || m.limit) && vertical;
    let has_steppers = steppers.unwrap_or(!vertical) && !vertical;
    let half = cap / 2.0 + 2.0;
    let label_below = if has_labels { 14.0 } else { 0.0 };
    let travel_style = if vertical {
        format!(
            "top:{}px;bottom:{half}px;left:{}px;right:{}px;--cap:{cap}px",
            half + if indicators { INDICATOR_SPACE } else { 0.0 },
            if has_labels { LABEL_SPACE } else { 0.0 },
            if has_meter { METER_SPACE } else { 0.0 },
        )
    } else {
        let side = half + if has_steppers { STEPPER_SPACE } else { 0.0 };
        format!("left:{side}px;right:{side}px;top:0;bottom:{label_below}px;--cap:{cap}px")
    };

    let emit = move |pos: f64| on_change.run(taper.value_at(pos));
    let step_by = move |direction: f64, count: f64| match step {
        Some(step) => on_change.run(value.get_untracked() + direction * step.get_untracked() * count),
        None => emit(position.get_untracked() + direction * 0.01 * count),
    };
    let track = move || -> Option<Track> {
        let rect = travel.get_untracked()?.get_bounding_client_rect();
        Some(if vertical {
            Track { start: rect.top(), len: rect.height().max(1.0), vertical }
        } else {
            Track { start: rect.left(), len: rect.width().max(1.0), vertical }
        })
    };
    let pointer_along = move |ev: &PointerEvent| (if vertical { ev.client_y() } else { ev.client_x() }) as f64;

    // Detent at the reset value (value, position); not at an end of the travel,
    // where the end stop already is one.
    let detent = reset.map(|r| (r, taper.to_position(r))).filter(|&(_, p)| p > 0.001 && p < 0.999);
    let side_of = move |p: f64| -> i8 {
        match detent {
            Some((_, d)) if (p - d).abs() < 1e-6 => 0,
            Some((_, d)) if p > d => 1,
            _ => -1,
        }
    };
    // Move the cap to the pixel [px], snapping to the detent near it; ticks
    // when the value reaches or jumps across the reset value.
    let drag_to = move |track: Track, px: f64, d: &mut Drag| {
        let pos = track.position(px);
        let Some((reset_value, reset_pos)) = detent else {
            emit(pos);
            return;
        };
        let side = if (px - track.px(reset_pos)).abs() <= DETENT_PX * d.speed() {
            0
        } else if pos > reset_pos {
            1
        } else {
            -1
        };
        if side != d.side && (side == 0 || d.side != 0) {
            haptic_tick();
        }
        d.side = side;
        if side == 0 { on_change.run(reset_value) } else { emit(pos) }
    };

    let on_down = move |ev: PointerEvent| {
        if ev.button() != 0 {
            return;
        }
        let Some(track) = track() else { return };
        let along = pointer_along(&ev);
        let p = position.get_untracked();
        let cap_px = track.px(p);
        if let Some(target) = ev.current_target().and_then(|t| t.dyn_into::<HtmlElement>().ok()) {
            let _ = target.focus();
            let _ = target.set_pointer_capture(ev.pointer_id());
        }
        ev.prevent_default();
        let mut d = Drag { pointer: along, cap: cap_px, fine: ev.shift_key(), side: side_of(p) };
        if (along - cap_px).abs() > cap / 2.0 + 4.0 {
            // Beside the cap: it jumps to the pointer.
            d.cap = along;
            drag_to(track, along, &mut d);
        }
        drag.set_value(d);
        fine.set(d.fine);
        dragging.set(true);
    };
    let on_move = move |ev: PointerEvent| {
        if !dragging.get_untracked() {
            return;
        }
        let Some(track) = track() else { return };
        let along = pointer_along(&ev);
        let mut d = drag.get_value();
        let mut px = d.cap_at(along);
        if ev.shift_key() != d.fine {
            // Shift pressed or released: continue from here at the other speed.
            d = Drag { pointer: along, cap: px, fine: ev.shift_key(), ..d };
            fine.set(d.fine);
        }
        if d.fine && track.clamp(px) != px {
            // Past an end in fine mode: turning back moves the cap at once.
            px = track.clamp(px);
            d = Drag { pointer: along, cap: px, ..d };
        }
        drag_to(track, px, &mut d);
        drag.set_value(d);
    };
    let on_up = move |_: PointerEvent| {
        dragging.set(false);
        if fine.get_untracked() {
            fine.set(false);
        }
    };
    let on_wheel = move |ev: WheelEvent| {
        let delta = if vertical || ev.delta_x() == 0.0 { -ev.delta_y() } else { ev.delta_x() };
        if delta == 0.0 {
            return;
        }
        ev.prevent_default();
        step_by(delta.signum(), 1.0);
    };
    let on_key = move |ev: KeyboardEvent| {
        let count = if ev.shift_key() { 10.0 } else { 1.0 };
        match ev.key().as_str() {
            "ArrowUp" | "ArrowRight" => step_by(1.0, count),
            "ArrowDown" | "ArrowLeft" => step_by(-1.0, count),
            "PageUp" => emit(position.get_untracked() + 0.1),
            "PageDown" => emit(position.get_untracked() - 0.1),
            "Escape" => {
                if let Some(t) = ev.current_target().and_then(|t| t.dyn_into::<HtmlElement>().ok()) {
                    let _ = t.blur();
                }
            }
            _ => return,
        }
        ev.prevent_default();
    };
    let on_dblclick = move |_| {
        if let Some(r) = reset {
            on_change.run(r);
        }
    };

    let along = move |p: f64| if vertical { format!("top:{:.3}%", (1.0 - p) * 100.0) } else { format!("left:{:.3}%", p * 100.0) };
    let mark_views = marks
        .iter()
        .map(|m| {
            let p = taper.to_position(*m);
            let is_major = major.contains(m);
            let text = label.map(|f| f.run(*m)).unwrap_or_default();
            view! {
                <div class="fader-mark" class:major=is_major style=along(p)></div>
                {(!text.is_empty()).then(|| view! { <span class="fader-label" class:major=is_major style=along(p)>{text}</span> })}
            }
        })
        .collect_view();

    let fill_style = move || {
        let p = position.get() * 100.0;
        if vertical { format!("height:{p:.3}%") } else { format!("width:{p:.3}%") }
    };
    let cap_style = move || along(position.get());
    let limit_view = limit.map(|limit| {
        let style = move || match limit.get() {
            Some(v) => {
                let p = taper.to_position(v);
                if vertical {
                    format!("top:0;height:{:.3}%", (1.0 - p) * 100.0)
                } else {
                    format!("left:{:.3}%;right:0", p * 100.0)
                }
            }
            None => "display:none".to_string(),
        };
        view! { <div class="fader-limit" style=style></div> }
    });

    let meter_view = meter.filter(|_| vertical).map(|m| view! { <FaderMeter spec=m indicators=indicators cap=cap/> });

    // One stepper press: [count] steps up (> 0) or down, kept inside the range.
    let stepper_views = has_steppers.then(|| {
        let nudge = Callback::new(move |count: f64| {
            let current = value.get_untracked();
            let next = match step {
                Some(step) => {
                    let (a, b) = (taper.value_at(0.0), taper.value_at(1.0));
                    (current + count * step.get_untracked()).clamp(a.min(b), a.max(b))
                }
                None => taper.value_at(position.get_untracked() + count * 0.01),
            };
            if next != current {
                on_change.run(next);
            }
        });
        view! {
            <FaderStepper up=false bottom=label_below on_step=nudge fader=root/>
            <FaderStepper up=true bottom=label_below on_step=nudge fader=root/>
        }
    });

    view! {
        <div
            class="fader"
            class:vertical=vertical
            class:horizontal=!vertical
            class:dragging=move || dragging.get()
            class:fine=move || fine.get()
            style=format!("--fc:{color}")
            tabindex="0"
            node_ref=root
            on:pointerdown=on_down
            on:pointermove=on_move
            on:pointerup=on_up
            on:pointercancel=on_up
            on:wheel=on_wheel
            on:keydown=on_key
            on:dblclick=on_dblclick
        >
            <div class="fader-travel" node_ref=travel style=travel_style>
                {mark_views}
                <div class="fader-slot"></div>
                <div class="fader-fill" style=fill_style></div>
                {limit_view}
                <div class="fader-cap" style=cap_style></div>
            </div>
            {stepper_views}
            {meter_view}
        </div>
    }
}

/// − or + button at an end of a horizontal fader: a press steps once, holding
/// repeats (first after 380 ms, then every 70 ms; 3 steps at a time after 12
/// repeats, 10 after 30).
#[component]
fn FaderStepper(
    /// + (right end) or − (left end).
    up: bool,
    /// Space below the travel (labels): the button is centred on the slot.
    bottom: f64,
    /// Steps to move (signed).
    on_step: Callback<f64>,
    /// The fader, which takes the keyboard focus on a press.
    fader: NodeRef<html::Div>,
) -> impl IntoView {
    let delay = StoredValue::new(None::<TimeoutHandle>);
    let repeat = StoredValue::new(None::<IntervalHandle>);
    let repeats = StoredValue::new(0u32);
    let pressed = RwSignal::new(false);
    let direction = if up { 1.0 } else { -1.0 };

    let stop_timers = move || {
        if let Some(handle) = delay.try_update_value(Option::take).flatten() {
            handle.clear();
        }
        if let Some(handle) = repeat.try_update_value(Option::take).flatten() {
            handle.clear();
        }
    };
    let again = move || {
        let n = repeats.try_update_value(|n| {
            *n += 1;
            *n
        });
        let factor = match n.unwrap_or(1) {
            n if n > 30 => 10.0,
            n if n > 12 => 3.0,
            _ => 1.0,
        };
        // A timer tick: never panic on a disposed callback.
        let _ = on_step.try_run(direction * factor);
    };
    let on_down = move |ev: PointerEvent| {
        // Not a drag or a focus change of the fader underneath.
        ev.stop_propagation();
        if ev.button() != 0 {
            return;
        }
        ev.prevent_default();
        if let Some(el) = fader.get_untracked() {
            let _ = el.focus();
        }
        if let Some(target) = ev.current_target().and_then(|t| t.dyn_into::<HtmlElement>().ok()) {
            let _ = target.set_pointer_capture(ev.pointer_id());
        }
        stop_timers();
        repeats.set_value(0);
        pressed.set(true);
        on_step.run(direction);
        let start_repeating = move || {
            delay.set_value(None);
            again();
            if let Ok(handle) = set_interval_with_handle(again, REPEAT_INTERVAL)
                && let Some(Some(handle)) = repeat.try_set_value(Some(handle))
            {
                // Unmounted meanwhile.
                handle.clear();
            }
        };
        if let Ok(handle) = set_timeout_with_handle(start_repeating, REPEAT_DELAY) {
            delay.set_value(Some(handle));
        }
    };
    let on_release = move |_: PointerEvent| {
        stop_timers();
        pressed.set(false);
    };
    on_cleanup(stop_timers);

    let title = if up { "Step up (hold to repeat)" } else { "Step down (hold to repeat)" };
    view! {
        <button
            type="button"
            class={if up { "fader-step up" } else { "fader-step down" }}
            class:pressed=move || pressed.get()
            tabindex="-1"
            title=title
            aria-label=title
            style=format!("bottom:{bottom}px")
            on:pointerdown=on_down
            on:pointerup=on_release
            on:pointercancel=on_release
            on:lostpointercapture=on_release
            on:dblclick={move |ev| ev.stop_propagation()}
            on:contextmenu={move |ev| ev.prevent_default()}
        >
            <span>{if up { "+" } else { "\u{2212}" }}</span>
        </button>
    }
}

/// The meter of a vertical fader (spanning exactly its scale) with the
/// limit/clip indicators above it.
#[component]
fn FaderMeter(spec: MeterSpec, indicators: bool, cap: f64) -> impl IntoView {
    let lit = NodeRef::<html::Div>::new();
    let peak = NodeRef::<html::Div>::new();
    let clip = NodeRef::<html::Div>::new();
    let limit = NodeRef::<html::Div>::new();
    let half = cap / 2.0 + 2.0;
    let top = half + if indicators { INDICATOR_SPACE } else { 0.0 };
    let MeterSpec { device, channel, clip: has_clip, limit: has_limit } = spec;

    let registrations = StoredValue::new(Vec::<u64>::new());
    let ch = channel.clone();
    Effect::new(move |_| {
        let (Some(lit), Some(peak)) = (lit.get(), peak.get()) else { return };
        let mut ids = Vec::new();
        device.meters.update_value(|bank| {
            ids.push(bank.register_bar(&ch, lit.into(), peak.into(), crate::taper::GAIN_TAPER));
            if let Some(el) = clip.get_untracked() {
                ids.push(bank.register_led(&ch, el.into(), Led::Clip));
            }
            if let Some(el) = limit.get_untracked() {
                ids.push(bank.register_led(&ch, el.into(), Led::Limit));
            }
        });
        registrations.set_value(ids);
    });
    on_cleanup(move || {
        let ids = registrations.get_value();
        device.meters.update_value(|bank| {
            for id in ids {
                bank.unregister(id);
            }
        });
    });

    // Bottom slot right above the scale: clip; top slot: limit.
    let slot_top = |slot: usize| format!("top:{}px", 2 + slot * 23);
    view! {
        {has_limit.then(|| view! { <div class="led limit" node_ref=limit style=slot_top(0) title="Limiter working">"L"</div> })}
        {has_clip.then(|| view! { <div class="led clip" node_ref=clip style=slot_top(1) title="Clip">"C"</div> })}
        <div class="meter" style=format!("top:{top}px;bottom:{half}px")>
            <div class="meter-dim"></div>
            <div class="meter-lit" node_ref=lit></div>
            <div class="meter-gaps"></div>
            <div class="meter-peak" node_ref=peak></div>
        </div>
    }
}
