//! The complete state of one DSP as shown in the panel. The server owns the
//! authoritative copy; clients hold a mirror that they update optimistically.
//! Both sides change it only through ops (see `ops`).
//!
//! Values are kept in protocol units where the protocol has no natural float
//! representation (frequencies, Q, times, thresholds, delay), so that a config
//! dump can be decoded and re-encoded without loss. The `protocol` module has
//! the conversions.
//!
//! The JSON form is the one of the Dart implementation (WebSocket `state`
//! message and `presets.json`), so both implementations can read each other's
//! data.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value, json};

use crate::model::{DeviceModel, GEQ_BAND_COUNT};

/// Error of [DeviceState::from_json] and the other JSON readers.
pub type JsonResult<T> = Result<T, String>;

pub(crate) fn as_int(v: &Value, what: &str) -> JsonResult<i64> {
    // Dart's `(x as num).toInt()`: integers, or doubles truncated.
    v.as_i64()
        .or_else(|| v.as_f64().filter(|f| f.is_finite()).map(|f| f as i64))
        .ok_or_else(|| format!("{what} must be a number"))
}

pub(crate) fn as_num(v: &Value, what: &str) -> JsonResult<f64> {
    v.as_f64().ok_or_else(|| format!("{what} must be a number"))
}

pub(crate) fn as_bool(v: &Value, what: &str) -> JsonResult<bool> {
    v.as_bool().ok_or_else(|| format!("{what} must be a bool"))
}

pub(crate) fn as_str<'a>(v: &'a Value, what: &str) -> JsonResult<&'a str> {
    v.as_str().ok_or_else(|| format!("{what} must be a string"))
}

fn field<'a>(json: &'a Value, key: &str) -> &'a Value {
    json.get(key).unwrap_or(&Value::Null)
}

fn obj<'a>(v: &'a Value, what: &str) -> JsonResult<&'a Map<String, Value>> {
    v.as_object().ok_or_else(|| format!("{what} must be an object"))
}

/// One PEQ band.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeqBand {
    /// 0 … 300, 20 Hz – 20 kHz
    pub freq_raw: i64,
    /// 0 … 100, Q 0.4 – 128
    pub q_raw: i64,
    /// −12.0 … +12.0 dB
    pub gain_db: f64,
    /// 0 … 8 (Peak, Low Shelf, …)
    #[serde(rename = "type")]
    pub peq_type: i64,
    pub bypass: bool,
}

impl Default for PeqBand {
    fn default() -> Self {
        PeqBand { freq_raw: 150, q_raw: 35, gain_db: 0.0, peq_type: 0, bypass: false }
    }
}

impl PeqBand {
    pub fn from_json(json: &Value) -> JsonResult<Self> {
        Ok(PeqBand {
            freq_raw: as_int(field(json, "freqRaw"), "freqRaw")?,
            q_raw: as_int(field(json, "qRaw"), "qRaw")?,
            gain_db: as_num(field(json, "gainDb"), "gainDb")?,
            peq_type: as_int(field(json, "type"), "type")?,
            bypass: as_bool(field(json, "bypass"), "bypass")?,
        })
    }
}

/// A hi or lo pass filter.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterState {
    /// 0 … 300, 20 Hz – 20 kHz
    pub freq_raw: i64,
    /// Index into `CROSSOVER_SLOPE_NAMES`; 0 = bypass.
    pub slope: i64,
}

impl FilterState {
    pub fn new(freq_raw: i64, slope: i64) -> Self {
        FilterState { freq_raw, slope }
    }

    /// The filter is active unless its slope is "bypass".
    pub fn enabled(&self) -> bool {
        self.slope != 0
    }

    pub fn from_json(json: &Value) -> JsonResult<Self> {
        Ok(FilterState {
            freq_raw: as_int(field(json, "freqRaw"), "freqRaw")?,
            slope: as_int(field(json, "slope"), "slope")?,
        })
    }
}

/// Noise gate of an input. Raw protocol values: times = ms − 1,
/// threshold = dB · 2 + 180.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GateState {
    pub attack: i64,
    pub release: i64,
    pub hold: i64,
    pub threshold: i64,
}

impl Default for GateState {
    fn default() -> Self {
        GateState { attack: 0, release: 99, hold: 99, threshold: 0 }
    }
}

