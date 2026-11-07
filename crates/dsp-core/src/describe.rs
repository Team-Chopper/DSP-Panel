//! Human-readable descriptions of protocol frames for the debug log.

use crate::format::to_fixed;
use crate::model::DeviceModel;
use crate::protocol::{self as p, Message, Protocol};

/// 31-band ISO 1/3-octave centre frequencies for the GEQ decode.
const GEQ_FREQS: [&str; 31] = [
    "20", "25", "31.5", "40", "50", "63", "80", "100", "125", "160", "200", "250", "315", "400", "500", "630", "800",
    "1K", "1.25K", "1.6K", "2K", "2.5K", "3.15K", "4K", "5K", "6.3K", "8K", "10K", "12.5K", "16K", "20K",
];

fn db(value: f64) -> String {
    if value >= 0.0 { format!("+{}", to_fixed(value, 1)) } else { to_fixed(value, 1) }
}

fn hz(value: f64) -> String {
    if value >= 1000.0 {
        let digits = if value >= 10000.0 { 1 } else { 2 };
        format!("{} kHz", to_fixed(value / 1000.0, digits))
    } else {
        format!("{} Hz", to_fixed(value, 0))
    }
}

fn u16_at(d: &[u8], at: usize) -> i64 {
    d[at] as i64 | ((d[at + 1] as i64) << 8)
}

fn slope(s: u8) -> String {
    p::CROSSOVER_SLOPE_NAMES.get(s as usize).map_or_else(|| s.to_string(), |n| n.to_string())
}

/// Preset number as shown by the original editor: 0 = F00, n = Un.
fn slot(n: u8) -> String {
    if n == 0 { "F00".into() } else { format!("U{n:02}") }
}

/// Describe a device → host frame, using the decoded message where possible.
pub fn describe_rx(data: &[u8], model: &DeviceModel, device_id: u8) -> String {
    match Protocol::with_id(*model, device_id).decode(data) {
        Some(Message::Preset { index, name }) => format!("PRESET U{:02}: \"{name}\"", index as u32 + 1),
        Some(Message::ChannelConfig { sub_index, chunk }) => {
            format!("CONFIG chunk 0x{sub_index:02x} ({} bytes)", chunk.len())
        }
        Some(Message::DeviceInfo { name }) => format!("DEVICE: {name}"),
        Some(Message::Keepalive { levels, dynamics }) => {
            let channels: Vec<&str> = model.channels().collect();
            let parts: Vec<String> = levels
                .iter()
                .zip(&channels)
                .filter_map(|(level, ch)| {
                    let v = p::level_to_db(*level);
                    (v > -100.0).then(|| format!("{ch}:{}", to_fixed(v, 1)))
                })
                .collect();
            let mut dyn_parts = Vec::new();
            for (o, out) in model.outputs.iter().enumerate() {
                if p::compressor_active(dynamics, o) {
                    dyn_parts.push(format!("{out}:COMP"));
                }
                if p::limiter_active(dynamics, o) {
                    dyn_parts.push(format!("{out}:LIMIT"));
                }
            }
            let levels = if parts.is_empty() { "(silence)".to_string() } else { parts.join(" ") };
            let dynamics = if dyn_parts.is_empty() { String::new() } else { format!(" {}", dyn_parts.join(" ")) };
            format!("METERS dBFS {levels}{dynamics}")
        }
        Some(Message::LockStatus { pts_on, limit, count, locked }) => format!(
            "LOCK STATUS: {}, PTS {} (count {count} of {limit})",
            if locked { "locked" } else { "not locked" },
            if pts_on { "on" } else { "off" },
        ),
        Some(Message::PresetSlots { used }) => {
            let used: Vec<String> =
                used.iter().enumerate().filter(|(_, u)| **u).map(|(i, _)| slot(i as u8 + 1)).collect();
            format!("PRESET SLOTS used: {}", if used.is_empty() { "none".to_string() } else { used.join(" ") })
        }
        Some(Message::CurrentPreset { number }) => format!("CURRENT PRESET: {}", slot(number)),
        Some(Message::PasswordResult { mode, ok }) => format!(
            "PASSWORD ({}): {}",
            if mode == 1 { "PTS" } else { "unlock" },
            if ok { "correct" } else { "wrong" },
        ),
        _ if data.len() > 5 && data[5] == 0x01 => "ACK".into(),
        _ => describe(data, model),
    }
}

