//! Device operations.
//!
//! Every change to a [DeviceState] is an "op", a small JSON object such as
//! `{"op": "gain", "ch": "In A", "db": -3.5}`. The client applies an op
//! locally (optimistic UI) and sends it to the server; the server validates
//! it, applies it to its own state, turns it into device commands and
//! forwards it to the other clients.
//!
//! Ops carry absolute values (mute on/off, full routing mask, full PEQ band)
//! so that concurrent clients cannot toggle each other back and forth.
//! Dynamics, delay and filter values are raw protocol values (see `protocol`
//! for the conversions).
//!
//! Links work like in the original editor: a master is linked to one or more
//! slaves (In A → In B). The master's link mask holds its own bit and the bits
//! of its slaves; a slave cannot be linked to anything else. Changing a linked
//! channel sends the change once, for that channel; the DSP applies it to the
//! linked channels (assumed, like the original editor does). The state of the
//! other members follows the original editor's rule, see [edit_group_of].

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::model::{DeviceModel, GEQ_BAND_COUNT};
use crate::protocol::{self as p, Frame, Protocol, ProtocolError};
use crate::state::{
    CompressorState, DeviceState, FilterState, GateState, JsonResult, LimiterState, PeqBand, as_bool, as_int,
    as_num, as_str,
};

/// Highest sine frequency index of the generator (31 ISO frequencies).
pub const MAX_TEST_TONE_PARAM: i64 = GEQ_BAND_COUNT as i64 - 1;

/// One change of a device state.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Gain { ch: String, db: f64 },
    Mute { ch: String, on: bool },
    Phase { ch: String, on: bool },
    Delay { ch: String, raw: i64 },
    MatrixRoute { out: String, mask: i64 },
    MatrixGain { out: String, input: String, db: f64 },
    GeqBand { ch: String, band: usize, db: f64 },
    GeqReset { ch: String },
    PeqBand { ch: String, band: usize, value: PeqBand },
    HiPass { ch: String, value: FilterState },
    LoPass { ch: String, value: FilterState },
    Gate { ch: String, value: GateState },
    Compressor { ch: String, value: CompressorState },
    Limiter { ch: String, value: LimiterState },
    /// Bypass the whole PEQ of a channel (cmd 0x3C).
    EqBypass { ch: String, on: bool },
    /// Bypass the GEQ of an input (cmd 0x49).
    GeqBypass { ch: String, on: bool },
    /// Delay display unit stored on the device (0 = ms, 1 = m, 2 = ft).
    DelayUnit { unit: i64 },
    /// Device channel name; `None` or empty restores the default ("InA").
    ChannelName { ch: String, name: Option<String> },
    /// Link mask of a channel (bits of the channels of its group it is the
    /// master of, including itself; a slave or unlinked channel has only its
    /// own bit).
    Link { ch: String, mask: i64 },
    /// Signal generator of the DSP (0 = analog input, i.e. off, 1 = pink,
    /// 2 = white, 3 = sine with `param` = frequency index).
    TestTone { tone_type: i64, param: i64 },
}

// ─── Constructors (used by the client) ───

impl Op {
    pub fn gain(ch: &str, db: f64) -> Op {
        Op::Gain { ch: ch.into(), db }
    }
    pub fn mute(ch: &str, on: bool) -> Op {
        Op::Mute { ch: ch.into(), on }
    }
    pub fn phase(ch: &str, on: bool) -> Op {
        Op::Phase { ch: ch.into(), on }
    }
    pub fn delay(ch: &str, raw: i64) -> Op {
        Op::Delay { ch: ch.into(), raw }
    }
    pub fn matrix_route(out: &str, mask: i64) -> Op {
        Op::MatrixRoute { out: out.into(), mask }
    }
    pub fn matrix_gain(out: &str, input: &str, db: f64) -> Op {
        Op::MatrixGain { out: out.into(), input: input.into(), db }
    }
    pub fn geq_band(ch: &str, band: usize, db: f64) -> Op {
        Op::GeqBand { ch: ch.into(), band, db }
    }
    pub fn geq_reset(ch: &str) -> Op {
        Op::GeqReset { ch: ch.into() }
    }
    pub fn peq_band(ch: &str, band: usize, value: PeqBand) -> Op {
        Op::PeqBand { ch: ch.into(), band, value }
    }
    pub fn hi_pass(ch: &str, value: FilterState) -> Op {
        Op::HiPass { ch: ch.into(), value }
    }
    pub fn lo_pass(ch: &str, value: FilterState) -> Op {
        Op::LoPass { ch: ch.into(), value }
    }
    pub fn gate(ch: &str, value: GateState) -> Op {
        Op::Gate { ch: ch.into(), value }
    }
    pub fn compressor(ch: &str, value: CompressorState) -> Op {
        Op::Compressor { ch: ch.into(), value }
    }
    pub fn limiter(ch: &str, value: LimiterState) -> Op {
        Op::Limiter { ch: ch.into(), value }
    }
    pub fn eq_bypass(ch: &str, on: bool) -> Op {
        Op::EqBypass { ch: ch.into(), on }
    }
    pub fn geq_bypass(ch: &str, on: bool) -> Op {
        Op::GeqBypass { ch: ch.into(), on }
    }
    pub fn delay_unit(unit: i64) -> Op {
        Op::DelayUnit { unit }
    }
    pub fn channel_name(ch: &str, name: Option<&str>) -> Op {
        Op::ChannelName { ch: ch.into(), name: name.map(Into::into) }
    }
    pub fn link(ch: &str, mask: i64) -> Op {
        Op::Link { ch: ch.into(), mask }
    }
    pub fn test_tone(tone_type: i64, param: i64) -> Op {
        Op::TestTone { tone_type, param }
    }

