//! Hub: owns all DSP sessions and all connected browser clients, and routes
//! messages between them (see dsp_core::api). Runs on the server's
//! single-threaded `LocalSet`; the WebSocket handlers talk to it through
//! [HubCommand]s.
//!
//! Mute all is a hub function: it mutes every channel of every ready DSP and
//! remembers (in safety.json) which channels were open. A DSP that becomes
//! ready while it is on is muted as well (also after a preset load); releasing
//! it unmutes only the remembered channels, those of a DSP that is offline at
//! that moment when it is ready again.
//!
//! The hub also keeps the panel data (panel.json: limiter calculator, link
//! group names, limit light settings), feeds the limit light page from the
//! meter answers, runs the network search and copies settings between DSPs.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

use dsp_core::api::msg;
use dsp_core::ops::{self, Op};
use dsp_core::protocol::level_to_db;
use dsp_core::{DeviceModel, DeviceState, DspStatus, LogEntry};
use indexmap::IndexMap;
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};
use tokio::task::{AbortHandle, spawn_local};

use crate::light::{LightSnapshot, LightTracker, watched_outputs};
use crate::log;
use crate::panel_data::PanelStore;
use crate::presets::PresetStore;
use crate::registry::{DeviceRegistry, DspConfig};
use crate::safety::SafetyStore;
use crate::scan::{self, ScanTiming};
use crate::session::{ConfigUpdate, DspSession, SessionEvent, Timing};

pub const MAX_MESSAGE_LENGTH: usize = 64 * 1024;
pub const MAX_DEVICES: usize = 32;

/// Messages from the WebSocket handlers to the hub.
#[derive(Debug)]
pub enum HubCommand {
    Connected { id: u64, remote: String, out: mpsc::UnboundedSender<String> },
    Text { id: u64, text: String },
    /// A binary frame (not part of the API).
    Binary { id: u64 },
    Disconnected { id: u64 },
}

struct Client {
    out: mpsc::UnboundedSender<String>,
    remote: String,
    subscriptions: HashSet<String>,
}

impl Client {
    fn send(&self, message: &Value) {
        let _ = self.out.send(message.to_string());
    }
}

struct HubInner {
    weak: Weak<RefCell<HubInner>>,
    registry: DeviceRegistry,
    timing: Timing,
    sessions: IndexMap<String, DspSession>,
    session_tasks: HashMap<String, AbortHandle>,
    clients: IndexMap<u64, Client>,
    presets: PresetStore,
    save_generation: u64,
    safety: SafetyStore,
    /// Last status seen per DSP, to notice when one becomes ready.
    statuses: HashMap<String, DspStatus>,
    panel: PanelStore,
    light: LightTracker,
    light_tx: watch::Sender<LightSnapshot>,
    scanning: bool,
}

/// Handle to the hub (single-threaded).
#[derive(Clone)]
pub struct Hub(Rc<RefCell<HubInner>>);

fn json_int(v: Option<&Value>) -> Option<i64> {
    v.and_then(Value::as_i64)
}

/// Where the hub keeps its data besides the DSP list.
pub struct HubStores {
    pub presets_file: PathBuf,
    pub safety: SafetyStore,
    pub panel: PanelStore,
    /// The limit light page reads from the other end.
    pub light: watch::Sender<LightSnapshot>,
}

impl HubStores {
    /// Stores that are not written to disk (tests).
    pub fn memory(presets_file: PathBuf) -> Self {
        HubStores {
            presets_file,
            safety: SafetyStore::memory(),
            panel: PanelStore::memory(),
            light: watch::channel(LightSnapshot::default()).0,
        }
    }
}

impl Hub {
    pub fn new(registry: DeviceRegistry, stores: HubStores, timing: Timing) -> Self {
        let HubStores { presets_file, safety, panel, light } = stores;
        Hub(Rc::new_cyclic(|weak| {
            RefCell::new(HubInner {
                weak: weak.clone(),
                registry,
                timing,
                sessions: IndexMap::new(),
                session_tasks: HashMap::new(),
                clients: IndexMap::new(),
                presets: PresetStore::new(presets_file),
                save_generation: 0,
                safety,
                statuses: HashMap::new(),
                panel,
                light: LightTracker::default(),
                light_tx: light,
                scanning: false,
            })
        }))
    }

    pub fn panel(&self) -> PanelStore {
        self.0.borrow().panel.clone()
    }

    pub fn presets(&self) -> PresetStore {
        self.0.borrow().presets.clone()
    }

    pub fn safety(&self) -> SafetyStore {
        self.0.borrow().safety.clone()
    }

