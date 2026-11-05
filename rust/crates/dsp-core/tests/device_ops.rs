//! Port of packages/dsp_core/test/device_ops_test.dart.

use dsp_core::ops::{self, Op};
use dsp_core::protocol::{self as p, Protocol};
use dsp_core::{CompressorState, DeviceModel, DeviceState, FilterState, GateState, LimiterState, PeqBand};
use serde_json::{Value, json};

const M408: DeviceModel = DeviceModel::DSP408;
const M206: DeviceModel = DeviceModel::DSP206;

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// Validate an op the way the server does: as JSON from the WebSocket.
fn validate(op: &Op, model: &DeviceModel) -> Option<String> {
    let wire: Value = serde_json::from_str(&op.to_json().to_string()).unwrap();
    ops::validate(&wire, model)
}

fn with_fields(base: Value, extra: impl serde::Serialize) -> Value {
    let mut v = base;
    if let (Value::Object(m), Value::Object(e)) = (&mut v, serde_json::to_value(extra).unwrap()) {
        m.extend(e);
    }
    v
}

#[test]
fn accepts_well_formed_ops() {
    let list = [
        Op::gain("In A", -3.5),
        Op::mute("Out 8", true),
        Op::phase("In B", true),
        Op::delay("Out 2", 960),
        Op::matrix_route("Out 2", 0x0F),
        Op::matrix_gain("Out 1", "In D", -6.5),
        Op::geq_band("In D", 30, 4.2),
        Op::geq_reset("In A"),
        Op::peq_band("Out 1", 8, PeqBand { gain_db: 3.0, ..PeqBand::default() }),
        Op::hi_pass("In A", FilterState::new(61, 8)),
        Op::lo_pass("Out 3", FilterState::new(280, 20)),
        Op::gate("In C", GateState::default()),
        Op::compressor("Out 4", CompressorState { ratio: 9, ..CompressorState::default() }),
        Op::limiter("Out 5", LimiterState { threshold: 200, ..LimiterState::default() }),
        Op::channel_name("Out 1", Some("Sub L")),
        Op::channel_name("Out 1", None),
        Op::eq_bypass("Out 3", true),
        Op::geq_bypass("In D", true),
        Op::delay_unit(2),
        Op::limiter("Out 6", LimiterState { knee: 12, ..LimiterState::default() }),
    ];
    for op in list {
        assert_eq!(validate(&op, &M408), None, "{op:?}");
        // and the JSON form parses back to the same op
        assert_eq!(Op::parse(&op.to_json()).unwrap(), op);
    }
}

