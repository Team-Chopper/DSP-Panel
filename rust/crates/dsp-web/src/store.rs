//! Client state: the DSP list from the server and, per DSP (or server preset in
//! the offline editor), a mirror of its device state. Changes are applied
//! locally right away (optimistic UI) and sent to the server as ops, coalesced
//! per key and flushed every 30 ms – like the Flutter client's providers.
//!
//! Everything is held in Leptos signals: a component reads exactly the values
//! it shows (through memos), so a change re-renders only what depends on it.
//! Meters bypass the signals entirely (see `meters`).

use std::cell::Cell;
use std::collections::HashMap;
use std::time::Duration;

use dsp_core::api::{TEST_LEVEL_MAX_DB, TEST_LEVEL_MIN_DB, TEST_RUNNING, msg};
use dsp_core::limiter_calc::LimiterSetup;
use dsp_core::ops::{self, Op};
use dsp_core::panel::{LightSettings, PanelData};
use dsp_core::protocol as p;
use dsp_core::{
    CompressorState, DeviceModel, DeviceState, DspInfo, DspStatus, FilterState, GateState, LimiterState, LogEntry, ServerPresetInfo,
};
use leptos::prelude::*;
use serde_json::{Value, json};

use crate::meters::MeterBank;
use crate::net::send;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerState {
    /// Asking the server whether a login is needed.
    Checking,
    /// Login required (or the session expired).
    LoginRequired,
    /// Opening the WebSocket.
    Connecting,
    /// Connected; messages flow.
    Connected,
    /// Server unreachable; retrying.
    Disconnected,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub text: String,
    pub error: bool,
}

thread_local! {
    static APP: Cell<Option<App>> = const { Cell::new(None) };
    /// Owner of the root component: per-device state is created under it, so
    /// it lives as long as the panel and not as long as the view that happened
    /// to trigger its creation (a dialog, a message handler).
    static ROOT: std::cell::RefCell<Option<Owner>> = const { std::cell::RefCell::new(None) };
}

fn in_root<T>(f: impl FnOnce() -> T) -> T {
    match ROOT.with(|r| r.borrow().clone()) {
        Some(owner) => owner.with(f),
        None => f(),
    }
}

/// The application state (cheap to copy: it only holds signal handles).
#[derive(Clone, Copy)]
pub struct App {
    pub server: RwSignal<ServerState>,
    pub auth_enabled: RwSignal<bool>,
    pub password_set: RwSignal<bool>,
    pub server_error: RwSignal<Option<String>>,
    /// DSP ids in the order of the server.
    pub dsps: RwSignal<Vec<String>>,
    /// True once the DSP list has been received.
    pub loaded: RwSignal<bool>,
    pub selected: RwSignal<Option<String>>,
    pub presets: RwSignal<Vec<ServerPresetInfo>>,
    /// Server presets open in the offline editor, as "preset:<id>".
    pub editors: RwSignal<Vec<String>>,
    pub toasts: RwSignal<Vec<Toast>>,
    /// Mute all is on (for every DSP, switched by any client).
    pub mute_all: RwSignal<bool>,
    /// Limiter calculator, link group names, limit light (kept on the server).
    pub panel: RwSignal<PanelData>,
    /// Result of the last network search (None while none was answered).
    pub scan: RwSignal<Option<ScanResult>>,
    /// This browser tab only looks (no changes); see `read_only`.
    pub read_only: RwSignal<bool>,
    devices: StoredValue<HashMap<String, Device>>,
    next_toast: StoredValue<u64>,
}

/// One device found by the network search.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct FoundDsp {
    pub host: String,
    pub port: i64,
    #[serde(default)]
    pub info: String,
    #[serde(default)]
    pub model: Option<String>,
    /// Name of the configured DSP at this address.
    #[serde(default)]
    pub known: Option<String>,
    /// The panel is connected to it (it was not probed).
    #[serde(default)]
    pub connected: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScanResult {
    pub found: Vec<FoundDsp>,
    pub networks: Vec<String>,
}

pub fn app() -> App {
    APP.with(|a| a.get()).expect("app initialised")
}

impl App {
    pub fn init() -> App {
        let app = App {
            server: RwSignal::new(ServerState::Checking),
            auth_enabled: RwSignal::new(false),
            password_set: RwSignal::new(true),
            server_error: RwSignal::new(None),
            dsps: RwSignal::new(Vec::new()),
            loaded: RwSignal::new(false),
            selected: RwSignal::new(None),
            presets: RwSignal::new(Vec::new()),
            editors: RwSignal::new(Vec::new()),
            toasts: RwSignal::new(Vec::new()),
            mute_all: RwSignal::new(false),
            panel: RwSignal::new(PanelData::default()),
            scan: RwSignal::new(None),
            read_only: RwSignal::new(crate::read_only::starts_read_only()),
            devices: StoredValue::new(HashMap::new()),
            next_toast: StoredValue::new(0),
        };
        APP.with(|a| a.set(Some(app)));
        ROOT.with(|r| *r.borrow_mut() = Owner::current());
        app
    }

