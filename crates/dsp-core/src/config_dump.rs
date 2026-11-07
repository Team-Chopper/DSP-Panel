//! The current configuration is read with "get config" requests (cmd 0x27,
//! sub-index 0, 1, …); each 0x24 response carries a 50-byte chunk. Together
//! the chunks form one stream: a header followed by one record per channel.
//!
//! Layout, confirmed for both models by the original editor
//! (PROTOCOL-RE.md). All values are u16 little-endian unless marked u8.
//!
//! ```text
//! Header (16):   flags(2) preset name(14 ASCII)
//! Input (140):   name(8) | gate: attack release hold threshold
//!                | GEQ 31 × value | PEQ 8 × [gain freq Q:u8 type:u8]
//!                | filter: hpfFreq lpfFreq hpfSlope:u8 lpfSlope:u8
//!                | tail: gain phase delay link
//! Output:        name(8) | routing mask | crossing gain × inputs
//!                | filter (6) | PEQ 9 × 6 | compressor: ratio attack
//!                release knee threshold | limiter: attack release knee
//!                threshold | tail: gain phase delay link
//! Switches:      mute bits of the inputs | mute bits of the outputs
//!                | PEQ bypass bits (bit = band), one word per channel byte
//! Rest:          test tone type | sine index | delay display unit
//!                | EQ bypass bits of the inputs | of the outputs
//!                | GEQ bypass bits of the inputs
//! ```
//!
//! DSP 408: records end at 0x580, total 0x5A8 bytes (29 chunks).
//! DSP 206: records end at 0x380, total 0x3A0 bytes (19 chunks).

use crate::model::{DeviceModel, GEQ_BAND_COUNT};
use crate::protocol::{self as p, encode_channel_name};
use crate::state::{CompressorState, DeviceState, FilterState, GateState, LimiterState, PeqBand};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfigDump {
    pub model: DeviceModel,
}

/// Result of [ConfigDump::decode].
#[derive(Clone, Debug)]
pub struct ConfigDumpResult {
    pub state: DeviceState,
    /// Values that were out of range; empty if the dump looks plausible.
    pub problems: Vec<String>,
    /// False if the dump ended before the mute and PEQ bypass bits.
    pub switches_read: bool,
    /// False if the dump ended before the test tone, delay unit and EQ/GEQ
    /// bypass bits.
    pub rest_read: bool,
}

impl ConfigDumpResult {
    pub fn is_plausible(&self) -> bool {
        self.problems.is_empty()
    }
}

/// Bounds-checked reader that records implausible values.
struct Reader<'a> {
    data: &'a [u8],
    problems: Vec<String>,
}

impl Reader<'_> {
    fn check(&mut self, value: i64, max: i64, fallback: i64, what: Option<&str>) -> i64 {
        if value <= max {
            return value;
        }
        self.problems.push(format!("{} = {value} (max {max})", what.unwrap_or("value")));
        fallback
    }

    fn u8(&mut self, at: usize, max: i64, fallback: i64, what: &str) -> i64 {
        self.check(self.data[at] as i64, max, fallback, Some(what))
    }

    fn u16(&mut self, at: usize, max: i64, fallback: i64, what: Option<&str>) -> i64 {
        let value = self.data[at] as i64 | ((self.data[at + 1] as i64) << 8);
        self.check(value, max, fallback, what)
    }

    fn word(&mut self, at: usize) -> i64 {
        self.u16(at, 0xFFFF, 0, None)
    }

    /// Printable ASCII up to the first zero, or `None` if the bytes are not
    /// text.
    fn ascii(&mut self, at: usize, length: usize, what: Option<&str>) -> Option<String> {
        let bytes = &self.data[at..at + length];
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        let text = &bytes[..end];
        if text.iter().any(|b| !(0x20..=0x7E).contains(b)) || bytes[end..].iter().any(|b| *b != 0) {
            if let Some(what) = what {
                self.problems.push(format!("{what} is not text"));
            }
            return None;
        }
        Some(String::from_utf8_lossy(text).trim_end().to_string())
    }
}