    /// Name of the op in JSON ("gain", "peqBand", …).
    pub fn name(&self) -> &'static str {
        match self {
            Op::Gain { .. } => "gain",
            Op::Mute { .. } => "mute",
            Op::Phase { .. } => "phase",
            Op::Delay { .. } => "delay",
            Op::MatrixRoute { .. } => "matrixRoute",
            Op::MatrixGain { .. } => "matrixGain",
            Op::GeqBand { .. } => "geqBand",
            Op::GeqReset { .. } => "geqReset",
            Op::PeqBand { .. } => "peqBand",
            Op::HiPass { .. } => "hiPass",
            Op::LoPass { .. } => "loPass",
            Op::Gate { .. } => "gate",
            Op::Compressor { .. } => "compressor",
            Op::Limiter { .. } => "limiter",
            Op::EqBypass { .. } => "eqBypass",
            Op::GeqBypass { .. } => "geqBypass",
            Op::DelayUnit { .. } => "delayUnit",
            Op::ChannelName { .. } => "channelName",
            Op::Link { .. } => "link",
            Op::TestTone { .. } => "testTone",
        }
    }

    /// The channel the op changes (`ch`), if it has one. Matrix ops address
    /// an output with `out` and return `None`.
    pub fn channel(&self) -> Option<&str> {
        match self {
            Op::Gain { ch, .. }
            | Op::Mute { ch, .. }
            | Op::Phase { ch, .. }
            | Op::Delay { ch, .. }
            | Op::GeqBand { ch, .. }
            | Op::GeqReset { ch }
            | Op::PeqBand { ch, .. }
            | Op::HiPass { ch, .. }
            | Op::LoPass { ch, .. }
            | Op::Gate { ch, .. }
            | Op::Compressor { ch, .. }
            | Op::Limiter { ch, .. }
            | Op::EqBypass { ch, .. }
            | Op::GeqBypass { ch, .. }
            | Op::ChannelName { ch, .. }
            | Op::Link { ch, .. } => Some(ch),
            Op::MatrixRoute { .. } | Op::MatrixGain { .. } | Op::DelayUnit { .. } | Op::TestTone { .. } => None,
        }
    }

    /// Ops that change the other members of a link group in the state as
    /// well. Matrix routing and crossing gains are not linked: linked channels
    /// are usually a left/right pair fed from different inputs.
    pub fn is_linkable(&self) -> bool {
        matches!(
            self,
            Op::Gain { .. }
                | Op::Mute { .. }
                | Op::Phase { .. }
                | Op::Delay { .. }
                | Op::PeqBand { .. }
                | Op::HiPass { .. }
                | Op::LoPass { .. }
                | Op::Gate { .. }
                | Op::Compressor { .. }
                | Op::Limiter { .. }
                | Op::GeqBand { .. }
                | Op::GeqReset { .. }
                | Op::EqBypass { .. }
                | Op::GeqBypass { .. }
        )
    }

    /// A copy of the op for another channel.
    pub fn with_channel(&self, other: &str) -> Op {
        let mut op = self.clone();
        match &mut op {
            Op::Gain { ch, .. }
            | Op::Mute { ch, .. }
            | Op::Phase { ch, .. }
            | Op::Delay { ch, .. }
            | Op::GeqBand { ch, .. }
            | Op::GeqReset { ch }
            | Op::PeqBand { ch, .. }
            | Op::HiPass { ch, .. }
            | Op::LoPass { ch, .. }
            | Op::Gate { ch, .. }
            | Op::Compressor { ch, .. }
            | Op::Limiter { ch, .. }
            | Op::EqBypass { ch, .. }
            | Op::GeqBypass { ch, .. }
            | Op::ChannelName { ch, .. }
            | Op::Link { ch, .. } => *ch = other.to_string(),
            _ => {}
        }
        op
    }

    // ─── JSON ───

    pub fn to_json(&self) -> Value {
        fn with(name: &str, ch: &str, value: Value) -> Value {
            let mut m = Map::new();
            m.insert("op".into(), name.into());
            m.insert("ch".into(), ch.into());
            if let Value::Object(fields) = value {
                m.extend(fields);
            }
            Value::Object(m)
        }
        let name = self.name();
        match self {
            Op::Gain { ch, db } => json!({"op": name, "ch": ch, "db": db}),
            Op::Mute { ch, on } | Op::Phase { ch, on } | Op::EqBypass { ch, on } | Op::GeqBypass { ch, on } => {
                json!({"op": name, "ch": ch, "on": on})
            }
            Op::Delay { ch, raw } => json!({"op": name, "ch": ch, "raw": raw}),
            Op::MatrixRoute { out, mask } => json!({"op": name, "out": out, "mask": mask}),
            Op::MatrixGain { out, input, db } => json!({"op": name, "out": out, "in": input, "db": db}),
            Op::GeqBand { ch, band, db } => json!({"op": name, "ch": ch, "band": band, "db": db}),
            Op::GeqReset { ch } => json!({"op": name, "ch": ch}),
            Op::PeqBand { ch, band, value } => {
                let mut v = with(name, ch, json!({"band": band}));
                if let (Value::Object(m), Ok(Value::Object(fields))) = (&mut v, serde_json::to_value(value)) {
                    m.extend(fields);
                }
                v
            }
            Op::HiPass { ch, value } | Op::LoPass { ch, value } => {
                with(name, ch, serde_json::to_value(value).unwrap_or_default())
            }
            Op::Gate { ch, value } => with(name, ch, serde_json::to_value(value).unwrap_or_default()),
            Op::Compressor { ch, value } => with(name, ch, serde_json::to_value(value).unwrap_or_default()),
            Op::Limiter { ch, value } => with(name, ch, serde_json::to_value(value).unwrap_or_default()),
            Op::DelayUnit { unit } => json!({"op": name, "unit": unit}),
            Op::ChannelName { ch, name: n } => json!({"op": name, "ch": ch, "name": n}),
            Op::Link { ch, mask } => json!({"op": name, "ch": ch, "mask": mask}),
            Op::TestTone { tone_type, param } => json!({"op": name, "type": tone_type, "param": param}),
        }
    }

    /// Validate a JSON op for [model] (see [validate]) and convert it.
    pub fn from_json(json: &Value, model: &DeviceModel) -> JsonResult<Op> {
        if let Some(error) = validate(json, model) {
            return Err(error);
        }
        Self::parse(json)
    }

    /// Convert a JSON op without checking ranges or channels.
    pub fn parse(json: &Value) -> JsonResult<Op> {
        let get = |k: &str| json.get(k).unwrap_or(&Value::Null);
        let ch = || as_str(get("ch"), "ch").map(String::from);
        let int = |k: &str| as_int(get(k), k);
        let num = |k: &str| as_num(get(k), k);
        let flag = |k: &str| as_bool(get(k), k);
        let op_name = get("op").as_str().unwrap_or_default();
        Ok(match op_name {
            "gain" => Op::Gain { ch: ch()?, db: num("db")? },
            "mute" => Op::Mute { ch: ch()?, on: flag("on")? },
            "phase" => Op::Phase { ch: ch()?, on: flag("on")? },
            "eqBypass" => Op::EqBypass { ch: ch()?, on: flag("on")? },
            "geqBypass" => Op::GeqBypass { ch: ch()?, on: flag("on")? },
            "delay" => Op::Delay { ch: ch()?, raw: int("raw")? },
            "delayUnit" => Op::DelayUnit { unit: int("unit")? },
            "matrixRoute" => Op::MatrixRoute { out: as_str(get("out"), "out")?.into(), mask: int("mask")? },
            "matrixGain" => Op::MatrixGain {
                out: as_str(get("out"), "out")?.into(),
                input: as_str(get("in"), "in")?.into(),
                db: num("db")?,
            },
            "geqBand" => Op::GeqBand { ch: ch()?, band: int("band")?.max(0) as usize, db: num("db")? },
            "geqReset" => Op::GeqReset { ch: ch()? },
            "peqBand" => Op::PeqBand { ch: ch()?, band: int("band")?.max(0) as usize, value: PeqBand::from_json(json)? },
            "hiPass" => Op::HiPass { ch: ch()?, value: FilterState::from_json(json)? },
            "loPass" => Op::LoPass { ch: ch()?, value: FilterState::from_json(json)? },
            "gate" => Op::Gate { ch: ch()?, value: GateState::from_json(json)? },
            "compressor" => Op::Compressor { ch: ch()?, value: CompressorState::from_json(json)? },
            "limiter" => Op::Limiter { ch: ch()?, value: LimiterState::from_json(json)? },
            "channelName" => Op::ChannelName { ch: ch()?, name: get("name").as_str().map(String::from) },
            "link" => Op::Link { ch: ch()?, mask: int("mask")? },
            "testTone" => Op::TestTone { tone_type: int("type")?, param: int("param")? },
            other => return Err(format!("unknown op {other}")),
        })
    }
}