    pub fn sessions(&self) -> Vec<DspSession> {
        self.0.borrow().sessions.values().cloned().collect()
    }

    pub fn client_count(&self) -> usize {
        self.0.borrow().clients.len()
    }

    pub fn start(&self) -> Result<(), String> {
        let configs = self.0.borrow().registry.load()?;
        for config in configs {
            self.add_session(config);
        }
        let presets = self.presets();
        presets.load();
        log::info(format!("{} DSP(s) configured, {} server preset(s)", self.0.borrow().sessions.len(), presets.len()));
        self.publish_light();
        self.update_meter_polling();
        Ok(())
    }

    /// Handle WebSocket commands until the channel closes.
    pub async fn run(&self, mut commands: mpsc::UnboundedReceiver<HubCommand>) {
        while let Some(command) = commands.recv().await {
            self.handle(command);
        }
    }

    pub fn handle(&self, command: HubCommand) {
        match command {
            HubCommand::Connected { id, remote, out } => self.handle_connection(id, remote, out),
            HubCommand::Text { id, text } => self.on_message(id, &text),
            HubCommand::Binary { id } => self.send_to(id, &json!({"t": msg::ERROR, "message": "Invalid message"})),
            HubCommand::Disconnected { id } => {
                let removed = self.0.borrow_mut().clients.shift_remove(&id);
                if let Some(client) = removed {
                    self.update_meter_polling();
                    log::info(format!("Client disconnected ({}), {} total", client.remote, self.client_count()));
                }
            }
        }
    }

    pub async fn shutdown(&self) {
        self.0.borrow_mut().save_generation += 1;
        self.save();
        self.presets().save();
        let (sessions, tasks) = {
            let mut inner = self.0.borrow_mut();
            inner.clients.clear(); // closes the WebSocket writers
            let tasks: Vec<AbortHandle> = inner.session_tasks.drain().map(|(_, t)| t).collect();
            (inner.sessions.values().cloned().collect::<Vec<_>>(), tasks)
        };
        for task in tasks {
            task.abort();
        }
        for session in sessions {
            session.dispose().await;
        }
    }

    fn add_session(&self, config: DspConfig) {
        let (timing, safety) = {
            let inner = self.0.borrow();
            (inner.timing.clone(), inner.safety.clone())
        };
        let id = config.id.clone();
        let session = DspSession::with_safety(config, timing, safety);
        let mut events = session.subscribe();
        let hub = Rc::downgrade(&self.0);
        let task_id = id.clone();
        let task = spawn_local(async move {
            while let Some(event) = events.recv().await {
                let Some(inner) = hub.upgrade() else { break };
                Hub(inner).on_session_event(&task_id, event);
            }
        });
        {
            let mut inner = self.0.borrow_mut();
            inner.sessions.insert(id.clone(), session.clone());
            inner.session_tasks.insert(id, task.abort_handle());
        }
        session.set_meters_wanted(false);
        session.start();
    }

    /// Meters are polled only while a client shows the DSP, or for the limit
    /// light; otherwise the session keeps the link alive with the idle
    /// heartbeat.
    fn update_meter_polling(&self) {
        let inner = self.0.borrow();
        let light = inner.panel.light().enabled;
        for (id, session) in &inner.sessions {
            session.set_meters_wanted(light || inner.clients.values().any(|c| c.subscriptions.contains(id)));
        }
    }

    // ─── Limit light ───

    fn publish_light(&self) {
        let inner = self.0.borrow();
        let snapshot = inner.light.snapshot(inner.panel.light().enabled);
        inner.light_tx.send_if_modified(|current| {
            let changed = *current != snapshot;
            *current = snapshot;
            changed
        });
    }

    fn note_light(&self, session: &DspSession) {
        let light = self.0.borrow().panel.light();
        if !light.enabled {
            return;
        }
        let id = session.id();
        let ignore = light.ignore.get(&id).cloned().unwrap_or_default();
        let watched = watched_outputs(&session.state(), &ignore);
        self.0.borrow_mut().light.note(&id, &watched, session.dynamics(), tokio::time::Instant::now());
        self.publish_light();
    }

    fn panel_message(&self) -> Value {
        json!({"t": msg::PANEL, "data": self.panel().to_json()})
    }

    // ─── Network search ───

