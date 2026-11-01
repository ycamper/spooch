//! What the settings window shows and does, the same on every OS: its
//! rows, made from the settings, and what a click or a recorded shortcut
//! on one changes. The shells own the window and carry out the `SettingsChange`s.

use spooch_platform::app::{Command, Signed};
use spooch_platform::settings::{Key as ShortcutKey, Settings, Shortcut};
use spooch_render::{Key, Mods, PALETTES, Palette, SettingsEvent, SettingsItem};

/// What the window needs to know besides the settings.
pub struct SettingsFacts<'a> {
    pub settings: &'a Settings,
    pub launch_at_login: bool,
    /// The OS's wording for starting with the computer.
    pub login_label: &'a str,
    pub sharing: &'a Signed,
}

/// What a row of the window is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Heading,
    Capture,
    Countdown,
    Reopen,
    Quit,
    PrintScreen,
    Seconds,
    Login,
    Palette,
    /// Sign in or make an account.
    SignedOut,
    /// Set the password or sign out.
    SignedIn,
    Sync,
    File,
}

/// What the shell has to do about a click.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsChange {
    /// Save these settings and use them: register the shortcuts again.
    Settings(Settings),
    /// What the tray item does: switches that touch the OS, and the
    /// account.
    Command(Command),
    /// Open the settings file.
    EditFile,
    /// Pause the global shortcuts (`true`) while one is being recorded,
    /// or bring them back.
    PauseShortcuts(bool),
    /// Say this in the window, as a shortcut already in use.
    Problem(String),
}

/// The settings window's rows.
#[derive(Default)]
pub struct SettingsForm {
    rows: Vec<Row>,
}

impl SettingsForm {
    /// The rows for `facts`, for `SettingsPage::set_items`.
    pub fn items(&mut self, facts: &SettingsFacts) -> Vec<SettingsItem> {
        let s = facts.settings;
        let heading = |t: &str| (Row::Heading, SettingsItem::Heading(t.into()));
        let shortcut = |row, label: &str, keys: Option<Shortcut>, clearable| {
            (
                row,
                SettingsItem::Shortcut {
                    label: label.into(),
                    keys: keys.map(|k| k.to_string()),
                    clearable,
                },
            )
        };
        let switch = |row, label: &str, on| {
            (
                row,
                SettingsItem::Switch {
                    label: label.into(),
                    on,
                },
            )
        };
        let palette = Palette::by_name(&s.palette).map_or(s.palette.as_str(), |i| PALETTES[i].name);
        let mut rows = vec![
            heading("Shortcuts"),
            shortcut(Row::Capture, "Capture", Some(s.capture), false),
            shortcut(
                Row::Countdown,
                "Capture after a countdown",
                s.countdown,
                true,
            ),
            shortcut(
                Row::Reopen,
                "Reopen the last capture",
                Some(s.reopen),
                false,
            ),
            shortcut(Row::Quit, "Quit Spooch", s.quit, true),
            switch(Row::PrintScreen, "Use the Print Screen key", s.print_screen),
            heading("General"),
            (
                Row::Seconds,
                SettingsItem::Stepper {
                    label: "Countdown".into(),
                    value: format!("{} s", s.countdown_seconds),
                },
            ),
            switch(Row::Login, facts.login_label, facts.launch_at_login),
            (
                Row::Palette,
                SettingsItem::Choice {
                    label: "Mark colours".into(),
                    value: palette.into(),
                },
            ),
            heading("Account"),
        ];
        rows.push(match facts.sharing {
            Signed::Out => (
                Row::SignedOut,
                SettingsItem::Buttons {
                    label: "Not signed in".into(),
                    buttons: vec!["Sign in\u{2026}".into(), "Create account\u{2026}".into()],
                },
            ),
            Signed::In(name) => (
                Row::SignedIn,
                SettingsItem::Buttons {
                    label: name
                        .as_ref()
                        .map_or_else(|| "Signed in".into(), |n| format!("Signed in as {n}")),
                    buttons: vec!["Set password\u{2026}".into(), "Sign out".into()],
                },
            ),
        });
        if matches!(facts.sharing, Signed::In(_)) {
            rows.push(switch(Row::Sync, "Sync captures", s.sync));
        }
        rows.push((Row::File, SettingsItem::Link("Open settings.txt".into())));
        let (rows, items) = rows.into_iter().unzip();
        self.rows = rows;
        items
    }