#[test]
fn rejects_malformed_ops() {
    let peq = serde_json::to_value(PeqBand::default()).unwrap();
    let bad = [
        Value::Null,
        json!("gain"),
        json!({"op": "gain", "ch": "In E", "db": 0}),
        json!({"op": "gain", "ch": "In A", "db": "loud"}),
        json!({"op": "mute", "ch": "In A"}),
        json!({"op": "delay", "ch": "In A", "raw": -1}),
        json!({"op": "delay", "ch": "In A", "raw": p::DELAY_RAW_MAX + 1}),
        json!({"op": "delay", "ch": "In A", "raw": 1.5}),
        json!({"op": "matrixRoute", "out": "In A", "mask": 1}),
        json!({"op": "matrixRoute", "out": "Out 1", "mask": 16}),
        json!({"op": "geqBand", "ch": "Out 1", "band": 0, "db": 0}),
        with_fields(json!({"op": "peqBand", "ch": "In A", "band": 8}), &peq),
        with_fields(json!({"op": "peqBand", "ch": "In A", "band": 0}), PeqBand { freq_raw: 301, ..PeqBand::default() }),
        json!({"op": "hiPass", "ch": "In A", "freqRaw": 100, "slope": 21}),
        with_fields(json!({"op": "gate", "ch": "Out 1"}), GateState::default()),
        with_fields(json!({"op": "compressor", "ch": "In A"}), CompressorState::default()),
        with_fields(json!({"op": "compressor", "ch": "Out 1"}), CompressorState { ratio: 16, ..CompressorState::default() }),
        json!({"op": "limiter", "ch": "Out 1", "attack": 0, "release": 0, "threshold": 221}),
        with_fields(json!({"op": "limiter", "ch": "Out 1"}), LimiterState { knee: 13, ..LimiterState::default() }),
        json!({"op": "limiter", "ch": "Out 1", "attack": 49, "release": 499, "threshold": 220}),
        with_fields(json!({"op": "gate", "ch": "In A"}), GateState { attack: 999, ..GateState::default() }),
        with_fields(json!({"op": "gate", "ch": "In A"}), GateState { release: 8, ..GateState::default() }),
        with_fields(json!({"op": "gate", "ch": "In A"}), GateState { hold: 999, ..GateState::default() }),
        with_fields(json!({"op": "compressor", "ch": "Out 1"}), CompressorState { release: 3000, ..CompressorState::default() }),
        json!({"op": "geqBypass", "ch": "Out 1", "on": true}),
        json!({"op": "delayUnit", "unit": 3}),
        json!({"op": "channelName", "ch": "In A", "name": "too long name"}),
        json!({"op": "channelName", "ch": "In A", "name": "Bässe"}),
        json!({"op": "matrixGain", "out": "In A", "in": "In A", "db": 0}),
        json!({"op": "matrixGain", "out": "Out 1", "in": "Out 2", "db": 0}),
        json!({"op": "matrixGain", "out": "Out 1", "in": "In A", "db": "quiet"}),
        json!({"op": "alias", "ch": "In A", "alias": "x"}),
        json!({"op": "selfDestruct"}),
    ];
    for op in bad {
        assert!(ops::validate(&op, &M408).is_some(), "{op}");
    }
}

#[test]
fn channels_are_validated_against_the_model() {
    assert!(validate(&Op::gain("In C", 0.0), &M206).is_some());
    assert!(validate(&Op::gain("Out 7", 0.0), &M206).is_some());
    assert!(validate(&Op::matrix_route("Out 1", 0x04), &M206).is_some());
    assert!(validate(&Op::matrix_route("Out 1", 0x03), &M206).is_none());
    assert!(validate(&Op::gain("Out 6", 0.0), &M206).is_none());
}

#[test]
fn gain_is_clamped_quantized_and_sent_once() {
    let mut state = DeviceState::default();
    let op = Op::gain("Out 2", -33.3);
    ops::apply(&mut state, &op);
    assert_eq!(state.gains["Out 2"], -33.5);
    let cmds = ops::commands(&state, &op).unwrap();
    assert_eq!(cmds.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["gain:Out 2"]);
    assert_eq!(cmds[0].1.len(), 1);
    ops::apply(&mut state, &Op::gain("Out 2", 50.0));
    assert_eq!(state.gains["Out 2"], 12.0);
}

#[test]
fn commands_use_the_channel_bytes_of_the_states_model() {
    let mut state = DeviceState::new(M206);
    let op = Op::phase("Out 1", true);
    ops::apply(&mut state, &op);
    let cmds = ops::commands(&state, &op).unwrap();
    assert_eq!(cmds[0], ("phase:Out 1".into(), vec![Protocol::new(M206).build_phase_command("Out 1", true).unwrap()]));
}

#[test]
fn geq_reset_produces_one_coalescing_key_per_band() {
    let mut state = DeviceState::default();
    ops::apply(&mut state, &Op::geq_band("In A", 3, 5.0));
    let op = Op::geq_reset("In A");
    ops::apply(&mut state, &op);
    assert_eq!(state.geq_bands["In A"], vec![0.0; 31]);
    assert_eq!(ops::commands(&state, &op).unwrap().len(), 31);
}

