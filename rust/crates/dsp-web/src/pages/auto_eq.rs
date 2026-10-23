//! Auto-EQ: measure pink noise with the RTA microphone and propose a
//! correction for one channel, like the auto-EQ of system processors.
//!
//! - Inputs on the GEQ page: the correction goes into the 31-band GEQ.
//! - Any channel on the EQ page: it goes into the free PEQ bands (peaks at
//!   0 dB), e.g. per output for subs and tops.
//!
//! During the measurement the other channels of the group are muted (and
//! restored afterwards). A linked channel is measured together with its
//! partners (left/right) or alone; for "alone" the link is lifted during the
//! measurement – a mute on a partner would otherwise mute the channel itself.

use std::time::Duration;

use dsp_core::eq_math::{FitLimits, GEQ_CENTER_FREQUENCIES, auto_eq_correction, fit_peq_correction, free_peq_bands};
use dsp_core::{PeqBand, ops};
use leptos::prelude::*;

use crate::components::common::{Check, Modal};
use crate::icons::{I, Icon};
use crate::pages::dsp::ui;
use crate::pages::generators::{DspGeneratorPanel, ToneGeneratorPanel};
use crate::rta::Rta;
use crate::store::{Device, PeqEdit, alias_in, app};

const MEASURE_SECONDS: f64 = 8.0;

/// Border colour of the Auto-EQ panel and colour of the proposals.
pub const PROPOSAL_COLOR: &str = "#FBBF24";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoEqTarget {
    Geq,
    Peq,
}

/// Mutes and link masks to restore after the measurement.
#[derive(Default)]
struct Saved {
    mutes: Option<Vec<(String, bool)>>,
    links: Option<Vec<(String, i64)>>,
}

/// Mutes first (while the link is still lifted), then the link masks.
fn restore_values(device: Device, saved: Saved) {
    for (c, on) in saved.mutes.unwrap_or_default() {
        if device.state.with_untracked(|s| s.mutes.get(&c).copied().unwrap_or(false)) != on {
            device.set_mute(&c, on);
        }
    }
    for (c, mask) in saved.links.unwrap_or_default() {
        device.set_link_mask(&c, mask);
    }
}

