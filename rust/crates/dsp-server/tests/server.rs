//! Port of server/test/server_test.dart: HTTP and WebSocket end to end.

use std::cell::RefCell;
use std::future::Future;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use dsp_core::api::msg;
use dsp_core::ops::Op;
use dsp_core::protocol::SILENCE_DB;
use dsp_server::auth::AuthService;
use dsp_server::config::ServerConfig;
use dsp_server::registry::DeviceRegistry;
use dsp_server::session::Timing;
use dsp_server::Server;
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

struct Setup {
    dir: PathBuf,
    fake: FakeDsp,
    server: Server,
    base: String,
}

impl Setup {
    async fn new(name: &str) -> Setup {
        let dir = std::env::temp_dir().join(format!("dsp_server_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("web")).unwrap();
        std::fs::write(dir.join("web/index.html"), "<html>panel</html>").unwrap();
        let mut fake = FakeDsp::new(FakeOptions::default());
        fake.start(0).await.unwrap();

        let mut auth = AuthService::new(true, dir.clone());
        auth.init().unwrap();
        auth.set_password("secret-password").unwrap();
        let config = ServerConfig {
            address: "127.0.0.1".into(),
            port: 0,
            tls_enabled: false,
            data_dir: dir.clone(),
            web_root: dir.join("web"),
            ..ServerConfig::default()
        };
        let timing = Timing {
            request: Duration::from_millis(200),
            sequence_gap: Duration::ZERO,
            reconnect_delays: vec![Duration::from_millis(100)],
            ..Timing::default()
        };
        let server = Server::start(config, timing).await.unwrap();
        let base = format!("127.0.0.1:{}", server.local_addr.port());
        Setup { dir, fake, server, base }
    }

    async fn login(&self) -> String {
        let response = reqwest::Client::new()
            .post(format!("http://{}/api/login", self.base))
            .body(json!({"password": "secret-password"}).to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let cookie = response.headers()["set-cookie"].to_str().unwrap().to_string();
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
        cookie.split(';').next().unwrap().to_string()
    }

    async fn connect(&self, cookie: &str, origin: Option<&str>) -> Result<TestClient, String> {
        let mut request = format!("ws://{}/ws", self.base).into_client_request().unwrap();
        let origin = origin.map_or(format!("http://{}", self.base), String::from);
        request.headers_mut().insert("cookie", cookie.parse().unwrap());
        request.headers_mut().insert("origin", origin.parse().unwrap());
        let (socket, _) = tokio_tungstenite::connect_async(request).await.map_err(|e| e.to_string())?;
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
            let _ = sink.close().await;
        });
        Ok(TestClient { messages, tx })
    }

    async fn stop(self) {
        self.server.stop().await;
        let mut fake = self.fake;
        fake.stop().await;
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

    fn of_type(&self, t: &str) -> Vec<Value> {
        self.messages.borrow().iter().filter(|m| m["t"] == t).cloned().collect()
    }

    async fn wait_for(&self, test: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
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

    async fn wait_until(&self, test: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !test() {
            assert!(Instant::now() < deadline, "condition not reached");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

fn info_status(m: &Value, status: &str) -> bool {
    m["t"] == msg::DSP_INFO && m["info"]["status"] == status
}

#[test]
fn serves_the_web_client_and_session_info_without_login() {
    run(async {
        let setup = Setup::new("static").await;
        let index = reqwest::get(format!("http://{}/", setup.base)).await.unwrap();
        assert_eq!(index.headers()["x-content-type-options"], "nosniff");
        assert_eq!(index.headers()["cross-origin-opener-policy"], "same-origin");
        assert!(index.text().await.unwrap().contains("panel"));

        let session: Value =
            reqwest::get(format!("http://{}/api/session", setup.base)).await.unwrap().json().await.unwrap();
        assert_eq!(session, json!({"authEnabled": true, "authenticated": false, "passwordSet": true}));
        setup.stop().await;
    });
}

#[test]
fn login_is_required_for_the_websocket() {
    run(async {
        let setup = Setup::new("login").await;
        let wrong = reqwest::Client::new()
            .post(format!("http://{}/api/login", setup.base))
            .body(json!({"password": "nope"}).to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(wrong.status(), 401);
        assert!(setup.connect("dsp_session=forged.token.value", None).await.is_err());

        let cookie = setup.login().await;
        let session: Value = reqwest::Client::new()
            .get(format!("http://{}/api/session", setup.base))
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(session["authenticated"], true);
        let client = setup.connect(&cookie, None).await.unwrap();
        client.wait_for(|m| m["t"] == msg::HELLO).await;
        setup.stop().await;
    });
}

#[test]
fn websocket_from_a_foreign_origin_is_rejected() {
    run(async {
        let setup = Setup::new("origin").await;
        let cookie = setup.login().await;
        assert!(setup.connect(&cookie, Some("https://evil.example")).await.is_err());
        setup.stop().await;
    });
}

#[test]
fn two_clients_manage_a_dsp_and_stay_in_sync() {
    run(async {
        let setup = Setup::new("sync").await;
        let cookie = setup.login().await;
        let a = setup.connect(&cookie, None).await.unwrap();
        let b = setup.connect(&cookie, None).await.unwrap();
        a.wait_for(|m| m["t"] == msg::HELLO).await;

        a.send(json!({"t": msg::ADD_DSP, "name": "FOH", "host": "127.0.0.1", "port": setup.fake.port()}));
        let info = b.wait_for(|m| info_status(m, "ready")).await;
        let id = info["info"]["id"].as_str().unwrap().to_string();

        a.send(json!({"t": msg::SUBSCRIBE, "dsp": id}));
        b.send(json!({"t": msg::SUBSCRIBE, "dsp": id}));
        let state = b.wait_for(|m| m["t"] == msg::STATE).await;
        assert_eq!(state["state"]["presets"].as_object().unwrap().len(), 20);

        a.send(json!({"t": msg::OP, "dsp": id, "op": Op::gain("Out 3", -12.0).to_json()}));
        let op = b.wait_for(|m| m["t"] == msg::OP).await;
        assert_eq!(op["op"], Op::gain("Out 3", -12.0).to_json());
        let meters = b.wait_for(|m| m["t"] == msg::METERS).await;
        // dBFS per channel plus the compressor/limiter status word
        let v = meters["v"].as_array().unwrap();
        assert_eq!(v.len(), 12);
        assert!(v.iter().all(|x| x.as_f64().unwrap() <= 0.0 && x.as_f64().unwrap() >= SILENCE_DB));
        assert_eq!(meters["dyn"], 0);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(a.of_type(msg::OP).is_empty(), "sender does not get its own op back");
        assert!(setup.fake.device().received_set_commands().contains(&"SET GAIN Out 3 → -12.0 dB".to_string()));

        // Invalid op: error plus resync for the sender only.
        let states_before = a.of_type(msg::STATE).len();
        a.send(json!({"t": msg::OP, "dsp": id, "op": {"op": "gain", "ch": "Out 9", "db": 0}}));
        a.wait_for(|m| m["t"] == msg::ERROR).await;
        a.wait_until(|| a.of_type(msg::STATE).len() > states_before).await;

        // Channel names, mutes and crossing gains go to the device.
        a.send(json!({"t": msg::OP, "dsp": id, "op": Op::channel_name("In A", Some("Vocals")).to_json()}));
        a.send(json!({"t": msg::OP, "dsp": id, "op": Op::mute("Out 2", true).to_json()}));
        a.send(json!({"t": msg::OP, "dsp": id, "op": Op::matrix_gain("Out 2", "In D", -20.0).to_json()}));
        tokio::time::sleep(Duration::from_millis(700)).await;
        {
            let d = setup.fake.device();
            assert_eq!(d.state.channel_names["In A"], "Vocals");
            assert!(d.state.mutes["Out 2"]);
            assert_eq!(d.state.matrix_gains["Out 2:In D"], -20.0);
        }
        let saved = DeviceRegistry::new(setup.dir.join("devices.json")).load().unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].port, setup.fake.port() as i64);
        assert_eq!(saved[0].model, "auto");
        assert_eq!(saved[0].device_id, 1);

        // Linking runs on the server like the original editor and ends with a
        // new state for every client.
        a.send(json!({"t": msg::LINK, "dsp": id, "master": "Out 1", "slaves": ["Out 2"]}));
        b.wait_for(|m| m["t"] == msg::STATE && m["state"]["links"]["Out 1"] == 3).await;
        let cmds = setup.fake.device().received_set_commands();
        let copy = cmds.iter().position(|c| c == "COPY CHANNEL Out 1 → Out 2").unwrap();
        let link = cmds.iter().position(|c| c == "SET LINK Out 1 → Out 1 + Out 2").unwrap();
        assert!(copy < link);
        a.send(json!({"t": msg::COPY, "dsp": id, "from": "Out 3", "to": ["Out 4"]}));
        b.wait_for(|m| m["t"] == msg::STATE && m["state"]["gains"]["Out 4"].as_f64() == Some(-12.0)).await;

        a.send(json!({"t": msg::REMOVE_DSP, "id": id}));
        b.wait_for(|m| m["t"] == msg::DSP_REMOVED).await;
        setup.stop().await;
    });
}

#[test]
fn server_presets_save_from_a_dsp_edit_offline_load_onto_the_dsp() {
    run(async {
        let setup = Setup::new("presets").await;
        let cookie = setup.login().await;
        let a = setup.connect(&cookie, None).await.unwrap();
        let b = setup.connect(&cookie, None).await.unwrap();
        a.wait_for(|m| m["t"] == msg::PRESETS).await;

        a.send(json!({"t": msg::ADD_DSP, "name": "FOH", "host": "127.0.0.1", "port": setup.fake.port()}));
        let info = a.wait_for(|m| info_status(m, "ready")).await;
        let dsp_id = info["info"]["id"].as_str().unwrap().to_string();

        // Snapshot of the DSP and an empty preset for the other model.
        a.send(json!({"t": msg::PRESET_CREATE, "name": "Hall north", "fromDsp": dsp_id}));
        a.send(json!({"t": msg::PRESET_CREATE, "name": "Club", "model": "dsp206"}));
        a.send(json!({"t": msg::PRESET_CREATE, "name": ""}));
        a.wait_for(|m| m["t"] == msg::ERROR).await;
        let list = b.wait_for(|m| m["t"] == msg::PRESETS && m["presets"].as_array().unwrap().len() == 2).await;
        let hall = list["presets"].as_array().unwrap().iter().find(|p| p["name"] == "Hall north").unwrap().clone();
        assert_eq!(hall["model"], "dsp408");
        let preset_dsp = format!("{}{}", msg::PRESET_PREFIX, hall["id"].as_str().unwrap());

        // Offline editing, synchronised between clients.
        a.send(json!({"t": msg::SUBSCRIBE, "dsp": preset_dsp}));
        b.send(json!({"t": msg::SUBSCRIBE, "dsp": preset_dsp}));
        b.wait_for(|m| m["t"] == msg::STATE && m["dsp"] == preset_dsp.as_str()).await;
        a.send(json!({"t": msg::OP, "dsp": preset_dsp, "op": Op::gain("Out 5", -15.0).to_json()}));
        a.send(json!({"t": msg::OP, "dsp": preset_dsp, "op": Op::delay("Out 6", 960).to_json()}));
        b.wait_for(|m| m["t"] == msg::OP && m["dsp"] == preset_dsp.as_str()).await;
        let offline = setup.fake.device().received_set_commands().iter().filter(|c| c.contains("Out 5")).count();
        assert_eq!(offline, 0, "offline");

        // Load onto the DSP.
        a.send(json!({"t": msg::PRESET_APPLY, "preset": hall["id"], "dsp": dsp_id}));
        a.wait_for(|m| info_status(m, "busy")).await;
        let a_messages = a.messages.clone();
        a.wait_until(|| {
            let list = a_messages.borrow();
            let busy = list.iter().position(|m| info_status(m, "busy"));
            busy.is_some_and(|b| list.iter().skip(b).any(|m| info_status(m, "ready")))
        })
        .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        {
            let d = setup.fake.device();
            assert_eq!(d.state.gains["Out 5"], -15.0);
            assert_eq!(d.state.delays["Out 6"], 960);
        }

        // Persisted.
        setup.server.hub.presets().save();
        let saved: Value =
            serde_json::from_str(&std::fs::read_to_string(setup.dir.join("presets.json")).unwrap()).unwrap();
        assert_eq!(saved["presets"].as_array().unwrap().len(), 2);

        a.send(json!({"t": msg::PRESET_DELETE, "preset": hall["id"]}));
        b.wait_for(|m| m["t"] == msg::PRESETS && m["presets"].as_array().unwrap().len() == 1).await;
        setup.stop().await;
    });
}

#[test]
fn rejects_invalid_dsp_settings() {
    run(async {
        let setup = Setup::new("invalid").await;
        let client = setup.connect(&setup.login().await, None).await.unwrap();
        client.send(json!({"t": msg::ADD_DSP, "name": "", "host": "1.2.3.4", "port": 9761}));
        client.send(json!({"t": msg::ADD_DSP, "name": "X", "host": "a b", "port": 9761}));
        client.send(json!({"t": msg::ADD_DSP, "name": "X", "host": "1.2.3.4", "port": 70000}));
        client.send(json!({"t": msg::ADD_DSP, "name": "X", "host": "1.2.3.4", "port": 9761, "deviceId": 251}));
        client.send(json!({"t": msg::ADD_DSP, "name": "X", "host": "1.2.3.4", "port": 9761, "password": "abc"}));
        client.send(json!({"t": "bogus"}));
        client.wait_until(|| client.of_type(msg::ERROR).len() == 6).await;
        assert!(setup.server.hub.sessions().is_empty());
        setup.stop().await;
    });
}
