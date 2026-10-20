//! Saving a finished capture to disk. The shells find the folder and the
//! local time; the naming and the write are shared.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// A wall-clock time in the user's time zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

static CLOCK: OnceLock<fn() -> LocalTime> = OnceLock::new();
static FOLDER: OnceLock<fn() -> Option<PathBuf>> = OnceLock::new();
static LAST: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Where saved pictures go: each shell sets this once at startup, so
/// shared code can find them again.
pub fn set_folder(folder: fn() -> Option<PathBuf>) {
    let _ = FOLDER.set(folder);
}

/// The folder saved pictures go in, once a shell has said.
pub fn folder() -> Option<PathBuf> {
    FOLDER.get().and_then(|f| f())
}

/// Note `path` as the picture saved last.
pub fn remember(path: &Path) {
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some(path.to_path_buf());
}

/// The picture saved last while Spooch has been running, if it's still
/// there.
pub fn last() -> Option<PathBuf> {
    LAST.lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .filter(|p| p.is_file())
}

/// The newest file in `dir`.
fn newest_in(dir: &Path) -> Option<PathBuf> {
    fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let modified = e.metadata().ok().filter(|m| m.is_file())?.modified().ok()?;
            Some((modified, e.path()))
        })
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

/// The newest picture saved from the capture named `stem`: `stem.png`,
/// `stem.jpg`, or one with " (2)" and so on after a name clash.
pub fn find_saved(stem: &str) -> Option<PathBuf> {
    find_saved_in(&folder()?, stem)
}

fn find_saved_in(dir: &Path, stem: &str) -> Option<PathBuf> {
    let clash = format!("{stem} (");
    fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| {
            let path = e.path();
            path.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s == stem || (s.starts_with(&clash) && s.ends_with(')')))
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

/// What "Show saved pictures" shows: the picture saved last, else the
/// newest in the folder, to select (`Ok`); or, with none yet, the folder
/// itself to open (`Err`), made if it's missing. `None` without a folder.
pub fn to_show() -> Option<Result<PathBuf, PathBuf>> {
    if let Some(p) = last() {
        return Some(Ok(p));
    }
    let dir = folder()?;
    match newest_in(&dir) {
        Some(p) => Some(Ok(p)),
        None => {
            let _ = fs::create_dir_all(&dir);
            Some(Err(dir))
        }
    }
}

/// How to read the local time: each shell sets this once at startup, so
/// shared code can tell what "today" is.
pub fn set_clock(clock: fn() -> LocalTime) {
    let _ = CLOCK.set(clock);
}

/// The local time now, by the shell's clock; UTC until one is set.
pub fn now() -> LocalTime {
    CLOCK.get().map_or_else(utc_now, |clock| clock())
}

fn utc_now() -> LocalTime {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    utc_from_unix(secs)
}

/// The UTC time `secs` after the Unix epoch.
pub fn utc_from_unix(secs: u64) -> LocalTime {
    let (year, month, day) = civil_from_days((secs / 86_400) as i64);
    let t = secs % 86_400;
    LocalTime {
        year,
        month,
        day,
        hour: (t / 3600) as u8,
        minute: (t / 60 % 60) as u8,
        second: (t % 60) as u8,
    }
}

/// The date `days` after 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (u16, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let year = (yoe + era * 400 + i64::from(month <= 2)) as u16;
    (year, month, day)
}

impl LocalTime {
    /// "2026-10-04".
    pub fn date(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// "14.05.09": dots, since Windows refuses colons in file names.
    pub fn dotted_time(&self) -> String {
        format!("{:02}.{:02}.{:02}", self.hour, self.minute, self.second)
    }

    /// "2026-10-04T14:05:09Z".
    pub fn rfc3339(&self) -> String {
        format!(
            "{}T{:02}:{:02}:{:02}Z",
            self.date(),
            self.hour,
            self.minute,
            self.second
        )
    }

    /// The date and time an RFC 3339 string names, as written, ignoring its
    /// fraction and zone.
    pub fn parse_rfc3339(s: &str) -> Option<LocalTime> {
        let b = s.as_bytes();
        if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || b[13] != b':' || b[16] != b':' {
            return None;
        }
        let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<u16>().ok();
        Some(LocalTime {
            year: n(0..4)?,
            month: n(5..7)? as u8,
            day: n(8..10)? as u8,
            hour: n(11..13)? as u8,
            minute: n(14..16)? as u8,
            second: n(17..19)? as u8,
        })
    }
}

/// "Spooch 2026-10-04 at 14.05.09": sorts by time and is a valid file name
/// everywhere.
pub fn file_stem(t: &LocalTime) -> String {
    format!("Spooch {} at {}", t.date(), t.dotted_time())
}

/// Write `bytes` into `dir` (created if needed) as `stem.extension`,
/// adding " (2)", " (3)" and so on rather than overwriting anything.
pub fn write_new(dir: &Path, stem: &str, extension: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    for n in 1..1000 {
        let path = if n == 1 {
            dir.join(format!("{stem}.{extension}"))
        } else {
            dir.join(format!("{stem} ({n}).{extension}"))
        };
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                if let Err(e) = f.write_all(bytes) {
                    // Don't leave a broken file to push the next save to " (2)".
                    drop(f);
                    let _ = fs::remove_file(&path);
                    return Err(e);
                }
                return Ok(path);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("too many captures named {stem}.{extension}"),
    ))
}

