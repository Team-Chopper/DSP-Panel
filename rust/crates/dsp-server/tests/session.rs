//! Port of server/test/dsp_session_test.dart: the session against the fake
//! DSP.

use std::future::Future;
use std::time::{Duration, Instant};

use dsp_core::ops::{self, Op};
use dsp_core::protocol::{self as p, Frame, Protocol};
use dsp_core::{
    CompressorState, ConfigDump, DeviceModel, DeviceState, DspStatus, FilterState, GateState, LimiterState, PeqBand,
};
use dsp_server::registry::DspConfig;
use dsp_server::session::{DspSession, SessionEvent, Timing};
use fake_dsp::{FakeDsp, FakeOptions};
use serde_json::Value;

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

async fn fake(model: DeviceModel) -> FakeDsp {
    let mut fake = FakeDsp::new(FakeOptions { model, ..FakeOptions::default() });
    fake.start(0).await.unwrap();
    fake
}

fn config(fake: &FakeDsp, refresh_ms: i64, model: &str) -> DspConfig {
    let mut c = DspConfig::new("test", "Test", "127.0.0.1", fake.port() as i64);
    c.refresh_ms = refresh_ms;
    c.model = model.into();
    c
}

fn session(fake: &FakeDsp, refresh_ms: i64) -> DspSession {
    DspSession::new(config(fake, refresh_ms, "auto"), timing())
}

async fn wait_for(session: &DspSession, condition: impl Fn() -> bool) {
    wait_for_within(session, Duration::from_secs(10), condition).await
}