impl ConfigDump {
    pub const CHUNK_SIZE: usize = 50;
    pub const HEADER_LENGTH: usize = 16;
    pub const INPUT_RECORD_LENGTH: usize = 140;

    pub const fn new(model: DeviceModel) -> Self {
        ConfigDump { model }
    }

    pub fn output_record_length(&self) -> usize {
        8 + 2 + 2 * self.model.inputs.len() + 6 + 9 * 6 + 10 + 8 + 8
    }

    /// Bytes needed to decode every channel record.
    pub fn required_length(&self) -> usize {
        Self::HEADER_LENGTH
            + self.model.inputs.len() * Self::INPUT_RECORD_LENGTH
            + self.model.outputs.len() * self.output_record_length()
    }

    /// Mute bits of the inputs; the outputs follow at +2.
    pub fn mute_offset(&self) -> usize {
        self.required_length()
    }

    /// PEQ bypass words, one per channel byte (inputs from 0, outputs from the
    /// model's first output byte).
    pub fn peq_bypass_offset(&self) -> usize {
        self.mute_offset() + 4
    }

    fn channel_slots(&self) -> usize {
        self.model.channel_byte(self.model.outputs[self.model.outputs.len() - 1]).unwrap_or(0) as usize + 1
    }

    /// Bytes needed to decode mute and PEQ bypass as well.
    pub fn switches_length(&self) -> usize {
        self.peq_bypass_offset() + 2 * self.channel_slots()
    }

    /// Test tone type and sine index (as in cmd 0x39).
    pub fn test_tone_offset(&self) -> usize {
        self.switches_length()
    }

    /// Delay display unit (cmd 0x15).
    pub fn delay_unit_offset(&self) -> usize {
        self.switches_length() + 4
    }

    /// Whole-PEQ bypass bits (cmd 0x3C) of the inputs; the outputs follow at +2.
    pub fn eq_bypass_offset(&self) -> usize {
        self.switches_length() + 6
    }

    /// GEQ bypass bits of the inputs (cmd 0x49).
    pub fn geq_bypass_offset(&self) -> usize {
        self.switches_length() + 10
    }

    /// Full dump length as the original editor reads it.
    pub fn total_length(&self) -> usize {
        self.switches_length() + 12
    }

    /// Number of 0x27 requests needed to read [Self::total_length] bytes.
    pub fn chunk_count(&self) -> usize {
        self.total_length().div_ceil(Self::CHUNK_SIZE)
    }

    pub fn input_offset(&self, index: usize) -> usize {
        Self::HEADER_LENGTH + index * Self::INPUT_RECORD_LENGTH
    }

    pub fn output_offset(&self, index: usize) -> usize {
        Self::HEADER_LENGTH + self.model.inputs.len() * Self::INPUT_RECORD_LENGTH + index * self.output_record_length()
    }

    // ─── Decode ───

