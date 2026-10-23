//! Real-time analyser: microphone input selection, capture and analysis.
//!
//! An AudioWorklet (`audio/rta-worklet.js`) hands the microphone samples over
//! in blocks of 4096; the analysis (`dsp_core::rta`) runs here in WebAssembly,
//! about 12 spectra per second. Pages draw the spectrum when [Rta::frame]
//! changes, nothing else is reactive.

use std::cell::Cell;

use dsp_core::rta::{Analyzer, Averaging, PREFERRED_SAMPLE_RATE};
use js_sys::{Array, Float32Array, Object, Reflect};
use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AudioContext, AudioContextOptions, AudioContextState, AudioWorkletNode, AudioWorkletNodeOptions, MediaStream,
    MediaStreamAudioSourceNode, MediaStreamConstraints, MediaStreamTrack, MessageEvent,
};

use crate::local_settings;

thread_local! {
    static SETTINGS: Cell<Option<RtaSettings>> = const { Cell::new(None) };
}

/// The microphone to use, remembered per browser.
#[derive(Clone, Copy)]
pub struct RtaSettings {
    /// (deviceId, label) of the audio inputs.
    pub devices: RwSignal<Vec<(String, String)>>,
    pub selected: RwSignal<Option<String>>,
}

impl RtaSettings {
    /// The settings; created by the root component, so they live as long as
    /// the panel.
    pub fn get() -> RtaSettings {
        SETTINGS.with(|s| {
            if let Some(settings) = s.get() {
                return settings;
            }
            let settings = RtaSettings {
                devices: RwSignal::new(Vec::new()),
                selected: RwSignal::new(local_settings::get_string("rtaInputDevice")),
            };
            s.set(Some(settings));
            settings
        })
    }

    pub fn select_device(&self, id: String) {
        local_settings::set_string("rtaInputDevice", &id);
        self.selected.set(Some(id));
    }

    /// List the audio inputs (labels are only known after the microphone was
    /// allowed once).
    pub fn enumerate_devices(&self) {
        let devices = self.devices;
        leptos::task::spawn_local(async move {
            let Some(media) = web_sys::window().and_then(|w| w.navigator().media_devices().ok()) else { return };
            let Ok(promise) = media.enumerate_devices() else { return };
            let Ok(list) = JsFuture::from(promise).await else { return };
            let list: Array = list.unchecked_into();
            let mut inputs = Vec::new();
            for item in list.iter() {
                let info: web_sys::MediaDeviceInfo = item.unchecked_into();
                if info.kind() == web_sys::MediaDeviceKind::Audioinput {
                    let label = info.label();
                    let label = if label.is_empty() { format!("Microphone {}", inputs.len() + 1) } else { label };
                    inputs.push((info.device_id(), label));
                }
            }
            devices.set(inputs);
        });
    }
}

// ─── Capture ───

/// The running audio graph. Dropping it releases the microphone.
struct Capture {
    context: AudioContext,
    stream: MediaStream,
    source: MediaStreamAudioSourceNode,
    node: AudioWorkletNode,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
}

impl Drop for Capture {
    fn drop(&mut self) {
        if let Ok(port) = self.node.port() {
            port.set_onmessage(None);
        }
        let _ = self.source.disconnect();
        let _ = self.node.disconnect();
        stop_tracks(&self.stream);
        let _ = self.context.close();
    }
}

fn stop_tracks(stream: &MediaStream) {
    for track in stream.get_tracks().iter() {
        track.unchecked_into::<MediaStreamTrack>().stop();
    }
}

/// Readable text of a rejected promise (DOMException name + message).
fn describe(error: &JsValue) -> String {
    let field = |name: &str| Reflect::get(error, &name.into()).ok().and_then(|v| v.as_string()).unwrap_or_default();
    match field("name").as_str() {
        "NotAllowedError" | "SecurityError" => "Microphone access was denied".into(),
        "NotFoundError" | "OverconstrainedError" => "No microphone found".into(),
        "NotReadableError" => "The microphone is in use by another application".into(),
        "" => error.as_string().unwrap_or_else(|| "unknown error".into()),
        name => {
            let message = field("message");
            if message.is_empty() { name.to_string() } else { format!("{name}: {message}") }
        }
    }
}

