//! Live safety against the fake DSP: the safe DSP test generator and the max
//! level per channel (session), mute all and the new messages (server end to
//! end).

use std::cell::RefCell;
use std::future::Future;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use dsp_core::api::{TEST_START_LEVEL_DB, msg};
use dsp_core::ops::Op;
use dsp_core::protocol::{self as p, Frame, Protocol};
use dsp_core::{DeviceModel, DeviceState, DspStatus};
use dsp_server::Server;
use dsp_server::config::ServerConfig;
use dsp_server::registry::{DeviceRegistry, DspConfig};
use dsp_server::safety::SafetyStore;
use dsp_server::session::{DspSession, TEST_RUNNING, Timing};
use fake_dsp::{FakeDsp, FakeOptions};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

fn run<F: Future>(test: F) -> F::Output {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    tokio::task::LocalSet::new().block_on(&runtime, test)
}

fn timing() -> Timing {
    Timing {
        request: Duration::from_millis(200),
        sequence_gap: Duration::ZERO,
        reconnect_delays: vec![Duration::from_millis(100)],
        ..Timing::default()
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dsp_safety_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

async fn fake(model: DeviceModel) -> FakeDsp {
    let mut fake = FakeDsp::new(FakeOptions { model, ..FakeOptions::default() });
    fake.start(0).await.unwrap();
    fake
}

fn config(fake: &FakeDsp) -> DspConfig {
    DspConfig::new("test", "Test", "127.0.0.1", fake.port() as i64)
}

async fn wait_for(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn op(o: &Op) -> Value {
    o.to_json()
}

fn position(frames: &[Frame], frame: &Frame) -> usize {
    frames.iter().position(|f| f == frame).unwrap_or_else(|| panic!("{frame:02X?} not sent"))
}

const OUTPUTS_206: [&str; 6] = ["Out 1", "Out 2", "Out 3", "Out 4", "Out 5", "Out 6"];

// ─── DSP test generator ───

#[test]
fn the_generator_lowers_the_outputs_first_and_restores_them_when_switched_off() {
    run(async {
        let dir = temp_dir("generator");
        let file = dir.join("safety.json");
        let fake = fake(DeviceModel::DSP206).await;
        let s = DspSession::with_safety(config(&fake), timing(), SafetyStore::new(file.clone()));
        s.start();
        wait_for("ready", || s.status() == DspStatus::Ready).await;
        assert_eq!(s.apply_op(&op(&Op::gain("Out 1", -3.0)), None), None);
        assert_eq!(s.apply_op(&op(&Op::gain("Out 2", 4.5)), None), None);

        assert_eq!(s.apply_op(&op(&Op::test_tone(1, 0)), None), None);
        assert_eq!(s.status(), DspStatus::Busy);
        assert_eq!(s.test_level(), Some(TEST_START_LEVEL_DB));
        assert_eq!(s.info().test_level, Some(TEST_START_LEVEL_DB));
        // On disk before anything is lowered.
        let saved = SafetyStore::new(file.clone()).test_gains("test").expect("gains saved");
        assert_eq!(saved.len(), 6);
        assert_eq!((saved["Out 1"], saved["Out 2"], saved["Out 3"]), (-3.0, 4.5, 0.0));
        wait_for("generator started", || s.status() == DspStatus::Ready).await;

        let proto = Protocol::new(DeviceModel::DSP206);
        let sent = fake.device().received.clone();
        let start = position(&sent, &p::build_test_tone_command(1, 0));
        for out in OUTPUTS_206 {
            let lowered = position(&sent, &proto.build_gain_command(out, -50.0).unwrap());
            assert!(lowered < start, "{out} is lowered before the generator starts");
        }
        {
            let d = fake.device();
            assert_eq!(d.state.test_tone_type, 1);
            assert!(OUTPUTS_206.iter().all(|o| d.state.gains[*o] == -50.0));
            assert_eq!(d.state.gains["In A"], 0.0, "inputs stay");
        }

        // While it runs only mutes and the test controls are accepted.
        assert_eq!(s.apply_op(&op(&Op::gain("Out 1", 0.0)), None).as_deref(), Some(TEST_RUNNING));
        assert_eq!(s.apply_op(&op(&Op::delay("Out 1", 96)), None).as_deref(), Some(TEST_RUNNING));
        assert_eq!(s.load_preset(1).as_deref(), Some(TEST_RUNNING));
        assert_eq!(s.copy_channels("Out 1", &["Out 2".into()]).as_deref(), Some(TEST_RUNNING));
        assert_eq!(s.link_channels("Out 1", &["Out 2".into()]).as_deref(), Some(TEST_RUNNING));
        assert_eq!(s.apply_full_state(&DeviceState::new(DeviceModel::DSP206), "X").as_deref(), Some(TEST_RUNNING));
        assert_eq!(s.apply_op(&op(&Op::mute("Out 3", true)), None), None);
        assert_eq!(s.apply_op(&op(&Op::test_tone(3, 17)), None), None);
        assert!(s.set_test_level(f64::NAN).is_some());
        assert_eq!(s.set_test_level(-20.0), None);
        assert_eq!(s.test_level(), Some(-20.0));
        wait_for("test level and sine", || {
            let d = fake.device();
            OUTPUTS_206.iter().all(|o| d.state.gains[*o] == -20.0) && d.state.test_tone_type == 3
        })
        .await;
        assert_eq!(s.set_test_level(10.0), None);
        assert_eq!(s.test_level(), Some(0.0), "the test level ends at 0 dB");

        // Off: the generator stops first, then the gains come back.
        fake.device().received.clear();
        assert_eq!(s.apply_op(&op(&Op::test_tone(0, 17)), None), None);
        wait_for("generator stopped", || s.status() == DspStatus::Ready && s.test_level().is_none()).await;
        let sent = fake.device().received.clone();
        let stop = position(&sent, &p::build_test_tone_command(0, 17));
        let restore = position(&sent, &proto.build_gain_command("Out 1", -3.0).unwrap());
        assert!(stop < restore);
        {
            let d = fake.device();
            assert_eq!(d.state.test_tone_type, 0);
            assert_eq!((d.state.gains["Out 1"], d.state.gains["Out 2"], d.state.gains["Out 6"]), (-3.0, 4.5, 0.0));
            assert!(d.state.mutes["Out 3"], "mutes made during the test stay");
        }
        assert!(SafetyStore::new(file).test_gains("test").is_none());
        assert!(s.set_test_level(-10.0).is_some(), "not running any more");
        assert_eq!(s.apply_op(&op(&Op::gain("Out 1", -1.0)), None), None);
        s.dispose().await;
        let _ = std::fs::remove_dir_all(dir);
    });
}

#[test]
fn after_a_restart_a_dsp_that_still_plays_goes_back_into_the_test_mode() {
    run(async {
        let dir = temp_dir("resume");
        let file = dir.join("safety.json");
        let fake = fake(DeviceModel::DSP206).await;
        let s = DspSession::with_safety(config(&fake), timing(), SafetyStore::new(file.clone()));
        s.start();
        wait_for("ready", || s.status() == DspStatus::Ready).await;
        assert_eq!(s.apply_op(&op(&Op::gain("Out 5", -7.0)), None), None);
        assert_eq!(s.apply_op(&op(&Op::test_tone(2, 0)), None), None);
        wait_for("generator started", || s.status() == DspStatus::Ready).await;
        assert_eq!(s.set_test_level(-30.0), None);
        wait_for("test level", || fake.device().state.gains["Out 5"] == -30.0).await;
        s.dispose().await;

        // A new server process: the saved gains come from the file, the
        // outputs are lowered again.
        let s = DspSession::with_safety(config(&fake), timing(), SafetyStore::new(file.clone()));
        s.start();
        wait_for("test mode resumed", || s.status() == DspStatus::Ready && s.test_level().is_some()).await;
        assert_eq!(s.test_level(), Some(TEST_START_LEVEL_DB));
        assert!(OUTPUTS_206.iter().all(|o| fake.device().state.gains[*o] == -50.0));
        assert_eq!(s.apply_op(&op(&Op::gain("Out 5", 0.0)), None).as_deref(), Some(TEST_RUNNING));

        assert_eq!(s.apply_op(&op(&Op::test_tone(0, 0)), None), None);
        wait_for("gains restored", || s.status() == DspStatus::Ready && s.test_level().is_none()).await;
        assert_eq!(fake.device().state.gains["Out 5"], -7.0);
        assert_eq!(fake.device().state.test_tone_type, 0);
        s.dispose().await;
        let _ = std::fs::remove_dir_all(dir);
    });
}

#[test]
fn a_dsp_back_on_the_analog_input_gets_its_saved_gains_back() {
    run(async {
        let fake = fake(DeviceModel::DSP206).await;
        let safety = SafetyStore::memory();
        let s = DspSession::with_safety(config(&fake), timing(), safety.clone());
        s.start();
        wait_for("ready", || s.status() == DspStatus::Ready).await;
        assert_eq!(s.apply_op(&op(&Op::gain("Out 2", -12.0)), None), None);
        assert_eq!(s.apply_op(&op(&Op::test_tone(1, 0)), None), None);
        wait_for("generator started", || s.status() == DspStatus::Ready).await;

        // The generator went off while the link was down (power cycle).
        fake.device().state.test_tone_type = 0;
        fake.drop_connections();
        wait_for("link lost", || s.status() != DspStatus::Ready).await;
        wait_for("gains restored", || s.status() == DspStatus::Ready && s.test_level().is_none()).await;
        assert_eq!(fake.device().state.gains["Out 2"], -12.0);
        assert_eq!(fake.device().state.gains["Out 1"], 0.0);
        assert!(safety.test_gains("test").is_none());
        s.dispose().await;
    });
}

#[test]
fn a_generator_started_elsewhere_is_taken_into_the_test_mode() {
    run(async {
        let fake = fake(DeviceModel::DSP206).await;
        {
            let mut d = fake.device();
            d.state.test_tone_type = 1;
            d.state.gains.insert("Out 4".into(), -2.0);
        }
        let safety = SafetyStore::memory();
        let s = DspSession::with_safety(config(&fake), timing(), safety.clone());
        s.start();
        wait_for("test mode", || s.status() == DspStatus::Ready && s.test_level().is_some()).await;
        assert_eq!(safety.test_gains("test").unwrap()["Out 4"], -2.0);
        assert_eq!(fake.device().state.gains["Out 4"], -50.0);
        assert_eq!(s.apply_op(&op(&Op::test_tone(0, 0)), None), None);
        wait_for("restored", || s.status() == DspStatus::Ready && s.test_level().is_none()).await;
        assert_eq!(fake.device().state.gains["Out 4"], -2.0);
        s.dispose().await;
    });
}

// ─── Max level ───

#[test]
fn gains_stay_below_the_max_level_unless_forced() {
    run(async {
        let fake = fake(DeviceModel::DSP206).await;
        let mut c = config(&fake);
        c.gain_caps.insert("Out 1".into(), -6.0);
        let s = DspSession::new(c, timing());
        s.start();
        wait_for("ready", || s.status() == DspStatus::Ready).await;
        assert_eq!(s.info().gain_caps["Out 1"], -6.0);

        let gain = |s: &DspSession| s.state().gains["Out 1"];
        // At 0 dB the channel is above its cap: it can only go down.
        assert_eq!(s.apply_op(&op(&Op::gain("Out 1", 3.0)), None), None);
        assert_eq!(gain(&s), 0.0);
        assert_eq!(s.apply_op(&op(&Op::gain("Out 1", -20.0)), None), None);
        assert_eq!(s.apply_op(&op(&Op::gain("Out 1", 3.0)), None), None);
        assert_eq!(gain(&s), -6.0, "capped");
        assert_eq!(s.apply_op(&op(&Op::gain("Out 1", -10.0)), None), None);
        assert_eq!(gain(&s), -10.0);
        // Forced above the cap; from there it can only go down.
        assert_eq!(s.apply_op_with(&op(&Op::gain("Out 1", 3.0)), None, true), None);
        assert_eq!(gain(&s), 3.0);
        assert_eq!(s.apply_op(&op(&Op::gain("Out 1", 6.0)), None), None);
        assert_eq!(gain(&s), 3.0);
        assert_eq!(s.apply_op(&op(&Op::gain("Out 1", 1.0)), None), None);
        assert_eq!(s.apply_op(&op(&Op::gain("Out 1", 2.0)), None), None);
        assert_eq!(gain(&s), 1.0);
        wait_for("gain on the device", || fake.device().state.gains["Out 1"] == 1.0).await;

        // Linked channels share the cap of their group.
        assert_eq!(s.set_gain_cap("Out 3", Some(-12.0)), None);
        assert_eq!(s.info().gain_caps.keys().collect::<Vec<_>>(), ["Out 1", "Out 3"]);
        assert_eq!(s.apply_op(&op(&Op::gain("Out 2", -20.0)), None), None);
        assert_eq!(s.link_channels("Out 2", &["Out 3".to_string()]), None);
        wait_for("linked", || s.status() == DspStatus::Ready).await;
        assert_eq!(s.apply_op(&op(&Op::gain("Out 2", 0.0)), None), None);
        assert_eq!((s.state().gains["Out 2"], s.state().gains["Out 3"]), (-12.0, -12.0));
        // Setting or removing a cap applies to the whole group.
        assert_eq!(s.set_gain_cap("Out 2", Some(-9.0)), None);
        assert_eq!((s.info().gain_caps["Out 2"], s.info().gain_caps["Out 3"]), (-9.0, -9.0));
        assert_eq!(s.set_gain_cap("Out 3", None), None);
        assert!(!s.info().gain_caps.contains_key("Out 2") && !s.info().gain_caps.contains_key("Out 3"));
        assert_eq!(s.apply_op(&op(&Op::gain("Out 2", 0.0)), None), None);
        assert_eq!(s.state().gains["Out 3"], 0.0);
        assert!(s.set_gain_cap("Out 4", Some(20.0)).is_some());
        assert!(s.set_gain_cap("Out 9", Some(0.0)).is_some());

        // A server preset stays below the caps as well.
        assert_eq!(s.apply_op(&op(&Op::gain("Out 4", -30.0)), None), None);
        assert_eq!(s.set_gain_cap("Out 4", Some(-9.0)), None);
        let mut preset = DeviceState::new(DeviceModel::DSP206);
        preset.gains.insert("Out 4".into(), 6.0);
        preset.gains.insert("Out 5".into(), 6.0);
        assert_eq!(s.apply_full_state(&preset, "Loud"), None);
        wait_for("preset loaded", || s.status() == DspStatus::Ready).await;
        assert_eq!(fake.device().state.gains["Out 4"], -9.0);
        assert_eq!(fake.device().state.gains["Out 5"], 6.0);

        // The test level stays below the caps too.
        assert_eq!(s.apply_op(&op(&Op::test_tone(1, 0)), None), None);
        wait_for("generator started", || s.status() == DspStatus::Ready).await;
        assert_eq!(s.set_test_level(0.0), None);
        wait_for("test level", || fake.device().state.gains["Out 6"] == 0.0).await;
        assert_eq!(fake.device().state.gains["Out 4"], -9.0);
        s.dispose().await;
    });
}

// ─── Server end to end ───

struct Setup {
    dir: PathBuf,
    foh: FakeDsp,
    mon: FakeDsp,
    server: Server,
    base: String,
}

impl Setup {
    async fn new(name: &str) -> Setup {
        let dir = temp_dir(name);
        let foh = fake(DeviceModel::DSP408).await;
        let mon = fake(DeviceModel::DSP206).await;
        let config = ServerConfig {
            address: "127.0.0.1".into(),
            port: 0,
            tls_enabled: false,
            auth_enabled: false,
            data_dir: dir.clone(),
            web_root: dir.join("web"),
            ..ServerConfig::default()
        };
        let server = Server::start(config, timing()).await.unwrap();
        let base = format!("127.0.0.1:{}", server.local_addr.port());
        Setup { dir, foh, mon, server, base }
    }

    async fn connect(&self) -> TestClient {
        let mut request = format!("ws://{}/ws", self.base).into_client_request().unwrap();
        request.headers_mut().insert("origin", format!("http://{}", self.base).parse().unwrap());
        let (socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        let (mut sink, mut stream) = socket.split();
        let messages = Rc::new(RefCell::new(Vec::new()));
        let received = messages.clone();
        tokio::task::spawn_local(async move {
            while let Some(Ok(message)) = stream.next().await {
                if let Message::Text(text) = message {
                    received.borrow_mut().push(serde_json::from_str::<Value>(&text).unwrap());
                }
            }
        });
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        tokio::task::spawn_local(async move {
            while let Some(text) = rx.recv().await {
                if sink.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
        });
        TestClient { messages, tx }
    }

    fn safety(&self) -> SafetyStore {
        SafetyStore::new(self.dir.join("safety.json"))
    }

    async fn stop(self) {
        self.server.stop().await;
        let (mut foh, mut mon) = (self.foh, self.mon);
        foh.stop().await;
        mon.stop().await;
        let _ = std::fs::remove_dir_all(self.dir);
    }
}

struct TestClient {
    messages: Rc<RefCell<Vec<Value>>>,
    tx: mpsc::UnboundedSender<String>,
}

impl TestClient {
    fn send(&self, message: Value) {
        self.tx.send(message.to_string()).unwrap();
    }

    async fn wait_for(&self, test: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(m) = self.messages.borrow().iter().find(|m| test(m)) {
                return m.clone();
            }
            if Instant::now() > deadline {
                let types: Vec<Value> = self.messages.borrow().iter().map(|m| m["t"].clone()).collect();
                panic!("Message not received. Got: {types:?}");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Add a DSP and wait until it is ready; returns its id.
    async fn add_dsp(&self, name: &str, port: u16) -> String {
        self.send(json!({"t": msg::ADD_DSP, "name": name, "host": "127.0.0.1", "port": port}));
        let info = self.wait_for(|m| m["t"] == msg::DSP_INFO && m["info"]["name"] == name && m["info"]["status"] == "ready").await;
        info["info"]["id"].as_str().unwrap().to_string()
    }
}

fn all_muted(fake: &FakeDsp) -> bool {
    fake.device().state.mutes.values().all(|m| *m)
}

#[test]
fn mute_all_mutes_every_dsp_and_opens_only_what_was_open() {
    run(async {
        let setup = Setup::new("mute_all").await;
        let a = setup.connect().await;
        let hello = a.wait_for(|m| m["t"] == msg::HELLO).await;
        assert_eq!(hello["muteAll"], false);
        let foh = a.add_dsp("FOH", setup.foh.port()).await;
        setup.mon.device().state.mutes.insert("Out 3".into(), true);

        a.send(json!({"t": msg::MUTE_ALL, "on": true}));
        a.wait_for(|m| m["t"] == msg::MUTE_ALL && m["on"] == true).await;
        wait_for("FOH muted", || all_muted(&setup.foh)).await;
        assert_eq!(setup.safety().unmute_list(&foh).len(), 12);

        // A DSP that connects while mute all is on is muted as well.
        let mon = a.add_dsp("MON", setup.mon.port()).await;
        wait_for("MON muted", || all_muted(&setup.mon)).await;
        let list = setup.safety().unmute_list(&mon);
        assert_eq!(list.len(), 7);
        assert!(!list.contains(&"Out 3".to_string()), "it was muted before");
        assert!(setup.safety().mute_all());

        // A channel a client opens and mutes again meanwhile stays muted.
        a.send(json!({"t": msg::OP, "dsp": foh, "op": Op::mute("Out 2", false).to_json()}));
        wait_for("Out 2 open", || !setup.foh.device().state.mutes["Out 2"]).await;
        a.send(json!({"t": msg::OP, "dsp": foh, "op": Op::mute("Out 2", true).to_json()}));
        wait_for("Out 2 muted", || setup.foh.device().state.mutes["Out 2"]).await;

        // MON goes offline, then mute all is released (by another client).
        a.send(json!({"t": msg::UPDATE_DSP, "id": mon, "enabled": false}));
        a.wait_for(|m| m["t"] == msg::DSP_INFO && m["info"]["id"] == mon.as_str() && m["info"]["status"] == "disconnected")
            .await;
        let b = setup.connect().await;
        let hello = b.wait_for(|m| m["t"] == msg::HELLO).await;
        assert_eq!(hello["muteAll"], true);
        b.send(json!({"t": msg::MUTE_ALL, "on": false}));
        a.wait_for(|m| m["t"] == msg::MUTE_ALL && m["on"] == false).await;
        wait_for("FOH open", || {
            let d = setup.foh.device();
            d.state.mutes.iter().all(|(ch, muted)| *muted == (ch == "Out 2"))
        })
        .await;
        assert!(all_muted(&setup.mon), "offline");
        let safety = setup.safety();
        assert!(!safety.mute_all());
        assert!(safety.unmute_list(&foh).is_empty());
        assert_eq!(safety.unmute_list(&mon).len(), 7);

        // MON is opened when it is back – except the channel muted before.
        a.send(json!({"t": msg::UPDATE_DSP, "id": mon, "enabled": true}));
        wait_for("MON open", || {
            let d = setup.mon.device();
            d.state.mutes.iter().all(|(ch, muted)| *muted == (ch == "Out 3"))
        })
        .await;
        wait_for("nothing left to open", || setup.safety().unmute_list(&mon).is_empty()).await;
        setup.stop().await;
    });
}

#[test]
fn max_levels_are_stored_with_the_dsp_and_force_lifts_them() {
    run(async {
        let setup = Setup::new("gain_cap").await;
        let a = setup.connect().await;
        let id = a.add_dsp("FOH", setup.foh.port()).await;
        a.send(json!({"t": msg::SUBSCRIBE, "dsp": id}));
        a.wait_for(|m| m["t"] == msg::STATE).await;
        a.send(json!({"t": msg::OP, "dsp": id, "op": Op::gain("Out 2", -20.0).to_json()}));
        wait_for("gain", || setup.foh.device().state.gains["Out 2"] == -20.0).await;

        a.send(json!({"t": msg::GAIN_CAP, "dsp": id, "ch": "Out 2", "db": -6.0}));
        a.wait_for(|m| m["t"] == msg::DSP_INFO && m["info"]["gainCaps"]["Out 2"] == -6.0).await;
        a.send(json!({"t": msg::GAIN_CAP, "dsp": id, "ch": "Out 2", "db": "loud"}));
        a.wait_for(|m| m["t"] == msg::ERROR).await;

        // A capped value goes back to the sender too.
        a.send(json!({"t": msg::OP, "dsp": id, "op": Op::gain("Out 2", 3.0).to_json()}));
        a.wait_for(|m| m["t"] == msg::OP && m["op"] == Op::gain("Out 2", -6.0).to_json()).await;
        a.send(json!({"t": msg::OP, "dsp": id, "op": Op::gain("Out 2", 3.0).to_json(), "force": true}));
        wait_for("forced gain", || setup.foh.device().state.gains["Out 2"] == 3.0).await;

        let registry = DeviceRegistry::new(setup.dir.join("devices.json"));
        wait_for("saved", || registry.load().is_ok_and(|l| l.first().is_some_and(|c| c.gain_caps.get("Out 2") == Some(&-6.0))))
            .await;
        a.send(json!({"t": msg::GAIN_CAP, "dsp": id, "ch": "Out 2", "db": null}));
        a.wait_for(|m| m["t"] == msg::DSP_INFO && m["info"]["gainCaps"].as_object().is_some_and(|c| c.is_empty())).await;

        // The test level needs a running generator.
        a.send(json!({"t": msg::TEST_LEVEL, "dsp": id, "db": -20.0}));
        a.wait_for(|m| m["t"] == msg::ERROR && m["message"].as_str().is_some_and(|t| t.contains("not running"))).await;
        setup.stop().await;
    });
}

#[test]
fn the_safety_file_survives_a_restart() {
    let dir = temp_dir("store");
    let file = dir.join("safety.json");
    let store = SafetyStore::new(file.clone());
    store.set_mute_all(true);
    store.set_unmute_list("a", vec!["In A".into(), "Out 2".into()]);
    store.set_test_gains("b", Some([("Out 1".to_string(), -3.5)].into())).unwrap();
    let again = SafetyStore::new(file.clone());
    assert!(again.mute_all());
    assert_eq!(again.unmute_list("a"), ["In A", "Out 2"]);
    assert_eq!(again.test_gains("b").unwrap()["Out 1"], -3.5);
    again.remove_dsp("a");
    again.set_test_gains("b", None).unwrap();
    let last = SafetyStore::new(file);
    assert!(last.unmute_list("a").is_empty() && last.test_gains("b").is_none());
    let _ = std::fs::remove_dir_all(dir);
}
