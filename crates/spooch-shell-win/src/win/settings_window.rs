//! The settings window: a plain fixed-size window with a dark system title
//! bar, showing the core's `SettingsPage`. Clicks and keys become
//! `SettingsEvent`s queued for the UI thread's loop, which owns the
//! settings and carries them out (`take_events`).

use std::cell::RefCell;

use spooch_app::{SettingsFacts, SettingsForm};
use spooch_render::skia_safe::Point;
use spooch_render::{Key, SettingsEvent, SettingsPage};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetMonitorInfoW, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromPoint, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_SNAPSHOT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos,
    IDC_ARROW, IsIconic, LoadCursorW, PostThreadMessageW, RegisterClassExW, SW_RESTORE,
    SW_SHOWNORMAL, SWP_NOZORDER, SetForegroundWindow, SetWindowPos, ShowWindow, WINDOW_EX_STYLE,
    WM_ACTIVATE, WM_APP, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN,
    WM_MOUSEMOVE, WM_PAINT, WM_SYSKEYDOWN, WNDCLASSEXW, WS_CAPTION, WS_MINIMIZEBOX, WS_OVERLAPPED,
    WS_SYSMENU,
};
use windows::core::{BOOL, PCWSTR, w};

use super::editing::{blit_from, editor_key, mods_now};
use super::log;

const CLASS_NAME: PCWSTR = w!("SpoochSettings");
/// Posted to the UI thread for the loop, with one of the `NEWS_` codes.
pub(super) const WM_SETTINGS_NEWS: u32 = WM_APP + 8;
/// The window has events, or wants showing as things are now.
pub(super) const NEWS_EVENTS: usize = 0;
/// It closed while a shortcut listened: register them again.
pub(super) const NEWS_RESUME: usize = 1;
/// Open it (from the editor's gear).
pub(super) const NEWS_OPEN: usize = 2;

struct SettingsWindow {
    hwnd: HWND,
    page: SettingsPage,
    form: SettingsForm,
    /// What happened since the loop last looked.
    events: Vec<SettingsEvent>,
}

thread_local! {
    static WINDOW: RefCell<Option<SettingsWindow>> = const { RefCell::new(None) };
}

fn with_window<R>(f: impl FnOnce(&mut SettingsWindow) -> R) -> Option<R> {
    WINDOW.with(|cell| cell.borrow_mut().as_mut().map(f))
}

/// Open the window, or bring it to the front if it's open.
pub(super) fn open(facts: &SettingsFacts) {
    if let Some(hwnd) = with_window(|w| w.hwnd) {
        refresh(facts);
        bring_up(hwnd);
        return;
    }
    if let Err(e) = create(facts) {
        log::line(format!("settings window: {e}"));
    }
}

/// Show the settings as they are now.
pub(super) fn refresh(facts: &SettingsFacts) {
    let hwnd = with_window(|w| {
        let items = w.form.items(facts);
        w.page.set_items(items);
        w.hwnd
    });
    if let Some(hwnd) = hwnd {
        invalidate(hwnd);
    }
}

/// The events since the last call, each with what it changes given
/// `settings`.
pub(super) fn take_changes(
    settings: &spooch_platform::settings::Settings,
) -> Vec<spooch_app::SettingsChange> {
    with_window(|w| {
        let events = std::mem::take(&mut w.events);
        events
            .into_iter()
            .filter_map(|e| w.form.change(e, settings))
            .collect()
    })
    .unwrap_or_default()
}

/// Say `note` under the rows, as a shortcut already in use.
pub(super) fn set_note(note: Option<String>) {
    if let Some(hwnd) = with_window(|w| {
        w.page.set_note(note);
        w.hwnd
    }) {
        invalidate(hwnd);
    }
}

fn create(facts: &SettingsFacts) -> windows::core::Result<()> {
    register()?;
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS_NAME,
            w!("Spooch settings"),
            style,
            0,
            0,
            400,
            400,
            None,
            None,
            GetModuleHandleW(None).ok().map(Into::into),
            None,
        )?
    };
    let dark = BOOL(1);
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const _ as *const _,
            size_of_val(&dark) as u32,
        );
    }
    // Sized for the screen it's on, centred on the one with the pointer.
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let mut page = SettingsPage::new(dpi as f32 / 96.0);
    let mut form = SettingsForm::default();
    page.set_items(form.items(facts));
    let (w, h) = page.size();
    let mut r = RECT {
        left: 0,
        top: 0,
        right: w,
        bottom: h,
    };
    unsafe {
        let _ = AdjustWindowRectExForDpi(&mut r, style, false, WINDOW_EX_STYLE(0), dpi);
    }
    let (ww, wh) = (r.right - r.left, r.bottom - r.top);
    let work = work_area();
    let (x, y) = (
        work.left + (work.right - work.left - ww) / 2,
        work.top + (work.bottom - work.top - wh) / 2,
    );
    unsafe {
        let _ = SetWindowPos(hwnd, None, x, y, ww, wh, SWP_NOZORDER);
    }
    WINDOW.with(|cell| {
        *cell.borrow_mut() = Some(SettingsWindow {
            hwnd,
            page,
            form,
            events: Vec::new(),
        })
    });
    bring_up(hwnd);
    Ok(())
}