/// Write the file a drag-out hands to other apps: exactly `name` in `dir`,
/// which holds nothing else. Files from earlier drags are removed first, so
/// the folder never grows; by then the apps they were dropped on have long
/// copied them.
pub fn write_drag_file(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    for entry in fs::read_dir(dir)?.flatten() {
        if entry.file_type().is_ok_and(|t| t.is_file()) {
            let _ = fs::remove_file(entry.path());
        }
    }
    let path = dir.join(name);
    fs::write(&path, bytes)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_capture_s_saved_pictures_are_found_by_its_name() {
        let dir = std::env::temp_dir().join(format!("spooch-find-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(find_saved_in(&dir, "Spooch a"), None);
        assert_eq!(newest_in(&dir), None);
        fs::write(dir.join("Spooch a.png"), b"1").unwrap();
        // Another capture whose name starts the same isn't it.
        fs::write(dir.join("Spooch a 2.png"), b"2").unwrap();
        assert_eq!(
            find_saved_in(&dir, "Spooch a"),
            Some(dir.join("Spooch a.png"))
        );
        // A clash saved later is the newer.
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(dir.join("Spooch a (2).jpg"), b"3").unwrap();
        assert_eq!(
            find_saved_in(&dir, "Spooch a"),
            Some(dir.join("Spooch a (2).jpg"))
        );
        assert_eq!(newest_in(&dir), Some(dir.join("Spooch a (2).jpg")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn days_since_1970_become_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_365), (2025, 10, 4));
        assert_eq!(civil_from_days(20_730), (2026, 10, 4));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }

    const T: LocalTime = LocalTime {
        year: 2026,
        month: 10,
        day: 4,
        hour: 9,
        minute: 5,
        second: 7,
    };

    #[test]
    fn names_sort_by_time_and_avoid_colons() {
        assert_eq!(file_stem(&T), "Spooch 2026-10-04 at 09.05.07");
    }

    #[test]
    fn never_overwrites() {
        let dir = std::env::temp_dir().join(format!("spooch-save-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let nested = dir.join("Spooch");
        let a = write_new(&nested, &file_stem(&T), "png", b"one").unwrap();
        let b = write_new(&nested, &file_stem(&T), "png", b"two").unwrap();
        assert_eq!(a.file_name().unwrap(), "Spooch 2026-10-04 at 09.05.07.png");
        assert_eq!(
            b.file_name().unwrap(),
            "Spooch 2026-10-04 at 09.05.07 (2).png"
        );
        assert_eq!(fs::read(&a).unwrap(), b"one");
        assert_eq!(fs::read(&b).unwrap(), b"two");
        let c = write_new(&nested, &file_stem(&T), "jpg", b"three").unwrap();
        assert_eq!(c.file_name().unwrap(), "Spooch 2026-10-04 at 09.05.07.jpg");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn drag_files_replace_earlier_ones() {
        let dir = std::env::temp_dir().join(format!("spooch-drag-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let a = write_drag_file(&dir, "a.png", b"one").unwrap();
        let b = write_drag_file(&dir, "b.jpg", b"two").unwrap();
        assert!(!a.exists());
        assert_eq!(fs::read(&b).unwrap(), b"two");
        // Same name twice is fine too.
        write_drag_file(&dir, "b.jpg", b"three").unwrap();
        assert_eq!(fs::read(&b).unwrap(), b"three");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rfc3339_both_ways() {
        assert_eq!(utc_from_unix(0).rfc3339(), "1970-01-01T00:00:00Z");
        assert_eq!(
            utc_from_unix(1_000_000_000).rfc3339(),
            "2001-09-09T01:46:40Z"
        );
        let t = LocalTime::parse_rfc3339("2026-10-17T09:30:15.123Z").unwrap();
        assert_eq!(t.rfc3339(), "2026-10-17T09:30:15Z");
        assert_eq!(LocalTime::parse_rfc3339("yesterday"), None);
    }
}