    /// Search the given networks (or the server's own and those of the
    /// configured DSPs) and answer [client] with a scanResult.
    fn start_scan(&self, client: u64, message: &Value) -> Option<String> {
        if self.0.borrow().scanning {
            return Some("A network search is already running".into());
        }
        let mut networks = Vec::new();
        match message.get("networks").filter(|v| !v.is_null()) {
            Some(list) => {
                let Some(list) = list.as_array() else { return Some("networks must be a list".into()) };
                for n in list {
                    match n.as_str().and_then(scan::parse_network) {
                        Some(net) => networks.push(net),
                        None => return Some(format!("Not a network: {n}")),
                    }
                }
            }
            None => {
                networks.extend(scan::own_network());
                for s in self.sessions() {
                    networks.extend(scan::network_of_host(&s.config().host));
                }
            }
        }
        let mut unique = Vec::new();
        for n in networks {
            if !unique.contains(&n) {
                unique.push(n);
            }
        }
        unique.truncate(scan::MAX_NETWORKS);
        if unique.is_empty() {
            return Some("No network to search – enter one, e.g. 192.168.1.0/24".into());
        }
        // A DSP takes one connection: the ones the panel talks to are known.
        let mut skip = HashSet::new();
        let mut known: HashMap<String, (String, bool)> = HashMap::new();
        for s in self.sessions() {
            let config = s.config();
            let connected = matches!(
                s.status(),
                DspStatus::Connecting | DspStatus::Initializing | DspStatus::Ready | DspStatus::Busy
            );
            if connected {
                skip.insert(config.host.clone());
            }
            known.insert(config.host.clone(), (config.name.clone(), connected));
        }
        self.0.borrow_mut().scanning = true;
        let weak = self.0.borrow().weak.clone();
        let texts: Vec<String> = unique.iter().map(|n| scan::network_text(*n)).collect();
        log::info(format!("Network search: {}", texts.join(", ")));
        spawn_local(async move {
            let found = scan::scan(&unique, &skip, scan::DSP_PORT, ScanTiming::default()).await;
            let Some(inner) = weak.upgrade() else { return };
            let hub = Hub(inner);
            hub.0.borrow_mut().scanning = false;
            let mut list: Vec<Value> = found
                .into_iter()
                .map(|f| {
                    let known_as = known.get(&f.host).map(|(name, _)| name.clone());
                    json!({"host": f.host, "port": f.port, "info": f.info, "model": f.model, "known": known_as, "connected": false})
                })
                .collect();
            // The DSPs the panel is connected to were not probed.
            for (host, (name, connected)) in known {
                if connected && scan::network_of_host(&host).is_some_and(|n| texts.contains(&scan::network_text(n))) {
                    list.push(json!({"host": host, "port": scan::DSP_PORT, "info": "", "model": null,
                        "known": name, "connected": true}));
                }
            }
            list.sort_by_key(|v| v["host"].as_str().and_then(|h| h.parse::<std::net::Ipv4Addr>().ok()));
            hub.send_to(client, &json!({"t": msg::SCAN_RESULT, "found": list, "networks": texts}));
        });
        None
    }

    // ─── Copy between DSPs ───

    fn copy_to(&self, message: &Value) -> Option<String> {
        let Some(source) = self.session(message.get("dsp")) else { return Some("Unknown DSP".into()) };
        let Some(from) = message.get("from").and_then(Value::as_str) else { return Some("from missing".into()) };
        let Some(parts) = string_list(message.get("parts")).filter(|p| !p.is_empty()) else {
            return Some("Choose what to copy".into());
        };
        let Some(targets) = message.get("targets").and_then(Value::as_array).filter(|t| !t.is_empty()) else {
            return Some("No channel to copy to".into());
        };
        if !source.config_readable() || source.status() == DspStatus::Disconnected {
            return Some("The settings of this DSP are not known".into());
        }
        let source_state = source.state();
        let source_id = source.id();
        let mut changed = 0;
        for t in targets {
            let (Some(dsp), Some(ch)) = (t.get("dsp").and_then(Value::as_str), t.get("ch").and_then(Value::as_str)) else {
                return Some("Invalid target".into());
            };
            if dsp == source_id && ch == from {
                continue;
            }
            let Some(target) = self.session(Some(&Value::from(dsp))) else { return Some("Unknown DSP".into()) };
            if target.status() != DspStatus::Ready {
                return Some(format!("{} is not ready", target.config().name));
            }
            let list = match ops::copy_parts(&source_state, from, &target.state(), ch, &parts) {
                Ok(list) => list,
                Err(e) => return Some(e),
            };
            for op in list {
                if let Some(error) = target.apply_op_with(&op.to_json(), None, false) {
                    return Some(format!("{}: {error}", target.config().name));
                }
                changed += 1;
            }
        }
        log::info(format!("Copied {} of {from} ({changed} values)", parts.join(", ")));
        None
    }

