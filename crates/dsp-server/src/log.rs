//! Minimal logging to stdout/stderr (captured by journald under systemd).

use std::sync::atomic::{AtomicBool, Ordering};

static VERBOSE: AtomicBool = AtomicBool::new(false);

pub fn set_verbose(on: bool) {
    VERBOSE.store(on, Ordering::Relaxed);
}

pub fn info(message: impl AsRef<str>) {
    println!("[INFO] {}", message.as_ref());
}

pub fn warn(message: impl AsRef<str>) {
    eprintln!("[WARN] {}", message.as_ref());
}

pub fn debug(message: impl AsRef<str>) {
    if VERBOSE.load(Ordering::Relaxed) {
        println!("[DEBUG] {}", message.as_ref());
    }
}
