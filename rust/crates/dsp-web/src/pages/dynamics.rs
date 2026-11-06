//! Gate (inputs), compressor and limiter (outputs), each on its own page:
//! channel selection, transfer curve (input level → output level) and the
//! parameters as horizontal faders below.
//!
//! The value ranges are those of the original editor: attack 1 … 999 ms,
//! release 10 … 3000 ms, gate hold 10 … 999 ms, knee 0 … 12 dB.
//!
//! The Limit page of a real DSP also has the limiter calculator and the
//! settings of the limit light page below the parameters.

use dsp_core::limiter_calc::LimiterSetup;
use dsp_core::protocol as p;
use dsp_core::{CompressorState, GateState, LimiterState};
use leptos::html;
use leptos::prelude::*;

use crate::components::common::{ChannelSelector, Check, PanelCard};
use crate::components::fader::Fader;
use crate::components::size::use_size;
use crate::components::value_field::ValueField;
use crate::fmt::{fixed, format_db_value, parse_number};
use crate::icons::{I, Icon};
use crate::meters::Led;
use crate::pages::dsp::ui;
use crate::pages::limiter_calc::LimiterCalcCard;
use crate::store::{Device, app, filter_of};
use crate::taper::Taper;

// Times in ms (raw + 1) of the original editor.
const ATTACK_MS: (i64, i64) = (1, 999);
const RELEASE_MS: (i64, i64) = (10, 3000);
const HOLD_MS: (i64, i64) = (10, 999);

fn format_ms(ms: i64) -> String {
    if ms >= 1000 { format!("{} s", fixed(ms as f64 / 1000.0, 2)) } else { format!("{ms} ms") }
}

fn threshold_label(db: f64) -> String {
    if db % 20.0 == 0.0 { format!("{}", db.round() as i64) } else { String::new() }
}

fn time_label(ms: f64) -> String {
    if ms >= 1000.0 { format!("{}s", (ms / 1000.0).round() as i64) } else { format!("{}", ms.round() as i64) }
}

fn knee_label(k: f64) -> String {
    if k % 3.0 == 0.0 { format!("{}", k.round() as i64) } else { String::new() }
}

/// "1.5 s" or "1.5s" (but not "ms") means seconds.
fn typed_in_seconds(input: &str) -> bool {
    let lower = input.to_lowercase();
    if lower.contains("ms") {
        return false;
    }
    let chars: Vec<char> = lower.chars().collect();
    chars.iter().enumerate().any(|(i, c)| {
        *c == 's'
            && chars[..i].iter().rev().find(|c| !c.is_whitespace()).is_some_and(|c| c.is_ascii_digit())
            && chars.get(i + 1).is_none_or(|n| !n.is_alphanumeric())
    })
}

/// Ratio (n in 1:n) of a ratio index; "Limit" is infinite.
fn ratio_value(index: i64) -> f64 {
    let name = p::COMPRESSOR_RATIO_NAMES.get(index as usize).copied().unwrap_or("1:1");
    if name == "Limit" {
        return f64::INFINITY;
    }
    name.rsplit(':').next().and_then(|n| n.parse().ok()).unwrap_or(1.0)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Gate,
    Compressor,
    Limiter,
}

#[derive(Clone, Copy)]
enum Field {
    Threshold,
    Attack,
    Hold,
    Release,
    Knee,
    Ratio,
}

/// The settings of one channel's gate, compressor or limiter.
#[derive(Clone, PartialEq)]
enum Dyn {
    Gate(GateState),
    Comp(CompressorState),
    Limit(LimiterState),
}

impl Dyn {
    fn of(device: Device, kind: Kind, ch: &str) -> Dyn {
        device.state.with(|s| match kind {
            Kind::Gate => Dyn::Gate(s.gates.get(ch).cloned().unwrap_or_default()),
            Kind::Compressor => Dyn::Comp(s.compressors.get(ch).cloned().unwrap_or_default()),
            Kind::Limiter => Dyn::Limit(s.limiters.get(ch).cloned().unwrap_or_default()),
        })
    }

