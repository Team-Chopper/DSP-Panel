//! Test signal generator: sine, pink noise or white noise through the audio
//! output of the computer/phone the panel runs on (Web Audio). Connect that
//! output to a DSP input to measure the system with the RTA (pink noise +
//! Auto-EQ) or to check speakers. One generator for the whole panel.
//!
//! The DSP's own generator (cmd 0x39) is a separate control, see
//! `Device::set_test_tone`.

use std::cell::Cell;

use leptos::prelude::*;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{AudioBuffer, AudioContext, AudioContextState, AudioScheduledSourceNode, GainNode, OscillatorNode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToneSignal {
    Sine,
    Pink,
    White,
}

impl ToneSignal {
    pub const ALL: [ToneSignal; 3] = [ToneSignal::Sine, ToneSignal::Pink, ToneSignal::White];

    pub fn label(self) -> &'static str {
        match self {
            ToneSignal::Sine => "Sine",
            ToneSignal::Pink => "Pink noise",
            ToneSignal::White => "White noise",
        }
    }
}

/// RMS level range in dBFS.
pub const MIN_LEVEL_DB: f64 = -60.0;
pub const MAX_LEVEL_DB: f64 = -6.0;

thread_local! {
    static GENERATOR: Cell<Option<ToneGenerator>> = const { Cell::new(None) };
}

#[derive(Default)]
struct Backend {
    context: Option<AudioContext>,
    gain: Option<GainNode>,
    source: Option<AudioScheduledSourceNode>,
    oscillator: Option<OscillatorNode>,
    /// Noise buffers: [white, pink].
    noise: [Option<AudioBuffer>; 2],
}

/// Linear gain for an RMS level: noise buffers are normalised to RMS 1, a sine
/// of amplitude 1 has RMS 1/√2.
fn gain_for(signal: ToneSignal, level_db: f64) -> f32 {
    let rms = 10f64.powf(level_db / 20.0);
    (if signal == ToneSignal::Sine { rms * std::f64::consts::SQRT_2 } else { rms }) as f32
}

/// 10 s of white or pink noise (Paul Kellet's pink filter), RMS 1.
fn noise_buffer(context: &AudioContext, pink: bool) -> Result<AudioBuffer, JsValue> {
    let rate = context.sample_rate();
    let length = rate.round() as usize * 10;
    let mut data = vec![0f32; length];
    // xorshift64*, seeded from the browser.
    let mut state = (js_sys::Math::random() * u64::MAX as f64) as u64 | 1;
    let mut random = move || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    };
    let (mut b0, mut b1, mut b2, mut b3, mut b4, mut b5, mut b6) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    let mut sum_squares = 0.0;
    for d in data.iter_mut() {
        let white = random() * 2.0 - 1.0;
        let v = if pink {
            b0 = 0.99886 * b0 + white * 0.0555179;
            b1 = 0.99332 * b1 + white * 0.0750759;
            b2 = 0.96900 * b2 + white * 0.1538520;
            b3 = 0.86650 * b3 + white * 0.3104856;
            b4 = 0.55000 * b4 + white * 0.5329522;
            b5 = -0.7616 * b5 - white * 0.0168980;
            let v = b0 + b1 + b2 + b3 + b4 + b5 + b6 + white * 0.5362;
            b6 = white * 0.115926;
            v
        } else {
            white
        };
        *d = v as f32;
        sum_squares += v * v;
    }
    let scale = (1.0 / (sum_squares / length as f64).sqrt()) as f32;
    for d in data.iter_mut() {
        *d *= scale;
    }
    let buffer = context.create_buffer(1, length as u32, rate)?;
    buffer.copy_to_channel(&data, 0)?;
    Ok(buffer)
}

