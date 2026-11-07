//! Protocol of the DSP 206 / DSP 408: framing, value encodings, command
//! builders and decoding of device answers.
//!
//! Frame layout (verified against captured traffic):
//!
//! ```text
//! 10 02 [src] [dst] [len] [cmd] [len-1 payload bytes] 10 03 [checksum]
//! ```
//!
//! Total frame length is len + 8. The host is address 0, the device answers
//! from its device ID (1 by default). The checksum is 0x01 XOR every byte
//! from [src] up to the last payload byte (the same as XOR over the whole
//! frame from 10 02 to 10 03). Payload bytes are NOT escaped, so a payload
//! may contain 10 03 — always use [len] to find the end of a frame.
//!
//! Value encodings are shared by both models and follow the original editor
//! (ORIGINAL-SOFTWARE.md): exact frequency and Q formulas, its value ranges
//! and its display tables.

use std::f64::consts::{LN_2, LN_10};

use crate::model::{DeviceModel, GEQ_BAND_COUNT};

/// Error of a command builder (a channel or index the model does not have).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolError(pub String);

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProtocolError {}

pub type Frame = Vec<u8>;
type Result<T> = std::result::Result<T, ProtocolError>;

fn err<T>(message: impl Into<String>) -> Result<T> {
    Err(ProtocolError(message.into()))
}

// ─── Meters ───

/// Meter level shown for silence.
pub const SILENCE_DB: f64 = -120.0;

/// A meter slot holds the upper three bytes of an IEEE 754 float32:
/// `bits = d1 << 24 | d0 << 16 | d2 << 8` (read like this by the original
/// editor, which shows 20·log10 of it). 1.0 is full scale.
pub fn decode_meter_value(d0: u8, d1: u8, d2: u8) -> f64 {
    let bits = ((d1 as u32) << 24) | ((d0 as u32) << 16) | ((d2 as u32) << 8);
    let value = f32::from_bits(bits) as f64;
    // The DSP answers 0xFF 0xFF (NaN) for channels during PEQ recalculation.
    if value.is_finite() { value.abs() } else { 0.0 }
}

/// Level in dBFS of a linear meter value, at least [SILENCE_DB].
pub fn level_to_db(level: f64) -> f64 {
    if level <= 1e-6 { SILENCE_DB } else { SILENCE_DB.max(20.0 * level.ln() / LN_10) }
}

fn meter_at(data: &[u8], slot: usize) -> f64 {
    decode_meter_value(data[6 + slot * 3], data[7 + slot * 3], data[8 + slot * 3])
}

/// Parse [count] channel meter levels from a keepalive response frame.
/// Each channel is 3 bytes, see [decode_meter_value].
pub fn parse_meter_levels(data: &[u8], count: usize) -> Vec<f64> {
    // Data starts at byte 6 (after 10 02 01 00 LEN 40)
    if data.len() < 6 + count * 3 {
        return vec![0.0; count];
    }
    (0..count).map(|ch| meter_at(data, ch)).collect()
}

/// Status word (u16 LE) after the meter slots of a 0x40 response, or 0.
pub fn parse_dynamics(data: &[u8]) -> u16 {
    if data.len() < 6 {
        return 0;
    }
    let footer = (5 + data[4] as usize).clamp(6, data.len());
    let at = 6 + (footer - 6) / 3 * 3;
    if at + 2 <= footer { data[at] as u16 | ((data[at + 1] as u16) << 8) } else { 0 }
}

/// The compressor of the output with index [output] reduces the gain.
pub fn compressor_active(dynamics: u16, output: usize) -> bool {
    (dynamics as u32) & (1 << output) != 0
}

/// The limiter of the output with index [output] reduces the gain.
pub fn limiter_active(dynamics: u16, output: usize) -> bool {
    (dynamics as u32) & (0x100 << output) != 0
}

/// Meter levels of [model] in channel order (inputs, then outputs).
///
/// The 0x40 response of the DSP 408 has 12 slots: In A–D, then Out 1–8.
/// The DSP 206 answers with the same layout: its outputs start at slot 4
/// (slots 2 and 3 are unused). Shorter frames are read without the gap.
pub fn parse_model_meters(data: &[u8], model: &DeviceModel) -> Vec<f64> {
    if data.len() < 6 {
        return vec![0.0; model.meter_count()];
    }
    let footer = 5 + data[4] as usize;
    let slots = (footer.clamp(6, data.len()) - 6) / 3;
    const OUTPUT_SLOT: usize = 4;
    if model.inputs.len() < OUTPUT_SLOT && slots >= OUTPUT_SLOT + model.outputs.len() {
        return (0..model.inputs.len())
            .map(|i| meter_at(data, i))
            .chain((0..model.outputs.len()).map(|o| meter_at(data, OUTPUT_SLOT + o)))
            .collect();
    }
    parse_meter_levels(data, model.meter_count())
}