#[component]
pub fn AutoEqPanel(
    channel: String,
    target: AutoEqTarget,
    /// Running RTA of the page (GEQ page); None: the panel starts its own.
    #[prop(optional)]
    rta: Option<Rta>,
    /// New GEQ proposal (all 31 bands) or None.
    #[prop(optional)]
    on_geq_proposal: Option<Callback<Option<Vec<f64>>>>,
    /// New PEQ proposal (all bands) or None.
    #[prop(optional)]
    on_peq_proposal: Option<Callback<Option<Vec<PeqBand>>>>,
    on_close: Callback<()>,
) -> impl IntoView {
    let device = ui().device;
    let ch = StoredValue::new(channel);
    let own_rta = rta.is_none();
    let rta = rta.unwrap_or_else(|| {
        let rta = Rta::new();
        rta.start();
        rta
    });

    let progress = RwSignal::new(0.0);
    let measuring = RwSignal::new(false);
    let max_boost = RwSignal::new(6.0);
    let isolate = RwSignal::new(true);
    let geq_proposal = RwSignal::new(None::<Vec<f64>>);
    let peq_proposal = RwSignal::new(None::<Vec<PeqBand>>);
    let note = RwSignal::new(None::<String>);
    let ask_linked = RwSignal::new(false);
    let saved = StoredValue::new_local(Saved::default());
    let timer = StoredValue::new(None::<IntervalHandle>);
    let has_proposal = move || geq_proposal.with(Option::is_some) || peq_proposal.with(Option::is_some);

    let alias = Memo::new(move |_| ch.with_value(|c| device.alias(c)));
    let is_output = device.model_untracked().is_output(&ch.get_value());

    let set_proposal = move |geq: Option<Vec<f64>>, peq: Option<Vec<PeqBand>>, text: Option<String>| {
        geq_proposal.set(geq.clone());
        peq_proposal.set(peq.clone());
        note.set(text);
        if let Some(cb) = on_geq_proposal {
            cb.run(geq);
        }
        if let Some(cb) = on_peq_proposal {
            cb.run(peq);
        }
    };

    // ─── Isolation ───

    let isolate_channel = move |alone: bool| {
        let c = ch.get_value();
        let (link_group, group, model) = device.state.with_untracked(|s| {
            let mut link_group = vec![c.clone()];
            link_group.extend(ops::linked_with(s, &c));
            (link_group, s.model.group_of(&c).iter().map(|g| g.to_string()).collect::<Vec<_>>(), s.model)
        });
        let mut next = Saved::default();
        if alone && link_group.len() > 1 {
            next.links = Some(link_group.iter().map(|l| (l.clone(), device.link_mask(l))).collect());
            for l in &link_group {
                device.set_link_mask(l, model.link_bit(l) as i64);
            }
        }
        let playing = if alone { vec![c.clone()] } else { link_group };
        let mute_of = |g: &str| device.state.with_untracked(|s| s.mutes.get(g).copied().unwrap_or(false));
        next.mutes = Some(group.iter().map(|g| (g.clone(), mute_of(g))).collect());
        for g in &group {
            let mute = !playing.contains(g);
            if mute_of(g) != mute {
                device.set_mute(g, mute);
            }
        }
        saved.set_value(next);
    };
    let restore = move || {
        let values = saved.try_update_value(std::mem::take).unwrap_or_default();
        restore_values(device, values);
    };

    // ─── Measurement ───

    let propose = move |measured: Vec<f64>| {
        let c = ch.get_value();
        let boost = max_boost.get_untracked();
        if target == AutoEqTarget::Geq {
            let current = device.state.with_untracked(|s| s.geq_bands.get(&c).cloned().unwrap_or_default());
            if current.len() == GEQ_CENTER_FREQUENCIES.len() {
                set_proposal(Some(auto_eq_correction(&measured, &GEQ_CENTER_FREQUENCIES, &current, boost)), None, None);
            }
            return;
        }
        let bands = device.state.with_untracked(|s| s.peq_bands.get(&c).cloned().unwrap_or_default());
        let free = free_peq_bands(&bands);
        if free.is_empty() {
            set_proposal(
                None,
                None,
                Some(
                    "No free PEQ band: Auto-EQ uses peak bands at 0 dB. Set a band to Peak with 0 dB and measure again."
                        .into(),
                ),
            );
            return;
        }
        let zeros = vec![0.0; GEQ_CENTER_FREQUENCIES.len()];
        let wanted = auto_eq_correction(&measured, &GEQ_CENTER_FREQUENCIES, &zeros, boost);
        let limits = FitLimits { max_boost: boost, ..FitLimits::default() };
        let proposal = fit_peq_correction(&wanted, &GEQ_CENTER_FREQUENCIES, &bands, limits);
        let used: Vec<usize> = free.iter().copied().filter(|i| proposal[*i].gain_db.abs() >= 0.05).collect();
        let text = if used.is_empty() {
            "The response is already within ±1 dB – nothing to correct.".to_string()
        } else {
            format!(
                "Uses band{} {} of {} free.",
                if used.len() > 1 { "s" } else { "" },
                used.iter().map(|i| (i + 1).to_string()).collect::<Vec<_>>().join(", "),
                free.len()
            )
        };
        set_proposal(None, Some(proposal), Some(text));
    };

    let stop_timer = move || {
        if let Some(handle) = timer.try_update_value(Option::take).flatten() {
            handle.clear();
        }
        measuring.set(false);
    };

    let begin = move |alone: Option<bool>| {
        if let Some(alone) = alone {
            isolate_channel(alone);
        }
        set_proposal(None, None, None);
        rta.start_measurement();
        progress.set(0.0);
        measuring.set(true);
        let tick = move || {
            let p = rta.measured_seconds() / MEASURE_SECONDS;
            if p < 1.0 {
                progress.set(p);
                return;
            }
            stop_timer();
            restore();
            if let Some(measured) = rta.stop_measurement(&GEQ_CENTER_FREQUENCIES) {
                propose(measured);
            }
        };
        if let Ok(handle) = set_interval_with_handle(tick, Duration::from_millis(200)) {
            timer.set_value(Some(handle));
        }
    };

    let measure = move || {
        if measuring.get_untracked() || !rta.running.get_untracked() {
            return;
        }
        if !isolate.get_untracked() {
            begin(None);
            return;
        }
        let linked = ch.with_value(|c| device.state.with_untracked(|s| !ops::linked_with(s, c).is_empty()));
        if linked { ask_linked.set(true) } else { begin(Some(true)) }
    };

    let cancel = move || {
        stop_timer();
        rta.cancel_measurement();
        restore();
    };

    let apply = move || {
        let c = ch.get_value();
        let (geq, peq) = (geq_proposal.get_untracked(), peq_proposal.get_untracked());
        if let Some(geq) = &geq {
            let current = device.state.with_untracked(|s| s.geq_bands.get(&c).cloned().unwrap_or_default());
            for (i, db) in geq.iter().enumerate() {
                if current.get(i) != Some(db) {
                    device.set_geq_band(&c, i, *db);
                }
            }
        }
        if let Some(peq) = &peq {
            let current = device.state.with_untracked(|s| s.peq_bands.get(&c).cloned().unwrap_or_default());
            for (i, b) in peq.iter().enumerate() {
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
        }
        set_proposal(None, None, None);
        let what = if geq.is_some() { "GEQ" } else { "PEQ" };
        app().toast(format!("Auto-EQ applied to the {what} of {}", alias.get_untracked()), false);
    };

    on_cleanup(move || {
        if let Some(handle) = timer.try_update_value(Option::take).flatten() {
            handle.clear();
        }
        if !own_rta {
            rta.cancel_measurement();
        }
        // Not while the view is being torn down: the mute changes update the
        // pages.
        let values = saved.try_update_value(std::mem::take).unwrap_or_default();
        if values.mutes.is_some() || values.links.is_some() {
            set_timeout(move || restore_values(device, values), Duration::ZERO);
        }
    });

    // ─── View ───

    let step = move || {
        let a = alias.get();
        if let Some(e) = rta.error.get() {
            e
        } else if !rta.running.get() {
            if rta.starting.get() || own_rta {
                "Starting the microphone …".into()
            } else {
                "Start the RTA to measure.".into()
            }
        } else if measuring.get() {
            format!(
                "Measuring {a} … {} / {} s",
                crate::fmt::fixed(progress.get() * MEASURE_SECONDS, 0),
                MEASURE_SECONDS.round() as i64
            )
        } else if has_proposal() {
            format!("Proposal for {a} shown as orange curve. {}", note.get().unwrap_or_default())
        } else {
            note.get().unwrap_or_else(|| {
                format!(
                    "1. Pink noise into the system (generator below or external){}.  2. Measurement mic at the listening position.  3. Measure.",
                    if is_output { format!(", routed to {a}") } else { format!(" at {a}") }
                )
            })
        }
    };
    let group_name = if is_output { "outputs" } else { "inputs" };
    let help = format!(
        "Corrects 40 Hz – 16 kHz towards the median level, smoothed, max. 12 dB cut{}. The microphone response is not \
         compensated – use a measurement mic. Whether the DSP generator passes the input EQ is not known yet (see TODO.md).",
        if target == AutoEqTarget::Peq { ", with the free PEQ bands" } else { "" }
    );
    let boosts = [0.0, 3.0, 6.0, 9.0]
        .map(|v| view! { <option value=v.to_string() selected=move || max_boost.get() == v>{format!("{v} dB")}</option> })
        .collect_view();

    let linked_dialog = move || {
        ask_linked.get().then(|| {
            let (a, names) = ch.with_value(|c| {
                device.state.with_untracked(|s| {
                    let names = ops::linked_with(s, c).iter().map(|p| alias_in(s, p)).collect::<Vec<_>>().join(", ");
                    (alias_in(s, c), names)
                })
            });
            let choose = move |alone: Option<bool>| {
                ask_linked.set(false);
                if let Some(alone) = alone {
                    begin(Some(alone));
                }
            };
            view! {
                <Modal title=format!("{a} is linked") width=440 on_close=Callback::new(move |_| choose(None))
                    actions=view! {
                        <button class="btn text" on:click=move |_| choose(None)>"Cancel"</button>
                        <button class="btn text" on:click=move |_| choose(Some(true))>"Alone"</button>
                        <button class="btn filled" on:click=move |_| choose(Some(false))>"Together"</button>
                    }.into_any()>
                    <div class="note-text">{format!("{a} is linked with {names} – usually a left/right pair.")}</div>
                    <div class="note-text">
                        "Together: both play, the other channels are muted, the result applies to all linked channels."
                    </div>
                    <div class="note-text">
                        {format!("Alone: the link is lifted during the measurement (and restored afterwards), only {a} plays. Applying the result changes the linked channels too.")}
                    </div>
                </Modal>
            }
        })
    };

    view! {
        <div class="auto-eq">
            <div class="auto-eq-head">
                <Icon icon=I::AutoFix size=16 color="var(--warning)"/>
                <span class="auto-eq-title">
                    {move || format!("Auto-EQ · {} → {}", alias.get(), if target == AutoEqTarget::Geq { "GEQ" } else { "PEQ" })}
                </span>
                <span class="auto-eq-step">{step}</span>
                <button class="icon-button small" title="Close Auto-EQ" on:click=move |_| on_close.run(())>
                    <Icon icon=I::Close size=16/>
                </button>
            </div>
            <Show when=move || measuring.get()>
                <div class="progress"><div class="progress-bar" style=move || format!("width:{:.1}%", progress.get() * 100.0)></div></div>
            </Show>
            <div class="wrap-row">
                {move || if measuring.get() {
                    view! {
                        <button class="btn outlined compact" on:click=move |_| cancel()>
                            <Icon icon=I::Stop size=18/>"Cancel"
                        </button>
                    }.into_any()
                } else {
                    view! {
                        <button class="btn filled compact" prop:disabled=move || !rta.running.get() on:click=move |_| measure()>
                            <Icon icon=I::Mic size=18/>{move || if has_proposal() { "Measure again" } else { "Measure" }}
                        </button>
                    }.into_any()
                }}
                <Check checked=isolate on_toggle=Callback::new(move |_| isolate.update(|v| *v = !*v))
                    disabled=measuring label=format!("Mute the other {group_name} while measuring")/>
                <div class="row tight">
                    <span class="small muted">"Max boost"</span>
                    <select class="select" on:change=move |ev| {
                        if let Ok(v) = event_target_value(&ev).parse::<f64>() {
                            max_boost.set(v);
                        }
                    }>{boosts}</select>
                </div>
                <Show when=has_proposal>
                    <button class="btn compact" style="background:var(--warning);color:var(--bg-deep)" on:click=move |_| apply()>
                        <Icon icon=I::Check size=18/>{move || format!("Apply to {}", alias.get())}
                    </button>
                    <button class="btn text compact" on:click=move |_| set_proposal(None, None, None)>"Discard"</button>
                </Show>
            </div>
            <div class="gen-row">
                <span class="gen-label">"Browser noise"</span>
                <ToneGeneratorPanel show_frequency=false/>
            </div>
            {(!device.offline).then(|| view! {
                <div class="gen-row">
                    <span class="gen-label">"DSP generator"</span>
                    <DspGeneratorPanel device=device/>
                </div>
            })}
            <div class="auto-eq-help">{help}</div>
        </div>
        {linked_dialog}
    }
}
