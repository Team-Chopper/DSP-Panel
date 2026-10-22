//! Web-panel API.
//!
//! Messages between browser and server are JSON objects with a "t" (type)
//! field, sent over a single WebSocket at /ws. The format is the one of the
//! Dart implementation, so the Flutter client and the Rust server (and the
//! other way round) work together.
//!
//! ```text
//! Client → server
//!   {t: subscribe,   dsp}                 full state + log, then live updates
//!   {t: unsubscribe, dsp}
//!   {t: addDsp,      name, host, port, model, deviceId?, password?}
//!                    model: auto|dsp408|dsp206
//!   {t: updateDsp,   id, name?, host?, port?, enabled?, model?, deviceId?, password?}
//!   {t: removeDsp,   id}
//!   {t: op,          dsp, op}             see ops.rs
//!   {t: link,        dsp, master, slaves} link like the original editor
//!                                         (0x2A copy, 0x3B, re-read)
//!   {t: copy,        dsp, from, to: [..]} copy on the device (0x2A, re-read)
//!   {t: loadPreset,  dsp, index}
//!   {t: savePreset,  dsp, index, name}
//!   {t: setRefresh,  dsp, ms}
//!   {t: debug,       dsp, rx, tx}         packet logging on/off
//!   {t: clearLog,    dsp}
//!
//! Presets stored on the server (edited offline, loaded onto a DSP)
//!   {t: presetCreate, name, model?, fromDsp?}   empty preset or snapshot of a DSP
//!   {t: presetRename, preset, name}
//!   {t: presetDelete, preset}
//!   {t: presetApply,  preset, dsp}              send all values to the DSP
//!   subscribe / unsubscribe / op with dsp: "preset:<id>" edit a preset offline
//!
//! Server → client
//!   {t: hello,      dsps: [DspInfo]}
//!   {t: dspInfo,    info: DspInfo}        added or changed
//!   {t: dspRemoved, id}
//!   {t: state,      dsp, state}           full DeviceState
//!   {t: op,         dsp, op}              change made by another client
//!   {t: meters,     dsp, v: [dBFS per channel], dyn, tx, rx}
//!                   dyn: bit n = compressor, bit 8+n = limiter of output n active
//!   {t: log,        dsp, entries: [LogEntry], reset?}
//!   {t: presets,    presets: [ServerPresetInfo]}  on connect and on every change
//!   {t: error,      dsp?, message}
//! ```

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Message types ("t").
pub mod msg {
    pub const SUBSCRIBE: &str = "subscribe";
    pub const UNSUBSCRIBE: &str = "unsubscribe";
    pub const ADD_DSP: &str = "addDsp";
    pub const UPDATE_DSP: &str = "updateDsp";
    pub const REMOVE_DSP: &str = "removeDsp";
    pub const OP: &str = "op";
    pub const LINK: &str = "link";
    pub const COPY: &str = "copy";
    pub const LOAD_PRESET: &str = "loadPreset";
    pub const SAVE_PRESET: &str = "savePreset";
    pub const SET_REFRESH: &str = "setRefresh";
    pub const DEBUG: &str = "debug";
    pub const CLEAR_LOG: &str = "clearLog";
    pub const PRESET_CREATE: &str = "presetCreate";
    pub const PRESET_RENAME: &str = "presetRename";
    pub const PRESET_DELETE: &str = "presetDelete";
    pub const PRESET_APPLY: &str = "presetApply";

    /// Prefix of the "dsp" id that addresses a server preset (offline editor).
    pub const PRESET_PREFIX: &str = "preset:";

    pub const HELLO: &str = "hello";
    pub const DSP_INFO: &str = "dspInfo";
    pub const DSP_REMOVED: &str = "dspRemoved";
    pub const STATE: &str = "state";
    pub const METERS: &str = "meters";
    pub const LOG: &str = "log";
    pub const PRESETS: &str = "presets";
    pub const ERROR: &str = "error";
}

/// Default meter polling interval in ms (allowed: 5–300), the interval of the
/// original editor.
pub const DEFAULT_REFRESH_MS: i64 = 300;

/// A preset stored on the server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerPresetInfo {
    pub id: String,
    pub name: String,
    /// Model id the preset belongs to.
    pub model: String,
    /// Last change, milliseconds since epoch.
    pub updated: i64,
}

/// Connection status of a DSP as seen by the server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DspStatus {
    /// Not connected and not trying (disabled).
    #[default]
    Disconnected,
    /// Opening the TCP connection.
    Connecting,
    /// Running the init sequence.
    Initializing,
    /// Ready for commands.
    Ready,
    /// Loading or saving a preset.
    Busy,
    /// Connection failed; the server retries automatically.
    Error,
}