// ─── Framing ───

/// Checksum of the DSP protocol: 0x01 XOR every byte.
pub fn calculate_checksum<'a>(data: impl IntoIterator<Item = &'a u8>) -> u8 {
    data.into_iter().fold(1u8, |c, b| c ^ b)
}

/// Build a host → device frame for a command byte and its payload.
pub fn build_frame(command: u8, payload: &[u8]) -> Frame {
    let mut data = Vec::with_capacity(payload.len() + 4);
    data.extend_from_slice(&[0x00, 0x01, (payload.len() + 1) as u8, command]);
    data.extend_from_slice(payload);
    let checksum = calculate_checksum(&data);
    let mut frame = Vec::with_capacity(data.len() + 5);
    frame.extend_from_slice(&[0x10, 0x02]);
    frame.extend_from_slice(&data);
    frame.extend_from_slice(&[0x10, 0x03, checksum]);
    frame
}

/// [frame] addressed to device [id] (destination byte and checksum).
pub fn with_device_id(frame: &[u8], id: u8) -> Frame {
    if frame.len() < 8 || frame[3] == id {
        return frame.to_vec();
    }
    let mut out = frame.to_vec();
    out[3] = id;
    let last = out.len() - 1;
    out[last] = calculate_checksum(&out[2..out.len() - 3]);
    out
}

fn u16_le(value: i64) -> [u8; 2] {
    [(value & 0xFF) as u8, ((value >> 8) & 0xFF) as u8]
}

// ─── Channel gain ───

/// Dart's `num.round()`: half away from zero (same as `f64::round`).
fn round(x: f64) -> i64 {
    x.round() as i64
}

/// Convert dB value to DSP protocol value.
pub fn gain_db_to_value(db: f64) -> i64 {
    if db < -20.0 {
        // Coarse range: 0.5 dB resolution (2 units per dB)
        round((db + 60.0) * 2.0).clamp(0, 80)
    } else {
        // Fine range: 0.1 dB resolution (10 units per dB)
        round(80.0 + (db + 20.0) * 10.0).clamp(80, 400)
    }
}

/// Convert DSP protocol value back to dB (inverse of [gain_db_to_value]).
pub fn gain_value_to_db(value: i64) -> f64 {
    if value < 80 {
        return value as f64 / 2.0 - 60.0;
    }
    (value - 280) as f64 / 10.0
}

/// Quantize gain value to the step of its range.
pub fn quantize_gain(db: f64) -> f64 {
    if db < -20.0 {
        // Coarse range: round to nearest 0.5 dB
        round(db * 2.0) as f64 / 2.0
    } else {
        // Fine range: round to nearest 0.1 dB
        round(db * 10.0) as f64 / 10.0
    }
}

// ─── EQ gain (PEQ and GEQ) ───

/// GEQ dB value (−12.0 … +12.0) to protocol value (0 … 240):
/// value = dB · 10 + 120, resolution 0.1 dB.
pub fn geq_db_to_value(db: f64) -> i64 {
    round(db * 10.0 + 120.0).clamp(0, 240)
}

pub fn geq_value_to_db(value: i64) -> f64 {
    (value - 120) as f64 / 10.0
}

/// Quantize GEQ dB to 0.1 dB resolution.
pub fn quantize_geq_db(db: f64) -> f64 {
    round(db * 10.0) as f64 / 10.0
}

/// PEQ gain: same encoding as GEQ (0 … 240, dB = (value − 120) / 10).
pub fn peq_gain_to_db(value: i64) -> f64 {
    geq_value_to_db(value)
}

pub fn peq_db_to_gain(db: f64) -> i64 {
    geq_db_to_value(db)
}

// ─── Frequency (PEQ, HPF, LPF) ───

