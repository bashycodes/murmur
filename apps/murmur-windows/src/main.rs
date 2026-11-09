//! Tauri entry point (built only with `--features app`).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    murmur_windows::app::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("murmur-windows is the Windows shell; build it on Windows (see README.md).");
    std::process::exit(1);
}