    pub fn device(&self, id: &str) -> Option<Device> {
        self.devices.with_value(|d| d.get(id).copied())
    }

    pub fn toast(&self, text: impl Into<String>, error: bool) {
        let text = text.into();
        // A refused fader drag would repeat the same message on every move.
        if self.toasts.with_untracked(|t| t.iter().any(|x| x.text == text)) {
            return;
        }
        let id = self.next_toast.get_value() + 1;
        self.next_toast.set_value(id);
        self.toasts.update(|t| t.push(Toast { id, text, error }));
        let toasts = self.toasts;
        set_timeout(move || toasts.update(|t| t.retain(|x| x.id != id)), Duration::from_secs(5));
    }

    /// Show [id] in the active tab: subscribe to it, unsubscribe the previous.
    pub fn select(&self, id: Option<String>) {
        let previous = self.selected.get_untracked();
        if previous == id {
            return;
        }
        if let Some(previous) = previous {
            send(json!({"t": msg::UNSUBSCRIBE, "dsp": previous}));
        }
        if let Some(id) = &id {
            send(json!({"t": msg::SUBSCRIBE, "dsp": id}));
        }
        self.selected.set(id);
    }

    // ─── DSP list ───

    pub fn add_dsp(&self, name: &str, host: &str, port: i64, model: &str, device_id: i64, password: &str) {
        send(json!({
            "t": msg::ADD_DSP, "name": name, "host": host, "port": port,
            "model": model, "deviceId": device_id, "password": password,
        }));
    }

    /// [password]: `None` keeps the stored one, "" removes it.
    #[allow(clippy::too_many_arguments)]
    pub fn update_dsp(
        &self,
        id: &str,
        name: &str,
        host: &str,
        port: i64,
        model: &str,
        device_id: i64,
        password: Option<&str>,
    ) {
        let mut message = json!({
            "t": msg::UPDATE_DSP, "id": id, "name": name, "host": host, "port": port,
            "model": model, "deviceId": device_id,
        });
        if let Some(password) = password {
            message["password"] = json!(password);
        }
        send(message);
    }

    pub fn remove_dsp(&self, id: &str) {
        send(json!({"t": msg::REMOVE_DSP, "id": id}));
    }

    /// Mute every channel of every DSP, or release (the server unmutes only
    /// what was open before).
    pub fn set_mute_all(&self, on: bool) {
        self.mute_all.set(on);
        send(json!({"t": msg::MUTE_ALL, "on": on}));
    }

    // ─── Panel data, several DSPs ───

    /// Search the LAN for DSPs; [networks] empty: the server's own network
    /// and those of the configured DSPs.
    pub fn scan_network(&self, networks: &[String]) {
        self.scan.set(None);
        let mut message = json!({"t": msg::SCAN});
        if !networks.is_empty() {
            message["networks"] = json!(networks);
        }
        send(message);
    }

    pub fn set_limiter_setup(&self, setup: LimiterSetup) {
        self.panel.update(|p| p.limiter = setup.clone());
        send(json!({"t": msg::LIMITER_SETUP, "setup": setup}));
    }

    /// Name a link group of [dsp] ("" removes the name).
    pub fn set_link_name(&self, dsp: &str, members: &[String], name: &str) {
        send(json!({"t": msg::LINK_NAME, "dsp": dsp, "members": members, "name": name}));
    }

    pub fn set_light_settings(&self, settings: LightSettings) {
        self.panel.update(|p| p.light = settings.clone());
        send(json!({"t": msg::LIGHT_SETTINGS, "settings": settings}));
    }

    /// Copy [parts] (api::COPY_PARTS) of [from] on [dsp] to [targets] (DSP id,
    /// channel), also on other DSPs.
    pub fn copy_to(&self, dsp: &str, from: &str, parts: &[String], targets: &[(String, String)]) {
        let targets: Vec<Value> = targets.iter().map(|(d, c)| json!({"dsp": d, "ch": c})).collect();
        send(json!({"t": msg::COPY_TO, "dsp": dsp, "from": from, "parts": parts, "targets": targets}));
    }

    /// Back to the values before the last preset load on [dsp].
    pub fn undo_preset_load(&self, dsp: &str) {
        send(json!({"t": msg::PRESET_UNDO, "dsp": dsp}));
    }

    // ─── Server presets ───

    pub fn presets_for(&self, model: &DeviceModel) -> Vec<ServerPresetInfo> {
        self.presets.with(|list| list.iter().filter(|p| p.model == model.id).cloned().collect())
    }

    pub fn save_preset_from_dsp(&self, dsp_id: &str, name: &str) {
        send(json!({"t": msg::PRESET_CREATE, "name": name, "fromDsp": dsp_id}));
    }

