//! The X11 shell: one client connection, one event loop, no toolkit.
//!
//! Idle, Spooch only holds passive grabs on its shortcuts. On capture it
//! captures the root window, uploads a bright and a dimmed copy to server
//! pixmaps, and maps one override-redirect window over the whole desktop.
//! Repaints compose into a back pixmap and copy only the damaged area, so
//! dragging does not flicker and does not push the desktop over the socket.

mod badge;
mod capture;
mod clipboard;
mod countdown;
mod cursors;
mod keys;
pub use spooch_platform::log;
mod menu;
mod overlay;
mod save;
mod settings_window;
mod system;
mod tray;
mod window;
mod xdnd;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use spooch_app::{
    Args, Carry, Config, DRAG_SLOP, Deliverable, Effect as EditEffect, Loaded, Pick, Picker,
    Response, Session as EditSession,
};
use spooch_platform::app::{self, Effect, Flow};
use spooch_platform::settings::{Action, Settings, Shortcut};
use spooch_render::skia_safe::{IRect, Point as DocPoint};
use spooch_render::{Counts, Cursor, Mods, Panel, Styles};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::render::{self, ConnectionExt as _, CreatePictureAux, PictOp};
use x11rb::protocol::xproto::{
    Atom, ButtonPressEvent, ChangeGCAux, ConnectionExt as _, CreateGCAux, CreateWindowAux,
    EventMask, Font, Gcontext, GrabMode, GrabStatus, ImageOrder, KeyButMask, KeyPressEvent,
    ModMask, MotionNotifyEvent, Pixmap, Rectangle, SubwindowMode, Timestamp, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::{CURRENT_TIME, NONE};

use capture::Transfer;
use clipboard::{Atoms, Clipboard};
use cursors::Cursors;
use keys::{Keymap, SHORTCUT_MODS, editor_key, keysym, modmask};
use log::Stopwatch;
use overlay::Session;

/// The background behind the tray menu's core-font text.
const TEXT_BACKGROUND: u32 = 0x202020;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Rect {
    fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    fn contains(&self, (x, y): (i32, i32)) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        let x1 = (self.x + self.w).max(o.x + o.w);
        let y1 = (self.y + self.h).max(o.y + o.h);
        Rect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        }
    }

    fn intersect(&self, o: &Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x + self.w).min(o.x + o.w);
        let y1 = (self.y + self.h).min(o.y + o.h);
        Rect {
            x: x0,
            y: y0,
            w: (x1 - x0).max(0),
            h: (y1 - y0).max(0),
        }
    }

    fn irect(&self) -> IRect {
        IRect::from_xywh(self.x, self.y, self.w, self.h)
    }

    fn from_irect(r: IRect) -> Rect {
        Rect {
            x: r.left,
            y: r.top,
            w: r.width(),
            h: r.height(),
        }
    }

    fn outset(&self, by: i32) -> Rect {
        Rect {
            x: self.x - by,
            y: self.y - by,
            w: self.w + 2 * by,
            h: self.h + 2 * by,
        }
    }

    fn x11(&self) -> Rectangle {
        Rectangle {
            x: self.x as i16,
            y: self.y as i16,
            width: self.w.max(0) as u16,
            height: self.h.max(0) as u16,
        }
    }
}

/// The canvas, which is what gets copied, saved or dragged out.
fn region(ed: &EditSession) -> Rect {
    Rect::from_irect(ed.doc.canvas())
}

/// Everything the edge handles and the core's chrome (rail, style row,
/// output bar and tooltips) cover.
fn chrome_rect(ed: &EditSession) -> Rect {
    region(ed)
        .outset(5)
        .union(&Rect::from_irect(ed.doc.chrome_bounds()))
}