    fn get(&self, f: Field) -> i64 {
        match (self, f) {
            (Dyn::Gate(g), Field::Threshold) => g.threshold,
            (Dyn::Gate(g), Field::Attack) => g.attack,
            (Dyn::Gate(g), Field::Hold) => g.hold,
            (Dyn::Gate(g), Field::Release) => g.release,
            (Dyn::Comp(c), Field::Threshold) => c.threshold,
            (Dyn::Comp(c), Field::Attack) => c.attack,
            (Dyn::Comp(c), Field::Release) => c.release,
            (Dyn::Comp(c), Field::Knee) => c.knee,
            (Dyn::Comp(c), Field::Ratio) => c.ratio,
            (Dyn::Limit(l), Field::Threshold) => l.threshold,
            (Dyn::Limit(l), Field::Attack) => l.attack,
            (Dyn::Limit(l), Field::Release) => l.release,
            (Dyn::Limit(l), Field::Knee) => l.knee,
            _ => 0,
        }
    }

    fn set(&mut self, f: Field, v: i64) {
        let slot = match (self, f) {
            (Dyn::Gate(g), Field::Threshold) => &mut g.threshold,
            (Dyn::Gate(g), Field::Attack) => &mut g.attack,
            (Dyn::Gate(g), Field::Hold) => &mut g.hold,
            (Dyn::Gate(g), Field::Release) => &mut g.release,
            (Dyn::Comp(c), Field::Threshold) => &mut c.threshold,
            (Dyn::Comp(c), Field::Attack) => &mut c.attack,
            (Dyn::Comp(c), Field::Release) => &mut c.release,
            (Dyn::Comp(c), Field::Knee) => &mut c.knee,
            (Dyn::Comp(c), Field::Ratio) => &mut c.ratio,
            (Dyn::Limit(l), Field::Threshold) => &mut l.threshold,
            (Dyn::Limit(l), Field::Attack) => &mut l.attack,
            (Dyn::Limit(l), Field::Release) => &mut l.release,
            (Dyn::Limit(l), Field::Knee) => &mut l.knee,
            _ => return,
        };
        *slot = v;
    }

    fn default_of(kind: Kind) -> Dyn {
        match kind {
            Kind::Gate => Dyn::Gate(GateState::default()),
            Kind::Compressor => Dyn::Comp(CompressorState::default()),
            Kind::Limiter => Dyn::Limit(LimiterState::default()),
        }
    }

    fn send(self, device: Device, ch: &str) {
        match self {
            Dyn::Gate(g) => device.set_gate(ch, g),
            Dyn::Comp(c) => device.set_compressor(ch, c),
            Dyn::Limit(l) => device.set_limiter(ch, l),
        }
    }

    fn threshold_db(&self) -> f64 {
        p::threshold_raw_to_db(self.get(Field::Threshold))
    }

    /// Output level for an input level (dB).
    fn response(&self, x: f64) -> f64 {
        let t = self.threshold_db();
        match self {
            Dyn::Gate(_) => {
                if x >= t {
                    x
                } else {
                    x - 60.0
                }
            }
            Dyn::Comp(c) => {
                let ratio = ratio_value(c.ratio);
                if ratio <= 1.0 {
                    return x;
                }
                let (over, knee) = (x - t, c.knee as f64);
                if knee > 0.0 && over.abs() <= knee / 2.0 {
                    // Soft knee: quadratic transition
                    return x + (1.0 / ratio - 1.0) * (over + knee / 2.0).powi(2) / (2.0 * knee);
                }
                if over <= 0.0 { x } else { t + over / ratio }
            }
            Dyn::Limit(l) => {
                let (over, knee) = (x - t, l.knee as f64);
                // Soft knee: quadratic transition into the limit
                if knee > 0.0 && over.abs() <= knee / 2.0 {
                    return x - (over + knee / 2.0).powi(2) / (2.0 * knee);
                }
                x.min(t)
            }
        }
    }

    fn active(&self) -> bool {
        let t = self.threshold_db();
        match self {
            Dyn::Gate(_) => t > -90.0,
            Dyn::Comp(c) => c.ratio != 0 && t < 20.0,
            Dyn::Limit(_) => t < 20.0,
        }
    }
}

// ─── Pages ───

#[component]
pub fn GatePage() -> impl IntoView {
    view! { <DynamicsPage kind=Kind::Gate/> }
}

#[component]
pub fn CompressorPage() -> impl IntoView {
    view! { <DynamicsPage kind=Kind::Compressor/> }
}

#[component]
pub fn LimiterPage() -> impl IntoView {
    view! { <DynamicsPage kind=Kind::Limiter/> }
}