    pub fn create_server_preset(&self, name: &str, model: &DeviceModel) {
        send(json!({"t": msg::PRESET_CREATE, "name": name, "model": model.id}));
    }

    pub fn rename_server_preset(&self, preset_id: &str, name: &str) {
        send(json!({"t": msg::PRESET_RENAME, "preset": preset_id, "name": name}));
    }

    pub fn delete_server_preset(&self, preset_id: &str) {
        self.close_editor(preset_id);
        send(json!({"t": msg::PRESET_DELETE, "preset": preset_id}));
    }

    pub fn apply_server_preset(&self, preset_id: &str, dsp_id: &str) {
        send(json!({"t": msg::PRESET_APPLY, "preset": preset_id, "dsp": dsp_id}));
    }

    fn editor_info(key: &str, preset: &ServerPresetInfo) -> DspInfo {
        DspInfo {
            status: DspStatus::Ready,
            progress: 1.0,
            model: preset.model.clone(),
            ..DspInfo::new(key, &preset.name, "", 0, true)
        }
    }

    /// Open a server preset in an offline editor tab and show it.
    pub fn open_editor(&self, preset_id: &str) {
        let key = format!("{}{preset_id}", msg::PRESET_PREFIX);
        let Some(preset) = self.presets.with_untracked(|l| l.iter().find(|p| p.id == preset_id).cloned()) else {
            return;
        };
        if self.device(&key).is_none() {
            let device = in_root(|| Device::new(&key, true, Self::editor_info(&key, &preset)));
            self.devices.update_value(|d| {
                d.insert(key.clone(), device);
            });
        }
        if !self.editors.with_untracked(|e| e.contains(&key)) {
            self.editors.update(|e| e.push(key.clone()));
        }
        self.select(Some(key));
    }

    pub fn close_editor(&self, preset_id: &str) {
        let key = format!("{}{preset_id}", msg::PRESET_PREFIX);
        if !self.editors.with_untracked(|e| e.contains(&key)) {
            return;
        }
        self.editors.update(|e| e.retain(|k| *k != key));
        if self.selected.get_untracked().as_deref() == Some(&key) {
            let first = self.dsps.with_untracked(|d| d.first().cloned());
            self.select(first);
        }
    }

    // ─── Messages from the server ───