/// Frequency encoding of the original editor: 1/30-octave steps around
/// 1 kHz (raw 170), Hz = 1000 · 2^((raw − 170) / 30): raw 0 = 19.7 Hz,
/// raw 300 = 20.16 kHz.
pub const FREQ_RAW_MAX: i64 = 300;
const FREQ_RAW_1K: i64 = 170;

pub fn peq_freq_to_hz(raw: i64) -> f64 {
    1000.0 * 2f64.powf((raw.clamp(0, FREQ_RAW_MAX) - FREQ_RAW_1K) as f64 / 30.0)
}

pub fn peq_hz_to_freq(hz: f64) -> i64 {
    if hz <= 0.0 {
        return 0;
    }
    round(FREQ_RAW_1K as f64 + 30.0 * (hz / 1000.0).ln() / LN_2).clamp(0, FREQ_RAW_MAX)
}

// ─── Q ───

/// Q encoding of the original editor: 12 steps per doubling,
/// Q = 2^((raw − 16) / 12); raw 0 = 0.40, 16 = 1.0, 100 = 128.
pub const Q_RAW_MAX: i64 = 100;

pub fn peq_raw_to_q(raw: i64) -> f64 {
    2f64.powf((raw.clamp(0, Q_RAW_MAX) - 16) as f64 / 12.0)
}

pub fn peq_q_to_raw(q: f64) -> i64 {
    if q <= 0.0 {
        return 0;
    }
    round(12.0 * q.ln() / LN_2 + 16.0).clamp(0, Q_RAW_MAX)
}

/// Q as the original editor shows it (its display table rounds the formula
/// to two or three significant digits).
pub fn format_q(raw: i64) -> &'static str {
    Q_DISPLAY[raw.clamp(0, Q_RAW_MAX) as usize]
}

const Q_DISPLAY: [&str; 101] = [
    "0.40", "0.42", "0.45", "0.47", "0.50", "0.53", "0.56", "0.59", "0.63", "0.67", //
    "0.71", "0.75", "0.79", "0.84", "0.89", "0.94", "1.00", "1.06", "1.12", "1.19", //
    "1.26", "1.3", "1.4", "1.5", "1.6", "1.7", "1.8", "1.9", "2.0", "2.1", //
    "2.2", "2.4", "2.5", "2.7", "2.8", "3.0", "3.2", "3.4", "3.6", "3.8", //
    "4.0", "4.2", "4.5", "4.8", "5.0", "5.3", "5.7", "6.0", "6.3", "6.7", //
    "7.1", "7.6", "8.0", "8.5", "9.0", "9.5", "10.1", "10.7", "11.3", "12", //
    "12.7", "13.5", "14", "15", "16", "17", "18", "19", "20", "21", //
    "23", "24", "25", "27", "29", "30", "32", "34", "36", "38", //
    "40", "43", "45", "48", "51", "54", "57", "60", "64", "68", //
    "72", "76", "81", "85", "91", "96", "102", "108", "114", "121", //
    "128",
];

/// Q 3.0, the limit of shelves and the PEQ low/high-pass types.
pub const Q_RAW_LIMITED: i64 = 35;

/// Q 0.71, set by the original editor when a band changes to a limited type.
pub const Q_RAW_TYPE_RESET: i64 = 10;

/// Highest Q (raw) of a PEQ type: shelves and the PEQ low/high-pass types
/// are limited to Q 3.0 by the original editor, peak and all-pass use the
/// full range.
pub fn max_q_raw_for_type(peq_type: i64) -> i64 {
    if (1..=6).contains(&peq_type) { Q_RAW_LIMITED } else { Q_RAW_MAX }
}

// ─── Delay and dynamics ───

/// Delay: raw = ms · 96 (96 kHz sample clock); limited to 680 ms.
pub const DELAY_RAW_MAX: i64 = 680 * 96;

pub fn delay_raw_to_ms(raw: i64) -> f64 {
    raw as f64 / 96.0
}

pub fn delay_ms_to_raw(ms: f64) -> i64 {
    round(ms * 96.0).clamp(0, DELAY_RAW_MAX)
}

/// Delay display units (cmd 0x15, stored in the config dump).
pub const DELAY_UNIT_NAMES: [&str; 3] = ["ms", "m", "ft"];

/// Speed of sound the original editor uses for the distance display.
pub const METERS_PER_MS: f64 = 0.344;
pub const FEET_PER_METER: f64 = 1.0 / 0.3048;