#[component]
fn DynamicsPage(kind: Kind) -> impl IntoView {
    let ui = ui();
    let device = ui.device;
    let model = ui.model;
    let inputs = kind == Kind::Gate;
    let selection = RwSignal::new(None::<String>);
    let channel = Memo::new(move |_| {
        let m = model.get();
        let group = if inputs { m.inputs } else { m.outputs };
        selection.get().filter(|c| group.contains(&c.as_str())).unwrap_or_else(|| group[0].to_string())
    });
    let color = if inputs { "var(--input)" } else { "var(--output)" };
    // The extra cards make the page taller than the screen: it scrolls.
    let page_class = if kind == Kind::Limiter && !device.offline { "dyn-page has-extras" } else { "dyn-page" };
    view! {
        <div class=page_class>
            <ChannelSelector device=device selected=channel inputs=inputs outputs=!inputs
                on_select=Callback::new(move |c: String| selection.set(Some(c)))/>
            {move || {
                let ch = channel.get();
                view! { <DynamicsContent device=device kind=kind channel=ch color=color/> }
            }}
        </div>
    }
}

#[component]
fn DynamicsContent(device: Device, kind: Kind, channel: String, color: &'static str) -> impl IntoView {
    let ch = StoredValue::new(channel);
    let state = Memo::new(move |_| ch.with_value(|c| Dyn::of(device, kind, c)));
    let set = move |f: Field, v: i64| {
        let mut next = state.get_untracked();
        next.set(f, v);
        ch.with_value(|c| next.send(device, c));
    };
    let defaults = Dyn::default_of(kind);
    // Memo: the alias reads the device state, the title changes only with it.
    let title = Memo::new(move |_| {
        let name = match kind {
            Kind::Gate => "Gate",
            Kind::Compressor => "Compressor",
            Kind::Limiter => "Limiter",
        };
        format!("{name} · {}", ch.with_value(|c| device.alias(c)))
    });

    let max_db = if kind == Kind::Gate { 0.0 } else { 20.0 };
    let mut params = vec![threshold_row(state, set, max_db, defaults.get(Field::Threshold), color)];
    params.push(time_row("Attack", Field::Attack, ATTACK_MS, state, set, defaults.get(Field::Attack), color));
    if kind == Kind::Gate {
        params.push(time_row("Hold", Field::Hold, HOLD_MS, state, set, defaults.get(Field::Hold), color));
    }
    params.push(time_row("Release", Field::Release, RELEASE_MS, state, set, defaults.get(Field::Release), color));
    if kind != Kind::Gate {
        params.push(knee_row(state, set, defaults.get(Field::Knee), color));
    }

    let badge = move || {
        let on = state.with(Dyn::active);
        view! {
            <span class="state-badge" class:on=on style=format!("--bc:{color}")>{if on { "Active" } else { "Off" }}</span>
        }
    };
    let side = match kind {
        Kind::Gate => view! {
            {badge}
            <Hint text="Below the threshold the gate closes. -90 dB = always open."/>
        }
        .into_any(),
        Kind::Compressor => {
            let ratio = Memo::new(move |_| state.with(|s| s.get(Field::Ratio)));
            let options = p::COMPRESSOR_RATIO_NAMES
                .iter()
                .enumerate()
                .map(|(i, name)| view! { <option value=i selected=move || ratio.get() == i as i64>{*name}</option> })
                .collect_view();
            view! {
                {badge}
                <ActivityLed device=device channel=ch.get_value() led=Led::Comp/>
                <div class="side-label">"Ratio"</div>
                <select class="ratio-select mono"
                    style=move || format!("color:{}", if ratio.get() == 0 { "var(--muted)" } else { color })
                    on:change=move |ev| {
                        if let Ok(v) = event_target_value(&ev).parse::<i64>() {
                            set(Field::Ratio, v);
                        }
                    }>
                    {options}
                </select>
            }
            .into_any()
        }
        Kind::Limiter => view! {
            {badge}
            <ActivityLed device=device channel=ch.get_value() led=Led::Limit/>
            <Hint text="+20 dB means the limiter never engages. The knee is hidden in the original editor."/>
        }
        .into_any(),
    };
    let extras = (kind == Kind::Limiter && !device.offline).then(|| limit_extras(device, ch, state, set));

    view! {
        <div class="dyn-body">
            <PanelCard title=title accent=color class="dyn-curve-card">
                <div class="dyn-curve-row">
                    <TransferCurve state=state color=color/>
                    <div class="dyn-side">{side}</div>
                </div>
            </PanelCard>
            <PanelCard class="dyn-params">{params}</PanelCard>
            {extras}
        </div>
    }
}

// ─── Limit page extras (real DSPs only) ───