impl serde::Serialize for Op {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_json().serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for Op {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Op::parse(&value).map_err(serde::de::Error::custom)
    }
}

// ─── Validation ───

/// Returns an error message, or `None` if [op] is well-formed for [model].
/// Value ranges are those of the original editor (see `protocol`).
pub fn validate(op: &Value, model: &DeviceModel) -> Option<String> {
    let Some(op) = op.as_object() else {
        return Some("op must be an object".into());
    };
    let str_of = |k: &str| op.get(k).and_then(Value::as_str);
    let is_num = |k: &str| op.get(k).and_then(Value::as_f64).is_some_and(f64::is_finite);
    let is_bool = |k: &str| op.get(k).is_some_and(Value::is_boolean);
    // Like Dart's `is int` on the server: JSON integers only.
    let int_in = |k: &str, min: i64, max: i64| op.get(k).and_then(Value::as_i64).is_some_and(|v| v >= min && v <= max);
    let times = |hold: bool| {
        int_in("attack", 0, p::ATTACK_RAW_MAX)
            && int_in("release", p::RELEASE_RAW_MIN, p::RELEASE_RAW_MAX)
            && (!hold || int_in("hold", p::HOLD_RAW_MIN, p::HOLD_RAW_MAX))
    };

    let ch = str_of("ch");
    let has_channel = ch.is_some_and(|c| model.has_channel(c));
    let is_input = ch.is_some_and(|c| model.is_input(c));
    let is_output = ch.is_some_and(|c| model.is_output(c));
    let fail = |m: &str| Some(m.to_string());

    match op.get("op").and_then(Value::as_str) {
        Some("gain") => {
            if !has_channel {
                return fail("invalid channel");
            }
            if !is_num("db") {
                return fail("db must be a number");
            }
        }
        Some("mute" | "phase" | "eqBypass") => {
            if !has_channel {
                return fail("invalid channel");
            }
            if !is_bool("on") {
                return fail("on must be a bool");
            }
        }
        Some("geqBypass") => {
            if !is_input {
                return fail("invalid GEQ channel");
            }
            if !is_bool("on") {
                return fail("on must be a bool");
            }
        }
        Some("delay") => {
            if !has_channel {
                return fail("invalid channel");
            }
            if !int_in("raw", 0, p::DELAY_RAW_MAX) {
                return fail("delay out of range");
            }
        }
        Some("delayUnit") => {
            if !int_in("unit", 0, p::DELAY_UNIT_NAMES.len() as i64 - 1) {
                return fail("invalid delay unit");
            }
        }
        Some("matrixRoute") => {
            if !str_of("out").is_some_and(|o| model.is_output(o)) {
                return fail("invalid output");
            }
            if !int_in("mask", 0, model.matrix_mask() as i64) {
                return fail("invalid input mask");
            }
        }
        Some("matrixGain") => {
            if !str_of("out").is_some_and(|o| model.is_output(o)) {
                return fail("invalid output");
            }
            if !str_of("in").is_some_and(|i| model.is_input(i)) {
                return fail("invalid input");
            }
            if !is_num("db") {
                return fail("db must be a number");
            }
        }
        Some("geqBand") => {
            if !is_input {
                return fail("invalid GEQ channel");
            }
            if !int_in("band", 0, GEQ_BAND_COUNT as i64 - 1) {
                return fail("invalid band");
            }
            if !is_num("db") {
                return fail("db must be a number");
            }
        }
        Some("geqReset") => {
            if !is_input {
                return fail("invalid GEQ channel");
            }
        }
        Some("peqBand") => {
            if !has_channel {
                return fail("invalid channel");
            }
            if !int_in("band", 0, model.peq_band_count(ch.unwrap_or_default()) as i64 - 1) {
                return fail("invalid band");
            }
            // The per-type Q limit is applied when a band is edited (the device
            // may hold higher values set by other software).
            if !int_in("freqRaw", 0, p::FREQ_RAW_MAX)
                || !int_in("qRaw", 0, p::Q_RAW_MAX)
                || !int_in("type", 0, p::PEQ_TYPE_NAMES.len() as i64 - 1)
                || !is_num("gainDb")
                || !is_bool("bypass")
            {
                return fail("invalid PEQ band");
            }
        }
        Some("hiPass" | "loPass") => {
            if !has_channel {
                return fail("invalid channel");
            }
            if !int_in("freqRaw", 0, p::FREQ_RAW_MAX) || !int_in("slope", 0, p::CROSSOVER_SLOPE_NAMES.len() as i64 - 1) {
                return fail("invalid filter");
            }
        }
        Some("gate") => {
            if !is_input {
                return fail("gate is only available on inputs");
            }
            if !times(true) || !int_in("threshold", 0, p::GATE_THRESHOLD_RAW_MAX) {
                return fail("invalid gate");
            }
        }
        Some("compressor") => {
            if !is_output {
                return fail("compressor is only available on outputs");
            }
            if !int_in("ratio", 0, p::COMPRESSOR_RATIO_NAMES.len() as i64 - 1)
                || !times(false)
                || !int_in("knee", 0, p::KNEE_MAX)
                || !int_in("threshold", 0, p::THRESHOLD_RAW_MAX)
            {
                return fail("invalid compressor");
            }
        }
        Some("limiter") => {
            if !is_output {
                return fail("limiter is only available on outputs");
            }
            if !times(false) || !int_in("knee", 0, p::KNEE_MAX) || !int_in("threshold", 0, p::THRESHOLD_RAW_MAX) {
                return fail("invalid limiter");
            }
        }
        Some("channelName") => {
            if !has_channel {
                return fail("invalid channel");
            }
            match op.get("name") {
                None | Some(Value::Null) => {}
                Some(Value::String(name))
                    if name.encode_utf16().count() <= 8 && name.chars().all(|c| (' '..='~').contains(&c)) => {}
                Some(_) => return fail("name must be at most 8 ASCII characters"),
            }
        }
        Some("link") => {
            if !has_channel {
                return fail("invalid channel");
            }
            let ch = ch.unwrap_or_default();
            let mask = op.get("mask").and_then(Value::as_i64);
            // A slave may also be stored with an empty mask.
            if !int_in("mask", 0, model.group_mask(ch) as i64)
                || mask.is_some_and(|m| m != 0 && m & model.link_bit(ch) as i64 == 0)
            {
                return fail("invalid link mask");
            }
        }
        Some("testTone") => {
            if !int_in("type", 0, p::TEST_TONE_TYPE_NAMES.len() as i64 - 1) || !int_in("param", 0, MAX_TEST_TONE_PARAM) {
                return fail("invalid test tone");
            }
        }
        other => {
            let shown = match other {
                Some(name) => name.to_string(),
                None => op.get("op").map_or("null".to_string(), Value::to_string),
            };
            return Some(format!("unknown op {shown}"));
        }
    }
    None
}

