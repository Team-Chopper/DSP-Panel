//! Port of packages/dsp_core/test/protocol_test.dart.

use dsp_core::describe::describe;
use dsp_core::protocol::{self as p, Message, Protocol, calculate_checksum};
use dsp_core::{DeviceModel, Frame};

const P408: Protocol = Protocol::new(DeviceModel::DSP408);
const P206: Protocol = Protocol::new(DeviceModel::DSP206);

/// Gain frame as built by the original app, used as golden reference.
fn legacy_gain_frame(channel: u8, db: f64) -> Frame {
    let q = if db < -20.0 { (db * 2.0).round() / 2.0 } else { (db * 10.0).round() / 10.0 };
    let value = if q < -20.0 { ((q + 60.0) * 2.0).round() as i64 } else { (80.0 + (q + 20.0) * 10.0).round() as i64 };
    let data = [0x00, 0x01, 0x04, 0x34, channel, (value & 0xFF) as u8, ((value >> 8) & 0xFF) as u8];
    let checksum = calculate_checksum(&data);
    [&[0x10, 0x02][..], &data, &[0x10, 0x03, checksum]].concat()
}

/// Host → device frame for [payload] (command byte first).
fn frame(payload: &[u8]) -> Frame {
    let data = [&[0x00, 0x01, payload.len() as u8][..], payload].concat();
    [&[0x10, 0x02][..], &data, &[0x10, 0x03, calculate_checksum(&data)]].concat()
}

fn response(cmd: u8, payload: &[u8]) -> Frame {
    let data = [&[0x01, 0x00, payload.len() as u8 + 1, cmd][..], payload].concat();
    [&[0x10, 0x02][..], &data, &[0x10, 0x03, calculate_checksum(&data)]].concat()
}

/// Meter slot for [value]: upper three bytes of the float32, sent as
/// [bits 16–23, bits 24–31, bits 8–15].
fn slot(value: f32) -> [u8; 3] {
    let bits = value.to_bits();
    [((bits >> 16) & 0xFF) as u8, (bits >> 24) as u8, ((bits >> 8) & 0xFF) as u8]
}

#[test]
fn mute_matches_the_hard_coded_table_of_the_original_app() {
    assert_eq!(P408.build_mute_command("In A", true).unwrap(), [0x10, 0x02, 0x00, 0x01, 0x03, 0x35, 0x00, 0x01, 0x10, 0x03, 0x37]);
    assert_eq!(P408.build_mute_command("In D", false).unwrap(), [0x10, 0x02, 0x00, 0x01, 0x03, 0x35, 0x03, 0x00, 0x10, 0x03, 0x35]);
    assert_eq!(P408.build_mute_command("Out 5", true).unwrap(), [0x10, 0x02, 0x00, 0x01, 0x03, 0x35, 0x08, 0x01, 0x10, 0x03, 0x3F]);
    assert_eq!(P408.build_mute_command("Out 8", false).unwrap(), [0x10, 0x02, 0x00, 0x01, 0x03, 0x35, 0x0B, 0x00, 0x10, 0x03, 0x3D]);
}

#[test]
fn keepalive() {
    assert_eq!(p::build_keepalive_command(), [0x10, 0x02, 0x00, 0x01, 0x01, 0x40, 0x10, 0x03, 0x41]);
}