/// Threshold (gate, compressor, limiter): raw = dB · 2 + 180.
pub fn threshold_raw_to_db(raw: i64) -> f64 {
    (raw - 180) as f64 / 2.0
}

pub fn threshold_db_to_raw(db: f64) -> i64 {
    round(db * 2.0 + 180.0).clamp(0, 0xFFFF)
}

/// Attack, release and hold times: raw = ms − 1.
pub fn time_raw_to_ms(raw: i64) -> i64 {
    raw + 1
}

pub fn time_ms_to_raw(ms: i64) -> i64 {
    (ms - 1).clamp(0, 0xFFFF)
}

/// Ranges of the original editor (raw): attack 0 … 998 (1 … 999 ms),
/// release 9 … 2999 (10 … 3000 ms), gate hold 9 … 998 (10 … 999 ms).
pub const ATTACK_RAW_MAX: i64 = 998;
pub const RELEASE_RAW_MIN: i64 = 9;
pub const RELEASE_RAW_MAX: i64 = 2999;
pub const HOLD_RAW_MIN: i64 = 9;
pub const HOLD_RAW_MAX: i64 = 998;

/// Thresholds: gate −90 … 0 dB, compressor and limiter −90 … +20 dB.
pub const GATE_THRESHOLD_RAW_MAX: i64 = 180;
pub const THRESHOLD_RAW_MAX: i64 = 220;

/// Knee of the compressor and the limiter in dB.
pub const KNEE_MAX: i64 = 12;

/// PEQ filter types.
pub const PEQ_TYPE_NAMES: [&str; 9] = [
    "Peak",
    "Low Shelf",
    "High Shelf",
    "LP -6dB",
    "LP -12dB",
    "HP -6dB",
    "HP -12dB",
    "All Pass 1",
    "All Pass 2",
];

/// Hi/Lo pass crossover slopes; index 0 = bypass (filter off).
/// BW = Butterworth, BL = Bessel, LK = Linkwitz-Riley.
pub const CROSSOVER_SLOPE_NAMES: [&str; 21] = [
    "Bypass", "BW -6", "BL -6", "BW -12", "BL -12", "LK -12", "BW -18", "BL -18", "BW -24", "BL -24",
    "LK -24", "BW -30", "BL -30", "BW -36", "BL -36", "LK -36", "BW -42", "BL -42", "BW -48",
    "BL -48", "LK -48",
];

