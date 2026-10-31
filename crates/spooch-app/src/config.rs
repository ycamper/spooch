//! Where Spooch keeps its settings, custom frames and the markers it
//! leaves itself, and reading them. The shell names the folder
//! (`%APPDATA%\Spooch`, `$XDG_CONFIG_HOME/spooch`); the rest is the same
//! everywhere.

use std::io;
use std::path::PathBuf;
use std::time::SystemTime;

use spooch_platform::log;
use spooch_platform::settings::{self, Settings};
use spooch_render::{Format, Styles};

/// The settings folder.
#[derive(Clone, Debug)]
pub struct Config {
    dir: PathBuf,
}

/// Settings as read, the styles they set, and anything wrong with them or
/// with the custom frames, already logged. At startup, also the picks and
/// format kept from the last run.
pub struct Loaded {
    pub settings: Settings,
    pub styles: Styles,
    pub format: Format,
    pub problems: Vec<String>,
}

impl Config {
    pub fn new(dir: PathBuf) -> Config {
        Config { dir }
    }

    pub fn settings_path(&self) -> PathBuf {
        self.dir.join("settings.txt")
    }

    /// People's own frames, next to the settings.
    pub fn frames_dir(&self) -> PathBuf {
        self.dir.join("frames")
    }

    /// The last picks (`Styles::remembered`), kept between runs.
    pub fn styles_path(&self) -> PathBuf {
        self.dir.join("styles.json")
    }

    /// A file Spooch leaves itself to remember something, like having
    /// shown the welcome.
    pub fn marker(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// Read the settings, apply them to `styles` and load the custom
    /// frames. An error means the file couldn't be read at all.
    pub fn load(&self, styles: Styles) -> io::Result<Loaded> {
        let (settings, problems) = Settings::load(&self.settings_path())?;
        Ok(self.apply(settings, problems, styles))
    }

    /// `load` at startup: the defaults when the file can't be read, and
    /// the picks kept from the last run on top.
    pub fn load_or_default(&self, styles: Styles) -> Loaded {
        let mut loaded = self.load(styles).unwrap_or_else(|e| {
            log::line(format!("could not read settings, using defaults: {e}"));
            self.apply(Settings::default(), Vec::new(), styles)
        });
        let path = self.styles_path();
        if let Ok(json) = std::fs::read_to_string(&path) {
            match loaded.styles.with_remembered(&json) {
                Ok((styles, format)) => {
                    loaded.styles = styles;
                    loaded.format = format.unwrap_or_default();
                }
                Err(e) => log::line(format!("could not read {}: {e}", path.display())),
            }
        }
        loaded
    }

    fn apply(&self, settings: Settings, mut problems: Vec<String>, styles: Styles) -> Loaded {
        let (styles, problem) = crate::apply_settings(styles, &settings);
        problems.extend(problem);
        problems.extend(spooch_render::load_frames(&self.frames_dir()));
        for p in &problems {
            log::line(format!("settings: {p}"));
        }
        Loaded {
            settings,
            styles,
            format: Format::default(),
            problems,
        }
    }

    pub fn save(&self, settings: &Settings) {
        if let Err(e) = settings.save(&self.settings_path()) {
            log::line(format!("could not save settings: {e}"));
        }
    }

    /// Whether to show the first-run welcome: once, or again when `forced`
    /// (`--welcome`).
    pub fn wants_welcome(&self, forced: bool) -> bool {
        forced || !settings::welcomed(&self.marker("welcomed"))
    }

    pub fn mark_welcomed(&self) {
        if let Err(e) = settings::mark_welcomed(&self.marker("welcomed")) {
            log::line(format!("could not save the welcome marker: {e}"));
        }
    }

    /// Call `changed` whenever the settings file or the frames folder
    /// changes, from a thread that looks once a second, so edits take
    /// effect when they're saved.
    pub fn watch(&self, changed: impl Fn() + Send + 'static) {
        let (file, frames) = (self.settings_path(), self.frames_dir());
        std::thread::spawn(move || {
            let mut seen = stamp(&file, &frames);
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                let now = stamp(&file, &frames);
                if now != seen {
                    seen = now;
                    changed();
                }
            }
        });
    }
}

/// What `watch` compares: the settings file's modified time, and each
/// frame file's name and modified time.
fn stamp(file: &std::path::Path, frames: &std::path::Path) -> Vec<(String, Option<SystemTime>)> {
    let modified = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let mut out = vec![(String::new(), modified(file))];
    let mut entries: Vec<_> = std::fs::read_dir(frames)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                modified(&e.path()),
            )
        })
        .collect();
    entries.sort();
    out.extend(entries);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_remembers_in_its_folder() {
        let dir = std::env::temp_dir().join(format!("spooch-config-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config = Config::new(dir.clone());
        // No file yet: the defaults, nothing wrong.
        let loaded = config.load(Styles::default()).unwrap();
        assert!(loaded.problems.is_empty());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(config.settings_path(), "palette = neon\n").unwrap();
        let loaded = config.load_or_default(Styles::default());
        assert!(loaded.problems.iter().any(|p| p.contains("neon")));

        assert!(config.wants_welcome(false));
        config.mark_welcomed();
        assert!(!config.wants_welcome(false));
        assert!(config.wants_welcome(true));

        // Saving the file or adding a frame changes what the watcher sees.
        let before = stamp(&config.settings_path(), &config.frames_dir());
        std::fs::create_dir_all(config.frames_dir()).unwrap();
        std::fs::write(config.frames_dir().join("mine.toml"), "").unwrap();
        assert_ne!(stamp(&config.settings_path(), &config.frames_dir()), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn startup_brings_back_the_last_picks() {
        let dir = std::env::temp_dir().join(format!("spooch-picks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config = Config::new(dir.clone());
        let fresh = config.load_or_default(Styles::default());
        assert_eq!(fresh.format, Format::Png);
        let json = r#"{"tools": {"box": {"color": 4, "size": 4}}, "format": "jpg"}"#;
        std::fs::write(config.styles_path(), json).unwrap();
        let loaded = config.load_or_default(Styles::default());
        assert_eq!(loaded.format, Format::Jpeg);
        assert_ne!(loaded.styles, fresh.styles);
        // A broken file is logged and ignored.
        std::fs::write(config.styles_path(), "{").unwrap();
        assert_eq!(
            config.load_or_default(Styles::default()).styles,
            fresh.styles
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