/// Below the limiter parameters: the limiter calculator for this output and
/// the limit light settings.
fn limit_extras(
    device: Device,
    ch: StoredValue<String>,
    state: Memo<Dyn>,
    set: impl Fn(Field, i64) + Copy + Send + Sync + 'static,
) -> AnyView {
    let panel = app().panel;
    let setup = Memo::new(move |_| panel.with(|data| data.limiter.clone()));
    let threshold_raw = Memo::new(move |_| state.with(|s| s.get(Field::Threshold)));
    // Read only when the timing button is pressed (not tracked by a view).
    let low_hz = Signal::derive(move || {
        ch.with_value(|c| {
            device.state.with(|s| {
                let hpf = filter_of(s, c, true);
                if hpf.enabled() { p::peq_freq_to_hz(hpf.freq_raw) } else { 20.0 }
            })
        })
    });
    let on_change = Callback::new(move |setup: LimiterSetup| app().set_limiter_setup(setup));
    let on_threshold = Callback::new(move |raw: i64| set(Field::Threshold, raw));
    // Attack and release in ms, one change of the limiter.
    let on_timing = Callback::new(move |(attack, release): (i64, i64)| {
        let mut next = state.get_untracked();
        next.set(Field::Attack, p::time_ms_to_raw(attack.clamp(ATTACK_MS.0, ATTACK_MS.1)));
        next.set(Field::Release, p::time_ms_to_raw(release.clamp(RELEASE_MS.0, RELEASE_MS.1)));
        ch.with_value(|c| next.send(device, c));
    });
    view! {
        <div class="limit-extras">
            <LimiterCalcCard dsp=device.id() channel=ch.get_value() setup=setup on_change=on_change
                threshold_raw=threshold_raw on_threshold=on_threshold low_hz=low_hz on_timing=on_timing/>
            <LimitLightCard device=device/>
        </div>
    }
    .into_any()
}

/// Settings of the limit light page (/limit, without login): the page on or
/// off, and which outputs of this DSP it watches.
#[component]
fn LimitLightCard(device: Device) -> impl IntoView {
    let panel = app().panel;
    let model = ui().model;
    let dsp = StoredValue::new(device.id());
    let light = Memo::new(move |_| panel.with(|data| data.light.clone()));
    let enabled = Memo::new(move |_| light.with(|l| l.enabled));
    let url = web_sys::window()
        .and_then(|w| w.location().origin().ok())
        .map_or_else(|| "/limit".to_string(), |origin| format!("{origin}/limit"));

    let set_enabled = move |ev: web_sys::Event| {
        let mut next = light.get_untracked();
        next.enabled = event_target_checked(&ev);
        app().set_light_settings(next);
    };
    // Watched = not in the ignore list of this DSP.
    let toggle = move |ch: &'static str| {
        let mut next = light.get_untracked();
        let id = dsp.get_value();
        let order = model.get_untracked().outputs;
        let list = next.ignore.entry(id.clone()).or_default();
        match list.iter().position(|c| c == ch) {
            Some(i) => {
                list.remove(i);
            }
            None => list.push(ch.to_string()),
        }
        list.sort_by_key(|c| order.iter().position(|o| *o == c.as_str()));
        if list.is_empty() {
            next.ignore.remove(&id);
        }
        app().set_light_settings(next);
    };
    let outputs = move || {
        model
            .get()
            .outputs
            .iter()
            .map(|&ch| {
                let watched = Memo::new(move |_| {
                    dsp.with_value(|d| light.with(|l| !l.ignore.get(d).is_some_and(|v| v.iter().any(|c| c == ch))))
                });
                let label = Memo::new(move |_| device.alias(ch));
                let on_toggle = Callback::new(move |_| toggle(ch));
                view! { <Check checked=watched label=label color="var(--output)" on_toggle=on_toggle/> }
            })
            .collect_view()
    };

    view! {
        <PanelCard title=Signal::derive(|| "Limit light".to_string()) accent="var(--warning)" class="light-card"
            trailing=view! {
                <label class="light-switch" title="Serve the limit light page">
                    <input type="checkbox" role="switch" aria-label="Limit light page"
                        prop:checked=move || enabled.get() on:change=set_enabled/>
                    <span class="light-switch-track"></span>
                </label>
            }.into_any()>
            <div class="light-body">
                <Show when=move || enabled.get()
                    fallback=|| view! { <div class="small muted">"The page is off."</div> }>
                    <a class="light-link mono" href="/limit" target="_blank" rel="noopener" data-view-ok>{url.clone()}</a>
                </Show>
                <div class="light-help">
                    "A page without login, e.g. for the DJ booth: one lamp that lights while a watched limiter acts. Muted outputs never count."
                </div>
                <div class="small muted">"Watched outputs"</div>
                <div class="light-outputs">{outputs}</div>
            </div>
        </PanelCard>
    }
}

