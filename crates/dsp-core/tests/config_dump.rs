//! Port of dart/packages/dsp_core/test/config_dump_test.dart.

use std::collections::{BTreeMap, BTreeSet};

use dsp_core::ops::{self, Op};
use dsp_core::protocol as p;
use dsp_core::testing::{captured_config_chunks, captured_test_preset_chunks};
use dsp_core::{CompressorState, ConfigDump, DeviceModel, DeviceState, FilterState, GateState, LimiterState, PeqBand};
use serde_json::json;

const DUMP408: ConfigDump = ConfigDump::new(DeviceModel::DSP408);
const DUMP206: ConfigDump = ConfigDump::new(DeviceModel::DSP206);
const DUMP204: ConfigDump = ConfigDump::new(DeviceModel::DSP204);

fn default_stream() -> Vec<u8> {
    captured_config_chunks().concat()
}

fn test_stream() -> Vec<u8> {
    captured_test_preset_chunks().concat()
}

fn u16_at(stream: &[u8], at: usize) -> i64 {
    stream[at] as i64 | ((stream[at + 1] as i64) << 8)
}

#[test]
fn layout_sizes() {
    assert_eq!(DUMP408.output_record_length(), 104);
    assert_eq!(DUMP408.required_length(), 1408);
    assert_eq!(DUMP408.mute_offset(), 0x580);
    assert_eq!(DUMP408.peq_bypass_offset(), 0x584);
    assert_eq!(DUMP408.switches_length(), 0x59C);
    assert_eq!(DUMP408.total_length(), 0x5A8);
    assert_eq!(DUMP408.chunk_count(), 29);
    assert_eq!(DUMP206.output_record_length(), 100);
    assert_eq!(DUMP206.required_length(), 896);
    assert_eq!(DUMP206.mute_offset(), 0x380);
    assert_eq!(DUMP206.peq_bypass_offset(), 0x384);
    assert_eq!(DUMP206.switches_length(), 0x394);
    assert_eq!(DUMP206.total_length(), 0x3A0);
    assert_eq!(DUMP206.chunk_count(), 19);
    // DSP 204 as measured on a device by another project: outputs at 0x128,
    // mute bits at 0x2B8, 724 bytes in 15 chunks.
    assert_eq!(DUMP204.output_record_length(), 100);
    assert_eq!(DUMP204.output_offset(0), 0x128);
    assert_eq!(DUMP204.mute_offset(), 0x2B8);
    assert_eq!(DUMP204.switches_length(), 0x2C8);
    assert_eq!(DUMP204.total_length(), 724);
    assert_eq!(DUMP204.chunk_count(), 15);
    // The captured dumps have exactly the expected size.
    assert_eq!(default_stream().len(), DUMP408.total_length());
    assert_eq!(test_stream().len(), DUMP408.total_length());
}

fn changed(model: DeviceModel) -> DeviceState {
    let mut state = DeviceState::new(model);
    let last_in = model.inputs[model.inputs.len() - 1];
    let last_out = model.outputs[model.outputs.len() - 1];
    for op in [
        Op::mute(last_in, true),
        Op::mute(last_out, true),
        Op::phase("In A", true),
        Op::phase("Out 2", true),
        Op::delay("Out 3", 960),
        Op::delay("In B", 65280),
        Op::peq_band("In A", 0, PeqBand { bypass: true, ..PeqBand::default() }),
        Op::peq_band(last_out, 8, PeqBand { bypass: true, ..PeqBand::default() }),
    ] {
        ops::apply(&mut state, &op);
    }
    state
}

#[test]
fn mute_phase_delay_and_peq_bypass_written_and_read_at_their_offsets() {
    for dump in [DUMP408, DUMP206, DUMP204] {
        let model = dump.model;
        let state = changed(model);
        let stream = dump.encode(&state);
        assert_eq!(u16_at(&stream, dump.mute_offset()), 1 << (model.inputs.len() - 1));
        assert_eq!(u16_at(&stream, dump.mute_offset() + 2), 1 << (model.outputs.len() - 1));
        assert_eq!(u16_at(&stream, dump.peq_bypass_offset()), 1, "In A band 1");
        let last_out = model.outputs[model.outputs.len() - 1];
        let at = dump.peq_bypass_offset() + 2 * model.channel_byte(last_out).unwrap() as usize;
        assert_eq!(u16_at(&stream, at), 1 << 8);
        // Channel tail: gain, phase, delay, link
        assert_eq!(u16_at(&stream, dump.input_offset(0) + 134), 1);
        assert_eq!(u16_at(&stream, dump.output_offset(1) + dump.output_record_length() - 6), 1);
        assert_eq!(u16_at(&stream, dump.output_offset(2) + dump.output_record_length() - 4), 960);

        let decoded = dump.decode(&stream);
        assert!(decoded.problems.is_empty(), "{:?}", decoded.problems);
        assert!(decoded.switches_read);
        assert_eq!(decoded.state.to_json(), state.to_json());
    }
}

