//! Server configuration (config.yaml), same format as the Dart server:
//!
//! ```yaml
//! listen:
//!   address: 0.0.0.0
//!   port: 8443
//! tls:
//!   enabled: true
//!   cert: /var/lib/dsp-panel/tls/cert.pem
//!   key: /var/lib/dsp-panel/tls/key.pem
//! auth:
//!   enabled: true
//! data_dir: /var/lib/dsp-panel
//! web_root: /opt/dsp-panel/web
//! ```

use std::fmt;
use std::path::{Component, Path, PathBuf};

use yaml_rust2::{Yaml, YamlLoader};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerConfig {
    pub address: String,
    pub port: u16,
    pub tls_enabled: bool,
    pub tls_cert: PathBuf,
    pub tls_key: PathBuf,
    pub auth_enabled: bool,
    pub data_dir: PathBuf,
    /// Directory of the web client; if it does not exist, the client built
    /// into the server binary is served (if any).
    pub web_root: PathBuf,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            address: "0.0.0.0".into(),
            port: 8443,
            tls_enabled: true,
            tls_cert: "/var/lib/dsp-panel/tls/cert.pem".into(),
            tls_key: "/var/lib/dsp-panel/tls/key.pem".into(),
            auth_enabled: true,
            data_dir: "/var/lib/dsp-panel".into(),
            web_root: "/opt/dsp-panel/web".into(),
        }
    }
}

impl fmt::Display for ServerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "listen={}:{} tls={} auth={} data_dir={} web_root={}",
            self.address,
            self.port,
            self.tls_enabled,
            self.auth_enabled,
            self.data_dir.display(),
            self.web_root.display()
        )
    }
}

/// [path] made absolute against the current directory and normalised
/// (`.` and `..` removed), like Dart's `p.normalize(p.absolute(path))`.
pub fn absolute(path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    normalize(&joined)
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

impl ServerConfig {
    /// Load a YAML config file. Missing keys keep their defaults; relative
    /// paths are resolved against the directory of the config file.
    pub fn load(path: &Path) -> Result<ServerConfig, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let docs = YamlLoader::load_from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let defaults = ServerConfig::default();
        let Some(yaml) = docs.into_iter().next().filter(|y| !y.is_null() && !y.is_badvalue()) else {
            return Ok(defaults);
        };
        let shown = path.display().to_string();
        if !matches!(yaml, Yaml::Hash(_)) {
            return Err(format!("{shown}: expected a mapping at top level"));
        }
        let base = absolute(path).parent().map(Path::to_path_buf).unwrap_or_default();
        let resolve = |value: PathBuf| if value.is_absolute() { normalize(&value) } else { normalize(&base.join(value)) };

        let wrong = |key: &str| format!("{shown}: \"{key}\" has the wrong type");
        let section = |key: &str| -> Result<Option<&Yaml>, String> {
            match &yaml[key] {
                Yaml::BadValue | Yaml::Null => Ok(None),
                v @ Yaml::Hash(_) => Ok(Some(v)),
                _ => Err(wrong(key)),
            }
        };
        let string = |map: Option<&Yaml>, key: &str| -> Result<Option<String>, String> {
            match map.map(|m| &m[key]) {
                None | Some(Yaml::BadValue) | Some(Yaml::Null) => Ok(None),
                Some(Yaml::String(s)) => Ok(Some(s.clone())),
                Some(_) => Err(wrong(key)),
            }
        };
        let int = |map: Option<&Yaml>, key: &str| -> Result<Option<i64>, String> {
            match map.map(|m| &m[key]) {
                None | Some(Yaml::BadValue) | Some(Yaml::Null) => Ok(None),
                Some(Yaml::Integer(i)) => Ok(Some(*i)),
                Some(_) => Err(wrong(key)),
            }
        };
        let boolean = |map: Option<&Yaml>, key: &str| -> Result<Option<bool>, String> {
            match map.map(|m| &m[key]) {
                None | Some(Yaml::BadValue) | Some(Yaml::Null) => Ok(None),
                Some(Yaml::Boolean(b)) => Ok(Some(*b)),
                Some(_) => Err(wrong(key)),
            }
        };

        let listen = section("listen")?;
        let tls = section("tls")?;
        let auth = section("auth")?;
        let root = Some(&yaml);
        let port = int(listen, "port")?.unwrap_or(defaults.port as i64);
        if !(1..=65535).contains(&port) {
            return Err(format!("{shown}: listen.port must be 1-65535"));
        }
        Ok(ServerConfig {
            address: string(listen, "address")?.unwrap_or(defaults.address),
            port: port as u16,
            tls_enabled: boolean(tls, "enabled")?.unwrap_or(defaults.tls_enabled),
            tls_cert: resolve(string(tls, "cert")?.map_or(defaults.tls_cert, PathBuf::from)),
            tls_key: resolve(string(tls, "key")?.map_or(defaults.tls_key, PathBuf::from)),
            auth_enabled: boolean(auth, "enabled")?.unwrap_or(defaults.auth_enabled),
            data_dir: resolve(string(root, "data_dir")?.map_or(defaults.data_dir, PathBuf::from)),
            web_root: resolve(string(root, "web_root")?.map_or(defaults.web_root, PathBuf::from)),
        })
    }
}