    pub fn on_message(&self, message: &Value) {
        let t = message.get("t").and_then(Value::as_str).unwrap_or_default();
        let device = message.get("dsp").and_then(Value::as_str).and_then(|id| self.device(id));
        match t {
            msg::PRESETS => {
                let list: Vec<ServerPresetInfo> = message
                    .get("presets")
                    .and_then(|p| serde_json::from_value(p.clone()).ok())
                    .unwrap_or_default();
                self.presets.set(list.clone());
                // Close editors of deleted presets, rename the others.
                for key in self.editors.get_untracked() {
                    let id = &key[msg::PRESET_PREFIX.len()..];
                    match list.iter().find(|p| p.id == id) {
                        None => self.close_editor(id),
                        Some(preset) => {
                            if let Some(d) = self.device(&key) {
                                d.info.set(Self::editor_info(&key, preset));
                            }
                        }
                    }
                }
            }
            msg::HELLO => {
                let infos: Vec<DspInfo> = message
                    .get("dsps")
                    .and_then(|d| serde_json::from_value(d.clone()).ok())
                    .unwrap_or_default();
                let ids: Vec<String> = infos.iter().map(|i| i.id.clone()).collect();
                for info in infos {
                    self.upsert(info);
                }
                self.dsps.set(ids);
                self.mute_all.set(message.get("muteAll") == Some(&Value::Bool(true)));
                if let Some(panel) = message.get("panel").and_then(|p| serde_json::from_value(p.clone()).ok()) {
                    self.panel.set(panel);
                }
                self.loaded.set(true);
                // After a reconnect, subscribe again to the DSP on screen.
                let selected = self.selected.get_untracked();
                self.selected.set(None);
                if let Some(selected) = selected {
                    let known = self.dsps.with_untracked(|d| d.contains(&selected))
                        || self.editors.with_untracked(|e| e.contains(&selected));
                    if known {
                        self.select(Some(selected));
                    }
                }
            }
            msg::DSP_INFO => {
                if let Some(info) = message.get("info").and_then(|i| serde_json::from_value::<DspInfo>(i.clone()).ok()) {
                    let id = info.id.clone();
                    self.upsert(info);
                    if !self.dsps.with_untracked(|d| d.contains(&id)) {
                        self.dsps.update(|d| d.push(id));
                    }
                }
            }
            msg::DSP_REMOVED => {
                let id = message.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                if self.selected.get_untracked().as_deref() == Some(&id) {
                    self.selected.set(None);
                }
                self.dsps.update(|d| d.retain(|x| *x != id));
            }
            msg::STATE => {
                if let (Some(d), Some(state)) = (device, message.get("state")) {
                    d.apply_state(state);
                }
            }
            msg::OP => {
                if let (Some(d), Some(op)) = (device, message.get("op")) {
                    d.apply_remote_op(op);
                }
            }
            msg::METERS => {
                let Some(d) = device else { return };
                let int = |k: &str| message.get(k).and_then(Value::as_i64).unwrap_or(0);
                if d.counters.get_untracked() != (int("tx"), int("rx")) {
                    d.counters.set((int("tx"), int("rx")));
                }
                let levels: Vec<f64> = message
                    .get("v")
                    .and_then(Value::as_array)
                    .map(|v| v.iter().filter_map(Value::as_f64).collect())
                    .unwrap_or_default();
                d.apply_meters(&levels, int("dyn") as u16);
            }
            msg::LOG => {
                let Some(d) = device else { return };
                let entries: Vec<LogEntry> = message
                    .get("entries")
                    .and_then(Value::as_array)
                    .map(|e| e.iter().filter_map(|x| LogEntry::from_json(x).ok()).collect())
                    .unwrap_or_default();
                let reset = message.get("reset") == Some(&Value::Bool(true));
                d.log.update(|log| {
                    if reset {
                        log.clear();
                    }
                    log.extend(entries);
                    let excess = log.len().saturating_sub(MAX_DEBUG_ENTRIES);
                    log.drain(..excess);
                });
            }
            msg::MUTE_ALL => {
                let on = message.get("on") == Some(&Value::Bool(true));
                if self.mute_all.get_untracked() != on {
                    self.mute_all.set(on);
                }
            }
            msg::ERROR => {
                let text = message.get("message").and_then(Value::as_str).unwrap_or("Error");
                self.toast(text, true);
            }
            msg::NOTICE => {
                let text = message.get("message").and_then(Value::as_str).unwrap_or_default();
                let name = device.map(|d| d.info.with_untracked(|i| i.name.clone())).unwrap_or_default();
                let text = if name.is_empty() { text.to_string() } else { format!("{name}: {text}") };
                self.toast(text, message.get("error") == Some(&Value::Bool(true)));
            }
            msg::PANEL => {
                if let Some(panel) = message.get("data").and_then(|p| serde_json::from_value::<PanelData>(p.clone()).ok())
                    && self.panel.with_untracked(|p| *p != panel)
                {
                    self.panel.set(panel);
                }
            }
            msg::SCAN_RESULT => {
                let found: Vec<FoundDsp> =
                    message.get("found").and_then(|f| serde_json::from_value(f.clone()).ok()).unwrap_or_default();
                let networks: Vec<String> =
                    message.get("networks").and_then(|n| serde_json::from_value(n.clone()).ok()).unwrap_or_default();
                self.scan.set(Some(ScanResult { found, networks }));
            }
            _ => {}
        }
    }

    fn upsert(&self, mut info: DspInfo) {
        match self.device(&info.id) {
            Some(d) => {
                // Keep the test level the user is dragging.
                if info.test_level.is_some() && d.pending_test_level.get_value().is_some() {
                    info.test_level = d.info.with_untracked(|i| i.test_level);
                }
                if d.info.with_untracked(|i| *i != info) {
                    d.info.set(info);
                }
            }
            None => {
                let device = in_root(|| Device::new(&info.id.clone(), false, info));
                self.devices.update_value(|d| {
                    d.insert(device.key.get_value(), device);
                });
            }
        }
    }
}

pub const MAX_DEBUG_ENTRIES: usize = 500;

/// Edits of a PEQ band (unset fields keep their value).
#[derive(Clone, Copy, Debug, Default)]
pub struct PeqEdit {
    pub gain_db: Option<f64>,
    pub freq_raw: Option<i64>,
    pub q_raw: Option<i64>,
    pub peq_type: Option<i64>,
    pub bypass: Option<bool>,
}

/// One DSP (or a server preset in the offline editor).
#[derive(Clone, Copy)]
pub struct Device {
    pub key: StoredValue<String>,
    /// A server preset in the offline editor (no device): linking and copying
    /// change the state directly instead of running on the device.
    pub offline: bool,
    pub info: RwSignal<DspInfo>,
    pub state: RwSignal<DeviceState>,
    pub log: RwSignal<Vec<LogEntry>>,
    pub counters: RwSignal<(i64, i64)>,
    pub show_decoded: RwSignal<bool>,
    /// Last active slope per filter ("hpf:In A"), restored when a filter is
    /// switched on again.
    pub last_slope: RwSignal<HashMap<String, i64>>,
    pub meters: StoredValue<MeterBank, LocalStorage>,
    outgoing: StoredValue<Vec<(String, Op)>>,
    flush_scheduled: StoredValue<bool>,
    /// Test level waiting to be sent (coalesced like the ops).
    pending_test_level: StoredValue<Option<f64>>,
}