    /// Decode [stream] (all chunks concatenated in order) into a new state.
    /// Values that fail plausibility checks are listed in
    /// [ConfigDumpResult::problems] and left at their defaults.
    pub fn decode(&self, stream: &[u8]) -> ConfigDumpResult {
        let model = self.model;
        let mut state = DeviceState::new(model);
        if stream.len() < self.required_length() {
            let problems = vec![format!("dump too short: {} of {} bytes", stream.len(), self.required_length())];
            return ConfigDumpResult { state, problems, switches_read: false, rest_read: false };
        }
        let mut r = Reader { data: stream, problems: Vec::new() };

        // Times and thresholds are checked loosely (to recognise a dump that is
        // not decoded right) and then limited to the original editor's ranges:
        // older panel versions allowed longer times.
        fn time(r: &mut Reader, at: usize, min: i64, max: i64, what: &str) -> i64 {
            r.u16(at, 9999, 0, Some(what)).clamp(min, max)
        }
        fn threshold(r: &mut Reader, at: usize, max: i64, what: &str) -> i64 {
            r.u16(at, 400, 0, Some(what)).clamp(0, max)
        }

        if let Some(name) = r.ascii(2, 14, None).filter(|n| !n.is_empty()) {
            state.current_preset = name;
        }

        for (i, ch) in model.inputs.iter().enumerate() {
            let o = self.input_offset(i);
            self.decode_name(&mut r, &mut state, ch, o);
            let gate = GateState {
                attack: time(&mut r, o + 8, 0, p::ATTACK_RAW_MAX, &format!("{ch} gate attack")),
                release: time(&mut r, o + 10, p::RELEASE_RAW_MIN, p::RELEASE_RAW_MAX, &format!("{ch} gate release")),
                hold: time(&mut r, o + 12, p::HOLD_RAW_MIN, p::HOLD_RAW_MAX, &format!("{ch} gate hold")),
                threshold: threshold(&mut r, o + 14, p::GATE_THRESHOLD_RAW_MAX, &format!("{ch} gate threshold")),
            };
            state.gates.insert(ch.to_string(), gate);
            let geq = (0..GEQ_BAND_COUNT)
                .map(|b| p::geq_value_to_db(r.u16(o + 16 + b * 2, 240, 120, Some(&format!("{ch} GEQ {b}")))))
                .collect();
            state.geq_bands.insert(ch.to_string(), geq);
            let peq = (0..8).map(|b| Self::decode_peq(&mut r, o + 78 + b * 6, &format!("{ch} PEQ {b}"))).collect();
            state.peq_bands.insert(ch.to_string(), peq);
            Self::decode_filters(&mut r, &mut state, ch, o + 126);
            Self::decode_tail(&mut r, &mut state, ch, o + 132);
        }

        let x_base = 10;
        let filter_base = x_base + 2 * model.inputs.len();
        for (j, ch) in model.outputs.iter().enumerate() {
            let o = self.output_offset(j);
            self.decode_name(&mut r, &mut state, ch, o);
            let routing = r.u16(o + 8, model.matrix_mask() as i64, 0, Some(&format!("{ch} routing")));
            state.matrix_routing.insert(ch.to_string(), routing);
            for (i, input) in model.inputs.iter().enumerate() {
                let raw = r.u16(o + x_base + i * 2, 400, 280, Some(&format!("{ch} crossing gain")));
                state.matrix_gains.insert(format!("{ch}:{input}"), p::gain_value_to_db(raw));
            }
            Self::decode_filters(&mut r, &mut state, ch, o + filter_base);
            let peq_base = filter_base + 6;
            let peq = (0..9).map(|b| Self::decode_peq(&mut r, o + peq_base + b * 6, &format!("{ch} PEQ {b}"))).collect();
            state.peq_bands.insert(ch.to_string(), peq);
            let comp = o + peq_base + 54;
            let compressor = CompressorState {
                ratio: r.u16(comp, p::COMPRESSOR_RATIO_NAMES.len() as i64 - 1, 0, Some(&format!("{ch} ratio"))),
                attack: time(&mut r, comp + 2, 0, p::ATTACK_RAW_MAX, &format!("{ch} comp attack")),
                release: time(&mut r, comp + 4, p::RELEASE_RAW_MIN, p::RELEASE_RAW_MAX, &format!("{ch} comp release")),
                knee: r.u16(comp + 6, p::KNEE_MAX, 0, Some(&format!("{ch} comp knee"))),
                threshold: threshold(&mut r, comp + 8, p::THRESHOLD_RAW_MAX, &format!("{ch} comp threshold")),
            };
            state.compressors.insert(ch.to_string(), compressor);
            let lim = comp + 10;
            let limiter = LimiterState {
                attack: time(&mut r, lim, 0, p::ATTACK_RAW_MAX, &format!("{ch} limiter attack")),
                release: time(&mut r, lim + 2, p::RELEASE_RAW_MIN, p::RELEASE_RAW_MAX, &format!("{ch} limiter release")),
                knee: r.u16(lim + 4, p::KNEE_MAX, 0, Some(&format!("{ch} limiter knee"))),
                threshold: threshold(&mut r, lim + 6, p::THRESHOLD_RAW_MAX, &format!("{ch} limiter threshold")),
            };
            state.limiters.insert(ch.to_string(), limiter);
            Self::decode_tail(&mut r, &mut state, ch, lim + 8);
        }

        // Mute and PEQ bypass follow the records; a shorter dump leaves them off.
        let switches_read = stream.len() >= self.switches_length();
        if switches_read {
            let input_mutes = r.word(self.mute_offset());
            let output_mutes = r.word(self.mute_offset() + 2);
            for (i, ch) in model.inputs.iter().enumerate() {
                state.mutes.insert(ch.to_string(), input_mutes & (1 << i) != 0);
            }
            for (j, ch) in model.outputs.iter().enumerate() {
                state.mutes.insert(ch.to_string(), output_mutes & (1 << j) != 0);
            }
            for ch in model.channels() {
                let byte = model.channel_byte(ch).unwrap_or(0) as usize;
                let bits = r.word(self.peq_bypass_offset() + 2 * byte);
                if let Some(bands) = state.peq_bands.get_mut(ch) {
                    for (b, band) in bands.iter_mut().enumerate() {
                        band.bypass = bits & (1 << b) != 0;
                    }
                }
            }
        }

        // Test tone, delay unit and the EQ/GEQ bypass bits end the dump. Values
        // out of range are ignored (left at their defaults) without rejecting
        // the dump: they are not needed to recognise the layout.
        let rest_read = stream.len() >= self.total_length();
        if rest_read {
            let tone_type = r.word(self.test_tone_offset());
            let index = r.word(self.test_tone_offset() + 2);
            if tone_type < p::TEST_TONE_TYPE_NAMES.len() as i64 && index < GEQ_BAND_COUNT as i64 {
                state.test_tone_type = tone_type;
                state.test_tone_param = index;
            }
            let unit = r.word(self.delay_unit_offset());
            if unit < p::DELAY_UNIT_NAMES.len() as i64 {
                state.delay_unit = unit;
            }
            let input_bypass = r.word(self.eq_bypass_offset());
            let output_bypass = r.word(self.eq_bypass_offset() + 2);
            let geq_bypass = r.word(self.geq_bypass_offset());
            for (i, ch) in model.inputs.iter().enumerate() {
                state.eq_bypass.insert(ch.to_string(), input_bypass & (1 << i) != 0);
                state.geq_bypass.insert(ch.to_string(), geq_bypass & (1 << i) != 0);
            }
            for (j, ch) in model.outputs.iter().enumerate() {
                state.eq_bypass.insert(ch.to_string(), output_bypass & (1 << j) != 0);
            }
        }

        ConfigDumpResult { state, problems: r.problems, switches_read, rest_read }
    }