impl GateState {
    pub fn from_json(json: &Value) -> JsonResult<Self> {
        Ok(GateState {
            attack: as_int(field(json, "attack"), "attack")?,
            release: as_int(field(json, "release"), "release")?,
            hold: as_int(field(json, "hold"), "hold")?,
            threshold: as_int(field(json, "threshold"), "threshold")?,
        })
    }
}

/// Compressor of an output. Raw protocol values: ratio index, times = ms − 1,
/// knee in dB, threshold = dB · 2 + 180.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CompressorState {
    pub ratio: i64,
    pub attack: i64,
    pub release: i64,
    pub knee: i64,
    pub threshold: i64,
}

impl Default for CompressorState {
    fn default() -> Self {
        CompressorState { ratio: 0, attack: 49, release: 499, knee: 0, threshold: 220 }
    }
}

impl CompressorState {
    pub fn from_json(json: &Value) -> JsonResult<Self> {
        Ok(CompressorState {
            ratio: as_int(field(json, "ratio"), "ratio")?,
            attack: as_int(field(json, "attack"), "attack")?,
            release: as_int(field(json, "release"), "release")?,
            knee: as_int(field(json, "knee"), "knee")?,
            threshold: as_int(field(json, "threshold"), "threshold")?,
        })
    }
}

/// Limiter of an output. Raw protocol values like [CompressorState]; the knee
/// (0 … 12 dB) is hidden in the original editor's UI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LimiterState {
    pub attack: i64,
    pub release: i64,
    pub knee: i64,
    pub threshold: i64,
}

impl Default for LimiterState {
    fn default() -> Self {
        LimiterState { attack: 49, release: 499, knee: 0, threshold: 220 }
    }
}