// ─── Parameters ───

fn param_row(label: &'static str, fader: AnyView, text: Signal<String>, on_submit: Callback<String, bool>) -> AnyView {
    view! {
        <div class="param-row">
            <span class="param-label">{label}</span>
            <div class="param-fader">{fader}</div>
            <ValueField text=text on_submit=on_submit width=100 height=30 font=12.0/>
        </div>
    }
    .into_any()
}

fn threshold_row(
    state: Memo<Dyn>,
    set: impl Fn(Field, i64) + Copy + Send + Sync + 'static,
    max_db: f64,
    default_raw: i64,
    color: &'static str,
) -> AnyView {
    let db = Memo::new(move |_| state.with(Dyn::threshold_db));
    let put = move |v: f64| set(Field::Threshold, p::threshold_db_to_raw(v.clamp(-90.0, max_db)));
    let marks: Vec<f64> = (0..).map(|i| -90.0 + 10.0 * i as f64).take_while(|db| *db <= max_db).collect();
    let fader = view! {
        <Fader vertical=false value=db taper=Taper::Linear(-90.0, max_db) marks=marks major=vec![0.0]
            label={threshold_label as fn(f64) -> String} color=color step=Signal::derive(|| 0.5)
            reset=p::threshold_raw_to_db(default_raw) on_change=Callback::new(put)/>
    }
    .into_any();
    param_row(
        "Threshold",
        fader,
        Signal::derive(move || format!("{} dB", format_db_value(db.get()))),
        Callback::new(move |input: String| match parse_number(&input) {
            Some(v) => {
                put(v);
                true
            }
            None => false,
        }),
    )
}

fn time_row(
    label: &'static str,
    field: Field,
    (min_ms, max_ms): (i64, i64),
    state: Memo<Dyn>,
    set: impl Fn(Field, i64) + Copy + Send + Sync + 'static,
    default_raw: i64,
    color: &'static str,
) -> AnyView {
    let ms = Memo::new(move |_| state.with(|s| p::time_raw_to_ms(s.get(field))));
    let put = move |ms: f64| set(field, p::time_ms_to_raw((ms.round() as i64).clamp(min_ms, max_ms)));
    let marks: Vec<f64> = [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0]
        .into_iter()
        .filter(|m| *m >= min_ms as f64 && *m <= max_ms as f64)
        .collect();
    let fader = view! {
        <Fader vertical=false value=Signal::derive(move || ms.get().clamp(min_ms, max_ms) as f64)
            taper=Taper::Log(min_ms as f64, max_ms as f64) marks=marks
            label={time_label as fn(f64) -> String} color=color
            reset=p::time_raw_to_ms(default_raw) as f64 on_change=Callback::new(put)/>
    }
    .into_any();
    param_row(
        label,
        fader,
        Signal::derive(move || format_ms(ms.get())),
        Callback::new(move |input: String| match parse_number(&input) {
            Some(v) => {
                put(if typed_in_seconds(&input) { v * 1000.0 } else { v });
                true
            }
            None => false,
        }),
    )
}

/// Knee of the compressor or limiter, 0 … 12 dB.
fn knee_row(
    state: Memo<Dyn>,
    set: impl Fn(Field, i64) + Copy + Send + Sync + 'static,
    default_knee: i64,
    color: &'static str,
) -> AnyView {
    let knee = Memo::new(move |_| state.with(|s| s.get(Field::Knee)));
    let put = move |v: f64| set(Field::Knee, (v.round() as i64).clamp(0, p::KNEE_MAX));
    let fader = view! {
        <Fader vertical=false value=Signal::derive(move || knee.get() as f64) taper=Taper::Linear(0.0, p::KNEE_MAX as f64)
            marks={(0..=p::KNEE_MAX).map(|k| k as f64).collect::<Vec<_>>()} label={knee_label as fn(f64) -> String} color=color
            step=Signal::derive(|| 1.0) reset=default_knee as f64 on_change=Callback::new(put)/>
    }
    .into_any();
    param_row(
        "Knee",
        fader,
        Signal::derive(move || format!("{} dB", knee.get())),
        Callback::new(move |input: String| match parse_number(&input) {
            Some(v) => {
                put(v);
                true
            }
            None => false,
        }),
    )
}