#[test]
fn captured_request_constants_equal_the_generic_frame_builder() {
    // Constants captured from the original app (the Dart TRacksProto tables).
    assert_eq!(p::handshake_command(), [0x10, 0x02, 0x00, 0x01, 0x01, 0x10, 0x10, 0x03, 0x11]);
    assert_eq!(p::request_lock_status(), [0x10, 0x02, 0x00, 0x01, 0x01, 0x2c, 0x10, 0x03, 0x2d]);
    assert_eq!(p::request_device_info(), [0x10, 0x02, 0x00, 0x01, 0x01, 0x13, 0x10, 0x03, 0x12]);
    assert_eq!(p::request_preset_slots(), [0x10, 0x02, 0x00, 0x01, 0x01, 0x22, 0x10, 0x03, 0x23]);
    assert_eq!(p::request_current_preset(), [0x10, 0x02, 0x00, 0x01, 0x01, 0x14, 0x10, 0x03, 0x15]);
    assert_eq!(p::build_heartbeat_command(), [0x10, 0x02, 0x00, 0x01, 0x01, 0x12, 0x10, 0x03, 0x13]);
    assert_eq!(p::request_preset_name(0), [0x10, 0x02, 0x00, 0x01, 0x02, 0x29, 0x00, 0x10, 0x03, 0x2b]);
    assert_eq!(p::request_preset_name(19), [0x10, 0x02, 0x00, 0x01, 0x02, 0x29, 0x13, 0x10, 0x03, 0x38]);
    assert_eq!(p::config_dump_command(0), [0x10, 0x02, 0x00, 0x01, 0x02, 0x27, 0x00, 0x10, 0x03, 0x25]);
    assert_eq!(p::config_dump_command(0x1c), [0x10, 0x02, 0x00, 0x01, 0x02, 0x27, 0x1c, 0x10, 0x03, 0x39]);
    assert_eq!(p::config_dump_command(0x10), [0x10, 0x02, 0x00, 0x01, 0x02, 0x27, 0x10, 0x10, 0x03, 0x35]);
}

#[test]
fn gain_over_the_full_range() {
    let channels: Vec<&str> = DeviceModel::DSP408.channels().collect();
    for tenth in (-600..=120).step_by(5) {
        let db = tenth as f64 / 10.0;
        for (ch, label) in channels.iter().enumerate() {
            assert_eq!(P408.build_gain_command(label, db).unwrap(), legacy_gain_frame(ch as u8, db), "{label} {db} dB");
        }
    }
}

#[test]
fn load_and_store_preset() {
    assert_eq!(p::build_load_preset_command(0), [0x10, 0x02, 0x00, 0x01, 0x02, 0x20, 0x01, 0x10, 0x03, 0x23]);
    let name = p::build_store_preset_name_command("Test");
    assert_eq!(name.len(), 14 + 9);
    assert_eq!(name[4], 0x0f);
    assert_eq!(&name[6..20], b"Test          ");
    assert_eq!(p::build_store_preset_name_command("Ümlaut")[6], 0x3F);
}

#[test]
fn dsp206_channel_bytes_outputs_follow_the_inputs_directly() {
    assert_eq!(P206.build_mute_command("In B", true).unwrap(), frame(&[0x35, 0x01, 0x01]));
    assert_eq!(P206.build_mute_command("Out 1", true).unwrap(), frame(&[0x35, 0x02, 0x01]));
    assert_eq!(P206.build_mute_command("Out 6", true).unwrap(), frame(&[0x35, 0x07, 0x01]));
    assert_eq!(P408.build_mute_command("Out 1", true).unwrap(), frame(&[0x35, 0x04, 0x01]));
}

#[test]
fn phase_delay_filters_peq_geq_matrix() {
    assert_eq!(P206.build_phase_command("Out 2", true).unwrap(), frame(&[0x36, 0x03, 0x01]));
    assert_eq!(P206.build_delay_command("Out 3", p::delay_ms_to_raw(10.0)).unwrap(), frame(&[0x38, 0x04, 0xC0, 0x03]));
    assert_eq!(P206.build_hi_pass_command("Out 1", 55, 1).unwrap(), frame(&[0x32, 0x02, 55, 0x00, 0x01]));
    assert_eq!(P206.build_lo_pass_command("In A", 300, 0).unwrap(), frame(&[0x31, 0x00, 0x2C, 0x01, 0x00]));
    assert_eq!(
        P206.build_peq_band_command("Out 6", 8, 3.0, 200, 16, 2, true).unwrap(),
        frame(&[0x33, 0x07, 0x08, 150, 0x00, 200, 0x00, 16, 2, 1])
    );
    assert_eq!(P206.build_geq_band_command("In B", 30, -12.0).unwrap(), frame(&[0x48, 0x01, 30, 0x00, 0x00]));
    assert_eq!(P206.build_matrix_routing_command("Out 4", 0x03).unwrap(), frame(&[0x3a, 0x05, 0x03]));
}

