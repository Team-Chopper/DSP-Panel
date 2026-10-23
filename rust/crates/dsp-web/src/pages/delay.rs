//! Delay: on top one window for the inputs (left) and one for the outputs
//! (right) with a speaker per channel that is dragged to the right to delay
//! it; below vertical faders to fine-tune. Values are shown in ms, m or ft;
//! like in the original editor the unit is stored on the DSP (0x15) and the
//! distance uses 344 m/s. The range of each side can be zoomed; the faders use
//! the same range, so a small range gives a fine adjustment.

use dsp_core::protocol as p;
use leptos::prelude::*;
use web_sys::PointerEvent;

use crate::components::common::{PanelCard, Segmented};
use crate::components::fader::Fader;
use crate::components::value_field::ValueField;
use crate::fmt::{fixed, parse_number};
use crate::icons::{I, Icon};
use crate::pages::dsp::ui;
use crate::store::Device;
use crate::taper::Taper;

fn max_ms() -> f64 {
    p::delay_raw_to_ms(p::DELAY_RAW_MAX)
}

/// Selectable ranges in ms (the last one is the device maximum).
fn ranges() -> [f64; 10] {
    [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, max_ms()]
}

const LABEL_WIDTH: f64 = 76.0;
const SPEAKER: f64 = 26.0;

/// Display unit: 0 = ms, 1 = m, 2 = ft (as stored on the DSP).
#[derive(Clone, Copy, PartialEq, Eq)]
struct Unit(usize);

impl Unit {
    fn new(unit: i64) -> Unit {
        Unit(if (0..=2).contains(&unit) { unit as usize } else { 0 })
    }

    fn suffix(self) -> &'static str {
        p::DELAY_UNIT_NAMES[self.0]
    }

    /// Length of one ms of delay in this unit (1 for ms).
    fn per_ms(self) -> f64 {
        match self.0 {
            1 => p::METERS_PER_MS,
            2 => p::METERS_PER_MS * p::FEET_PER_METER,
            _ => 1.0,
        }
    }

    /// A delay in ms as a value in this unit.
    fn of_ms(self, ms: f64) -> f64 {
        ms * self.per_ms()
    }

    fn to_ms(self, value: f64) -> f64 {
        value / self.per_ms()
    }

    fn format(self, ms: f64) -> String {
        let v = self.of_ms(ms);
        fixed(v, if v < 10.0 { 2 } else { 1 })
    }

    fn format_range(self, ms: f64) -> String {
        let v = self.of_ms(ms);
        format!("{} {}", fixed(v, if v < 1.0 { 2 } else if v < 10.0 { 1 } else { 0 }), self.suffix())
    }
}

/// Unit typed after a number ("3 m", "10 ms", "8 ft"), if any.
fn typed_unit(input: &str) -> Option<usize> {
    let lower = input.to_lowercase();
    let after_number = |suffix: &str| {
        lower.match_indices(suffix).any(|(i, _)| lower[..i].trim_end().chars().last().is_some_and(|c| c.is_ascii_digit()))
    };
    if after_number("ms") {
        Some(0)
    } else if after_number("ft") {
        Some(2)
    } else if after_number("m") {
        Some(1)
    } else {
        None
    }
}

fn delay_ms(device: Device, ch: &str) -> f64 {
    device.state.with(|s| p::delay_raw_to_ms(s.delays.get(ch).copied().unwrap_or(0)))
}

#[component]
pub fn DelayPage() -> impl IntoView {
    let ui = ui();
    let device = ui.device;
    let model = ui.model;
    let unit = Memo::new(move |_| Unit::new(device.state.with(|s| s.delay_unit)));
    let chosen_in = RwSignal::new(None::<f64>);
    let chosen_out = RwSignal::new(None::<f64>);
    let auto = move |inputs: bool| {
        Memo::new(move |_| {
            let m = model.get();
            let channels = if inputs { m.inputs } else { m.outputs };
            let longest = channels.iter().map(|c| delay_ms(device, c)).fold(0.0, f64::max);
            ranges().into_iter().find(|r| *r >= longest * 1.2 && *r >= 5.0).unwrap_or_else(max_ms)
        })
    };
    let (auto_in, auto_out) = (auto(true), auto(false));
    let range_in = Signal::derive(move || chosen_in.get().unwrap_or_else(|| auto_in.get()));
    let range_out = Signal::derive(move || chosen_out.get().unwrap_or_else(|| auto_out.get()));
    let columns = move || {
        let m = model.get();
        format!("grid-template-columns:{}fr {}fr", m.inputs.len().max(3), m.outputs.len().max(3))
    };
    view! {
        <div class="delay-page">
            <div class="delay-top">
                <span class="small muted">"Unit"</span>
                <Segmented labels=p::DELAY_UNIT_NAMES.to_vec() selected=Signal::derive(move || unit.get().0)
                    on_select=Callback::new(move |u: usize| device.set_delay_unit(u as i64))/>
                <span class="delay-hint">"Drag a speaker to the right to delay the channel; fine-tune with the faders below."</span>
            </div>
            <div class="delay-body" style=columns>
                <Stage device=device inputs=true range=range_in chosen=chosen_in unit=unit/>
                <Stage device=device inputs=false range=range_out chosen=chosen_out unit=unit/>
                <DelayFaders device=device inputs=true range=range_in unit=unit/>
                <DelayFaders device=device inputs=false range=range_out unit=unit/>
            </div>
        </div>
    }
}