struct App {
    conn: RustConnection,
    root: Window,
    depth: u8,
    visual: u32,
    width: u16,
    height: u16,
    keys: Keymap,
    transfer: Transfer,
    /// Picture format of the root visual, for dimming with RENDER.
    dim_format: Option<render::Pictformat>,
    /// A 32-bit ARGB picture format, for blending the core's chrome panels.
    argb_format: Option<render::Pictformat>,
    gc: Gcontext,
    /// Copies from the root window including every window on it.
    capture_gc: Gcontext,
    /// Recoloured per use, for the selection frame and the tray menu.
    bar_gc: Gcontext,
    cursors: Cursors,
    clipboard: Clipboard,
    xdnd: xdnd::Atoms,
    /// The file of the last drag-out, served until the target is done.
    offer: Option<xdnd::Offer>,
    /// The styles, format and Recents every capture and editor shares.
    carry: Carry,
    session: Option<Session>,
    /// The editor window, while it's open.
    editor_window: Option<window::EditorWindow>,
    /// The settings window, while it's open.
    settings_window: Option<settings_window::SettingsWindow>,
    /// The shortcuts are off while the settings window records one.
    paused: bool,
    once: bool,
    save: Option<String>,
    settings: Settings,
    config: Config,
    /// Grabbed shortcut keys, as (keycode, modifiers), to ungrab on reload.
    grabs: Vec<(u8, u16)>,
    tray: Option<tray::Tray>,
    menu: Option<menu::Menu>,
    /// The first-run welcome's answers, while its questions are open.
    welcome_answers: Option<std::sync::mpsc::Receiver<(bool, bool)>>,
    /// The client message type a helper thread sends to wake the loop.
    wake: u32,
    /// The countdown before a delayed capture, while it runs.
    countdown: Option<countdown::Countdown>,
    /// Counts countdowns, so a tick from one called off is ignored.
    countdowns: u32,
    /// The toast after a copy, while it's up.
    toast: Option<badge::Badge>,
    /// Counts toasts, so the end of one replaced is ignored.
    toasts: u32,
    /// What shortcuts, menu picks and countdown ticks do, shared with the
    /// other platforms.
    flow: Flow,
    quit: bool,
}

