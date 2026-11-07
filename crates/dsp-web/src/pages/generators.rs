//! Controls of the test signal generators (RTA page and Auto-EQ): the browser
//! generator and the DSP's own generator.

use std::cell::Cell;

use dsp_core::api::{TEST_LEVEL_MAX_DB, TEST_LEVEL_MIN_DB, TEST_START_LEVEL_DB};
use dsp_core::eq_math::GEQ_CENTER_FREQUENCIES;
use dsp_core::protocol as p;
use leptos::prelude::*;

use crate::components::common::{Modal, Segmented, Slider};
use crate::components::fader::Fader;
use crate::components::value_field::ValueField;
use crate::fmt::{db_mark_label, format_db_value, format_hz, parse_frequency, parse_number};
use crate::icons::{I, Icon};
use crate::store::Device;
use crate::taper::Taper;
use crate::tone::{MAX_LEVEL_DB, MIN_LEVEL_DB, ToneGenerator, ToneSignal};

thread_local! {
    /// The warning before the first start is shown once per page load.
    static DSP_GENERATOR_CONFIRMED: Cell<bool> = const { Cell::new(false) };
}

/// '20 Hz', '31.5 Hz', '1.25 kHz'
fn iso_label(hz: f64) -> String {
    if hz >= 1000.0 { format!("{} kHz", hz / 1000.0) } else { format!("{hz} Hz") }
}

/// The DSP's own signal generator (cmd 0x39 like the original editor; not
/// tested on a device). "Analog input" switches it off. While it runs the
/// server keeps all outputs at the test level (the gains from before come back
/// when it stops).
#[component]
pub fn DspGeneratorPanel(device: Device) -> impl IntoView {
    let tone_type = Memo::new(move |_| device.state.with(|s| s.test_tone_type));
    let test_level = Memo::new(move |_| device.info.with(|i| i.test_level));
    // Only this rebuilds the fader, not every new level.
    let test_running = Memo::new(move |_| test_level.with(Option::is_some));
    let param = Memo::new(move |_| {
        device.state.with(|s| s.test_tone_param.clamp(0, GEQ_CENTER_FREQUENCIES.len() as i64 - 1))
    });
    let confirm = RwSignal::new(None::<i64>);
    let set = Callback::new(move |t: usize| {
        let t = t as i64;
        if t != 0 && tone_type.get_untracked() == 0 && !DSP_GENERATOR_CONFIRMED.with(Cell::get) {
            confirm.set(Some(t));
            return;
        }
        device.set_test_tone(t, None);
    });
    let frequencies = move || {
        GEQ_CENTER_FREQUENCIES
            .iter()
            .enumerate()
            .map(|(i, hz)| view! { <option value=i selected=move || param.get() == i as i64>{iso_label(*hz)}</option> })
            .collect_view()
    };
    view! {
        <div class="wrap-row">
            <Segmented labels=p::TEST_TONE_TYPE_NAMES.to_vec()
                selected=Signal::derive(move || tone_type.get() as usize) on_select=set/>
            <Show when=move || tone_type.get() == 3>
                <div class="row tight">
                    <span class="small muted">"Freq"</span>
                    <select class="select mono" on:change=move |ev| {
                        if let Ok(i) = event_target_value(&ev).parse::<i64>() {
                            device.set_test_tone(3, Some(i));
                        }
                    }>
                        {frequencies()}
                    </select>
                    <span class="muted" title="The 31 ISO 1/3-octave frequencies of the original editor">
                        <Icon icon=I::Info size=14/>
                    </span>
                </div>
            </Show>
            <Show when=move || tone_type.get() != 0>
                <span class="small" style="color:var(--warning)">"generator running"</span>
            </Show>
            {move || test_running.get().then(|| {
                let level = Signal::derive(move || test_level.get().unwrap_or(TEST_START_LEVEL_DB));
                view! {
                    <div class="row tight test-level">
                        <span class="small muted">"Test level"</span>
                        <div class="test-level-fader">
                            <Fader vertical=false value=level
                                taper=Taper::Linear(TEST_LEVEL_MIN_DB, TEST_LEVEL_MAX_DB)
                                marks=vec![-60.0, -50.0, -40.0, -30.0, -20.0, -10.0, 0.0] major=vec![TEST_START_LEVEL_DB]
                                label={db_mark_label as fn(f64) -> String} color="var(--warning)"
                                step=Signal::derive(|| 1.0) reset=TEST_START_LEVEL_DB
                                on_change=Callback::new(move |v: f64| device.set_test_level(v))/>
                        </div>
                        <ValueField width=64 text=Signal::derive(move || format!("{} dB", format_db_value(level.get())))
                            title="All outputs while the generator runs (-60 … 0 dB)"
                            on_submit=Callback::new(move |input: String| match parse_number(&input) {
                                Some(v) => {
                                    device.set_test_level(v);
                                    true
                                }
                                None => false,
                            })/>
                    </div>
                }
            })}
        </div>
        {move || confirm.get().map(|t| {
            let close = Callback::new(move |_| confirm.set(None));
            view! {
                <Modal title="Start the DSP generator?" on_close=close actions=view! {
                    <button class="btn text" on:click=move |_| confirm.set(None)>"Cancel"</button>
                    <button class="btn filled" on:click=move |_| {
                        DSP_GENERATOR_CONFIRMED.with(|c| c.set(true));
                        confirm.set(None);
                        device.set_test_tone(t, None);
                    }>"Start"</button>
                }.into_any()>
                    <div class="note-text">
                        "The DSP plays the test signal itself – depending on routing it can reach the speakers. The panel first saves all output gains and lowers every output to -50 dB; the test level sets them while it runs. Switching the generator off stops it first and then restores the saved gains."
                    </div>
                    <div class="note-text">
                        "The command is the one of the original editor but has not been tested on a device."
                    </div>
                </Modal>
            }
        })}
    }
}

