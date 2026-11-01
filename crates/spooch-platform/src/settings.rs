//! The settings file: a short, commented `key = value` text file people can
//! edit by hand (the tray menu opens it). Unknown keys and bad values are
//! reported and skipped, never fatal, so a typo can't stop Spooch starting.

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

/// A key a shortcut ends in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A letter (stored upper case) or digit.
    Char(char),
    /// F1 to F24.
    F(u8),
    PrintScreen,
}

/// A global shortcut: modifiers plus one key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// The Windows or Super key.
    pub logo: bool,
    pub key: Key,
}

impl Shortcut {
    /// Print Screen on its own.
    pub const PRINT_SCREEN: Shortcut = Shortcut::new(false, false, false, Key::PrintScreen);

    /// A shortcut from its modifiers, in the fields' order, and its key.
    pub const fn new(ctrl: bool, shift: bool, alt: bool, key: Key) -> Self {
        Self {
            ctrl,
            shift,
            alt,
            logo: false,
            key,
        }
    }

    /// Parse "Ctrl+Shift+2", "Alt+F9" or "PrintScreen". Case and spaces
    /// don't matter.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut s = Shortcut::new(false, false, false, Key::Char('?'));
        let mut key = None;
        for part in text.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "" => return Err(format!("{text:?} has an empty part")),
                "ctrl" | "control" => s.ctrl = true,
                "shift" => s.shift = true,
                "alt" | "option" => s.alt = true,
                "super" | "win" | "windows" | "logo" | "meta" => s.logo = true,
                k => {
                    if key.is_some() {
                        return Err(format!("{text:?} has more than one key"));
                    }
                    key = Some(
                        parse_key(k).ok_or_else(|| format!("{text:?}: unknown key {part:?}"))?,
                    );
                }
            }
        }
        s.key = key.ok_or_else(|| format!("{text:?} has no key, only modifiers"))?;
        Ok(s)
    }
}

fn parse_key(k: &str) -> Option<Key> {
    match k {
        "printscreen" | "print" | "prtscn" | "prtsc" => return Some(Key::PrintScreen),
        _ => {}
    }
    let mut chars = k.chars();
    if let (Some(c), None) = (chars.next(), chars.next())
        && c.is_ascii_alphanumeric()
    {
        return Some(Key::Char(c.to_ascii_uppercase()));
    }
    let n: u8 = k.strip_prefix('f')?.parse().ok()?;
    (1..=24).contains(&n).then_some(Key::F(n))
}

impl fmt::Display for Shortcut {
    /// "Ctrl+Alt+Shift+2": the order Windows and GNOME show them in.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, name) in [
            (self.ctrl, "Ctrl+"),
            (self.alt, "Alt+"),
            (self.shift, "Shift+"),
            (self.logo, "Super+"),
        ] {
            if on {
                f.write_str(name)?;
            }
        }
        match self.key {
            Key::Char(c) => write!(f, "{c}"),
            Key::F(n) => write!(f, "F{n}"),
            Key::PrintScreen => f.write_str("PrintScreen"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub capture: Shortcut,
    pub reopen: Shortcut,
    /// None by default: a global quit key mostly quits Spooch by accident
    /// (Ctrl+Shift+Q quits browsers too), and the tray menu has Quit.
    pub quit: Option<Shortcut>,
    /// Also capture with the Print Screen key on its own.
    pub print_screen: bool,
    /// Capture after a countdown, so open menus and hover states can be
    /// caught. Pressing it again during the countdown calls it off.
    pub countdown: Option<Shortcut>,
    /// How long the countdown runs, 1 to 10 seconds.
    pub countdown_seconds: u8,
    /// The mark colour preset new captures start with: Vivid, Soft or
    /// Ink. The editor checks the name.
    pub palette: String,
    /// The sharing service's address.
    pub share_server: String,
    /// Sync captures with the signed-in account. Off until switched on.
    pub sync: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            capture: Shortcut::new(true, true, false, Key::Char('2')),
            reopen: Shortcut::new(true, true, true, Key::Char('2')),
            quit: None,
            print_screen: false,
            countdown: Some(Shortcut::new(true, true, false, Key::Char('3'))),
            countdown_seconds: 3,
            palette: "vivid".to_string(),
            share_server: "https://api.spooch.app".to_string(),
            sync: false,
        }
    }
}