    fn decode_name(&self, r: &mut Reader, state: &mut DeviceState, ch: &str, offset: usize) {
        if let Some(name) = r.ascii(offset, 8, Some(&format!("{ch} name"))) {
            state.channel_names.insert(ch.to_string(), name);
        }
    }

    fn decode_peq(r: &mut Reader, o: usize, what: &str) -> PeqBand {
        PeqBand {
            gain_db: p::peq_gain_to_db(r.u16(o, 240, 120, Some(&format!("{what} gain")))),
            freq_raw: r.u16(o + 2, p::FREQ_RAW_MAX, 150, Some(&format!("{what} freq"))),
            q_raw: r.u8(o + 4, 255, 35, &format!("{what} Q")).clamp(0, p::Q_RAW_MAX),
            peq_type: r.u8(o + 5, p::PEQ_TYPE_NAMES.len() as i64 - 1, 0, &format!("{what} type")),
            bypass: false,
        }
    }

    fn decode_filters(r: &mut Reader, state: &mut DeviceState, ch: &str, o: usize) {
        const MAX_SLOPE: i64 = p::CROSSOVER_SLOPE_NAMES.len() as i64 - 1;
        let hi = FilterState {
            freq_raw: r.u16(o, p::FREQ_RAW_MAX, 0, Some(&format!("{ch} HPF freq"))),
            slope: r.u8(o + 4, MAX_SLOPE, 0, &format!("{ch} HPF slope")),
        };
        let lo = FilterState {
            freq_raw: r.u16(o + 2, p::FREQ_RAW_MAX, 300, Some(&format!("{ch} LPF freq"))),
            slope: r.u8(o + 5, MAX_SLOPE, 0, &format!("{ch} LPF slope")),
        };
        state.hi_pass.insert(ch.to_string(), hi);
        state.lo_pass.insert(ch.to_string(), lo);
    }

