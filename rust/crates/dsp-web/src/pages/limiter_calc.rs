//! Limiter calculator on the Limit page: the amplifier channel behind the
//! selected output (from a library of amplifiers kept on the server), the
//! power the limiter allows, the headroom to the amplifier's clipping, and
//! buttons that set the threshold for a wanted power. See
//! dsp_core::limiter_calc for the math and its assumptions.

use dsp_core::limiter_calc::{
    Amplifier, LimiterSetup, OutputAmp, limited_power, threshold_for_watts, timing_for_band,
};
use dsp_core::protocol as p;
use leptos::prelude::*;

use crate::components::common::{Modal, PanelCard};
use crate::fmt::{fixed, parse_number};
use crate::icons::{I, Icon};
use crate::store::app;

fn watts_text(w: f64) -> String {
    if w >= 100.0 { format!("{} W", w.round() as i64) } else { format!("{} W", fixed(w, 1)) }
}

fn number_input(value: Option<f64>, placeholder: &'static str, on_set: impl Fn(Option<f64>) + 'static) -> impl IntoView {
    view! {
        <input class="calc-input mono" type="text" inputmode="decimal" placeholder=placeholder
            prop:value=value.map(|v| crate::fmt::fixed(v, if v.fract() == 0.0 { 0 } else { 2 })).unwrap_or_default()
            on:change=move |ev| on_set(parse_number(&event_target_value(&ev)))/>
    }
}

