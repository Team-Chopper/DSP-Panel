//! Port of dart/server/test/auth_test.dart.

use dsp_server::auth::{AuthService, MAX_FAILURES_PER_MINUTE, pbkdf2};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dsp_auth_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn pbkdf2_matches_the_rfc_7914_test_vector() {
    // RFC 7914 §11: PBKDF2-HMAC-SHA256, P="passwd", S="salt", c=1 (first 32 bytes)
    let key = pbkdf2(b"passwd", b"salt", 1);
    let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hex, "55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc");
}

#[test]
fn password_and_session_tokens() {
    let dir = temp_dir("tokens");
    let mut auth = AuthService::new(true, dir.clone());
    auth.init().unwrap();
    assert!(!auth.has_password());
    assert!(!auth.verify_password(""));

    auth.set_password("correct horse").unwrap();
    assert!(auth.verify_password("correct horse"));
    assert!(!auth.verify_password("wrong"));

    let token = auth.issue_token();
    assert!(auth.verify_token(Some(&token)));
    assert!(!auth.verify_token(Some(&format!("{token}x"))));
    let (_, rest) = token.split_once('.').unwrap();
    assert!(!auth.verify_token(Some(&format!("9999999999.{rest}"))));
    assert!(!auth.verify_token(None));

    // Survives a restart (same secret and hash on disk) …
    let mut restarted = AuthService::new(true, dir.clone());
    restarted.init().unwrap();
    assert!(restarted.verify_token(Some(&token)));
    assert!(restarted.verify_password("correct horse"));

    // … but not a password change.
    restarted.set_password("battery staple").unwrap();
    assert!(!restarted.verify_token(Some(&token)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rate_limiting_after_repeated_failures() {
    let mut auth = AuthService::new(true, temp_dir("rate"));
    for _ in 0..MAX_FAILURES_PER_MINUTE {
        assert!(auth.allow_attempt("10.0.0.1"));
        auth.record_failure("10.0.0.1");
    }
    assert!(!auth.allow_attempt("10.0.0.1"));
    assert!(auth.allow_attempt("10.0.0.2"));
}

/// auth.json written by the Dart server (password "secret-password").
#[test]
fn reads_the_files_of_the_dart_server() {
    let dir = temp_dir("dart");
    std::fs::create_dir_all(&dir).unwrap();
    // salt = 16 zero bytes, hash = PBKDF2 of "secret-password" with 1000 iterations
    let salt = [0u8; 16];
    let hash = pbkdf2(b"secret-password", &salt, 1000);
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;
    let json = format!(
        "{{\n  \"algorithm\": \"pbkdf2-hmac-sha256\",\n  \"iterations\": 1000,\n  \"salt\": \"{}\",\n  \"hash\": \"{}\"\n}}",
        b64.encode(salt),
        b64.encode(&hash)
    );
    std::fs::write(dir.join("auth.json"), json).unwrap();
    std::fs::write(dir.join("secret.key"), b64.encode([7u8; 32])).unwrap();
    let mut auth = AuthService::new(true, dir.clone());
    auth.init().unwrap();
    assert!(auth.verify_password("secret-password"));
    assert!(!auth.verify_password("other"));
    let _ = std::fs::remove_dir_all(dir);
}
