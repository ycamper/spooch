//! The app's flow around the editor, the same on every platform: what the
//! tray menu shows, and what global shortcuts, menu picks and the countdown
//! do. The shells draw the menu, keep the timers and windows, and carry out
//! the `Effect`s; anything an OS does differently (Print Screen, start at
//! sign-in, dialogs) stays with them.

use crate::settings::{Action, Settings};

/// What a tray menu item does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Capture,
    /// Capture after the countdown.
    Countdown,
    /// Open the editor window on the newest capture; its gallery has the
    /// rest.
    OpenEditor,
    /// Show the picture saved last, selected in its folder.
    ShowSaved,
    TogglePrintScreen,
    ToggleLaunchAtLogin,
    EditSettings,
    /// Make an account; sign in to one (with a name and password, or a
    /// copied device token); set its password; sign out.
    CreateAccount,
    SignIn,
    SetPassword,
    SignOut,
    /// Sync captures with the account, or stop.
    ToggleSync,
    Quit,
}

/// Whether sharing is signed in, and as whom once the service has said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Signed {
    Out,
    In(Option<String>),
}

/// One line of the tray menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuItem {
    /// `label`, its shortcut if it has one, what it does (`None` greys it
    /// out), and a tick for switches.
    Entry {
        label: String,
        key: Option<String>,
        command: Option<Command>,
        checked: Option<bool>,
    },
    Separator,
}

impl MenuItem {
    fn entry(label: impl Into<String>, command: Option<Command>) -> MenuItem {
        MenuItem::Entry {
            label: label.into(),
            key: None,
            command,
            checked: None,
        }
    }
}

/// The tray menu: capturing, the editor, the switches and the settings
/// file (Spooch reloads it when it's saved), sharing, and Quit. Recents
/// are browsed in the editor's gallery. `login_label` is the OS's wording for starting
/// with the computer ("Start when I sign in" on Windows); without
/// `captures` there's nothing for the editor to open yet.
pub fn tray_menu(
    settings: &Settings,
    captures: bool,
    launch_at_login: bool,
    login_label: &str,
    sharing: &Signed,
) -> Vec<MenuItem> {
    let keyed = |label: String, key: Option<String>, command: Command| MenuItem::Entry {
        label,
        key,
        command: Some(command),
        checked: None,
    };
    let switch = |label: &str, on: bool, command: Command| MenuItem::Entry {
        label: label.into(),
        key: None,
        command: Some(command),
        checked: Some(on),
    };
    let mut items = vec![
        keyed(
            "Capture".into(),
            Some(settings.capture.to_string()),
            Command::Capture,
        ),
        keyed(
            format!("Capture in {} seconds", settings.countdown_seconds),
            settings.countdown.map(|c| c.to_string()),
            Command::Countdown,
        ),
        MenuItem::entry("Open editor", captures.then_some(Command::OpenEditor)),
        MenuItem::entry("Show saved pictures", Some(Command::ShowSaved)),
        MenuItem::Separator,
        switch(
            "Use the Print Screen key",
            settings.print_screen,
            Command::TogglePrintScreen,
        ),
        switch(login_label, launch_at_login, Command::ToggleLaunchAtLogin),
        MenuItem::entry("Settings...", Some(Command::EditSettings)),
        MenuItem::Separator,
    ];
    // The account: who's signed in, greyed out, above what to do with it.
    match sharing {
        Signed::Out => {
            items.push(MenuItem::entry(
                "Create account...",
                Some(Command::CreateAccount),
            ));
            items.push(MenuItem::entry("Sign in...", Some(Command::SignIn)));
        }
        Signed::In(account) => {
            let who = match account {
                Some(name) => format!("Signed in as {name}"),
                None => "Signed in".into(),
            };
            items.push(MenuItem::entry(who, None));
            items.push(switch("Sync captures", settings.sync, Command::ToggleSync));
            items.push(MenuItem::entry(
                "Set password...",
                Some(Command::SetPassword),
            ));
            items.push(MenuItem::entry("Sign out", Some(Command::SignOut)));
        }
    }
    items.push(MenuItem::Separator);
    items.push(MenuItem::entry("Quit Spooch", Some(Command::Quit)));
    items
}

/// Something the shell has to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Freeze the screen and start selecting.
    Capture,
    /// Open the newest kept capture, if there is one.
    ReopenLatest,
    /// Open the editor window on this kept capture, or the newest.
    OpenEditor(Option<String>),
    /// Show the countdown badge with this number, creating it and its
    /// one-second timer if it isn't up yet. Call `Flow::tick` each second.
    ShowCountdown(u32),
    /// Take the badge off the screen and stop its timer.
    HideCountdown,
    TogglePrintScreen,
    ToggleLaunchAtLogin,
    /// Open the settings file.
    EditSettings,
    /// Show the saved pictures: `save::to_show` says what to select or
    /// open.
    ShowSaved,
    /// Ask for an account's details and make it.
    CreateAccount,
    /// Sign in with a copied device token, else ask for a name and
    /// password.
    SignIn,
    /// Ask for the account's new password.
    SetPassword,
    /// Revoke this device on the service and forget its token.
    SignOut,
    /// Switch syncing captures on or off, and save that.
    ToggleSync,
    Quit,
    Log(String),
}