impl LimiterState {
    /// Older versions stored the knee as "unknown".
    pub fn from_json(json: &Value) -> JsonResult<Self> {
        let knee = [field(json, "knee"), field(json, "unknown")]
            .into_iter()
            .find(|v| !v.is_null())
            .map_or(Ok(0), |v| as_int(v, "knee"))?;
        Ok(LimiterState {
            attack: as_int(field(json, "attack"), "attack")?,
            release: as_int(field(json, "release"), "release")?,
            knee,
            threshold: as_int(field(json, "threshold"), "threshold")?,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceState {
    pub model: DeviceModel,

    /// Preset names by 0-based index (0 = U01).
    pub presets: BTreeMap<i64, String>,
    pub current_preset: String,
    pub device_name: String,

    /// Gain in dB for every channel.
    pub gains: BTreeMap<String, f64>,
    pub mutes: BTreeMap<String, bool>,
    pub phase: BTreeMap<String, bool>,

    /// Delay per channel, raw (ms · 96).
    pub delays: BTreeMap<String, i64>,

    /// Matrix routing: active input bitmask per output (cmd 0x3A).
    pub matrix_routing: BTreeMap<String, i64>,

    /// Matrix gain per crossing point in dB (−60 … 0), key "Out 1:In A"
    /// (cmd 0x41).
    pub matrix_gains: BTreeMap<String, f64>,

    /// 31-band GEQ per input channel (−12 … +12 dB).
    pub geq_bands: BTreeMap<String, Vec<f64>>,

    /// PEQ bands: 8 per input, 9 per output.
    pub peq_bands: BTreeMap<String, Vec<PeqBand>>,
    pub hi_pass: BTreeMap<String, FilterState>,
    pub lo_pass: BTreeMap<String, FilterState>,

    /// Inputs.
    pub gates: BTreeMap<String, GateState>,
    /// Outputs.
    pub compressors: BTreeMap<String, CompressorState>,
    /// Outputs.
    pub limiters: BTreeMap<String, LimiterState>,

    /// Channel names stored on the device ("InA", "Kick" …).
    pub channel_names: BTreeMap<String, String>,

    /// Channel link mask per channel as stored on the device: a master holds
    /// its own bit and the bits of its slaves, a slave only its own bit.
    pub links: BTreeMap<String, i64>,

    /// Whole PEQ of a channel bypassed (cmd 0x3C).
    pub eq_bypass: BTreeMap<String, bool>,

    /// GEQ of an input bypassed (cmd 0x49).
    pub geq_bypass: BTreeMap<String, bool>,

    /// Signal generator of the DSP (cmd 0x39, kept in the config dump): type
    /// (0 = analog input, i.e. off, 1 = pink, 2 = white, 3 = sine) and
    /// parameter (sine frequency index).
    pub test_tone_type: i64,
    pub test_tone_param: i64,

    /// Delay display unit stored on the device (cmd 0x15): 0 = ms, 1 = m,
    /// 2 = ft.
    pub delay_unit: i64,

    /// Used user preset slots by 0-based index (answer to 0x22); empty if the
    /// device did not answer.
    pub preset_slots: BTreeSet<i64>,

    /// Current preset number reported by the device (0x14; 0 = none/factory,
    /// n = Un), or −1 if unknown.
    pub current_preset_number: i64,
}

impl Default for DeviceState {
    fn default() -> Self {
        DeviceState::new(DeviceModel::DSP408)
    }
}

fn per<T>(channels: impl Iterator<Item = &'static str>, value: impl Fn(&'static str) -> T) -> BTreeMap<String, T> {
    channels.map(|c| (c.to_string(), value(c))).collect()
}

impl DeviceState {
    /// Default state of [model] before anything has been read from the device.
    pub fn new(model: DeviceModel) -> Self {
        let channels = model.channels();
        let inputs = || model.inputs.iter().copied();
        let outputs = || model.outputs.iter().copied();
        DeviceState {
            model,
            presets: BTreeMap::new(),
            current_preset: "Unknown".into(),
            device_name: String::new(),
            gains: per(channels.clone(), |_| 0.0),
            mutes: per(channels.clone(), |_| false),
            phase: per(channels.clone(), |_| false),
            delays: per(channels.clone(), |_| 0),
            matrix_routing: model
                .outputs
                .iter()
                .enumerate()
                .map(|(i, o)| (o.to_string(), 1i64 << (i % model.inputs.len())))
                .collect(),
            matrix_gains: outputs().flat_map(|o| inputs().map(move |i| (format!("{o}:{i}"), 0.0))).collect(),
            geq_bands: per(inputs(), |_| vec![0.0; GEQ_BAND_COUNT]),
            peq_bands: per(channels.clone(), |c| vec![PeqBand::default(); model.peq_band_count(c)]),
            hi_pass: per(channels.clone(), |_| FilterState::new(0, 0)),
            lo_pass: per(channels.clone(), |_| FilterState::new(300, 0)),
            gates: per(inputs(), |_| GateState::default()),
            compressors: per(outputs(), |_| CompressorState::default()),
            limiters: per(outputs(), |_| LimiterState::default()),
            channel_names: per(channels.clone(), |c| model.default_device_name(c)),
            links: per(channels.clone(), |c| model.link_bit(c) as i64),
            eq_bypass: per(channels, |_| false),
            geq_bypass: per(inputs(), |_| false),
            test_tone_type: 0,
            test_tone_param: 0,
            delay_unit: 0,
            preset_slots: BTreeSet::new(),
            current_preset_number: -1,
        }
    }

    /// Link mask of [ch], its own bit if none is stored.
    pub fn link_mask(&self, ch: &str) -> i64 {
        self.links.get(ch).copied().unwrap_or(self.model.link_bit(ch) as i64)
    }

    pub fn to_json(&self) -> Value {
        fn map<T: Serialize>(m: &BTreeMap<String, T>) -> Value {
            serde_json::to_value(m).unwrap_or(Value::Null)
        }
        json!({
            "model": self.model.id,
            "presets": self.presets.iter().map(|(k, v)| (k.to_string(), Value::from(v.clone()))).collect::<Map<_, _>>(),
            "currentPreset": self.current_preset,
            "deviceName": self.device_name,
            "gains": map(&self.gains),
            "mutes": map(&self.mutes),
            "phase": map(&self.phase),
            "delays": map(&self.delays),
            "matrixRouting": map(&self.matrix_routing),
            "matrixGains": map(&self.matrix_gains),
            "geqBands": map(&self.geq_bands),
            "peqBands": map(&self.peq_bands),
            "hiPass": map(&self.hi_pass),
            "loPass": map(&self.lo_pass),
            "gates": map(&self.gates),
            "compressors": map(&self.compressors),
            "limiters": map(&self.limiters),
            "channelNames": map(&self.channel_names),
            "links": map(&self.links),
            "eqBypass": map(&self.eq_bypass),
            "geqBypass": map(&self.geq_bypass),
            "testTone": [self.test_tone_type, self.test_tone_param],
            "delayUnit": self.delay_unit,
            "presetSlots": self.preset_slots.iter().collect::<Vec<_>>(),
            "currentPresetNumber": self.current_preset_number,
        })
    }

    pub fn from_json(json: &Value) -> JsonResult<Self> {
        let mut state = DeviceState::new(DeviceModel::by_id(json.get("model").and_then(Value::as_str)));

        // Missing maps are empty (keep the defaults).
        let empty = Map::new();
        let map = |key: &str| -> JsonResult<&Map<String, Value>> {
            match json.get(key) {
                None | Some(Value::Null) => Ok(&empty),
                Some(v) => obj(v, key),
            }
        };

        for (k, v) in map("presets")? {
            let index = k.parse::<i64>().map_err(|_| format!("invalid preset index {k}"))?;
            state.presets.insert(index, as_str(v, "preset name")?.to_string());
        }
        state.current_preset = as_str(field(json, "currentPreset"), "currentPreset")?.to_string();
        state.device_name = as_str(field(json, "deviceName"), "deviceName")?.to_string();

        for (k, v) in map("gains")? {
            state.gains.insert(k.clone(), as_num(v, "gain")?);
        }
        for (k, v) in map("mutes")? {
            state.mutes.insert(k.clone(), as_bool(v, "mute")?);
        }
        for (k, v) in map("phase")? {
            state.phase.insert(k.clone(), as_bool(v, "phase")?);
        }
        for (k, v) in map("delays")? {
            state.delays.insert(k.clone(), as_int(v, "delay")?);
        }
        for (k, v) in map("matrixRouting")? {
            state.matrix_routing.insert(k.clone(), as_int(v, "routing")?);
        }
        for (k, v) in map("matrixGains")? {
            state.matrix_gains.insert(k.clone(), as_num(v, "matrix gain")?);
        }
        for (k, v) in map("geqBands")? {
            let list = v.as_array().ok_or("geqBands must be lists")?;
            state.geq_bands.insert(k.clone(), list.iter().map(|x| as_num(x, "GEQ band")).collect::<JsonResult<_>>()?);
        }
        for (k, v) in map("peqBands")? {
            let list = v.as_array().ok_or("peqBands must be lists")?;
            state.peq_bands.insert(k.clone(), list.iter().map(PeqBand::from_json).collect::<JsonResult<_>>()?);
        }
        for (k, v) in map("hiPass")? {
            state.hi_pass.insert(k.clone(), FilterState::from_json(v)?);
        }
        for (k, v) in map("loPass")? {
            state.lo_pass.insert(k.clone(), FilterState::from_json(v)?);
        }
        for (k, v) in map("gates")? {
            state.gates.insert(k.clone(), GateState::from_json(v)?);
        }
        for (k, v) in map("compressors")? {
            state.compressors.insert(k.clone(), CompressorState::from_json(v)?);
        }
        for (k, v) in map("limiters")? {
            state.limiters.insert(k.clone(), LimiterState::from_json(v)?);
        }
        for (k, v) in map("channelNames")? {
            state.channel_names.insert(k.clone(), as_str(v, "channel name")?.to_string());
        }
        for (k, v) in map("links")? {
            state.links.insert(k.clone(), as_int(v, "link")?);
        }
        for (k, v) in map("eqBypass")? {
            state.eq_bypass.insert(k.clone(), as_bool(v, "eqBypass")?);
        }
        for (k, v) in map("geqBypass")? {
            state.geq_bypass.insert(k.clone(), as_bool(v, "geqBypass")?);
        }
        if let Some([t, p]) = json.get("testTone").and_then(Value::as_array).map(Vec::as_slice) {
            state.test_tone_type = as_int(t, "testTone")?;
            state.test_tone_param = as_int(p, "testTone")?;
        }
        state.delay_unit = match json.get("delayUnit") {
            None | Some(Value::Null) => 0,
            Some(v) => as_int(v, "delayUnit")?,
        };
        if let Some(slots) = json.get("presetSlots").and_then(Value::as_array) {
            for s in slots {
                state.preset_slots.insert(as_int(s, "presetSlots")?);
            }
        }
        state.current_preset_number = match json.get("currentPresetNumber") {
            None | Some(Value::Null) => -1,
            Some(v) => as_int(v, "currentPresetNumber")?,
        };
        Ok(state)
    }
}

impl Serialize for DeviceState {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_json().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for DeviceState {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        DeviceState::from_json(&value).map_err(serde::de::Error::custom)
    }
}
