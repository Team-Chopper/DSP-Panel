//! Limiter calculator on the Limit page: the amplifier channel behind the
//! selected output (from a library of amplifiers kept on the server), the
//! power the limiter allows, the headroom to the amplifier's clipping, and
//! buttons that set the threshold for a wanted power. See
//! dsp_core::limiter_calc for the math and its assumptions.

use dsp_core::limiter_calc::{
    Amplifier, LimiterSetup, MAX_AMPS, MAX_WATTS, OutputAmp, limited_power, threshold_for_watts, timing_for_band,
};
use dsp_core::protocol as p;
use leptos::prelude::*;
use web_sys::{Event, HtmlInputElement};

use crate::components::common::{Modal, PanelCard};
use crate::fmt::{fixed, format_db_value, parse_number};
use crate::icons::{I, Icon};
use crate::store::app;

fn watts_text(w: f64) -> String {
    if w >= 100.0 { format!("{} W", w.round() as i64) } else { format!("{} W", fixed(w, 1)) }
}

/// A stored number as a field shows it: at most two decimals, no trailing
/// zeros ("8", "1.4").
fn number_text(v: f64) -> String {
    let s = fixed(v, 2);
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

/// Number field: shows [value]; Enter or leaving the field hands the typed
/// number to [on_set] (`None`: the field was emptied). Text that is not a
/// number is dropped; the field always ends up showing the stored value.
#[component]
fn NumberInput(
    #[prop(into)] value: Signal<Option<f64>>,
    #[prop(into)] placeholder: Signal<String>,
    on_set: Callback<Option<f64>>,
    #[prop(optional, into)] title: Option<String>,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    let shown = move || value.get().map(number_text).unwrap_or_default();
    view! {
        <input class=format!("calc-input mono {class}") type="text" inputmode="decimal" autocomplete="off"
            spellcheck="false" title=title placeholder=move || placeholder.get()
            prop:value=shown
            on:change=move |ev| {
                let el = event_target::<HtmlInputElement>(&ev);
                let typed = el.value();
                let parsed = parse_number(&typed);
                if parsed.is_some() || typed.trim().is_empty() {
                    on_set.run(parsed);
                }
                el.set_value(&value.get_untracked().map(number_text).unwrap_or_default());
            }/>
    }
}

/// What the calculator can say about the selected output.
#[derive(Clone, Copy, PartialEq)]
enum Status {
    /// No amplifier chosen (or it is no longer in the library).
    NoAmp,
    /// The amplifier has neither an input sensitivity nor a gain.
    NoGain,
    Ready,
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
    /// Lowest frequency of the output (its HPF), for the timing preset; only
    /// read when the button is pressed.
    #[prop(into)]
    low_hz: Signal<f64>,
    /// Attack and release in ms.
    on_timing: Callback<(i64, i64)>,
) -> impl IntoView {
    let key = StoredValue::new(LimiterSetup::output_key(&dsp, &channel));
    let dsp = StoredValue::new(dsp);
    let editing = RwSignal::new(false);
    let custom_w = RwSignal::new(None::<f64>);

    let output = Memo::new(move |_| key.with_value(|k| setup.with(|s| s.outputs.get(k).cloned())));
    let amp = Memo::new(move |_| output.with(|o| o.as_ref().and_then(|o| setup.with(|s| s.amp(&o.amp).cloned()))));
    let reference = Memo::new(move |_| dsp.with_value(|d| setup.with(|s| s.reference(d))));
    let amp_list =
        Memo::new(move |_| setup.with(|s| s.amps.iter().map(|a| (a.id.clone(), a.name.clone())).collect::<Vec<_>>()));
    let status = Memo::new(move |_| {
        match (output.with(Option::is_some), amp.with(|a| a.as_ref().map(|a| a.voltage_gain().is_some()))) {
            (true, Some(true)) => Status::Ready,
            (true, Some(false)) => Status::NoGain,
            _ => Status::NoAmp,
        }
    });
    let result = Memo::new(move |_| {
        let threshold = p::threshold_raw_to_db(threshold_raw.get());
        let reference = reference.get();
        amp.with(|a| {
            output.with(|o| match (a, o) {
                (Some(a), Some(o)) => limited_power(a, o.ohms, threshold, reference),
                _ => None,
            })
        })
    });
    let ohms = Memo::new(move |_| output.with(|o| o.as_ref().map(|o| o.ohms)));
    let speaker_w = Memo::new(move |_| output.with(|o| o.as_ref().map(|o| o.speaker_w).filter(|w| *w > 0.0)));

    let update = move |f: &dyn Fn(&mut LimiterSetup)| {
        let mut next = setup.get_untracked();
        f(&mut next);
        on_change.run(next);
    };
    let set_output = move |o: Option<OutputAmp>| {
        let k = key.get_value();
        update(&|s| match &o {
            Some(o) => {
                s.outputs.insert(k.clone(), o.clone());
            }
            None => {
                s.outputs.remove(&k);
            }
        });
    };
    let on_amp = move |ev: Event| {
        let id = event_target_value(&ev);
        if id.is_empty() {
            set_output(None);
        } else {
            let previous = output.get_untracked();
            set_output(Some(OutputAmp {
                amp: id,
                ohms: previous.as_ref().map_or(8.0, |o| o.ohms),
                speaker_w: previous.map_or(0.0, |o| o.speaker_w),
            }));
        }
    };
    let set_ohms = Callback::new(move |v: Option<f64>| {
        if let (Some(v), Some(o)) = (v, output.get_untracked()) {
            set_output(Some(OutputAmp { ohms: v.clamp(1.0, 64.0), ..o }));
        }
    });
    let set_speaker = Callback::new(move |v: Option<f64>| {
        if let Some(o) = output.get_untracked() {
            set_output(Some(OutputAmp { speaker_w: v.unwrap_or(0.0).clamp(0.0, MAX_WATTS), ..o }));
        }
    });
    let set_reference = Callback::new(move |v: Option<f64>| {
        let d = dsp.get_value();
        update(&|s| match v {
            Some(v) if v != 0.0 => {
                s.reference_dbu.insert(d.clone(), v.clamp(-20.0, 30.0));
            }
            _ => {
                s.reference_dbu.remove(&d);
            }
        });
    });
    let reference_value = Signal::derive(move || Some(reference.get()));
    let set_custom = Callback::new(move |v: Option<f64>| custom_w.set(v.filter(|w| *w > 0.0)));

    let limit_to = move |watts: f64| {
        let (Some(a), Some(o)) = (amp.get_untracked(), output.get_untracked()) else { return };
        match threshold_for_watts(&a, o.ohms, watts, reference.get_untracked()) {
            Some(raw) => {
                on_threshold.run(raw);
                let db = format_db_value(p::threshold_raw_to_db(raw));
                let note = if raw >= p::THRESHOLD_RAW_MAX {
                    " (the highest threshold: the limiter will not act)"
                } else if raw <= 0 {
                    " (the lowest threshold)"
                } else {
                    ""
                };
                app().toast(format!("Threshold {db} dB for {}{note}", watts_text(watts)), false);
            }
            None => app().toast("This amplifier has no usable input sensitivity or gain", true),
        }
    };

    // Values that only change with the setup, not with the threshold.
    let targets = Memo::new(move |_| {
        let mut list: Vec<(&'static str, f64)> = Vec::new();
        let (Some(a), Some(o)) = (amp.get(), output.get()) else { return list };
        let rated = a.rated_watts(o.ohms);
        let amp_w = rated * 10f64.powf(-0.1);
        if o.speaker_w > 0.0 {
            list.push(("Speaker", o.speaker_w));
        }
        if rated > 0.0 {
            list.push(("Amplifier −1 dB", amp_w));
        }
        if o.speaker_w > 0.0 && rated > 0.0 {
            list.push(("Lower of both", o.speaker_w.min(amp_w)));
        }
        list
    });
    let notes = Memo::new(move |_| {
        let mut notes = Vec::new();
        let (Some(r), Some(o)) = (result.get(), output.get()) else { return notes };
        if r.rated_watts <= 0.0 {
            notes.push(format!("The amplifier is not rated for {} Ω.", number_text(o.ohms)));
        } else if r.clip_db > 0.05 {
            notes.push(format!("The amplifier clips {} dB before the limiter acts.", fixed(r.clip_db, 1)));
        }
        if o.speaker_w > 0.0 && r.watts > o.speaker_w * 1.001 {
            notes.push(format!("{} is above the speaker rating ({}).", watts_text(r.watts), watts_text(o.speaker_w)));
        }
        notes
    });
    let watts = move || result.get().map(|r| watts_text(r.watts)).unwrap_or_default();
    let watts_detail = move || {
        let (Some(r), Some(ohms)) = (result.get(), ohms.get()) else { return String::new() };
        let share = if r.rated_watts > 0.0 {
            format!(" – {} % of the amplifier", (r.watts / r.rated_watts * 100.0).round())
        } else {
            String::new()
        };
        format!("into {} Ω at the threshold{share}", number_text(ohms))
    };

    let details = move || match status.get() {
        Status::NoAmp => {
            let text = if amp_list.with(Vec::is_empty) {
                "Add the amplifiers you use with \"Amplifiers …\", then choose the one behind this output."
            } else {
                "Choose the amplifier behind this output to see the power the limiter allows."
            };
            view! { <div class="calc-note muted">{text}</div> }.into_any()
        }
        Status::NoGain => {
            // The input sensitivity refers to the rated power: without one it says nothing.
            let text = if amp.with(|a| a.as_ref().is_some_and(|a| a.sensitivity_v.is_some())) {
                "The amplifier needs a rated power: its input sensitivity refers to it."
            } else {
                "The amplifier needs an input sensitivity or a gain."
            };
            view! { <div class="calc-note warn">{text}</div> }.into_any()
        }
        Status::Ready => view! {
            <div class="calc-result">
                <span class="calc-watts mono">{watts}</span>
                <span class="muted">{watts_detail}</span>
            </div>
            {move || notes.get().into_iter().map(|n| view! { <div class="calc-note warn">{n}</div> }).collect_view()}
            <div class="wrap-row">
                <span class="small muted">"Limit to"</span>
                {move || {
                    targets
                        .get()
                        .into_iter()
                        .map(|(label, w)| {
                            view! {
                                <button class="btn outlined compact" on:click=move |_| limit_to(w)>
                                    {format!("{label} · {}", watts_text(w))}
                                </button>
                            }
                        })
                        .collect_view()
                }}
                <NumberInput value=custom_w placeholder="W" title="Power to limit to (W)" on_set=set_custom/>
                <button class="btn text compact" prop:disabled=move || custom_w.with(Option::is_none)
                    on:click=move |_| if let Some(w) = custom_w.get_untracked() { limit_to(w) }>"Set"</button>
            </div>
        }
        .into_any(),
    };

    let amp_options = move || {
        amp_list
            .get()
            .into_iter()
            .map(|(id, name)| {
                let value = id.clone();
                let name = if name.trim().is_empty() { "Amplifier".to_string() } else { name };
                view! {
                    <option value=value prop:selected=move || output.with(|o| o.as_ref().is_some_and(|o| o.amp == id))>
                        {name}
                    </option>
                }
            })
            .collect_view()
    };

    let timing = move |_| {
        let hz = low_hz.get_untracked();
        let (attack, release) = timing_for_band(hz);
        on_timing.run((attack as i64, release as i64));
        app().toast(format!("Attack {attack} ms, release {release} ms (for {} Hz)", hz.round()), false);
    };
    let close_editor = Callback::new(move |_| editing.set(false));

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
                    <select class="select" on:change=on_amp>
                        <option value="" prop:selected=move || output.with(Option::is_none)>"—"</option>
                        {amp_options}
                    </select>
                    <Show when=move || output.with(Option::is_some)>
                        <span class="small muted">"Load Ω"</span>
                        <NumberInput value=ohms placeholder="8" title="Nominal load on this amplifier channel (Ω)" on_set=set_ohms/>
                        <span class="small muted">"Speaker W"</span>
                        <NumberInput value=speaker_w placeholder="RMS" title="Rated (RMS / AES) power of the loudspeaker" on_set=set_speaker/>
                    </Show>
                </div>
                {details}
                <div class="wrap-row">
                    <button class="btn text compact" on:click=timing
                        title="Attack about one period of the lowest frequency of the output (its HPF, 20 Hz without), release 16 × attack">
                        <Icon icon=I::Timer size=16/>"Timing from the HPF"
                    </button>
                    <span class="spacer"></span>
                    <span class="small muted" title="Output level at limiter threshold 0 dB. Not published for the DSP; measurements put it near 0 dBu. Check it with a measurement.">
                        "0 dB threshold ="
                    </span>
                    <NumberInput value=reference_value placeholder="0" on_set=set_reference/>
                    <span class="small muted">"dBu"</span>
                </div>
                <div class="calc-help">
                    "A planning aid from data sheet values: verify the result with a measurement."
                </div>
            </div>
        </PanelCard>
        {move || editing.get().then(|| view! {
            <AmpEditor setup=setup on_change=on_change on_close=close_editor/>
        })}
    }
}

/// Edit the amplifier library; Save replaces it as a whole.
#[component]
fn AmpEditor(#[prop(into)] setup: Signal<LimiterSetup>, on_change: Callback<LimiterSetup>, on_close: Callback<()>) -> impl IntoView {
    let amps = RwSignal::new(setup.get_untracked().amps);
    let save = move |_| {
        let mut next = setup.get_untracked();
        next.amps = amps.get_untracked();
        // Outputs of removed amplifiers lose their amplifier (as on the server).
        let ids: Vec<String> = next.amps.iter().map(|a| a.id.clone()).collect();
        next.outputs.retain(|_, o| ids.contains(&o.amp));
        on_change.run(next);
        on_close.run(());
    };
    let add = move |_| {
        amps.update(|list| {
            let mut n = js_sys::Date::now() as u64 % 1_000_000_000;
            while list.iter().any(|a| a.id == format!("amp{n}")) {
                n += 1;
            }
            let name = format!("Amplifier {}", list.len() + 1);
            list.push(Amplifier {
                id: format!("amp{n}"),
                name,
                watts8: 0.0,
                watts4: 0.0,
                watts2: 0.0,
                sensitivity_v: Some(1.4),
                gain_db: None,
            });
        })
    };
    let full = Memo::new(move |_| amps.with(|l| l.len() >= MAX_AMPS));
    let ids = move || amps.with(|l| l.iter().map(|a| a.id.clone()).collect::<Vec<_>>());
    view! {
        <Modal title="Amplifiers" width=720 on_close=on_close
            actions=view! {
                <button class="btn text" on:click=move |_| on_close.run(())>"Cancel"</button>
                <button class="btn filled" on:click=save>"Save"</button>
            }.into_any()>
            <div class="note-text muted">"Rated power per channel from the data sheet, and the input sensitivity (V for rated power) or the voltage gain."</div>
            <Show when=move || amps.with(|l| !l.is_empty())
                fallback=|| view! { <div class="calc-note muted">"No amplifiers yet."</div> }>
                <div class="amp-row amp-head small muted">
                    <span class="amp-name">"Name"</span><span>"8 Ω"</span><span>"4 Ω"</span><span>"2 Ω"</span><span>"Input"</span><span></span><span></span>
                </div>
            </Show>
            // Keyed rows: an edit does not rebuild the fields (focus stays).
            <For each=ids key=|id| id.clone() let:id>
                <AmpRow amps=amps id=id/>
            </For>
            <button class="btn text compact amp-add" prop:disabled=move || full.get() on:click=add>
                <Icon icon=I::Add size=16/>"Add amplifier"
            </button>
        </Modal>
    }
}

/// One amplifier of the library editor.
#[component]
fn AmpRow(amps: RwSignal<Vec<Amplifier>>, id: String) -> impl IntoView {
    let id = StoredValue::new(id);
    let read = move |f: fn(&Amplifier) -> Option<f64>| -> Signal<Option<f64>> {
        Memo::new(move |_| id.with_value(|id| amps.with(|l| l.iter().find(|a| a.id == *id).and_then(f)))).into()
    };
    let edit = move |f: &dyn Fn(&mut Amplifier)| {
        let id = id.get_value();
        amps.update(|l| {
            if let Some(a) = l.iter_mut().find(|a| a.id == id) {
                f(a);
            }
        });
    };
    let name =
        Memo::new(move |_| id.with_value(|id| amps.with(|l| l.iter().find(|a| a.id == *id).map(|a| a.name.clone()))));
    let by_gain = Memo::new(move |_| {
        id.with_value(|id| amps.with(|l| l.iter().find(|a| a.id == *id).is_some_and(|a| a.sensitivity_v.is_none())))
    });
    let w8 = read(|a| Some(a.watts8).filter(|w| *w > 0.0));
    let w4 = read(|a| Some(a.watts4).filter(|w| *w > 0.0));
    let w2 = read(|a| Some(a.watts2).filter(|w| *w > 0.0));
    let input = read(|a| a.sensitivity_v.or(a.gain_db));
    let unit = Signal::derive(move || (if by_gain.get() { "dB" } else { "V" }).to_string());

    let watts = |v: Option<f64>| v.unwrap_or(0.0).clamp(0.0, MAX_WATTS);
    let set_w8 = Callback::new(move |v: Option<f64>| edit(&|a| a.watts8 = watts(v)));
    let set_w4 = Callback::new(move |v: Option<f64>| edit(&|a| a.watts4 = watts(v)));
    let set_w2 = Callback::new(move |v: Option<f64>| edit(&|a| a.watts2 = watts(v)));
    let set_input = Callback::new(move |v: Option<f64>| {
        let Some(v) = v else { return };
        if by_gain.get_untracked() {
            edit(&|a| a.gain_db = Some(v.clamp(0.0, 60.0)));
        } else if v > 0.0 {
            edit(&|a| a.sensitivity_v = Some(v.clamp(0.05, 20.0)));
        }
    });
    let set_name = move |ev: Event| {
        let v: String = event_target_value(&ev).trim().chars().take(40).collect();
        edit(&|a| a.name = v.clone());
    };
    let set_mode = move |ev: Event| {
        let gain = event_target_value(&ev) == "gain";
        edit(&|a| {
            if gain {
                a.sensitivity_v = None;
                a.gain_db = Some(a.gain_db.unwrap_or(32.0));
            } else {
                a.gain_db = None;
                a.sensitivity_v = Some(a.sensitivity_v.unwrap_or(1.4));
            }
        });
    };
    let remove = move |_| {
        let id = id.get_value();
        amps.update(|l| l.retain(|a| a.id != id));
    };

    view! {
        <div class="amp-row">
            <input class="calc-input amp-name" type="text" maxlength="40" placeholder="Name"
                prop:value=move || name.get().unwrap_or_default() on:change=set_name/>
            <NumberInput value=w8 placeholder="W 8 Ω" title="Rated power at 8 Ω (W)" class="amp-w" on_set=set_w8/>
            <NumberInput value=w4 placeholder="W 4 Ω" title="Rated power at 4 Ω (W)" class="amp-w" on_set=set_w4/>
            <NumberInput value=w2 placeholder="W 2 Ω" title="Rated power at 2 Ω (W)" class="amp-w" on_set=set_w2/>
            <select class="select amp-mode" on:change=set_mode>
                <option value="sens" prop:selected=move || !by_gain.get()>"Sensitivity V"</option>
                <option value="gain" prop:selected=move || by_gain.get()>"Gain dB"</option>
            </select>
            <NumberInput value=input placeholder=unit class="amp-value" on_set=set_input/>
            <button class="icon-button small amp-remove" title="Remove" on:click=remove>
                <Icon icon=I::Delete size=16/>
            </button>
        </div>
    }
}