impl Device {
    fn new(key: &str, offline: bool, info: DspInfo) -> Self {
        let model = DeviceModel::by_id(Some(&info.model));
        Device {
            key: StoredValue::new(key.to_string()),
            offline,
            info: RwSignal::new(info),
            state: RwSignal::new(DeviceState::new(model)),
            log: RwSignal::new(Vec::new()),
            counters: RwSignal::new((0, 0)),
            show_decoded: RwSignal::new(false),
            last_slope: RwSignal::new(HashMap::new()),
            meters: StoredValue::new_local(MeterBank::new(&model)),
            outgoing: StoredValue::new(Vec::new()),
            flush_scheduled: StoredValue::new(false),
            pending_test_level: StoredValue::new(None),
        }
    }

    pub fn id(&self) -> String {
        self.key.get_value()
    }

    pub fn model_untracked(&self) -> DeviceModel {
        self.state.with_untracked(|s| s.model)
    }

    /// Display name of a channel: the name stored on the device, or the label
    /// if the device still uses its default name ("InA").
    pub fn alias(&self, channel: &str) -> String {
        self.state.with(|s| alias_in(s, channel))
    }

    /// Ready for commands (socket connected and not loading).
    pub fn is_ready(&self) -> bool {
        self.offline || self.info.with_untracked(|i| i.status == DspStatus::Ready)
    }

    pub fn is_connected(&self) -> bool {
        self.offline || self.info.with(|i| i.status == DspStatus::Ready)
    }

    /// The DSP test generator runs (only mutes and the test controls are
    /// accepted meanwhile).
    pub fn test_running(&self) -> bool {
        !self.offline && self.info.with_untracked(|i| i.test_level.is_some())
    }

    // ─── Changes ───

    /// Apply an op locally and queue it for the server. Ops for the same
    /// [key] are coalesced: only the newest is sent.
    pub fn change(&self, key: &str, op: Op) {
        if !self.is_ready() {
            return;
        }
        let model = self.model_untracked();
        if ops::validate(&op.to_json(), &model).is_some() {
            return;
        }
        if self.test_running() && !ops::allowed_during_test(&op) {
            app().toast(TEST_RUNNING, true);
            return;
        }
        // Linked channels follow locally right away (by the original editor's
        // rule); the server does the same, so only the original op is sent.
        self.state.update(|s| {
            for expanded in ops::expand_linked(s, &op) {
                ops::apply(s, &expanded);
            }
        });
        self.outgoing.update_value(|out| match out.iter_mut().find(|(k, _)| k == key) {
            Some(entry) => entry.1 = op,
            None => out.push((key.to_string(), op)),
        });
        if !self.flush_scheduled.get_value() {
            self.flush_scheduled.set_value(true);
            let device = *self;
            set_timeout(move || device.flush(), Duration::from_millis(30));
        }
    }

    fn flush(&self) {
        self.flush_scheduled.set_value(false);
        let id = self.id();
        let pending = self.outgoing.try_update_value(std::mem::take).unwrap_or_default();
        for (_, op) in pending {
            send(json!({"t": msg::OP, "dsp": id, "op": op.to_json()}));
        }
    }

    /// Send queued changes now (before a link or copy job on the server).
    fn flush_now(&self) {
        self.flush();
    }

    /// Set a gain within the channel's max level, like the server does.
    pub fn set_gain(&self, channel: &str, db: f64) {
        let mut db = p::quantize_gain(db.clamp(-60.0, 12.0));
        if let Some(limit) = self.gain_limit(channel) {
            db = db.min(limit);
        }
        self.change(&format!("gain:{channel}"), Op::gain(channel, db));
    }

    /// Set a gain above the channel's max level (confirmed by the user): sent
    /// right away with the force flag.
    pub fn set_gain_forced(&self, channel: &str, db: f64) {
        if !self.is_ready() || self.test_running() {
            return;
        }
        let op = Op::gain(channel, p::quantize_gain(db.clamp(-60.0, 12.0)));
        if ops::validate(&op.to_json(), &self.model_untracked()).is_some() {
            return;
        }
        let key = format!("gain:{channel}");
        self.outgoing.update_value(|out| out.retain(|(k, _)| *k != key));
        self.state.update(|s| {
            for expanded in ops::expand_linked(s, &op) {
                ops::apply(s, &expanded);
            }
        });
        send(json!({"t": msg::OP, "dsp": self.id(), "op": op.to_json(), "force": true}));
    }

    /// Max level of [channel] (shared by its link group), if it has one.
    pub fn gain_cap(&self, channel: &str) -> Option<f64> {
        self.info.with(|i| self.state.with(|s| ops::group_cap(s, &i.gain_caps, channel)))
    }