/// [g] limited to the original editor's ranges (older versions allowed longer
/// times).
pub fn clamp_gate(g: &GateState) -> GateState {
    GateState {
        attack: g.attack.clamp(0, p::ATTACK_RAW_MAX),
        release: g.release.clamp(p::RELEASE_RAW_MIN, p::RELEASE_RAW_MAX),
        hold: g.hold.clamp(p::HOLD_RAW_MIN, p::HOLD_RAW_MAX),
        threshold: g.threshold.clamp(0, p::GATE_THRESHOLD_RAW_MAX),
    }
}

pub fn clamp_compressor(c: &CompressorState) -> CompressorState {
    CompressorState {
        ratio: c.ratio.clamp(0, p::COMPRESSOR_RATIO_NAMES.len() as i64 - 1),
        attack: c.attack.clamp(0, p::ATTACK_RAW_MAX),
        release: c.release.clamp(p::RELEASE_RAW_MIN, p::RELEASE_RAW_MAX),
        knee: c.knee.clamp(0, p::KNEE_MAX),
        threshold: c.threshold.clamp(0, p::THRESHOLD_RAW_MAX),
    }
}

pub fn clamp_limiter(l: &LimiterState) -> LimiterState {
    LimiterState {
        attack: l.attack.clamp(0, p::ATTACK_RAW_MAX),
        release: l.release.clamp(p::RELEASE_RAW_MIN, p::RELEASE_RAW_MAX),
        knee: l.knee.clamp(0, p::KNEE_MAX),
        threshold: l.threshold.clamp(0, p::THRESHOLD_RAW_MAX),
    }
}

// ─── Apply ───

