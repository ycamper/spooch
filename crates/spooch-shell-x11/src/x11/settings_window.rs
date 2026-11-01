//! The settings window: a plain fixed-size window the window manager
//! decorates, showing the core's `SettingsPage`. Clicks and keys are
//! carried out straight away, since the loop owns the settings.

use spooch_app::{SettingsChange, SettingsFacts, SettingsForm};
use spooch_render::skia_safe::Point;
use spooch_render::{SettingsEvent, SettingsPage};
use x11rb::CURRENT_TIME;
use x11rb::connection::Connection;
use x11rb::properties::{WmSizeHints, WmSizeHintsSpecification};
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, Window, WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;

use super::{App, log, system};

/// What X11 desktops call starting with the computer.
const LOGIN_LABEL: &str = "Start when I log in";
/// The window's background while it waits for its first paint.
const BACKDROP: u32 = 0x1c1d21;

/// The open settings window.
pub(super) struct SettingsWindow {
    window: Window,
    page: SettingsPage,
    form: SettingsForm,
    size: (u16, u16),
    dirty: bool,
    protocols: Atom,
    delete: Atom,
}

impl App {
    /// Run `f` with what the settings window shows.
    fn with_facts<R>(&self, f: impl FnOnce(&SettingsFacts) -> R) -> R {
        let sharing = self.carry.sharing.signed();
        f(&SettingsFacts {
            settings: &self.settings,
            launch_at_login: system::launch_at_login(),
            login_label: LOGIN_LABEL,
            sharing: &sharing,
        })
    }

    /// Open the window, or bring it to the front if it's open.
    pub(super) fn open_settings(&mut self) {
        if let Some(w) = self.settings_window.as_ref().map(|sw| sw.window) {
            self.refresh_settings();
            // The window manager raises, restores and focuses it.
            match super::atom(&self.conn, b"_NET_ACTIVE_WINDOW") {
                Ok(active) => self.send_to_root(w, active, [1, CURRENT_TIME, 0, 0, 0]),
                Err(e) => log::line(format!("settings window: {e}")),
            }
            return;
        }
        match self.make_settings() {
            Ok(sw) => self.settings_window = Some(sw),
            Err(e) => log::line(format!("settings window: {e}")),
        }
    }

    /// Show the settings as they are now.
    pub(super) fn refresh_settings(&mut self) {
        let Some(mut sw) = self.settings_window.take() else {
            return;
        };
        let items = self.with_facts(|f| sw.form.items(f));
        sw.page.set_items(items);
        sw.dirty = true;
        self.settings_window = Some(sw);
    }

