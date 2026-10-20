//! Spooch for Windows.
//!
//! The Win32 shell: the tray, global shortcuts, capture (DXGI Desktop
//! Duplication, falling back to GDI), one frozen overlay per monitor, the
//! editor window, the clipboard and drag-out. The editor itself is shared
//! (`spooch-app`, `spooch-render`); this only translates input and shows
//! pixels.
//!
//! Usage: `spooch.exe [--backend=gdi|dxgi] [--no-recents] [--editor]
//! [--welcome]`. The log, with every capture's timings, goes to stderr and
//! `%LOCALAPPDATA%\Spooch\spooch.log`.

// A tray app: no console window, even when started from Explorer or at
// sign-in. Redirected stderr (`2> file`) still works.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod win;

#[cfg(windows)]
fn main() {
    win::log::catch_panics();
    let result = win::run();
    if let Err(e) = &result {
        win::log::line(format!("fatal: {e}"));
    }
    win::log::flush();
    if result.is_err() {
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("spooch-shell-win only runs on Windows.");
    std::process::exit(1);
}