/// Apply an op that passed [validate] to [state], clamping and quantizing
/// values the same way on client and server.
pub fn apply(state: &mut DeviceState, op: &Op) {
    match op {
        Op::Gain { ch, db } => {
            state.gains.insert(ch.clone(), p::quantize_gain(db.clamp(-60.0, 12.0)));
        }
        Op::Mute { ch, on } => {
            state.mutes.insert(ch.clone(), *on);
        }
        Op::Phase { ch, on } => {
            state.phase.insert(ch.clone(), *on);
        }
        Op::EqBypass { ch, on } => {
            state.eq_bypass.insert(ch.clone(), *on);
        }
        Op::GeqBypass { ch, on } => {
            state.geq_bypass.insert(ch.clone(), *on);
        }
        Op::Delay { ch, raw } => {
            state.delays.insert(ch.clone(), *raw);
        }
        Op::DelayUnit { unit } => state.delay_unit = *unit,
        Op::MatrixRoute { out, mask } => {
            state.matrix_routing.insert(out.clone(), *mask);
        }
        Op::MatrixGain { out, input, db } => {
            state.matrix_gains.insert(format!("{out}:{input}"), p::quantize_gain(db.clamp(-60.0, p::MATRIX_GAIN_MAX_DB)));
        }
        Op::GeqBand { ch, band, db } => {
            let bands = state.geq_bands.entry(ch.clone()).or_insert_with(|| vec![0.0; GEQ_BAND_COUNT]);
            if let Some(b) = bands.get_mut(*band) {
                *b = p::quantize_geq_db(db.clamp(-12.0, 12.0));
            }
        }
        Op::GeqReset { ch } => {
            state.geq_bands.insert(ch.clone(), vec![0.0; GEQ_BAND_COUNT]);
        }
        Op::PeqBand { ch, band, value } => {
            let count = state.model.peq_band_count(ch);
            let bands = state.peq_bands.entry(ch.clone()).or_insert_with(|| vec![PeqBand::default(); count]);
            if let Some(b) = bands.get_mut(*band) {
                *b = PeqBand { gain_db: p::quantize_geq_db(value.gain_db.clamp(-12.0, 12.0)), ..value.clone() };
            }
        }
        Op::HiPass { ch, value } => {
            state.hi_pass.insert(ch.clone(), value.clone());
        }
        Op::LoPass { ch, value } => {
            state.lo_pass.insert(ch.clone(), value.clone());
        }
        Op::Gate { ch, value } => {
            state.gates.insert(ch.clone(), value.clone());
        }
        Op::Compressor { ch, value } => {
            state.compressors.insert(ch.clone(), value.clone());
        }
        Op::Limiter { ch, value } => {
            state.limiters.insert(ch.clone(), value.clone());
        }
        Op::ChannelName { ch, name } => {
            let name = name.as_deref().map(str::trim).filter(|n| !n.is_empty());
            let value = name.map_or_else(|| state.model.default_device_name(ch), String::from);
            state.channel_names.insert(ch.clone(), value);
        }
        Op::Link { ch, mask } => {
            state.links.insert(ch.clone(), *mask);
        }
        Op::TestTone { tone_type, param } => {
            state.test_tone_type = *tone_type;
            state.test_tone_param = *param;
        }
    }
}

// ─── Device commands (server) ───

/// Device frames for an op that has already been applied to [state], keyed by
/// a coalescing key: a newer value for the same key replaces an older one that
/// has not been sent yet.
pub fn commands(state: &DeviceState, op: &Op) -> Result<Vec<(String, Vec<Frame>)>, ProtocolError> {
    let proto = Protocol::new(state.model);
    let one = |key: String, frame: Frame| Ok(vec![(key, vec![frame])]);
    match op {
        Op::Gain { ch, .. } => one(format!("gain:{ch}"), proto.build_gain_command(ch, state.gains[ch])?),
        Op::Mute { ch, .. } => one(format!("mute:{ch}"), proto.build_mute_command(ch, state.mutes[ch])?),
        Op::Phase { ch, .. } => one(format!("phase:{ch}"), proto.build_phase_command(ch, state.phase[ch])?),
        Op::EqBypass { ch, .. } => {
            one(format!("eqBypass:{ch}"), proto.build_eq_bypass_command(ch, state.eq_bypass[ch])?)
        }
        Op::GeqBypass { ch, .. } => {
            one(format!("geqBypass:{ch}"), proto.build_geq_bypass_command(ch, state.geq_bypass[ch])?)
        }
        Op::Delay { ch, .. } => one(format!("delay:{ch}"), proto.build_delay_command(ch, state.delays[ch])?),
        Op::DelayUnit { .. } => one("delayUnit".into(), p::build_delay_unit_command(state.delay_unit)),
        Op::MatrixRoute { out, .. } => {
            one(format!("matrix:{out}"), proto.build_matrix_routing_command(out, state.matrix_routing[out])?)
        }
        Op::MatrixGain { out, input, .. } => {
            let db = state.matrix_gains.get(&format!("{out}:{input}")).copied().unwrap_or(0.0);
            one(format!("mixgain:{out}:{input}"), proto.build_matrix_gain_command(out, input, db)?)
        }
        Op::GeqBand { ch, band, .. } => {
            let db = state.geq_bands[ch][*band];
            one(format!("geq:{ch}:{band}"), proto.build_geq_band_command(ch, *band, db)?)
        }
        Op::GeqReset { ch } => (0..GEQ_BAND_COUNT)
            .map(|band| Ok((format!("geq:{ch}:{band}"), vec![proto.build_geq_band_command(ch, band, 0.0)?])))
            .collect(),
        Op::PeqBand { ch, band, .. } => {
            let b = &state.peq_bands[ch][*band];
            let frame =
                proto.build_peq_band_command(ch, *band, b.gain_db, b.freq_raw, b.q_raw, b.peq_type, b.bypass)?;
            one(format!("peq:{ch}:{band}"), frame)
        }
        Op::HiPass { ch, .. } => {
            let f = &state.hi_pass[ch];
            one(format!("hpf:{ch}"), proto.build_hi_pass_command(ch, f.freq_raw, f.slope)?)
        }
        Op::LoPass { ch, .. } => {
            let f = &state.lo_pass[ch];
            one(format!("lpf:{ch}"), proto.build_lo_pass_command(ch, f.freq_raw, f.slope)?)
        }
        Op::Gate { ch, .. } => {
            let g = &state.gates[ch];
            one(format!("gate:{ch}"), proto.build_gate_command(ch, g.attack, g.release, g.hold, g.threshold)?)
        }
        Op::Compressor { ch, .. } => {
            let c = &state.compressors[ch];
            let frame = proto.build_compressor_command(ch, c.ratio, c.attack, c.release, c.knee, c.threshold)?;
            one(format!("comp:{ch}"), frame)
        }
        Op::Limiter { ch, .. } => {
            let l = &state.limiters[ch];
            one(format!("limit:{ch}"), proto.build_limiter_command(ch, l.attack, l.release, l.knee, l.threshold)?)
        }
        Op::ChannelName { ch, .. } => {
            one(format!("name:{ch}"), proto.build_channel_name_command(ch, &state.channel_names[ch])?)
        }
        Op::Link { ch, .. } => one(format!("link:{ch}"), proto.build_link_command(ch, state.links[ch])?),
        Op::TestTone { .. } => {
            one("testTone".into(), p::build_test_tone_command(state.test_tone_type, state.test_tone_param))
        }
    }
}