// ─── Speakers ───

#[component]
fn Stage(
    device: Device,
    inputs: bool,
    range: Signal<f64>,
    chosen: RwSignal<Option<f64>>,
    unit: Memo<Unit>,
) -> impl IntoView {
    let model = ui().model;
    let color = if inputs { "var(--input)" } else { "var(--output)" };
    // Range kept while a speaker is dragged (the automatic range would
    // otherwise change under the pointer).
    let drag_range = RwSignal::new(None::<f64>);
    let shown = Signal::derive(move || drag_range.get().unwrap_or_else(|| range.get()));
    let lane = format!("(100% - {}px)", LABEL_WIDTH + SPEAKER);
    let lane_left = LABEL_WIDTH + SPEAKER / 2.0;

    let grid = {
        let lane = lane.clone();
        move || {
            let (r, u) = (shown.get(), unit.get());
            (0..=10)
                .map(|i| {
                    let left = format!("left:calc({lane_left}px + {} * {lane})", i as f64 / 10.0);
                    let label = (i % 2 == 0).then(|| {
                        let v = u.of_ms(r * i as f64 / 10.0);
                        let text = if v < 10.0 { fixed(v, if v == 0.0 { 0 } else if v < 1.0 { 2 } else { 1 }) } else { fixed(v, 0) };
                        view! { <span class="stage-label" style=left.clone()>{text}</span> }
                    });
                    view! {
                        <div class="stage-line" class:major=i % 5 == 0 style=left.clone()></div>
                        {label}
                    }
                })
                .collect_view()
        }
    };

    let rows = move || {
        let m = model.get();
        let channels = if inputs { m.inputs } else { m.outputs };
        channels
            .iter()
            .map(|ch| view! { <SpeakerRow device=device channel=ch color=color range=shown unit=unit drag_range=drag_range/> })
            .collect_view()
    };

    let title = Signal::derive(move || if inputs { "Inputs" } else { "Outputs" }.to_string());
    let trailing = view! { <RangeMenu chosen=chosen current=range unit=unit/> }.into_any();
    view! {
        <PanelCard title=title accent=color trailing=trailing class="stage-card">
            <div class="stage">
                {grid}
                <div class="stage-rows">{rows}</div>
            </div>
        </PanelCard>
    }
}

#[component]
fn SpeakerRow(
    device: Device,
    channel: &'static str,
    color: &'static str,
    range: Signal<f64>,
    unit: Memo<Unit>,
    drag_range: RwSignal<Option<f64>>,
) -> impl IntoView {
    let ms = Memo::new(move |_| delay_ms(device, channel));
    let frac = move || (ms.get() / range.get()).clamp(0.0, 1.0);
    let lane = format!("(100% - {}px)", LABEL_WIDTH + SPEAKER);
    let lane_left = LABEL_WIDTH + SPEAKER / 2.0;
    let active_style = {
        let lane = lane.clone();
        move || format!("left:{lane_left}px;width:calc({:.5} * {lane});background:{color}", frac())
    };
    let speaker_style = move || {
        let on = ms.get() > 0.0;
        format!(
            "left:calc({LABEL_WIDTH}px + {:.5} * {lane});border-color:{color};background:{};color:{}",
            frac(),
            if on { color } else { "var(--surface-high)" },
            if on { "var(--bg)" } else { color }
        )
    };

    // Drag relative to the start, so small steps are not lost to the 1/96 ms
    // resolution: (start x, start ms, lane width).
    let drag = StoredValue::new(None::<(f64, f64, f64)>);
    let on_down = move |ev: PointerEvent| {
        if ev.button() != 0 {
            return;
        }
        let Some(target) = ev.current_target().and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok())
        else {
            return;
        };
        let row_width = target.parent_element().map_or(0.0, |row| row.get_bounding_client_rect().width());
        let _ = target.set_pointer_capture(ev.pointer_id());
        ev.prevent_default();
        drag_range.set(Some(range.get_untracked()));
        drag.set_value(Some((ev.client_x() as f64, ms.get_untracked(), (row_width - LABEL_WIDTH - SPEAKER).max(1.0))));
    };
    let on_move = move |ev: PointerEvent| {
        let Some((start_x, start_ms, lane_width)) = drag.get_value() else { return };
        let r = drag_range.get_untracked().unwrap_or_else(|| range.get_untracked());
        let next = (start_ms + (ev.client_x() as f64 - start_x) / lane_width * r).clamp(0.0, r.min(max_ms()));
        device.set_delay_ms(channel, next);
    };
    let on_up = move |_: PointerEvent| {
        if drag.get_value().is_some() {
            drag.set_value(None);
            drag_range.set(None);
        }
    };
    let title = move || format!("{}: {} {}", device.alias(channel), unit.get().format(ms.get()), unit.get().suffix());
    view! {
        <div class="speaker-row">
            <span class="speaker-name" style=format!("color:{color}")>{move || device.alias(channel)}</span>
            <div class="speaker-lane" style=format!("left:{lane_left}px;right:{}px", SPEAKER / 2.0)></div>
            <div class="speaker-active" style=active_style></div>
            <div class="speaker" style=speaker_style title=title
                on:pointerdown=on_down on:pointermove=on_move on:pointerup=on_up on:pointercancel=on_up
                on:dblclick=move |_| device.set_delay_ms(channel, 0.0)>
                <Icon icon=I::Speaker size=18/>
            </div>
        </div>
    }
}