/// Whether `url` may be a sharing service: HTTPS, or plain HTTP to this
/// computer, since the device token goes with every request.
fn share_server_ok(url: &str) -> bool {
    if url.starts_with("https://") {
        return true;
    }
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or("");
    let host = host.rsplit_once(':').map_or(host, |(h, port)| {
        if port.bytes().all(|b| b.is_ascii_digit()) {
            h
        } else {
            host
        }
    });
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

impl Settings {
    /// Read settings from `text`. Returns them with one message per line
    /// that couldn't be used; those keep their default.
    pub fn parse(text: &str) -> (Settings, Vec<String>) {
        let mut s = Settings::default();
        let mut problems = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                problems.push(format!("line {}: expected key = value", n + 1));
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            let result = match key {
                "capture" => Shortcut::parse(value).map(|v| s.capture = v),
                "reopen" => Shortcut::parse(value).map(|v| s.reopen = v),
                "quit" => parse_optional(value).map(|v| s.quit = v),
                "print_screen" => parse_bool(value).map(|v| s.print_screen = v),
                "sync" => parse_bool(value).map(|v| s.sync = v),
                "countdown" => parse_optional(value).map(|v| s.countdown = v),
                "countdown_seconds" => match value.parse::<u8>() {
                    Ok(n @ 1..=10) => {
                        s.countdown_seconds = n;
                        Ok(())
                    }
                    _ => Err(format!("{value:?} is not 1 to 10 seconds")),
                },
                "palette" => {
                    s.palette = value.to_ascii_lowercase();
                    Ok(())
                }
                "share_server" if share_server_ok(value) => {
                    s.share_server = value.trim_end_matches('/').to_string();
                    Ok(())
                }
                "share_server" => Err(format!(
                    "{value:?} isn't an https:// address (http:// only for this computer)"
                )),
                _ => Err(format!("unknown setting {key:?}")),
            };
            if let Err(e) = result {
                problems.push(format!("line {}: {e}", n + 1));
            }
        }
        (s, problems)
    }

    /// Each setting's key and its value as the file writes it, in the
    /// file's order.
    fn entries(&self) -> [(&'static str, String); 9] {
        [
            ("capture", self.capture.to_string()),
            ("reopen", self.reopen.to_string()),
            ("quit", self.quit_text()),
            ("print_screen", self.print_screen.to_string()),
            ("countdown", optional_text(self.countdown)),
            ("countdown_seconds", self.countdown_seconds.to_string()),
            ("palette", self.palette.clone()),
            ("share_server", self.share_server.clone()),
            ("sync", self.sync.to_string()),
        ]
    }

    /// The settings as a commented file.
    pub fn to_text(&self) -> String {
        let mut out = HEADER.to_string();
        for (key, value) in self.entries() {
            let doc = DOCS.iter().find(|(k, _)| *k == key).map_or("", |(_, d)| d);
            out.push('\n');
            for line in doc.lines() {
                out.push_str("# ");
                out.push_str(line);
                out.push('\n');
            }
            out.push_str(&format!(
                "{key} = {value}
"
            ));
        }
        out
    }

    /// `text`, the file as it is, with each setting's value written over
    /// the old one, keeping comments, order and anything else in it.
    /// Settings it lacks are added at the end with their comments.
    pub fn update_text(&self, text: &str) -> String {
        let entries = self.entries();
        let mut seen = [false; 9];
        let mut out = String::with_capacity(text.len() + 64);
        for line in text.lines() {
            let trimmed = line.trim_start();
            let known = (!trimmed.starts_with('#'))
                .then(|| trimmed.split_once('='))
                .flatten()
                .and_then(|(key, _)| entries.iter().position(|(k, _)| *k == key.trim()));
            match known {
                Some(i) => {
                    seen[i] = true;
                    let indent = &line[..line.len() - trimmed.len()];
                    let (key, value) = &entries[i];
                    out.push_str(&format!("{indent}{key} = {value}"));
                }
                None => out.push_str(line),
            }
            out.push('\n');
        }
        let missing: Vec<usize> = (0..entries.len()).filter(|&i| !seen[i]).collect();
        if !missing.is_empty() {
            let full = self.to_text();
            let blocks: Vec<&str> = full
                .split(
                    "

",
                )
                .collect();
            for i in missing {
                let key = entries[i].0;
                let prefix = format!("{key} = ");
                if let Some(block) = blocks
                    .iter()
                    .find(|b| b.lines().any(|l| l.starts_with(&prefix)))
                {
                    out.push('\n');
                    out.push_str(block.trim_end());
                    out.push('\n');
                }
            }
        }
        out
    }

    /// Load from `path`, creating it with the defaults if it doesn't exist.
    pub fn load(path: &Path) -> io::Result<(Settings, Vec<String>)> {
        match fs::read_to_string(path) {
            Ok(text) => Ok(Settings::parse(&text)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let s = Settings::default();
                s.save(path)?;
                Ok((s, Vec::new()))
            }
            Err(e) => Err(e),
        }
    }

    /// Write the settings to `path`, keeping what else is in the file.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = match fs::read_to_string(path) {
            Ok(old) => self.update_text(&old),
            Err(e) if e.kind() == io::ErrorKind::NotFound => self.to_text(),
            Err(e) => return Err(e),
        };
        // `Config::watch` may read it at any moment.
        crate::write_atomic(path, text.as_bytes())
    }

    /// The quit shortcut as written in the file: "none" when there isn't one.
    fn quit_text(&self) -> String {
        optional_text(self.quit)
    }

    /// The shortcuts in one line, for the log.
    pub fn summary(&self) -> String {
        format!(
            "capture {}, reopen {}, countdown {}, quit {}{}",
            self.capture,
            self.reopen,
            optional_text(self.countdown),
            self.quit_text(),
            if self.print_screen {
                ", and Print Screen captures"
            } else {
                ""
            }
        )
    }

    /// Every shortcut with what it does, for registering and for menus.
    pub fn shortcuts(&self) -> Vec<(Action, Shortcut)> {
        let mut all = vec![
            (Action::Capture, self.capture),
            (Action::Reopen, self.reopen),
        ];
        if let Some(q) = self.quit {
            all.push((Action::Quit, q));
        }
        if let Some(c) = self.countdown {
            all.push((Action::Countdown, c));
        }
        if self.print_screen {
            all.push((Action::Capture, Shortcut::PRINT_SCREEN));
        }
        all
    }
}