async fn wait_for_within(session: &DspSession, timeout: Duration, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + timeout;
    while !condition() {
        if Instant::now() > deadline {
            panic!("Timed out; status={:?} message={:?}", session.status(), session.message());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn op(o: &Op) -> Value {
    o.to_json()
}

/// Frames the session sends while initializing a device of [model], in the
/// order of the original editor.
fn init_sequence(model: DeviceModel) -> Vec<Frame> {
    let mut frames = vec![
        p::handshake_command(),
        p::request_device_info(),
        p::request_lock_status(),
        p::request_preset_slots(),
        p::request_current_preset(),
    ];
    frames.extend((0..20).map(p::request_preset_name));
    frames.extend((0..ConfigDump::new(model).chunk_count() as u8).map(p::config_dump_command));
    frames.extend([p::build_heartbeat_command(), p::build_heartbeat_command()]);
    frames
}

fn received_except_meters(fake: &FakeDsp) -> Vec<Frame> {
    fake.device().received.iter().filter(|f| f[5] != 0x40).cloned().collect()
}

#[test]
fn connects_runs_init_and_reads_the_configuration() {
    run(async {
        let fake = fake(DeviceModel::DSP408).await;
        let s = session(&fake, 300);
        s.start();
        wait_for(&s, || s.status() == DspStatus::Ready).await;

        assert_eq!(s.state().device_name, "DSP408 V0104P");
        assert_eq!(s.model(), DeviceModel::DSP408);
        assert!(s.config_readable());
        assert_eq!(s.state().presets.len(), 20);
        assert_eq!(s.state().current_preset, "Default Preset");
        assert_eq!(s.state().peq_bands["In A"][0].freq_raw, 0x29);
        // Every init command was sent exactly once, in order.
        let expected = init_sequence(DeviceModel::DSP408);
        let received: Vec<Frame> = fake.device().received.iter().take(expected.len()).cloned().collect();
        assert_eq!(received, expected);
        s.dispose().await;
    });
}

#[test]
fn status_goes_through_connecting_initializing_ready() {
    run(async {
        let fake = fake(DeviceModel::DSP408).await;
        let s = session(&fake, 300);
        let mut events = s.subscribe();
        let watcher = s.clone();
        let collect = tokio::task::spawn_local(async move {
            let mut statuses: Vec<DspStatus> = Vec::new();
            while let Some(event) = events.recv().await {
                if matches!(event, SessionEvent::InfoChanged) && statuses.last() != Some(&watcher.status()) {
                    statuses.push(watcher.status());
                }
                if watcher.status() == DspStatus::Ready {
                    break;
                }
            }
            statuses
        });
        s.start();
        let statuses = collect.await.unwrap();
        let wanted = [DspStatus::Connecting, DspStatus::Initializing, DspStatus::Ready];
        let mut iter = statuses.iter();
        for w in wanted {
            assert!(iter.any(|s| *s == w), "{statuses:?}");
        }
        s.dispose().await;
    });
}

#[test]
fn polls_meters_when_idle_with_the_limiter_status_of_the_outputs() {
    run(async {
        let fake = fake(DeviceModel::DSP408).await;
        let s = session(&fake, 20);
        s.start();
        wait_for(&s, || s.status() == DspStatus::Ready).await;
        wait_for(&s, || s.meters().iter().any(|v| *v > 0.0)).await;
        assert!(s.meters().iter().all(|v| *v < 1.0), "fake levels stay below full scale");
        assert_eq!(s.dynamics(), 0);

        // A limiter at -90 dB works all the time.
        let limiter = Op::limiter("Out 2", LimiterState { threshold: 0, ..LimiterState::default() });
        assert_eq!(s.apply_op(&op(&limiter), None), None);
        wait_for(&s, || p::limiter_active(s.dynamics(), 1)).await;
        assert!(!p::limiter_active(s.dynamics(), 0));
        s.dispose().await;
    });
}

#[test]
fn coalesces_rapid_value_changes_per_key() {
    run(async {
        let fake = fake(DeviceModel::DSP408).await;
        let s = session(&fake, 300);
        s.start();
        wait_for(&s, || s.status() == DspStatus::Ready).await;
        fake.device().received.clear();

        // Slider drag: many values in one go, plus a mute in between.
        for i in 0..=50 {
            assert_eq!(s.apply_op(&op(&Op::gain("In A", -(i as f64) / 2.0)), None), None);
            if i == 10 {
                s.apply_op(&op(&Op::mute("Out 1", true)), None);
            }
        }
        wait_for(&s, || fake.device().received_set_commands().len() >= 3).await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        let sent = fake.device().received_set_commands();
        // First value goes out immediately; the rest collapse into the newest.
        let mut gains: Vec<&String> = sent.iter().filter(|c| c.starts_with("SET GAIN")).collect();
        gains.sort();
        gains.dedup();
        assert_eq!(gains, ["SET GAIN In A → +0.0 dB", "SET GAIN In A → -25.0 dB"]);
        assert!(sent.contains(&"SET MUTE Out 1 → MUTED".to_string()));
        assert!(sent.len() <= 6);
        assert_eq!(s.state().gains["In A"], -25.0);
        s.dispose().await;
    });
}

#[test]
fn rejects_invalid_ops_without_crashing() {
    run(async {
        let fake = fake(DeviceModel::DSP408).await;
        let s = session(&fake, 300);
        s.start();
        wait_for(&s, || s.status() == DspStatus::Ready).await;
        assert!(s.apply_op(&serde_json::json!({"op": "gain", "ch": "In Z", "db": 0}), None).is_some());
        assert!(s.apply_op(&serde_json::json!({"op": "peqBand", "ch": "In A", "band": 99}), None).is_some());
        assert_eq!(s.apply_op(&op(&Op::gain("In B", -3.0)), None), None);
        assert_eq!(s.status(), DspStatus::Ready);
        s.dispose().await;
    });
}

#[test]
fn reconnects_after_the_device_drops_the_connection() {
    run(async {
        let fake = fake(DeviceModel::DSP408).await;
        let s = session(&fake, 20);
        s.start();
        wait_for(&s, || s.status() == DspStatus::Ready).await;
        fake.drop_connections();
        wait_for(&s, || s.status() != DspStatus::Ready).await;
        wait_for(&s, || s.status() == DspStatus::Ready).await;
        assert_eq!(fake.connection_count(), 1);
        s.dispose().await;
    });
}

#[test]
fn detects_a_device_that_stops_answering() {
    run(async {
        let fake = fake(DeviceModel::DSP408).await;
        let s = session(&fake, 20);
        s.start();
        wait_for(&s, || s.status() == DspStatus::Ready).await;
        fake.device().ignore_commands.insert(0x40);
        wait_for(&s, || s.status() == DspStatus::Error).await;
        assert!(s.message().unwrap_or_default().contains("not responding"), "{:?}", s.message());
        fake.device().ignore_commands.clear();
        wait_for(&s, || s.status() == DspStatus::Ready).await;
        s.dispose().await;
    });
}

#[test]
fn loads_and_saves_presets_exclusively() {
    run(async {
        let fake = fake(DeviceModel::DSP408).await;
        let s = session(&fake, 20);
        s.start();
        wait_for(&s, || s.status() == DspStatus::Ready).await;

        fake.device().received.clear();
        assert_eq!(s.load_preset(3), None);
        assert_eq!(s.status(), DspStatus::Busy);
        assert!(s.apply_op(&op(&Op::gain("In A", -1.0)), None).is_some(), "busy");
        wait_for(&s, || s.status() == DspStatus::Ready).await;
        let mut expected = vec![p::build_load_preset_command(3)];
        expected.extend((0..29).map(p::config_dump_command));
        expected.extend([p::build_heartbeat_command(), p::build_heartbeat_command()]);
        assert_eq!(received_except_meters(&fake), expected);
        assert_eq!(s.state().current_preset, "Preset 4");

        assert_eq!(s.apply_op(&op(&Op::gain("Out 2", -6.0)), None), None);
        assert_eq!(s.save_preset(4, "My Show"), None);
        wait_for(&s, || s.status() == DspStatus::Ready).await;
        assert_eq!(s.state().presets[&4], "My Show");
        assert_eq!(s.state().current_preset, "My Show");
        // The pending gain reached the device before the preset was stored.
        let cmds = fake.device().received_set_commands();
        let gain = cmds.iter().position(|c| c == "SET GAIN Out 2 → -6.0 dB").unwrap();
        let store = cmds.iter().position(|c| c == "STORE PRESET NAME").unwrap();
        assert!(gain < store);

        // Recalling the stored preset reads the saved value back from the device.
        assert_eq!(s.load_preset(0), None);
        wait_for(&s, || s.status() == DspStatus::Ready && s.state().current_preset == "Default Preset").await;
        assert_eq!(s.state().gains["Out 2"], 0.0);
        assert_eq!(s.load_preset(4), None);
        wait_for(&s, || s.status() == DspStatus::Ready && s.state().current_preset == "My Show").await;
        assert_eq!(s.state().gains["Out 2"], -6.0);
        s.dispose().await;
    });
}

#[test]
fn dsp206_is_detected_and_read_with_its_own_layout() {
    run(async {
        let fake = fake(DeviceModel::DSP206).await;
        let s = session(&fake, 20);
        s.start();
        wait_for(&s, || s.status() == DspStatus::Ready).await;
        assert_eq!(s.model(), DeviceModel::DSP206);
        assert_eq!(s.info().model, "dsp206");
        assert_eq!(s.state().device_name, "DSP206 V0104P");
        assert!(s.config_readable());
        let channels: Vec<String> = DeviceModel::DSP206.channels().map(String::from).collect();
        assert_eq!(s.state().gains.keys().cloned().collect::<Vec<_>>(), channels);
        let expected = init_sequence(DeviceModel::DSP206);
        let received: Vec<Frame> = fake.device().received.iter().take(expected.len()).cloned().collect();
        assert_eq!(received, expected);
        wait_for(&s, || s.meters().len() == 8 && s.meters().iter().any(|v| *v > 0.0)).await;
        s.dispose().await;
    });
}

#[test]
fn dsp206_all_functions_reach_the_device_and_are_read_back_after_reconnecting() {
    run(async {
        let fake = fake(DeviceModel::DSP206).await;
        let s = session(&fake, 20);
        s.start();
        wait_for(&s, || s.status() == DspStatus::Ready).await;

        let list = [
            Op::gain("Out 6", -12.0),
            Op::mute("In B", true),
            Op::phase("Out 1", true),
            Op::delay("Out 2", 960),
            Op::matrix_route("Out 3", 3),
            Op::matrix_gain("Out 3", "In B", -9.5),
            Op::geq_band("In A", 10, -3.5),
            Op::peq_band("Out 4", 8, PeqBand { freq_raw: 250, q_raw: 30, gain_db: 4.5, peq_type: 2, bypass: false }),
            Op::peq_band("In B", 3, PeqBand { bypass: true, ..PeqBand::default() }),
            Op::hi_pass("Out 1", FilterState::new(11, 8)),
            Op::lo_pass("Out 1", FilterState::new(61, 8)),
            Op::gate("In A", GateState { attack: 4, release: 199, hold: 49, threshold: 40 }),
            Op::compressor("Out 5", CompressorState { ratio: 9, attack: 9, release: 99, knee: 2, threshold: 170 }),
            Op::limiter("Out 6", LimiterState { attack: 19, release: 199, threshold: 200, ..LimiterState::default() }),
            Op::channel_name("Out 1", Some("Sub L")),
        ];
        for o in &list {
            assert_eq!(s.apply_op(&op(o), None), None, "{o:?}");
        }
        assert!(s.apply_op(&op(&Op::gain("Out 7", 0.0)), None).is_some(), "no Out 7 on a 206");
        wait_for(&s, || fake.device().state.channel_names["Out 1"] == "Sub L").await;

        {
            let d = fake.device();
            assert_eq!(d.state.gains["Out 6"], -12.0);
            assert!(d.state.phase["Out 1"]);
            assert_eq!(d.state.delays["Out 2"], 960);
            assert_eq!(d.state.matrix_gains["Out 3:In B"], -9.5);
            assert_eq!(d.state.compressors["Out 5"].ratio, 9);
            assert!(d.received_set_commands().contains(&"SET MATRIX GAIN Out 3 ← In B → -9.5 dB".to_string()));
        }

        // Reconnect: everything in the dump comes back from the device.
        fake.drop_connections();
        wait_for(&s, || s.status() != DspStatus::Ready).await;
        wait_for(&s, || s.status() == DspStatus::Ready).await;
        let st = s.state();
        assert_eq!(st.gains["Out 6"], -12.0);
        assert_eq!(st.matrix_routing["Out 3"], 3);
        assert_eq!(st.matrix_gains["Out 3:In B"], -9.5);
        assert_eq!(st.geq_bands["In A"][10], -3.5);
        assert_eq!(st.peq_bands["Out 4"][8].gain_db, 4.5);
        assert_eq!(st.hi_pass["Out 1"], FilterState::new(11, 8));
        assert_eq!(st.gates["In A"].threshold, 40);
        assert_eq!(st.compressors["Out 5"], CompressorState { ratio: 9, attack: 9, release: 99, knee: 2, threshold: 170 });
        assert_eq!(st.limiters["Out 6"].threshold, 200);
        assert_eq!(st.channel_names["Out 1"], "Sub L");
        assert!(st.mutes["In B"]);
        assert!(!st.mutes["In A"]);
        assert!(st.phase["Out 1"]);
        assert_eq!(st.delays["Out 2"], 960);
        let bypass: Vec<bool> = st.peq_bands["In B"].iter().map(|b| b.bypass).collect();
        assert_eq!(bypass, [false, false, false, true, false, false, false, false]);
        s.dispose().await;
    });
}

#[test]
fn dsp206_links_like_the_original_editor() {
    run(async {
        let fake = fake(DeviceModel::DSP206).await;
        let s = session(&fake, 300);
        s.start();
        wait_for(&s, || s.status() == DspStatus::Ready).await;
        assert_eq!(s.apply_op(&op(&Op::gain("Out 1", -4.0)), None), None);
        fake.device().received.clear();

        assert_eq!(s.link_channels("Out 1", &["Out 2".to_string()]), None);
        assert_eq!(s.status(), DspStatus::Busy);
        wait_for(&s, || s.status() == DspStatus::Ready).await;
        let sent = received_except_meters(&fake);
        let proto = Protocol::new(DeviceModel::DSP206);
        assert_eq!(
            sent[..3].to_vec(),
            vec![
                proto.build_gain_command("Out 1", -4.0).unwrap(), // queued before the link
                proto.build_copy_channel_command("Out 1", "Out 2").unwrap(),
                proto.build_link_command("Out 1", 0x03).unwrap(),
            ]
        );
        // Then everything is read again.
        assert_eq!(sent[3], p::request_preset_slots());
        let dumps = sent.iter().filter(|f| f[5] == 0x27).count();
        assert_eq!(dumps, ConfigDump::new(DeviceModel::DSP206).chunk_count());
        let st = s.state();
        assert_eq!(st.links["Out 1"], 3);
        assert_eq!(st.links["Out 2"], 2, "the slave keeps its own bit");
        assert_eq!(st.gains["Out 2"], -4.0, "the device copied the master");
        assert_eq!(ops::linked_with(&st, "Out 2"), ["Out 1"]);

        // A change of the master is sent once; the DSP applies it to the slave.
        fake.device().received.clear();
        assert_eq!(s.apply_op(&op(&Op::gain("Out 1", -9.0)), None), None);
        assert_eq!(s.state().gains["Out 2"], -9.0);
        wait_for(&s, || fake.device().state.gains["Out 2"] == -9.0).await;
        assert_eq!(fake.device().received_set_commands(), ["SET GAIN Out 1 → -9.0 dB"]);

        // Unlinking sends the master's own bit.
        assert_eq!(s.link_channels("Out 1", &[]), None);
        wait_for(&s, || s.status() == DspStatus::Ready).await;
        assert_eq!(fake.device().state.links["Out 1"], 1);
        assert!(ops::linked_with(&s.state(), "Out 2").is_empty());
        assert!(s.link_channels("Out 1", &["In A".to_string()]).is_some());
        s.dispose().await;
    });
}

#[test]
fn dsp206_copies_channels_on_the_device_and_reads_the_result_back() {
    run(async {
        let fake = fake(DeviceModel::DSP206).await;
        let s = session(&fake, 300);
        s.start();
        wait_for(&s, || s.status() == DspStatus::Ready).await;
        {
            let mut d = fake.device();
            d.state.gains.insert("Out 3".into(), -12.0);
            d.state.mutes.insert("Out 3".into(), true);
            d.state.channel_names.insert("Out 3".into(), "Fill".into());
        }
        assert_eq!(s.copy_channels("Out 3", &["Out 4".into(), "Out 5".into()]), None);
        wait_for(&s, || s.status() == DspStatus::Ready).await;
        let cmds = fake.device().received_set_commands();
        let a = cmds.iter().position(|c| c == "COPY CHANNEL Out 3 → Out 4").unwrap();
        let b = cmds.iter().position(|c| c == "COPY CHANNEL Out 3 → Out 5").unwrap();
        assert!(a < b);
        let st = s.state();
        assert_eq!(st.gains["Out 5"], -12.0);
        assert!(st.mutes["Out 4"]);
        assert_eq!(st.channel_names["Out 4"], "Out4", "names are not copied");
        assert!(s.copy_channels("Out 3", &["In A".into()]).is_some());
        assert!(s.copy_channels("Out 3", &["Out 3".into()]).is_some());
        s.dispose().await;
    });
}

#[test]
fn dsp206_a_complete_state_from_a_server_preset_is_sent_to_the_device() {
    run(async {
        let fake = fake(DeviceModel::DSP206).await;
        let s = session(&fake, 300);
        s.start();
        wait_for(&s, || s.status() == DspStatus::Ready).await;

        let mut preset = DeviceState::new(DeviceModel::DSP206);
        for o in [
            Op::gain("Out 3", -7.5),
            Op::peq_band("In B", 2, PeqBand { freq_raw: 120, q_raw: 30, gain_db: -3.0, peq_type: 1, bypass: false }),
            Op::compressor("Out 6", CompressorState { ratio: 11, threshold: 160, ..CompressorState::default() }),
            Op::channel_name("Out 1", Some("Sub")),
            Op::mute("In A", true),
        ] {
            ops::apply(&mut preset, &o);
        }
        let wrong = s.apply_full_state(&DeviceState::new(DeviceModel::DSP408), "Wrong").unwrap();
        assert!(wrong.contains("DSP 408"));
        assert_eq!(s.apply_full_state(&preset, "Club"), None);
        assert_eq!(s.status(), DspStatus::Busy);
        wait_for_within(&s, Duration::from_secs(30), || s.status() == DspStatus::Ready).await;

        {
            let d = fake.device();
            assert_eq!(d.state.gains["Out 3"], -7.5);
            assert_eq!(d.state.peq_bands["In B"][2].peq_type, 1);
            assert_eq!(d.state.compressors["Out 6"].ratio, 11);
            assert_eq!(d.state.channel_names["Out 1"], "Sub");
            assert!(d.state.mutes["In A"]);
        }
        assert_eq!(s.state().current_preset, "Club");
        s.dispose().await;
    });
}

#[test]
fn dsp206_a_wrong_configured_model_is_refused() {
    run(async {
        let fake = fake(DeviceModel::DSP206).await;
        let s = DspSession::new(config(&fake, 300, "dsp408"), timing());
        s.start();
        wait_for(&s, || s.status() == DspStatus::Error).await;
        assert!(s.message().unwrap_or_default().contains("configured as DSP 408"), "{:?}", s.message());
        let cmds: Vec<u8> = fake.device().received.iter().map(|f| f[5]).collect();
        let pos = |c: u8| cmds.iter().position(|x| *x == c).unwrap();
        assert!(pos(0x10) < pos(0x13) && pos(0x13) < pos(0x11));
        assert!(!cmds.contains(&0x27));
        s.dispose().await;
    });
}

#[test]
fn slot_table_and_current_preset_are_read() {
    run(async {
        let fake = fake(DeviceModel::DSP408).await;
        fake.device().clear_preset(5);
        fake.device().current_preset = 3;
        let s = session(&fake, 300);
        s.start();
        wait_for(&s, || s.status() == DspStatus::Ready).await;
        let st = s.state();
        assert!(!st.preset_slots.contains(&5));
        assert!(st.preset_slots.contains(&4));
        assert_eq!(st.current_preset_number, 3);
        s.dispose().await;
    });
}

#[test]
fn a_device_with_an_unknown_protocol_version_is_refused() {
    run(async {
        let fake = fake(DeviceModel::DSP408).await;
        fake.device().device_info = "DSP408 V0201".into();
        let s = session(&fake, 300);
        s.start();
        wait_for(&s, || s.status() == DspStatus::Error).await;
        assert!(s.message().unwrap_or_default().contains("Wrong device version"));
        s.dispose().await;
    });
}

#[test]
fn a_locked_dsp_needs_its_password() {
    run(async {
        let fake = fake(DeviceModel::DSP408).await;
        fake.device().locked = true;
        let s = session(&fake, 300);
        s.start();
        wait_for(&s, || s.status() == DspStatus::Error).await;
        assert!(s.message().unwrap_or_default().contains("locked"));
        assert!(!fake.device().received.iter().any(|f| f[5] == 0x22), "nothing is read from a locked DSP");
        s.stop().await;

        let mut c = config(&fake, 300, "auto");
        c.password = "1234".into();
        let s = DspSession::new(c, timing());
        s.start();
        wait_for(&s, || s.status() == DspStatus::Ready).await;
        let last = fake.device().received.iter().rfind(|f| f[5] == 0x2D).cloned().unwrap();
        assert_eq!(last, p::build_password_command(0, "1234"));
        s.dispose().await;
    });
}

#[test]
fn device_id_addresses_the_frames() {
    run(async {
        let mut fake = FakeDsp::new(FakeOptions { device_id: 7, ..FakeOptions::default() });
        fake.start(0).await.unwrap();
        let mut c = config(&fake, 300, "auto");
        c.device_id = 7;
        let s = DspSession::new(c, timing());
        s.start();
        wait_for(&s, || s.status() == DspStatus::Ready).await;
        assert_eq!(fake.device().received[0], p::with_device_id(&p::handshake_command(), 7));
        assert_eq!(s.info().device_id, 7);
        s.dispose().await;
    });
}

#[test]
fn heartbeat_instead_of_meters_while_nobody_watches_and_0x11_when_stopping() {
    run(async {
        let fake = fake(DeviceModel::DSP408).await;
        let t = Timing { heartbeat_idle: Duration::from_millis(100), ..timing() };
        let s = DspSession::new(config(&fake, 20, "auto"), t);
        s.set_meters_wanted(false);
        s.start();
        wait_for(&s, || s.status() == DspStatus::Ready).await;
        fake.device().received.clear();
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(!fake.device().received.iter().any(|f| f[5] == 0x40));
        assert!(fake.device().received.iter().any(|f| f[5] == 0x12));
        s.set_meters_wanted(true);
        wait_for(&s, || fake.device().received.iter().any(|f| f[5] == 0x40)).await;

        s.stop().await;
        wait_for(&s, || fake.device().received.last().is_some_and(|f| f[5] == 0x11)).await;
        assert_eq!(fake.device().received.last().cloned().unwrap(), p::build_disconnect_command());
        s.dispose().await;
    });
}

#[test]
fn a_server_preset_is_loaded_like_a_preset_file() {
    run(async {
        let fake = fake(DeviceModel::DSP408).await;
        let s = session(&fake, 300);
        s.start();
        wait_for(&s, || s.status() == DspStatus::Ready).await;
        fake.device().received.clear();
        assert_eq!(s.apply_full_state(&DeviceState::default(), "Hall"), None);
        wait_for_within(&s, Duration::from_secs(30), || s.status() == DspStatus::Ready).await;
        let sent = received_except_meters(&fake);
        assert_eq!(sent[0], p::build_store_preset_name_command("Hall"));
        assert_eq!(sent[1][5], 0x3D, "In A name first");
        assert_eq!(sent.last().cloned().unwrap(), p::build_heartbeat_command());
        assert_eq!(sent.iter().filter(|f| f[5] == 0x34).count(), 12, "gain once per channel");
        s.dispose().await;
    });
}

#[test]
fn mute_phase_and_delay_of_a_dsp408_come_from_the_device() {
    run(async {
        let fake = fake(DeviceModel::DSP408).await;
        {
            let mut d = fake.device();
            d.state.mutes.insert("Out 3".into(), true);
            d.state.phase.insert("In C".into(), true);
            d.state.delays.insert("In A".into(), 96);
        }
        let s = session(&fake, 300);
        s.start();
        wait_for(&s, || s.status() == DspStatus::Ready).await;
        let st = s.state();
        assert!(st.mutes["Out 3"]);
        assert!(!st.mutes["Out 2"]);
        assert!(st.phase["In C"]);
        assert_eq!(st.delays["In A"], 96);
        s.dispose().await;
    });
}

#[test]
fn stop_disconnects_and_does_not_reconnect() {
    run(async {
        let fake = fake(DeviceModel::DSP408).await;
        let s = session(&fake, 300);
        s.start();
        wait_for(&s, || s.status() == DspStatus::Ready).await;
        s.stop().await;
        assert_eq!(s.status(), DspStatus::Disconnected);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(s.status(), DspStatus::Disconnected);
        assert_eq!(fake.connection_count(), 0);
        s.dispose().await;
    });
}

#[test]
fn reports_an_unreachable_device_and_keeps_retrying() {
    run(async {
        let mut fake = fake(DeviceModel::DSP408).await;
        let port = fake.port();
        fake.stop().await;
        let c = DspConfig::new("x", "X", "127.0.0.1", port as i64);
        let s = DspSession::new(c, Timing { reconnect_delays: vec![Duration::from_millis(50)], ..Timing::default() });
        s.start();
        wait_for(&s, || s.status() == DspStatus::Error).await;
        assert!(s.message().unwrap_or_default().contains("Connection failed"), "{:?}", s.message());

        let mut fake = FakeDsp::new(FakeOptions::default());
        fake.start(port).await.unwrap();
        wait_for(&s, || s.status() == DspStatus::Ready).await;
        s.dispose().await;
    });
}