#[test]
fn matrix_crossing_gain() {
    // output, input index, value like the channel gain (280 = 0 dB)
    assert_eq!(P408.build_matrix_gain_command("Out 2", "In C", 0.0).unwrap(), frame(&[0x41, 0x05, 0x02, 0x18, 0x01]));
    assert_eq!(P408.build_matrix_gain_command("Out 8", "In A", -60.0).unwrap(), frame(&[0x41, 0x0B, 0x00, 0x00, 0x00]));
    assert_eq!(P206.build_matrix_gain_command("Out 1", "In B", -6.04).unwrap(), frame(&[0x41, 0x02, 0x01, 0xDC, 0x00]));
    // limited to 0 dB
    assert_eq!(
        P206.build_matrix_gain_command("Out 1", "In A", 6.0).unwrap(),
        P206.build_matrix_gain_command("Out 1", "In A", 0.0).unwrap()
    );
    assert!(P206.build_matrix_gain_command("Out 1", "In C", 0.0).is_err());
    assert!(P206.build_matrix_gain_command("In A", "In B", 0.0).is_err());
    assert_eq!(
        describe(&P408.build_matrix_gain_command("Out 2", "In C", -3.0).unwrap(), &DeviceModel::DSP408),
        "SET MATRIX GAIN Out 2 ← In C → -3.0 dB"
    );
}

#[test]
fn dynamics_and_channel_name() {
    assert_eq!(
        P206.build_compressor_command("Out 1", 9, 49, 499, 3, 170).unwrap(),
        frame(&[0x30, 0x02, 9, 0, 49, 0, 0xF3, 0x01, 3, 0, 170, 0])
    );
    assert_eq!(P206.build_gate_command("In A", 0, 99, 99, 0).unwrap(), frame(&[0x3E, 0x00, 0, 0, 99, 0, 99, 0, 0, 0]));
    assert_eq!(
        P206.build_limiter_command("Out 2", 49, 499, 6, 220).unwrap(),
        frame(&[0x3F, 0x03, 49, 0, 0xF3, 0x01, 6, 0, 220, 0])
    );
    assert_eq!(
        P206.build_channel_name_command("Out 1", "Sub L").unwrap(),
        frame(&[0x3D, 0x02, b'S', b'u', b'b', b' ', b'L', 0, 0, 0])
    );
}

#[test]
fn channels_the_model_does_not_have_are_rejected() {
    assert!(P206.build_gain_command("In C", 0.0).is_err());
    assert!(P206.build_gain_command("Out 7", 0.0).is_err());
    assert!(P206.build_geq_band_command("Out 1", 0, 0.0).is_err());
    assert!(P206.build_gate_command("Out 1", 0, 0, 0, 0).is_err());
    assert!(P408.build_geq_band_command("In A", 31, 0.0).is_err());
    assert!(P408.build_matrix_routing_command("In A", 1).is_err());
    assert!(P408.build_peq_band_command("In A", 8, 0.0, 0, 0, 0, false).is_err());
}

#[test]
fn channel_gain_decoding_inverts_encoding_across_both_ranges() {
    for tenth in -600..=120 {
        let q = p::quantize_gain(tenth as f64 / 10.0);
        assert!((p::gain_value_to_db(p::gain_db_to_value(q)) - q).abs() < 1e-9, "{q} dB");
    }
    assert_eq!(p::gain_value_to_db(0), -60.0);
    assert_eq!(p::gain_value_to_db(60), -30.0);
    assert_eq!(p::gain_value_to_db(80), -20.0);
    assert_eq!(p::gain_value_to_db(280), 0.0);
}

#[test]
fn frequency_encoding() {
    assert!((p::peq_freq_to_hz(0) - 19.69).abs() < 0.01);
    assert!((p::peq_freq_to_hz(140) - 500.0).abs() < 1e-9);
    assert!((p::peq_freq_to_hz(170) - 1000.0).abs() < 1e-9);
    assert!((p::peq_freq_to_hz(300) - 20159.0).abs() < 1.0);
    assert_eq!(p::peq_hz_to_freq(1000.0), 170);
    assert_eq!(p::peq_hz_to_freq(5.0), 0);
    assert_eq!(p::peq_hz_to_freq(50000.0), 300);
    for raw in 0..=300 {
        assert_eq!(p::peq_hz_to_freq(p::peq_freq_to_hz(raw)), raw);
    }
}