    fn decode_tail(r: &mut Reader, state: &mut DeviceState, ch: &str, o: usize) {
        let gain = p::quantize_gain(p::gain_value_to_db(r.u16(o, 400, 280, Some(&format!("{ch} gain")))));
        state.gains.insert(ch.to_string(), gain);
        state.phase.insert(ch.to_string(), r.u16(o + 2, 1, 0, Some(&format!("{ch} phase"))) == 1);
        state.delays.insert(ch.to_string(), r.u16(o + 4, p::DELAY_RAW_MAX, 0, Some(&format!("{ch} delay"))));
        state.links.insert(ch.to_string(), r.u16(o + 6, 0xFF, 0, Some(&format!("{ch} link"))));
    }

    // ─── Encode ───

    /// Encode [state] as a dump stream of [Self::total_length] bytes.
    pub fn encode(&self, state: &DeviceState) -> Vec<u8> {
        let model = self.model;
        let mut out = vec![0u8; self.total_length()];
        fn u16(out: &mut [u8], at: usize, v: i64) {
            out[at] = (v & 0xFF) as u8;
            out[at + 1] = ((v >> 8) & 0xFF) as u8;
        }
        fn bits(flags: impl Iterator<Item = bool>) -> i64 {
            flags.enumerate().fold(0, |mask, (i, on)| if on { mask | (1 << i) } else { mask })
        }
        let name = |out: &mut [u8], at: usize, ch: &str| {
            let fallback = model.default_device_name(ch);
            let n = state.channel_names.get(ch).map_or(fallback.as_str(), String::as_str);
            out[at..at + 8].copy_from_slice(&encode_channel_name(n));
        };
        let peq = |out: &mut [u8], at: usize, b: &PeqBand| {
            u16(out, at, p::peq_db_to_gain(b.gain_db));
            u16(out, at + 2, b.freq_raw);
            out[at + 4] = b.q_raw as u8;
            out[at + 5] = b.peq_type as u8;
        };
        let default_hi = FilterState::new(0, 0);
        let default_lo = FilterState::new(300, 0);
        let filters = |out: &mut [u8], at: usize, ch: &str| {
            let hi = state.hi_pass.get(ch).unwrap_or(&default_hi);
            let lo = state.lo_pass.get(ch).unwrap_or(&default_lo);
            u16(out, at, hi.freq_raw);
            u16(out, at + 2, lo.freq_raw);
            out[at + 4] = hi.slope as u8;
            out[at + 5] = lo.slope as u8;
        };
        let tail = |out: &mut [u8], at: usize, ch: &str| {
            u16(out, at, p::gain_db_to_value(state.gains.get(ch).copied().unwrap_or(0.0)));
            u16(out, at + 2, state.phase.get(ch).copied().unwrap_or(false) as i64);
            u16(out, at + 4, state.delays.get(ch).copied().unwrap_or(0));
            u16(out, at + 6, state.link_mask(ch));
        };
        let bands_of = |ch: &str| -> Vec<PeqBand> {
            state.peq_bands.get(ch).cloned().unwrap_or_else(|| vec![PeqBand::default(); model.peq_band_count(ch)])
        };

        out[0] = 0xFF;
        out[1] = 0xFF;
        let preset = if state.current_preset == "Unknown" { "" } else { state.current_preset.as_str() };
        let mut units: Vec<u16> = preset.encode_utf16().take(14).collect();
        units.resize(14, b' ' as u16);
        for (k, c) in units.iter().enumerate() {
            out[2 + k] = (c & 0x7F) as u8;
        }

        for (i, ch) in model.inputs.iter().enumerate() {
            let o = self.input_offset(i);
            name(&mut out, o, ch);
            let g = state.gates.get(*ch).cloned().unwrap_or_default();
            u16(&mut out, o + 8, g.attack);
            u16(&mut out, o + 10, g.release);
            u16(&mut out, o + 12, g.hold);
            u16(&mut out, o + 14, g.threshold);
            let geq = state.geq_bands.get(*ch);
            for b in 0..GEQ_BAND_COUNT {
                let db = geq.and_then(|g| g.get(b)).copied().unwrap_or(0.0);
                u16(&mut out, o + 16 + b * 2, p::geq_db_to_value(db));
            }
            for (b, band) in bands_of(ch).iter().take(8).enumerate() {
                peq(&mut out, o + 78 + b * 6, band);
            }
            filters(&mut out, o + 126, ch);
            tail(&mut out, o + 132, ch);
        }

        let filter_base = 10 + 2 * model.inputs.len();
        for (j, ch) in model.outputs.iter().enumerate() {
            let o = self.output_offset(j);
            name(&mut out, o, ch);
            u16(&mut out, o + 8, state.matrix_routing.get(*ch).copied().unwrap_or(0));
            for (i, input) in model.inputs.iter().enumerate() {
                let db = state.matrix_gains.get(&format!("{ch}:{input}")).copied().unwrap_or(0.0);
                u16(&mut out, o + 10 + i * 2, p::gain_db_to_value(db));
            }
            filters(&mut out, o + filter_base, ch);
            let peq_base = o + filter_base + 6;
            for (b, band) in bands_of(ch).iter().take(9).enumerate() {
                peq(&mut out, peq_base + b * 6, band);
            }
            let c = state.compressors.get(*ch).cloned().unwrap_or_default();
            let comp = peq_base + 54;
            for (k, v) in [c.ratio, c.attack, c.release, c.knee, c.threshold].into_iter().enumerate() {
                u16(&mut out, comp + 2 * k, v);
            }
            let l = state.limiters.get(*ch).cloned().unwrap_or_default();
            for (k, v) in [l.attack, l.release, l.knee, l.threshold].into_iter().enumerate() {
                u16(&mut out, comp + 10 + 2 * k, v);
            }
            tail(&mut out, comp + 18, ch);
        }

        let flag = |m: &std::collections::BTreeMap<String, bool>, ch: &str| m.get(ch).copied().unwrap_or(false);
        u16(&mut out, self.mute_offset(), bits(model.inputs.iter().map(|c| flag(&state.mutes, c))));
        u16(&mut out, self.mute_offset() + 2, bits(model.outputs.iter().map(|c| flag(&state.mutes, c))));
        for ch in model.channels() {
            let byte = model.channel_byte(ch).unwrap_or(0) as usize;
            u16(&mut out, self.peq_bypass_offset() + 2 * byte, bits(bands_of(ch).iter().map(|b| b.bypass)));
        }
        u16(&mut out, self.test_tone_offset(), state.test_tone_type);
        u16(&mut out, self.test_tone_offset() + 2, state.test_tone_param);
        u16(&mut out, self.delay_unit_offset(), state.delay_unit);
        u16(&mut out, self.eq_bypass_offset(), bits(model.inputs.iter().map(|c| flag(&state.eq_bypass, c))));
        u16(&mut out, self.eq_bypass_offset() + 2, bits(model.outputs.iter().map(|c| flag(&state.eq_bypass, c))));
        u16(&mut out, self.geq_bypass_offset(), bits(model.inputs.iter().map(|c| flag(&state.geq_bypass, c))));
        out
    }

    /// Split a dump stream into the chunks the device sends.
    pub fn split(stream: &[u8]) -> Vec<Vec<u8>> {
        stream.chunks(Self::CHUNK_SIZE).map(<[u8]>::to_vec).collect()
    }
}