/// Slope in dB/octave for a crossover slope index (0 for bypass).
pub fn slope_db_per_octave(slope: i64) -> i64 {
    if slope <= 0 || slope >= CROSSOVER_SLOPE_NAMES.len() as i64 {
        return 0;
    }
    CROSSOVER_SLOPE_NAMES[slope as usize]
        .rsplit(" -")
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Default slope when a filter is switched on (BW -24).
pub const DEFAULT_SLOPE: i64 = 8;

/// Compressor ratios by index.
pub const COMPRESSOR_RATIO_NAMES: [&str; 16] = [
    "1:1", "1:1.1", "1:1.3", "1:1.5", "1:1.7", "1:2", "1:2.5", "1:3", "1:3.5", "1:4", "1:5", "1:6",
    "1:8", "1:10", "1:20", "Limit",
];

/// Highest matrix crossing gain (the original editor allows −60 … 0 dB).
pub const MATRIX_GAIN_MAX_DB: f64 = 0.0;

/// Signal generator types of the test tone command (index = type byte);
/// "Analog input" switches the generator off.
pub const TEST_TONE_TYPE_NAMES: [&str; 4] = ["Analog input", "Pink noise", "White noise", "Sine"];

/// Device IDs the original editor can address.
pub const MIN_DEVICE_ID: u8 = 1;
pub const MAX_DEVICE_ID: u8 = 250;

/// 8 bytes of printable ASCII, zero-padded; other characters become '?'.
pub fn encode_channel_name(name: &str) -> [u8; 8] {
    let mut out = [0u8; 8];
    for (slot, c) in out.iter_mut().zip(name.encode_utf16()) {
        *slot = printable_or_question(c as u32);
    }
    out
}

fn printable_or_question(c: u32) -> u8 {
    if (0x20..=0x7E).contains(&c) { c as u8 } else { 0x3F }
}

// ─── Fixed requests ───

/// Command that is sent immediately after connecting.
pub fn handshake_command() -> Frame {
    build_frame(0x10, &[])
}

/// PTS / lock status (0x2C); answered with ptsOn limit count locked.
pub fn request_lock_status() -> Frame {
    build_frame(0x2C, &[])
}

/// Device information (0x13).
pub fn request_device_info() -> Frame {
    build_frame(0x13, &[])
}

/// Preset slot table (0x22): 20 bytes, 0xFF = slot used.
pub fn request_preset_slots() -> Frame {
    build_frame(0x22, &[])
}

/// Current preset number (0x14).
pub fn request_current_preset() -> Frame {
    build_frame(0x14, &[])
}

/// Name of user preset [index] (0 = U01).
pub fn request_preset_name(index: u8) -> Frame {
    build_frame(0x29, &[index])
}

/// Number of user presets (U01–U20).
pub const PRESET_COUNT: usize = 20;

/// Config dump request for one chunk (cmd 0x27). The number of chunks
/// depends on the model, see `ConfigDump::chunk_count`.
pub fn config_dump_command(sub_index: u8) -> Frame {
    build_frame(0x27, &[sub_index])
}

/// Disconnect (0x11): sent by the original editor when it ends a session.
pub fn build_disconnect_command() -> Frame {
    build_frame(0x11, &[])
}

/// Heartbeat (0x12): sent by the original editor while the link is idle,
/// and after a dump or a store.
pub fn build_heartbeat_command() -> Frame {
    build_frame(0x12, &[])
}

/// Keepalive / meter request (0x40).
pub fn build_keepalive_command() -> Frame {
    build_frame(0x40, &[])
}

/// Delay display unit (0x15): 0 = ms, 1 = m, 2 = ft.
pub fn build_delay_unit_command(unit: i64) -> Frame {
    build_frame(0x15, &[(unit & 0xFF) as u8])
}

/// Test tone (0x39): `type param` — type 0 = analog input (generator off),
/// 1 = pink noise, 2 = white noise, 3 = sine with param = index of the 31
/// ISO 1/3-octave frequencies (0 = 20 Hz … 30 = 20 kHz).
pub fn build_test_tone_command(tone_type: i64, param: i64) -> Frame {
    build_frame(0x39, &[(tone_type & 0xFF) as u8, (param & 0xFF) as u8])
}

/// Password check (0x2D): `mode password(4 ASCII)`, mode 0 = unlock, 1 = PTS.
/// The original editor converts the password to upper case.
pub fn build_password_command(mode: u8, password: &str) -> Frame {
    let upper: Vec<u16> = crate::format::to_upper_simple(password).encode_utf16().collect();
    let mut payload = vec![mode];
    for i in 0..4 {
        payload.push((upper.get(i).copied().unwrap_or(b' ' as u16) & 0x7F) as u8);
    }
    build_frame(0x2D, &payload)
}

/// Load preset (0x20) for the given 0-based preset index; the protocol
/// uses 1-based indices.
pub fn build_load_preset_command(preset_index: u8) -> Frame {
    build_frame(0x20, &[preset_index + 1])
}

/// Store preset name (0x26): the name padded with spaces to exactly 14
/// characters; characters outside printable ASCII become '?'.
pub fn build_store_preset_name_command(name: &str) -> Frame {
    let mut units: Vec<u16> = name.encode_utf16().take(14).collect();
    units.resize(14, b' ' as u16);
    let bytes: Vec<u8> = units.iter().map(|c| printable_or_question(*c as u32)).collect();
    build_frame(0x26, &bytes)
}

/// Store to preset slot (0x21); the slot is 1-based (U01 = 1).
pub fn build_store_preset_slot_command(preset_index: u8) -> Frame {
    build_frame(0x21, &[preset_index + 1])
}

/// Command sequence for storing the current settings to a preset slot like
/// the original editor: store name (0x26), store to slot (0x21), heartbeat.
pub fn store_preset_sequence(preset_index: u8, name: &str) -> Vec<Frame> {
    vec![
        build_store_preset_name_command(name),
        build_store_preset_slot_command(preset_index),
        build_heartbeat_command(),
    ]
}

// ─── Messages ───

/// A decoded device answer.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// Device info, e.g. "DSP408 V0104P".
    DeviceInfo { name: String },
    /// Name of a user preset.
    Preset { index: u8, name: String },
    /// Config chunk (cmd 0x24).
    ChannelConfig { sub_index: u8, chunk: Vec<u8> },
    /// PTS / lock status (answer to 0x2C): `ptsOn limit(BE) count(BE) locked`.
    LockStatus { pts_on: bool, limit: u16, count: u16, locked: bool },
    /// Preset slot table (answer to 0x22): used slots by 0-based index.
    PresetSlots { used: Vec<bool> },
    /// Current preset number (answer to 0x14).
    CurrentPreset { number: u8 },
    /// Result of a password check (answer to 0x2D); mode 0 = unlock, 1 = PTS.
    PasswordResult { mode: u8, ok: bool },
    /// Meter answer (0x40): linear levels (1.0 = full scale) in the model's
    /// channel order, and the dynamics word (bit n = compressor, bit 8 + n =
    /// limiter of output n working).
    Keepalive { levels: Vec<f64>, dynamics: u16 },
    /// Anything else, including the generic 0x01 acknowledgement.
    Unknown,
}