// ─── Linking ───

/// Channels whose state changes together with [ch], by the original editor's
/// rule: the channel's own link mask if it contains its own bit, otherwise the
/// mask of the first lower channel of the group that contains it; just [ch] if
/// there is none.
pub fn edit_group_of(state: &DeviceState, ch: &str) -> Vec<String> {
    let model = state.model;
    let group = model.group_of(ch);
    let bit = model.link_bit(ch) as i64;
    let mut mask = state.link_mask(ch);
    if mask & bit == 0 {
        mask = 0;
        let index = group.iter().position(|c| *c == ch).unwrap_or(0);
        for lower in &group[..index] {
            if state.link_mask(lower) & bit != 0 {
                mask = state.link_mask(lower);
                break;
            }
        }
    }
    let members: Vec<String> =
        group.iter().filter(|c| mask & model.link_bit(c) as i64 != 0).map(|c| c.to_string()).collect();
    if members.iter().any(|m| m == ch) { members } else { vec![ch.to_string()] }
}

/// Master whose link mask contains [ch] (not [ch] itself), or `None`.
///
/// Symmetric masks (every member lists every other, as stored by older panel
/// versions) are read as a group whose first channel is the master.
pub fn master_of(state: &DeviceState, ch: &str) -> Option<String> {
    let model = state.model;
    let group = model.group_of(ch);
    let bit = model.link_bit(ch) as i64;
    let own = state.link_mask(ch);
    let index = |c: &str| group.iter().position(|g| *g == c);
    for other in group {
        if *other == ch || state.link_mask(other) & bit == 0 {
            continue;
        }
        let mutual = own & model.link_bit(other) as i64 != 0;
        if mutual && index(ch) < index(other) {
            continue;
        }
        return Some(other.to_string());
    }
    None
}

/// Slaves of [ch] (empty unless [ch] is a master).
pub fn slaves_of(state: &DeviceState, ch: &str) -> Vec<String> {
    let model = state.model;
    if master_of(state, ch).is_some() {
        return Vec::new();
    }
    let mask = state.link_mask(ch);
    model
        .group_of(ch)
        .iter()
        .filter(|other| {
            **other != ch
                && mask & model.link_bit(other) as i64 != 0
                && master_of(state, other).as_deref() == Some(ch)
        })
        .map(|c| c.to_string())
        .collect()
}

/// All members of [ch]'s link group, master first; just [ch] if unlinked.
pub fn link_group_of(state: &DeviceState, ch: &str) -> Vec<String> {
    let master = master_of(state, ch).unwrap_or_else(|| ch.to_string());
    let mut group = vec![master.clone()];
    group.extend(slaves_of(state, &master));
    group
}

/// Channels [ch] is linked with (not including itself).
pub fn linked_with(state: &DeviceState, ch: &str) -> Vec<String> {
    link_group_of(state, ch).into_iter().filter(|c| c != ch).collect()
}

/// [op] plus a copy for every other channel whose state follows the channel it
/// changes (see [edit_group_of]). Only [op] itself goes to the device.
pub fn expand_linked(state: &DeviceState, op: &Op) -> Vec<Op> {
    let Some(ch) = op.channel().filter(|_| op.is_linkable()) else {
        return vec![op.clone()];
    };
    let mut ops = vec![op.clone()];
    ops.extend(edit_group_of(state, ch).iter().filter(|o| *o != ch).map(|o| op.with_channel(o)));
    ops
}

/// Channels that can become slaves of [master]: same group, not [master], not
/// a slave of another master and not a master themselves.
pub fn link_candidates(state: &DeviceState, master: &str) -> Vec<String> {
    if master_of(state, master).is_some() {
        return Vec::new();
    }
    state
        .model
        .group_of(master)
        .iter()
        .filter(|other| {
            **other != master
                && master_of(state, other).is_none_or(|m| m == master)
                && slaves_of(state, other).is_empty()
        })
        .map(|c| c.to_string())
        .collect()
}

/// A link change checked like the original link dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkChange {
    /// New mask of the master.
    pub mask: i64,
    /// Newly added slaves (they get the master's settings first).
    pub added: Vec<String>,
    /// Released channels whose mask is not just their own bit (one listing
    /// other channels, written by older panel versions, or 0 as other control
    /// software leaves on slaves, where the DSP then ignores changes); reset
    /// to their own bit.
    pub reset: Vec<String>,
}

pub fn link_change(state: &DeviceState, master: &str, slaves: &[String]) -> JsonResult<LinkChange> {
    let model = state.model;
    if !model.has_channel(master) {
        return Err(format!("Unknown channel {master}"));
    }
    let group = model.group_of(master);
    if slaves.iter().any(|c| !group.contains(&c.as_str()) || c == master) {
        return Err("Only inputs or only outputs can be linked".into());
    }
    if let Some(m) = master_of(state, master) {
        return Err(format!("{master} is linked to {m} and cannot be a master"));
    }
    let candidates = link_candidates(state, master);
    let invalid: Vec<&str> = slaves.iter().filter(|s| !candidates.contains(s)).map(String::as_str).collect();
    if !invalid.is_empty() {
        return Err(format!("{} cannot be linked to {master}", invalid.join(", ")));
    }
    let previous = slaves_of(state, master);
    Ok(LinkChange {
        mask: slaves.iter().fold(model.link_bit(master) as i64, |m, c| m | model.link_bit(c) as i64),
        added: slaves.iter().filter(|s| !previous.contains(s)).cloned().collect(),
        reset: previous
            .iter()
            .filter(|s| !slaves.contains(s) && state.link_mask(s) != model.link_bit(s) as i64)
            .cloned()
            .collect(),
    })
}

