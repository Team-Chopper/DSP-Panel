//! EQ page (parametric EQ of every channel): channel selection, the PEQ graph
//! over the full width (magnitude or phase, optionally compared with another
//! channel), then the band table with the Freq/Q/Gain faders of the selected
//! band, HPF and LPF, and the channel gain. A filter settings file of a
//! measurement program can be taken over as a proposal.

use dsp_core::eq_math::{
    ChannelEq, filter_phase_deg, filter_response_db, peq_band_audible, peq_phase_deg, peq_response_db, wrap_deg,
};
use dsp_core::filter_file::{self, FileBands};
use dsp_core::protocol as p;
use dsp_core::{FilterState, PeqBand};
use leptos::html;
use leptos::prelude::*;
use web_sys::{PointerEvent, WheelEvent};

use crate::components::common::{ChannelSelector, PanelCard, Segmented};
use crate::components::fader::Fader;
use crate::components::size::use_size;
use crate::components::value_field::ValueField;
use crate::fmt::{db_mark_label, format_db_value, format_hz, parse_frequency, parse_number};
use crate::icons::{I, Icon};
use crate::local_settings;
use crate::pages::auto_eq::{AutoEqPanel, AutoEqTarget, PROPOSAL_COLOR};
use crate::pages::dsp::ui;
use crate::pages::gain::ChannelStrip;
use crate::store::{Device, PeqEdit, alias_in, app, filter_of};
use crate::taper::Taper;

/// One colour per PEQ band (9 bands on outputs).
pub const PEQ_BAND_COLORS: [&str; 9] =
    ["#F87171", "#4ADE80", "#38BDF8", "#FBBF24", "#A78BFA", "#FDE68A", "#F472B6", "#2DD4BF", "#FB923C"];

pub fn band_color(i: usize) -> &'static str {
    PEQ_BAND_COLORS[i % PEQ_BAND_COLORS.len()]
}

pub const PEQ_GAIN_MAX: f64 = 12.0;

const FREQ_MARK_HZ: [f64; 10] = [20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0, 20000.0];

pub fn freq_marks() -> Vec<f64> {
    FREQ_MARK_HZ.iter().map(|hz| p::peq_hz_to_freq(*hz) as f64).collect()
}

/// Nominal frequency of a scale mark (20, 50, … 20k).
pub fn freq_label(raw: f64) -> String {
    let hz = FREQ_MARK_HZ
        .iter()
        .find(|hz| p::peq_hz_to_freq(**hz) as f64 == raw)
        .copied()
        .unwrap_or_else(|| p::peq_freq_to_hz(raw.round() as i64));
    if hz >= 1000.0 { format!("{}k", (hz / 1000.0).round() as i64) } else { format!("{}", hz.round() as i64) }
}

fn q_marks() -> Vec<f64> {
    [0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0].iter().map(|q| p::peq_q_to_raw(*q) as f64).collect()
}

fn q_label(raw: f64) -> String {
    let q = p::peq_raw_to_q(raw.round() as i64);
    if q >= 1.0 { format!("{}", q.round() as i64) } else { crate::fmt::fixed(q, 1) }
}

fn peq_gain_label(db: f64) -> String {
    if db % 6.0 == 0.0 { db_mark_label(db) } else { String::new() }
}

const PEQ_GAIN_MARKS: [f64; 9] = [12.0, 9.0, 6.0, 3.0, 0.0, -3.0, -6.0, -9.0, -12.0];

fn round_gain(db: f64) -> f64 {
    ((db * 10.0).round() / 10.0).clamp(-PEQ_GAIN_MAX, PEQ_GAIN_MAX)
}

