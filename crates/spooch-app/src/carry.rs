//! What carries on from one capture to the next: the styles and format
//! last picked, where captures are kept, sharing and syncing.

use std::path::PathBuf;
use std::sync::Mutex;

use spooch_platform::log;
use spooch_platform::recents::Recents;
use spooch_platform::save::LocalTime;
use spooch_platform::settings::Settings;
use spooch_render::skia_safe::IRect;
use spooch_render::{Format, Styles};

use crate::{Config, Session, Setup, Sharing, Syncing};

/// The shell keeps one, for every capture and editor window.
#[derive(Clone, Default)]
pub struct Carry {
    pub styles: Styles,
    pub format: Format,
    /// `None` with `--no-recents`.
    pub recents: Option<Recents>,
    pub sharing: Sharing,
    pub sync: Syncing,
    /// Where the picks are kept between runs, and what was last written
    /// there.
    kept_in: Option<(PathBuf, String)>,
}

impl Carry {
    /// What the shell starts with: sharing and syncing as `settings` say.
    pub fn new(styles: Styles, recents: Option<Recents>, settings: &Settings) -> Carry {
        let sharing = Sharing::new(&settings.share_server);
        let sync = Syncing::start(recents.clone(), sharing.clone(), settings.sync);
        Carry {
            styles,
            format: Format::default(),
            recents,
            sharing,
            sync,
            kept_in: None,
        }
    }

    /// Start from `format`, and keep the picks in `path` whenever they
    /// change, so they survive a restart.
    pub fn keeping_picks_in(mut self, path: PathBuf, format: Format) -> Carry {
        self.format = format;
        self.kept_in = Some((path, self.styles.remembered(format)));
        self
    }

    /// Switch syncing on or off, and keep that in the settings.
    pub fn toggle_sync(&mut self, settings: &mut Settings, config: &Config) {
        settings.sync = !settings.sync;
        config.save(settings);
        self.sync.set_on(settings.sync);
        log::line(if settings.sync { "sync on" } else { "sync off" });
    }

    /// What a capture made at `when` starts with.
    pub fn setup(&self, when: LocalTime) -> Setup {
        Setup {
            styles: self.styles,
            format: self.format,
            recents: self.recents.clone(),
            sharing: self.sharing.clone(),
            sync: self.sync.clone(),
            when,
        }
    }

    /// Share to `server`, and sync with the account there when `sync`, as
    /// settings say; the sign-in carries on unless the server changed.
    pub fn share_to(&mut self, server: &str, sync: bool) {
        if self.sharing.server() != server {
            // Open sessions still hold the old one; it mustn't keep syncing
            // the same Recents with the old account.
            self.sync.set_on(false);
            self.sharing = Sharing::new(server);
            self.sync = Syncing::start(self.recents.clone(), self.sharing.clone(), sync);
        } else {
            self.sync.set_on(sync);
        }
    }

    /// Keep what `s` has picked for the next capture.
    pub fn remember(&mut self, s: &Session) {
        self.styles = s.doc.styles();
        self.format = s.doc.format();
        self.keep_picks();
    }

    /// Write the picks out if they changed, on a thread so closing never
    /// waits on the disk.
    fn keep_picks(&mut self) {
        let Some((path, written)) = &mut self.kept_in else {
            return;
        };
        let json = self.styles.remembered(self.format);
        if *written == json {
            return;
        }
        written.clone_from(&json);
        // Writers take the newest picks, one at a time, so a slow write
        // can't land after a newer one.
        static NEWEST: Mutex<Option<(PathBuf, String)>> = Mutex::new(None);
        static WRITING: Mutex<()> = Mutex::new(());
        *NEWEST.lock().unwrap_or_else(|e| e.into_inner()) = Some((path.clone(), json));
        std::thread::spawn(|| {
            let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
            let Some((path, json)) = NEWEST.lock().unwrap_or_else(|e| e.into_inner()).take() else {
                return;
            };
            let tmp = path.with_extension("json.tmp");
            if let Err(e) = std::fs::write(&tmp, &json).and_then(|_| std::fs::rename(&tmp, &path)) {
                log::line(format!(
                    "could not keep the picks in {}: {e}",
                    path.display()
                ));
            }
        });
    }

    /// `s` is closing: remember its picks and keep it in Recents in the
    /// background. `done` gets the log line.
    pub fn retire(&mut self, s: &mut Session, done: impl FnOnce(String) + Send + 'static) {
        self.remember(s);
        s.keep_in_background(done);
    }

    /// Kept capture `id`, or the newest, for the editor window.
    pub fn open_editor(&self, id: Option<String>) -> Result<(Session, String), String> {
        Session::open_editor(self.recents.as_ref(), id, self.styles)
            .map(|(s, line)| (self.hand_on(s), line))
    }

    /// Kept capture `id` over the frozen screen, on `screens`.
    pub fn reopen(&self, id: &str, screens: Vec<IRect>) -> Result<(Session, String), String> {
        Session::reopen(self.recents.as_ref(), id, screens, self.styles)
            .map(|(s, line)| (self.hand_on(s), line))
    }

    /// `s`, sharing and syncing as the rest do.
    fn hand_on(&self, s: Session) -> Session {
        s.with_sharing(self.sharing.clone())
            .with_sync(self.sync.clone())
    }

    /// The newest kept capture, if there is one.
    pub fn latest(&self) -> Option<String> {
        self.recents.as_ref()?.latest()
    }
}