/// Context at [rate] (None: the device rate) with the stream connected.
/// Firefox cannot connect a stream to a context of another rate.
fn context_for(stream: &MediaStream, rate: Option<f32>) -> Result<(AudioContext, MediaStreamAudioSourceNode), JsValue> {
    let context = match rate {
        Some(rate) => {
            let options = AudioContextOptions::new();
            options.set_sample_rate(rate);
            AudioContext::new_with_context_options(&options)?
        }
        None => AudioContext::new()?,
    };
    match context.create_media_stream_source(stream) {
        Ok(source) => Ok((context, source)),
        Err(e) => {
            let _ = context.close();
            Err(e)
        }
    }
}

async fn open_stream(device_id: Option<String>) -> Result<MediaStream, String> {
    let media = web_sys::window()
        .and_then(|w| w.navigator().media_devices().ok())
        .ok_or("This browser offers no microphone here (HTTPS needed)")?;
    let audio = Object::new();
    for key in ["echoCancellation", "noiseSuppression", "autoGainControl"] {
        let _ = Reflect::set(&audio, &key.into(), &false.into());
    }
    let _ = Reflect::set(&audio, &"channelCount".into(), &1.into());
    if let Some(id) = device_id.filter(|id| !id.is_empty()) {
        let _ = Reflect::set(&audio, &"deviceId".into(), &id.into());
    }
    let constraints = MediaStreamConstraints::new();
    constraints.set_audio(&audio);
    let promise = media.get_user_media_with_constraints(&constraints).map_err(|e| describe(&e))?;
    let stream = JsFuture::from(promise).await.map_err(|e| describe(&e))?;
    Ok(stream.unchecked_into())
}

async fn open_graph(
    stream: &MediaStream,
) -> Result<(AudioContext, MediaStreamAudioSourceNode, AudioWorkletNode), JsValue> {
    let (context, source) =
        context_for(stream, Some(PREFERRED_SAMPLE_RATE as f32)).or_else(|_| context_for(stream, None))?;
    let result = async {
        JsFuture::from(context.audio_worklet()?.add_module("audio/rta-worklet.js")?).await?;
        let options = AudioWorkletNodeOptions::new();
        options.set_number_of_inputs(1);
        options.set_number_of_outputs(1);
        options.set_output_channel_count(&Array::of1(&1.into()));
        let node = AudioWorkletNode::new_with_options(&context, "rta-tap", &options)?;
        source.connect_with_audio_node(&node)?;
        // Connected to the output (silent) so that every browser runs it.
        node.connect_with_audio_node(&context.destination())?;
        if context.state() == AudioContextState::Suspended {
            JsFuture::from(context.resume()?).await?;
        }
        Ok::<_, JsValue>(node)
    }
    .await;
    match result {
        Ok(node) => Ok((context, source, node)),
        Err(e) => {
            let _ = context.close();
            Err(e)
        }
    }
}

/// Microphone capture with its analyser. Each view that shows a spectrum owns
/// one (RTA page, GEQ page, an Auto-EQ panel without a page RTA); it is
/// released when that view goes away.
#[derive(Clone, Copy)]
pub struct Rta {
    analyzer: StoredValue<Analyzer, LocalStorage>,
    capture: StoredValue<Option<Capture>, LocalStorage>,
    /// Counts the starts; a start that was overtaken by a stop is dropped.
    generation: StoredValue<u64>,
    pub running: RwSignal<bool>,
    pub starting: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
    /// Changes with every batch of new spectra.
    pub frame: RwSignal<u64>,
    pub sample_rate: RwSignal<f64>,
    pub averaging: RwSignal<Averaging>,
}