// ─── Side panel ───

#[component]
fn Hint(text: &'static str) -> impl IntoView {
    view! {
        <div class="hint">
            <Icon icon=I::Info size=13/>
            <span>{text}</span>
        </div>
    }
}

/// Lights while the DSP reports the compressor or limiter of [channel]
/// reducing the gain (status word of the meter answer). Written by the meter
/// bank directly, nothing re-renders.
#[component]
fn ActivityLed(device: Device, channel: String, led: Led) -> impl IntoView {
    let el = NodeRef::<html::Div>::new();
    let registration = StoredValue::new(None::<u64>);
    Effect::new(move |_| {
        let Some(node) = el.get() else { return };
        let id = device.meters.try_update_value(|bank| bank.register_led(&channel, node.into(), led));
        registration.set_value(id);
    });
    on_cleanup(move || {
        if let Some(id) = registration.try_get_value().flatten() {
            device.meters.try_update_value(|bank| bank.unregister(id));
        }
    });
    view! {
        <div class="activity" node_ref=el>
            <span class="activity-dot"></span>
            <span class="when-on">"Reducing gain"</span>
            <span class="when-off">"Not reducing gain"</span>
        </div>
    }
}

// ─── Transfer curve ───

const CURVE_MIN: f64 = -90.0;
const CURVE_MAX: f64 = 20.0;

/// Input level (x) against output level (y), −90 … +20 dB.
#[component]
fn TransferCurve(state: Memo<Dyn>, color: &'static str) -> impl IntoView {
    let container = NodeRef::<html::Div>::new();
    let size = use_size(container);
    let drawing = move || {
        let (w, h) = size.get();
        if w < 20.0 || h < 20.0 {
            return ().into_any();
        }
        let x = move |db: f64| (db - CURVE_MIN) / (CURVE_MAX - CURVE_MIN) * w;
        let y = move |db: f64| h - (db.clamp(CURVE_MIN, CURVE_MAX) - CURVE_MIN) / (CURVE_MAX - CURVE_MIN) * h;
        let grid = (0..=10)
            .map(|i| {
                let db = -80.0 + 10.0 * i as f64;
                let major = db % 20.0 == 0.0;
                let opacity = if major { 1.0 } else { 0.35 };
                view! {
                    <line x1=x(db) y1="0" x2=x(db) y2=h stroke="var(--border)" stroke-opacity=opacity/>
                    <line x1="0" y1=y(db) x2=w y2=y(db) stroke="var(--border)" stroke-opacity=opacity/>
                    {major.then(|| view! {
                        <text class="curve-label" x=x(db) + 2.0 y=h - 2.0>{format!("{}", db as i64)}</text>
                        <text class="curve-label" x="2" y=y(db) - 2.0>{format!("{}", db as i64)}</text>
                    })}
                }
            })
            .collect_view();
        let curve = move || {
            state.with(|s| {
                let t = s.threshold_db().clamp(CURVE_MIN, CURVE_MAX);
                let mut path = String::with_capacity(221 * 14);
                for step in 0..=220 {
                    let input = CURVE_MIN + (CURVE_MAX - CURVE_MIN) * step as f64 / 220.0;
                    path.push_str(&format!(
                        "{}{:.1},{:.1}",
                        if step == 0 { "M" } else { "L" },
                        x(input),
                        y(s.response(input))
                    ));
                }
                let (kx, ky) = (x(t), y(s.response(t)));
                view! {
                    <line x1=kx y1="0" x2=kx y2=h stroke=color stroke-opacity="0.35"/>
                    <path d=path fill="none" stroke=color stroke-width="2.5" stroke-linejoin="round"/>
                    <circle cx=kx cy=ky r="9" fill=color fill-opacity="0.24"/>
                    <circle cx=kx cy=ky r="6" fill=color stroke="var(--bg-deep)" stroke-width="1.5"/>
                }
            })
        };
        view! {
            <svg class="layer">
                {grid}
                <line x1=x(CURVE_MIN) y1=y(CURVE_MIN) x2=x(CURVE_MAX) y2=y(CURVE_MAX) stroke="var(--muted)" stroke-opacity="0.43"/>
                {curve}
                <text class="curve-title" x="4" y="11">"in → out"</text>
            </svg>
        }
        .into_any()
    };
    view! {
        <div class="transfer">
            <div class="transfer-inner" node_ref=container>{drawing}</div>
        </div>
    }
}