#[test]
fn q_encoding_and_display_table() {
    assert!((p::peq_raw_to_q(16) - 1.0).abs() < 1e-9);
    assert!((p::peq_raw_to_q(0) - 0.397).abs() < 0.001);
    assert!((p::peq_raw_to_q(100) - 128.0).abs() < 1e-9);
    assert_eq!(p::peq_q_to_raw(1.0), 16);
    for raw in 0..=p::Q_RAW_MAX {
        assert_eq!(p::peq_q_to_raw(p::peq_raw_to_q(raw)), raw);
        // The table rounds the formula (to 2–3 significant digits).
        let shown: f64 = p::format_q(raw).parse().unwrap();
        assert!((shown - p::peq_raw_to_q(raw)).abs() / p::peq_raw_to_q(raw) < 0.045, "raw {raw}");
    }
    let shown: Vec<&str> = [0, 10, 16, 35, 56, 100].into_iter().map(p::format_q).collect();
    assert_eq!(shown, ["0.40", "0.71", "1.00", "3.0", "10.1", "128"]);
    // Shelves and PEQ low/high-pass types are limited to Q 3.
    let limits: Vec<i64> = (0..9).map(p::max_q_raw_for_type).collect();
    assert_eq!(limits, [100, 35, 35, 35, 35, 35, 35, 100, 100]);
    assert!((p::peq_raw_to_q(p::Q_RAW_TYPE_RESET) - 0.71).abs() < 0.01);
}

#[test]
fn delay_threshold_times() {
    assert_eq!(p::delay_ms_to_raw(10.0), 960);
    assert_eq!(p::delay_raw_to_ms(960), 10.0);
    assert_eq!(p::delay_ms_to_raw(5000.0), p::DELAY_RAW_MAX);
    assert_eq!(p::threshold_db_to_raw(0.0), 180);
    assert_eq!(p::threshold_db_to_raw(20.0), 220);
    assert_eq!(p::threshold_raw_to_db(0), -90.0);
    assert_eq!(p::time_ms_to_raw(50), 49);
    assert_eq!(p::time_raw_to_ms(499), 500);
}

#[test]
fn slopes() {
    assert_eq!(p::CROSSOVER_SLOPE_NAMES.len(), 21);
    assert_eq!(p::CROSSOVER_SLOPE_NAMES[0], "Bypass");
    assert_eq!(p::CROSSOVER_SLOPE_NAMES[p::DEFAULT_SLOPE as usize], "BW -24");
    assert_eq!(p::slope_db_per_octave(0), 0);
    assert_eq!(p::slope_db_per_octave(5), 12);
    assert_eq!(p::slope_db_per_octave(20), 48);
}

#[test]
fn decode_preset_name() {
    let msg = P408.decode(&response(0x29, &[&[0x03][..], b"Live Show     "].concat()));
    assert_eq!(msg, Some(Message::Preset { index: 3, name: "Live Show".into() }));
}

#[test]
fn device_info_keeps_the_first_character() {
    let msg = P408.decode(&response(0x13, b"DSP408 V0104P"));
    assert_eq!(msg, Some(Message::DeviceInfo { name: "DSP408 V0104P".into() }));
}

#[test]
fn meter_slots_are_truncated_float32_values() {
    // 1.0 = 0x3F800000: the old float16 reading of these bytes was 1.875.
    assert_eq!(slot(1.0), [0x80, 0x3F, 0x00]);
    assert_eq!(p::decode_meter_value(0x80, 0x3F, 0x00), 1.0);
    assert_eq!(p::decode_meter_value(0xFF, 0xFF, 0x00), 0.0, "NaN while recalculating");
    let v = p::decode_meter_value(0x00, 0x38, 0x00); // the old "0.5 noise floor"
    assert!((p::level_to_db(v) + 90.3).abs() < 0.1);
    assert_eq!(p::level_to_db(0.0), p::SILENCE_DB);
    assert!((p::level_to_db(0.5) + 6.02).abs() < 0.01);
}

