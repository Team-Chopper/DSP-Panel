//! UI settings remembered per browser (localStorage), e.g. which EQ curves are
//! hidden on the gain page. Without storage (private mode) nothing is kept.

use std::collections::BTreeSet;

const PREFIX: &str = "dsp-panel.";

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

pub fn get_string(key: &str) -> Option<String> {
    storage()?.get_item(&format!("{PREFIX}{key}")).ok().flatten()
}

pub fn set_string(key: &str, value: &str) {
    if let Some(s) = storage() {
        let _ = s.set_item(&format!("{PREFIX}{key}"), value);
    }
}

pub fn get_string_set(key: &str) -> BTreeSet<String> {
    get_string(key).and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or_default()
}

pub fn set_string_set(key: &str, values: &BTreeSet<String>) {
    set_string(key, &serde_json::to_string(values).unwrap_or_default());
}