#[test]
fn filters_send_their_slope_slope_0_is_bypass() {
    let mut state = DeviceState::default();
    let op = Op::hi_pass("In A", FilterState::new(61, 0));
    ops::apply(&mut state, &op);
    assert!(!state.hi_pass["In A"].enabled());
    let cmds = ops::commands(&state, &op).unwrap();
    assert_eq!(cmds[0], ("hpf:In A".into(), vec![Protocol::default().build_hi_pass_command("In A", 61, 0).unwrap()]));
}

#[test]
fn matrix_gain_is_clamped_quantized_and_sent_per_crossing() {
    let mut state = DeviceState::new(M206);
    let op = Op::matrix_gain("Out 3", "In B", -33.3);
    assert_eq!(validate(&op, &M206), None);
    ops::apply(&mut state, &op);
    assert_eq!(state.matrix_gains["Out 3:In B"], -33.5);
    let expected = Protocol::new(M206).build_matrix_gain_command("Out 3", "In B", -33.5).unwrap();
    assert_eq!(ops::commands(&state, &op).unwrap(), vec![("mixgain:Out 3:In B".to_string(), vec![expected])]);
    ops::apply(&mut state, &Op::matrix_gain("Out 3", "In B", 4.0));
    assert_eq!(state.matrix_gains["Out 3:In B"], 0.0);
    assert!(validate(&Op::matrix_gain("Out 1", "In C", 0.0), &M206).is_some());
    // Not shared with linked channels
    let mut linked = DeviceState::new(M206);
    linked.links.insert("Out 1".into(), 3);
    assert_eq!(ops::expand_linked(&linked, &Op::matrix_gain("Out 1", "In A", -3.0)).len(), 1);
}

#[test]
fn limiter_sends_its_knee() {
    let mut state = DeviceState::default();
    let op = Op::limiter("Out 1", LimiterState { threshold: 200, knee: 7, ..LimiterState::default() });
    ops::apply(&mut state, &op);
    assert_eq!(state.limiters["Out 1"].knee, 7);
    let expected = Protocol::default().build_limiter_command("Out 1", 49, 499, 7, 200).unwrap();
    assert_eq!(ops::commands(&state, &op).unwrap()[0].1, vec![expected]);
    // Older presets stored the knee as "unknown".
    let old = LimiterState::from_json(&json!({"attack": 1, "release": 9, "unknown": 3, "threshold": 0})).unwrap();
    assert_eq!(old.knee, 3);
}

#[test]
fn values_of_older_versions_are_limited_to_the_original_ranges() {
    let gate = ops::clamp_gate(&GateState { attack: 2998, release: 0, hold: 2000, threshold: 200 });
    assert_eq!(gate, GateState { attack: 998, release: 9, hold: 998, threshold: 180 });
    let comp = ops::clamp_compressor(&CompressorState { attack: 4999, release: 4999, knee: 40, ..CompressorState::default() });
    assert_eq!([comp.attack, comp.release, comp.knee], [998, 2999, 12]);
}

#[test]
fn eq_bypass_geq_bypass_and_delay_unit() {
    let mut state = DeviceState::new(M206);
    for (op, key, frame) in [
        (Op::eq_bypass("Out 2", true), "eqBypass:Out 2", p::build_frame(0x3C, &[0x03, 1])),
        (Op::geq_bypass("In B", true), "geqBypass:In B", p::build_frame(0x49, &[0x01, 1])),
        (Op::delay_unit(1), "delayUnit", p::build_frame(0x15, &[1])),
    ] {
        assert_eq!(validate(&op, &M206), None);
        ops::apply(&mut state, &op);
        assert_eq!(ops::commands(&state, &op).unwrap(), vec![(key.to_string(), vec![frame])]);
    }
    assert!(state.eq_bypass["Out 2"] && state.geq_bypass["In B"]);
    assert_eq!(state.delay_unit, 1);
}

#[test]
fn channel_name_empty_restores_the_default_device_name() {
    let mut state = DeviceState::default();
    let op = Op::channel_name("In A", Some("Vox"));
    ops::apply(&mut state, &op);
    assert_eq!(state.channel_names["In A"], "Vox");
    let expected = Protocol::default().build_channel_name_command("In A", "Vox").unwrap();
    assert_eq!(ops::commands(&state, &op).unwrap()[0].1, vec![expected]);
    ops::apply(&mut state, &Op::channel_name("In A", Some("  ")));
    assert_eq!(state.channel_names["In A"], "InA");
}

