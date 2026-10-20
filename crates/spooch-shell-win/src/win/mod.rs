//! The Windows shell's wiring: tray, shortcuts, capture, the overlay and
//! the editor window.

mod capture;
mod capture_dxgi;
mod capture_gdi;
mod clipboard;
mod countdown;
mod editing;
mod entry;
pub use spooch_platform::log;
mod monitors;
mod overlay;
mod save;
mod system;
mod tray;
mod window;

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use capture::CaptureBackend;
use spooch_app::{Args, Carry, Config, Deliverable};
use spooch_platform::app::{self, Effect, Flow, MenuItem};
use spooch_platform::recents::Recents;
use spooch_platform::settings::{Action, Key, Settings, Shortcut};
use spooch_render::Styles;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::OleInitialize;
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThreadId, SetProcessWorkingSetSize,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
    UnregisterHotKey, VK_F1, VK_SNAPSHOT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, MSG, PostMessageW,
    PostThreadMessageW, RegisterClassExW, RegisterWindowMessageW, TranslateMessage, WINDOW_STYLE,
    WM_APP, WM_CONTEXTMENU, WM_HOTKEY, WM_LBUTTONUP, WM_RBUTTONUP, WM_TIMER, WNDCLASSEXW,
    WS_EX_TOOLWINDOW,
};
use windows::core::w;

use log::Stopwatch;

/// Posted to the UI thread when Explorer restarts and the tray icon has to
/// be added again.
const WM_TRAY_READD: u32 = WM_APP + 4;
/// Posted to the UI thread with the first-run welcome's answers.
const WM_WELCOME_DONE: u32 = WM_APP + 5;
/// Posted to the UI thread when the settings file or frames folder changes.
const WM_SETTINGS_CHANGED: u32 = WM_APP + 7;
thread_local! {
    /// The styles, format and Recents every capture and editor shares.
    static CARRY: RefCell<Carry> = RefCell::new(Carry::default());
}

/// Run `f` on what carries from one capture to the next.
pub(super) fn carry<R>(f: impl FnOnce(&mut Carry) -> R) -> R {
    CARRY.with(|c| f(&mut c.borrow_mut()))
}

/// Explorer's "TaskbarCreated" message, registered at startup.
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

/// The resident shell: settings, the shortcuts they register, and the tray.
struct Shell {
    owner: HWND,
    settings: Settings,
    config: Config,
    /// What each registered hotkey id does: id `n` is `hotkeys[n - 1]`.
    hotkeys: Vec<Action>,
    tray: Option<tray::Tray>,
    recents: Option<Recents>,
}

impl Shell {
    /// Register every shortcut in the settings, replacing any registered
    /// before. Returns the ones another app already holds.
    fn register_hotkeys(&mut self) -> Vec<String> {
        for id in 1..=self.hotkeys.len() {
            unsafe {
                let _ = UnregisterHotKey(None, id as i32);
            }
        }
        self.hotkeys.clear();
        let mut taken = Vec::new();
        for (action, shortcut) in self.settings.shortcuts() {
            let id = self.hotkeys.len() as i32 + 1;
            let (mods, vk) = win_hotkey(&shortcut);
            match unsafe { RegisterHotKey(None, id, mods | MOD_NOREPEAT, vk) } {
                Ok(()) => self.hotkeys.push(action),
                Err(e) => {
                    log::line(format!("{shortcut} is taken by another app: {e}"));
                    taken.push(shortcut.to_string());
                    // Keep ids aligned with `hotkeys`.
                    self.hotkeys.push(action);
                }
            }
        }
        log::line(format!("shortcuts: {}", self.settings.summary()));
        taken
    }

    fn save_settings(&self) {
        self.config.save(&self.settings);
    }

    /// Reread the settings file and register its shortcuts.
    fn reload_settings(&mut self) {
        match self.config.load(carry(|c| c.styles)) {
            Ok(loaded) => {
                self.settings = loaded.settings;
                carry(|c| {
                    c.styles = loaded.styles;
                    c.share_to(&self.settings.share_server, self.settings.sync);
                });
                let taken = self.register_hotkeys();
                if let Some(text) = app::settings_problems(&loaded.problems, &taken) {
                    system::tell(Some(self.owner), &text);
                }
            }
            Err(e) => system::tell(
                Some(self.owner),
                &format!("Couldn't read the settings: {e}"),
            ),
        }
    }