#[component]
pub fn EqPage() -> impl IntoView {
    let ui = ui();
    let device = ui.device;
    let model = ui.model;
    let channel = RwSignal::new(ui.eq_channel.get_untracked().unwrap_or_else(|| model.get_untracked().inputs[0].to_string()));
    // Keep the channel valid when the model changes.
    Effect::new(move |_| {
        let m = model.get();
        if !m.has_channel(&channel.get_untracked()) {
            channel.set(m.inputs[0].to_string());
        }
    });
    let band = RwSignal::new(0usize);
    let auto_eq = RwSignal::new(false);
    let proposal = RwSignal::new(None::<Vec<PeqBand>>);
    let import = RwSignal::new(None::<FileImport>);
    let phase_view = RwSignal::new(local_settings::get_string("eqGraph").as_deref() == Some("phase"));
    let compare = RwSignal::new(None::<String>);
    Effect::new(move |_| {
        let (m, ch) = (model.get(), channel.get());
        if compare.get_untracked().is_some_and(|c| c == ch || !m.has_channel(&c)) {
            compare.set(None);
        }
    });

    let color = move || if model.get().is_input(&channel.get()) { "var(--input)" } else { "var(--output)" };
    let band_count = Memo::new(move |_| {
        let ch = channel.get();
        device.state.with(|s| s.peq_bands.get(&ch).map_or(0, Vec::len))
    });
    Effect::new(move |_| {
        if band.get_untracked() >= band_count.get() {
            band.set(0);
        }
    });
    let eq_bypass = Memo::new(move |_| {
        let ch = channel.get();
        device.state.with(|s| s.eq_bypass.get(&ch).copied().unwrap_or(false))
    });

    let peq_title = Signal::derive(move || {
        let ch = channel.get();
        format!("PEQ · {}", device.state.with(|s| alias_in(s, &ch)))
    });
    let trailing = view! {
        <span class="muted" style="font-size:11px">{move || format!("{} bands", band_count.get())}</span>
        <SmallToggle label="EQ Bypass" active=eq_bypass color="var(--danger)" width=72
            title="Bypass the whole PEQ of this channel (like \"EQ Bypass\" in the original editor)"
            on_click=Callback::new(move |_| {
                let ch = channel.get_untracked();
                device.set_eq_bypass(&ch, !eq_bypass.get_untracked());
            })/>
        <button class="btn text compact" class:warn=move || auto_eq.get() on:click=move |_| {
            auto_eq.update(|a| *a = !*a);
            import.set(None);
            proposal.set(None);
        }>
            <Icon icon=I::Tune size=16/>"Auto-EQ"
        </button>
    }
    .into_any();

    let open_file = move |ev: leptos::ev::Event| {
        let input = event_target::<web_sys::HtmlInputElement>(&ev);
        let Some(file) = input.files().and_then(|f| f.get(0)) else { return };
        input.set_value("");
        let ch = channel.get_untracked();
        leptos::task::spawn_local(async move {
            let name = file.name();
            let text = match wasm_bindgen_futures::JsFuture::from(file.text()).await {
                Ok(v) => v.as_string().unwrap_or_default(),
                Err(_) => {
                    app().toast(format!("{name} could not be read"), true);
                    return;
                }
            };
            let filters = filter_file::parse(&text);
            if filters.is_empty() {
                app().toast(format!("No filter lines (\"Filter 1: ON PK Fc …\") in {name}"), true);
                return;
            }
            let current = device.state.with_untracked(|s| s.peq_bands.get(&ch).cloned().unwrap_or_default());
            let result = filter_file::to_bands(&filters, &current);
            auto_eq.set(false);
            proposal.set(Some(result.bands.clone()));
            import.set(Some(FileImport { name, channel: ch, result }));
        });
    };
    let close_import = Callback::new(move |_| {
        import.set(None);
        proposal.set(None);
    });
    let compare_options = move || {
        let ch = channel.get();
        let m = model.get();
        device.state.with(|s| {
            m.channels()
                .filter(|c| *c != ch)
                .map(|c| {
                    let name = alias_in(s, c);
                    view! { <option value=c selected=move || compare.get().as_deref() == Some(c)>{name}</option> }
                })
                .collect_view()
        })
    };
    let tools = view! {
        <div class="eq-graph-tools">
            <span data-view-ok>
                <Segmented labels=vec!["Magnitude", "Phase"] selected=Signal::derive(move || phase_view.get() as usize)
                    on_select=Callback::new(move |i: usize| {
                        phase_view.set(i == 1);
                        local_settings::set_string("eqGraph", if i == 1 { "phase" } else { "magnitude" });
                    })/>
            </span>
            <span class="small muted">"Compare"</span>
            <select class="select" data-view-ok title="Draw the curve of another channel (dashed)" on:change=move |ev| {
                let v = event_target_value(&ev);
                compare.set((!v.is_empty()).then_some(v));
            }>
                <option value="" selected=move || compare.with(Option::is_none)>"—"</option>
                {compare_options}
            </select>
            <span class="eq-graph-note">
                {move || phase_view.get().then_some(
                    "Phase of PEQ, all-passes, HPF/LPF and polarity, without delay and GEQ. The compared channel includes the delay difference.",
                )}
            </span>
            <label class="btn text compact file-button" title="Take over a filter settings text file (\"Filter 1: ON PK Fc 63 Hz Gain -5 dB Q 4\", REW / Equalizer APO format) as a proposal">
                <Icon icon=I::Download size=16/>"Filter file …"
                <input type="file" accept=".txt,text/plain" on:change=open_file/>
            </label>
        </div>
    };

    view! {
        <div class="eq-page" style=move || format!("--chc:{}", color())>
            <ChannelSelector device=device selected=channel on_select=Callback::new(move |c: String| {
                if c != channel.get_untracked() {
                    proposal.set(None);
                    import.set(None);
                }
                channel.set(c);
            })/>
            {tools}
            <Show when=move || import.with(Option::is_some)>
                <div class="auto-eq-wrap">
                    {move || import.get().map(|i| view! { <FilterFilePanel import=i on_close=close_import/> })}
                </div>
            </Show>
            <Show when=move || auto_eq.get()>
                <div class="auto-eq-wrap">
                    {move || {
                        let ch = channel.get();
                        view! {
                            <AutoEqPanel channel=ch target=AutoEqTarget::Peq
                                on_peq_proposal=Callback::new(move |p: Option<Vec<PeqBand>>| proposal.set(p))
                                on_close=Callback::new(move |_| {
                                    auto_eq.set(false);
                                    proposal.set(None);
                                })/>
                        }
                    }}
                </div>
            </Show>
            <div class="peq-graph-box">
                <PeqGraph device=device channel=channel selected=band proposal=proposal phase=phase_view compare=compare/>
            </div>
            <div class="eq-bottom-scroll">
                <div class="eq-bottom">
                    <PanelCard title=peq_title accent="var(--chc)" trailing=trailing class="peq-card">
                        <div class="peq-card-body">
                            <BandTable device=device channel=channel selected=band/>
                            <BandFaders device=device channel=channel band=band/>
                        </div>
                    </PanelCard>
                    <PanelCard title=Signal::derive(|| "Filters".to_string())>
                        <div class="filters">
                            <FilterColumn device=device channel=channel high_pass=true/>
                            <div class="vsep"></div>
                            <FilterColumn device=device channel=channel high_pass=false/>
                        </div>
                    </PanelCard>
                    <PanelCard title=Signal::derive(|| "Gain".to_string()) class="gain-card">
                        {move || {
                            let ch = channel.get();
                            let c = if model.get_untracked().is_input(&ch) { "var(--input)" } else { "var(--output)" };
                            view! { <ChannelStrip channel=ch color=c show_name=false/> }
                        }}
                    </PanelCard>
                </div>
            </div>
        </div>
    }
}

/// Small labelled toggle ("Bypass").
#[component]
pub fn SmallToggle(
    label: &'static str,
    #[prop(into)] active: Signal<bool>,
    color: &'static str,
    #[prop(default = 56)] width: u32,
    #[prop(optional)] title: &'static str,
    on_click: Callback<()>,
) -> impl IntoView {
    view! {
        <button class="small-toggle" class:on=move || active.get() style=format!("--tc:{color};width:{width}px")
            title=title on:click=move |_| on_click.run(())>
            {label}
        </button>
    }
}

// ─── Filter file ───

/// A filter settings file mapped onto the PEQ of a channel.
#[derive(Clone, Debug, PartialEq)]
struct FileImport {
    name: String,
    channel: String,
    result: FileBands,
}

