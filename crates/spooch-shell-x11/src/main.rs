//! Spooch for Linux on X11.
//!
//! The X11 shell (`x11rb`, no toolkit): the XEmbed tray and its menu, key
//! grabs, the frozen desktop under one override-redirect window, the editor
//! window, the clipboard and XDND drag-out. The editor itself is shared
//! (`spooch-app`, `spooch-render`); this only translates input and shows
//! pixels.
//!
//! Usage: `spooch-x11 [--backend=shm|getimage] [--no-recents] [--welcome]
//! [--once] [--save=out.png]`. `--once` captures straight away and exits
//! after; `--save` also writes the copied PNG to a file. The log goes to
//! stderr and
//! `$XDG_STATE_HOME/spooch/spooch.log`.

#[cfg(target_os = "linux")]
mod x11;

#[cfg(target_os = "linux")]
fn main() {
    x11::log::catch_panics();
    let result = x11::run();
    if let Err(e) = &result {
        x11::log::line(format!("fatal: {e}"));
    }
    x11::log::flush();
    if result.is_err() {
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("spooch-shell-x11 only runs on Linux.");
    std::process::exit(1);
}