/// The state behind the shortcuts and the menu: the countdown.
#[derive(Debug, Default)]
pub struct Flow {
    /// Seconds left on the countdown, while one runs.
    countdown: Option<u32>,
}

impl Flow {
    fn counting_down(&self) -> bool {
        self.countdown.is_some()
    }

    /// A global shortcut was pressed. `busy` is true while a capture is
    /// open; only Quit works then.
    pub fn shortcut(&mut self, action: Action, settings: &Settings, busy: bool) -> Vec<Effect> {
        match action {
            Action::Quit => vec![Effect::Log("quit (shortcut)".into()), Effect::Quit],
            _ if busy => Vec::new(),
            Action::Capture => self.then_stop(Effect::Capture),
            // Pressed again during the countdown, it calls it off.
            Action::Countdown if self.counting_down() => {
                let mut e = self.stop();
                e.push(Effect::Log("countdown called off".into()));
                e
            }
            Action::Countdown => self.start(settings),
            Action::Reopen => self.then_stop(Effect::ReopenLatest),
        }
    }

    /// A tray menu item was picked.
    pub fn command(&mut self, command: Command, settings: &Settings, busy: bool) -> Vec<Effect> {
        match command {
            Command::Quit => vec![Effect::Log("quit (tray menu)".into()), Effect::Quit],
            _ if busy => Vec::new(),
            Command::Capture => self.then_stop(Effect::Capture),
            Command::Countdown => self.start(settings),
            Command::OpenEditor => vec![Effect::OpenEditor(None)],
            Command::TogglePrintScreen => vec![Effect::TogglePrintScreen],
            Command::ToggleLaunchAtLogin => vec![Effect::ToggleLaunchAtLogin],
            Command::EditSettings => vec![Effect::EditSettings],
            Command::ShowSaved => vec![Effect::ShowSaved],
            Command::CreateAccount => vec![Effect::CreateAccount],
            Command::SignIn => vec![Effect::SignIn],
            Command::SetPassword => vec![Effect::SetPassword],
            Command::ToggleSync => vec![Effect::ToggleSync],
            Command::SignOut => vec![Effect::SignOut],
        }
    }

    /// A second of the countdown has passed. At the end the badge goes,
    /// then the capture starts (unless one is already open).
    pub fn tick(&mut self, busy: bool) -> Vec<Effect> {
        match self.countdown {
            None => Vec::new(),
            Some(n) if n > 1 => {
                self.countdown = Some(n - 1);
                vec![Effect::ShowCountdown(n - 1)]
            }
            Some(_) => {
                self.countdown = None;
                let mut e = vec![Effect::HideCountdown];
                if !busy {
                    e.push(Effect::Capture);
                }
                e
            }
        }
    }

    fn start(&mut self, settings: &Settings) -> Vec<Effect> {
        let n = u32::from(settings.countdown_seconds.max(1));
        self.countdown = Some(n);
        vec![
            Effect::ShowCountdown(n),
            Effect::Log(format!("countdown: capturing in {n} s")),
        ]
    }

    fn stop(&mut self) -> Vec<Effect> {
        match self.countdown.take() {
            Some(_) => vec![Effect::HideCountdown],
            None => Vec::new(),
        }
    }

    /// Call any countdown off, then do `then`.
    fn then_stop(&mut self, then: Effect) -> Vec<Effect> {
        let mut e = self.stop();
        e.push(then);
        e
    }
}

/// The first-run welcome's question. `tray_hint` says where the icon is,
/// ending in a blank line, or is empty.
pub fn welcome_text(settings: &Settings, tray_hint: &str) -> String {
    let countdown = settings
        .countdown
        .map(|c| format!(" {c} captures after a short countdown."))
        .unwrap_or_default();
    format!(
        "Spooch is running. {tray_hint}{} captures a region.{countdown} {} reopens the last \
         capture.\n\nUse the Print Screen key for Spooch too?",
        settings.capture, settings.reopen
    )
}

/// What to tell someone when another app holds some of the shortcuts at
/// startup, if any.
pub fn shortcuts_taken(taken: &[String]) -> Option<String> {
    (!taken.is_empty()).then(|| {
        format!(
            "Another app already uses {}. Pick other shortcuts with \"Settings...\" in \
             Spooch's tray menu.",
            taken.join(" and ")
        )
    })
}