/// The file's filters as a proposal (orange curve) with what could not be
/// taken over exactly; "Apply" writes the changed bands.
#[component]
fn FilterFilePanel(import: FileImport, on_close: Callback<()>) -> impl IntoView {
    let device = ui().device;
    let FileImport { name, channel, result } = import;
    let alias = device.alias(&channel);
    let count = result.bands.len();
    let summary = if result.used == 0 {
        "No usable filter in the file.".to_string()
    } else {
        format!(
            "{} filter{} → band{} 1{}; the other bands become flat (Peak, 0 dB). Proposal shown as orange curve.",
            result.used,
            if result.used == 1 { "" } else { "s" },
            if result.used == 1 { "" } else { "s" },
            if result.used == 1 { String::new() } else { format!("–{}", result.used) },
        )
    };
    let notes = result.notes.iter().map(|n| view! { <li>{n.clone()}</li> }).collect_view();
    let has_notes = !result.notes.is_empty();
    let bands = StoredValue::new(result.bands);
    let ch = StoredValue::new(channel);
    let apply = move |_| {
        let c = ch.get_value();
        let current = device.state.with_untracked(|s| s.peq_bands.get(&c).cloned().unwrap_or_default());
        for (i, b) in bands.get_value().iter().enumerate() {
            if current.get(i) != Some(b) {
                let edit = PeqEdit {
                    gain_db: Some(b.gain_db),
                    freq_raw: Some(b.freq_raw),
                    q_raw: Some(b.q_raw),
                    peq_type: Some(b.peq_type),
                    bypass: Some(b.bypass),
                };
                device.set_peq_band(&c, i, edit);
            }
        }
        app().toast(format!("Filter file applied to the PEQ of {}", device.alias(&c)), false);
        on_close.run(());
    };
    view! {
        <div class="auto-eq">
            <div class="auto-eq-head">
                <Icon icon=I::Download size=16 color="var(--warning)"/>
                <span class="auto-eq-title">{format!("Filter file · {name} → {alias} ({count} bands)")}</span>
                <span class="auto-eq-step">{summary}</span>
                <button class="icon-button small" title="Close" on:click=move |_| on_close.run(())>
                    <Icon icon=I::Close size=16/>
                </button>
            </div>
            {has_notes.then(|| view! { <ul class="filter-file-notes">{notes}</ul> })}
            <div class="wrap-row">
                <button class="btn compact" style="background:var(--warning);color:var(--bg-deep)"
                    prop:disabled=result.used == 0 on:click=apply>
                    <Icon icon=I::Check size=18/>{format!("Apply to {alias}")}
                </button>
                <button class="btn text compact" on:click=move |_| on_close.run(())>"Discard"</button>
            </div>
        </div>
    }
}

// ─── Band table ───

/// Short type names for the narrow type column.
const SHORT_TYPE_NAMES: [&str; 9] = ["Peak", "Lo Shelf", "Hi Shelf", "LP 6", "LP 12", "HP 6", "HP 12", "AP 1", "AP 2"];

#[component]
fn BandTable(device: Device, channel: RwSignal<String>, selected: RwSignal<usize>) -> impl IntoView {
    let rows = move || {
        let ch = channel.get();
        let count = device.state.with(|s| s.peq_bands.get(&ch).map_or(0, Vec::len));
        (0..count).map(|i| view! { <BandRow device=device channel=ch.clone() index=i selected=selected/> }).collect_view()
    };
    view! {
        <div class="band-table">
            <div class="band-head">
                <span style="width:30px">"#"</span>
                <span style="width:118px">"TYPE"</span>
                <span style="width:82px">"FREQ Hz"</span>
                <span style="width:82px">"GAIN dB"</span>
                <span style="width:82px">"Q"</span>
            </div>
            {rows}
        </div>
    }
}

#[component]
fn BandRow(device: Device, channel: String, index: usize, selected: RwSignal<usize>) -> impl IntoView {
    let ch = channel.clone();
    let band = Memo::new(move |_| device.state.with(|s| s.peq_bands.get(&ch).and_then(|b| b.get(index)).cloned().unwrap_or_default()));
    let color = band_color(index);
    let active = move || selected.get() == index;
    let edit = move |e: PeqEdit| {
        selected.set(index);
        device.set_peq_band(&channel, index, e);
    };
    let edit2 = edit.clone();
    let edit3 = edit.clone();
    let edit4 = edit.clone();
    let edit5 = edit.clone();
    view! {
        <div class="band-row" class:active=active style=format!("--bc:{color}") on:click=move |_| selected.set(index)>
            <span class="band-index">
                <span class="dot" style=move || format!("background:{}", if band.with(|b| b.bypass) { "var(--muted)" } else { color })></span>
                <span class="mono">{index + 1}</span>
            </span>
            <select class="type-select" on:change=move |ev| {
                if let Ok(t) = event_target_value(&ev).parse::<i64>() {
                    edit(PeqEdit { peq_type: Some(t), ..Default::default() });
                }
            }>
                {(0..p::PEQ_TYPE_NAMES.len()).map(|t| {
                    view! {
                        <option value=t selected=move || band.with(|b| b.peq_type == t as i64) title=p::PEQ_TYPE_NAMES[t]>
                            {SHORT_TYPE_NAMES[t]}
                        </option>
                    }
                }).collect_view()}
            </select>
            <div class="band-value">
                <ValueField height=24 text=Signal::derive(move || format_hz(p::peq_freq_to_hz(band.with(|b| b.freq_raw))))
                    on_submit=Callback::new(move |input: String| match parse_frequency(&input) {
                        Some(hz) if hz > 0.0 => {
                            edit2(PeqEdit { freq_raw: Some(p::peq_hz_to_freq(hz.clamp(20.0, 20000.0))), ..Default::default() });
                            true
                        }
                        _ => false,
                    })/>
            </div>
            <div class="band-value">
                <ValueField height=24 text=Signal::derive(move || format_db_value(band.with(|b| b.gain_db)))
                    on_submit=Callback::new(move |input: String| match parse_number(&input) {
                        Some(db) => {
                            edit3(PeqEdit { gain_db: Some(round_gain(db)), ..Default::default() });
                            true
                        }
                        None => false,
                    })/>
            </div>
            <div class="band-value">
                <ValueField height=24 text=Signal::derive(move || p::format_q(band.with(|b| b.q_raw)).to_string())
                    on_submit=Callback::new(move |input: String| match parse_number(&input) {
                        Some(q) if q > 0.0 => {
                            edit4(PeqEdit { q_raw: Some(p::peq_q_to_raw(q)), ..Default::default() });
                            true
                        }
                        _ => false,
                    })/>
            </div>
            <SmallToggle label="Bypass" active=Signal::derive(move || band.with(|b| b.bypass)) color="var(--danger)"
                on_click=Callback::new(move |_| {
                    let bypass = !band.get_untracked().bypass;
                    edit5(PeqEdit { bypass: Some(bypass), ..Default::default() });
                })/>
        </div>
    }
}