fn apply_all(state: &mut DeviceState, list: Vec<Op>) {
    for op in list {
        assert_eq!(validate(&op, &state.model), None, "{op:?}");
        ops::apply(state, &op);
    }
}

#[test]
fn link_op_mask_must_stay_in_the_group_and_contain_the_channel() {
    assert_eq!(validate(&Op::link("Out 1", 0x03), &M408), None);
    assert!(validate(&Op::link("Out 1", 0x02), &M408).is_some(), "own bit missing");
    assert_eq!(validate(&Op::link("Out 2", 0x00), &M408), None, "slave stored without bits");
    assert!(validate(&Op::link("Out 1", 0x100), &M408).is_some());
    assert!(validate(&Op::link("In A", 0x10), &M408).is_some(), "only 4 inputs");
    assert_eq!(validate(&Op::link("Out 6", 0x30), &M206), None);
}

#[test]
fn master_slave_as_read_from_the_device() {
    // DSP 206 dump: Out 1 lists Out 2, Out 2 only itself.
    let mut state = DeviceState::new(M206);
    state.links.insert("Out 1".into(), 0x03);
    state.links.insert("Out 2".into(), 0x02);
    assert_eq!(ops::master_of(&state, "Out 2").as_deref(), Some("Out 1"));
    assert_eq!(ops::master_of(&state, "Out 1"), None);
    assert_eq!(ops::slaves_of(&state, "Out 1"), strings(&["Out 2"]));
    // A change of the master moves its slaves.
    assert_eq!(ops::expand_linked(&state, &Op::gain("Out 1", -3.0)), [Op::gain("Out 1", -3.0), Op::gain("Out 2", -3.0)]);
    // A slave whose own mask holds its bit is its own group …
    assert_eq!(ops::edit_group_of(&state, "Out 2"), strings(&["Out 2"]));
    assert_eq!(ops::expand_linked(&state, &Op::gain("Out 2", -3.0)), [Op::gain("Out 2", -3.0)]);
    // … one without it belongs to the first lower channel that lists it.
    state.links.insert("Out 2".into(), 0);
    assert_eq!(ops::edit_group_of(&state, "Out 2"), strings(&["Out 1", "Out 2"]));
    assert_eq!(ops::expand_linked(&state, &Op::gain("Out 2", -3.0)), [Op::gain("Out 2", -3.0), Op::gain("Out 1", -3.0)]);
    assert_eq!(ops::edit_group_of(&state, "Out 3"), strings(&["Out 3"]));
}

#[test]
fn link_change_new_slaves_mask_and_masks_of_older_versions_to_reset() {
    let mut state = DeviceState::new(M408);
    state.links.insert("Out 1".into(), 0x07);
    state.links.insert("Out 2".into(), 0x07); // symmetric mask of an older version
    state.links.insert("Out 3".into(), 0x04);
    let change = ops::link_change(&state, "Out 1", &strings(&["Out 3", "Out 5"])).unwrap();
    assert_eq!(change.mask, 0x15);
    assert_eq!(change.added, strings(&["Out 5"]));
    assert_eq!(change.reset, strings(&["Out 2"]));
}

