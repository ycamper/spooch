//! The log: every line goes to stderr and to `spooch.log` in the folder
//! the shell names with `init` (`%LOCALAPPDATA%\Spooch`,
//! `$XDG_STATE_HOME/spooch`).
//!
//! Lines go through a channel to a logger thread. The UI thread must never
//! block on output: a console in QuickEdit selection mode, or a pipe nobody
//! drains, stalls every write, and a stalled UI thread is a hung app.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::mpsc::{self, Sender, SyncSender};
use std::time::{Duration, Instant};

enum Msg {
    Line(String),
    /// Acknowledged once every earlier line is written.
    Flush(SyncSender<()>),
}

static DIR: OnceLock<PathBuf> = OnceLock::new();
static LOGGER: OnceLock<Sender<Msg>> = OnceLock::new();

/// Write the log file into `dir` from now on. Lines logged before this
/// only reach stderr.
pub fn init(dir: PathBuf) {
    let _ = DIR.set(dir);
}

/// Log panics before the default report: a windowed app's stderr goes
/// nowhere, and a detached one's is easily lost.
pub fn catch_panics() {
    let report = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        line(format!("panic: {info}"));
        flush();
        report(info);
    }));
}

fn open_file() -> Option<File> {
    let dir = DIR.get()?;
    fs::create_dir_all(dir).ok()?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("spooch.log"))
        .ok()
}

fn logger() -> &'static Sender<Msg> {
    LOGGER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Msg>();
        std::thread::spawn(move || {
            let mut file = None;
            for msg in rx {
                let msg = match msg {
                    Msg::Line(msg) => msg,
                    Msg::Flush(done) => {
                        let _ = done.send(());
                        continue;
                    }
                };
                if file.is_none() {
                    file = open_file();
                }
                // The file first: it is what people paste back to us.
                if let Some(f) = file.as_mut() {
                    let _ = writeln!(f, "{msg}");
                    let _ = f.flush();
                }
                let _ = writeln!(std::io::stderr().lock(), "{msg}");
            }
        });
        tx
    })
}

pub fn line(msg: impl Into<String>) {
    let _ = logger().send(Msg::Line(msg.into()));
}

/// Wait until every queued line is written. Only for exit paths, where
/// blocking is fine; gives up after a second so a stuck console cannot
/// keep the process alive.
pub fn flush() {
    let (tx, rx) = mpsc::sync_channel(1);
    if logger().send(Msg::Flush(tx)).is_ok() {
        let _ = rx.recv_timeout(Duration::from_secs(1));
    }
}

/// Milliseconds since `started`, for the log.
pub fn ms_since(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

/// Milestones within one capture, measured from its start (the hotkey
/// press), logged together by `report`.
pub struct Stopwatch {
    start: Instant,
    marks: Vec<(String, f64)>,
}

impl Stopwatch {
    pub fn start() -> Self {
        Self {
            start: Instant::now(),
            marks: Vec::new(),
        }
    }

    pub fn mark(&mut self, label: impl Into<String>) {
        let ms = self.ms();
        self.marks.push((label.into(), ms));
    }

    /// Milliseconds since the start.
    pub fn ms(&self) -> f64 {
        ms_since(self.start)
    }

    /// Log `title` with the time so far, then each milestone.
    pub fn report(&self, title: &str) {
        line(format!("--- {title}: {:.1} ms", self.ms()));
        for (label, ms) in &self.marks {
            line(format!("  {ms:8.1} ms  {label}"));
        }
    }
}