#[test]
fn test_tone_delay_unit_eq_and_geq_bypass_at_the_end_of_the_dump() {
    for dump in [DUMP408, DUMP206] {
        let model = dump.model;
        let mut state = DeviceState::new(model);
        state.test_tone_type = 3;
        state.test_tone_param = 17;
        state.delay_unit = 2;
        state.eq_bypass.insert(model.inputs[model.inputs.len() - 1].into(), true);
        state.eq_bypass.insert(model.outputs[model.outputs.len() - 1].into(), true);
        state.geq_bypass.insert("In B".into(), true);
        state.limiters.insert(model.outputs[0].into(), LimiterState { knee: 6, ..LimiterState::default() });
        let stream = dump.encode(&state);
        let base = dump.switches_length();
        assert_eq!(base, if model == DeviceModel::DSP408 { 0x59C } else { 0x394 });
        assert_eq!([u16_at(&stream, base), u16_at(&stream, base + 2), u16_at(&stream, base + 4)], [3, 17, 2]);
        assert_eq!(u16_at(&stream, base + 6), 1 << (model.inputs.len() - 1), "EQ bypass of the inputs");
        assert_eq!(u16_at(&stream, base + 8), 1 << (model.outputs.len() - 1), "EQ bypass of the outputs");
        assert_eq!(u16_at(&stream, base + 10), 0x02, "GEQ bypass of In B");
        let decoded = dump.decode(&stream);
        assert!(decoded.rest_read);
        assert_eq!(decoded.state.to_json(), state.to_json());
    }
}

#[test]
fn times_of_older_versions_are_limited_knee_above_12_is_rejected() {
    let mut state = DeviceState::new(DeviceModel::DSP206);
    state.gates.insert("In A".into(), GateState { attack: 2998, release: 2998, hold: 2998, ..GateState::default() });
    state.compressors.insert("Out 1".into(), CompressorState { attack: 4999, release: 4999, ..CompressorState::default() });
    let decoded = DUMP206.decode(&DUMP206.encode(&state));
    assert!(decoded.problems.is_empty());
    assert_eq!(
        serde_json::to_value(&decoded.state.gates["In A"]).unwrap(),
        json!({"attack": 998, "release": 2998, "hold": 998, "threshold": 0})
    );
    let c = &decoded.state.compressors["Out 1"];
    assert_eq!([c.attack, c.release], [998, 2999]);

    state.limiters.insert("Out 2".into(), LimiterState { knee: 13, ..LimiterState::default() });
    let problems = DUMP206.decode(&DUMP206.encode(&state)).problems;
    assert_eq!(problems.len(), 1);
    assert!(problems[0].contains("limiter knee"));
}

#[test]
fn a_dump_that_ends_after_the_records_leaves_the_switches_off() {
    let stream = DUMP206.encode(&changed(DeviceModel::DSP206));
    let result = DUMP206.decode(&stream[..DUMP206.required_length()]);
    assert!(result.is_plausible());
    assert!(!result.switches_read);
    assert_eq!(result.state.mutes.values().copied().collect::<BTreeSet<_>>(), BTreeSet::from([false]));
    assert!(result.state.phase["Out 2"], "phase is part of the records");
}

