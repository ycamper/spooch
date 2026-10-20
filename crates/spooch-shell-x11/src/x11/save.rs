//! Where Ctrl+S saves and drag-out files go, and the local time captures
//! are named by.

use std::path::{Path, PathBuf};

use spooch_platform::save::LocalTime;

use super::system::{home, xdg_dir};

/// Where drag-out files are written: `$XDG_CACHE_HOME/spooch/drag`, in
/// the home folder rather than /tmp, which sandboxed apps can't read.
pub fn drag_folder() -> PathBuf {
    xdg_dir("XDG_CACHE_HOME", ".cache")
        .join("spooch")
        .join("drag")
}

/// `$XDG_PICTURES_DIR/Spooch`, falling back to `~/Pictures/Spooch`.
pub fn folder() -> PathBuf {
    let home = home();
    let config = xdg_dir("XDG_CONFIG_HOME", ".config");
    let pictures = std::fs::read_to_string(config.join("user-dirs.dirs"))
        .ok()
        .and_then(|s| pictures_dir(&s, &home))
        .unwrap_or_else(|| home.join("Pictures"));
    pictures.join("Spooch")
}

/// Read `XDG_PICTURES_DIR="$HOME/Pictures"` from user-dirs.dirs, which
/// only ever holds absolute paths or paths starting with `$HOME/`.
fn pictures_dir(dirs: &str, home: &Path) -> Option<PathBuf> {
    let value = dirs.lines().find_map(|l| {
        l.trim()
            .strip_prefix("XDG_PICTURES_DIR=")
            .map(|v| v.trim_matches('"'))
    })?;
    let path = match value.strip_prefix("$HOME") {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => PathBuf::from(value),
    };
    // A value of "$HOME/" means the folder is disabled.
    (path.is_absolute() && path != home).then_some(path)
}

pub fn now() -> LocalTime {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        LocalTime {
            year: (tm.tm_year + 1900) as u16,
            month: (tm.tm_mon + 1) as u8,
            day: tm.tm_mday as u8,
            hour: tm.tm_hour as u8,
            minute: tm.tm_min as u8,
            second: tm.tm_sec as u8,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_pictures_folder() {
        let home = Path::new("/home/user");
        let dirs =
            "# comment\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_PICTURES_DIR=\"$HOME/Bilder\"\n";
        assert_eq!(
            pictures_dir(dirs, home),
            Some(PathBuf::from("/home/user/Bilder"))
        );
        assert_eq!(
            pictures_dir("XDG_PICTURES_DIR=\"/data/pics\"", home),
            Some(PathBuf::from("/data/pics"))
        );
        assert_eq!(pictures_dir("XDG_PICTURES_DIR=\"$HOME/\"", home), None);
        assert_eq!(pictures_dir("", home), None);
    }
}