/// What to tell someone after reloading settings: the lines that couldn't
/// be used and the shortcuts other apps hold, or `None` if all is well.
pub fn settings_problems(problems: &[String], taken: &[String]) -> Option<String> {
    let lines: Vec<String> = problems
        .iter()
        .cloned()
        .chain(taken.iter().map(|k| format!("{k} is taken by another app")))
        .collect();
    (!lines.is_empty()).then(|| {
        format!(
            "Some settings couldn't be used, so they keep their defaults:\n\n{}",
            lines.join("\n")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_countdown_counts_down_then_captures() {
        let s = Settings::default();
        let mut f = Flow::default();
        let e = f.shortcut(Action::Countdown, &s, false);
        assert_eq!(e[0], Effect::ShowCountdown(3));
        assert_eq!(f.tick(false), vec![Effect::ShowCountdown(2)]);
        assert_eq!(f.tick(false), vec![Effect::ShowCountdown(1)]);
        assert_eq!(f.tick(false), vec![Effect::HideCountdown, Effect::Capture]);
        assert!(!f.counting_down());
        assert!(f.tick(false).is_empty());
    }

    #[test]
    fn the_countdown_is_called_off_by_its_key_or_a_capture() {
        let s = Settings::default();
        let mut f = Flow::default();
        f.shortcut(Action::Countdown, &s, false);
        let e = f.shortcut(Action::Countdown, &s, false);
        assert_eq!(e[0], Effect::HideCountdown);
        assert!(!f.counting_down());
        f.command(Command::Countdown, &s, false);
        assert_eq!(
            f.shortcut(Action::Capture, &s, false),
            vec![Effect::HideCountdown, Effect::Capture]
        );
        // Ending while a capture is open just takes the badge away.
        f.shortcut(Action::Countdown, &s, false);
        f.tick(false);
        f.tick(false);
        assert_eq!(f.tick(true), vec![Effect::HideCountdown]);
    }

    #[test]
    fn only_quit_works_while_a_capture_is_open() {
        let s = Settings::default();
        let mut f = Flow::default();
        assert!(f.shortcut(Action::Capture, &s, true).is_empty());
        assert!(f.command(Command::OpenEditor, &s, true).is_empty());
        assert!(f.shortcut(Action::Quit, &s, true).contains(&Effect::Quit));
    }

    #[test]
    fn the_menu_lists_shortcuts_switches_and_quit() {
        let s = Settings {
            print_screen: true,
            ..Settings::default()
        };
        let items = tray_menu(&s, false, false, "Start when I sign in", &Signed::Out);
        assert_eq!(items.len(), 13);
        for command in [Command::ShowSaved, Command::CreateAccount] {
            assert!(
                items.iter().any(
                    |i| matches!(i, MenuItem::Entry { command: Some(c), .. } if *c == command)
                ),
                "{command:?}"
            );
        }
        assert!(items.iter().any(|i| matches!(
            i,
            MenuItem::Entry {
                command: Some(Command::SignIn),
                ..
            }
        )));
        let MenuItem::Entry { label, key, .. } = &items[0] else {
            panic!("not an entry")
        };
        assert_eq!(
            (label.as_str(), key.as_deref()),
            ("Capture", Some("Ctrl+Shift+2"))
        );
        let MenuItem::Entry { label, key, .. } = &items[1] else {
            panic!("not an entry")
        };
        assert_eq!(
            (label.as_str(), key.as_deref()),
            ("Capture in 3 seconds", Some("Ctrl+Shift+3"))
        );
        assert!(items.iter().any(|i| matches!(
            i,
            MenuItem::Entry {
                command: Some(Command::TogglePrintScreen),
                checked: Some(true),
                ..
            }
        )));
        // Nothing kept yet: Open editor is there, greyed out.
        assert!(items.iter().any(|i| matches!(
            i,
            MenuItem::Entry { label, command: None, .. } if label == "Open editor"
        )));
        assert!(matches!(
            items.last(),
            Some(MenuItem::Entry {
                command: Some(Command::Quit),
                ..
            })
        ));
    }

    #[test]
    fn signed_in_the_menu_says_who_and_offers_sign_out() {
        let s = Settings::default();
        let items = tray_menu(&s, true, false, "x", &Signed::In(Some("mark".into())));
        let at = items
            .iter()
            .position(|i| matches!(i, MenuItem::Entry { label, command: None, .. } if label == "Signed in as mark"))
            .unwrap();
        let commands: Vec<_> = items[at + 1..at + 4]
            .iter()
            .map(|i| match i {
                MenuItem::Entry { command, .. } => command.clone(),
                MenuItem::Separator => None,
            })
            .collect();
        assert_eq!(
            commands,
            [
                Some(Command::ToggleSync),
                Some(Command::SetPassword),
                Some(Command::SignOut)
            ]
        );
    }

    #[test]
    fn settings_problems_are_listed_or_none() {
        assert_eq!(settings_problems(&[], &[]), None);
        let m = settings_problems(&["line 3: bad".into()], &["Ctrl+Shift+3".into()]).unwrap();
        assert!(m.contains("line 3: bad") && m.contains("Ctrl+Shift+3 is taken"));
    }
}