impl Message {
    /// The PTS limit is reached: the original editor asks for the PTS password.
    pub fn pts_expired(&self) -> bool {
        matches!(self, Message::LockStatus { pts_on: true, limit, count, .. } if limit <= count)
    }
}

fn ascii(bytes: &[u8]) -> String {
    let text: String = bytes
        .iter()
        .filter(|b| **b != 0)
        .map(|b| if (0x20..=0x7E).contains(b) { *b as char } else { '?' })
        .collect();
    text.trim().to_string()
}

/// Command builders and decoding for one model and device ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Protocol {
    /// Model whose channel bytes the command builders use.
    pub model: DeviceModel,
    /// Device ID: the address the device answers from (1 … 250). Command
    /// builders address ID 1; see [with_device_id].
    pub device_id: u8,
}

impl Default for Protocol {
    fn default() -> Self {
        Protocol::new(DeviceModel::DSP408)
    }
}

impl Protocol {
    pub const fn new(model: DeviceModel) -> Self {
        Protocol { model, device_id: 1 }
    }

    pub const fn with_id(model: DeviceModel, device_id: u8) -> Self {
        Protocol { model, device_id }
    }

    fn channel(&self, channel: &str) -> Result<u8> {
        match self.model.channel_byte(channel) {
            Some(ch) => Ok(ch),
            None => err(format!("Invalid channel for {}: {}", self.model.name, channel)),
        }
    }

    fn input(&self, channel: &str) -> Result<u8> {
        if !self.model.is_input(channel) {
            return err(format!("Invalid channel for {}: {}", self.model.name, channel));
        }
        self.channel(channel)
    }

    fn output(&self, channel: &str) -> Result<u8> {
        if !self.model.is_output(channel) {
            return err(format!("Invalid channel for {}: {}", self.model.name, channel));
        }
        self.channel(channel)
    }

    /// Gain (0x34): `ch value(2B LE)`
    pub fn build_gain_command(&self, channel: &str, db: f64) -> Result<Frame> {
        let ch = self.channel(channel)?;
        let v = u16_le(gain_db_to_value(quantize_gain(db)));
        Ok(build_frame(0x34, &[ch, v[0], v[1]]))
    }

    /// Mute (0x35): `ch 0|1`
    pub fn build_mute_command(&self, channel: &str, mute: bool) -> Result<Frame> {
        Ok(build_frame(0x35, &[self.channel(channel)?, mute as u8]))
    }

    /// Phase invert (0x36): `ch 0|1`
    pub fn build_phase_command(&self, channel: &str, invert: bool) -> Result<Frame> {
        Ok(build_frame(0x36, &[self.channel(channel)?, invert as u8]))
    }

    /// Delay (0x38): `ch value(2B LE)`, value = ms · 96
    pub fn build_delay_command(&self, channel: &str, raw: i64) -> Result<Frame> {
        let ch = self.channel(channel)?;
        let v = u16_le(raw.clamp(0, DELAY_RAW_MAX));
        Ok(build_frame(0x38, &[ch, v[0], v[1]]))
    }

    /// GEQ band (0x48): `ch band value(2B LE)` — inputs only
    pub fn build_geq_band_command(&self, channel: &str, band: usize, db: f64) -> Result<Frame> {
        let ch = self.input(channel)?;
        if band >= GEQ_BAND_COUNT {
            return err(format!("Invalid band index: {band}"));
        }
        let v = u16_le(geq_db_to_value(quantize_geq_db(db)));
        Ok(build_frame(0x48, &[ch, band as u8, v[0], v[1]]))
    }

