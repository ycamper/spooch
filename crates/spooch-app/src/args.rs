//! What Spooch was started with, the same on every OS. Anything it doesn't
//! know is ignored.

use std::path::PathBuf;

use spooch_platform::log;
use spooch_platform::recents::Recents;

/// The command line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Args {
    /// `--backend=NAME`: force a capture backend (the shell says which
    /// exist).
    pub backend: Option<String>,
    /// `--no-recents`: keep nothing.
    pub no_recents: bool,
    /// `--editor`: open the newest capture in the editor window at start.
    pub editor: bool,
    /// `--welcome`: show the first-run welcome again.
    pub welcome: bool,
    /// `--once`: capture straight away, then quit (for tests).
    pub once: bool,
    /// `--save=PATH`: also write each copied PNG here (for tests).
    pub save: Option<String>,
}

impl Args {
    /// Recents in `root`, unless `--no-recents` or there's no root. Logs
    /// which.
    pub fn recents(&self, root: Option<PathBuf>) -> Option<Recents> {
        if self.no_recents {
            log::line("recents off (--no-recents)");
            return None;
        }
        let root = root?;
        log::line(format!("recents in {}", root.display()));
        Some(Recents::new(root))
    }

    /// This process's arguments.
    pub fn from_env() -> Args {
        Args::parse(std::env::args().skip(1))
    }

    pub fn parse(args: impl IntoIterator<Item = String>) -> Args {
        let mut a = Args::default();
        for arg in args {
            match arg.as_str() {
                "--no-recents" => a.no_recents = true,
                "--editor" => a.editor = true,
                "--welcome" => a.welcome = true,
                "--once" => a.once = true,
                _ => {
                    if let Some(v) = arg.strip_prefix("--backend=") {
                        a.backend = Some(v.to_string());
                    } else if let Some(v) = arg.strip_prefix("--save=") {
                        a.save = Some(v.to_string());
                    }
                }
            }
        }
        a
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_flags_and_values() {
        let a = Args::parse(
            [
                "--backend=gdi",
                "--no-recents",
                "--welcome",
                "--save=/tmp/x.png",
                "--what",
            ]
            .map(String::from),
        );
        assert_eq!(a.backend.as_deref(), Some("gdi"));
        assert!(a.no_recents && a.welcome && !a.editor && !a.once);
        assert_eq!(a.save.as_deref(), Some("/tmp/x.png"));
        assert_eq!(Args::parse(Vec::new()), Args::default());
    }
}