#[test]
fn copy_parts_between_models_sends_only_what_differs() {
    let mut src = DeviceState::new(M408);
    ops::apply(&mut src, &Op::gain("Out 1", -4.0));
    ops::apply(&mut src, &Op::delay("Out 1", 960));
    ops::apply(&mut src, &Op::matrix_route("Out 1", 0b1010)); // In B + In D
    ops::apply(&mut src, &Op::matrix_gain("Out 1", "In B", -6.0));
    ops::apply(&mut src, &Op::peq_band("Out 1", 8, PeqBand { gain_db: 2.0, ..PeqBand::default() }));
    let dst = DeviceState::new(M206);
    let list = ops::copy_parts(&src, "Out 1", &dst, "Out 3", &strings(&["level", "routing", "eq"])).unwrap();
    assert_eq!(
        list,
        [
            Op::gain("Out 3", -4.0),
            Op::delay("Out 3", 960),
            Op::peq_band("Out 3", 8, PeqBand { gain_db: 2.0, ..PeqBand::default() }),
            // The 206 has no In D; Out 3 is fed from In A by default.
            Op::matrix_route("Out 3", 0b10),
            Op::matrix_gain("Out 3", "In B", -6.0),
        ]
    );
    assert!(ops::copy_parts(&src, "Out 1", &dst, "In A", &strings(&["level"])).is_err());
    assert!(ops::copy_parts(&src, "Out 1", &dst, "Out 2", &strings(&["mute"])).is_err());
    assert!(ops::copy_parts(&src, "Out 2", &dst, "Out 2", &strings(&["level", "dynamics"])).unwrap().is_empty());
}

#[test]
fn link_change_resets_released_slaves_with_mask_zero() {
    // Slaves with link word 0, as other control software leaves them: the DSP
    // ignores changes on such a channel once it is no longer in a group.
    let mut state = DeviceState::new(M408);
    state.links.insert("Out 1".into(), 0x07);
    state.links.insert("Out 2".into(), 0);
    state.links.insert("Out 3".into(), 0);
    assert_eq!(ops::link_group_of(&state, "Out 3"), strings(&["Out 1", "Out 2", "Out 3"]));
    let change = ops::link_change(&state, "Out 1", &strings(&["Out 2"])).unwrap();
    assert_eq!(change.mask, 0x03);
    assert_eq!(change.reset, strings(&["Out 3"]));
    let list = ops::unlink(&state, "Out 1").unwrap();
    assert_eq!(list, [Op::link("Out 1", 0x01), Op::link("Out 2", 0x02), Op::link("Out 3", 0x04)]);
}

#[test]
fn symmetric_masks_form_one_group_with_the_first_channel_as_master() {
    let mut state = DeviceState::new(M408);
    for o in ["Out 1", "Out 2", "Out 3"] {
        state.links.insert(o.into(), 7);
    }
    assert_eq!(ops::link_group_of(&state, "Out 3"), strings(&["Out 1", "Out 2", "Out 3"]));
    assert_eq!(ops::master_of(&state, "Out 2").as_deref(), Some("Out 1"));
    assert_eq!(ops::linked_with(&state, "Out 2"), strings(&["Out 1", "Out 3"]));
}

#[test]
fn link_slaves_master_keeps_the_mask_slaves_only_their_own_bit() {
    let mut state = DeviceState::new(M408);
    let list = ops::link_slaves(&state, "Out 1", &strings(&["Out 2"])).unwrap();
    apply_all(&mut state, list);
    assert_eq!(state.links["Out 1"], 0x03);
    assert_eq!(state.links["Out 2"], 0x02);

    // A slave cannot be linked further, nor become a slave of another master.
    assert!(ops::link_candidates(&state, "Out 2").is_empty());
    assert!(ops::link_slaves(&state, "Out 2", &strings(&["Out 3"])).is_err());
    assert!(!ops::link_candidates(&state, "Out 3").contains(&"Out 2".to_string()));
    assert!(!ops::link_candidates(&state, "Out 3").contains(&"Out 1".to_string()), "Out 1 is a master");

    // More channels are added to the master.
    let list = ops::link_slaves(&state, "Out 1", &strings(&["Out 2", "Out 4"])).unwrap();
    apply_all(&mut state, list);
    assert_eq!(state.links["Out 1"], 0x0B);
    assert_eq!(ops::linked_with(&state, "Out 4"), strings(&["Out 1", "Out 2"]));

    // A slave leaves its master; the rest stays linked.
    let list = ops::unlink(&state, "Out 2").unwrap();
    apply_all(&mut state, list);
    assert_eq!(ops::linked_with(&state, "Out 1"), strings(&["Out 4"]));
    assert!(ops::linked_with(&state, "Out 2").is_empty());

    // Unlinking the master releases everything.
    let list = ops::unlink(&state, "Out 1").unwrap();
    apply_all(&mut state, list);
    assert_eq!(state.links["Out 1"], 0x01);
    assert!(ops::linked_with(&state, "Out 4").is_empty());
    assert!(ops::link_slaves(&state, "In A", &strings(&["Out 1"])).is_err());
}