impl DspStatus {
    pub fn name(self) -> &'static str {
        match self {
            DspStatus::Disconnected => "disconnected",
            DspStatus::Connecting => "connecting",
            DspStatus::Initializing => "initializing",
            DspStatus::Ready => "ready",
            DspStatus::Busy => "busy",
            DspStatus::Error => "error",
        }
    }
}

fn default_refresh() -> i64 {
    DEFAULT_REFRESH_MS
}
fn default_model() -> String {
    "dsp408".into()
}
fn default_model_setting() -> String {
    "auto".into()
}
fn default_true() -> bool {
    true
}
fn default_device_id() -> i64 {
    1
}

/// Public description of one configured DSP and its connection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DspInfo {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: i64,
    pub enabled: bool,
    #[serde(default)]
    pub status: DspStatus,
    /// Progress of init or preset load/save, 0.0 – 1.0.
    #[serde(default)]
    pub progress: f64,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default = "default_refresh")]
    pub refresh_ms: i64,
    #[serde(default)]
    pub log_rx: bool,
    #[serde(default)]
    pub log_tx: bool,
    /// Model in use ("dsp408", "dsp206"): detected from the device, or the
    /// configured one.
    #[serde(default = "default_model")]
    pub model: String,
    /// Configured model: "auto" (detect) or a model id.
    #[serde(default = "default_model_setting")]
    pub model_setting: String,
    /// False if the config dump could not be decoded plausibly; the panel then
    /// shows default values and a warning.
    #[serde(default = "default_true")]
    pub config_readable: bool,
    /// Device ID the DSP answers from (1 … 250).
    #[serde(default = "default_device_id")]
    pub device_id: i64,
    /// A password for a locked DSP is stored (the password itself is not sent
    /// to clients).
    #[serde(default)]
    pub has_password: bool,
}

impl DspInfo {
    pub fn new(id: &str, name: &str, host: &str, port: i64, enabled: bool) -> Self {
        DspInfo {
            id: id.into(),
            name: name.into(),
            host: host.into(),
            port,
            enabled,
            status: DspStatus::Disconnected,
            progress: 0.0,
            message: None,
            refresh_ms: DEFAULT_REFRESH_MS,
            log_rx: false,
            log_tx: false,
            model: default_model(),
            model_setting: default_model_setting(),
            config_readable: true,
            device_id: 1,
            has_password: false,
        }
    }
}

/// One line in a DSP's debug log. Packet entries carry both a raw hex and a
/// decoded representation so the client can toggle between them.
#[derive(Clone, Debug, PartialEq)]
pub struct LogEntry {
    /// Seconds since the connection was opened.
    pub timestamp: f64,
    pub text: String,
    pub raw_hex: Option<String>,
    pub decoded: Option<String>,
}

impl LogEntry {
    pub fn status(text: impl Into<String>, timestamp: f64) -> Self {
        LogEntry { timestamp, text: text.into(), raw_hex: None, decoded: None }
    }

    pub fn packet(raw_hex: String, decoded: String, timestamp: f64) -> Self {
        LogEntry { timestamp, text: String::new(), raw_hex: Some(raw_hex), decoded: Some(decoded) }
    }

    /// The string to display, given the current decode toggle.
    pub fn display(&self, decode_on: bool) -> &str {
        match (&self.raw_hex, &self.decoded) {
            (Some(hex), decoded) => {
                if decode_on {
                    decoded.as_deref().unwrap_or(hex)
                } else {
                    hex
                }
            }
            _ => &self.text,
        }
    }

    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("ts".into(), Value::from(self.timestamp));
        match &self.raw_hex {
            None => {
                m.insert("text".into(), Value::from(self.text.clone()));
            }
            Some(hex) => {
                m.insert("hex".into(), Value::from(hex.clone()));
            }
        }
        if let Some(dec) = &self.decoded {
            m.insert("dec".into(), Value::from(dec.clone()));
        }
        Value::Object(m)
    }

    pub fn from_json(json: &Value) -> Result<Self, String> {
        let ts = json.get("ts").and_then(Value::as_f64).ok_or("ts must be a number")?;
        if let Some(hex) = json.get("hex").and_then(Value::as_str) {
            let dec = json.get("dec").and_then(Value::as_str).unwrap_or_default();
            return Ok(LogEntry::packet(hex.into(), dec.into(), ts));
        }
        let text = json.get("text").and_then(Value::as_str).ok_or("text must be a string")?;
        Ok(LogEntry::status(text, ts))
    }
}

impl Serialize for LogEntry {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_json().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LogEntry {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        LogEntry::from_json(&value).map_err(serde::de::Error::custom)
    }
}