/// Play/stop, signal, frequency (sine) and level of the browser generator.
#[component]
pub fn ToneGeneratorPanel(
    /// Show the frequency control (hidden for Auto-EQ, which uses pink noise).
    #[prop(default = true)]
    show_frequency: bool,
) -> impl IntoView {
    let generator = ToneGenerator::get();
    let playing = generator.playing;
    let signal_index =
        Signal::derive(move || ToneSignal::ALL.iter().position(|s| *s == generator.signal.get()).unwrap_or(0));
    let frequency = move || {
        (show_frequency && generator.signal.get() == ToneSignal::Sine).then(|| {
            view! {
                <div class="row tight">
                    <span class="small muted">"Freq"</span>
                    <Slider width=180 min=0.0 max=1.0
                        value=Signal::derive(move || (generator.frequency.get() / 20.0).ln() / 1000f64.ln())
                        on_input=Callback::new(move |v: f64| generator.set_frequency(20.0 * 1000f64.powf(v)))/>
                    <ValueField width=72 text=Signal::derive(move || format_hz(generator.frequency.get()))
                        on_submit=Callback::new(move |input: String| match parse_frequency(&input) {
                            Some(hz) if hz > 0.0 => {
                                generator.set_frequency(hz);
                                true
                            }
                            _ => false,
                        })/>
                </div>
            }
        })
    };
    view! {
        <div class="wrap-row">
            <button class="btn compact" style=move || format!("background:{};color:var(--bg-deep)",
                    if playing.get() { "var(--danger)" } else { "var(--warning)" })
                on:click=move |_| generator.toggle()>
                {move || if playing.get() { view! { <Icon icon=I::Stop size=18/> } } else { view! { <Icon icon=I::VolumeUp size=18/> } }}
                {move || if playing.get() { "Stop signal" } else { "Play signal" }}
            </button>
            <Segmented labels=ToneSignal::ALL.iter().map(|s| s.label()).collect() selected=signal_index
                on_select=Callback::new(move |i: usize| generator.set_signal(ToneSignal::ALL[i]))/>
            {frequency}
            <div class="row tight">
                <span class="small muted">"Level"</span>
                <Slider min=MIN_LEVEL_DB max=MAX_LEVEL_DB value=generator.level_db
                    on_input=Callback::new(move |v: f64| generator.set_level(v))/>
                <span class="small mono">{move || format!("{} dBFS", generator.level_db.get().round() as i64)}</span>
            </div>
        </div>
    }
}