/// The top of a new settings file.
const HEADER: &str = "# Spooch settings. Spooch reloads this file when it's saved. Custom
# frames go in the frames folder next to it (see the README there).
#
# Shortcuts are modifiers (Ctrl, Alt, Shift, Super) and one key: a letter,
# a digit, F1 to F24, or PrintScreen. For example: Ctrl+Shift+2, Alt+F9.
";

/// The comment above each setting in a new file.
const DOCS: [(&str, &str); 9] = [
    ("capture", "Freeze the screen and select a region."),
    (
        "reopen",
        "Reopen the newest capture from Recents, marks still editable.",
    ),
    (
        "quit",
        "Quit Spooch from a shortcut, or none. The tray menu has Quit too.",
    ),
    (
        "print_screen",
        "Also capture with the Print Screen key on its own (true or false).",
    ),
    (
        "countdown",
        "Capture after a countdown, to catch open menus and hover states, or none.\n\
         Press it again during the countdown to call it off.",
    ),
    (
        "countdown_seconds",
        "How long the countdown runs, 1 to 10 seconds.",
    ),
    (
        "palette",
        "The mark colours new captures start with: vivid, soft or ink. The style\n\
         row's colour button switches them too.",
    ),
    (
        "share_server",
        "Where captures are shared to. Sign in from the tray menu.",
    ),
    (
        "sync",
        "Sync captures with your account (true or false), once signed in. Nothing\n\
         is uploaded until this is on.",
    ),
];