    // ─── Clients ───

    fn handle_connection(&self, id: u64, remote: String, out: mpsc::UnboundedSender<String>) {
        let hello = {
            let inner = self.0.borrow();
            json!({
                "t": msg::HELLO,
                "dsps": inner.sessions.values().map(|s| s.info()).collect::<Vec<_>>(),
                "muteAll": inner.safety.mute_all(),
                "panel": inner.panel.to_json(),
            })
        };
        let presets = self.preset_list_message();
        let client = Client { out, remote: remote.clone(), subscriptions: HashSet::new() };
        client.send(&hello);
        client.send(&presets);
        self.0.borrow_mut().clients.insert(id, client);
        log::info(format!("Client connected ({remote}), {} total", self.client_count()));
    }

    fn send_to(&self, client: u64, message: &Value) {
        if let Some(c) = self.0.borrow().clients.get(&client) {
            c.send(message);
        }
    }

    fn on_message(&self, client: u64, data: &str) {
        if data.encode_utf16().count() > MAX_MESSAGE_LENGTH {
            self.send_to(client, &json!({"t": msg::ERROR, "message": "Invalid message"}));
            return;
        }
        let message = match serde_json::from_str::<Value>(data) {
            Ok(v @ Value::Object(_)) => v,
            _ => {
                self.send_to(client, &json!({"t": msg::ERROR, "message": "Invalid JSON"}));
                return;
            }
        };
        if let Some(error) = self.dispatch(client, &message) {
            let dsp = message.get("dsp").cloned().unwrap_or(Value::Null);
            self.send_to(client, &json!({"t": msg::ERROR, "dsp": dsp, "message": error}));
        }
    }

    fn session(&self, id: Option<&Value>) -> Option<DspSession> {
        let id = id.and_then(Value::as_str)?;
        self.0.borrow().sessions.get(id).cloned()
    }