    /// Matrix routing (0x3A): `output input_bitmask`
    pub fn build_matrix_routing_command(&self, output: &str, input_mask: i64) -> Result<Frame> {
        let ch = self.output(output)?;
        Ok(build_frame(0x3A, &[ch, (input_mask & self.model.matrix_mask() as i64) as u8]))
    }

    /// Matrix crossing gain (0x41): `output input_index value(2B LE)`, value
    /// encoded like the channel gain.
    pub fn build_matrix_gain_command(&self, output: &str, input: &str, db: f64) -> Result<Frame> {
        let ch = self.output(output)?;
        let Some(index) = self.model.input_index(input) else {
            return err(format!("Invalid input for {}: {}", self.model.name, input));
        };
        let v = u16_le(gain_db_to_value(quantize_gain(db.min(MATRIX_GAIN_MAX_DB))));
        Ok(build_frame(0x41, &[ch, index as u8, v[0], v[1]]))
    }

    /// PEQ band (0x33): `ch band gain(2B LE) freq(2B LE) Q type bypass`
    #[allow(clippy::too_many_arguments)]
    pub fn build_peq_band_command(
        &self,
        channel: &str,
        band: usize,
        gain_db: f64,
        freq_raw: i64,
        q_raw: i64,
        peq_type: i64,
        bypass: bool,
    ) -> Result<Frame> {
        let ch = self.channel(channel)?;
        if band >= self.model.peq_band_count(channel) {
            return err(format!("Invalid PEQ band: {band}"));
        }
        let g = u16_le(peq_db_to_gain(gain_db));
        let f = u16_le(freq_raw.clamp(0, FREQ_RAW_MAX));
        Ok(build_frame(
            0x33,
            &[ch, band as u8, g[0], g[1], f[0], f[1], (q_raw & 0xFF) as u8, peq_type as u8, bypass as u8],
        ))
    }

    /// Hi pass (0x32): `ch freq(2B LE) slope` — slope 0 = bypass
    pub fn build_hi_pass_command(&self, channel: &str, freq_raw: i64, slope: i64) -> Result<Frame> {
        let ch = self.channel(channel)?;
        let f = u16_le(freq_raw.clamp(0, FREQ_RAW_MAX));
        Ok(build_frame(0x32, &[ch, f[0], f[1], slope as u8]))
    }

    /// Lo pass (0x31): `ch freq(2B LE) slope` — slope 0 = bypass
    pub fn build_lo_pass_command(&self, channel: &str, freq_raw: i64, slope: i64) -> Result<Frame> {
        let ch = self.channel(channel)?;
        let f = u16_le(freq_raw.clamp(0, FREQ_RAW_MAX));
        Ok(build_frame(0x31, &[ch, f[0], f[1], slope as u8]))
    }

    /// Compressor (0x30): `ch ratio attack release knee threshold` (2B each)
    pub fn build_compressor_command(
        &self,
        channel: &str,
        ratio: i64,
        attack: i64,
        release: i64,
        knee: i64,
        threshold: i64,
    ) -> Result<Frame> {
        let ch = self.channel(channel)?;
        let mut payload = vec![ch];
        for v in [ratio, attack, release, knee, threshold] {
            payload.extend_from_slice(&u16_le(v));
        }
        Ok(build_frame(0x30, &payload))
    }

    /// Gate (0x3E): `ch attack release hold threshold` (2B each) — inputs only
    pub fn build_gate_command(
        &self,
        channel: &str,
        attack: i64,
        release: i64,
        hold: i64,
        threshold: i64,
    ) -> Result<Frame> {
        let ch = self.input(channel)?;
        let mut payload = vec![ch];
        for v in [attack, release, hold, threshold] {
            payload.extend_from_slice(&u16_le(v));
        }
        Ok(build_frame(0x3E, &payload))
    }

    /// Limiter (0x3F): `ch attack release knee threshold` (2B each)
    pub fn build_limiter_command(
        &self,
        channel: &str,
        attack: i64,
        release: i64,
        knee: i64,
        threshold: i64,
    ) -> Result<Frame> {
        let ch = self.channel(channel)?;
        let mut payload = vec![ch];
        for v in [attack, release, knee, threshold] {
            payload.extend_from_slice(&u16_le(v));
        }
        Ok(build_frame(0x3F, &payload))
    }