// ─── Faders of the selected band ───

#[component]
fn BandFaders(device: Device, channel: RwSignal<String>, band: RwSignal<usize>) -> impl IntoView {
    // Re-create the faders when the channel, band or Q range changes.
    let key = Memo::new(move |_| {
        let ch = channel.get();
        let b = band.get();
        let t = device.state.with(|s| s.peq_bands.get(&ch).and_then(|l| l.get(b)).map_or(0, |x| x.peq_type));
        (ch, b, p::max_q_raw_for_type(t))
    });
    move || {
        let (ch, b, q_max) = key.get();
        let color = band_color(b);
        let data = {
            let ch = ch.clone();
            Memo::new(move |_| device.state.with(|s| s.peq_bands.get(&ch).and_then(|l| l.get(b)).cloned().unwrap_or_default()))
        };
        let set = {
            let ch = ch.clone();
            move |e: PeqEdit| device.set_peq_band(&ch, b, e)
        };
        let (s1, s2, s3, s4, s5, s6) = (set.clone(), set.clone(), set.clone(), set.clone(), set.clone(), set);
        let q_marks: Vec<f64> = q_marks().into_iter().filter(|m| *m <= q_max as f64).collect();
        view! {
            <div class="band-faders">
                <FaderColumn title="Freq">
                    <div class="fader-box">
                        <Fader value=Signal::derive(move || data.with(|d| d.freq_raw as f64))
                            taper=Taper::Linear(0.0, p::FREQ_RAW_MAX as f64) marks=freq_marks()
                            label={freq_label as fn(f64) -> String} color=color step=Signal::derive(|| 1.0)
                            on_change=Callback::new(move |v: f64| s1(PeqEdit { freq_raw: Some(v.round() as i64), ..Default::default() }))/>
                    </div>
                    <ValueField text=Signal::derive(move || format_hz(p::peq_freq_to_hz(data.with(|d| d.freq_raw))))
                        on_submit=Callback::new(move |input: String| match parse_frequency(&input) {
                            Some(hz) if hz > 0.0 => {
                                s2(PeqEdit { freq_raw: Some(p::peq_hz_to_freq(hz.clamp(20.0, 20000.0))), ..Default::default() });
                                true
                            }
                            _ => false,
                        })/>
                </FaderColumn>
                <FaderColumn title="Q">
                    <div class="fader-box">
                        // Shelves and PEQ low/high-pass types go up to Q 3 only.
                        <Fader value=Signal::derive(move || data.with(|d| d.q_raw.clamp(0, q_max) as f64))
                            taper=Taper::Linear(0.0, q_max as f64) marks=q_marks
                            label={q_label as fn(f64) -> String} color=color step=Signal::derive(|| 1.0)
                            reset=p::peq_q_to_raw(3.0) as f64
                            on_change=Callback::new(move |v: f64| s3(PeqEdit { q_raw: Some(v.round() as i64), ..Default::default() }))/>
                    </div>
                    <ValueField text=Signal::derive(move || p::format_q(data.with(|d| d.q_raw)).to_string())
                        on_submit=Callback::new(move |input: String| match parse_number(&input) {
                            Some(q) if q > 0.0 => {
                                s4(PeqEdit { q_raw: Some(p::peq_q_to_raw(q)), ..Default::default() });
                                true
                            }
                            _ => false,
                        })/>
                </FaderColumn>
                <FaderColumn title="Gain">
                    <div class="fader-box">
                        <Fader value=Signal::derive(move || data.with(|d| d.gain_db))
                            taper=Taper::Linear(-PEQ_GAIN_MAX, PEQ_GAIN_MAX) marks=PEQ_GAIN_MARKS.to_vec() major=vec![0.0]
                            label={peq_gain_label as fn(f64) -> String} color=color step=Signal::derive(|| 0.5) reset=0.0
                            on_change=Callback::new(move |v: f64| s5(PeqEdit { gain_db: Some(round_gain(v)), ..Default::default() }))/>
                    </div>
                    <ValueField text=Signal::derive(move || format_db_value(data.with(|d| d.gain_db)))
                        on_submit=Callback::new(move |input: String| match parse_number(&input) {
                            Some(db) => {
                                s6(PeqEdit { gain_db: Some(round_gain(db)), ..Default::default() });
                                true
                            }
                            None => false,
                        })/>
                </FaderColumn>
            </div>
        }
    }
}

#[component]
pub fn FaderColumn(title: &'static str, children: Children) -> impl IntoView {
    view! {
        <div class="fader-column">
            <div class="fader-column-title">{title}</div>
            {children()}
        </div>
    }
}

// ─── HPF / LPF ───