    /// Highest gain the panel sets for [channel] without asking: max(cap,
    /// current gain), or `None` without a cap.
    pub fn gain_limit(&self, channel: &str) -> Option<f64> {
        let cap = self.info.with_untracked(|i| self.state.with_untracked(|s| ops::group_cap(s, &i.gain_caps, channel)))?;
        let current = self.state.with_untracked(|s| s.gains.get(channel).copied().unwrap_or(0.0));
        Some(cap.max(current))
    }

    /// Change the max level of [channel] and its link group (`None` removes
    /// it); stored on the server.
    pub fn set_gain_cap(&self, channel: &str, db: Option<f64>) {
        send(json!({"t": msg::GAIN_CAP, "dsp": self.id(), "ch": channel, "db": db}));
    }

    /// Level of all outputs while the DSP test generator runs (sent at most
    /// every 30 ms).
    pub fn set_test_level(&self, db: f64) {
        let db = p::quantize_gain(db.clamp(TEST_LEVEL_MIN_DB, TEST_LEVEL_MAX_DB));
        if !self.test_running() || !self.is_ready() {
            return;
        }
        self.info.update(|i| i.test_level = Some(db));
        let scheduled = self.pending_test_level.get_value().is_some();
        self.pending_test_level.set_value(Some(db));
        if !scheduled {
            let device = *self;
            set_timeout(
                move || {
                    if let Some(db) = device.pending_test_level.try_update_value(Option::take).flatten() {
                        send(json!({"t": msg::TEST_LEVEL, "dsp": device.id(), "db": db}));
                    }
                },
                Duration::from_millis(30),
            );
        }
    }

    pub fn toggle_mute(&self, channel: &str) {
        let Some(on) = self.state.with_untracked(|s| s.mutes.get(channel).copied()) else { return };
        self.change(&format!("mute:{channel}"), Op::mute(channel, !on));
    }

    pub fn set_mute(&self, channel: &str, on: bool) {
        self.change(&format!("mute:{channel}"), Op::mute(channel, on));
    }

    pub fn toggle_phase(&self, channel: &str) {
        let Some(on) = self.state.with_untracked(|s| s.phase.get(channel).copied()) else { return };
        self.change(&format!("phase:{channel}"), Op::phase(channel, !on));
    }

    /// Set delay in milliseconds (0 – 680 ms, 1/96 ms steps).
    pub fn set_delay_ms(&self, channel: &str, ms: f64) {
        self.change(&format!("delay:{channel}"), Op::delay(channel, p::delay_ms_to_raw(ms)));
    }

    pub fn toggle_matrix_input(&self, output: &str, input: &str) {
        let (mask, bit) = self.state.with_untracked(|s| {
            (s.matrix_routing.get(output).copied().unwrap_or(0), s.model.matrix_bit(input) as i64)
        });
        self.change(&format!("matrix:{output}"), Op::matrix_route(output, mask ^ bit));
    }

    pub fn set_matrix_gain(&self, output: &str, input: &str, db: f64) {
        self.change(&format!("mixgain:{output}:{input}"), Op::matrix_gain(output, input, db));
    }

    pub fn set_geq_band(&self, channel: &str, band: usize, db: f64) {
        self.change(&format!("geq:{channel}:{band}"), Op::geq_band(channel, band, db));
    }

    pub fn reset_geq(&self, channel: &str) {
        let prefix = format!("geq:{channel}:");
        self.outgoing.update_value(|out| out.retain(|(k, _)| !k.starts_with(&prefix)));
        self.change(&format!("geqReset:{channel}"), Op::geq_reset(channel));
    }

    /// Set PEQ band parameters. Like the original editor, shelves and the PEQ
    /// low/high-pass types start at Q 0.71 and go up to Q 3 only.
    pub fn set_peq_band(&self, channel: &str, band: usize, edit: PeqEdit) {
        let Some(mut b) = self.state.with_untracked(|s| s.peq_bands.get(channel).and_then(|l| l.get(band)).cloned())
        else {
            return;
        };
        if let Some(gain) = edit.gain_db {
            b.gain_db = gain.clamp(-12.0, 12.0);
        }
        if let Some(freq) = edit.freq_raw {
            b.freq_raw = freq.clamp(0, p::FREQ_RAW_MAX);
        }
        if let Some(q) = edit.q_raw {
            b.q_raw = q;
        }
        if let Some(t) = edit.peq_type
            && t != b.peq_type {
                b.peq_type = t;
                if p::max_q_raw_for_type(t) < p::Q_RAW_MAX {
                    b.q_raw = p::Q_RAW_TYPE_RESET;
                }
            }
        b.q_raw = b.q_raw.clamp(0, p::max_q_raw_for_type(b.peq_type));
        if let Some(bypass) = edit.bypass {
            b.bypass = bypass;
        }
        self.change(&format!("peq:{channel}:{band}"), Op::peq_band(channel, band, b));
    }