#[test]
fn link_slaves_copies_the_master_settings_to_new_slaves() {
    let mut state = DeviceState::new(M408);
    ops::apply(&mut state, &Op::gain("In A", -7.0));
    ops::apply(&mut state, &Op::geq_band("In A", 3, 4.0));
    let list = ops::link_slaves(&state, "In A", &strings(&["In C"])).unwrap();
    apply_all(&mut state, list);
    assert_eq!(state.gains["In C"], -7.0);
    assert_eq!(state.geq_bands["In C"][3], 4.0);
    assert_eq!(state.gains["In B"], 0.0);
}

#[test]
fn expand_linked_repeats_channel_ops_for_linked_channels() {
    let mut state = DeviceState::new(M206);
    for op in ops::link_slaves(&state, "Out 1", &strings(&["Out 2"])).unwrap() {
        ops::apply(&mut state, &op);
    }
    assert_eq!(ops::expand_linked(&state, &Op::gain("Out 1", -6.0)), [Op::gain("Out 1", -6.0), Op::gain("Out 2", -6.0)]);
    // Routing is not mirrored: a left/right pair is fed from different inputs.
    assert_eq!(ops::expand_linked(&state, &Op::matrix_route("Out 2", 3)), [Op::matrix_route("Out 2", 3)]);
    assert_eq!(ops::expand_linked(&state, &Op::gain("Out 3", 0.0)).len(), 1);
    // Names and links themselves are never mirrored.
    assert_eq!(ops::expand_linked(&state, &Op::channel_name("Out 1", Some("Sub"))).len(), 1);
}

#[test]
fn link_command_layout() {
    let mut state = DeviceState::new(M206);
    let op = Op::link("Out 2", 0x03);
    ops::apply(&mut state, &op);
    assert_eq!(ops::commands(&state, &op).unwrap()[0].1, vec![p::build_frame(0x3B, &[0x03, 0x03])]);
}

#[test]
fn test_tone() {
    let mut state = DeviceState::new(M206);
    assert_eq!(validate(&Op::test_tone(1, 0), &M206), None);
    assert!(validate(&Op::test_tone(4, 0), &M206).is_some());
    assert!(validate(&Op::test_tone(3, 31), &M206).is_some());
    let op = Op::test_tone(3, 17);
    assert_eq!(ops::expand_linked(&state, &op), std::slice::from_ref(&op));
    ops::apply(&mut state, &op);
    assert_eq!([state.test_tone_type, state.test_tone_param], [3, 17]);
    assert_eq!(ops::commands(&state, &op).unwrap()[0], ("testTone".into(), vec![p::build_frame(0x39, &[3, 17])]));
    assert_eq!(DeviceState::from_json(&state.to_json()).unwrap().test_tone_param, 17);
}