    /// What `event` changes, given the settings now.
    pub fn change(&self, event: SettingsEvent, settings: &Settings) -> Option<SettingsChange> {
        let row = |i: usize| self.rows.get(i).copied();
        let with = |f: &dyn Fn(&mut Settings)| {
            let mut s = settings.clone();
            f(&mut s);
            Some(SettingsChange::Settings(s))
        };
        match event {
            SettingsEvent::Recording(on) => Some(SettingsChange::PauseShortcuts(on)),
            SettingsEvent::Click(i) => match row(i)? {
                Row::PrintScreen => Some(SettingsChange::Command(Command::TogglePrintScreen)),
                Row::Login => Some(SettingsChange::Command(Command::ToggleLaunchAtLogin)),
                Row::Sync => Some(SettingsChange::Command(Command::ToggleSync)),
                Row::File => Some(SettingsChange::EditFile),
                Row::Palette => {
                    let next = Palette::by_name(&settings.palette).map_or(0, |i| i + 1);
                    let name = PALETTES[next % PALETTES.len()].name.to_lowercase();
                    with(&|s| s.palette = name.clone())
                }
                _ => None,
            },
            SettingsEvent::Step(i, d) if row(i)? == Row::Seconds => {
                let n = settings
                    .countdown_seconds
                    .saturating_add_signed(d)
                    .clamp(1, 10);
                with(&|s| s.countdown_seconds = n)
            }
            SettingsEvent::Step(..) => None,
            SettingsEvent::Button(i, b) => Some(SettingsChange::Command(match (row(i)?, b) {
                (Row::SignedOut, 0) => Command::SignIn,
                (Row::SignedOut, _) => Command::CreateAccount,
                (Row::SignedIn, 0) => Command::SetPassword,
                (Row::SignedIn, _) => Command::SignOut,
                _ => return None,
            })),
            SettingsEvent::Shortcut(i, keys) => {
                let row = row(i)?;
                let Some((mods, key)) = keys else {
                    return match row {
                        Row::Quit => with(&|s| s.quit = None),
                        Row::Countdown => with(&|s| s.countdown = None),
                        _ => None,
                    };
                };
                let Some(new) = shortcut_of(mods, key) else {
                    return Some(SettingsChange::Problem(
                        "Shortcuts end in a letter, a digit or an F key.".into(),
                    ));
                };
                let others = [
                    (Row::Capture, Some(settings.capture), "capturing"),
                    (Row::Countdown, settings.countdown, "the countdown"),
                    (Row::Reopen, Some(settings.reopen), "reopening"),
                    (Row::Quit, settings.quit, "quitting"),
                ];
                if let Some((_, _, what)) =
                    others.iter().find(|(r, s, _)| *r != row && *s == Some(new))
                {
                    return Some(SettingsChange::Problem(format!(
                        "{new} is already the shortcut for {what}."
                    )));
                }
                match row {
                    Row::Capture => with(&|s| s.capture = new),
                    Row::Countdown => with(&|s| s.countdown = Some(new)),
                    Row::Reopen => with(&|s| s.reopen = new),
                    Row::Quit => with(&|s| s.quit = Some(new)),
                    _ => None,
                }
            }
        }
    }
}

/// The global shortcut for keys pressed in the window, if it can be one.
fn shortcut_of(mods: Mods, key: Key) -> Option<Shortcut> {
    let key = match key {
        Key::Char(c) if c.is_ascii_alphanumeric() => ShortcutKey::Char(c.to_ascii_uppercase()),
        Key::F(n @ 1..=24) => ShortcutKey::F(n),
        Key::PrintScreen => ShortcutKey::PrintScreen,
        _ => return None,
    };
    Some(Shortcut::new(mods.ctrl, mods.shift, mods.alt, key))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(items: &[SettingsItem], label: &str) -> usize {
        items
            .iter()
            .position(|i| match i {
                SettingsItem::Shortcut { label: l, .. }
                | SettingsItem::Switch { label: l, .. }
                | SettingsItem::Stepper { label: l, .. }
                | SettingsItem::Choice { label: l, .. }
                | SettingsItem::Buttons { label: l, .. } => l == label,
                SettingsItem::Link(l) | SettingsItem::Heading(l) => l == label,
            })
            .unwrap()
    }

    #[test]
    fn clicks_change_settings_or_run_commands() {
        let settings = Settings::default();
        let mut form = SettingsForm::default();
        let items = form.items(&SettingsFacts {
            settings: &settings,
            launch_at_login: false,
            login_label: "Start when I sign in",
            sharing: &Signed::Out,
        });
        let changed = |e| match form.change(e, &settings) {
            Some(SettingsChange::Settings(s)) => s,
            other => panic!("{other:?}"),
        };
        // The countdown steps and stops at its ends.
        let secs = index(&items, "Countdown");
        assert_eq!(changed(SettingsEvent::Step(secs, 1)).countdown_seconds, 4);
        // The palette moves on.
        assert_eq!(
            changed(SettingsEvent::Click(index(&items, "Mark colours"))).palette,
            "soft"
        );
        // A recorded shortcut.
        let capture = index(&items, "Capture");
        let ctrl = Mods {
            ctrl: true,
            ..Mods::default()
        };
        let s = changed(SettingsEvent::Shortcut(capture, Some((ctrl, Key::F(9)))));
        assert_eq!(s.capture.to_string(), "Ctrl+F9");
        // One already used elsewhere isn't taken.
        let ctrl_shift = Mods {
            ctrl: true,
            shift: true,
            ..Mods::default()
        };
        let quit = index(&items, "Quit Spooch");
        assert!(matches!(
            form.change(
                SettingsEvent::Shortcut(quit, Some((ctrl_shift, Key::Char('2')))),
                &settings
            ),
            Some(SettingsChange::Problem(_))
        ));
        // Quit clears; Capture can't.
        assert_eq!(changed(SettingsEvent::Shortcut(quit, None)).quit, None);
        assert_eq!(
            form.change(SettingsEvent::Shortcut(capture, None), &settings),
            None
        );
        // Switches and the account are the tray's commands.
        assert_eq!(
            form.change(
                SettingsEvent::Click(index(&items, "Use the Print Screen key")),
                &settings
            ),
            Some(SettingsChange::Command(Command::TogglePrintScreen))
        );
        assert_eq!(
            form.change(
                SettingsEvent::Button(index(&items, "Not signed in"), 1),
                &settings
            ),
            Some(SettingsChange::Command(Command::CreateAccount))
        );
        assert_eq!(
            form.change(
                SettingsEvent::Click(index(&items, "Open settings.txt")),
                &settings
            ),
            Some(SettingsChange::EditFile)
        );
        // Signed in, the same buttons set the password or sign out.
        let items = form.items(&SettingsFacts {
            settings: &settings,
            launch_at_login: false,
            login_label: "Start when I sign in",
            sharing: &Signed::In(Some("mark".into())),
        });
        assert_eq!(
            form.change(
                SettingsEvent::Button(index(&items, "Signed in as mark"), 1),
                &settings
            ),
            Some(SettingsChange::Command(Command::SignOut))
        );
    }
}