    pub fn set_eq_bypass(&self, channel: &str, on: bool) {
        self.change(&format!("eqBypass:{channel}"), Op::eq_bypass(channel, on));
    }

    pub fn set_geq_bypass(&self, channel: &str, on: bool) {
        self.change(&format!("geqBypass:{channel}"), Op::geq_bypass(channel, on));
    }

    pub fn set_delay_unit(&self, unit: i64) {
        self.change("delayUnit", Op::delay_unit(unit));
    }

    fn filter_key(channel: &str, high_pass: bool) -> String {
        format!("{}:{channel}", if high_pass { "hpf" } else { "lpf" })
    }

    pub fn filter(&self, channel: &str, high_pass: bool) -> FilterState {
        self.state.with(|s| filter_of(s, channel, high_pass))
    }

    /// Set the hi pass ([high_pass]) or lo pass. [enabled] switches between
    /// bypass and the last active slope.
    pub fn set_filter(&self, channel: &str, high_pass: bool, freq_raw: Option<i64>, enabled: Option<bool>, slope: Option<i64>) {
        let key = Self::filter_key(channel, high_pass);
        let mut f = self.state.with_untracked(|s| filter_of(s, channel, high_pass));
        if f.slope != 0 {
            self.last_slope.update(|m| {
                m.insert(key.clone(), f.slope);
            });
        }
        if let Some(freq) = freq_raw {
            f.freq_raw = freq.clamp(0, p::FREQ_RAW_MAX);
        }
        if let Some(slope) = slope {
            f.slope = slope;
        }
        if let Some(enabled) = enabled {
            let last = self.last_slope.with_untracked(|m| m.get(&key).copied()).unwrap_or(p::DEFAULT_SLOPE);
            f.slope = if enabled { last } else { 0 };
        }
        let op = if high_pass { Op::hi_pass(channel, f) } else { Op::lo_pass(channel, f) };
        self.change(&key, op);
    }

    /// Slope of a filter: the active one, or the one it gets when it is
    /// switched on again.
    pub fn filter_slope(&self, channel: &str, high_pass: bool) -> i64 {
        let f = self.filter(channel, high_pass);
        if f.slope != 0 {
            return f.slope;
        }
        let key = Self::filter_key(channel, high_pass);
        self.last_slope.with(|m| m.get(&key).copied()).unwrap_or(p::DEFAULT_SLOPE)
    }

    /// Choose the filter curve. A bypassed filter stays bypassed and uses the
    /// curve when it is switched on.
    pub fn set_filter_slope(&self, channel: &str, slope: i64, high_pass: bool) {
        let f = self.state.with_untracked(|s| filter_of(s, channel, high_pass));
        if f.slope == 0 {
            let key = Self::filter_key(channel, high_pass);
            self.last_slope.update(|m| {
                m.insert(key, slope);
            });
        } else {
            self.set_filter(channel, high_pass, None, None, Some(slope));
        }
    }

    pub fn set_test_tone(&self, tone_type: i64, param: Option<i64>) {
        let param = param.unwrap_or_else(|| self.state.with_untracked(|s| s.test_tone_param));
        self.change("testTone", Op::test_tone(tone_type, param));
    }

    // Values are kept in the original editor's ranges.
    pub fn set_gate(&self, channel: &str, gate: GateState) {
        self.change(&format!("gate:{channel}"), Op::gate(channel, ops::clamp_gate(&gate)));
    }

    pub fn set_compressor(&self, channel: &str, c: CompressorState) {
        self.change(&format!("comp:{channel}"), Op::compressor(channel, ops::clamp_compressor(&c)));
    }

    pub fn set_limiter(&self, channel: &str, l: LimiterState) {
        self.change(&format!("limit:{channel}"), Op::limiter(channel, ops::clamp_limiter(&l)));
    }

    /// Set the channel name stored on the device; empty restores the default.
    pub fn set_alias(&self, channel: &str, alias: &str) {
        let name = alias.trim();
        let op = Op::channel_name(channel, if name.is_empty() { None } else { Some(name) });
        self.change(&format!("name:{channel}"), op);
    }

    // ─── Linking and copying ───

    /// Make [slaves] the slaves of [master] (empty = unlink its slaves), like
    /// the original editor. On a DSP the server copies on the device (0x2A),
    /// sets the master's mask (0x3B) and reads everything back.
    pub fn set_link_slaves(&self, master: &str, slaves: Vec<String>) {
        if !self.is_ready() {
            return;
        }
        if self.offline {
            let list = self.state.with_untracked(|s| ops::link_slaves(s, master, &slaves));
            match list {
                Ok(list) => {
                    for op in list {
                        self.change(&op_key(&op), op);
                    }
                }
                Err(e) => app().toast(e, true),
            }
            return;
        }
        self.flush_now();
        send(json!({"t": msg::LINK, "dsp": self.id(), "master": master, "slaves": slaves}));
    }

