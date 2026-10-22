//! Parity with the Dart implementation: replays the reference data written by
//! rust/tool/parity/bin/generate.dart (the Dart dsp_core) and checks that the
//! Rust port validates, links, applies, encodes, decodes and describes the
//! same way, byte for byte.

use std::sync::OnceLock;

use dsp_core::describe::{describe, describe_rx};
use dsp_core::ops::{self, Op};
use dsp_core::protocol::{self as p, Protocol, format_as_hex, hex_to_bytes};
use dsp_core::{ConfigDump, DeviceModel, DeviceState, Message};
use serde_json::Value;

fn fixture() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| {
        let text = include_str!("fixtures/parity.json");
        serde_json::from_str(text).expect("parity.json is valid JSON")
    })
}

fn hex(bytes: &[u8]) -> String {
    format_as_hex(bytes).replace(' ', "")
}

fn model_of(v: &Value) -> DeviceModel {
    DeviceModel::by_id(v.as_str())
}

fn ops_json(list: &[Op]) -> Value {
    Value::Array(list.iter().map(Op::to_json).collect())
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect()
}

fn ops_result(result: Result<Vec<Op>, String>) -> (Option<Value>, Option<String>) {
    match result {
        Ok(list) => (Some(ops_json(&list)), None),
        Err(e) => (None, Some(e)),
    }
}

fn check_ops_result(expected: &Value, result: Result<Vec<Op>, String>, what: &str) {
    let (ops, error) = ops_result(result);
    assert_eq!(error.as_deref(), expected.get("error").and_then(Value::as_str), "{what}: error");
    if let Some(ops) = ops {
        assert_eq!(&ops, &expected["ops"], "{what}: ops");
    }
}

#[test]
fn op_sequences() {
    let mut checked = 0;
    for seq in fixture()["sequences"].as_array().unwrap() {
        let model = model_of(&seq["model"]);
        let mut state = DeviceState::new(model);
        for (i, step) in seq["steps"].as_array().unwrap().iter().enumerate() {
            let op_json = &step["op"];
            let what = format!("{} step {i}: {op_json}", model.id);
            let error = ops::validate(op_json, &model);
            assert_eq!(error.as_deref(), step["error"].as_str(), "{what}: validate");
            if error.is_none() {
                let op = Op::parse(op_json).unwrap_or_else(|e| panic!("{what}: parse {e}"));
                let expanded = ops::expand_linked(&state, &op);
                assert_eq!(ops_json(&expanded), step["expanded"], "{what}: expandLinked");
                for o in &expanded {
                    ops::apply(&mut state, o);
                }
                let commands = ops::commands(&state, &op).unwrap();
                let expected = step["commands"].as_object().unwrap();
                let keys: Vec<&str> = commands.iter().map(|(k, _)| k.as_str()).collect();
                let expected_keys: Vec<&str> = expected.keys().map(String::as_str).collect();
                assert_eq!(keys, expected_keys, "{what}: command keys");
                for (key, frames) in &commands {
                    let frames: Vec<String> = frames.iter().map(|f| hex(f)).collect();
                    assert_eq!(frames, strings(&expected[key]), "{what}: frames of {key}");
                }
                let descriptions: Vec<String> =
                    commands.iter().flat_map(|(_, frames)| frames.iter().map(|f| describe(f, &model))).collect();
                assert_eq!(descriptions, strings(&step["describe"]), "{what}: describe");
                checked += 1;
            }
            if let Some(expected) = step.get("state") {
                assert_eq!(&state.to_json(), expected, "{what}: state");
                let links = step["links"].as_object().unwrap();
                for (ch, l) in links {
                    assert_eq!(ops::edit_group_of(&state, ch), strings(&l["editGroup"]), "{what}: editGroupOf {ch}");
                    assert_eq!(ops::master_of(&state, ch).as_deref(), l["master"].as_str(), "{what}: masterOf {ch}");
                    assert_eq!(ops::slaves_of(&state, ch), strings(&l["slaves"]), "{what}: slavesOf {ch}");
                    assert_eq!(ops::link_group_of(&state, ch), strings(&l["group"]), "{what}: linkGroupOf {ch}");
                    assert_eq!(ops::link_candidates(&state, ch), strings(&l["candidates"]), "{what}: candidates {ch}");
                }
                let ls = &step["linkSlaves"];
                let result = ops::link_slaves(&state, ls["master"].as_str().unwrap(), &strings(&ls["slaves"]));
                check_ops_result(ls, result, &format!("{what}: linkSlaves"));
                let ul = &step["unlink"];
                check_ops_result(ul, ops::unlink(&state, ul["ch"].as_str().unwrap()), &format!("{what}: unlink"));
                let cp = &step["copy"];
                let result = ops::copy_channel(&state, cp["from"].as_str().unwrap(), cp["to"].as_str().unwrap());
                check_ops_result(cp, result, &format!("{what}: copyChannel"));
            }
        }
        assert_eq!(state.to_json(), seq["state"], "{}: final state", model.id);
        assert_eq!(ops_json(&ops::full_state(&state)), seq["fullState"], "{}: fullState", model.id);
        assert_eq!(hex(&ConfigDump::new(model).encode(&state)), seq["dump"].as_str().unwrap(), "{}: dump", model.id);
        // The state also survives the JSON round trip.
        assert_eq!(DeviceState::from_json(&seq["state"]).unwrap().to_json(), seq["state"]);
    }
    assert!(checked > 1500, "only {checked} valid ops checked");
}