impl Rta {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Rta {
        let rta = Rta {
            analyzer: StoredValue::new_local(Analyzer::new(PREFERRED_SAMPLE_RATE)),
            capture: StoredValue::new_local(None),
            generation: StoredValue::new(0),
            running: RwSignal::new(false),
            starting: RwSignal::new(false),
            error: RwSignal::new(None),
            frame: RwSignal::new(0),
            sample_rate: RwSignal::new(PREFERRED_SAMPLE_RATE),
            averaging: RwSignal::new(Averaging::Fast),
        };
        on_cleanup(move || {
            rta.generation.try_update_value(|g| *g += 1);
            rta.capture.try_update_value(|c| *c = None);
        });
        rta
    }

    /// Start the microphone (asks for permission the first time).
    pub fn start(self) {
        if self.running.get_untracked() || self.starting.get_untracked() {
            return;
        }
        self.starting.set(true);
        self.error.set(None);
        self.generation.update_value(|g| *g += 1);
        let generation = self.generation.get_value();
        let settings = RtaSettings::get();
        let device_id = settings.selected.get_untracked();
        leptos::task::spawn_local(async move {
            let opened = match open_stream(device_id).await {
                Ok(stream) => match open_graph(&stream).await {
                    Ok(graph) => Ok((stream, graph)),
                    Err(e) => {
                        stop_tracks(&stream);
                        Err(format!("Microphone could not be started: {}", describe(&e)))
                    }
                },
                Err(e) => Err(e),
            };
            if self.generation.try_get_value() != Some(generation) {
                // Stopped or released meanwhile; dropping the graph closes it.
                if let Ok((stream, (context, ..))) = opened {
                    stop_tracks(&stream);
                    let _ = context.close();
                }
                return;
            }
            match opened {
                Ok((stream, (context, source, node))) => {
                    let rate = context.sample_rate() as f64;
                    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |ev: MessageEvent| {
                        let samples = ev.data().unchecked_into::<Float32Array>().to_vec();
                        let spectra = self.analyzer.try_update_value(|a| a.push(&samples)).unwrap_or(0);
                        if spectra > 0 {
                            self.frame.try_update(|f| *f += 1);
                        }
                    });
                    if let Ok(port) = node.port() {
                        port.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
                    }
                    self.analyzer.update_value(|a| {
                        *a = Analyzer::new(rate);
                        a.averaging = self.averaging.get_untracked();
                    });
                    self.capture.set_value(Some(Capture { context, stream, source, node, _on_message: on_message }));
                    self.sample_rate.set(rate);
                    self.running.set(true);
                    // Labels are known now that the microphone is allowed.
                    settings.enumerate_devices();
                }
                Err(e) => self.error.set(Some(e)),
            }
            self.starting.set(false);
        });
    }

    /// Stop the microphone and clear the display.
    pub fn stop(self) {
        self.generation.update_value(|g| *g += 1);
        self.capture.set_value(None);
        self.analyzer.update_value(Analyzer::clear);
        self.running.set(false);
        self.starting.set(false);
        self.frame.update(|f| *f += 1);
    }

    pub fn set_averaging(self, averaging: Averaging) {
        self.averaging.set(averaging);
        self.analyzer.update_value(|a| a.averaging = averaging);
    }

    pub fn reset_peaks(self) {
        self.analyzer.update_value(Analyzer::reset_peaks);
        self.frame.update(|f| *f += 1);
    }

    /// (levels, peaks) per 1/24-octave band in dB.
    pub fn with_spectrum<R>(self, f: impl FnOnce(&[f64], &[f64]) -> R) -> R {
        self.analyzer.with_value(|a| f(&a.magnitudes, &a.peaks))
    }

    // ─── Measurement (Auto-EQ) ───

    pub fn start_measurement(self) {
        self.analyzer.update_value(Analyzer::start_measurement);
    }

    pub fn measured_seconds(self) -> f64 {
        self.analyzer.try_with_value(Analyzer::measured_seconds).unwrap_or(0.0)
    }

    pub fn stop_measurement(self, centers: &[f64]) -> Option<Vec<f64>> {
        self.analyzer.try_update_value(|a| a.stop_measurement(centers)).flatten()
    }

    pub fn cancel_measurement(self) {
        self.analyzer.try_update_value(Analyzer::cancel_measurement);
    }
}
