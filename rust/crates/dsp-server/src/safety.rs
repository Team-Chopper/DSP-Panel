//! Live-safety state that has to survive a crash or restart of the server,
//! kept in safety.json in the data directory:
//!
//! ```json
//! {
//!   "muteAll": true,
//!   "unmute": {"<dsp id>": ["In A", "Out 1"]},
//!   "testGains": {"<dsp id>": {"Out 1": -3.0, "Out 2": 0.0}}
//! }
//! ```
//!
//! - `unmute`: per DSP the channels mute all has to open again – while mute
//!   all is on, the channels that were open before it; after it was released,
//!   those of a DSP that was offline at that moment (opened when it connects).
//! - `testGains`: the output gains saved before the DSP test generator was
//!   switched on; they are restored when it is switched off.
//!
//! Every change is written at once (atomically), before the device is touched.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{Map, Value, json};

use crate::log;
use crate::registry::write_atomic;

#[derive(Default)]
struct Inner {
    file: Option<PathBuf>,
    mute_all: bool,
    unmute: BTreeMap<String, Vec<String>>,
    test_gains: BTreeMap<String, BTreeMap<String, f64>>,
}

/// Handle to the safety state (single-threaded, shared by the hub and the
/// sessions).
#[derive(Clone, Default)]
pub struct SafetyStore(Rc<RefCell<Inner>>);

impl SafetyStore {
    /// A store kept in [file]; loads it if it exists.
    pub fn new(file: PathBuf) -> Self {
        let store = SafetyStore(Rc::new(RefCell::new(Inner { file: Some(file), ..Inner::default() })));
        store.load();
        store
    }

    /// A store that is not written to disk.
    pub fn memory() -> Self {
        SafetyStore::default()
    }

    fn load(&self) {
        let Some(file) = self.0.borrow().file.clone() else { return };
        if !file.exists() {
            return;
        }
        let json = std::fs::read_to_string(&file).map_err(|e| e.to_string()).and_then(|text| {
            serde_json::from_str::<Value>(&text).map_err(|e| e.to_string())
        });
        let json = match json {
            Ok(json) => json,
            Err(e) => {
                log::warn(format!("Could not read {}: {e}", file.display()));
                return;
            }
        };
        let mut i = self.0.borrow_mut();
        i.mute_all = json.get("muteAll").and_then(Value::as_bool).unwrap_or(false);
        for (dsp, list) in json.get("unmute").and_then(Value::as_object).into_iter().flatten() {
            let channels: Vec<String> =
                list.as_array().into_iter().flatten().filter_map(|c| c.as_str().map(String::from)).collect();
            if !channels.is_empty() {
                i.unmute.insert(dsp.clone(), channels);
            }
        }
        for (dsp, gains) in json.get("testGains").and_then(Value::as_object).into_iter().flatten() {
            let gains: BTreeMap<String, f64> = gains
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(ch, db)| db.as_f64().filter(|d| d.is_finite()).map(|d| (ch.clone(), d)))
                .collect();
            if !gains.is_empty() {
                i.test_gains.insert(dsp.clone(), gains);
            }
        }
    }

    fn save(&self) -> Result<(), String> {
        let i = self.0.borrow();
        let Some(file) = &i.file else { return Ok(()) };
        let gains: Map<String, Value> = i.test_gains.iter().map(|(dsp, g)| (dsp.clone(), json!(g))).collect();
        let json = json!({"muteAll": i.mute_all, "unmute": i.unmute, "testGains": gains});
        write_atomic(file, &serde_json::to_string_pretty(&json).unwrap_or_default()).map_err(|e| {
            let message = format!("Could not save {}: {e}", file.display());
            log::warn(&message);
            message
        })
    }

    // ─── Mute all ───

    pub fn mute_all(&self) -> bool {
        self.0.borrow().mute_all
    }

    pub fn set_mute_all(&self, on: bool) {
        self.0.borrow_mut().mute_all = on;
        let _ = self.save();
    }

    /// Channels of [dsp] that mute all opens again (empty if none).
    pub fn unmute_list(&self, dsp: &str) -> Vec<String> {
        self.0.borrow().unmute.get(dsp).cloned().unwrap_or_default()
    }

    /// An empty list removes the entry.
    pub fn set_unmute_list(&self, dsp: &str, channels: Vec<String>) {
        {
            let mut i = self.0.borrow_mut();
            if i.unmute.get(dsp) == Some(&channels) || (channels.is_empty() && !i.unmute.contains_key(dsp)) {
                return;
            }
            if channels.is_empty() {
                i.unmute.remove(dsp);
            } else {
                i.unmute.insert(dsp.to_string(), channels);
            }
        }
        let _ = self.save();
    }

    // ─── DSP test generator ───

    /// Output gains of [dsp] saved before the test generator was started.
    pub fn test_gains(&self, dsp: &str) -> Option<BTreeMap<String, f64>> {
        self.0.borrow().test_gains.get(dsp).cloned()
    }

    /// Save (or with `None` forget) the output gains of [dsp]. Fails if they
    /// could not be written to disk.
    pub fn set_test_gains(&self, dsp: &str, gains: Option<BTreeMap<String, f64>>) -> Result<(), String> {
        {
            let mut i = self.0.borrow_mut();
            match gains {
                Some(gains) => {
                    i.test_gains.insert(dsp.to_string(), gains);
                }
                None => {
                    if i.test_gains.remove(dsp).is_none() {
                        return Ok(());
                    }
                }
            }
        }
        self.save()
    }

    /// Forget everything about a removed DSP.
    pub fn remove_dsp(&self, dsp: &str) {
        let changed = {
            let mut i = self.0.borrow_mut();
            i.unmute.remove(dsp).is_some() | i.test_gains.remove(dsp).is_some()
        };
        if changed {
            let _ = self.save();
        }
    }
}