    /// Handle one client message. Returns an error message.
    fn dispatch(&self, client: u64, message: &Value) -> Option<String> {
        let t = message.get("t").and_then(Value::as_str);
        match t {
            Some(msg::ADD_DSP) => return self.add_dsp(message),
            Some(msg::UPDATE_DSP) => return self.update_dsp(message),
            Some(msg::REMOVE_DSP) => return self.remove_dsp(message),
            Some(msg::PRESET_CREATE) => return self.preset_create(message),
            Some(msg::PRESET_RENAME) => {
                let Some(id) = message.get("preset").and_then(Value::as_str) else {
                    return Some("preset missing".into());
                };
                let error = self.presets().rename(id, message.get("name"));
                if error.is_none() {
                    self.broadcast(&self.preset_list_message());
                }
                return error;
            }
            Some(msg::PRESET_DELETE) => {
                let Some(id) = message.get("preset").and_then(Value::as_str) else {
                    return Some("preset missing".into());
                };
                let key = format!("{}{id}", msg::PRESET_PREFIX);
                for c in self.0.borrow_mut().clients.values_mut() {
                    c.subscriptions.remove(&key);
                }
                let error = self.presets().delete(id);
                if error.is_none() {
                    self.broadcast(&self.preset_list_message());
                }
                return error;
            }
            Some(msg::PRESET_APPLY) => {
                let preset = message.get("preset").and_then(Value::as_str).and_then(|id| self.presets().get(id));
                let target = self.session(message.get("dsp"));
                let Some((name, state)) = preset else { return Some("Unknown preset".into()) };
                let Some(target) = target else { return Some("Unknown DSP".into()) };
                return target.apply_full_state(&state, &name);
            }
            Some(msg::MUTE_ALL) => {
                return match message.get("on").and_then(Value::as_bool) {
                    Some(on) => {
                        self.set_mute_all(on);
                        None
                    }
                    None => Some("on missing".into()),
                };
            }
            Some(msg::SCAN) => return self.start_scan(client, message),
            Some(msg::COPY_TO) => return self.copy_to(message),
            Some(msg::LIMITER_SETUP) => {
                let Some(setup) = message.get("setup") else { return Some("setup missing".into()) };
                let error = self.panel().set_limiter(setup).err();
                self.broadcast(&self.panel_message());
                return error;
            }
            Some(msg::LIGHT_SETTINGS) => {
                let Some(settings) = message.get("settings") else { return Some("settings missing".into()) };
                let error = self.panel().set_light(settings).err();
                if !self.panel().light().enabled {
                    self.0.borrow_mut().light = LightTracker::default();
                }
                self.publish_light();
                self.update_meter_polling();
                self.broadcast(&self.panel_message());
                return error;
            }
            _ => {}
        }

        // Everything else addresses one DSP, or a server preset in the offline editor.
        if let Some(id) = message.get("dsp").and_then(Value::as_str)
            && id.starts_with(msg::PRESET_PREFIX) {
                return self.dispatch_preset(client, message, id);
            }
        let Some(session) = self.session(message.get("dsp")) else {
            return Some("Unknown DSP".into());
        };
        let id = session.id();

        match t {
            Some(msg::SUBSCRIBE) => {
                if let Some(c) = self.0.borrow_mut().clients.get_mut(&client) {
                    c.subscriptions.insert(id.clone());
                }
                self.update_meter_polling();
                self.send_to(client, &json!({"t": msg::STATE, "dsp": id, "state": session.state_json()}));
                let entries: Vec<Value> = session.log_entries().iter().map(LogEntry::to_json).collect();
                self.send_to(client, &json!({"t": msg::LOG, "dsp": id, "reset": true, "entries": entries}));
                None
            }
            Some(msg::UNSUBSCRIBE) => {
                if let Some(c) = self.0.borrow_mut().clients.get_mut(&client) {
                    c.subscriptions.remove(&id);
                }
                self.update_meter_polling();
                None
            }
            Some(msg::OP) => {
                let Some(op) = message.get("op").filter(|o| o.is_object()) else {
                    return Some("op missing".into());
                };
                let force = message.get("force") == Some(&Value::Bool(true));
                let error = session.apply_op_with(op, Some(client), force);
                if error.is_some() {
                    // The client already applied the op locally — resynchronise it.
                    self.send_to(client, &json!({"t": msg::STATE, "dsp": id, "state": session.state_json()}));
                } else if let Ok(Op::Mute { ch, on: false }) = Op::parse(op) {
                    self.opened_during_mute_all(&session, &ch);
                }
                error
            }
            Some(msg::GAIN_CAP) => {
                let Some(ch) = message.get("ch").and_then(Value::as_str) else {
                    return Some("ch missing".into());
                };
                match message.get("db") {
                    None | Some(Value::Null) => session.set_gain_cap(ch, None),
                    Some(db) => match db.as_f64() {
                        Some(db) => session.set_gain_cap(ch, Some(db)),
                        None => Some("db must be a number".into()),
                    },
                }
            }
            Some(msg::TEST_LEVEL) => {
                let error = match message.get("db").and_then(Value::as_f64) {
                    Some(db) => session.set_test_level(db),
                    None => Some("db missing".into()),
                };
                if error.is_some() {
                    // The client moved its fader already.
                    self.send_to(client, &json!({"t": msg::DSP_INFO, "info": session.info()}));
                }
                error
            }
            Some(msg::LINK) => {
                let master = message.get("master").and_then(Value::as_str);
                let slaves = string_list(message.get("slaves"));
                match (master, slaves) {
                    (Some(master), Some(slaves)) => session.link_channels(master, &slaves),
                    _ => Some("master/slaves missing".into()),
                }
            }
            Some(msg::COPY) => {
                let from = message.get("from").and_then(Value::as_str);
                let to = string_list(message.get("to"));
                match (from, to) {
                    (Some(from), Some(to)) => session.copy_channels(from, &to),
                    _ => Some("from/to missing".into()),
                }
            }
            Some(msg::PRESET_UNDO) => session.undo_load(),
            Some(msg::LINK_NAME) => {
                let members = string_list(message.get("members"));
                let name = message.get("name").and_then(Value::as_str);
                let (Some(members), Some(name)) = (members, name) else { return Some("members/name missing".into()) };
                let model = session.model();
                if members.iter().any(|m| !model.has_channel(m)) {
                    return Some("Unknown channel".into());
                }
                let mut members = members;
                members.sort_by_key(|m| model.channels().position(|c| c == m));
                members.dedup();
                let error = self.panel().set_link_name(&id, &members, name).err();
                self.broadcast(&self.panel_message());
                error
            }
            Some(msg::LOAD_PRESET) => match json_int(message.get("index")) {
                Some(index) => session.load_preset(index),
                None => Some("index missing".into()),
            },
            Some(msg::SAVE_PRESET) => {
                match (json_int(message.get("index")), message.get("name").and_then(Value::as_str)) {
                    (Some(index), Some(name)) => session.save_preset(index, name),
                    _ => Some("index/name missing".into()),
                }
            }
            Some(msg::SET_REFRESH) => match json_int(message.get("ms")) {
                Some(ms) => {
                    session.set_refresh_interval(ms);
                    None
                }
                None => Some("ms missing".into()),
            },
            Some(msg::DEBUG) => {
                let flag = |k: &str| message.get(k).and_then(Value::as_bool);
                session.set_debug(flag("rx"), flag("tx"));
                None
            }
            Some(msg::CLEAR_LOG) => {
                session.clear_log();
                None
            }
            _ => Some(format!("Unknown message type: {}", t.map_or("null".to_string(), String::from))),
        }
    }