    /// Turn "Use the Print Screen key" on or off. Windows 11 gives Print
    /// Screen to Snipping Tool, which keeps it from every other app, so
    /// turning it on asks to turn that off; turning it off gives it back if
    /// Spooch was the one that took it.
    fn toggle_print_screen(&mut self) {
        let marker = self.config.marker("took-print-screen");
        if self.settings.print_screen {
            self.settings.print_screen = false;
            if marker.exists() {
                if let Err(e) = system::set_snipping_takes_print_screen(true) {
                    log::line(format!("could not give Print Screen back: {e}"));
                }
                let _ = std::fs::remove_file(&marker);
                log::line("gave Print Screen back to Snipping Tool");
            }
        } else {
            if system::snipping_takes_print_screen() != Some(false) && system::windows_11() {
                let yes = system::ask(
                    Some(self.owner),
                    "Windows opens Snipping Tool when you press Print Screen, which keeps \
                     the key from other apps.\n\nTurn that off so Print Screen opens Spooch? \
                     You can switch it back from this menu, or in Windows Settings > \
                     Accessibility > Keyboard.",
                );
                if !yes {
                    return;
                }
                match system::set_snipping_takes_print_screen(false) {
                    Ok(()) => {
                        let _ = std::fs::write(&marker, b"");
                        log::line("took Print Screen from Snipping Tool");
                    }
                    Err(e) => {
                        system::tell(
                            Some(self.owner),
                            &format!("Couldn't change that setting: {e}"),
                        );
                        return;
                    }
                }
            }
            self.settings.print_screen = true;
        }
        self.save_settings();
        let taken = self.register_hotkeys();
        if self.settings.print_screen && !taken.is_empty() {
            system::tell(
                Some(self.owner),
                "Another app is holding Print Screen, so Spooch can't use it yet. \
                 Windows may need you to sign out and back in after the Snipping Tool \
                 change.",
            );
        }
    }

    /// Switch syncing on or off, and keep that in the settings.
    fn toggle_sync(&mut self) {
        carry(|c| c.toggle_sync(&mut self.settings, &self.config));
    }

    fn toggle_launch_at_login(&self) {
        let on = !system::launch_at_login();
        match system::set_launch_at_login(on) {
            Ok(()) => log::line(format!(
                "start at sign-in {}",
                if on { "on" } else { "off" }
            )),
            Err(e) => system::tell(Some(self.owner), &format!("Couldn't change that: {e}")),
        }
    }

    /// The first-run welcome: where Spooch lives, its shortcuts, and two
    /// offers. Shown once; `--welcome` shows it again.
    ///
    /// The questions run on their own thread, so Spooch keeps working while
    /// they wait; the answers come back as `WM_WELCOME_DONE`.
    fn welcome(&mut self) {
        if !self.config.wants_welcome(Args::from_env().welcome) {
            return;
        }
        let text = app::welcome_text(
            &self.settings,
            "Its red icon is in the notification area by the clock (it may be under the ^ \
             arrow); click it for the menu.\n\n",
        );
        let ask_login = !system::launch_at_login();
        let main = unsafe { GetCurrentThreadId() };
        std::thread::spawn(move || {
            let print = system::ask(None, &text);
            let login =
                ask_login && system::ask(None, "Start Spooch automatically when you sign in?");
            let answers = usize::from(print) | usize::from(login) << 1;
            unsafe {
                let _ = PostThreadMessageW(main, WM_WELCOME_DONE, WPARAM(answers), LPARAM(0));
            }
        });
    }

    /// Act on the welcome's answers: bit 0 is Print Screen, bit 1 is
    /// starting at sign-in.
    fn welcome_answered(&mut self, answers: usize) {
        if answers & 1 != 0 && !self.settings.print_screen {
            self.toggle_print_screen();
        }
        if answers & 2 != 0 && !system::launch_at_login() {
            self.toggle_launch_at_login();
        }
        self.config.mark_welcomed();
    }

