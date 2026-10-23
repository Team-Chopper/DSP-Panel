//! Mixing-console style fader: a slot with scale lines across it (labels
//! beside), a ridged cap and an optional level meter. Used vertically for
//! gain, EQ, filter, GEQ and delay, horizontally for the dynamics.
//!
//! Drag the cap (or anywhere on the track) to move it, scroll to step,
//! double-click to reset. A clicked fader takes the keyboard focus: the arrow
//! keys step it (Shift: 10 steps), Page Up/Down move 10 % of the travel.
//!
//! Plain HTML/CSS: the cap moves with a CSS position, the meter is clipped
//! from the meter bank without any re-render (see `meters`).

use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
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

#[component]
pub fn Fader(
    #[prop(into)] value: Signal<f64>,
    taper: Taper,
    on_change: Callback<f64>,
    /// Value set by a double-click.
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
    /// Wheel and arrow key step in value units (default: 1 % of the travel).
    #[prop(optional, into)]
    step: Option<Signal<f64>>,
    #[prop(default = 30.0)] cap: f64,
) -> impl IntoView {
    let travel = NodeRef::<html::Div>::new();
    let dragging = RwSignal::new(false);
    let grab = StoredValue::new(0.0f64);
    let position = Memo::new(move |_| taper.to_position(value.get()));

    let has_labels = label.is_some();
    let has_meter = meter.is_some() && vertical;
    let indicators = meter.as_ref().is_some_and(|m| m.clip || m.limit) && vertical;
    let half = cap / 2.0 + 2.0;
    let travel_style = if vertical {
        format!(
            "top:{}px;bottom:{half}px;left:{}px;right:{}px;--cap:{cap}px",
            half + if indicators { INDICATOR_SPACE } else { 0.0 },
            if has_labels { LABEL_SPACE } else { 0.0 },
            if has_meter { METER_SPACE } else { 0.0 },
        )
    } else {
        format!("left:{half}px;right:{half}px;top:0;bottom:{}px;--cap:{cap}px", if has_labels { 14.0 } else { 0.0 })
    };

    let emit = move |pos: f64| on_change.run(taper.value_at(pos));
    let step_by = move |direction: f64, count: f64| match step {
        Some(step) => on_change.run(value.get_untracked() + direction * step.get_untracked() * count),
        None => emit(position.get_untracked() + direction * 0.01 * count),
    };
    // Position under the pointer, and the cap's pixel position.
    let measure = move |ev: &PointerEvent| -> Option<(f64, f64, f64)> {
        let rect = travel.get_untracked()?.get_bounding_client_rect();
        if vertical {
            let along = ev.client_y() as f64;
            let p = 1.0 - (along - rect.top()) / rect.height().max(1.0);
            Some((along, p.clamp(0.0, 1.0), rect.top() + (1.0 - position.get_untracked()) * rect.height()))
        } else {
            let along = ev.client_x() as f64;
            let p = (along - rect.left()) / rect.width().max(1.0);
            Some((along, p.clamp(0.0, 1.0), rect.left() + position.get_untracked() * rect.width()))
        }
    };
    let pixel_to_position = move |px: f64| -> f64 {
        let Some(el) = travel.get_untracked() else { return 0.0 };
        let rect = el.get_bounding_client_rect();
        if vertical {
            (1.0 - (px - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0)
        } else {
            ((px - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0)
        }
    };

    let on_down = move |ev: PointerEvent| {
        if ev.button() != 0 {
            return;
        }
        let Some((along, pos, cap_px)) = measure(&ev) else { return };
        if let Some(target) = ev.current_target().and_then(|t| t.dyn_into::<HtmlElement>().ok()) {
            let _ = target.focus();
            let _ = target.set_pointer_capture(ev.pointer_id());
        }
        ev.prevent_default();
        if (along - cap_px).abs() <= cap / 2.0 + 4.0 {
            grab.set_value(along - cap_px);
        } else {
            grab.set_value(0.0);
            emit(pos);
        }
        dragging.set(true);
    };
    let on_move = move |ev: PointerEvent| {
        if !dragging.get_untracked() {
            return;
        }
        let along = if vertical { ev.client_y() } else { ev.client_x() } as f64;
        emit(pixel_to_position(along - grab.get_value()));
    };
    let on_up = move |_: PointerEvent| dragging.set(false);
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

    let meter_view = meter.filter(|_| vertical).map(|m| view! { <FaderMeter spec=m indicators=indicators cap=cap/> });

    view! {
        <div
            class="fader"
            class:vertical=vertical
            class:horizontal=!vertical
            class:dragging=move || dragging.get()
            style=format!("--fc:{color}")
            tabindex="0"
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
                <div class="fader-cap" style=cap_style></div>
            </div>
            {meter_view}
        </div>
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