    // ─── Server presets ───

    fn preset_list_message(&self) -> Value {
        json!({"t": msg::PRESETS, "presets": self.presets().list()})
    }

    fn preset_create(&self, message: &Value) -> Option<String> {
        let name = message.get("name");
        if let Some(error) = PresetStore::validate_name(name) {
            return Some(error);
        }
        let name = name.and_then(Value::as_str).unwrap_or_default();
        let state = match message.get("fromDsp").filter(|v| !v.is_null()) {
            Some(from) => {
                let Some(session) = self.session(Some(from)) else { return Some("Unknown DSP".into()) };
                if session.status() != DspStatus::Ready {
                    return Some("The DSP must be connected to save its settings".into());
                }
                if !session.config_readable() {
                    return Some("The configuration of this DSP could not be read".into());
                }
                session.state()
            }
            None => match message.get("model").and_then(Value::as_str) {
                Some(model) if DeviceModel::is_known_id(model) => DeviceState::new(DeviceModel::by_id(Some(model))),
                _ => return Some("Choose a model for the new preset".into()),
            },
        };
        match self.presets().create(name, &state) {
            Ok((name, model)) => {
                log::info(format!("Server preset created: {name} ({model})"));
                self.broadcast(&self.preset_list_message());
                None
            }
            Err(e) => Some(e),
        }
    }

    fn dispatch_preset(&self, client: u64, message: &Value, dsp_id: &str) -> Option<String> {
        let preset_id = &dsp_id[msg::PRESET_PREFIX.len()..];
        let presets = self.presets();
        let Some(state) = presets.state_json(preset_id) else { return Some("Unknown preset".into()) };
        match message.get("t").and_then(Value::as_str) {
            Some(msg::SUBSCRIBE) => {
                if let Some(c) = self.0.borrow_mut().clients.get_mut(&client) {
                    c.subscriptions.insert(dsp_id.to_string());
                }
                self.send_to(client, &json!({"t": msg::STATE, "dsp": dsp_id, "state": state}));
                None
            }
            Some(msg::UNSUBSCRIBE) => {
                if let Some(c) = self.0.borrow_mut().clients.get_mut(&client) {
                    c.subscriptions.remove(dsp_id);
                }
                None
            }
            Some(msg::OP) => {
                let Some(op) = message.get("op").filter(|o| o.is_object()) else {
                    return Some("op missing".into());
                };
                match presets.apply_op(preset_id, op) {
                    Ok(applied) => {
                        for op in applied {
                            let m = json!({"t": msg::OP, "dsp": dsp_id, "op": op.to_json()});
                            self.to_subscribers(dsp_id, &m, Some(client));
                        }
                        None
                    }
                    Err(error) => {
                        let state = presets.state_json(preset_id).unwrap_or(state);
                        self.send_to(client, &json!({"t": msg::STATE, "dsp": dsp_id, "state": state}));
                        Some(error)
                    }
                }
            }
            _ => Some("Not available for an offline preset".into()),
        }
    }

    // ─── DSP list ───

    fn add_dsp(&self, message: &Value) -> Option<String> {
        let field = |k: &str| message.get(k).filter(|v| !v.is_null());
        let name = field("name").and_then(Value::as_str);
        let host = field("host").and_then(Value::as_str);
        let port = field("port").map_or(Some(9761), Value::as_i64);
        let model = field("model").map_or(Some("auto"), Value::as_str);
        let device_id = field("deviceId").map_or(Some(1), Value::as_i64);
        let password = field("password").map_or(Some(""), Value::as_str);
        let (Some(name), Some(host), Some(port), Some(model), Some(device_id), Some(password)) =
            (name, host, port, model, device_id, password)
        else {
            return Some("Name, host and port are required".into());
        };
        let host = host.trim();
        if let Some(error) =
            DspConfig::validate_fields(Some(name), Some(host), Some(port), Some(model), Some(device_id), Some(password))
        {
            return Some(error);
        }
        if self.0.borrow().sessions.len() >= MAX_DEVICES {
            return Some(format!("At most {MAX_DEVICES} DSPs"));
        }
        let config = DspConfig {
            id: DspConfig::new_id(),
            name: name.trim().to_string(),
            host: host.to_string(),
            port,
            model: model.to_string(),
            device_id,
            password: password.to_uppercase(),
            ..DspConfig::new("", "", "", port)
        };
        let (id, name, address) = (config.id.clone(), config.name.clone(), format!("{}:{}", config.host, config.port));
        self.add_session(config);
        if let Some(session) = self.session(Some(&Value::from(id))) {
            self.broadcast(&json!({"t": msg::DSP_INFO, "info": session.info()}));
        }
        self.schedule_save();
        log::info(format!("DSP added: {name} ({address})"));
        None
    }