    fn menu(&self) -> Vec<MenuItem> {
        app::tray_menu(
            &self.settings,
            self.recents.as_ref().is_some_and(|r| r.latest().is_some()),
            system::launch_at_login(),
            "Start when I sign in",
            &carry(|c| c.sharing.signed()),
        )
    }

    /// Sign in, on a thread so Spooch keeps working meanwhile: with a
    /// device token copied to the clipboard, else a name and password a
    /// window asks for.
    fn sign_in(&self) {
        let copied = clipboard::text(self.owner).filter(|t| spooch_app::looks_like_token(t));
        // Not left in the clipboard (or its history) once taken.
        if copied.is_some() {
            clipboard::clear(self.owner);
        }
        account(move |sharing| sharing.sign_in_asking(copied, entry::ask));
    }
}

/// Run an account form on its own thread, telling what came of it.
fn account(f: impl FnOnce(&spooch_app::Sharing) -> Option<String> + Send + 'static) {
    let sharing = carry(|c| c.sharing.clone());
    std::thread::spawn(move || {
        if let Some(text) = f(&sharing) {
            system::tell(None, &text);
        }
    });
}

/// The modifiers and virtual key RegisterHotKey wants for a shortcut.
fn win_hotkey(s: &Shortcut) -> (HOT_KEY_MODIFIERS, u32) {
    let mut mods = HOT_KEY_MODIFIERS(0);
    for (on, m) in [
        (s.ctrl, MOD_CONTROL),
        (s.shift, MOD_SHIFT),
        (s.alt, MOD_ALT),
        (s.logo, MOD_WIN),
    ] {
        if on {
            mods |= m;
        }
    }
    let vk = match s.key {
        // Virtual keys for letters and digits are their upper-case ASCII.
        Key::Char(c) => c as u32,
        Key::F(n) => u32::from(VK_F1.0) + u32::from(n) - 1,
        Key::PrintScreen => u32::from(VK_SNAPSHOT.0),
    };
    (mods, vk)
}

enum Backend {
    Gdi(capture_gdi::GdiCapture),
    Dxgi(capture_dxgi::DxgiCapture),
}

impl Backend {
    fn as_dyn(&mut self) -> &mut dyn CaptureBackend {
        match self {
            Backend::Gdi(b) => b,
            Backend::Dxgi(b) => b,
        }
    }

    fn set_monitors(&mut self, m: Vec<monitors::Monitor>) {
        match self {
            Backend::Gdi(b) => b.monitors = m,
            Backend::Dxgi(b) => b.monitors = m,
        }
    }
}