/// Ops that make [slaves] the slaves of [master] in a state without a device
/// (offline preset editor), like the original editor: newly linked slaves get
/// the master's settings, then the master's mask is set. An empty list unlinks
/// [master].
pub fn link_slaves(state: &DeviceState, master: &str, slaves: &[String]) -> JsonResult<Vec<Op>> {
    let change = link_change(state, master, slaves)?;
    let mut ops = Vec::new();
    for s in &change.added {
        ops.extend(copy_channel(state, master, s)?);
    }
    ops.push(Op::link(master, change.mask));
    for s in &change.reset {
        ops.push(Op::link(s, state.model.link_bit(s) as i64));
    }
    Ok(ops)
}

/// Master and remaining slaves after [ch] leaves its link group (a slave
/// leaves its master, a master releases all its slaves).
pub fn unlink_target(state: &DeviceState, ch: &str) -> (String, Vec<String>) {
    match master_of(state, ch) {
        None => (ch.to_string(), Vec::new()),
        Some(master) => {
            let rest = slaves_of(state, &master).into_iter().filter(|s| s != ch).collect();
            (master, rest)
        }
    }
}

/// Ops that remove [ch] from its link group (offline preset editor).
pub fn unlink(state: &DeviceState, ch: &str) -> JsonResult<Vec<Op>> {
    let (master, slaves) = unlink_target(state, ch);
    link_slaves(state, &master, &slaves)
}

/// Ops that copy the settings of [from] to [to] (same group only) in a state
/// without a device, like the original editor's offline copy: gain, phase,
/// delay, mute, PEQ (with bypass), EQ bypass, filters, and GEQ/gate (inputs)
/// or dynamics (outputs). Name, link and matrix are not copied. On a DSP the
/// device copies itself (cmd 0x2A).
pub fn copy_channel(state: &DeviceState, from: &str, to: &str) -> JsonResult<Vec<Op>> {
    let model = state.model;
    if from == to || !model.has_channel(from) || !model.has_channel(to) || model.is_input(from) != model.is_input(to) {
        return Err("Settings can only be copied between two inputs or two outputs".into());
    }
    let mut ops = vec![
        Op::gain(to, state.gains.get(from).copied().unwrap_or(0.0)),
        Op::phase(to, state.phase.get(from).copied().unwrap_or(false)),
        Op::delay(to, state.delays.get(from).copied().unwrap_or(0)),
        Op::mute(to, state.mutes.get(from).copied().unwrap_or(false)),
    ];
    for (b, band) in state.peq_bands.get(from).into_iter().flatten().enumerate() {
        ops.push(Op::peq_band(to, b, band.clone()));
    }
    ops.push(Op::eq_bypass(to, state.eq_bypass.get(from).copied().unwrap_or(false)));
    ops.push(Op::hi_pass(to, state.hi_pass.get(from).cloned().unwrap_or_default()));
    ops.push(Op::lo_pass(to, state.lo_pass.get(from).cloned().unwrap_or(FilterState::new(300, 0))));
    if model.is_input(from) {
        let geq = state.geq_bands.get(from).cloned().unwrap_or_else(|| vec![0.0; GEQ_BAND_COUNT]);
        for (b, db) in geq.iter().enumerate().take(GEQ_BAND_COUNT) {
            ops.push(Op::geq_band(to, b, *db));
        }
        ops.push(Op::gate(to, clamp_gate(&state.gates.get(from).cloned().unwrap_or_default())));
    } else {
        ops.push(Op::compressor(to, clamp_compressor(&state.compressors.get(from).cloned().unwrap_or_default())));
        ops.push(Op::limiter(to, clamp_limiter(&state.limiters.get(from).cloned().unwrap_or_default())));
    }
    Ok(ops)
}

/// Ops that bring a device to [state] completely (e.g. when a preset stored
/// on the server is loaded onto a DSP), in the order the original editor
/// loads a preset file: per input name, gate, GEQ, filters, gain, phase,
/// delay, link; per output name, routing, crossing gains, filters, dynamics,
/// gain, phase, delay, link; then all mutes and all PEQ bands. The values a
/// preset file does not hold (EQ/GEQ bypass, delay unit) follow at the end.
pub fn full_state(state: &DeviceState) -> Vec<Op> {
    let model = state.model;
    let mut ops = Vec::new();
    let name = |ch: &str| state.channel_names.get(ch).map(String::as_str);
    for ch in model.inputs {
        ops.push(Op::channel_name(ch, name(ch)));
        ops.push(Op::gate(ch, state.gates.get(*ch).cloned().unwrap_or_default()));
        let geq = state.geq_bands.get(*ch).cloned().unwrap_or_else(|| vec![0.0; GEQ_BAND_COUNT]);
        for (b, db) in geq.iter().enumerate() {
            ops.push(Op::geq_band(ch, b, *db));
        }
        ops.push(Op::hi_pass(ch, state.hi_pass.get(*ch).cloned().unwrap_or_default()));
        ops.push(Op::lo_pass(ch, state.lo_pass.get(*ch).cloned().unwrap_or(FilterState::new(300, 0))));
        ops.push(Op::gain(ch, state.gains.get(*ch).copied().unwrap_or(0.0)));
        ops.push(Op::phase(ch, state.phase.get(*ch).copied().unwrap_or(false)));
        ops.push(Op::delay(ch, state.delays.get(*ch).copied().unwrap_or(0)));
        ops.push(Op::link(ch, state.link_mask(ch)));
    }
    for ch in model.outputs {
        ops.push(Op::channel_name(ch, name(ch)));
        ops.push(Op::matrix_route(ch, state.matrix_routing.get(*ch).copied().unwrap_or(0)));
        for input in model.inputs {
            ops.push(Op::matrix_gain(ch, input, state.matrix_gains.get(&format!("{ch}:{input}")).copied().unwrap_or(0.0)));
        }
        ops.push(Op::hi_pass(ch, state.hi_pass.get(*ch).cloned().unwrap_or_default()));
        ops.push(Op::lo_pass(ch, state.lo_pass.get(*ch).cloned().unwrap_or(FilterState::new(300, 0))));
        ops.push(Op::compressor(ch, state.compressors.get(*ch).cloned().unwrap_or_default()));
        ops.push(Op::limiter(ch, state.limiters.get(*ch).cloned().unwrap_or_default()));
        ops.push(Op::gain(ch, state.gains.get(*ch).copied().unwrap_or(0.0)));
        ops.push(Op::phase(ch, state.phase.get(*ch).copied().unwrap_or(false)));
        ops.push(Op::delay(ch, state.delays.get(*ch).copied().unwrap_or(0)));
        ops.push(Op::link(ch, state.link_mask(ch)));
    }
    for ch in model.channels() {
        ops.push(Op::mute(ch, state.mutes.get(ch).copied().unwrap_or(false)));
    }
    for ch in model.channels() {
        for (b, band) in state.peq_bands.get(ch).into_iter().flatten().enumerate() {
            ops.push(Op::peq_band(ch, b, band.clone()));
        }
    }
    for ch in model.channels() {
        ops.push(Op::eq_bypass(ch, state.eq_bypass.get(ch).copied().unwrap_or(false)));
    }
    for ch in model.inputs {
        ops.push(Op::geq_bypass(ch, state.geq_bypass.get(*ch).copied().unwrap_or(false)));
    }
    ops.push(Op::delay_unit(state.delay_unit));
    ops
}