    /// Set a raw link mask (to lift a link for a measurement and restore it).
    pub fn set_link_mask(&self, channel: &str, mask: i64) {
        self.change(&format!("link:{channel}"), Op::link(channel, mask));
    }

    pub fn link_mask(&self, channel: &str) -> i64 {
        self.state.with_untracked(|s| s.link_mask(channel))
    }

    /// Remove [channel] from its link group.
    pub fn unlink(&self, channel: &str) {
        let (master, slaves) = self.state.with_untracked(|s| ops::unlink_target(s, channel));
        self.set_link_slaves(&master, slaves);
    }

    /// Copy the settings of [from] to each of [targets] (same group). On a DSP
    /// the device copies (0x2A) and the server reads the result back.
    pub fn copy_channel(&self, from: &str, targets: Vec<String>) {
        if !self.is_ready() {
            return;
        }
        if self.offline {
            for to in &targets {
                let list = self.state.with_untracked(|s| ops::copy_channel(s, from, to));
                if let Ok(list) = list {
                    for op in list {
                        self.change(&op_key(&op), op);
                    }
                }
            }
            return;
        }
        self.flush_now();
        send(json!({"t": msg::COPY, "dsp": self.id(), "from": from, "to": targets}));
    }

    // ─── Connection (server side) ───

    pub fn connect(&self) {
        send(json!({"t": msg::UPDATE_DSP, "id": self.id(), "enabled": true}));
    }

    pub fn disconnect(&self) {
        send(json!({"t": msg::UPDATE_DSP, "id": self.id(), "enabled": false}));
    }

    pub fn load_preset(&self, index: usize) {
        if self.is_ready() {
            send(json!({"t": msg::LOAD_PRESET, "dsp": self.id(), "index": index}));
        }
    }

    pub fn save_preset(&self, index: usize, name: &str) {
        if self.is_ready() {
            send(json!({"t": msg::SAVE_PRESET, "dsp": self.id(), "index": index, "name": name}));
        }
    }

    pub fn set_refresh(&self, ms: i64) {
        send(json!({"t": msg::SET_REFRESH, "dsp": self.id(), "ms": ms}));
    }

    pub fn toggle_rx(&self) {
        let on = self.info.with_untracked(|i| i.log_rx);
        send(json!({"t": msg::DEBUG, "dsp": self.id(), "rx": !on}));
    }

    pub fn toggle_tx(&self) {
        let on = self.info.with_untracked(|i| i.log_tx);
        send(json!({"t": msg::DEBUG, "dsp": self.id(), "tx": !on}));
    }

    pub fn clear_log(&self) {
        send(json!({"t": msg::CLEAR_LOG, "dsp": self.id()}));
    }

    // ─── Updates from the server ───

    /// Replace the whole state (after subscribing, init or preset load); keep
    /// local changes that have not been sent yet.
    fn apply_state(&self, json: &Value) {
        let Ok(mut next) = DeviceState::from_json(json) else { return };
        if next.model != self.model_untracked() {
            self.outgoing.set_value(Vec::new());
            self.meters.update_value(|m| m.reset(&next.model));
        }
        for (_, op) in self.outgoing.get_value() {
            if ops::validate(&op.to_json(), &next.model).is_none() {
                for expanded in ops::expand_linked(&next, &op) {
                    ops::apply(&mut next, &expanded);
                }
            }
        }
        self.state.set(next);
    }

    /// A change made by another client. The server sends linked channels as
    /// separate ops, so no expansion here.
    fn apply_remote_op(&self, json: &Value) {
        let model = self.model_untracked();
        if let Ok(op) = Op::from_json(json, &model) {
            self.state.update(|s| ops::apply(s, &op));
        }
    }

    fn apply_meters(&self, levels: &[f64], dynamics: u16) {
        let state = self.state;
        self.meters.update_value(|m| state.with_untracked(|s| m.apply(levels, dynamics, s)));
    }
}

/// Coalescing key of an op (like the Flutter client's _keyFor).
fn op_key(op: &Op) -> String {
    let json = op.to_json();
    ["op", "ch", "out", "in", "band"]
        .iter()
        .filter_map(|k| json.get(*k).filter(|v| !v.is_null()).map(|v| v.as_str().map_or(v.to_string(), String::from)))
        .collect::<Vec<_>>()
        .join(":")
}

pub fn alias_in(s: &DeviceState, channel: &str) -> String {
    match s.channel_names.get(channel) {
        Some(name) if *name != s.model.default_device_name(channel) => name.clone(),
        _ => channel.to_string(),
    }
}

pub fn filter_of(s: &DeviceState, channel: &str, high_pass: bool) -> FilterState {
    let map = if high_pass { &s.hi_pass } else { &s.lo_pass };
    map.get(channel).cloned().unwrap_or(if high_pass { FilterState::new(0, 0) } else { FilterState::new(300, 0) })
}