pub fn run() -> Result<(), String> {
    spooch_platform::save::set_clock(save::now);
    spooch_platform::save::set_folder(|| Some(save::folder()));
    log::init(system::xdg_dir("XDG_STATE_HOME", ".local/state").join("spooch"));
    let args = Args::from_env();
    let backend_arg = args.backend.clone().unwrap_or_else(|| "shm".into());
    let once = args.once;
    let editor = args.editor;
    let config = Config::new(system::config_dir());
    let Loaded {
        settings,
        styles,
        format,
        ..
    } = config.load_or_default(Styles::default());
    // Every capture is kept so it can be reopened, unless --no-recents.
    let recents = args.recents(Some(
        system::xdg_dir("XDG_DATA_HOME", ".local/share")
            .join("spooch")
            .join("recents"),
    ));
    let save = args.save;

    let started = Instant::now();
    let (conn, screen_num) = x11rb::connect(None).map_err(|e| format!("connect: {e}"))?;
    let setup = conn.setup();
    let screen = &setup.roots[screen_num];
    let (root, depth, visual) = (screen.root, screen.root_depth, screen.root_visual);
    let (width, height) = (screen.width_in_pixels, screen.height_in_pixels);
    let white = screen.white_pixel;

    // Spooch treats pixels as BGRX bytes, which is what little-endian
    // 24/32-bit TrueColor servers (all of today's) hand back.
    let bpp32 = setup
        .pixmap_formats
        .iter()
        .any(|f| f.depth == depth && f.bits_per_pixel == 32);
    let red_mask = screen
        .allowed_depths
        .iter()
        .flat_map(|d| &d.visuals)
        .find(|v| v.visual_id == visual)
        .map(|v| v.red_mask);
    if setup.image_byte_order != ImageOrder::LSB_FIRST || !bpp32 || red_mask != Some(0xff0000) {
        return Err(format!(
            "unsupported pixel layout: depth {depth}, byte order {:?}, red mask {red_mask:?}",
            setup.image_byte_order
        ));
    }

    let transfer = Transfer::new(backend_arg != "getimage");
    let formats = conn
        .render_query_pict_formats()
        .ok()
        .and_then(|c| c.reply().ok());
    let dim_format = formats.as_ref().and_then(|r| {
        r.screens
            .get(screen_num)?
            .depths
            .iter()
            .flat_map(|d| &d.visuals)
            .find(|v| v.visual == visual)
            .map(|v| v.format)
    });
    let argb_format = formats.as_ref().and_then(|r| {
        r.formats
            .iter()
            .find(|f| {
                f.depth == 32
                    && f.type_ == render::PictType::DIRECT
                    && f.direct.alpha_mask == 0xff
                    && f.direct.alpha_shift == 24
                    && f.direct.red_shift == 16
                    && f.direct.green_shift == 8
                    && f.direct.blue_shift == 0
            })
            .map(|f| f.id)
    });

    // Font matching takes tens of milliseconds; do it before the first capture.
    spooch_render::warm_up();
    let keys = Keymap::load(&conn)?;
    let atoms = Atoms::intern(&conn)?;
    let id = || conn.generate_id().map_err(|e| e.to_string());

    // A hidden window that owns the clipboard between captures.
    let owner = id()?;
    conn.create_window(
        0,
        owner,
        root,
        -1,
        -1,
        1,
        1,
        0,
        WindowClass::INPUT_ONLY,
        0,
        &CreateWindowAux::new(),
    )
    .map_err(|e| e.to_string())?;

    let gc = id()?;
    conn.create_gc(gc, root, &CreateGCAux::new().graphics_exposures(0))
        .map_err(|e| e.to_string())?;
    let capture_gc = id()?;
    conn.create_gc(
        capture_gc,
        root,
        &CreateGCAux::new()
            .subwindow_mode(SubwindowMode::INCLUDE_INFERIORS)
            .graphics_exposures(0),
    )
    .map_err(|e| e.to_string())?;
    let font: Font = id()?;
    conn.open_font(font, b"fixed").map_err(|e| e.to_string())?;
    let bar_gc = id()?;
    conn.create_gc(
        bar_gc,
        root,
        &CreateGCAux::new()
            .foreground(white)
            .background(TEXT_BACKGROUND)
            .font(font)
            .graphics_exposures(0),
    )
    .map_err(|e| e.to_string())?;
    let cursors = Cursors::load(&conn)?;
    let xdnd = xdnd::Atoms::intern(&conn)?;
    let wake = atom(&conn, b"_SPOOCH_WAKE")?;

    let mut app = App {
        root,
        depth,
        visual,
        width,
        height,
        keys,
        transfer,
        dim_format,
        argb_format,
        gc,
        capture_gc,
        bar_gc,
        cursors,
        clipboard: Clipboard::new(owner, atoms),
        xdnd,
        offer: None,
        carry: Carry::new(styles, recents, &settings)
            .keeping_picks_in(config.styles_path(), format),
        session: None,
        editor_window: None,
        settings_window: None,
        paused: false,
        once,
        save,
        settings,
        config,
        grabs: Vec::new(),
        tray: None,
        menu: None,
        welcome_answers: None,
        wake,
        countdown: None,
        countdowns: 0,
        toast: None,
        toasts: 0,
        flow: Flow::default(),
        quit: false,
        conn,
    };
    let taken = app.grab_shortcuts();
    if !once {
        let (owner, atom) = (app.clipboard.owner, app.wake);
        app.config
            .watch(move || wake_with(owner, atom, [SETTINGS_CHANGED, 0, 0, 0, 0]));
    }
    let grabbed = !taken.contains(&app.settings.capture.to_string());
    if !once {
        match tray::Tray::new(&app.conn, screen_num) {
            Ok(t) => {
                if t.is_docked() {
                    log::line("tray icon docked");
                } else {
                    log::line("no system tray yet; the icon appears if one starts");
                }
                app.tray = Some(t);
            }
            Err(e) => log::line(format!("could not create the tray icon: {e}")),
        }
    }
    let monitors = app.monitors();
    log::line(format!(
        "Spooch on X11: transfer {}, dim {}, desktop {width}x{height}, {} monitor(s) [{}], {}, ready in {:.1} ms, {}",
        app.transfer.name(),
        if app.dim_format.is_some() {
            "render"
        } else {
            "none (no RENDER)"
        },
        monitors.len(),
        monitors
            .iter()
            .map(|(name, r)| format!("{name} {}x{} at ({}, {})", r.w, r.h, r.x, r.y))
            .collect::<Vec<_>>()
            .join("; "),
        if grabbed {
            format!("shortcuts: {}", app.settings.summary())
        } else {
            format!(
                "hotkey NOT grabbed (another app holds {}?)",
                app.settings.capture
            )
        },
        log::ms_since(started),
        memory()
    ));

    if app.once {
        app.capture();
    } else {
        if let Some(text) = app::shortcuts_taken(&taken) {
            system::tell(&text);
        }
        app.welcome();
        // --editor opens the newest capture in the editor window.
        if editor {
            app.open_editor_recent(None);
        }
    }
    app.event_loop()
}

/// The wake message's first word when the settings file or the frames
/// folder changed (0 is the welcome's answers, 1 a countdown tick, 2 the
/// gallery's thumbnails).
const SETTINGS_CHANGED: u32 = 3;
/// The wake message's first word when a toast's time is up; the second is
/// which toast.
const TOAST_DONE: u32 = 4;