#[component]
pub fn LimiterCalcCard(
    /// Id of the DSP (key of the setup).
    dsp: String,
    channel: String,
    #[prop(into)] setup: Signal<LimiterSetup>,
    on_change: Callback<LimiterSetup>,
    /// Current threshold of the limiter, raw.
    #[prop(into)]
    threshold_raw: Signal<i64>,
    on_threshold: Callback<i64>,
    /// Lowest frequency of the output (its HPF), for the timing preset.
    #[prop(into)]
    low_hz: Signal<f64>,
    on_timing: Callback<(i64, i64)>,
) -> impl IntoView {
    let key = LimiterSetup::output_key(&dsp, &channel);
    let editing = RwSignal::new(false);
    let custom_w = RwSignal::new(None::<f64>);

    let output = {
        let key = key.clone();
        Memo::new(move |_| setup.with(|s| s.outputs.get(&key).cloned()))
    };
    let reference = {
        let dsp = dsp.clone();
        Memo::new(move |_| setup.with(|s| s.reference(&dsp)))
    };
    let amp = Memo::new(move |_| output.get().and_then(|o| setup.with(|s| s.amp(&o.amp).cloned())));

    let update = move |f: &dyn Fn(&mut LimiterSetup)| {
        let mut next = setup.get_untracked();
        f(&mut next);
        on_change.run(next);
    };
    let set_output = {
        let key = key.clone();
        move |o: Option<OutputAmp>| {
            let key = key.clone();
            update(&move |s| match &o {
                Some(o) => {
                    s.outputs.insert(key.clone(), o.clone());
                }
                None => {
                    s.outputs.remove(&key);
                }
            })
        }
    };
    let set_amp = set_output.clone();
    let set_ohms = set_output.clone();
    let set_speaker = set_output;
    let set_reference = {
        let dsp = dsp.clone();
        move |v: Option<f64>| {
            let dsp = dsp.clone();
            update(&move |s| {
                match v {
                    Some(v) if v != 0.0 => s.reference_dbu.insert(dsp.clone(), v.clamp(-20.0, 30.0)),
                    _ => s.reference_dbu.remove(&dsp),
                };
            })
        }
    };

    let result = Memo::new(move |_| {
        let (a, o) = (amp.get()?, output.get()?);
        limited_power(&a, o.ohms, p::threshold_raw_to_db(threshold_raw.get()), reference.get())
    });
    let limit_to = move |watts: f64| {
        let (Some(a), Some(o)) = (amp.get_untracked(), output.get_untracked()) else { return };
        match threshold_for_watts(&a, o.ohms, watts, reference.get_untracked()) {
            Some(raw) => {
                on_threshold.run(raw);
                app().toast(format!("Threshold {} dB for {}", fixed(p::threshold_raw_to_db(raw), 1), watts_text(watts)), false);
            }
            None => app().toast("This amplifier has no sensitivity or gain", true),
        }
    };

    let amp_options = move || {
        let selected = output.get().map(|o| o.amp);
        setup.with(|s| {
            s.amps
                .iter()
                .map(|a| {
                    let id = a.id.clone();
                    let sel = selected.as_deref() == Some(id.as_str());
                    view! { <option value=id selected=sel>{a.name.clone()}</option> }
                })
                .collect_view()
        })
    };

    let details = move || {
        let Some(o) = output.get() else {
            return view! {
                <div class="calc-note muted">"Choose the amplifier behind this output to see the power the limiter allows."</div>
            }
            .into_any();
        };
        let Some(r) = result.get() else {
            return view! { <div class="calc-note warn">"The amplifier needs an input sensitivity or a gain."</div> }.into_any();
        };
        let mut notes = Vec::new();
        if r.rated_watts <= 0.0 {
            notes.push(format!("The amplifier is not rated for {} Ω.", fixed(o.ohms, 1)));
        } else if r.clip_db > 0.05 {
            notes.push(format!("The amplifier clips {} dB before the limiter acts.", fixed(r.clip_db, 1)));
        }
        if o.speaker_w > 0.0 && r.watts > o.speaker_w * 1.001 {
            notes.push(format!("{} is above the speaker rating ({}).", watts_text(r.watts), watts_text(o.speaker_w)));
        }
        let share = if r.rated_watts > 0.0 { format!(" – {} % of the amplifier", (r.watts / r.rated_watts * 100.0).round()) } else { String::new() };
        let amp_w = r.rated_watts * 10f64.powf(-0.1);
        let mut chips = Vec::new();
        if o.speaker_w > 0.0 {
            chips.push(("Speaker", o.speaker_w));
        }
        if r.rated_watts > 0.0 {
            chips.push(("Amplifier −1 dB", amp_w));
        }
        if o.speaker_w > 0.0 && r.rated_watts > 0.0 {
            chips.push(("Lower of both", o.speaker_w.min(amp_w)));
        }
        let chips = chips
            .into_iter()
            .map(|(label, w)| {
                view! {
                    <button class="btn outlined compact" on:click=move |_| limit_to(w)>{format!("{label} · {}", watts_text(w))}</button>
                }
            })
            .collect_view();
        view! {
            <div class="calc-result">
                <span class="calc-watts mono">{watts_text(r.watts)}</span>
                <span class="muted">{format!("into {} Ω at the threshold{share}", fixed(o.ohms, 1))}</span>
            </div>
            {notes.into_iter().map(|n| view! { <div class="calc-note warn">{n}</div> }).collect_view()}
            <div class="wrap-row">
                <span class="small muted">"Limit to"</span>
                {chips}
                {number_input(None, "W", move |v| custom_w.set(v))}
                <button class="btn text compact" prop:disabled=move || custom_w.get().is_none_or(|w| w <= 0.0)
                    on:click=move |_| if let Some(w) = custom_w.get_untracked() { limit_to(w) }>"Set"</button>
            </div>
        }
        .into_any()
    };

    let timing = move |_| {
        let (attack, release) = timing_for_band(low_hz.get_untracked());
        on_timing.run((attack as i64, release as i64));
        app().toast(format!("Attack {attack} ms, release {release} ms (from {} Hz)", low_hz.get_untracked().round()), false);
    };

    view! {
        <PanelCard title=Signal::derive(|| "Limiter calculator".to_string()) class="calc-card"
            trailing=view! {
                <button class="btn text compact" on:click=move |_| editing.set(true)>
                    <Icon icon=I::Edit size=16/>"Amplifiers …"
                </button>
            }.into_any()>
            <div class="calc-body">
                <div class="wrap-row">
                    <span class="small muted">"Amplifier"</span>
                    <select class="select" on:change=move |ev| {
                        let id = event_target_value(&ev);
                        if id.is_empty() {
                            set_amp(None);
                        } else {
                            let previous = output.get_untracked();
                            set_amp(Some(OutputAmp {
                                amp: id,
                                ohms: previous.as_ref().map_or(8.0, |o| o.ohms),
                                speaker_w: previous.map_or(0.0, |o| o.speaker_w),
                            }));
                        }
                    }>
                        <option value="" selected=move || output.with(Option::is_none)>"—"</option>
                        {amp_options}
                    </select>
                    <Show when=move || output.with(Option::is_some)>
                        <span class="small muted">"Load Ω"</span>
                        {
                            let set_ohms = set_ohms.clone();
                            move || number_input(output.get().map(|o| o.ohms), "8", {
                                let set_ohms = set_ohms.clone();
                                move |v| if let (Some(v), Some(o)) = (v, output.get_untracked()) {
                                    set_ohms(Some(OutputAmp { ohms: v.clamp(1.0, 64.0), ..o }));
                                }
                            })
                        }
                        <span class="small muted">"Speaker W"</span>
                        {
                            let set_speaker = set_speaker.clone();
                            move || number_input(output.get().map(|o| o.speaker_w).filter(|w| *w > 0.0), "RMS", {
                                let set_speaker = set_speaker.clone();
                                move |v| if let Some(o) = output.get_untracked() {
                                    set_speaker(Some(OutputAmp { speaker_w: v.unwrap_or(0.0).max(0.0), ..o }));
                                }
                            })
                        }
                    </Show>
                </div>
                {details}
                <div class="wrap-row">
                    <button class="btn text compact" on:click=timing
                        title="Attack about one period of the lowest frequency of the output (its HPF), release 16 × attack">
                        <Icon icon=I::Timer size=16/>"Timing from the HPF"
                    </button>
                    <span class="spacer"></span>
                    <span class="small muted" title="Output level at limiter threshold 0 dB. Not published for the DSP; measurements put it near 0 dBu. Check it with a measurement.">
                        "0 dB threshold ="
                    </span>
                    {move || number_input(Some(reference.get()), "0", set_reference.clone())}
                    <span class="small muted">"dBu"</span>
                </div>
                <div class="calc-help">
                    "A planning aid from data sheet values: verify the result with a measurement."
                </div>
            </div>
        </PanelCard>
        {move || editing.get().then(|| view! {
            <AmpEditor setup=setup on_change=on_change on_close=Callback::new(move |_| editing.set(false))/>
        })}
    }
}

