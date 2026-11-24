//! UI settings remembered per browser (localStorage), e.g. the EQ graph view and
//! the RTA input. Without storage (private mode) nothing is kept.

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