/// The atom for `name`, interning it if need be.
fn atom(conn: &RustConnection, name: &[u8]) -> Result<Atom, String> {
    Ok(conn
        .intern_atom(false, name)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())?
        .atom)
}

/// Wake the event loop from another thread: send a client message to our
/// own window over the wake connection.
fn wake(window: Window, kind: u32) {
    wake_with(window, kind, [0u32; 5]);
}

/// What a session's background thread calls for its news: a wake the loop
/// answers with `news_ready`.
fn news_waker(owner: Window, atom: u32) -> impl Fn() + Send + Sync + 'static {
    move || wake_with(owner, atom, [window::NEWS, 0, 0, 0, 0])
}

/// `wake`, with data for the loop: a countdown tick is `[TICK, generation]`.
fn wake_with(window: Window, kind: u32, data: [u32; 5]) {
    // One connection for every wake, made on first use and again after a
    // failure, rather than connecting and authenticating each time.
    static SENDER: std::sync::Mutex<Option<RustConnection>> = std::sync::Mutex::new(None);
    let mut sender = SENDER.lock().unwrap_or_else(|e| e.into_inner());
    if sender.is_none() {
        *sender = x11rb::connect(None).ok().map(|(conn, _)| conn);
    }
    let Some(conn) = sender.as_ref() else {
        log::line("wake: couldn't reach the X server");
        return;
    };
    let event = x11rb::protocol::xproto::ClientMessageEvent::new(32, window, kind, data);
    let sent = conn
        .send_event(false, window, EventMask::NO_EVENT, event)
        .is_ok()
        && conn.flush().is_ok();
    if !sent {
        log::line("wake: lost the X connection; reconnecting next time");
        *sender = None;
    }
}

impl App {
    /// Grab every shortcut in the settings, replacing earlier grabs.
    /// Returns the ones another app already holds.
    fn grab_shortcuts(&mut self) -> Vec<String> {
        for (code, mods) in self.grabs.drain(..) {
            let _ = self.conn.ungrab_key(code, self.root, ModMask::from(mods));
        }
        if self.paused {
            return Vec::new();
        }
        // NumLock (Mod2) and CapsLock change the modifier state, so each
        // combination needs its own grab.
        let extras = [
            0,
            u16::from(ModMask::LOCK),
            u16::from(ModMask::M2),
            u16::from(ModMask::LOCK | ModMask::M2),
        ];
        let mut taken = Vec::new();
        for (_, shortcut) in self.settings.shortcuts() {
            let codes = self.keys.codes(keysym(shortcut.key));
            let mut ok = !codes.is_empty();
            for code in codes {
                for extra in extras {
                    let mods = modmask(&shortcut) | extra;
                    let grabbed = self
                        .conn
                        .grab_key(
                            true,
                            self.root,
                            ModMask::from(mods),
                            code,
                            GrabMode::ASYNC,
                            GrabMode::ASYNC,
                        )
                        .map(|c| c.check().is_ok())
                        .unwrap_or(false);
                    if grabbed {
                        self.grabs.push((code, mods));
                    } else {
                        ok = false;
                    }
                }
            }
            if !ok {
                log::line(format!("{shortcut} is taken by another app"));
                taken.push(shortcut.to_string());
            }
        }
        taken
    }

    /// The action for a key press outside a capture, if it's a shortcut.
    fn shortcut_action(&self, e: &KeyPressEvent) -> Option<Action> {
        let sym = self.keys.sym(e.detail);
        let mods = u16::from(e.state) & SHORTCUT_MODS;
        self.settings
            .shortcuts()
            .into_iter()
            .find(|(_, s)| keysym(s.key) == sym && modmask(s) == mods)
            .map(|(a, _)| a)
    }

    fn save_settings(&self) {
        self.config.save(&self.settings);
    }

    /// Reread the settings file and grab its shortcuts.
    fn reload_settings(&mut self) {
        match self.config.load(self.carry.styles) {
            Ok(loaded) => {
                self.settings = loaded.settings;
                self.carry.styles = loaded.styles;
                self.carry
                    .share_to(&self.settings.share_server, self.settings.sync);
                let taken = self.grab_shortcuts();
                log::line(format!(
                    "settings reloaded, shortcuts: {}",
                    self.settings.summary()
                ));
                if let Some(text) = app::settings_problems(&loaded.problems, &taken) {
                    system::tell(&text);
                }
            }
            Err(e) => system::tell(&format!("Couldn't read the settings: {e}")),
        }
    }