/// Edit the amplifier library.
#[component]
fn AmpEditor(#[prop(into)] setup: Signal<LimiterSetup>, on_change: Callback<LimiterSetup>, on_close: Callback<()>) -> impl IntoView {
    let amps = RwSignal::new(setup.get_untracked().amps);
    let save = move |_| {
        let mut next = setup.get_untracked();
        next.amps = amps.get_untracked();
        on_change.run(next);
        on_close.run(());
    };
    let add = move |_| {
        amps.update(|list| {
            let id = format!("amp{}", js_sys::Date::now() as u64 % 1_000_000_000);
            list.push(Amplifier {
                id,
                name: "Amplifier".into(),
                watts8: 0.0,
                watts4: 0.0,
                watts2: 0.0,
                sensitivity_v: Some(1.4),
                gain_db: None,
            });
        })
    };
    let rows = move || {
        amps.get()
            .into_iter()
            .enumerate()
            .map(|(i, a)| {
                let edit = move |f: Box<dyn Fn(&mut Amplifier)>| amps.update(|l| if let Some(a) = l.get_mut(i) { f(a) });
                let by_gain = a.sensitivity_v.is_none();
                let value = if by_gain { a.gain_db } else { a.sensitivity_v };
                view! {
                    <div class="amp-row">
                        <input class="calc-input amp-name" type="text" maxlength="40" prop:value=a.name.clone()
                            on:change=move |ev| { let v = event_target_value(&ev); edit(Box::new(move |a| a.name = v.clone())) }/>
                        {number_input(Some(a.watts8).filter(|w| *w > 0.0), "W", move |v| edit(Box::new(move |a| a.watts8 = v.unwrap_or(0.0).max(0.0))))}
                        {number_input(Some(a.watts4).filter(|w| *w > 0.0), "W", move |v| edit(Box::new(move |a| a.watts4 = v.unwrap_or(0.0).max(0.0))))}
                        {number_input(Some(a.watts2).filter(|w| *w > 0.0), "W", move |v| edit(Box::new(move |a| a.watts2 = v.unwrap_or(0.0).max(0.0))))}
                        <select class="select" on:change=move |ev| {
                            let gain = event_target_value(&ev) == "gain";
                            edit(Box::new(move |a| if gain {
                                a.sensitivity_v = None;
                                a.gain_db = Some(a.gain_db.unwrap_or(32.0));
                            } else {
                                a.gain_db = None;
                                a.sensitivity_v = Some(a.sensitivity_v.unwrap_or(1.4));
                            }))
                        }>
                            <option value="sens" selected=!by_gain>"Sensitivity V"</option>
                            <option value="gain" selected=by_gain>"Gain dB"</option>
                        </select>
                        {number_input(value, if by_gain { "dB" } else { "V" }, move |v| edit(Box::new(move |a| if by_gain {
                            a.gain_db = v.map(|v| v.clamp(0.0, 60.0));
                        } else {
                            a.sensitivity_v = v.filter(|v| *v > 0.0);
                        })))}
                        <button class="icon-button small" title="Remove" on:click=move |_| amps.update(|l| { l.remove(i); })>
                            <Icon icon=I::Delete size=16/>
                        </button>
                    </div>
                }
            })
            .collect_view()
    };
    view! {
        <Modal title="Amplifiers" width=720 on_close=on_close
            actions=view! {
                <button class="btn text" on:click=move |_| on_close.run(())>"Cancel"</button>
                <button class="btn filled" on:click=save>"Save"</button>
            }.into_any()>
            <div class="note-text muted">"Rated power per channel from the data sheet, and the input sensitivity (V for rated power) or the voltage gain."</div>
            <div class="amp-row amp-head small muted">
                <span class="amp-name">"Name"</span><span>"8 Ω"</span><span>"4 Ω"</span><span>"2 Ω"</span><span>"Input"</span><span></span><span></span>
            </div>
            {rows}
            <button class="btn text compact" on:click=add><Icon icon=I::Add size=16/>"Add amplifier"</button>
        </Modal>
    }
}