#[test]
fn default_preset_dump() {
    let stream = default_stream();
    let result = DUMP408.decode(&stream);
    let s = &result.state;
    assert!(result.problems.is_empty(), "{:?}", result.problems);

    // header and channel names
    assert_eq!(s.current_preset, "Default Preset");
    assert_eq!(s.channel_names["In A"], "InA");
    assert_eq!(s.channel_names["Out 8"], "Out8");

    // gains, routing and crossing gains
    assert!(s.gains.values().all(|g| *g == 0.0));
    let routing: BTreeMap<String, i64> = (1..=8).map(|o| (format!("Out {o}"), 1 << ((o - 1) % 4))).collect();
    assert_eq!(s.matrix_routing, routing);
    assert!(s.matrix_gains.values().all(|g| *g == 0.0));

    // gate, EQ and filters
    assert_eq!(s.gates["In A"], GateState::default());
    assert_eq!(p::threshold_raw_to_db(s.gates["In A"].threshold), -90.0);
    assert_eq!(s.geq_bands["In D"], vec![0.0; 31]);
    // Default PEQ frequencies are spread over the whole band.
    assert_eq!(s.peq_bands["In A"].iter().map(|b| b.freq_raw).collect::<Vec<_>>(), [41, 71, 101, 140, 170, 200, 240, 270]);
    assert_eq!(
        s.peq_bands["Out 1"].iter().map(|b| b.freq_raw).collect::<Vec<_>>(),
        [31, 63, 95, 127, 158, 190, 222, 253, 285]
    );
    assert!(s.peq_bands["Out 1"].iter().all(|b| b.gain_db == 0.0 && b.q_raw == 35 && b.peq_type == 0));
    for ch in DeviceModel::DSP408.channels() {
        assert_eq!(s.hi_pass[ch], FilterState::new(0, 0), "{ch}");
        assert_eq!(s.lo_pass[ch], FilterState::new(300, 0), "{ch}");
    }

    // dynamics and links
    assert_eq!(s.compressors["Out 4"], CompressorState { ratio: 0, attack: 49, release: 499, knee: 0, threshold: 220 });
    assert_eq!(s.limiters["Out 8"], LimiterState { attack: 49, release: 499, knee: 0, threshold: 220 });
    assert_eq!(s.links["In C"], 4);
    assert_eq!(s.links["Out 8"], 0x80);

    // nothing muted, inverted, delayed or bypassed
    assert!(result.switches_read);
    assert!(s.mutes.values().all(|m| !m));
    assert!(s.phase.values().all(|m| !m));
    assert!(s.delays.values().all(|d| *d == 0));
    assert!(!s.peq_bands.values().flatten().any(|b| b.bypass));

    // re-encodes to the identical bytes
    assert_eq!(DUMP408.encode(s), stream);
}

#[test]
fn test_preset_dump_with_changes() {
    let stream = test_stream();
    let result = DUMP408.decode(&stream);
    let s = &result.state;
    assert!(result.problems.is_empty(), "{:?}", result.problems);
    assert_eq!(s.current_preset, "Test");
    assert_eq!(["Out 1", "Out 2", "Out 3"].map(|o| s.matrix_routing[o]), [3, 3, 3]);
    assert_eq!(s.gains["Out 8"], -60.0);
    assert_eq!(s.hi_pass["Out 8"], FilterState::new(55, 1)); // 70 Hz BW -6
    assert_eq!(s.lo_pass["Out 8"], FilterState::new(280, 20)); // 12.6 kHz LK -48
    assert_eq!(s.lo_pass["In A"].slope, 20);
    assert_eq!(s.peq_bands["Out 8"][0].peq_type, 7); // All Pass 1
    assert!((s.peq_bands["Out 8"][2].gain_db - 6.9).abs() < 1e-9);
    assert_eq!(s.peq_bands["In A"][7].gain_db, 12.0);
    assert_eq!(p::peq_freq_to_hz(s.hi_pass["Out 8"].freq_raw).round(), 70.0);
    assert_eq!(DUMP408.encode(s), stream);
}

#[test]
fn dsp206_round_trip_of_a_modified_state() {
    let mut state = DeviceState::new(DeviceModel::DSP206);
    state.current_preset = "Club".into();
    state.gains.insert("Out 6".into(), -12.5);
    state.matrix_routing.insert("Out 3".into(), 3);
    state.channel_names.insert("Out 1".into(), "Sub L".into());
    state.hi_pass.insert("Out 2".into(), FilterState::new(61, 8));
    state.compressors.insert("Out 2".into(), CompressorState { ratio: 9, attack: 9, release: 99, knee: 2, threshold: 170 });
    state.gates.insert("In B".into(), GateState { attack: 4, release: 199, hold: 49, threshold: 40 });
    state.geq_bands.get_mut("In A").unwrap()[10] = -3.5;

    let stream = DUMP206.encode(&state);
    assert_eq!(stream.len(), DUMP206.total_length());
    let decoded = DUMP206.decode(&stream);
    assert!(decoded.problems.is_empty());
    assert_eq!(decoded.state.to_json(), state.to_json());
}

#[test]
fn a_408_dump_decoded_with_the_206_layout_is_implausible() {
    assert!(!DUMP206.decode(&default_stream()).is_plausible());
}

#[test]
fn too_short() {
    let problems = DUMP206.decode(&[0; 100]).problems;
    assert_eq!(problems.len(), 1);
    assert!(problems[0].contains("too short"));
}

#[test]
fn split_produces_50_byte_chunks() {
    let chunks = ConfigDump::split(&default_stream());
    assert_eq!(chunks.len(), 29);
    assert_eq!(chunks.last().unwrap().len(), 48);
    assert_eq!(chunks[0], captured_config_chunks()[0]);
}
