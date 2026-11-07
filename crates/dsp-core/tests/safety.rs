//! Live-safety helpers: max level per channel and the ops allowed while the
//! DSP test generator runs.

use std::collections::BTreeMap;

use dsp_core::api::DspInfo;
use dsp_core::ops::{self, Op, allowed_during_test, capped_gain, group_cap};
use dsp_core::{DeviceModel, DeviceState};

#[test]
fn a_gain_never_goes_above_max_of_cap_and_current() {
    assert_eq!(capped_gain(3.0, -20.0, None), 3.0);
    assert_eq!(capped_gain(3.0, -20.0, Some(-6.0)), -6.0);
    assert_eq!(capped_gain(-10.0, -20.0, Some(-6.0)), -10.0);
    // Already above the cap: only down.
    assert_eq!(capped_gain(6.0, 2.0, Some(-6.0)), 2.0);
    assert_eq!(capped_gain(1.0, 2.0, Some(-6.0)), 1.0);
}

#[test]
fn linked_channels_share_the_lowest_cap_of_their_group() {
    let mut state = DeviceState::new(DeviceModel::DSP206);
    let caps: BTreeMap<String, f64> = [("Out 2".to_string(), -12.0), ("Out 1".to_string(), -3.0)].into();
    assert_eq!(group_cap(&state, &caps, "Out 1"), Some(-3.0));
    assert_eq!(group_cap(&state, &caps, "Out 3"), None);
    ops::apply(&mut state, &Op::link("Out 1", 0b11));
    assert_eq!(group_cap(&state, &caps, "Out 1"), Some(-12.0));
    assert_eq!(group_cap(&state, &caps, "Out 2"), Some(-12.0));
}

#[test]
fn only_mutes_and_test_controls_while_the_generator_runs() {
    assert!(allowed_during_test(&Op::mute("Out 1", true)));
    assert!(allowed_during_test(&Op::test_tone(0, 0)));
    assert!(allowed_during_test(&Op::link("In A", 1)));
    assert!(!allowed_during_test(&Op::gain("Out 1", -10.0)));
    assert!(!allowed_during_test(&Op::delay("Out 1", 0)));
}

#[test]
fn dsp_info_without_the_new_fields_still_parses() {
    let json = serde_json::json!({"id": "a", "name": "A", "host": "h", "port": 1, "enabled": true});
    let info: DspInfo = serde_json::from_value(json).unwrap();
    assert!(info.gain_caps.is_empty());
    assert_eq!(info.test_level, None);
    let round = serde_json::to_value(DspInfo { test_level: Some(-50.0), ..info }).unwrap();
    assert_eq!(round["testLevel"], -50.0);
    assert_eq!(round["gainCaps"], serde_json::json!({}));
}