fn meters(n: usize, value: f32, dynamics: Option<u16>) -> Frame {
    let mut payload: Vec<u8> = (0..n).flat_map(|_| slot(value)).collect();
    if let Some(d) = dynamics {
        payload.extend_from_slice(&[(d & 0xFF) as u8, (d >> 8) as u8]);
    }
    response(0x40, &payload)
}

#[test]
fn meter_levels_per_model_and_the_dynamics_word() {
    let Some(Message::Keepalive { levels, dynamics }) = P408.decode(&meters(12, 1.0, Some(0x0201))) else {
        panic!("not a keepalive");
    };
    assert_eq!(levels, vec![1.0; 12]);
    assert_eq!(dynamics, 0x0201);
    assert!(p::compressor_active(dynamics, 0));
    assert!(!p::limiter_active(dynamics, 0));
    assert!(p::limiter_active(dynamics, 1));
    let Some(Message::Keepalive { levels, .. }) = P206.decode(&meters(8, 1.0, Some(0))) else { panic!() };
    assert_eq!(levels, vec![1.0; 8]);
    // A frame without the status word
    let Some(Message::Keepalive { dynamics, .. }) = P408.decode(&meters(12, 0.5, None)) else { panic!() };
    assert_eq!(dynamics, 0);
}

#[test]
fn dsp206_meters_in_the_12_slot_layout_outputs_start_at_slot_4() {
    let mut payload: Vec<u8> = (0..12).flat_map(|i| slot(i as f32 + 1.0)).collect();
    payload.extend_from_slice(&[0, 0]);
    let frame = response(0x40, &payload);
    let Some(Message::Keepalive { levels, .. }) = P206.decode(&frame) else { panic!() };
    assert_eq!(levels, [1.0, 2.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]);
    let Some(Message::Keepalive { levels, .. }) = P408.decode(&frame) else { panic!() };
    assert_eq!(levels, (1..=12).map(|i| i as f64).collect::<Vec<_>>());
}

#[test]
fn config_chunk_whose_payload_contains_10_03() {
    let msg = P408.decode(&response(0x24, &[0x05, 0x10, 0x03, 0x77]));
    assert_eq!(msg, Some(Message::ChannelConfig { sub_index: 5, chunk: vec![0x10, 0x03, 0x77] }));
}

#[test]
fn requests_host_to_device_are_not_decoded() {
    assert_eq!(P408.decode(&p::handshake_command()), None);
}

#[test]
fn lock_status_preset_slots_current_preset_and_password_result() {
    // Captured answer of a DSP 408: PTS off, limit 9999, count 0, not locked.
    let lock = P408.decode(&response(0x2c, &[0x00, 0x27, 0x0f, 0x00, 0x00, 0x00])).unwrap();
    assert_eq!(lock, Message::LockStatus { pts_on: false, limit: 9999, count: 0, locked: false });
    assert!(!lock.pts_expired());
    let expired = P408.decode(&response(0x2c, &[0x01, 0x00, 0x0A, 0x00, 0x0A, 0x01])).unwrap();
    assert!(expired.pts_expired());
    assert!(matches!(expired, Message::LockStatus { locked: true, .. }));

    let mut slots = vec![0xff, 0xff, 0xff, 0xff];
    slots.extend([0; 14]);
    slots.extend([0xff, 0xff]);
    let Some(Message::PresetSlots { used }) = P408.decode(&response(0x22, &slots)) else { panic!() };
    assert_eq!(used.iter().filter(|u| **u).count(), 6);
    assert_eq!(used.len(), 20);
    assert_eq!(P408.decode(&response(0x14, &[0x14])), Some(Message::CurrentPreset { number: 20 }));
    assert_eq!(P408.decode(&response(0x2d, &[0x00, 0x01])), Some(Message::PasswordResult { mode: 0, ok: true }));
}