pub fn run() -> Result<(), String> {
    spooch_platform::save::set_clock(save::now);
    spooch_platform::save::set_folder(|| save::folder().ok());
    if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
        log::init(PathBuf::from(dir).join("Spooch"));
    }
    unsafe {
        // Physical pixels everywhere, per monitor. Must happen before any window exists.
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
            .map_err(|e| format!("SetProcessDpiAwarenessContext: {e}"))?;
        // Drag-out runs the OLE drag loop on this thread.
        if let Err(e) = OleInitialize(None) {
            log::line(format!("OleInitialize failed, so drag-out won't work: {e}"));
        }
    }

    // DXGI Desktop Duplication by default (about 40 ms to the overlay
    // against 100 for GDI, for 80 MB of committed memory); GDI when it's
    // unavailable, or with --backend=gdi.
    let args = Args::from_env();
    let mut backend = match args.backend.as_deref().unwrap_or("dxgi") {
        "dxgi" => {
            let started = Stopwatch::start();
            match capture_dxgi::DxgiCapture::new() {
                Ok(dxgi) => {
                    log::line(format!(
                        "desktop duplication ready in {:.1} ms",
                        started.ms()
                    ));
                    Backend::Dxgi(dxgi)
                }
                Err(e) => {
                    log::line(format!("desktop duplication unavailable ({e}); using GDI"));
                    Backend::Gdi(capture_gdi::GdiCapture {
                        monitors: Vec::new(),
                    })
                }
            }
        }
        "gdi" => Backend::Gdi(capture_gdi::GdiCapture {
            monitors: Vec::new(),
        }),
        other => return Err(format!("unknown backend {other:?}; use dxgi or gdi")),
    };

    let owner = message_window()?;
    let config = Config::new(system::config_dir());
    let main = unsafe { GetCurrentThreadId() };
    config.watch(move || unsafe {
        let _ = PostThreadMessageW(main, WM_SETTINGS_CHANGED, WPARAM(0), LPARAM(0));
    });
    let loaded = config.load_or_default(Styles::default());
    // Every capture is kept so it can be reopened, unless --no-recents.
    let recents = args.recents(
        std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Spooch").join("Recents")),
    );
    carry(|c| *c = Carry::new(loaded.styles, recents.clone(), &loaded.settings));
    let mut shell = Shell {
        owner,
        settings: loaded.settings,
        config,
        hotkeys: Vec::new(),
        tray: None,
        recents,
    };
    let taken = shell.register_hotkeys();
    shell.tray = tray::Tray::new(owner);
    if shell.tray.is_none() {
        log::line("could not add the tray icon");
    }
    log::line(format!(
        "Spooch ({}), settings in {}. Esc cancels a capture.",
        backend.as_dyn().name(),
        shell.config.settings_path().display()
    ));
    if let Some(text) = app::shortcuts_taken(&taken) {
        system::tell(Some(owner), &text);
    }
    log_monitors(&monitors::enumerate());
    warm_up(&mut backend);
    // Font matching takes tens of milliseconds; do it before the first capture.
    spooch_render::warm_up();
    shell.welcome();
    // --editor opens the newest capture in the editor window.
    if args.editor {
        window::open_recent(None);
    }
    trim_memory();
    log_memory("idle");

    let mut current: Option<Stopwatch> = None;
    let mut flow = Flow::default();
    let mut badge: Option<countdown::Countdown> = None;
    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        if msg.hwnd.is_invalid() && msg.message == WM_HOTKEY {
            let action = msg
                .wParam
                .0
                .checked_sub(1)
                .and_then(|i| shell.hotkeys.get(i).copied());
            if let Some(a) = action {
                let effects = flow.shortcut(a, &shell.settings, overlay::is_active());
                if apply(effects, &mut shell, &mut backend, &mut current, &mut badge) {
                    break;
                }
            }
            continue;
        }
        if msg.hwnd.is_invalid() && msg.message == tray::WM_TRAY_MENU {
            if overlay::is_active() {
                continue;
            }
            let menu = shell.menu();
            if let Some(c) = shell.tray.as_ref().and_then(|t| t.show_menu(&menu)) {
                let effects = flow.command(c, &shell.settings, overlay::is_active());
                if apply(effects, &mut shell, &mut backend, &mut current, &mut badge) {
                    break;
                }
            }
            continue;
        }
        if msg.hwnd.is_invalid()
            && msg.message == WM_TIMER
            && badge.as_ref().is_some_and(|b| b.is_timer(msg.wParam.0))
        {
            let effects = flow.tick(overlay::is_active());
            if apply(effects, &mut shell, &mut backend, &mut current, &mut badge) {
                break;
            }
            continue;
        }
        if msg.hwnd.is_invalid() && msg.message == WM_SETTINGS_CHANGED {
            log::line("settings changed; reloading");
            shell.reload_settings();
            continue;
        }
        if msg.hwnd.is_invalid() && msg.message == WM_WELCOME_DONE {
            shell.welcome_answered(msg.wParam.0);
            continue;
        }
        if msg.hwnd.is_invalid() && msg.message == WM_TRAY_READD {
            if let Some(t) = &shell.tray {
                t.add();
            }
            continue;
        }
        if msg.hwnd.is_invalid() && msg.message == overlay::WM_OVERLAY_DONE {
            if let Some(sw) = current.take() {
                complete(owner, sw);
            }
            continue;
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

/// Carry out what the shared flow decided. Returns whether to quit.
fn apply(
    effects: Vec<Effect>,
    shell: &mut Shell,
    backend: &mut Backend,
    current: &mut Option<Stopwatch>,
    badge: &mut Option<countdown::Countdown>,
) -> bool {
    for effect in effects {
        match effect {
            Effect::Capture => *current = capture(backend),
            Effect::ReopenLatest => match carry(|c| c.latest()) {
                Some(id) => *current = reopen(backend, &id),
                None => log::line("nothing in recents to reopen yet"),
            },
            Effect::OpenEditor(id) => window::open_recent(id),
            Effect::ShowCountdown(n) => match badge {
                Some(b) => b.set(n),
                None => *badge = countdown::Countdown::start(n),
            },
            // Dropping it takes the badge off the screen.
            Effect::HideCountdown => *badge = None,
            Effect::EditSettings => system::open(&shell.config.settings_path()),
            Effect::ShowSaved => match spooch_platform::save::to_show() {
                Some(Ok(file)) => system::reveal(&file),
                Some(Err(folder)) => system::open(&folder),
                None => log::line("no folder for saved pictures"),
            },
            Effect::TogglePrintScreen => shell.toggle_print_screen(),
            Effect::ToggleLaunchAtLogin => shell.toggle_launch_at_login(),
            Effect::CreateAccount => account(|s| s.create_account_asking(entry::ask)),
            Effect::SignIn => shell.sign_in(),
            Effect::SetPassword => account(|s| s.set_password_asking(entry::ask)),
            Effect::ToggleSync => shell.toggle_sync(),
            Effect::SignOut => carry(|c| c.sharing.sign_out()),
            Effect::Quit => return true,
            Effect::Log(line) => log::line(&line),
        }
    }
    false
}

/// Freeze the screen and open the kept capture `id` on it.
fn reopen(backend: &mut Backend, id: &str) -> Option<Stopwatch> {
    let sw = capture(backend)?;
    overlay::reopen(id);
    Some(sw)
}

/// Hotkey pressed: capture everything, then show the overlay.
fn capture(backend: &mut Backend) -> Option<Stopwatch> {
    let mut sw = Stopwatch::start();
    let mons = monitors::enumerate();
    sw.mark(format!("enumerated {} monitor(s)", mons.len()));
    backend.set_monitors(mons.clone());

    let frames = match backend.as_dyn().capture_all() {
        Ok(f) => f,
        // A lost or refused duplication should never cost the user a capture.
        Err(e) if matches!(backend, Backend::Dxgi(_)) => {
            log::line(format!(
                "desktop duplication failed ({e}); falling back to GDI"
            ));
            let mut gdi = capture_gdi::GdiCapture {
                monitors: mons.clone(),
            };
            match gdi.capture_all() {
                Ok(f) => f,
                Err(e) => {
                    log::line(format!("capture failed: {e}"));
                    return None;
                }
            }
        }
        Err(e) => {
            log::line(format!("capture failed: {e}"));
            return None;
        }
    };
    for f in &frames {
        sw.mark(format!(
            "  captured {} {}x{} in {:.1} ms",
            f.display.name,
            f.image.width,
            f.image.height,
            f.elapsed.as_secs_f64() * 1000.0
        ));
    }
    sw.mark("all monitors captured");

    let paired = mons.iter().map(|m| m.px).zip(frames).collect();
    if let Err(e) = overlay::show(paired, &mut sw) {
        log::line(format!("overlay failed: {e}"));
        return None;
    }
    let to_overlay = sw.ms();
    sw.mark("overlay visible on every monitor");
    log::line(format!(
        "hotkey to frozen overlay: {to_overlay:.1} ms (target < 100 ms) {}",
        if to_overlay < 100.0 { "PASS" } else { "SLOW" }
    ));
    Some(sw)
}

/// Selection finished or cancelled: copy, tear down, report.
fn complete(owner: HWND, mut sw: Stopwatch) {
    sw.mark("selection finished");
    let Some(done) = overlay::finish() else {
        return;
    };
    sw.mark("overlay closed");
    match (done.outcome, done.image) {
        (overlay::Outcome::Region(r, save), Some(px)) => {
            sw.mark(format!("region at ({}, {})", r.left, r.top));
            match Deliverable::new(px, done.format, &done.stem, save, done.counts) {
                Ok(d) => editing::deliver(owner, d, &mut sw),
                Err(e) => sw.mark(format!("encoding failed: {e}")),
            }
        }
        (overlay::Outcome::Shared, _) => sw.mark("shared"),
        (overlay::Outcome::Window, _) => {
            if let Some(session) = done.editor {
                sw.mark("to the editor window");
                window::open(session.into_window());
            }
        }
        (overlay::Outcome::Dragged(r, counts, path), _) => sw.mark(spooch_app::dragged_line(
            spooch_render::skia_safe::IRect::from_ltrb(r.left, r.top, r.right, r.bottom),
            counts,
            &path,
        )),
        _ => sw.mark("cancelled"),
    }
    sw.report("capture");
    log_memory("after capture");
    trim_memory();
    log_memory("after trim");
}

/// The first capture after launch pays one-off costs (GDI and DWM readback
/// setup, page-ins, window class registration): 77 ms instead of about 35 ms
/// on a test machine. Pay them at startup with a throwaway capture.
fn warm_up(backend: &mut Backend) {
    let sw = Stopwatch::start();
    if let Err(e) = overlay::prepare() {
        log::line(format!("overlay class registration failed: {e}"));
    }
    let mons = monitors::enumerate();
    backend.set_monitors(mons);
    match backend.as_dyn().capture_all() {
        Ok(frames) => drop(frames),
        Err(e) => log::line(format!("warm-up capture failed: {e}")),
    }
    log::line(format!("warm-up capture: {:.1} ms", sw.ms()));
}

/// Freed frames stay in the working set until Windows needs the pages.
/// Hand them back so idle really is idle.
pub(super) fn trim_memory() {
    unsafe {
        let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

fn log_monitors(mons: &[monitors::Monitor]) {
    for m in mons {
        log::line(format!(
            "monitor {}: {}x{} px at ({}, {}), scale {:.2}{}",
            m.display.name,
            m.width(),
            m.height(),
            m.px.left,
            m.px.top,
            m.display.scale,
            if m.display.is_primary {
                ", primary"
            } else {
                ""
            }
        ));
    }
}

fn log_memory(when: &str) {
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
            counters.cb,
        )
    }
    .is_ok();
    if ok {
        let mb = |b: usize| b as f64 / (1024.0 * 1024.0);
        let ws = mb(counters.WorkingSetSize);
        log::line(format!(
            "working set {when}: {ws:.1} MB, private {:.1} MB (idle target < 30 MB){}",
            mb(counters.PrivateUsage),
            if when == "idle" && ws >= 30.0 {
                " SLOW"
            } else {
                ""
            }
        ));
    }
}

/// A hidden window that owns the clipboard between captures and the tray
/// icon. It's an ordinary top-level window, never shown, rather than a
/// message-only one, because only top-level windows hear Explorer's
/// "TaskbarCreated" after a restart.
fn message_window() -> Result<HWND, String> {
    extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        // Hand tray clicks and Explorer restarts to the message loop, which
        // has the state to act on them.
        let post = |m: u32| unsafe {
            let _ = PostMessageW(None, m, WPARAM(0), LPARAM(0));
        };
        if msg == tray::WM_TRAY {
            let event = (l.0 & 0xffff) as u32;
            if matches!(event, WM_LBUTTONUP | WM_RBUTTONUP | WM_CONTEXTMENU) {
                post(tray::WM_TRAY_MENU);
            }
            return LRESULT(0);
        }
        let created = TASKBAR_CREATED.load(Ordering::Relaxed);
        if created != 0 && msg == created {
            post(WM_TRAY_READD);
            return LRESULT(0);
        }
        unsafe { DefWindowProcW(hwnd, msg, w, l) }
    }
    TASKBAR_CREATED.store(
        unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) },
        Ordering::Relaxed,
    );
    unsafe {
        let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(proc),
            hInstance: instance.into(),
            lpszClassName: w!("SpoochMessages"),
            ..Default::default()
        };
        RegisterClassExW(&class);
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            w!("SpoochMessages"),
            w!("Spooch"),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .map_err(|e| format!("message window: {e}"))
    }
}