#[test]
fn copy_channel_copies_everything_except_name_link_and_routing() {
    let mut state = DeviceState::new(M408);
    for op in [
        Op::gain("Out 1", -4.0),
        Op::mute("Out 1", true),
        Op::phase("Out 1", true),
        Op::delay("Out 1", 480),
        Op::peq_band("Out 1", 3, PeqBand { gain_db: 5.0, freq_raw: 200, q_raw: 40, peq_type: 1, bypass: false }),
        Op::hi_pass("Out 1", FilterState::new(61, 8)),
        Op::matrix_route("Out 1", 0x05),
        Op::compressor("Out 1", CompressorState { ratio: 7, ..CompressorState::default() }),
        Op::channel_name("Out 1", Some("Sub L")),
    ] {
        ops::apply(&mut state, &op);
    }
    for op in ops::copy_channel(&state, "Out 1", "Out 6").unwrap() {
        assert_eq!(validate(&op, &M408), None, "{op:?}");
        ops::apply(&mut state, &op);
    }
    assert_eq!(state.gains["Out 6"], -4.0);
    assert!(state.phase["Out 6"]);
    assert_eq!(state.delays["Out 6"], 480);
    assert_eq!(state.peq_bands["Out 6"][3], state.peq_bands["Out 1"][3]);
    assert_eq!(state.hi_pass["Out 6"].slope, 8);
    assert_eq!(state.matrix_routing["Out 6"], DeviceState::new(M408).matrix_routing["Out 6"], "routing is not copied");
    assert_eq!(state.compressors["Out 6"].ratio, 7);
    assert!(state.mutes["Out 6"]);
    assert_eq!(state.channel_names["Out 6"], "Out6");
}

#[test]
fn copy_only_within_inputs_or_within_outputs() {
    let state = DeviceState::new(M408);
    assert!(ops::copy_channel(&state, "In A", "Out 1").is_err());
    assert!(ops::copy_channel(&state, "In A", "In A").is_err());
    let list = ops::copy_channel(&state, "In A", "In B").unwrap();
    assert_eq!(list.iter().filter(|op| op.name() == "geqBand").count(), 31);
}

#[test]
fn full_state_reproduces_a_state_from_defaults() {
    let mut source = DeviceState::new(M206);
    for op in [
        Op::gain("In B", -12.5),
        Op::mute("Out 4", true),
        Op::delay("Out 2", 96),
        Op::geq_band("In A", 7, 3.5),
        Op::lo_pass("Out 1", FilterState::new(61, 10)),
        Op::gate("In A", GateState { threshold: 40, ..GateState::default() }),
        Op::limiter("Out 6", LimiterState { threshold: 200, ..LimiterState::default() }),
        Op::channel_name("In A", Some("Vox")),
        Op::link("Out 1", 3),
        Op::link("Out 2", 3),
        Op::matrix_route("Out 5", 3),
        Op::matrix_gain("Out 5", "In B", -12.0),
    ] {
        ops::apply(&mut source, &op);
    }
    let mut target = DeviceState::new(M206);
    for op in ops::full_state(&source) {
        assert_eq!(validate(&op, &M206), None, "{op:?}");
        ops::apply(&mut target, &op);
    }
    // Panel-side metadata (presets, device name) is not part of the device settings.
    target.current_preset = source.current_preset.clone();
    assert_eq!(target.to_json().to_string(), source.to_json().to_string());
}

#[test]
fn device_state_survives_a_json_round_trip() {
    let mut state = DeviceState::new(M206);
    state.presets.insert(0, "Default".into());
    state.current_preset = "Default".into();
    state.device_name = "DSP206 V0104P".into();
    ops::apply(&mut state, &Op::gain("In B", -12.3));
    let band = PeqBand { freq_raw: 212, q_raw: 30, gain_db: -4.5, peq_type: 2, bypass: true };
    ops::apply(&mut state, &Op::peq_band("Out 4", 2, band));
    ops::apply(&mut state, &Op::hi_pass("Out 4", FilterState::new(100, 3)));
    ops::apply(&mut state, &Op::delay("Out 6", 4800));
    ops::apply(&mut state, &Op::compressor("Out 5", CompressorState { ratio: 5, ..CompressorState::default() }));
    ops::apply(&mut state, &Op::channel_name("In B", Some("Keys")));
    state.matrix_gains.insert("Out 1:In B".into(), -1.0);

    let text = state.to_json().to_string();
    let copy = DeviceState::from_json(&serde_json::from_str(&text).unwrap()).unwrap();
    assert_eq!(copy.model, M206);
    assert_eq!(copy.to_json().to_string(), text);
    assert!(copy.peq_bands["Out 4"][2].bypass);
    assert_eq!(copy.presets[&0], "Default");
}
