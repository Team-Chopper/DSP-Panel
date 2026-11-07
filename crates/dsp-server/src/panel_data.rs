//! Panel data the server keeps for every client (the DSPs do not know it), in
//! panel.json in the data directory:
//!
//! ```json
//! {
//!   "limiter": {"amps": [...], "outputs": {"<dsp id>/Out 1": {...}}, "referenceDbu": {}},
//!   "linkNames": {"<dsp id>": {"Out 1+Out 2": "Tops"}},
//!   "light": {"enabled": false, "ignore": {"<dsp id>": ["Out 5"]}}
//! }
//! ```
//!
//! - `limiter`: amplifiers and the amplifier channel behind each output, for
//!   the limiter calculator (dsp_core::limiter_calc).
//! - `linkNames`: names of link groups, keyed by their members in channel
//!   order; a changed group shows no name, the same group formed again gets
//!   it back.
//! - `light`: the limit light page (/limit, without login) and the outputs it
//!   does not watch.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use dsp_core::limiter_calc::LimiterSetup;
use dsp_core::panel::{LINK_NAME_MAX, LightSettings, PanelData, group_key};
use serde_json::Value;

use crate::log;
use crate::registry::write_atomic;

#[derive(Default)]
struct Inner {
    file: Option<PathBuf>,
    data: PanelData,
}

/// Handle to the panel data (single-threaded, owned by the hub).
#[derive(Clone, Default)]
pub struct PanelStore(Rc<RefCell<Inner>>);

/// Names kept per DSP (they stay with their members while the group is
/// dissolved).
const MAX_LINK_NAMES: usize = 32;

impl PanelStore {
    pub fn new(file: PathBuf) -> Self {
        let store = PanelStore(Rc::new(RefCell::new(Inner { file: Some(file), ..Inner::default() })));
        store.load();
        store
    }

    /// A store that is not written to disk.
    pub fn memory() -> Self {
        PanelStore::default()
    }

    fn load(&self) {
        let Some(file) = self.0.borrow().file.clone() else { return };
        if !file.exists() {
            return;
        }
        let data = std::fs::read_to_string(&file)
            .map_err(|e| e.to_string())
            .and_then(|text| serde_json::from_str::<PanelData>(&text).map_err(|e| e.to_string()));
        match data {
            Ok(mut data) => {
                data.limiter = data.limiter.validated().unwrap_or_default();
                self.0.borrow_mut().data = data;
            }
            Err(e) => log::warn(format!("Could not read {}: {e}", file.display())),
        }
    }

    fn save(&self) {
        let i = self.0.borrow();
        let Some(file) = &i.file else { return };
        if let Err(e) = write_atomic(file, &serde_json::to_string_pretty(&i.data).unwrap_or_default()) {
            log::warn(format!("Could not save {}: {e}", file.display()));
        }
    }

    pub fn data(&self) -> PanelData {
        self.0.borrow().data.clone()
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(&self.0.borrow().data).unwrap_or(Value::Null)
    }

    pub fn light(&self) -> LightSettings {
        self.0.borrow().data.light.clone()
    }

    /// Replace the limiter calculator setup (validated).
    pub fn set_limiter(&self, setup: &Value) -> Result<(), String> {
        let setup: LimiterSetup = serde_json::from_value(setup.clone()).map_err(|_| "Invalid limiter data".to_string())?;
        self.0.borrow_mut().data.limiter = setup.validated()?;
        self.save();
        Ok(())
    }

    /// Name a link group of [dsp] (an empty name removes it).
    pub fn set_link_name(&self, dsp: &str, members: &[String], name: &str) -> Result<(), String> {
        if members.len() < 2 {
            return Err("A link group has at least two channels".into());
        }
        let name: String = name.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string();
        if name.chars().count() > LINK_NAME_MAX {
            return Err(format!("At most {LINK_NAME_MAX} characters"));
        }
        {
            let mut i = self.0.borrow_mut();
            let names = i.data.link_names.entry(dsp.to_string()).or_default();
            if name.is_empty() {
                names.remove(&group_key(members));
            } else {
                if names.len() >= MAX_LINK_NAMES && !names.contains_key(&group_key(members)) {
                    return Err(format!("At most {MAX_LINK_NAMES} group names per DSP"));
                }
                names.insert(group_key(members), name);
            }
            i.data.link_names.retain(|_, n| !n.is_empty());
        }
        self.save();
        Ok(())
    }

    pub fn set_light(&self, settings: &Value) -> Result<(), String> {
        let mut light: LightSettings =
            serde_json::from_value(settings.clone()).map_err(|_| "Invalid light settings".to_string())?;
        light.ignore.retain(|k, v| k.len() <= 40 && !v.is_empty());
        for list in light.ignore.values_mut() {
            list.retain(|c| c.len() <= 8);
            list.truncate(16);
        }
        self.0.borrow_mut().data.light = light;
        self.save();
        Ok(())
    }

    /// Forget everything about a removed DSP.
    pub fn remove_dsp(&self, dsp: &str) {
        {
            let mut i = self.0.borrow_mut();
            let prefix = format!("{dsp}/");
            i.data.limiter.outputs.retain(|k, _| !k.starts_with(&prefix));
            i.data.limiter.reference_dbu.remove(dsp);
            i.data.link_names.remove(dsp);
            i.data.light.ignore.remove(dsp);
        }
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn members(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn link_names_follow_their_groups() {
        let store = PanelStore::memory();
        store.set_link_name("d", &members(&["Out 1", "Out 2"]), " Tops ").unwrap();
        store.set_link_name("d", &members(&["Out 3", "Out 4"]), "Subs").unwrap();
        assert!(store.set_link_name("d", &members(&["Out 5"]), "x").is_err());
        assert!(store.set_link_name("d", &members(&["Out 5", "Out 6"]), "a name that is too long").is_err());
        assert_eq!(store.data().link_name("d", &members(&["Out 1", "Out 2"])), Some("Tops"));
        assert_eq!(store.data().link_name("d", &members(&["Out 1", "Out 2", "Out 3"])), None);
        store.set_link_name("d", &members(&["Out 1", "Out 2"]), "").unwrap();
        store.set_link_name("d", &members(&["Out 3", "Out 4"]), "").unwrap();
        assert!(store.data().link_names.is_empty());
    }

    #[test]
    fn saved_and_loaded() {
        let dir = std::env::temp_dir().join(format!("panel-data-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("panel.json");
        let store = PanelStore::new(file.clone());
        store.set_light(&json!({"enabled": true, "ignore": {"d": ["Out 5"], "e": []}})).unwrap();
        let amps = json!({"amps": [{"id": "a", "name": "Amp", "watts8": 500.0, "watts4": 800.0, "watts2": 0.0,
            "sensitivityV": 1.4}], "outputs": {"d/Out 1": {"amp": "a", "ohms": 8.0, "speakerW": 400.0}}});
        store.set_limiter(&amps).unwrap();
        let loaded = PanelStore::new(file);
        assert_eq!(loaded.data(), store.data());
        assert!(loaded.light().enabled);
        assert_eq!(loaded.light().ignore.len(), 1);
        loaded.remove_dsp("d");
        assert!(loaded.data().limiter.outputs.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