#[test]
fn config_dumps() {
    for case in fixture()["dumps"].as_array().unwrap() {
        let model = model_of(&case["model"]);
        let stream = hex_to_bytes(case["stream"].as_str().unwrap()).unwrap();
        let what = format!("{} dump of {} bytes", model.id, stream.len());
        let dump = ConfigDump::new(model);
        let result = dump.decode(&stream);
        assert_eq!(result.problems, strings(&case["problems"]), "{what}: problems");
        assert_eq!(result.switches_read, case["switchesRead"].as_bool().unwrap(), "{what}: switches");
        assert_eq!(result.rest_read, case["restRead"].as_bool().unwrap(), "{what}: rest");
        assert_eq!(result.state.to_json(), case["state"], "{what}: state");
        assert_eq!(hex(&dump.encode(&result.state)), case["encoded"].as_str().unwrap(), "{what}: encoded");
    }
}

#[test]
fn device_answers() {
    for case in fixture()["rx"].as_array().unwrap() {
        let frame = hex_to_bytes(case["frame"].as_str().unwrap()).unwrap();
        let rx = case["rx"].as_bool().unwrap();
        for model in DeviceModel::ALL {
            let text = if rx { describe_rx(&frame, &model, 1) } else { describe(&frame, &model) };
            assert_eq!(text, case[format!("describe_{}", model.id)].as_str().unwrap(), "{}", case["frame"]);
            if let Some(expected) = case.get(format!("meters_{}", model.id)) {
                let Some(Message::Keepalive { levels, dynamics }) = Protocol::new(model).decode(&frame) else {
                    panic!("not a meter answer: {}", case["frame"]);
                };
                let expected: Vec<f64> = expected.as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
                assert_eq!(levels, expected);
                assert_eq!(dynamics as i64, case["dynamics"].as_i64().unwrap());
                for (level, db) in levels.iter().zip(case[format!("dbfs_{}", model.id)].as_array().unwrap()) {
                    assert!((p::level_to_db(*level) - db.as_f64().unwrap()).abs() < 1e-9);
                }
            }
        }
    }
}

fn rows(key: &str) -> &'static Vec<Value> {
    fixture()["encodings"][key].as_array().unwrap()
}

#[test]
fn value_encodings() {
    for r in rows("gain") {
        let db = r[0].as_f64().unwrap();
        assert_eq!(p::quantize_gain(db), r[1].as_f64().unwrap(), "quantize {db}");
        assert_eq!(p::gain_db_to_value(p::quantize_gain(db)), r[2].as_i64().unwrap(), "gain {db}");
    }
    for r in rows("geq") {
        let db = r[0].as_f64().unwrap();
        assert_eq!(p::quantize_geq_db(db), r[1].as_f64().unwrap(), "quantize GEQ {db}");
        assert_eq!(p::geq_db_to_value(db), r[2].as_i64().unwrap(), "GEQ {db}");
    }
    for (raw, hz) in rows("freq").iter().enumerate() {
        let hz = hz.as_f64().unwrap();
        assert!((p::peq_freq_to_hz(raw as i64) - hz).abs() <= hz * 1e-12, "freq {raw}");
    }
    for r in rows("hz") {
        assert_eq!(p::peq_hz_to_freq(r[0].as_f64().unwrap()), r[1].as_i64().unwrap(), "hz {}", r[0]);
    }
    for (raw, q) in rows("q").iter().enumerate() {
        let q = q.as_f64().unwrap();
        assert!((p::peq_raw_to_q(raw as i64) - q).abs() <= q * 1e-12, "Q {raw}");
    }
    for r in rows("qRaw") {
        assert_eq!(p::peq_q_to_raw(r[0].as_f64().unwrap()), r[1].as_i64().unwrap(), "Q raw {}", r[0]);
    }
    for r in rows("delay") {
        assert_eq!(p::delay_ms_to_raw(r[0].as_f64().unwrap()), r[1].as_i64().unwrap(), "delay {}", r[0]);
    }
    for r in rows("threshold") {
        assert_eq!(p::threshold_db_to_raw(r[0].as_f64().unwrap()), r[1].as_i64().unwrap(), "threshold {}", r[0]);
    }
    for r in rows("channelName") {
        assert_eq!(hex(&p::encode_channel_name(r[0].as_str().unwrap())), r[1].as_str().unwrap(), "name {}", r[0]);
    }
    for r in rows("presetName") {
        let frame = p::build_store_preset_name_command(r[0].as_str().unwrap());
        assert_eq!(hex(&frame), r[1].as_str().unwrap(), "preset name {}", r[0]);
    }
    for r in rows("password") {
        assert_eq!(hex(&p::build_password_command(0, r[0].as_str().unwrap())), r[1].as_str().unwrap(), "password {}", r[0]);
    }
}