    /// Turn "Use the Print Screen key" on or off. GNOME keeps Print Screen
    /// for its own screenshot tool, so if the grab fails Spooch offers to
    /// switch GNOME's shortcut off, and puts it back when the option is
    /// turned off again.
    fn toggle_print_screen(&mut self) {
        let marker = self.config.marker("gnome-print-screen");
        if self.settings.print_screen {
            self.settings.print_screen = false;
            self.save_settings();
            self.grab_shortcuts();
            if let Ok(old) = std::fs::read_to_string(&marker) {
                match system::set_gnome_screenshot_binding(old.trim()) {
                    Ok(()) => log::line("gave Print Screen back to GNOME"),
                    Err(e) => log::line(format!("could not give Print Screen back: {e}")),
                }
                let _ = std::fs::remove_file(&marker);
            }
            return;
        }
        self.settings.print_screen = true;
        let mut taken = self.grab_shortcuts();
        let print = Shortcut::PRINT_SCREEN.to_string();
        if taken.contains(&print)
            && let Some(old) = system::gnome_screenshot_binding()
        {
            let yes = system::ask(
                "GNOME uses Print Screen for its own screenshot tool, which keeps the key \
                 from other apps.\n\nTurn GNOME's Print Screen shortcut off so it opens \
                 Spooch? You can switch it back from this menu, or in Settings > Keyboard > \
                 Keyboard Shortcuts > Screenshots.",
            );
            if yes == Some(true) {
                match system::set_gnome_screenshot_binding("@as []") {
                    Ok(()) => {
                        if let Some(dir) = marker.parent() {
                            let _ = std::fs::create_dir_all(dir);
                        }
                        let _ = std::fs::write(&marker, &old);
                        log::line(format!("took Print Screen from GNOME (was {old})"));
                        // GNOME lets go of the key asynchronously.
                        for _ in 0..10 {
                            std::thread::sleep(Duration::from_millis(100));
                            taken = self.grab_shortcuts();
                            if !taken.contains(&print) {
                                break;
                            }
                        }
                    }
                    Err(e) => log::line(format!("could not change GNOME's shortcut: {e}")),
                }
            }
        }
        if taken.contains(&print) {
            self.settings.print_screen = false;
            self.grab_shortcuts();
            system::tell("Another app is holding Print Screen, so Spooch can't use it.");
        }
        self.save_settings();
    }

    /// Switch syncing on or off, and keep that in the settings.
    fn toggle_sync(&mut self) {
        self.carry.toggle_sync(&mut self.settings, &self.config);
    }

    fn toggle_launch_at_login(&self) {
        let on = !system::launch_at_login();
        match system::set_launch_at_login(on) {
            Ok(()) => log::line(format!("start at login {}", if on { "on" } else { "off" })),
            Err(e) => system::tell(&format!("Couldn't change that: {e}")),
        }
    }

    /// The first-run welcome: the shortcuts, and two offers. Shown once;
    /// `--welcome` shows it again.
    ///
    /// The questions run on their own thread, so Spooch keeps working while
    /// they wait. The answers come back through a channel, and a message
    /// to our own window wakes the event loop to act on them.
    fn welcome(&mut self) {
        if !self.config.wants_welcome(Args::from_env().welcome) {
            return;
        }
        let where_ = if self.tray.as_ref().is_some_and(tray::Tray::is_docked) {
            "Its red icon is in the top bar; click it for the menu.\n\n"
        } else {
            ""
        };
        let text = app::welcome_text(&self.settings, where_);
        let notice = format!(
            "Spooch is running: {} captures a region.",
            self.settings.capture
        );
        let ask_login = !system::launch_at_login();
        let (tx, rx) = std::sync::mpsc::channel();
        self.welcome_answers = Some(rx);
        let (wake_window, wake_atom) = (self.clipboard.owner, self.wake);
        std::thread::spawn(move || {
            let print = system::ask(&text);
            let login = print.is_some()
                && ask_login
                && system::ask("Start Spooch automatically when you log in?") == Some(true);
            if print.is_none() {
                system::tell(&notice);
            }
            let _ = tx.send((print == Some(true), login));
            wake(wake_window, wake_atom);
        });
    }