    /// Channel link (0x3B): `ch mask` — one byte, bits relative to the
    /// channel's group (In A = bit 0, Out 1 = bit 0). The original editor
    /// sends it for the master only: own bit + slaves.
    pub fn build_link_command(&self, channel: &str, mask: i64) -> Result<Frame> {
        Ok(build_frame(0x3B, &[self.channel(channel)?, (mask & 0xFF) as u8]))
    }

    /// Copy channel (0x2A): `src dst` (absolute channel bytes, same group).
    /// The device copies the settings itself; the original editor reads the
    /// whole configuration afterwards.
    pub fn build_copy_channel_command(&self, from: &str, to: &str) -> Result<Frame> {
        let src = self.channel(from)?;
        let dst = self.channel(to)?;
        if self.model.is_input(from) != self.model.is_input(to) {
            return err("Only inputs or only outputs can be copied");
        }
        Ok(build_frame(0x2A, &[src, dst]))
    }

    /// Bypass the whole PEQ of a channel (0x3C): `ch 1|0`.
    pub fn build_eq_bypass_command(&self, channel: &str, bypass: bool) -> Result<Frame> {
        Ok(build_frame(0x3C, &[self.channel(channel)?, bypass as u8]))
    }

    /// Bypass the GEQ of an input (0x49): `ch 1|0`.
    pub fn build_geq_bypass_command(&self, channel: &str, bypass: bool) -> Result<Frame> {
        Ok(build_frame(0x49, &[self.input(channel)?, bypass as u8]))
    }

    /// Channel name (0x3D): `ch name(8 ASCII, zero-padded)`
    pub fn build_channel_name_command(&self, channel: &str, name: &str) -> Result<Frame> {
        let mut payload = vec![self.channel(channel)?];
        payload.extend_from_slice(&encode_channel_name(name));
        Ok(build_frame(0x3D, &payload))
    }

    // ─── Decoding ───

    /// Decode a complete protocol frame (as produced by the frame decoder).
    /// Only answers of this device (source = device ID, destination = host)
    /// are decoded; the original editor ignores other addresses.
    pub fn decode(&self, data: &[u8]) -> Option<Message> {
        if data.len() < 8 || data[0] != 0x10 || data[1] != 0x02 {
            return None;
        }
        // End of payload, i.e. position of the 10 03 footer
        let footer = 5 + data[4] as usize;
        if footer > data.len() {
            return None;
        }
        let is_response = data[2] == self.device_id && data[3] == 0x00;
        if !is_response || data.len() <= 7 {
            return None;
        }
        let cmd = data[5];
        let data_byte = if footer > 6 { Some(data[6]) } else { None };

        Some(match cmd {
            0x40 => Message::Keepalive {
                levels: parse_model_meters(data, &self.model),
                dynamics: parse_dynamics(data),
            },
            0x13 if footer > 6 => Message::DeviceInfo { name: ascii(&data[6..footer]) },
            0x29 if data_byte.is_some() => Message::Preset {
                index: data[6],
                name: ascii(data.get(7..footer).unwrap_or(&[])),
            },
            0x2C if footer >= 12 => Message::LockStatus {
                pts_on: data[6] != 0,
                limit: ((data[7] as u16) << 8) | data[8] as u16,
                count: ((data[9] as u16) << 8) | data[10] as u16,
                locked: data[11] != 0,
            },
            0x22 if footer > 6 => Message::PresetSlots { used: data[6..footer].iter().map(|b| *b == 0xFF).collect() },
            0x14 if data_byte.is_some() => Message::CurrentPreset { number: data[6] },
            0x2D if footer >= 8 => Message::PasswordResult { mode: data[6], ok: data[7] == 1 },
            0x24 if data_byte.is_some() => Message::ChannelConfig {
                sub_index: data[6],
                chunk: data.get(7..footer).unwrap_or(&[]).to_vec(),
            },
            _ => Message::Unknown,
        })
    }
}

// ─── Hex helpers ───

/// Format data as hex string for display ("10 02 00 …").
pub fn format_as_hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

/// Format data as ASCII string for display.
pub fn format_as_ascii(data: &[u8]) -> String {
    data.iter().map(|b| if (0x20..=0x7E).contains(b) { *b as char } else { '.' }).collect()
}

/// Bytes of a hex string ("1002…"); invalid digits give `None`.
pub fn hex_to_bytes(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok()).collect()
}