/// Describe a raw protocol frame.
pub fn describe(data: &[u8], model: &DeviceModel) -> String {
    if data.len() < 6 || data[0] != 0x10 || data[1] != 0x02 {
        return format!("UNKNOWN ({} bytes)", data.len());
    }
    let cmd = data[5];
    let len = data.len();
    let ch_name = |byte: u8| model.channel_for_byte(byte).map_or_else(|| format!("ch 0x{byte:x}"), String::from);
    let ch = || if len > 6 { ch_name(data[6]) } else { "?".into() };

    match cmd {
        0x10 => "HANDSHAKE".into(),
        0x40 => "METERS → request meter levels".into(),
        0x11 => "DISCONNECT".into(),
        0x12 => "HEARTBEAT".into(),
        0x13 => "GET DEVICE INFO".into(),
        0x14 => "GET CURRENT PRESET".into(),
        0x15 if len > 6 => {
            let unit = p::DELAY_UNIT_NAMES.get(data[6] as usize).copied().unwrap_or("?");
            format!("SET DELAY UNIT → {unit}")
        }
        0x15 => "SET DELAY UNIT".into(),
        0x22 => "GET PRESET SLOTS".into(),
        0x2C => "GET LOCK STATUS".into(),
        0x2D if len > 6 => format!("CHECK {} PASSWORD", if data[6] == 1 { "PTS" } else { "UNLOCK" }),
        0x2D => "CHECK PASSWORD".into(),
        0x2A if len > 7 => format!("COPY CHANNEL {} → {}", ch(), ch_name(data[7])),
        0x2A => "COPY CHANNEL".into(),
        0x20 if len > 6 => format!("LOAD PRESET {}", slot(data[6])),
        0x20 => "LOAD PRESET".into(),
        0x21 if len > 6 => format!("STORE PRESET → {}", slot(data[6])),
        0x21 => "STORE PRESET".into(),
        0x26 => "STORE PRESET NAME".into(),
        0x29 if len > 6 => format!("GET PRESET U{:02} → request name", data[6] as u32 + 1),
        0x29 => "GET PRESET".into(),
        0x27 if len > 6 => format!("GET CONFIG CHUNK 0x{:02x}", data[6]),
        0x27 => "GET CONFIG".into(),
        0x34 if len > 8 => format!("SET GAIN {} → {} dB", ch(), db(p::gain_value_to_db(u16_at(data, 7)))),
        0x34 => "SET GAIN".into(),
        0x35 if len > 7 => format!("SET MUTE {} → {}", ch(), if data[7] == 1 { "MUTED" } else { "UNMUTED" }),
        0x35 => "SET MUTE".into(),
        0x36 if len > 7 => format!("SET PHASE {} → {}", ch(), if data[7] == 1 { "INVERTED" } else { "NORMAL" }),
        0x36 => "SET PHASE".into(),
        0x38 if len > 8 => format!("SET DELAY {} → {} ms", ch(), to_fixed(p::delay_raw_to_ms(u16_at(data, 7)), 2)),
        0x38 => "SET DELAY".into(),
        0x48 if len > 8 => {
            let band = data[7] as usize;
            let freq = GEQ_FREQS.get(band).map_or_else(|| format!("#{band}"), |f| f.to_string());
            format!("SET GEQ {} {freq}Hz → {} dB", ch(), db(p::geq_value_to_db(u16_at(data, 8))))
        }
        0x48 => "SET GEQ".into(),
        0x3C if len > 7 => format!("SET EQ BYPASS {} → {}", ch(), if data[7] == 1 { "BYPASS" } else { "ACTIVE" }),
        0x3C => "SET EQ BYPASS".into(),
        0x49 if len > 7 => format!("SET GEQ BYPASS {} → {}", ch(), if data[7] == 1 { "BYPASS" } else { "ACTIVE" }),
        0x49 => "SET GEQ BYPASS".into(),
        0x33 if len > 14 => {
            let peq_type = p::PEQ_TYPE_NAMES.get(data[13] as usize).map_or_else(|| data[13].to_string(), |t| t.to_string());
            let bypass = if data[14] == 1 { " [BYPASS]" } else { "" };
            format!(
                "SET PEQ {} band {} → {} dB {} Q {} {peq_type}{bypass}",
                ch(),
                data[7] as u32 + 1,
                db(p::peq_gain_to_db(u16_at(data, 8))),
                hz(p::peq_freq_to_hz(u16_at(data, 10))),
                to_fixed(p::peq_raw_to_q(data[12] as i64), 2),
            )
        }
        0x33 => "SET PEQ".into(),
        0x32 | 0x31 => {
            let name = if cmd == 0x32 { "HPF" } else { "LPF" };
            if len > 9 {
                let s = data[9];
                if s == 0 {
                    format!("SET {name} {} → BYPASS", ch())
                } else {
                    format!("SET {name} {} → {} {}", ch(), hz(p::peq_freq_to_hz(u16_at(data, 7))), slope(s))
                }
            } else {
                format!("SET {name}")
            }
        }
        0x30 if len > 16 => {
            let ratio = u16_at(data, 7);
            let ratio_name =
                p::COMPRESSOR_RATIO_NAMES.get(ratio as usize).map_or_else(|| ratio.to_string(), |r| r.to_string());
            format!(
                "SET COMPRESSOR {} → {ratio_name}, {} dB, {}/{} ms, knee {} dB",
                ch(),
                db(p::threshold_raw_to_db(u16_at(data, 15))),
                p::time_raw_to_ms(u16_at(data, 9)),
                p::time_raw_to_ms(u16_at(data, 11)),
                u16_at(data, 13),
            )
        }
        0x30 => "SET COMPRESSOR".into(),
        0x3E if len > 14 => format!(
            "SET GATE {} → {} dB, attack/release/hold {}/{}/{} ms",
            ch(),
            db(p::threshold_raw_to_db(u16_at(data, 13))),
            p::time_raw_to_ms(u16_at(data, 7)),
            p::time_raw_to_ms(u16_at(data, 9)),
            p::time_raw_to_ms(u16_at(data, 11)),
        ),
        0x3E => "SET GATE".into(),
        0x3F if len > 14 => format!(
            "SET LIMITER {} → {} dB, {}/{} ms, knee {} dB",
            ch(),
            db(p::threshold_raw_to_db(u16_at(data, 13))),
            p::time_raw_to_ms(u16_at(data, 7)),
            p::time_raw_to_ms(u16_at(data, 9)),
            u16_at(data, 11),
        ),
        0x3F => "SET LIMITER".into(),
        0x39 if len > 8 => {
            let t = data[6];
            let name = p::TEST_TONE_TYPE_NAMES.get(t as usize).copied().unwrap_or("?");
            let extra = if t == 3 { format!(" (frequency index {})", data[7]) } else { String::new() };
            format!("SET TEST TONE → {name}{extra}")
        }
        0x39 => "SET TEST TONE".into(),
        0x3B if len > 7 => {
            let channel = ch();
            let linked: Vec<&str> = model
                .group_of(&channel)
                .iter()
                .copied()
                .filter(|c| data[7] as i32 & model.link_bit(c) != 0)
                .collect();
            format!("SET LINK {channel} → {}", linked.join(" + "))
        }
        0x3B => "SET LINK".into(),
        0x3D if len > 14 => {
            let name: String = data[7..15].iter().filter(|b| **b != 0).map(|b| *b as char).collect();
            format!("SET NAME {} → \"{name}\"", ch())
        }
        0x3D => "SET NAME".into(),
        0x3A if len > 7 => {
            let mask = data[7] as i32;
            let inputs: Vec<&str> = model.inputs.iter().copied().filter(|i| mask & model.matrix_bit(i) != 0).collect();
            format!("SET MATRIX {} ← {}", ch(), if inputs.is_empty() { "none".to_string() } else { inputs.join(" + ") })
        }
        0x3A => "SET MATRIX".into(),
        0x41 if len > 9 => {
            let input = model.inputs.get(data[7] as usize).map_or_else(|| format!("input {}", data[7]), |i| i.to_string());
            format!("SET MATRIX GAIN {} ← {input} → {} dB", ch(), db(p::gain_value_to_db(u16_at(data, 8))))
        }
        0x41 => "SET MATRIX GAIN".into(),
        0x24 if len > 6 => format!("CONFIG CHUNK 0x{:02x} ({len} bytes)", data[6]),
        0x24 => "CONFIG CHUNK".into(),
        _ => format!("CMD 0x{cmd:02x} ({len} bytes)"),
    }
}