/// What a global shortcut does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Capture,
    Reopen,
    Quit,
    /// Capture after `countdown_seconds`, or call a running countdown off.
    Countdown,
}

/// A shortcut, or "none" (or nothing) for no shortcut.
fn parse_optional(v: &str) -> Result<Option<Shortcut>, String> {
    match v.to_ascii_lowercase().as_str() {
        "" | "none" | "off" => Ok(None),
        _ => Shortcut::parse(v).map(Some),
    }
}

fn parse_bool(v: &str) -> Result<bool, String> {
    match v.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Ok(true),
        "false" | "no" | "off" | "0" => Ok(false),
        _ => Err(format!("{v:?} is not true or false")),
    }
}

/// Whether the person has seen the first-run welcome, kept as an empty
/// marker file next to the settings.
pub fn welcomed(marker: &Path) -> bool {
    marker.exists()
}

pub fn mark_welcomed(marker: &Path) -> io::Result<()> {
    if let Some(dir) = marker.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(marker, b"")
}

/// A shortcut as written in the file, "none" when there isn't one.
fn optional_text(s: Option<Shortcut>) -> String {
    s.map_or_else(|| "none".to_string(), |s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_share_server_is_https_or_this_computer() {
        for ok in [
            "https://api.spooch.app",
            "http://localhost:8080",
            "http://127.0.0.1",
        ] {
            assert!(share_server_ok(ok), "{ok}");
        }
        for bad in [
            "http://api.spooch.app",
            "http://localhost.evil.test",
            "ftp://x",
        ] {
            assert!(!share_server_ok(bad), "{bad}");
        }
        let (s, problems) = Settings::parse("share_server = http://example.com\n");
        assert_eq!(problems.len(), 1);
        assert_eq!(s.share_server, Settings::default().share_server);
    }

    #[test]
    fn shortcuts_parse_and_print() {
        let s = Shortcut::parse("ctrl + shift + 2").unwrap();
        assert_eq!(s, Settings::default().capture);
        assert_eq!(s.to_string(), "Ctrl+Shift+2");
        assert_eq!(
            Shortcut::parse("Shift+Alt+Ctrl+2").unwrap().to_string(),
            "Ctrl+Alt+Shift+2"
        );
        assert_eq!(Shortcut::parse("Alt+f9").unwrap().key, Key::F(9));
        assert_eq!(Shortcut::parse("Super+s").unwrap().to_string(), "Super+S");
        assert_eq!(Shortcut::parse("PrtScn").unwrap().key, Key::PrintScreen);
        assert!(Shortcut::parse("Ctrl+Shift").is_err());
        assert!(Shortcut::parse("Ctrl+A+B").is_err());
        assert!(Shortcut::parse("Ctrl+F25").is_err());
        assert!(Shortcut::parse("Ctrl++2").is_err());
    }

    #[test]
    fn the_file_round_trips_and_bad_lines_keep_defaults() {
        let s = Settings {
            capture: Shortcut::parse("Ctrl+Alt+S").unwrap(),
            print_screen: true,
            ..Settings::default()
        };
        let (back, problems) = Settings::parse(&s.to_text());
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(back, s);

        let (s, problems) = Settings::parse(
            "capture = Ctrl+Nope\nreopen=Alt+R\nprint_screen = maybe\ncolour = red\njunk\n",
        );
        assert_eq!(s.capture, Settings::default().capture);
        assert_eq!(s.reopen.to_string(), "Alt+R");
        assert!(!s.print_screen);
        assert_eq!(problems.len(), 4, "{problems:?}");
    }

    #[test]
    fn saving_keeps_comments_and_other_lines() {
        let old = "# mine
capture = Ctrl+Shift+2   
  # about quit
  quit = none
future_key = 1
";
        let s = Settings {
            capture: Shortcut::parse("Alt+F9").unwrap(),
            sync: true,
            ..Settings::default()
        };
        let text = s.update_text(old);
        assert!(
            text.starts_with(
                "# mine
capture = Alt+F9
  # about quit
  quit = none
future_key = 1
"
            ),
            "{text}"
        );
        // What the file lacked comes after, with its comment.
        assert!(text.contains("# Sync captures with your account"));
        let (back, problems) = Settings::parse(&text);
        assert_eq!(back, s);
        assert_eq!(problems.len(), 1, "only the unknown key");
        // A whole file round trips unchanged.
        assert_eq!(s.update_text(&s.to_text()), s.to_text());
    }

    #[test]
    fn print_screen_adds_a_capture_shortcut() {
        let mut s = Settings::default();
        assert_eq!(s.shortcuts().len(), 3, "capture, reopen and countdown");
        s.print_screen = true;
        let all = s.shortcuts();
        assert_eq!(all.len(), 4);
        assert_eq!(all[3].0, Action::Capture);
        assert_eq!(all[3].1.key, Key::PrintScreen);
    }

    #[test]
    fn countdown_has_a_shortcut_and_a_length() {
        let s = Settings::default();
        assert_eq!(
            s.countdown.map(|c| c.to_string()).as_deref(),
            Some("Ctrl+Shift+3")
        );
        assert_eq!(s.countdown_seconds, 3);
        let (s, problems) = Settings::parse(
            "countdown = none
countdown_seconds = 5
",
        );
        assert_eq!((s.countdown, s.countdown_seconds), (None, 5));
        assert!(problems.is_empty());
        assert!(!s.shortcuts().iter().any(|(a, _)| *a == Action::Countdown));
        let (s, problems) = Settings::parse(
            "countdown_seconds = 60
",
        );
        assert_eq!((s.countdown_seconds, problems.len()), (3, 1));
        // It survives a round trip through the file.
        let (back, _) = Settings::parse(&Settings::default().to_text());
        assert_eq!(back, Settings::default());
    }

    #[test]
    fn quit_has_no_shortcut_unless_set() {
        let s = Settings::default();
        assert_eq!(s.quit, None);
        assert!(s.to_text().contains("quit = none"));
        assert!(s.shortcuts().iter().all(|(a, _)| *a != Action::Quit));
        let (s, problems) = Settings::parse("quit = Ctrl+Alt+Q");
        assert!(problems.is_empty());
        assert_eq!(s.quit_text(), "Ctrl+Alt+Q");
        assert!(s.shortcuts().iter().any(|(a, _)| *a == Action::Quit));
        let (back, _) = Settings::parse(&s.to_text());
        assert_eq!(back, s);
        assert_eq!(Settings::parse("quit = none").0.quit, None);
        assert_eq!(Settings::parse("quit =").0.quit, None);
    }

    #[test]
    fn load_creates_the_file_with_defaults() {
        let dir = std::env::temp_dir().join(format!("spooch-settings-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("settings.txt");
        let (s, problems) = Settings::load(&path).unwrap();
        assert_eq!(s, Settings::default());
        assert!(problems.is_empty());
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("capture = Ctrl+Shift+2")
        );
        let marker = dir.join("welcomed");
        assert!(!welcomed(&marker));
        mark_welcomed(&marker).unwrap();
        assert!(welcomed(&marker));
        fs::remove_dir_all(&dir).unwrap();
    }
}
