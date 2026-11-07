//! The list of configured DSPs, persisted as devices.json in the data
//! directory (same format as the Dart server, plus "gainCaps").

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use dsp_core::protocol::{MAX_DEVICE_ID, MIN_DEVICE_ID, quantize_gain};
use dsp_core::{DEFAULT_REFRESH_MS, DeviceModel};
use serde_json::{Map, Value, json};

/// Range of a channel's max level (the gain range).
pub const GAIN_CAP_MIN_DB: f64 = -60.0;
pub const GAIN_CAP_MAX_DB: f64 = 12.0;

/// Persistent settings of one DSP.
#[derive(Clone, Debug, PartialEq)]
pub struct DspConfig {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: i64,
    pub enabled: bool,
    pub refresh_ms: i64,
    /// "auto" (detect from the device) or a model id.
    pub model: String,
    /// Device ID of the DSP (1 … 250), the address byte of the frames.
    pub device_id: i64,
    /// Password of a locked DSP (4 characters) or "".
    pub password: String,
    /// Max level per channel in dB, enforced by the server (the DSP does not
    /// know it).
    pub gain_caps: BTreeMap<String, f64>,
}

impl DspConfig {
    pub fn new(id: &str, name: &str, host: &str, port: i64) -> Self {
        DspConfig {
            id: id.into(),
            name: name.into(),
            host: host.into(),
            port,
            enabled: true,
            refresh_ms: DEFAULT_REFRESH_MS,
            model: "auto".into(),
            device_id: 1,
            password: String::new(),
            gain_caps: BTreeMap::new(),
        }
    }

    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("id".into(), json!(self.id));
        m.insert("name".into(), json!(self.name));
        m.insert("host".into(), json!(self.host));
        m.insert("port".into(), json!(self.port));
        m.insert("enabled".into(), json!(self.enabled));
        m.insert("refreshMs".into(), json!(self.refresh_ms));
        m.insert("model".into(), json!(self.model));
        m.insert("deviceId".into(), json!(self.device_id));
        if !self.password.is_empty() {
            m.insert("password".into(), json!(self.password));
        }
        if !self.gain_caps.is_empty() {
            m.insert("gainCaps".into(), json!(self.gain_caps));
        }
        Value::Object(m)
    }

    /// Older versions also stored "remembered" mute/phase/delay/PEQ bypass
    /// values; they are read from the device now and the key is ignored.
    pub fn from_json(json: &Value) -> Result<Self, String> {
        let text = |k: &str| json.get(k).and_then(Value::as_str).map(String::from).ok_or(format!("{k} missing"));
        let int = |k: &str| json.get(k).and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)));
        let model = json.get("model").and_then(Value::as_str).unwrap_or("auto").to_string();
        let password = json.get("password").and_then(Value::as_str).unwrap_or("").to_string();
        Ok(DspConfig {
            id: text("id")?,
            name: text("name")?,
            host: text("host")?,
            port: int("port").unwrap_or(9761),
            enabled: json.get("enabled").and_then(Value::as_bool).unwrap_or(true),
            refresh_ms: refresh_from_json(int("refreshMs")),
            model: if is_valid_model_setting(&model) { model } else { "auto".into() },
            device_id: int("deviceId").unwrap_or(1).clamp(MIN_DEVICE_ID as i64, MAX_DEVICE_ID as i64),
            password: if is_valid_password(&password) { password.to_uppercase() } else { String::new() },
            gain_caps: json
                .get("gainCaps")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .filter_map(|(ch, db)| db.as_f64().filter(|d| is_valid_gain_cap(*d)).map(|d| (ch.clone(), quantize_gain(d))))
                .collect(),
        })
    }

    pub fn new_id() -> String {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).expect("system random source");
        bytes.iter().map(|b| format!("{:x}", b & 0x0F)).collect()
    }

    /// Validate user input for name/host/port/model/device ID/password.
    pub fn validate_fields(
        name: Option<&str>,
        host: Option<&str>,
        port: Option<i64>,
        model: Option<&str>,
        device_id: Option<i64>,
        password: Option<&str>,
    ) -> Option<String> {
        if let Some(id) = device_id
            && (id < MIN_DEVICE_ID as i64 || id > MAX_DEVICE_ID as i64) {
                return Some(format!("Device ID must be between {MIN_DEVICE_ID} and {MAX_DEVICE_ID}"));
            }
        if password.is_some_and(|p| !is_valid_password(p)) {
            return Some("The password has 4 characters (empty = none)".into());
        }
        if name.is_some_and(|n| n.trim().is_empty() || n.encode_utf16().count() > 40) {
            return Some("Name must be 1–40 characters".into());
        }
        if let Some(host) = host {
            let valid_chars = host.chars().all(|c| c.is_ascii_alphanumeric() || ".-:[]".contains(c));
            if host.is_empty() || host.len() > 253 || !valid_chars {
                return Some("Invalid IP address or host name".into());
            }
        }
        if port.is_some_and(|p| !(1..=65535).contains(&p)) {
            return Some("Port must be between 1 and 65535".into());
        }
        if model.is_some_and(|m| !is_valid_model_setting(m)) {
            return Some("Unknown model".into());
        }
        None
    }
}

/// Stored interval; 40 ms was the default of versions 0.7–0.8 and is replaced
/// by the original editor's 300 ms.
fn refresh_from_json(value: Option<i64>) -> i64 {
    match value {
        None | Some(40) => DEFAULT_REFRESH_MS,
        Some(ms) => ms.clamp(5, 300),
    }
}

/// Empty (no password) or 4 printable ASCII characters, like the original
/// editor's lock password.
pub fn is_valid_password(password: &str) -> bool {
    password.is_empty() || (password.encode_utf16().count() == 4 && password.chars().all(|c| ('!'..='~').contains(&c)))
}

pub fn is_valid_gain_cap(db: f64) -> bool {
    db.is_finite() && (GAIN_CAP_MIN_DB..=GAIN_CAP_MAX_DB).contains(&db)
}

pub fn is_valid_model_setting(model: &str) -> bool {
    model == "auto" || DeviceModel::is_known_id(model)
}

/// Write [content] atomically: temp file + rename.
pub fn write_atomic(file: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = PathBuf::from(format!("{}.tmp", file.display()));
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, file)
}

pub struct DeviceRegistry {
    pub file: PathBuf,
}

impl DeviceRegistry {
    pub fn new(file: PathBuf) -> Self {
        DeviceRegistry { file }
    }

    pub fn load(&self) -> Result<Vec<DspConfig>, String> {
        if !self.file.exists() {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(&self.file).map_err(|e| e.to_string())?;
        let json: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        json.get("devices")
            .and_then(Value::as_array)
            .map(|list| list.iter().map(DspConfig::from_json).collect())
            .unwrap_or(Ok(Vec::new()))
    }

    pub fn save(&self, devices: &[DspConfig]) -> std::io::Result<()> {
        let json = json!({"devices": devices.iter().map(DspConfig::to_json).collect::<Vec<_>>()});
        write_atomic(&self.file, &serde_json::to_string_pretty(&json).unwrap_or_default())
    }
}
