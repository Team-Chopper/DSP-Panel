//! Authentication.
//!
//! One shared password, stored as PBKDF2-HMAC-SHA256 hash in auth.json in the
//! data directory. A successful login yields a signed session token (cookie)
//! that is valid for 30 days. The token embeds a fingerprint of the password
//! hash, so changing the password logs out every session. File formats and
//! tokens are those of the Dart server: a data directory can be shared.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::registry::write_atomic;

type HmacSha256 = Hmac<Sha256>;

pub const COOKIE_NAME: &str = "dsp_session";
pub const SESSION_LIFETIME: Duration = Duration::from_secs(30 * 24 * 3600);
pub const ITERATIONS: u32 = 210_000;
pub const MAX_FAILURES_PER_MINUTE: usize = 5;

struct PasswordHash {
    salt: Vec<u8>,
    iterations: u32,
    hash: Vec<u8>,
}

impl PasswordHash {
    fn to_json(&self) -> Value {
        json!({
            "algorithm": "pbkdf2-hmac-sha256",
            "iterations": self.iterations,
            "salt": STANDARD.encode(&self.salt),
            "hash": STANDARD.encode(&self.hash),
        })
    }

    fn from_json(json: &Value) -> Result<Self, String> {
        let bytes = |k: &str| {
            json.get(k)
                .and_then(Value::as_str)
                .and_then(|s| STANDARD.decode(s).ok())
                .ok_or(format!("auth.json: {k} missing"))
        };
        Ok(PasswordHash {
            salt: bytes("salt")?,
            iterations: json.get("iterations").and_then(Value::as_u64).ok_or("auth.json: iterations missing")? as u32,
            hash: bytes("hash")?,
        })
    }
}

pub struct AuthService {
    pub enabled: bool,
    data_dir: PathBuf,
    secret: Vec<u8>,
    hash: Option<PasswordHash>,
    failures: HashMap<String, Vec<Instant>>,
}

impl AuthService {
    pub fn new(enabled: bool, data_dir: PathBuf) -> Self {
        AuthService { enabled, data_dir, secret: Vec::new(), hash: None, failures: HashMap::new() }
    }

    pub fn has_password(&self) -> bool {
        self.hash.is_some()
    }

    fn auth_file(&self) -> PathBuf {
        self.data_dir.join("auth.json")
    }

    fn secret_file(&self) -> PathBuf {
        self.data_dir.join("secret.key")
    }

    /// Load (or create) the signing secret and the password hash.
    pub fn init(&mut self) -> Result<(), String> {
        std::fs::create_dir_all(&self.data_dir).map_err(|e| format!("{}: {e}", self.data_dir.display()))?;
        let secret_file = self.secret_file();
        if secret_file.exists() {
            let text = std::fs::read_to_string(&secret_file).map_err(|e| e.to_string())?;
            self.secret = STANDARD.decode(text.trim()).map_err(|e| format!("secret.key: {e}"))?;
        } else {
            self.secret = random_bytes(32);
            write_private(&secret_file, &STANDARD.encode(&self.secret)).map_err(|e| e.to_string())?;
        }
        let auth_file = self.auth_file();
        if auth_file.exists() {
            let text = std::fs::read_to_string(&auth_file).map_err(|e| e.to_string())?;
            let json: Value = serde_json::from_str(&text).map_err(|e| format!("auth.json: {e}"))?;
            self.hash = Some(PasswordHash::from_json(&json)?);
        }
        Ok(())
    }

    pub fn set_password(&mut self, password: &str) -> Result<(), String> {
        let salt = random_bytes(16);
        let hash = pbkdf2(password.as_bytes(), &salt, ITERATIONS);
        let entry = PasswordHash { salt, iterations: ITERATIONS, hash };
        let text = serde_json::to_string_pretty(&entry.to_json()).unwrap_or_default();
        write_private(&self.auth_file(), &text).map_err(|e| e.to_string())?;
        self.hash = Some(entry);
        Ok(())
    }

    pub fn verify_password(&self, password: &str) -> bool {
        let Some(hash) = &self.hash else { return false };
        constant_time_equals(&pbkdf2(password.as_bytes(), &hash.salt, hash.iterations), &hash.hash)
    }

    // ─── Rate limiting ───

    /// False if [client] had too many failed logins in the last minute.
    pub fn allow_attempt(&mut self, client: &str) -> bool {
        let cutoff = Instant::now().checked_sub(Duration::from_secs(60));
        let count = match self.failures.get_mut(client) {
            None => 0,
            Some(list) => {
                list.retain(|t| cutoff.is_none_or(|c| *t >= c));
                list.len()
            }
        };
        if count == 0 {
            self.failures.remove(client);
        }
        count < MAX_FAILURES_PER_MINUTE
    }

    pub fn record_failure(&mut self, client: &str) {
        if self.failures.len() > 10_000 {
            self.failures.clear(); // bound memory
        }
        self.failures.entry(client.to_string()).or_default().push(Instant::now());
    }

    // ─── Session tokens ───

    pub fn issue_token(&self) -> String {
        let expires = (SystemTime::now() + SESSION_LIFETIME).duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let payload = format!("{expires}.{}", self.password_fingerprint());
        let signature = self.sign(&payload);
        format!("{payload}.{signature}")
    }

    pub fn verify_token(&self, token: Option<&str>) -> bool {
        let (Some(token), Some(_)) = (token, &self.hash) else { return false };
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return false;
        }
        let payload = format!("{}.{}", parts[0], parts[1]);
        if !constant_time_equals(self.sign(&payload).as_bytes(), parts[2].as_bytes()) {
            return false;
        }
        let Ok(expires) = parts[0].parse::<i64>() else { return false };
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
        if expires * 1000 < now {
            return false;
        }
        parts[1] == self.password_fingerprint()
    }

    fn sign(&self, payload: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(&self.secret).expect("HMAC accepts any key length");
        mac.update(payload.as_bytes());
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    }

    fn password_fingerprint(&self) -> String {
        let hash = self.hash.as_ref().map(|h| h.hash.as_slice()).unwrap_or_default();
        URL_SAFE.encode(&Sha256::digest(hash)[..9])
    }
}

/// PBKDF2-HMAC-SHA256 with a 32-byte output (one block).
pub fn pbkdf2(password: &[u8], salt: &[u8], iterations: u32) -> Vec<u8> {
    let mac = HmacSha256::new_from_slice(password).expect("HMAC accepts any key length");
    let mut first = mac.clone();
    first.update(salt);
    first.update(&[0, 0, 0, 1]);
    let mut u = first.finalize().into_bytes();
    let mut result = u.to_vec();
    for _ in 1..iterations {
        let mut next = mac.clone();
        next.update(&u);
        u = next.finalize().into_bytes();
        for (r, b) in result.iter_mut().zip(u.iter()) {
            *r ^= b;
        }
    }
    result
}

fn constant_time_equals(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

fn random_bytes(n: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; n];
    getrandom::fill(&mut bytes).expect("system random source");
    bytes
}

/// Write a file only the service user can read (temp file, 0600, rename).
fn write_private(file: &std::path::Path, content: &str) -> std::io::Result<()> {
    let tmp = PathBuf::from(format!("{}.tmp", file.display()));
    write_atomic(&tmp, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, file)
}