#[test]
fn answers_of_another_device_id_are_ignored() {
    let from = |id: u8| {
        let data = [id, 0x00, 0x02, 0x14, 0x03];
        [&[0x10, 0x02][..], &data, &[0x10, 0x03, calculate_checksum(&data)]].concat()
    };
    assert!(matches!(P408.decode(&from(1)), Some(Message::CurrentPreset { .. })));
    assert_eq!(P408.decode(&from(7)), None);
    assert!(matches!(Protocol::with_id(DeviceModel::DSP408, 7).decode(&from(7)), Some(Message::CurrentPreset { .. })));
}

#[test]
fn device_id_addressing_keeps_the_checksum_valid() {
    let f = p::with_device_id(&p::build_keepalive_command(), 0x21);
    assert_eq!(f, [0x10, 0x02, 0x00, 0x21, 0x01, 0x40, 0x10, 0x03, 0x01 ^ 0x21 ^ 0x01 ^ 0x40]);
    // XOR over the whole frame from 10 02 to 10 03 gives the same checksum.
    assert_eq!(f[..f.len() - 1].iter().fold(0u8, |x, b| x ^ b), f[f.len() - 1]);
    assert_eq!(p::with_device_id(&p::build_keepalive_command(), 1), p::build_keepalive_command());
}

#[test]
fn copy_bypass_delay_unit_password_disconnect() {
    assert_eq!(P408.build_copy_channel_command("Out 1", "Out 3").unwrap(), frame(&[0x2A, 0x04, 0x06]));
    assert_eq!(P206.build_copy_channel_command("In A", "In B").unwrap(), frame(&[0x2A, 0x00, 0x01]));
    assert!(P408.build_copy_channel_command("In A", "Out 1").is_err());
    assert_eq!(P206.build_eq_bypass_command("Out 6", true).unwrap(), frame(&[0x3C, 0x07, 0x01]));
    assert_eq!(P408.build_geq_bypass_command("In D", false).unwrap(), frame(&[0x49, 0x03, 0x00]));
    assert!(P408.build_geq_bypass_command("Out 1", true).is_err());
    assert_eq!(p::build_delay_unit_command(2), frame(&[0x15, 0x02]));
    assert_eq!(p::build_password_command(1, "ab12"), frame(&[0x2D, 0x01, b'A', b'B', b'1', b'2']));
    assert_eq!(p::build_disconnect_command(), frame(&[0x11]));
    let m408 = DeviceModel::DSP408;
    assert_eq!(describe(&P408.build_copy_channel_command("Out 1", "Out 3").unwrap(), &m408), "COPY CHANNEL Out 1 → Out 3");
    assert_eq!(
        describe(&P206.build_eq_bypass_command("Out 6", true).unwrap(), &DeviceModel::DSP206),
        "SET EQ BYPASS Out 6 → BYPASS"
    );
    assert_eq!(describe(&p::build_load_preset_command(19), &m408), "LOAD PRESET U20");
}

#[test]
fn packet_describer() {
    let m408 = DeviceModel::DSP408;
    assert_eq!(describe(&P408.build_gain_command("Out 3", -6.0).unwrap(), &m408), "SET GAIN Out 3 → -6.0 dB");
    assert_eq!(
        describe(&P206.build_gain_command("Out 3", -6.0).unwrap(), &DeviceModel::DSP206),
        "SET GAIN Out 3 → -6.0 dB"
    );
    assert_eq!(describe(&P408.build_hi_pass_command("In A", 100, 0).unwrap(), &m408), "SET HPF In A → BYPASS");
    assert_eq!(describe(&P408.build_lo_pass_command("In A", 100, 9).unwrap(), &m408), "SET LPF In A → 198 Hz BL -24");
    assert_eq!(
        describe(&P408.build_matrix_routing_command("Out 1", 0x05).unwrap(), &m408),
        "SET MATRIX Out 1 ← In A + In C"
    );
    assert_eq!(describe(&P408.build_delay_command("Out 2", 960).unwrap(), &m408), "SET DELAY Out 2 → 10.00 ms");
}