    fn make_settings(&self) -> Result<SettingsWindow, String> {
        let atom = |name: &[u8]| super::atom(&self.conn, name);
        let protocols = atom(b"WM_PROTOCOLS")?;
        let delete = atom(b"WM_DELETE_WINDOW")?;
        // X11 has no per-monitor scale here; the editor draws at 1 too.
        let mut page = SettingsPage::new(1.0);
        let mut form = SettingsForm::default();
        page.set_items(self.with_facts(|f| form.items(f)));
        let (w, h) = page.size();
        let r = spooch_app::centred(self.monitor_under(self.pointer()), w, h);
        let window = self.conn.generate_id().map_err(|e| e.to_string())?;
        self.conn
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                window,
                self.root,
                r.left as i16,
                r.top as i16,
                w as u16,
                h as u16,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::new()
                    .background_pixel(BACKDROP)
                    .event_mask(
                        EventMask::EXPOSURE
                            | EventMask::KEY_PRESS
                            | EventMask::BUTTON_PRESS
                            | EventMask::POINTER_MOTION
                            | EventMask::LEAVE_WINDOW
                            | EventMask::FOCUS_CHANGE,
                    ),
            )
            .map_err(|e| e.to_string())?;
        let _ = self.conn.change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"spooch\0Spooch\0",
        );
        let _ = self.conn.change_property32(
            PropMode::REPLACE,
            window,
            protocols,
            AtomEnum::ATOM,
            &[delete],
        );
        // Where it was put, and one size only.
        let mut hints = WmSizeHints::new();
        hints.position = Some((WmSizeHintsSpecification::ProgramSpecified, r.left, r.top));
        hints.min_size = Some((w, h));
        hints.max_size = Some((w, h));
        let _ = hints.set_normal_hints(&self.conn, window);
        self.set_title(window, "Spooch settings");
        self.conn.map_window(window).map_err(|e| e.to_string())?;
        Ok(SettingsWindow {
            window,
            page,
            form,
            size: (w as u16, h as u16),
            dirty: true,
            protocols,
            delete,
        })
    }

    /// Events for the settings window. Returns whether it was one.
    pub(super) fn settings_event(&mut self, event: &Event) -> bool {
        let Some(sw) = &mut self.settings_window else {
            return false;
        };
        let w = sw.window;
        let point = |x: i16, y: i16| Point::new(f32::from(x), f32::from(y));
        let events = match event {
            Event::Expose(e) if e.window == w => {
                sw.dirty = true;
                Vec::new()
            }
            Event::MotionNotify(e) if e.event == w => {
                sw.dirty |= sw.page.hover(Some(point(e.event_x, e.event_y)));
                Vec::new()
            }
            Event::LeaveNotify(e) if e.event == w => {
                sw.dirty |= sw.page.hover(None);
                Vec::new()
            }
            // Only the left button clicks; the wheel and others do nothing.
            Event::ButtonPress(e) if e.event == w => {
                if e.detail != 1 {
                    return true;
                }
                sw.dirty = true;
                sw.page.press(point(e.event_x, e.event_y))
            }
            Event::KeyPress(e) if e.event == w => {
                let sym = self.keys.sym(e.detail);
                match super::editor_key(&self.keys, sym, e.detail) {
                    Some(k) => sw.page.key(k, super::keys::mods(e.state)),
                    None => Vec::new(),
                }
            }
            // Coming back to it shows changes made elsewhere.
            Event::FocusIn(e) if e.event == w => {
                self.refresh_settings();
                return true;
            }
            Event::FocusOut(e) if e.event == w => return true,
            Event::ClientMessage(e)
                if e.window == w
                    && e.type_ == sw.protocols
                    && e.data.as_data32()[0] == sw.delete =>
            {
                self.close_settings();
                return true;
            }
            _ => return false,
        };
        self.settings_events(events);
        true
    }

    /// Carry out what the window's `events` change.
    fn settings_events(&mut self, events: Vec<SettingsEvent>) {
        if events.is_empty() {
            return;
        }
        for event in events {
            let change = self
                .settings_window
                .as_ref()
                .and_then(|sw| sw.form.change(event, &self.settings));
            match change {
                Some(SettingsChange::Command(c)) => self.run_command(c),
                // Saving it brings the reload that grabs the shortcuts.
                Some(SettingsChange::Settings(s)) => {
                    self.settings = s;
                    self.save_settings();
                    self.settings_note(None);
                }
                Some(SettingsChange::PauseShortcuts(on)) => {
                    self.paused = on;
                    self.grab_shortcuts();
                }
                Some(SettingsChange::EditFile) => system::open(&self.config.settings_path()),
                Some(SettingsChange::Problem(p)) => self.settings_note(Some(p)),
                None => {}
            }
        }
        self.refresh_settings();
    }

    fn settings_note(&mut self, note: Option<String>) {
        if let Some(sw) = &mut self.settings_window {
            sw.page.set_note(note);
            sw.dirty = true;
        }
    }

    /// Paint the settings window if it changed.
    pub(super) fn settings_paint(&mut self) {
        let Some(sw) = &mut self.settings_window else {
            return;
        };
        if !sw.dirty {
            return;
        }
        sw.dirty = false;
        let (pixels, stride) = sw.page.render();
        let _ = self.transfer.put_rows(
            &self.conn, sw.window, self.gc, self.depth, sw.size, pixels, stride,
        );
    }

    fn close_settings(&mut self) {
        let Some(sw) = self.settings_window.take() else {
            return;
        };
        // A shortcut still listening gives the global ones back.
        if self.paused {
            self.paused = false;
            self.grab_shortcuts();
        }
        let _ = self.conn.destroy_window(sw.window);
        let _ = self.conn.flush();
    }
}