    /// Run an account form on a thread, so Spooch keeps working while it's
    /// filled in and checked, and tell what came of it.
    fn account(&self, f: impl FnOnce(&spooch_app::Sharing) -> Option<String> + Send + 'static) {
        let sharing = self.carry.sharing.clone();
        std::thread::spawn(move || {
            if let Some(text) = f(&sharing) {
                system::tell(&text);
            }
        });
    }

    /// Act on the welcome's answers, if they've come.
    fn welcome_answered(&mut self) {
        let Some(answers) = self
            .welcome_answers
            .as_ref()
            .and_then(|rx| rx.try_recv().ok())
        else {
            return;
        };
        self.welcome_answers = None;
        let (print, login) = answers;
        if print && !self.settings.print_screen {
            self.toggle_print_screen();
        }
        if login && !system::launch_at_login() {
            self.toggle_launch_at_login();
        }
        self.config.mark_welcomed();
    }

    /// Open the tray menu at the pointer.
    fn open_menu(&mut self, time: u32) {
        if self.session.is_some() || self.menu.is_some() {
            return;
        }
        let at = self.pointer();
        let items = app::tray_menu(
            &self.settings,
            self.carry
                .recents
                .as_ref()
                .is_some_and(|r| r.latest().is_some()),
        );
        let area = self
            .monitors()
            .into_iter()
            .map(|(_, r)| r)
            .find(|r| r.contains(at))
            .map_or((0, 0, i32::from(self.width), i32::from(self.height)), |r| {
                (r.x, r.y, r.w, r.h)
            });
        match menu::Menu::open(&self.conn, self.root, area, items, at, time) {
            Ok(m) => {
                m.draw(&self.conn, self.bar_gc);
                self.menu = Some(m);
            }
            Err(e) => log::line(format!("could not open the menu: {e}")),
        }
    }

    fn close_menu(&mut self) {
        if let Some(m) = self.menu.take() {
            m.close(&self.conn);
        }
    }