// ─── Live safety ───

/// Ops that stay allowed while the DSP test generator runs: mutes, the
/// generator itself and raw link masks (Auto-EQ lifts a link for a
/// measurement). Nothing that changes a level.
pub fn allowed_during_test(op: &Op) -> bool {
    matches!(op, Op::Mute { .. } | Op::TestTone { .. } | Op::Link { .. })
}

/// Max level of [ch] from the per-channel [caps] (kept by the server, unknown
/// to the DSP): the lowest cap of the channels that change together with it,
/// so linked channels share the cap of their group.
pub fn group_cap(state: &DeviceState, caps: &BTreeMap<String, f64>, ch: &str) -> Option<f64> {
    edit_group_of(state, ch).iter().filter_map(|c| caps.get(c).copied()).reduce(f64::min)
}

/// [db] limited by [cap] to max(cap, current): a channel that is already above
/// its cap can only be turned down.
pub fn capped_gain(db: f64, current: f64, cap: Option<f64>) -> f64 {
    cap.map_or(db, |cap| db.min(cap.max(current)))
}

// ─── Copy between DSPs ───

/// Ops that copy [parts] (api::COPY_PARTS) of channel [from] of [src] to
/// channel [to] of a device in state [dst] – the same kind of channel, maybe
/// on another DSP or model:
/// - level: gain, phase, delay
/// - filters: HPF and LPF
/// - eq: the PEQ bands both channels have and the EQ bypass; inputs also the
///   GEQ and its bypass
/// - dynamics: gate (inputs), compressor and limiter (outputs)
/// - routing (outputs): matrix routing and crossing gains of the inputs both
///   models have
///
/// Values the target already has are left out. Unlike the device copy (0x2A)
/// nothing else is touched (no mute, name or link).
pub fn copy_parts(src: &DeviceState, from: &str, dst: &DeviceState, to: &str, parts: &[String]) -> JsonResult<Vec<Op>> {
    let (sm, dm) = (src.model, dst.model);
    if !sm.has_channel(from) || !dm.has_channel(to) {
        return Err("Unknown channel".into());
    }
    if sm.is_input(from) != dm.is_input(to) {
        return Err("Settings can only be copied between two inputs or two outputs".into());
    }
    if let Some(p) = parts.iter().find(|p| !crate::api::COPY_PARTS.contains(&p.as_str())) {
        return Err(format!("Unknown part {p}"));
    }
    let has = |p: &str| parts.iter().any(|x| x == p);
    let input = sm.is_input(from);
    let mut list = Vec::new();
    if has("level") {
        list.push(Op::gain(to, src.gains.get(from).copied().unwrap_or(0.0)));
        list.push(Op::phase(to, src.phase.get(from).copied().unwrap_or(false)));
        list.push(Op::delay(to, src.delays.get(from).copied().unwrap_or(0)));
    }
    if has("filters") {
        list.push(Op::hi_pass(to, src.hi_pass.get(from).cloned().unwrap_or_default()));
        list.push(Op::lo_pass(to, src.lo_pass.get(from).cloned().unwrap_or(FilterState::new(300, 0))));
    }
    if has("eq") {
        let count = sm.peq_band_count(from).min(dm.peq_band_count(to));
        for (b, band) in src.peq_bands.get(from).into_iter().flatten().take(count).enumerate() {
            list.push(Op::peq_band(to, b, band.clone()));
        }
        list.push(Op::eq_bypass(to, src.eq_bypass.get(from).copied().unwrap_or(false)));
        if input {
            for (b, db) in src.geq_bands.get(from).into_iter().flatten().enumerate().take(GEQ_BAND_COUNT) {
                list.push(Op::geq_band(to, b, *db));
            }
            list.push(Op::geq_bypass(to, src.geq_bypass.get(from).copied().unwrap_or(false)));
        }
    }
    if has("dynamics") {
        if input {
            list.push(Op::gate(to, clamp_gate(&src.gates.get(from).cloned().unwrap_or_default())));
        } else {
            list.push(Op::compressor(to, clamp_compressor(&src.compressors.get(from).cloned().unwrap_or_default())));
            list.push(Op::limiter(to, clamp_limiter(&src.limiters.get(from).cloned().unwrap_or_default())));
        }
    }
    if has("routing") && !input {
        let shared: Vec<&str> = sm.inputs.iter().copied().filter(|i| dm.inputs.contains(i)).collect();
        let mask = src.matrix_routing.get(from).copied().unwrap_or(0);
        let routed = shared.iter().filter(|i| mask & sm.matrix_bit(i) as i64 != 0).fold(0i64, |m, i| m | dm.matrix_bit(i) as i64);
        list.push(Op::matrix_route(to, routed));
        for i in shared {
            let db = src.matrix_gains.get(&format!("{from}:{i}")).copied().unwrap_or(0.0);
            list.push(Op::matrix_gain(to, i, db));
        }
    }
    let mut state = dst.clone();
    Ok(list
        .into_iter()
        .filter(|op| {
            let mut next = state.clone();
            apply(&mut next, op);
            let changes = next != state;
            state = next;
            changes
        })
        .collect())
}