    fn update_dsp(&self, message: &Value) -> Option<String> {
        let Some(session) = self.session(message.get("id")) else { return Some("Unknown DSP".into()) };
        let field = |k: &str| message.get(k).filter(|v| !v.is_null());
        let wrong_type = field("name").is_some_and(|v| !v.is_string())
            || field("host").is_some_and(|v| !v.is_string())
            || field("port").is_some_and(|v| !v.is_i64())
            || field("enabled").is_some_and(|v| !v.is_boolean())
            || field("model").is_some_and(|v| !v.is_string())
            || field("deviceId").is_some_and(|v| !v.is_i64())
            || field("password").is_some_and(|v| !v.is_string());
        if wrong_type {
            return Some("Invalid values".into());
        }
        let text = |k: &str| field(k).and_then(Value::as_str);
        let host = text("host").map(str::trim);
        let port = field("port").and_then(Value::as_i64);
        let device_id = field("deviceId").and_then(Value::as_i64);
        if let Some(error) =
            DspConfig::validate_fields(text("name"), host, port, text("model"), device_id, text("password"))
        {
            return Some(error);
        }
        let update = ConfigUpdate {
            name: text("name").map(|n| n.trim().to_string()),
            host: host.map(String::from),
            port,
            enabled: field("enabled").and_then(Value::as_bool),
            model: text("model").map(String::from),
            device_id,
            password: text("password").map(String::from),
        };
        spawn_local(async move { session.update_config(update).await });
        None
    }

    fn remove_dsp(&self, message: &Value) -> Option<String> {
        let id = message.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        let (session, task) = {
            let mut inner = self.0.borrow_mut();
            let Some(session) = inner.sessions.shift_remove(&id) else { return Some("Unknown DSP".into()) };
            for c in inner.clients.values_mut() {
                c.subscriptions.remove(&id);
            }
            (session, inner.session_tasks.remove(&id))
        };
        if let Some(task) = task {
            task.abort();
        }
        let name = session.config().name;
        spawn_local(async move { session.dispose().await });
        {
            let mut inner = self.0.borrow_mut();
            inner.statuses.remove(&id);
            inner.safety.remove_dsp(&id);
            inner.panel.remove_dsp(&id);
            inner.light.forget(&id);
        }
        self.publish_light();
        self.broadcast(&json!({"t": msg::DSP_REMOVED, "id": id}));
        self.broadcast(&self.panel_message());
        self.schedule_save();
        log::info(format!("DSP removed: {name}"));
        None
    }

    // ─── Session events ───

    fn on_session_event(&self, id: &str, event: SessionEvent) {
        let Some(session) = self.session(Some(&Value::from(id))) else { return };
        match event {
            SessionEvent::InfoChanged => {
                self.broadcast(&json!({"t": msg::DSP_INFO, "info": session.info()}));
                let status = session.status();
                let previous = self.0.borrow_mut().statuses.insert(id.to_string(), status);
                if status == DspStatus::Ready && previous != Some(DspStatus::Ready) {
                    self.became_ready(&session);
                }
                if !matches!(status, DspStatus::Ready | DspStatus::Busy) {
                    self.0.borrow_mut().light.forget(id);
                    self.publish_light();
                }
            }
            SessionEvent::StateReplaced => {
                self.to_subscribers(id, &json!({"t": msg::STATE, "dsp": id, "state": session.state_json()}), None)
            }
            SessionEvent::OpApplied { op, origin } => {
                self.to_subscribers(id, &json!({"t": msg::OP, "dsp": id, "op": op.to_json()}), origin)
            }
            SessionEvent::MetersUpdated => {
                let values: Vec<f64> = session.meters().iter().map(|v| (level_to_db(*v) * 10.0).round() / 10.0).collect();
                let (tx, rx) = session.packets();
                let m = json!({"t": msg::METERS, "dsp": id, "v": values, "dyn": session.dynamics(), "tx": tx, "rx": rx});
                self.to_subscribers(id, &m, None);
                self.note_light(&session);
            }
            // To everyone: a preset is often loaded from the preset page,
            // without showing the DSP.
            SessionEvent::Notice { text, error } => {
                self.broadcast(&json!({"t": msg::NOTICE, "dsp": id, "message": text, "error": error}))
            }
            SessionEvent::LogAppended { entries, reset } => {
                let entries: Vec<Value> = entries.iter().map(LogEntry::to_json).collect();
                self.to_subscribers(id, &json!({"t": msg::LOG, "dsp": id, "reset": reset, "entries": entries}), None);
            }
            SessionEvent::ConfigChanged => self.schedule_save(),
        }
    }