    /// What the badges draw with.
    fn screen(&self) -> badge::Screen<'_> {
        badge::Screen {
            conn: &self.conn,
            root: self.root,
            depth: self.depth,
            gc: self.gc,
        }
    }

    /// Say `text` in a toast centred on `over` for a moment, replacing any
    /// toast already up.
    fn show_toast(&mut self, text: &str, over: IRect) {
        self.hide_toast();
        let ((w, h), pixels) = spooch_render::toast(text, 1.0, Some(badge::behind()));
        let r = spooch_app::centred(over, w, h);
        let size = (w as u16, h as u16);
        match badge::Badge::show(self.screen(), (r.left, r.top), size, &pixels) {
            Ok(b) => self.toast = Some(b),
            Err(e) => {
                log::line(format!("could not show the toast: {e}"));
                return;
            }
        }
        self.toasts = self.toasts.wrapping_add(1);
        let (window, atom, which) = (self.clipboard.owner, self.wake, self.toasts);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(u64::from(spooch_render::TOAST_MS)));
            wake_with(window, atom, [TOAST_DONE, which, 0, 0, 0]);
        });
    }

    fn hide_toast(&mut self) {
        if let Some(b) = self.toast.take() {
            b.close(&self.conn);
        }
    }

    /// Show `n` on the countdown badge, putting it up if it isn't yet.
    fn show_countdown(&mut self, n: u32) {
        if let Some(mut c) = self.countdown.take() {
            c.set(self.screen(), n);
            self.countdown = Some(c);
            return;
        }
        let at = self.pointer();
        // The middle of the pointer's monitor.
        let size = i32::from(countdown::SIZE);
        let r = spooch_app::centred(self.monitor_under(at), size, size);
        let at = (r.left, r.top);
        self.countdowns = self.countdowns.wrapping_add(1);
        let (window, atom) = (self.clipboard.owner, self.wake);
        match countdown::Countdown::start(
            self.screen(),
            at,
            n,
            self.countdowns,
            move |tick, generation| wake_with(window, atom, [tick, generation, 0, 0, 0]),
        ) {
            Ok(c) => self.countdown = Some(c),
            Err(e) => log::line(format!("could not show the countdown: {e}")),
        }
    }

    fn hide_countdown(&mut self) {
        if let Some(c) = self.countdown.take() {
            c.close(&self.conn);
        }
    }

    /// A second of the countdown `generation` has passed.
    fn countdown_tick(&mut self, generation: u32) {
        if self
            .countdown
            .as_ref()
            .is_some_and(|c| c.generation == generation)
        {
            let effects = self.flow.tick(self.session.is_some());
            self.apply(effects);
        }
    }

    fn run_command(&mut self, cmd: menu::Command) {
        let effects = self
            .flow
            .command(cmd, &self.settings, self.session.is_some());
        self.apply(effects);
    }

    /// Carry out what the shared flow decided.
    fn apply(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Capture => self.capture(),
                Effect::ReopenLatest => self.reopen_latest(),
                Effect::OpenEditor(id) => self.open_editor_recent(id),
                Effect::ShowCountdown(n) => self.show_countdown(n),
                Effect::HideCountdown => self.hide_countdown(),
                Effect::OpenSettings => self.open_settings(),
                Effect::ShowSaved => match spooch_platform::save::to_show() {
                    Some(Ok(file)) => system::reveal(&file),
                    Some(Err(folder)) => system::open(&folder),
                    None => log::line("no folder for saved pictures"),
                },
                Effect::TogglePrintScreen => self.toggle_print_screen(),
                Effect::ToggleLaunchAtLogin => self.toggle_launch_at_login(),
                Effect::CreateAccount => {
                    self.account(|s| s.create_account_asking(system::ask_form))
                }
                Effect::SignIn => self.account(|s| s.sign_in_asking(None, system::ask_form)),
                Effect::SetPassword => self.account(|s| s.set_password_asking(system::ask_form)),
                Effect::ToggleSync => self.toggle_sync(),
                Effect::SignOut => self.carry.sharing.sign_out(),
                Effect::Quit => self.quit = true,
                Effect::Log(line) => log::line(&line),
            }
        }
    }

    /// Events for the tray icon and its menu. Returns whether it was one.
    fn tray_event(&mut self, event: &Event) -> bool {
        if let Some(m) = &mut self.menu {
            match event {
                Event::Expose(e) if e.window == m.window => {
                    m.draw(&self.conn, self.bar_gc);
                    return true;
                }
                Event::MotionNotify(e) => {
                    if m.motion(i32::from(e.event_x), i32::from(e.event_y)) {
                        m.draw(&self.conn, self.bar_gc);
                    }
                    return true;
                }
                Event::ButtonPress(e) => {
                    // The grab reports presses relative to the menu.
                    let (x, y) = if e.event == m.window {
                        (i32::from(e.event_x), i32::from(e.event_y))
                    } else {
                        (-1, -1)
                    };
                    match m.press(x, y, e.time) {
                        Some(Some(cmd)) => {
                            self.close_menu();
                            self.run_command(cmd);
                        }
                        Some(None) => self.close_menu(),
                        None => {}
                    }
                    return true;
                }
                Event::ButtonRelease(_) => return true,
                Event::KeyPress(e) => {
                    if self.keys.sym(e.detail) == keys::XK_ESCAPE {
                        self.close_menu();
                    }
                    return true;
                }
                _ => {}
            }
        }
        let Some(t) = &mut self.tray else {
            return false;
        };
        match event {
            Event::Expose(e) if e.window == t.window => {
                t.draw(&self.conn);
                true
            }
            Event::ConfigureNotify(e) if e.window == t.window => {
                t.resized(e.width, e.height);
                t.draw(&self.conn);
                true
            }
            Event::ButtonPress(e) if e.event == t.window => {
                self.open_menu(e.time);
                true
            }
            Event::ClientMessage(e) if e.type_ == t.manager_atom => {
                if t.manager_appeared(&self.conn, e) {
                    log::line("system tray appeared; icon docked");
                }
                true
            }
            _ => false,
        }
    }

    /// Where the pointer is on the root window, or the origin.
    fn pointer(&self) -> (i32, i32) {
        self.conn
            .query_pointer(self.root)
            .ok()
            .and_then(|c| c.reply().ok())
            .map_or((0, 0), |r| (i32::from(r.root_x), i32::from(r.root_y)))
    }

    /// The monitor `p` is on, or the whole root.
    fn monitor_under(&self, p: (i32, i32)) -> IRect {
        let monitors: Vec<IRect> = self.monitors().iter().map(|(_, r)| r.irect()).collect();
        spooch_app::monitor_at(&monitors, p)
            .unwrap_or_else(|| IRect::from_wh(i32::from(self.width), i32::from(self.height)))
    }

    /// Monitors from RandR 1.5, in root window pixels.
    fn monitors(&self) -> Vec<(String, Rect)> {
        let Ok(reply) = self
            .conn
            .randr_get_monitors(self.root, true)
            .map_err(|e| e.to_string())
            .and_then(|c| c.reply().map_err(|e| e.to_string()))
        else {
            return Vec::new();
        };
        reply
            .monitors
            .iter()
            .map(|m| {
                let name = self
                    .conn
                    .get_atom_name(m.name)
                    .ok()
                    .and_then(|c| c.reply().ok())
                    .map(|r| String::from_utf8_lossy(&r.name).into_owned())
                    .unwrap_or_default();
                let name = if m.primary {
                    format!("{name} (primary)")
                } else {
                    name
                };
                (
                    name,
                    Rect {
                        x: m.x.into(),
                        y: m.y.into(),
                        w: m.width.into(),
                        h: m.height.into(),
                    },
                )
            })
            .collect()
    }

    fn event_loop(&mut self) -> Result<(), String> {
        while !self.quit {
            let event = self.conn.wait_for_event().map_err(|e| e.to_string())?;
            self.handle(event);
            // Coalesce bursts of pointer motion into one repaint.
            while let Some(event) = self.conn.poll_for_event().map_err(|e| e.to_string())? {
                self.handle(event);
            }
            self.paint();
            self.editor_paint();
            self.settings_paint();
            self.conn.flush().map_err(|e| e.to_string())?;
        }
        log::line("quit");
        Ok(())
    }

    fn handle(&mut self, event: Event) {
        if let Event::ClientMessage(e) = &event
            && e.type_ == self.wake
        {
            let d = e.data.as_data32();
            if d[0] == countdown::TICK {
                self.countdown_tick(d[1]);
            } else if d[0] == TOAST_DONE {
                if d[1] == self.toasts {
                    self.hide_toast();
                }
            } else if d[0] == window::NEWS {
                self.news_ready();
            } else if d[0] == SETTINGS_CHANGED {
                log::line("settings changed; reloading");
                self.reload_settings();
                self.refresh_settings();
            } else {
                self.welcome_answered();
            }
            return;
        }
        if self.tray_event(&event) || self.editor_event(&event) || self.settings_event(&event) {
            return;
        }
        match event {
            Event::KeyPress(e) => self.key(e),
            Event::KeyRelease(e) => self.key_release(e),
            Event::ButtonPress(e) => self.button_press(e),
            Event::ButtonRelease(e) => self.button_release(e),
            Event::MotionNotify(e) => self.motion(e),
            Event::Expose(e) => {
                if let Some(s) = &mut self.session {
                    let r = Rect {
                        x: e.x.into(),
                        y: e.y.into(),
                        w: e.width.into(),
                        h: e.height.into(),
                    };
                    s.damage = s.damage.union(&r);
                }
            }
            Event::SelectionRequest(e) if e.selection == self.xdnd.selection => {
                if let Some(offer) = &self.offer {
                    let what = xdnd::request(&self.conn, &self.xdnd, offer, &e);
                    log::line(format!("drop target asked for {what}"));
                }
            }
            Event::SelectionRequest(e) => {
                let what = self.clipboard.request(&self.conn, &e);
                log::line(format!("clipboard request: {what}"));
            }
            Event::ClientMessage(e) if e.type_ == self.xdnd.status => {
                if let Some(drag) = self.session.as_mut().and_then(|s| s.drag.as_mut()) {
                    drag.status(&self.conn, &self.xdnd, &e);
                }
            }
            Event::ClientMessage(e) if e.type_ == self.xdnd.finished => {
                if let Some(offer) = self.offer.take() {
                    log::line(format!("drop finished: {}", offer.path.display()));
                }
            }
            Event::SelectionClear(_) => {
                self.clipboard.cleared();
                log::line("clipboard taken by another app");
                if self.once && self.session.is_none() {
                    self.quit = true;
                }
            }
            Event::Error(e) => log::line(format!("X error: {e:?}")),
            _ => {}
        }
    }
}

/// Round trip to the server, so timings include the server's work.
fn sync(conn: &RustConnection) -> Result<(), String> {
    conn.get_input_focus()
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn memory() -> String {
    let statm = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    let mut fields = statm
        .split_whitespace()
        .map(|f| f.parse::<u64>().unwrap_or(0));
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(0) as u64;
    let (_size, resident) = (fields.next().unwrap_or(0), fields.next().unwrap_or(0));
    format!("{:.1} MB resident", (resident * page) as f64 / 1_048_576.0)
}