fn register() -> windows::core::Result<()> {
    thread_local! {
        static REGISTERED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if REGISTERED.with(std::cell::Cell::get) {
        return Ok(());
    }
    unsafe {
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(proc),
            hInstance: GetModuleHandleW(None)?.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            return Err(windows::core::Error::from_thread());
        }
    }
    REGISTERED.with(|r| r.set(true));
    Ok(())
}

/// The work area of the monitor the pointer is on.
fn work_area() -> RECT {
    let mut cursor = POINT::default();
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        let _ = GetCursorPos(&mut cursor);
        let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            return info.rcWork;
        }
    }
    RECT {
        left: 0,
        top: 0,
        right: 1280,
        bottom: 800,
    }
}

fn bring_up(hwnd: HWND) {
    unsafe {
        let how = if IsIconic(hwnd).as_bool() {
            SW_RESTORE
        } else {
            SW_SHOWNORMAL
        };
        let _ = ShowWindow(hwnd, how);
        let _ = SetForegroundWindow(hwnd);
    }
}

fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// Tell the UI thread's loop `news`, one of the `NEWS_` codes. Window
/// procedures run on that thread, so the loop sees it next.
pub(super) fn post(news: usize) {
    unsafe {
        let _ = PostThreadMessageW(
            GetCurrentThreadId(),
            WM_SETTINGS_NEWS,
            WPARAM(news),
            LPARAM(0),
        );
    }
}

/// Queue `events` for the UI thread's loop, and repaint.
fn queue(hwnd: HWND, events: Vec<SettingsEvent>) {
    invalidate(hwnd);
    if !events.is_empty() {
        with_window(|w| w.events.extend(events));
        post(NEWS_EVENTS);
    }
}

fn point(lparam: LPARAM) -> Point {
    let x = (lparam.0 & 0xFFFF) as i16 as f32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
    Point::new(x, y)
}

extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
            with_window(|w| {
                let size = w.page.size();
                let (pixels, stride) = w.page.render();
                blit_from(hdc, pixels, stride, (0, 0), (0, 0), size);
            });
            unsafe {
                let _ = EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let mut t = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            unsafe {
                let _ = TrackMouseEvent(&mut t);
            }
            if with_window(|w| w.page.hover(Some(point(lparam)))) == Some(true) {
                invalidate(hwnd);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            if with_window(|w| w.page.hover(None)) == Some(true) {
                invalidate(hwnd);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let events = with_window(|w| w.page.press(point(lparam))).unwrap_or_default();
            queue(hwnd, events);
            LRESULT(0)
        }
        // Alt combinations come as system keys; take them while a
        // shortcut listens, so Alt doesn't open the window menu.
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let recording = with_window(|w| w.page.recording()) == Some(true);
            let key = editor_key(wparam.0 as u16);
            if let (true, Some(key)) = (recording, key) {
                let events = with_window(|w| w.page.key(key, mods_now())).unwrap_or_default();
                queue(hwnd, events);
                return LRESULT(0);
            }
            if recording {
                return LRESULT(0);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        // Windows sends Print Screen only as it's let go.
        WM_KEYUP if wparam.0 as u16 == VK_SNAPSHOT.0 => {
            let events =
                with_window(|w| w.page.key(Key::PrintScreen, mods_now())).unwrap_or_default();
            queue(hwnd, events);
            LRESULT(0)
        }
        // Coming back to it shows any change made elsewhere (the tray,
        // the file, a sign-in).
        WM_ACTIVATE => {
            post(NEWS_EVENTS);
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_CLOSE => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            // A shortcut still listening gives the global ones back.
            let left = WINDOW.with(|cell| cell.borrow_mut().take());
            if left.is_some_and(|w| w.page.recording()) {
                post(NEWS_RESUME);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
