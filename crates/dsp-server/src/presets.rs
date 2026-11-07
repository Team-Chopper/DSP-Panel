//! Presets kept on the server (in addition to the 20 slots on each DSP).
//! A preset is a complete device state of one model. It can be edited offline
//! — the panel addresses it like a DSP with the id "preset:<id>" — and loaded
//! onto any DSP of the same model. presets.json has the format of the Dart
//! server.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dsp_core::ops::{self, Op};
use dsp_core::{DeviceState, ServerPresetInfo};
use indexmap::IndexMap;
use serde_json::{Value, json};
use tokio::task::spawn_local;

use crate::log;
use crate::registry::{DspConfig, write_atomic};

pub const MAX_PRESETS: usize = 500;

pub struct ServerPreset {
    pub id: String,
    pub name: String,
    pub state: DeviceState,
    pub updated: i64,
}

impl ServerPreset {
    pub fn info(&self) -> ServerPresetInfo {
        ServerPresetInfo {
            id: self.id.clone(),
            name: self.name.clone(),
            model: self.state.model.id.into(),
            updated: self.updated,
        }
    }

    fn to_json(&self) -> Value {
        json!({"id": self.id, "name": self.name, "updated": self.updated, "state": self.state.to_json()})
    }

    fn from_json(json: &Value) -> Result<Self, String> {
        let text = |k: &str| json.get(k).and_then(Value::as_str).map(String::from).ok_or(format!("{k} missing"));
        Ok(ServerPreset {
            id: text("id")?,
            name: text("name")?,
            state: DeviceState::from_json(json.get("state").ok_or("state missing")?)?,
            updated: json
                .get("updated")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
                .ok_or("updated missing")?,
        })
    }
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64
}

struct StoreInner {
    file: PathBuf,
    presets: IndexMap<String, ServerPreset>,
    save_generation: u64,
}

/// Handle to the preset store (single-threaded).
#[derive(Clone)]
pub struct PresetStore(Rc<RefCell<StoreInner>>);

impl PresetStore {
    pub fn new(file: PathBuf) -> Self {
        PresetStore(Rc::new(RefCell::new(StoreInner { file, presets: IndexMap::new(), save_generation: 0 })))
    }

    pub fn list(&self) -> Vec<ServerPresetInfo> {
        let mut list: Vec<ServerPresetInfo> = self.0.borrow().presets.values().map(ServerPreset::info).collect();
        list.sort_by_key(|p| p.name.to_lowercase());
        list
    }

    pub fn len(&self) -> usize {
        self.0.borrow().presets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn contains(&self, id: &str) -> bool {
        self.0.borrow().presets.contains_key(id)
    }

    /// Name and a copy of the state of a preset.
    pub fn get(&self, id: &str) -> Option<(String, DeviceState)> {
        self.0.borrow().presets.get(id).map(|p| (p.name.clone(), p.state.clone()))
    }

    pub fn state_json(&self, id: &str) -> Option<Value> {
        self.0.borrow().presets.get(id).map(|p| p.state.to_json())
    }

    pub fn load(&self) {
        let mut inner = self.0.borrow_mut();
        if !inner.file.exists() {
            return;
        }
        let parsed = std::fs::read_to_string(&inner.file)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()));
        let json = match parsed {
            Ok(json) => json,
            Err(e) => {
                log::warn(format!("Could not read {}: {e}", inner.file.display()));
                return;
            }
        };
        for p in json.get("presets").and_then(Value::as_array).into_iter().flatten() {
            match ServerPreset::from_json(p) {
                Ok(preset) => {
                    inner.presets.insert(preset.id.clone(), preset);
                }
                Err(e) => log::warn(format!("Skipping unreadable preset in {}: {e}", inner.file.display())),
            }
        }
    }

    pub fn validate_name(name: Option<&Value>) -> Option<String> {
        match name.and_then(Value::as_str) {
            Some(n) if !n.trim().is_empty() && n.trim().encode_utf16().count() <= 40 => None,
            _ => Some("Preset name must be 1–40 characters".into()),
        }
    }

    /// Create a preset from [state] (copied). Returns its name and model.
    pub fn create(&self, name: &str, state: &DeviceState) -> Result<(String, &'static str), String> {
        let mut inner = self.0.borrow_mut();
        if inner.presets.len() >= MAX_PRESETS {
            return Err(format!("At most {MAX_PRESETS} presets"));
        }
        let mut copy = DeviceState::from_json(&state.to_json())?;
        copy.current_preset = name.trim().to_string();
        let preset = ServerPreset { id: DspConfig::new_id(), name: name.trim().to_string(), state: copy, updated: now_ms() };
        let result = (preset.name.clone(), preset.state.model.name);
        inner.presets.insert(preset.id.clone(), preset);
        drop(inner);
        self.schedule_save();
        Ok(result)
    }

    pub fn rename(&self, id: &str, name: Option<&Value>) -> Option<String> {
        if !self.contains(id) {
            return Some("Unknown preset".into());
        }
        if let Some(error) = Self::validate_name(name) {
            return Some(error);
        }
        let name = name.and_then(Value::as_str).unwrap_or_default().trim().to_string();
        if let Some(preset) = self.0.borrow_mut().presets.get_mut(id) {
            preset.state.current_preset = name.clone();
            preset.name = name;
            preset.updated = now_ms();
        }
        self.schedule_save();
        None
    }

    pub fn delete(&self, id: &str) -> Option<String> {
        if self.0.borrow_mut().presets.shift_remove(id).is_none() {
            return Some("Unknown preset".into());
        }
        self.schedule_save();
        None
    }

    /// Apply an op from the offline editor (linked channels included).
    /// Returns the applied ops (to send to the other clients) or an error.
    pub fn apply_op(&self, id: &str, op: &Value) -> Result<Vec<Op>, String> {
        let applied = {
            let mut inner = self.0.borrow_mut();
            let Some(preset) = inner.presets.get_mut(id) else {
                return Err("Unknown preset".into());
            };
            if let Some(error) = ops::validate(op, &preset.state.model) {
                return Err(error);
            }
            let op = Op::parse(op)?;
            let expanded = ops::expand_linked(&preset.state, &op);
            for o in &expanded {
                ops::apply(&mut preset.state, o);
            }
            preset.updated = now_ms();
            expanded
        };
        self.schedule_save();
        Ok(applied)
    }

    fn schedule_save(&self) {
        let generation = {
            let mut inner = self.0.borrow_mut();
            inner.save_generation += 1;
            inner.save_generation
        };
        let store = self.clone();
        spawn_local(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if store.0.borrow().save_generation == generation {
                store.save();
            }
        });
    }

    pub fn save(&self) {
        let mut inner = self.0.borrow_mut();
        inner.save_generation += 1; // cancels a scheduled save
        let json = json!({"presets": inner.presets.values().map(ServerPreset::to_json).collect::<Vec<_>>()});
        if let Err(e) = write_atomic(&inner.file, &json.to_string()) {
            log::warn(format!("Could not save {}: {e}", inner.file.display()));
        }
    }
}
