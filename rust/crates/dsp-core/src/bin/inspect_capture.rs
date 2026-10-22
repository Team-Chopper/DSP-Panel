//! Inspect a captured DSP session (helper for the tasks in TODO.md).
//!
//! ```text
//! cargo run -p dsp-core --bin inspect-capture -- <capture.txt> [--model dsp206]
//! ```
//!
//! The capture file has one frame per line as hex, optionally preceded by a
//! line number ("103\t10020100342400…"), e.g. exported from Wireshark with
//! "Copy as hex stream". The tool prints every frame decoded and, if the
//! capture contains a config dump, the decoded channel records and every
//! value the decoder considered implausible.

use std::collections::BTreeMap;
use std::process::ExitCode;

use dsp_core::describe::{describe, describe_rx};
use dsp_core::format::{double_to_string, to_fixed};
use dsp_core::protocol::{self as p, Message, Protocol, hex_to_bytes};
use dsp_core::{ConfigDump, DeviceModel, FrameDecoder};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: inspect-capture <capture.txt> [--model dsp206]");
        return ExitCode::from(64);
    };
    let mut model = args.iter().position(|a| a == "--model").map(|i| DeviceModel::by_id(args.get(i + 1).map(String::as_str)));
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut chunks = BTreeMap::new();
    for line in text.lines() {
        let hex = line.split_whitespace().last().unwrap_or_default().to_lowercase();
        if !hex.starts_with("1002") || hex.len() % 2 == 1 {
            continue;
        }
        let Some(bytes) = hex_to_bytes(&hex) else { continue };
        if bytes.len() < 6 {
            continue;
        }
        let valid = FrameDecoder::new().add(&bytes).len() == 1;
        let rx = bytes[2] == 0x01;
        let current = model.unwrap_or(DeviceModel::DSP408);
        match Protocol::new(current).decode(&bytes) {
            Some(Message::DeviceInfo { name }) if model.is_none() => model = DeviceModel::detect(&name),
            Some(Message::ChannelConfig { sub_index, chunk }) => {
                chunks.insert(sub_index, chunk);
            }
            _ => {}
        }
        let current = model.unwrap_or(DeviceModel::DSP408);
        let desc = if rx { describe_rx(&bytes, &current, 1) } else { describe(&bytes, &current) };
        println!("{} {} {desc}", if rx { "RX" } else { "TX" }, if valid { " " } else { "!" });
    }

    if chunks.is_empty() {
        return ExitCode::SUCCESS;
    }
    let model = model.unwrap_or(DeviceModel::DSP408);
    let mut stream = Vec::new();
    for i in 0.. {
        let Some(chunk) = chunks.get(&i) else { break };
        stream.extend_from_slice(chunk);
        if i == u8::MAX {
            break;
        }
    }
    let dump = ConfigDump::new(model);
    println!(
        "\nConfig dump: {} chunks, {} bytes ({} layout expects {})",
        chunks.len(),
        stream.len(),
        model.name,
        dump.total_length()
    );
    let result = dump.decode(&stream);
    let s = &result.state;
    let hz = |raw: i64| to_fixed(p::peq_freq_to_hz(raw), 0);
    println!("Preset: \"{}\"", s.current_preset);
    for ch in model.channels() {
        let (hp, lp) = (&s.hi_pass[ch], &s.lo_pass[ch]);
        let routing = if model.is_output(ch) { format!("routing=0x{:x}", s.matrix_routing[ch]) } else { String::new() };
        println!(
            "{ch:<6} name=\"{}\" gain={} dB {}{}delay={} ms HPF {} Hz {} LPF {} Hz {} {routing}",
            s.channel_names[ch],
            double_to_string(s.gains[ch]),
            if s.mutes[ch] { "MUTE " } else { "" },
            if s.phase[ch] { "PHASE " } else { "" },
            to_fixed(p::delay_raw_to_ms(s.delays[ch]), 2),
            hz(hp.freq_raw),
            p::CROSSOVER_SLOPE_NAMES[hp.slope as usize],
            hz(lp.freq_raw),
            p::CROSSOVER_SLOPE_NAMES[lp.slope as usize],
        );
    }
    if !result.switches_read {
        println!("\nDump ends before the mute/PEQ bypass bits.");
    }
    if result.is_plausible() {
        println!("\nAll values plausible.");
    } else {
        println!("\nImplausible values (layout probably differs):");
        for problem in result.problems.iter().take(40) {
            println!("  {problem}");
        }
    }

    // Raw words around the record tails, to locate unknown fields.
    println!("\nRecord tails [gain phase delay link]:");
    let tail = |at: usize, ch: &str| {
        if at + 8 > stream.len() {
            return;
        }
        let words: Vec<String> =
            (0..8).step_by(2).map(|k| (stream[at + k] as u32 | ((stream[at + k + 1] as u32) << 8)).to_string()).collect();
        println!("  {ch:<6} [{}]", words.join(", "));
    };
    for (i, ch) in model.inputs.iter().enumerate() {
        tail(dump.input_offset(i) + 132, ch);
    }
    for (j, ch) in model.outputs.iter().enumerate() {
        tail(dump.output_offset(j) + dump.output_record_length() - 8, ch);
    }
    ExitCode::SUCCESS
}