impl Backend {
    fn start(&mut self, signal: ToneSignal, frequency: f64, level_db: f64) -> Result<(), JsValue> {
        self.stop();
        if self.context.is_none() {
            self.context = Some(AudioContext::new()?);
        }
        let context = self.context.clone().expect("set above");
        if context.state() == AudioContextState::Suspended {
            let _ = context.resume();
        }
        let gain = context.create_gain()?;
        gain.gain().set_value(gain_for(signal, level_db));
        gain.connect_with_audio_node(&context.destination())?;
        let source: AudioScheduledSourceNode = if signal == ToneSignal::Sine {
            let osc = context.create_oscillator()?;
            osc.set_type(web_sys::OscillatorType::Sine);
            osc.frequency().set_value(frequency as f32);
            osc.connect_with_audio_node(&gain)?;
            self.oscillator = Some(osc.clone());
            osc.unchecked_into()
        } else {
            let pink = signal == ToneSignal::Pink;
            let slot = &mut self.noise[pink as usize];
            if slot.is_none() {
                *slot = Some(noise_buffer(&context, pink)?);
            }
            let src = context.create_buffer_source()?;
            src.set_buffer(slot.as_ref());
            src.set_loop(true);
            src.connect_with_audio_node(&gain)?;
            src.unchecked_into()
        };
        source.start()?;
        self.source = Some(source);
        self.gain = Some(gain);
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(source) = self.source.take() {
            let _ = source.stop();
            let _ = source.disconnect();
        }
        if let Some(gain) = self.gain.take() {
            let _ = gain.disconnect();
        }
        self.oscillator = None;
    }
}

#[derive(Clone, Copy)]
pub struct ToneGenerator {
    pub playing: RwSignal<bool>,
    pub signal: RwSignal<ToneSignal>,
    pub frequency: RwSignal<f64>,
    pub level_db: RwSignal<f64>,
    backend: StoredValue<Backend, LocalStorage>,
}

impl ToneGenerator {
    /// The generator; created by the root component, so it lives as long as
    /// the panel.
    pub fn get() -> ToneGenerator {
        GENERATOR.with(|g| {
            if let Some(generator) = g.get() {
                return generator;
            }
            let generator = ToneGenerator {
                playing: RwSignal::new(false),
                signal: RwSignal::new(ToneSignal::Pink),
                frequency: RwSignal::new(1000.0),
                level_db: RwSignal::new(-20.0),
                backend: StoredValue::new_local(Backend::default()),
            };
            g.set(Some(generator));
            generator
        })
    }

    pub fn start(self) {
        let (signal, frequency, level) =
            (self.signal.get_untracked(), self.frequency.get_untracked(), self.level_db.get_untracked());
        let started = self.backend.try_update_value(|b| b.start(signal, frequency, level));
        let ok = matches!(started, Some(Ok(())));
        if let Some(Err(e)) = started {
            leptos::logging::warn!("Tone generator: cannot start audio: {e:?}");
        }
        self.playing.set(ok);
    }

    pub fn stop(self) {
        self.backend.update_value(Backend::stop);
        self.playing.set(false);
    }

    pub fn toggle(self) {
        if self.playing.get_untracked() { self.stop() } else { self.start() }
    }

    pub fn set_signal(self, signal: ToneSignal) {
        if signal == self.signal.get_untracked() {
            return;
        }
        self.signal.set(signal);
        if self.playing.get_untracked() {
            self.start();
        }
    }

    pub fn set_frequency(self, hz: f64) {
        let hz = hz.clamp(20.0, 20000.0);
        self.frequency.set(hz);
        self.backend.with_value(|b| {
            if let Some(osc) = &b.oscillator {
                osc.frequency().set_value(hz as f32);
            }
        });
    }

    pub fn set_level(self, db: f64) {
        let db = db.clamp(MIN_LEVEL_DB, MAX_LEVEL_DB);
        self.level_db.set(db);
        let signal = self.signal.get_untracked();
        self.backend.with_value(|b| {
            if let Some(gain) = &b.gain {
                gain.gain().set_value(gain_for(signal, db));
            }
        });
    }
}