#[component]
fn FilterColumn(device: Device, channel: RwSignal<String>, high_pass: bool) -> impl IntoView {
    let filter = Memo::new(move |_| {
        let ch = channel.get();
        device.state.with(|s| filter_of(s, &ch, high_pass))
    });
    let slope = Memo::new(move |_| {
        let ch = channel.get();
        device.filter_slope(&ch, high_pass)
    });
    let color = if high_pass { "var(--danger)" } else { "var(--info)" };
    let set_freq = move |raw: i64| device.set_filter(&channel.get_untracked(), high_pass, Some(raw), None, None);
    view! {
        <div class="filter-column">
            <div class="row">
                <span class="filter-name" style=move || format!("color:{}", if filter.with(FilterState::enabled) { color } else { "var(--muted)" })>
                    {if high_pass { "HPF" } else { "LPF" }}
                </span>
                <span class="spacer"></span>
                <SmallToggle label="Bypass" active=Signal::derive(move || !filter.with(FilterState::enabled)) color="var(--danger)"
                    on_click=Callback::new(move |_| {
                        let enabled = filter.get_untracked().enabled();
                        device.set_filter(&channel.get_untracked(), high_pass, None, Some(!enabled), None);
                    })/>
            </div>
            <div class="fader-box" style=move || format!("opacity:{}", if filter.with(FilterState::enabled) { 1.0 } else { 0.55 })>
                <Fader value=Signal::derive(move || filter.with(|f| f.freq_raw as f64))
                    taper=Taper::Linear(0.0, p::FREQ_RAW_MAX as f64) marks=freq_marks()
                    label={freq_label as fn(f64) -> String} color=color step=Signal::derive(|| 1.0)
                    reset=if high_pass { 0.0 } else { p::FREQ_RAW_MAX as f64 }
                    on_change=Callback::new(move |v: f64| set_freq(v.round() as i64))/>
            </div>
            <ValueField text=Signal::derive(move || format_hz(p::peq_freq_to_hz(filter.with(|f| f.freq_raw))))
                title="Cut-off frequency in Hz"
                on_submit=Callback::new(move |input: String| match parse_frequency(&input) {
                    Some(hz) if hz > 0.0 => {
                        set_freq(p::peq_hz_to_freq(hz.clamp(20.0, 20000.0)));
                        true
                    }
                    _ => false,
                })/>
            <select class="slope-select" on:change=move |ev| {
                if let Ok(s) = event_target_value(&ev).parse::<i64>() {
                    device.set_filter_slope(&channel.get_untracked(), s, high_pass);
                }
            }>
                {(1..p::CROSSOVER_SLOPE_NAMES.len()).map(|s| view! {
                    <option value=s selected=move || slope.get() == s as i64>{p::CROSSOVER_SLOPE_NAMES[s]}</option>
                }).collect_view()}
            </select>
        </div>
    }
}

// ─── PEQ graph ───

const MIN_HZ: f64 = 20.0;
const MAX_HZ: f64 = 20000.0;
const RANGE_DB: f64 = 15.0;
const STEPS: usize = 240;
const PAD_LEFT: f64 = 34.0;
const PAD_RIGHT: f64 = 10.0;
const PAD_TOP: f64 = 10.0;
const PAD_BOTTOM: f64 = 22.0;

#[derive(Clone, Copy, PartialEq)]
struct Geometry {
    width: f64,
    height: f64,
}

impl Geometry {
    fn w(&self) -> f64 {
        (self.width - PAD_LEFT - PAD_RIGHT).max(1.0)
    }
    fn h(&self) -> f64 {
        (self.height - PAD_TOP - PAD_BOTTOM).max(1.0)
    }
    fn hz_to_x(&self, hz: f64) -> f64 {
        PAD_LEFT + (hz / MIN_HZ).ln() / (MAX_HZ / MIN_HZ).ln() * self.w()
    }
    fn x_to_hz(&self, x: f64) -> f64 {
        MIN_HZ * (MAX_HZ / MIN_HZ).powf(((x - PAD_LEFT) / self.w()).clamp(0.0, 1.0))
    }
    fn db_to_y(&self, db: f64) -> f64 {
        PAD_TOP + self.h() / 2.0 - db.clamp(-RANGE_DB, RANGE_DB) / RANGE_DB * self.h() / 2.0
    }
    fn y_to_db(&self, y: f64) -> f64 {
        (PAD_TOP + self.h() / 2.0 - y) / (self.h() / 2.0) * RANGE_DB
    }
    fn band_point(&self, b: &PeqBand) -> (f64, f64) {
        (self.hz_to_x(p::peq_freq_to_hz(b.freq_raw)), self.db_to_y(b.gain_db))
    }
    fn path(&self, db: &[f64]) -> String {
        let mut out = String::with_capacity(db.len() * 14);
        for (s, v) in db.iter().enumerate() {
            let x = PAD_LEFT + self.w() * s as f64 / STEPS as f64;
            out.push_str(&format!("{}{:.1},{:.1}", if s == 0 { "M" } else { "L" }, x, self.db_to_y(*v)));
        }
        out
    }
    fn deg_to_y(&self, deg: f64) -> f64 {
        PAD_TOP + self.h() / 2.0 - deg.clamp(-180.0, 180.0) / 180.0 * self.h() / 2.0
    }
    /// Phase curve (−180 … 180°); a wrap starts a new segment instead of a
    /// vertical line.
    fn phase_path(&self, deg: &[f64]) -> String {
        let mut out = String::with_capacity(deg.len() * 14);
        for (s, v) in deg.iter().enumerate() {
            let x = PAD_LEFT + self.w() * s as f64 / STEPS as f64;
            let jump = s == 0 || (v - deg[s - 1]).abs() > 180.0;
            out.push_str(&format!("{}{:.1},{:.1}", if jump { "M" } else { "L" }, x, self.deg_to_y(*v)));
        }
        out
    }
}

#[derive(Clone, PartialEq)]
struct GraphInput {
    bands: Vec<PeqBand>,
    hp: FilterState,
    lp: FilterState,
    hp_preview: i64,
    lp_preview: i64,
    inverted: bool,
}

/// Another channel drawn dashed for comparison: its EQ, its delay minus the
/// delay of the shown channel (ms), its colour.
#[derive(Clone, PartialEq)]
struct Compared {
    eq: ChannelEq,
    delay_ms: f64,
    color: &'static str,
}

