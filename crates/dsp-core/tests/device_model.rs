//! Port of dart/packages/dsp_core/test/device_model_test.dart.

use std::collections::BTreeMap;

use dsp_core::{DeviceModel, DeviceState};

#[test]
fn detects_the_model_from_the_device_info_like_the_original_editor() {
    assert_eq!(DeviceModel::detect("DSP408 V0104P"), Some(DeviceModel::DSP408));
    assert_eq!(DeviceModel::detect("DSP206 V0102"), Some(DeviceModel::DSP206));
    assert_eq!(DeviceModel::detect("DSP204 V0104P"), Some(DeviceModel::DSP204));
    assert!(DeviceModel::DSP204.supports_version("DSP204 V0104P"));
    assert_eq!(DeviceModel::detect("DSP 206 V0104P"), None, "the answer must start with \"DSP206\"");
    assert_eq!(DeviceModel::detect("Something 408"), None);
    assert!(DeviceModel::DSP408.supports_version("DSP408 V0104P"));
    assert!(!DeviceModel::DSP408.supports_version("DSP408 V0201"));
    assert!(!DeviceModel::DSP206.supports_version("DSP408 V0104P"));
}

#[test]
fn channel_bytes() {
    let m206 = DeviceModel::DSP206;
    assert_eq!(m206.channels().collect::<Vec<_>>(), ["In A", "In B", "Out 1", "Out 2", "Out 3", "Out 4", "Out 5", "Out 6"]);
    assert_eq!(m206.channel_byte("Out 1"), Some(0x02));
    assert_eq!(m206.channel_byte("In C"), None);
    assert_eq!(m206.channel_for_byte(0x07), Some("Out 6"));
    assert_eq!(m206.channel_for_byte(0x08), None);
    assert_eq!(m206.meter_count(), 8);
    assert_eq!(m206.matrix_mask(), 0x03);

    let m408 = DeviceModel::DSP408;
    assert_eq!(m408.channel_byte("Out 1"), Some(0x04));
    assert_eq!(m408.channel_for_byte(0x0B), Some("Out 8"));
    assert_eq!(m408.meter_count(), 12);
    assert_eq!(m408.matrix_bit("In D"), 0x08);

    let m204 = DeviceModel::DSP204;
    assert_eq!(m204.channels().collect::<Vec<_>>(), ["In A", "In B", "Out 1", "Out 2", "Out 3", "Out 4"]);
    assert_eq!(m204.channel_byte("Out 1"), Some(0x02));
    assert_eq!(m204.channel_for_byte(0x05), Some("Out 4"));
    assert_eq!(m204.channel_for_byte(0x06), None);
}

#[test]
fn dsp204_meters_use_the_408_slots() {
    // 12 slots like the DSP 408: In A/B in slots 0 and 1, Out 1–4 from slot 4.
    let mut frame = vec![0x10, 0x02, 0x01, 0x00, 1 + 12 * 3 + 2, 0x40];
    for slot in 0..12u8 {
        let level = if slot == 1 || slot == 5 { 1.0f32 } else { 0.0 };
        let b = level.to_bits().to_be_bytes();
        frame.extend_from_slice(&[b[1], b[0], b[2]]);
    }
    frame.extend_from_slice(&[0x00, 0x03, 0x10, 0x03, 0x00]);
    let levels = dsp_core::protocol::parse_model_meters(&frame, &DeviceModel::DSP204);
    assert_eq!(levels, [0.0, 1.0, 0.0, 1.0, 0.0, 0.0]);
    let dynamics = dsp_core::protocol::parse_dynamics(&frame);
    assert!(dsp_core::protocol::limiter_active(dynamics, 0) && dsp_core::protocol::limiter_active(dynamics, 1));
    assert!(!dsp_core::protocol::limiter_active(dynamics, 2));
}

#[test]
fn lookup_by_id() {
    assert_eq!(DeviceModel::by_id(Some("dsp206")), DeviceModel::DSP206);
    assert_eq!(DeviceModel::by_id(Some("unknown")), DeviceModel::DSP408);
    assert!(DeviceModel::is_known_id("dsp206"));
    assert_eq!(DeviceModel::by_id(Some("dsp204")), DeviceModel::DSP204);
    assert!(!DeviceModel::is_known_id("auto"));
}

#[test]
fn default_state_follows_the_topology() {
    let s = DeviceState::new(DeviceModel::DSP206);
    assert_eq!(s.gains.keys().map(String::as_str).collect::<Vec<_>>(), DeviceModel::DSP206.channels().collect::<Vec<_>>());
    assert_eq!(s.geq_bands.keys().collect::<Vec<_>>(), ["In A", "In B"]);
    assert_eq!(s.compressors.len(), 6);
    assert_eq!(s.matrix_gains.len(), 12);
    let routing: BTreeMap<String, i64> =
        [("Out 1", 1), ("Out 2", 2), ("Out 3", 1), ("Out 4", 2), ("Out 5", 1), ("Out 6", 2)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
    assert_eq!(s.matrix_routing, routing);
    assert_eq!(s.peq_bands["In B"].len(), 8);
    assert_eq!(s.peq_bands["Out 6"].len(), 9);
}
