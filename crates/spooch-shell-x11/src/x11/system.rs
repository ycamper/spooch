//! Small pieces of the desktop the tray menu and first-run welcome need:
//! where settings live, starting at login, GNOME's hold on the Print
//! Screen key, opening files, and questions. Questions go through `zenity`
//! when it's installed (it is on GNOME), else a notification.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/".into()))
}

/// An XDG base folder: `$var` when it holds an absolute path, else
/// `fallback` in the home folder.
pub fn xdg_dir(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(fallback))
}

fn config_home() -> PathBuf {
    xdg_dir("XDG_CONFIG_HOME", ".config")
}

/// `$XDG_CONFIG_HOME/spooch`, where settings live.
pub fn config_dir() -> PathBuf {
    config_home().join("spooch")
}

fn autostart_path() -> PathBuf {
    config_home().join("autostart").join("spooch.desktop")
}

fn autostart_entry() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!(
        "[Desktop Entry]\nType=Application\nName=Spooch\nComment=Screenshots you can mark up\nExec=\"{}\"\nX-GNOME-Autostart-enabled=true\n",
        exe.display()
    ))
}

/// Whether the desktop starts this Spooch at login.
pub fn launch_at_login() -> bool {
    match (fs::read_to_string(autostart_path()), autostart_entry()) {
        (Ok(have), Some(want)) => have == want,
        _ => false,
    }
}

pub fn set_launch_at_login(on: bool) -> io::Result<()> {
    let path = autostart_path();
    if on {
        let entry =
            autostart_entry().ok_or_else(|| io::Error::other("can't find Spooch's own path"))?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, entry)
    } else {
        match fs::remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

/// Show `path` selected in the file manager, through the freedesktop
/// FileManager1 interface (Nautilus and most others have it); else open
/// the folder it's in.
pub fn reveal(path: &Path) {
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        let uri = format!("['{}']", super::xdnd::file_uri(&path));
        let shown = Command::new("gdbus")
            .args([
                "call",
                "--session",
                "--dest",
                "org.freedesktop.FileManager1",
                "--object-path",
                "/org/freedesktop/FileManager1",
                "--method",
                "org.freedesktop.FileManager1.ShowItems",
                &uri,
                "",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !shown && let Some(dir) = path.parent() {
            open(dir);
        }
    });
}

/// Open a file or folder with the desktop's default app, without waiting.
pub fn open(path: &Path) {
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        let _ = Command::new("xdg-open")
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    });
}

fn zenity(args: &[&str]) -> Option<bool> {
    Command::new("zenity")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()
        .map(|s| s.success())
}

/// A yes/no question. `None` when there's no way to ask (no zenity).
pub fn ask(text: &str) -> Option<bool> {
    zenity(&[
        "--question",
        "--title=Spooch",
        "--width=420",
        "--ok-label=Yes",
        "--cancel-label=No",
        &format!("--text={}", markup(text)),
    ])
}

/// `text` escaped for zenity, which reads `--text` as Pango markup.
fn markup(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// What was typed in each field of `form`. `None` if cancelled or there's
/// no way to ask (no zenity). zenity's forms start empty, so the fields'
/// values aren't shown.
pub fn ask_form(form: &spooch_app::Form) -> Option<Vec<String>> {
    let mut args = vec![
        "--forms".to_string(),
        format!("--title={}", form.title),
        "--width=460".into(),
        format!("--text={}", markup(&form.prompt)),
        "--separator=\u{1f}".into(),
    ];
    for f in &form.fields {
        let kind = if f.secret {
            "--add-password"
        } else {
            "--add-entry"
        };
        args.push(format!("{kind}={}", f.label));
    }
    let out = Command::new("zenity")
        .args(&args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let out = match out {
        Ok(out) => out,
        Err(_) => {
            tell_now("Signing in needs zenity to ask for the details.");
            return None;
        }
    };
    if !out.status.success() {
        return None;
    }
    let typed = String::from_utf8_lossy(&out.stdout);
    let mut answers: Vec<String> = typed
        .trim_end_matches('\n')
        .split('\u{1f}')
        .map(str::to_string)
        .collect();
    answers.resize(form.fields.len(), String::new());
    Some(answers)
}

/// A notice. Doesn't wait for it to be read.
pub fn tell(text: &str) {
    let text = text.to_string();
    std::thread::spawn(move || tell_now(&text));
}

fn tell_now(text: &str) {
    let shown = zenity(&[
        "--info",
        "--title=Spooch",
        "--width=420",
        &format!("--text={}", markup(text)),
    ]);
    if shown.is_none() {
        let _ = Command::new("notify-send")
            .args(["Spooch", text])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

const SCHEMA: &str = "org.gnome.shell.keybindings";
const KEY: &str = "show-screenshot-ui";

/// GNOME's shortcut for its screenshot tool, which is Print Screen and
/// keeps the key from every other app. `None` off GNOME.
pub fn gnome_screenshot_binding() -> Option<String> {
    let out = Command::new("gsettings")
        .args(["get", SCHEMA, KEY])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Set GNOME's screenshot shortcut, as `gsettings` prints it, for example
/// `['Print']` or `@as []` for none.
pub fn set_gnome_screenshot_binding(value: &str) -> Result<(), String> {
    let status = Command::new("gsettings")
        .args(["set", SCHEMA, KEY, value])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| e.to_string())?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("gsettings exited with {status}"))
}

/// The scale `Xft.dpi` in X resources `text` asks for, from 1 to 4.
pub fn xft_scale(text: &[u8]) -> f32 {
    String::from_utf8_lossy(text)
        .lines()
        .find_map(|l| l.strip_prefix("Xft.dpi:"))
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|dpi| dpi.is_finite())
        .map_or(1.0, |dpi| (dpi / 96.0).clamp(1.0, 4.0))
}

#[cfg(test)]
mod tests {
    use super::xft_scale;

    #[test]
    fn xft_dpi_sets_the_scale() {
        assert_eq!(xft_scale(b"Xft.antialias:\t1\nXft.dpi:\t192\n"), 2.0);
        assert_eq!(xft_scale(b"Xft.dpi: 144"), 1.5);
        assert_eq!(xft_scale(b""), 1.0);
        assert_eq!(xft_scale(b"Xft.dpi:\tlots"), 1.0);
        assert_eq!(xft_scale(b"Xft.dpi:\t72"), 1.0);
        assert_eq!(xft_scale(b"Xft.dpi:\t9999"), 4.0);
        assert_eq!(xft_scale(b"Xft.dpi:\tNaN"), 1.0);
    }
}