    fn broadcast(&self, message: &Value) {
        let text = message.to_string();
        for c in self.0.borrow().clients.values() {
            let _ = c.out.send(text.clone());
        }
    }

    fn to_subscribers(&self, dsp_id: &str, message: &Value, except: Option<u64>) {
        let mut text: Option<String> = None;
        for (id, c) in &self.0.borrow().clients {
            if Some(*id) == except || !c.subscriptions.contains(dsp_id) {
                continue;
            }
            let text = text.get_or_insert_with(|| message.to_string());
            let _ = c.out.send(text.clone());
        }
    }

    // ─── Mute all ───

    /// Switch mute all on or off for every DSP (any client may do both).
    pub fn set_mute_all(&self, on: bool) {
        let safety = self.safety();
        safety.set_mute_all(on);
        for session in self.sessions() {
            if on {
                self.mute_dsp(&session);
            } else {
                self.unmute_dsp(&session);
            }
        }
        self.broadcast(&json!({"t": msg::MUTE_ALL, "on": on}));
        log::info(if on { "Mute all on" } else { "Mute all released" });
    }

    /// Mute every open channel of [session] and remember them (in addition to
    /// those remembered before: the DSP may have been muted earlier).
    fn mute_dsp(&self, session: &DspSession) {
        let Some(open) = session.mute_open_channels() else { return };
        let safety = self.safety();
        let id = session.id();
        let mut list = safety.unmute_list(&id);
        for ch in open {
            if !list.contains(&ch) {
                list.push(ch);
            }
        }
        safety.set_unmute_list(&id, list);
    }

    /// Unmute the remembered channels of [session]; they stay remembered if
    /// it is not ready.
    fn unmute_dsp(&self, session: &DspSession) {
        let safety = self.safety();
        let id = session.id();
        let list = safety.unmute_list(&id);
        if !list.is_empty() && session.unmute_channels(&list) {
            safety.set_unmute_list(&id, Vec::new());
        }
    }

    /// A DSP is ready (connected, or a job ended): mute it while mute all is
    /// on, or unmute what it still owes from a release.
    fn became_ready(&self, session: &DspSession) {
        if self.safety().mute_all() {
            self.mute_dsp(session);
        } else {
            self.unmute_dsp(session);
        }
    }

    /// A client opened [ch] while mute all is on: it is no longer the release's
    /// business (a later mute of it stays).
    fn opened_during_mute_all(&self, session: &DspSession, ch: &str) {
        let safety = self.safety();
        if !safety.mute_all() {
            return;
        }
        let id = session.id();
        let group = ops::edit_group_of(&session.state(), ch);
        let mut list = safety.unmute_list(&id);
        list.retain(|c| !group.contains(c));
        safety.set_unmute_list(&id, list);
    }

    // ─── Persistence ───

    fn schedule_save(&self) {
        let generation = {
            let mut inner = self.0.borrow_mut();
            inner.save_generation += 1;
            inner.save_generation
        };
        let weak = self.0.borrow().weak.clone();
        spawn_local(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if let Some(inner) = weak.upgrade()
                && inner.borrow().save_generation == generation {
                    Hub(inner).save();
                }
        });
    }

    fn save(&self) {
        let inner = self.0.borrow();
        let configs: Vec<DspConfig> = inner.sessions.values().map(DspSession::config).collect();
        if let Err(e) = inner.registry.save(&configs) {
            log::warn(format!("Could not save {}: {e}", inner.registry.file.display()));
        }
    }
}

fn string_list(v: Option<&Value>) -> Option<Vec<String>> {
    v?.as_array()?.iter().map(|s| s.as_str().map(String::from)).collect()
}