#[derive(Clone, PartialEq, Default)]
struct GraphShapes {
    /// (path, colour, selected, dashed preview)
    band_paths: Vec<(String, &'static str, bool, bool)>,
    fill: Option<(String, &'static str)>,
    /// (path, colour, dashed)
    filter_paths: Vec<(String, &'static str, bool)>,
    proposal: Option<String>,
    /// (path, colour) of the compared channel.
    compare: Option<(String, &'static str)>,
    total: String,
    /// (x, y, colour, label, bypass, selected)
    points: Vec<(f64, f64, &'static str, usize, bool, bool)>,
}

fn band_curve(b: &PeqBand, hz: &[f64]) -> Vec<f64> {
    let fc = p::peq_freq_to_hz(b.freq_raw);
    let q = p::peq_raw_to_q(b.q_raw);
    hz.iter().map(|f| peq_response_db(*f, fc, q, b.gain_db, b.peq_type)).collect()
}

fn filter_curve(freq_raw: i64, slope: i64, high: bool, hz: &[f64]) -> Vec<f64> {
    let fc = p::peq_freq_to_hz(freq_raw);
    hz.iter().map(|x| filter_response_db(*x, fc, slope, high)).collect()
}

fn plus(a: &[f64], b: &[f64]) -> Vec<f64> {
    a.iter().zip(b).map(|(x, y)| x + y).collect()
}

/// Total of the audible bands plus the active filters.
fn total(bands: &[PeqBand], curves: &[Vec<f64>], hp: Option<&Vec<f64>>, lp: Option<&Vec<f64>>) -> Vec<f64> {
    let mut sum = vec![0.0; STEPS + 1];
    for (b, c) in bands.iter().zip(curves) {
        if peq_band_audible(b) {
            for (s, v) in sum.iter_mut().zip(c) {
                *s += v;
            }
        }
    }
    for f in [hp, lp].into_iter().flatten() {
        for (s, v) in sum.iter_mut().zip(f) {
            *s += v;
        }
    }
    sum
}

fn graph_hz(g: &Geometry) -> Vec<f64> {
    (0..=STEPS).map(|s| g.x_to_hz(PAD_LEFT + g.w() * s as f64 / STEPS as f64)).collect()
}

/// Phase of [bands] (bypassed ones left out) with the filters and polarity
/// of [input].
fn phase_curve(input: &GraphInput, bands: &[PeqBand], hz: &[f64]) -> Vec<f64> {
    let active: Vec<(f64, f64, f64, i64)> = bands
        .iter()
        .filter(|b| !b.bypass)
        .map(|b| (p::peq_freq_to_hz(b.freq_raw), p::peq_raw_to_q(b.q_raw), b.gain_db, b.peq_type))
        .collect();
    let filters: Vec<(f64, i64, bool)> = [(&input.hp, true), (&input.lp, false)]
        .into_iter()
        .filter(|(f, _)| f.enabled())
        .map(|(f, high)| (p::peq_freq_to_hz(f.freq_raw), f.slope, high))
        .collect();
    hz.iter()
        .map(|f| {
            let mut deg = if input.inverted { 180.0 } else { 0.0 };
            for (fc, q, gain, t) in &active {
                deg += peq_phase_deg(*f, *fc, *q, *gain, *t);
            }
            for (fc, slope, high) in &filters {
                deg += filter_phase_deg(*f, *fc, *slope, *high);
            }
            wrap_deg(deg)
        })
        .collect()
}

fn phase_shapes(g: &Geometry, input: &GraphInput, proposal: Option<&Vec<PeqBand>>, compare: Option<&Compared>) -> GraphShapes {
    let hz = graph_hz(g);
    GraphShapes {
        total: g.phase_path(&phase_curve(input, &input.bands, &hz)),
        proposal: proposal.map(|bands| g.phase_path(&phase_curve(input, bands, &hz))),
        compare: compare.map(|c| {
            let deg: Vec<f64> =
                hz.iter().map(|f| wrap_deg(c.eq.phase_deg(*f) - 360.0 * f * c.delay_ms / 1000.0)).collect();
            (g.phase_path(&deg), c.color)
        }),
        ..GraphShapes::default()
    }
}

fn shapes(
    g: &Geometry,
    input: &GraphInput,
    selected: usize,
    proposal: Option<&Vec<PeqBand>>,
    compare: Option<&Compared>,
) -> GraphShapes {
    let hz = graph_hz(g);
    let bands = &input.bands;
    let curves: Vec<Vec<f64>> = bands.iter().map(|b| band_curve(b, &hz)).collect();
    let hp = input.hp.enabled().then(|| filter_curve(input.hp.freq_raw, input.hp.slope, true, &hz));
    let lp = input.lp.enabled().then(|| filter_curve(input.lp.freq_raw, input.lp.slope, false, &hz));
    let sum = total(bands, &curves, hp.as_ref(), lp.as_ref());
    let mut out = GraphShapes::default();

    for (i, b) in bands.iter().enumerate() {
        let color = band_color(i);
        // Peak/shelf at 0 dB and all-pass bands draw no curve.
        let flat = (b.peq_type <= 2 && b.gain_db.abs() < 0.05) || b.peq_type >= 7;
        if flat {
            continue;
        }
        if b.bypass {
            // Preview of a bypassed band: the total as it would be with the
            // band switched on.
            out.band_paths.push((g.path(&plus(&sum, &curves[i])), color, i == selected, true));
            continue;
        }
        let path = g.path(&curves[i]);
        if i == selected {
            let zero = g.db_to_y(0.0);
            out.fill = Some((
                format!("{path}L{:.1},{zero:.1}L{PAD_LEFT:.1},{zero:.1}Z", PAD_LEFT + g.w()),
                color,
            ));
        }
        out.band_paths.push((path, color, i == selected, false));
    }
    // HPF/LPF: active ones in their colour, bypassed ones as a dashed preview
    // of the total with the filter switched on.
    for (f, high, preview, color) in
        [(&input.hp, true, input.hp_preview, "var(--danger)"), (&input.lp, false, input.lp_preview, "var(--info)")]
    {
        let curve = filter_curve(f.freq_raw, if f.enabled() { f.slope } else { preview }, high, &hz);
        if f.enabled() {
            out.filter_paths.push((g.path(&curve), color, false));
        } else if curve.iter().any(|db| *db < -0.5) {
            out.filter_paths.push((g.path(&plus(&sum, &curve)), color, true));
        }
    }
    if let Some(proposed) = proposal {
        let pc: Vec<Vec<f64>> = proposed.iter().map(|b| band_curve(b, &hz)).collect();
        out.proposal = Some(g.path(&total(proposed, &pc, hp.as_ref(), lp.as_ref())));
    }
    if let Some(c) = compare {
        let db: Vec<f64> = hz.iter().map(|f| c.eq.db(*f)).collect();
        out.compare = Some((g.path(&db), c.color));
    }
    out.total = g.path(&sum);

    // Band points, selected one on top.
    let mut order: Vec<usize> = (0..bands.len()).filter(|i| *i != selected).collect();
    if selected < bands.len() {
        order.push(selected);
    }
    for i in order {
        let (x, y) = g.band_point(&bands[i]);
        out.points.push((x, y, band_color(i), i + 1, bands[i].bypass, i == selected));
    }
    out
}

/// Parametric EQ graph of one channel: band curves, HPF/LPF and the total
/// response. Drag a band point to change frequency and gain, scroll over it to
/// change Q, click to select it. Bypassed bands are shown as a dashed preview
/// of what they would do; an Auto-EQ proposal is drawn in orange. In the phase
/// view only the curves are drawn (no band points).
#[component]
pub fn PeqGraph(
    device: Device,
    #[prop(into)] channel: Signal<String>,
    selected: RwSignal<usize>,
    #[prop(into)] proposal: Signal<Option<Vec<PeqBand>>>,
    #[prop(optional, into)] phase: Signal<bool>,
    /// Another channel to draw dashed.
    #[prop(optional, into)]
    compare: Signal<Option<String>>,
) -> impl IntoView {
    let container = NodeRef::<html::Div>::new();
    let size = use_size(container);
    let geometry = Memo::new(move |_| {
        let (width, height) = size.get();
        Geometry { width, height }
    });
    let input = Memo::new(move |_| {
        let ch = channel.get();
        let (hp_preview, lp_preview) = (device.filter_slope(&ch, true), device.filter_slope(&ch, false));
        device.state.with(|s| {
            let mut bands = s.peq_bands.get(&ch).cloned().unwrap_or_default();
            // EQ Bypass: the bands are drawn like bypassed bands.
            if s.eq_bypass.get(&ch).copied().unwrap_or(false) {
                for b in &mut bands {
                    b.bypass = true;
                }
            }
            let inverted = s.phase.get(&ch).copied().unwrap_or(false);
            GraphInput { bands, hp: filter_of(s, &ch, true), lp: filter_of(s, &ch, false), hp_preview, lp_preview, inverted }
        })
    });
    let compared = Memo::new(move |_| {
        let other = compare.get()?;
        let ch = channel.get();
        device.state.with(|s| {
            if !s.model.has_channel(&other) || other == ch {
                return None;
            }
            let delay = |c: &str| p::delay_raw_to_ms(s.delays.get(c).copied().unwrap_or(0));
            let color = if s.model.is_input(&other) { "var(--input)" } else { "var(--output)" };
            Some(Compared { eq: ChannelEq::of(s, &other), delay_ms: delay(&other) - delay(&ch), color })
        })
    });
    let drawn = Memo::new(move |_| {
        let g = geometry.get();
        if g.width < 50.0 || g.height < 50.0 {
            return GraphShapes::default();
        }
        proposal.with(|prop| {
            compared.with(|c| {
                input.with(|i| {
                    if phase.get() {
                        phase_shapes(&g, i, prop.as_ref(), c.as_ref())
                    } else {
                        shapes(&g, i, selected.get(), prop.as_ref(), c.as_ref())
                    }
                })
            })
        })
    });

    // ─── Interaction ───
    let drag = StoredValue::new(None::<(usize, f64, f64, bool)>); // band, start x, start y, moved
    let local = move |ev: &PointerEvent| -> Option<(f64, f64)> {
        let rect = container.get_untracked()?.get_bounding_client_rect();
        Some((ev.client_x() as f64 - rect.left(), ev.client_y() as f64 - rect.top()))
    };
    let nearest = move |x: f64, y: f64| -> usize {
        let g = geometry.get_untracked();
        input.with_untracked(|i| {
            i.bands
                .iter()
                .enumerate()
                .map(|(k, b)| {
                    let (bx, by) = g.band_point(b);
                    (k, (bx - x).hypot(by - y))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map_or(0, |(k, _)| k)
        })
    };
    let on_down = move |ev: PointerEvent| {
        if ev.button() != 0 || phase.get_untracked() {
            return;
        }
        let Some((x, y)) = local(&ev) else { return };
        let band = nearest(x, y);
        selected.set(band);
        drag.set_value(Some((band, x, y, false)));
        if let Some(el) = container.get_untracked() {
            let _ = el.set_pointer_capture(ev.pointer_id());
        }
    };
    let on_move = move |ev: PointerEvent| {
        let Some((band, sx, sy, moved)) = drag.get_value() else { return };
        let Some((x, y)) = local(&ev) else { return };
        if !moved && (x - sx).hypot(y - sy) < 3.0 {
            return;
        }
        drag.set_value(Some((band, sx, sy, true)));
        let g = geometry.get_untracked();
        let edit = PeqEdit {
            freq_raw: Some(p::peq_hz_to_freq(g.x_to_hz(x))),
            gain_db: Some(round_gain(g.y_to_db(y))),
            ..Default::default()
        };
        device.set_peq_band(&channel.get_untracked(), band, edit);
    };
    let on_up = move |_: PointerEvent| drag.set_value(None);
    let on_wheel = move |ev: WheelEvent| {
        if phase.get_untracked() {
            return;
        }
        let Some(el) = container.get_untracked() else { return };
        let rect = el.get_bounding_client_rect();
        let (x, y) = (ev.client_x() as f64 - rect.left(), ev.client_y() as f64 - rect.top());
        ev.prevent_default();
        let band = nearest(x, y);
        selected.set(band);
        let step = if ev.delta_y() < 0.0 { 1 } else { -1 };
        let b = input.with_untracked(|i| i.bands.get(band).cloned()).unwrap_or_default();
        let q = (b.q_raw + step).clamp(0, p::max_q_raw_for_type(b.peq_type));
        device.set_peq_band(&channel.get_untracked(), band, PeqEdit { q_raw: Some(q), ..Default::default() });
    };

    // Grid and labels depend on the size only.
    let grid = move || {
        let g = geometry.get();
        if g.width < 50.0 {
            return ().into_any();
        }
        let left = PAD_LEFT;
        let right = PAD_LEFT + g.w();
        let top = PAD_TOP;
        let bottom = PAD_TOP + g.h();
        let mut db_lines = Vec::new();
        if phase.get() {
            for deg in (-4..=4).map(|k| k as f64 * 45.0) {
                let y = g.deg_to_y(deg);
                let class = if deg == 0.0 { "graph-zero" } else { "graph-grid" };
                db_lines.push(view! { <line class=class x1=left y1=y x2=right y2=y/> }.into_any());
                if deg % 90.0 == 0.0 {
                    let text = format!("{}°", deg as i64);
                    db_lines.push(view! { <text class="graph-text" x=left - 4.0 y=y + 3.0 text-anchor="end">{text}</text> }.into_any());
                }
            }
        }
        let mut db = -12.0;
        while db <= 12.0 && !phase.get() {
            let y = g.db_to_y(db);
            let class = if db == 0.0 { "graph-zero" } else { "graph-grid" };
            db_lines.push(view! { <line class=class x1=left y1=y x2=right y2=y/> }.into_any());
            if db % 6.0 == 0.0 {
                let text = if db > 0.0 { format!("+{}", db as i64) } else { format!("{}", db as i64) };
                db_lines.push(view! { <text class="graph-text" x=left - 4.0 y=y + 3.0 text-anchor="end">{text}</text> }.into_any());
            }
            db += 3.0;
        }
        let mut hz_lines = Vec::new();
        for decade in [20.0, 100.0, 1000.0, 10000.0] {
            for m in 1..10 {
                let hz = decade * m as f64;
                if !(MIN_HZ..=MAX_HZ).contains(&hz) || (decade == 20.0 && m > 4) {
                    continue;
                }
                let x = g.hz_to_x(hz);
                let class = if m == 1 { "graph-decade" } else { "graph-minor" };
                hz_lines.push(view! { <line class=class x1=x y1=top x2=x y2=bottom/> });
            }
        }
        let labels = FREQ_MARK_HZ
            .iter()
            .map(|hz| {
                let text = if *hz >= 1000.0 { format!("{}k", (*hz / 1000.0).round() as i64) } else { format!("{}", *hz as i64) };
                view! { <text class="graph-text" x=g.hz_to_x(*hz) y=bottom + 14.0 text-anchor="middle">{text}</text> }
            })
            .collect_view();
        view! {
            <rect x=left y=top width=g.w() height=g.h() rx="6" fill="var(--bg-deep)"/>
            {hz_lines}
            {db_lines}
            {labels}
        }
        .into_any()
    };
    let curves = move || {
        let d = drawn.get();
        let g = geometry.get_untracked();
        let fill = d.fill.map(|(path, color)| view! { <path d=path fill=color fill-opacity="0.14" stroke="none"/> });
        let bands = d
            .band_paths
            .into_iter()
            .map(|(path, color, sel, dashed)| {
                let (opacity, width) = match (dashed, sel) {
                    (true, true) => (0.82, 1.8),
                    (true, false) => (0.51, 1.3),
                    (false, true) => (0.86, 2.0),
                    (false, false) => (0.43, 1.2),
                };
                view! {
                    <path d=path fill="none" stroke=color stroke-opacity=opacity stroke-width=width
                        stroke-dasharray=if dashed { "5 4" } else { "none" }/>
                }
            })
            .collect_view();
        let filters = d
            .filter_paths
            .into_iter()
            .map(|(path, color, dashed)| {
                view! {
                    <path d=path fill="none" stroke=color stroke-opacity=if dashed { 0.51 } else { 0.67 }
                        stroke-width=if dashed { 1.3 } else { 1.5 } stroke-dasharray=if dashed { "5 4" } else { "none" }/>
                }
            })
            .collect_view();
        let proposal = d.proposal.map(|path| {
            view! { <path d=path fill="none" stroke=PROPOSAL_COLOR stroke-width="2.2" stroke-linejoin="round"/> }
        });
        let compare = d.compare.map(|(path, color)| {
            view! { <path d=path fill="none" stroke=color stroke-width="1.8" stroke-dasharray="6 4" stroke-opacity="0.9"/> }
        });
        let points = d
            .points
            .into_iter()
            .map(|(x, y, color, label, bypass, sel)| {
                let r = if sel { 11.0 } else { 8.0 };
                view! {
                    {sel.then(|| view! { <circle cx=x cy=y r=r + 4.0 fill=color fill-opacity="0.24"/> })}
                    {if bypass {
                        view! {
                            <circle cx=x cy=y r=r fill="var(--bg-deep)"/>
                            <circle cx=x cy=y r=r - 1.0 fill="none" stroke=color stroke-width="2"/>
                        }.into_any()
                    } else {
                        view! { <circle cx=x cy=y r=r fill=color/> }.into_any()
                    }}
                    <text class="point-label" x=x y=y + if sel { 4.0 } else { 3.2 } text-anchor="middle"
                        font-size=if sel { "11" } else { "9" } fill=if bypass { color } else { "var(--bg)" }>{label}</text>
                }
            })
            .collect_view();
        view! {
            <clipPath id="peq-clip"><rect x=PAD_LEFT y=PAD_TOP width=g.w() height=g.h()/></clipPath>
            <g clip-path="url(#peq-clip)">
                {fill}
                {bands}
                {filters}
                {compare}
                {proposal}
                <path d=d.total fill="none" stroke="var(--text)" stroke-width="2.4" stroke-linejoin="round"/>
            </g>
            {points}
        }
    };

    view! {
        <div class="peq-graph" node_ref=container
            on:pointerdown=on_down on:pointermove=on_move on:pointerup=on_up on:pointercancel=on_up on:wheel=on_wheel>
            <svg width=move || geometry.get().width height=move || geometry.get().height>
                {grid}
                {curves}
            </svg>
        </div>
    }
}