#[component]
fn RangeMenu(chosen: RwSignal<Option<f64>>, current: Signal<f64>, unit: Memo<Unit>) -> impl IntoView {
    let options = move || {
        let u = unit.get();
        let auto_label = format!("Auto · {}", u.format_range(current.get()));
        let auto = view! { <option value="auto" selected=move || chosen.get().is_none()>{auto_label}</option> };
        let fixed = ranges()
            .into_iter()
            .enumerate()
            .map(|(i, r)| view! { <option value=i selected=move || chosen.get() == Some(r)>{u.format_range(r)}</option> })
            .collect_view();
        view! { {auto}{fixed} }
    };
    view! {
        <label class="range-menu" title="Range of the speakers and faders">
            <Icon icon=I::ZoomIn size=14/>
            <select class="mono" on:change=move |ev| {
                let value = event_target_value(&ev);
                chosen.set(value.parse::<usize>().ok().and_then(|i| ranges().get(i).copied()));
            }>{options}</select>
        </label>
    }
}

// ─── Faders ───

#[component]
fn DelayFaders(device: Device, inputs: bool, range: Signal<f64>, unit: Memo<Unit>) -> impl IntoView {
    let model = ui().model;
    let color = if inputs { "var(--input)" } else { "var(--output)" };
    let columns = move || {
        let m = model.get();
        let channels = if inputs { m.inputs } else { m.outputs };
        channels
            .iter()
            .map(|ch| {
                let ch: &'static str = ch;
                let ms = Memo::new(move |_| delay_ms(device, ch));
                // A new fader for a new range or unit (scale and labels).
                let fader = move || {
                    let (r, u) = (range.get(), unit.get());
                    let marks: Vec<f64> = (0..=10).map(|i| r * i as f64 / 10.0).collect();
                    let label = Callback::new(move |v: f64| {
                        let i = (v / r * 10.0).round() as i64;
                        if i % 5 == 0 { u.format(v) } else { String::new() }
                    });
                    view! {
                        <Fader value=Signal::derive(move || ms.get().clamp(0.0, r)) taper=Taper::Linear(0.0, r) marks=marks
                            major=vec![0.0] label=label color=color step=Signal::derive(move || r / 200.0) reset=0.0
                            on_change=Callback::new(move |v: f64| device.set_delay_ms(ch, v.clamp(0.0, max_ms())))/>
                    }
                };
                let field = move || {
                    let u = unit.get();
                    view! {
                        <ValueField text=Signal::derive(move || u.format(ms.get()))
                            title=format!("Delay in {} (max. {})", u.suffix(), u.format_range(max_ms()))
                            on_submit=Callback::new(move |input: String| match parse_number(&input) {
                                Some(v) if v >= 0.0 => {
                                    // A typed unit wins over the shown one.
                                    let typed = typed_unit(&input).map_or(u, Unit);
                                    device.set_delay_ms(ch, typed.to_ms(v).clamp(0.0, max_ms()));
                                    true
                                }
                                _ => false,
                            })/>
                    }
                };
                view! {
                    <div class="delay-column">
                        <div class="delay-name" style=format!("color:{color}")>{move || device.alias(ch)}</div>
                        <div class="fader-box">{fader}</div>
                        {field}
                        <div class="delay-suffix">{move || unit.get().suffix()}</div>
                    </div>
                }
            })
            .collect_view()
    };
    view! {
        <PanelCard class="delay-faders">
            <div class="delay-columns">{columns}</div>
        </PanelCard>
    }
}
