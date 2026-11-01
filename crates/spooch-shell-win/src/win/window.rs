//! The Win32 editor window: shows `Document::render_window` and feeds input
//! to the session. The frame is Spooch's own (`WM_NCCALCSIZE` hides the
//! system one but keeps its shadow and borders); `HTMAXBUTTON` keeps snap
//! layouts.

use std::cell::{Cell, RefCell};

use spooch_app::{EDITOR_MIN, Effect, Response, Session, editor_rect};
use spooch_render::skia_safe::{IRect, Point as DocPoint};
use spooch_render::{WindowButton, WindowPart};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmExtendFrameIntoClientArea,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetMonitorInfoW, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromPoint, PAINTSTRUCT, ScreenToClient,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent, VK_MENU,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect,
    GetCursorPos, GetSystemMetrics, GetWindowRect, HTCLIENT, IDC_ARROW, IsIconic, IsWindowVisible,
    LoadCursorW, MINMAXINFO, MSG, PM_REMOVE, PeekMessageW, RegisterClassExW, SW_RESTORE,
    SW_SHOWNORMAL, SetCursor, SetForegroundWindow, SetWindowTextW, ShowWindow, WINDOW_EX_STYLE,
    WM_APP, WM_CHAR, WM_CLOSE, WM_ERASEBKGND, WM_GETMINMAXINFO, WM_KEYDOWN, WM_KEYUP,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT,
    WM_SETCURSOR, WM_SIZE, WM_SYSKEYDOWN, WM_SYSKEYUP, WNDCLASSEXW, WS_OVERLAPPEDWINDOW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTCLOSE, HTLEFT, HTMAXBUTTON, HTMINBUTTON,
    HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IsZoomed, NCCALCSIZE_PARAMS, SIZE_MAXIMIZED, SM_CXFRAME,
    SM_CXPADDEDBORDER, SM_CYFRAME, SW_MAXIMIZE, SW_MINIMIZE, SWP_FRAMECHANGED, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, WM_NCCALCSIZE, WM_NCHITTEST, WM_NCLBUTTONDOWN,
    WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE,
};
use windows::core::{HSTRING, PCWSTR, w};

use super::editing::{
    Wheel, blit_from, cursor_id, deliver, drag_dir, drag_slop, editor_key, mods_now, shell_drag,
};
use super::log::{self, Stopwatch};

const CLASS_NAME: PCWSTR = w!("SpoochEditor");
/// Posted from the gallery's thumbnail thread when it has some ready.
const WM_NEWS: u32 = WM_APP + 6;

struct EditorWindow {
    hwnd: HWND,
    session: Session,
}

thread_local! {
    static WINDOW: RefCell<Option<EditorWindow>> = const { RefCell::new(None) };
    static REGISTERED: Cell<bool> = const { Cell::new(false) };
}

/// Run `f` on the open window, unless it's already borrowed (window
/// procedures re-enter through synchronous messages).
fn with_window<R>(f: impl FnOnce(&mut EditorWindow) -> R) -> Option<R> {
    WINDOW.with(|w| w.try_borrow_mut().ok()?.as_mut().map(f))
}

/// Show `session` in the editor window, making the window if it isn't
/// open. A capture already in it is kept in Recents first.
pub fn open(session: Session) {
    let title = HSTRING::from(session.stem());
    let mut pending = Some(session);
    let reused = with_window(|w| {
        if let Some(mut s) = pending.take() {
            s.set_waker(super::editing::waker(w.hwnd, WM_NEWS));
            w.session.replace(s, |line| log::line(&line));
        }
        w.hwnd
    });
    match (reused, pending) {
        (Some(hwnd), _) => {
            unsafe {
                let _ = SetWindowTextW(hwnd, &title);
            }
            invalidate(hwnd);
            bring_up(hwnd);
        }
        (None, Some(session)) => create(session, &title),
        (None, None) => {}
    }
}

/// Open the kept capture `id`, or the newest, in the editor window.
pub fn open_recent(id: Option<String>) {
    match super::carry(|c| c.open_editor(id)) {
        Ok((session, line)) => {
            log::line(&line);
            open(session);
        }
        Err(e) => log::line(&e),
    }
}

fn create(session: Session, title: &HSTRING) {
    if let Err(e) = register() {
        log::line(format!("editor window: {e}"));
        return;
    }
    let (x, y, w, h) = initial_rect();
    let hwnd = unsafe {
        let instance = GetModuleHandleW(None).ok();
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS_NAME,
            title,
            WS_OVERLAPPEDWINDOW,
            x,
            y,
            w,
            h,
            None,
            None,
            instance.map(Into::into),
            None,
        )
    };
    let hwnd = match hwnd {
        Ok(h) => h,
        Err(e) => {
            log::line(format!("editor window: {e}"));
            return;
        }
    };
    use_own_frame(hwnd);
    let mut session = session;
    let (cw, ch) = client_size(hwnd);
    session.resize(cw, ch);
    session.set_waker(super::editing::waker(hwnd, WM_NEWS));
    WINDOW.with(|cell| *cell.borrow_mut() = Some(EditorWindow { hwnd, session }));
    bring_up(hwnd);
}

/// Show the window, restored and in front. The first window a process
/// shows takes the show state it was started with, so Spooch started
/// hidden would keep its editor hidden: show it until it's visible.
fn bring_up(hwnd: HWND) {
    unsafe {
        let how = if IsIconic(hwnd).as_bool() {
            SW_RESTORE
        } else {
            SW_SHOWNORMAL
        };
        for _ in 0..2 {
            let _ = ShowWindow(hwnd, how);
            if IsWindowVisible(hwnd).as_bool() {
                break;
            }
        }
        let _ = SetForegroundWindow(hwnd);
    }
}

fn register() -> windows::core::Result<()> {
    if REGISTERED.with(Cell::get) {
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

/// Where the window goes: on the work area of the monitor the pointer is
/// on, as `spooch_app::editor_rect` places it.
fn initial_rect() -> (i32, i32, i32, i32) {
    let mut cursor = POINT::default();
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let work = unsafe {
        let _ = GetCursorPos(&mut cursor);
        let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            let r = info.rcWork;
            IRect::from_ltrb(r.left, r.top, r.right, r.bottom)
        } else {
            IRect::from_wh(1280, 800)
        }
    };
    let r = editor_rect(work);
    (r.left, r.top, r.width(), r.height())
}

fn client_size(hwnd: HWND) -> (i32, i32) {
    let mut r = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut r);
    }
    (r.right - r.left, r.bottom - r.top)
}

fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// A client point from a mouse message's `lparam`; negative while the
/// mouse is captured and outside the window.
fn client_point(lparam: LPARAM) -> DocPoint {
    let x = (lparam.0 & 0xFFFF) as i16 as f32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
    DocPoint::new(x, y)
}

fn pointer(hwnd: HWND) -> DocPoint {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
        let _ = ScreenToClient(hwnd, &mut p);
    }
    DocPoint::new(p.x as f32, p.y as f32)
}

extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_NEWS => {
            carry_out(hwnd, with_window(|e| e.session.poll()));
            LRESULT(0)
        }
        WM_SIZE => {
            let (w, h) = (
                (lparam.0 & 0xFFFF) as i32,
                ((lparam.0 >> 16) & 0xFFFF) as i32,
            );
            let maximized = wparam.0 as u32 == SIZE_MAXIMIZED;
            if w > 0 && h > 0 {
                carry_out(
                    hwnd,
                    with_window(|e| {
                        e.session.doc.set_maximized(maximized);
                        e.session.resize(w, h)
                    }),
                );
            }
            LRESULT(0)
        }
        // The whole window is the client: the system frame stays only for
        // its shadow and resize borders. Maximised, Windows puts the frame
        // off the screen's edge, so keep the client inside it.
        WM_NCCALCSIZE if wparam.0 != 0 => {
            if unsafe { IsZoomed(hwnd) }.as_bool() {
                let params = lparam.0 as *mut NCCALCSIZE_PARAMS;
                if !params.is_null() {
                    let (fx, fy) = frame_thickness();
                    unsafe {
                        let r = &mut (*params).rgrc[0];
                        r.left += fx;
                        r.right -= fx;
                        r.top += fy;
                        r.bottom -= fy;
                    }
                }
            }
            LRESULT(0)
        }
        WM_NCHITTEST => {
            let p = screen_to_client(hwnd, lparam);
            let part = with_window(|e| e.session.doc.window_part(p));
            LRESULT(match part {
                Some(part) => hit_code(part) as isize,
                None => HTCLIENT as isize,
            })
        }
        // The window buttons are non-client so Windows offers snap
        // layouts; their hover and clicks come here.
        WM_NCMOUSEMOVE => {
            let p = screen_to_client(hwnd, lparam);
            track_leave(hwnd);
            carry_out(
                hwnd,
                with_window(|e| e.session.motion(p, (4.0, 4.0), mods_now())),
            );
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_NCMOUSELEAVE => {
            let away = DocPoint::new(-1.0, -1.0);
            carry_out(
                hwnd,
                with_window(|e| e.session.motion(away, (4.0, 4.0), mods_now())),
            );
            LRESULT(0)
        }
        WM_NCLBUTTONDOWN | WM_NCLBUTTONUP if is_button(wparam.0 as u32) => {
            if msg == WM_NCLBUTTONUP {
                let p = screen_to_client(hwnd, lparam);
                let mods = mods_now();
                carry_out(hwnd, with_window(|e| e.session.press(p, mods)));
            }
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let info = lparam.0 as *mut MINMAXINFO;
            if !info.is_null() {
                // EDITOR_MIN is at 100%; the window is in physical pixels.
                let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96) as i32;
                unsafe {
                    (*info).ptMinTrackSize = POINT {
                        x: EDITOR_MIN.0 * dpi / 96,
                        y: EDITOR_MIN.1 * dpi / 96,
                    };
                }
            }
            LRESULT(0)
        }
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
            let p = pointer(hwnd);
            match with_window(|e| e.session.cursor(p)) {
                Some(c) => {
                    unsafe {
                        if let Ok(c) = LoadCursorW(None, cursor_id(c)) {
                            SetCursor(Some(c));
                        }
                    }
                    LRESULT(1)
                }
                None => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
            }
        }
        WM_LBUTTONDOWN => {
            let p = client_point(lparam);
            let mods = mods_now();
            carry_out(hwnd, with_window(|e| e.session.press(p, mods)));
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let p = client_point(lparam);
            let slop = drag_slop();
            let mods = mods_now();
            carry_out(hwnd, with_window(|e| e.session.motion(p, slop, mods)));
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            unsafe {
                let _ = ReleaseCapture();
            }
            let p = client_point(lparam);
            carry_out(hwnd, with_window(|e| e.session.release(p)));
            LRESULT(0)
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let w = Wheel::from(msg, wparam);
            let p = pointer(hwnd);
            carry_out(
                hwnd,
                with_window(|e| e.session.wheel(p, w.notches, w.sideways, w.ctrl)),
            );
            LRESULT(0)
        }
        WM_KEYUP => {
            if let Some(k) = editor_key(wparam.0 as u16) {
                with_window(|e| e.session.key_up(k));
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let vk = wparam.0 as u16;
            let mods = mods_now();
            let r = with_window(|e| editor_key(vk).map(|k| e.session.key(k, mods))).flatten();
            if r.as_ref().is_some_and(|r| r.used) {
                // The key was a shortcut: don't let the character it also
                // types reach WM_CHAR and start a label.
                let mut m = MSG::default();
                unsafe {
                    let _ = PeekMessageW(&mut m, Some(hwnd), WM_CHAR, WM_CHAR, PM_REMOVE);
                }
            }
            carry_out(hwnd, r);
            LRESULT(0)
        }
        WM_CHAR => {
            if let Some(c) = char::from_u32(wparam.0 as u32) {
                let p = pointer(hwnd);
                carry_out(hwnd, with_window(|e| e.session.type_char(c, p)));
            }
            LRESULT(0)
        }
        // Holding Alt previews the framed result.
        WM_SYSKEYDOWN | WM_SYSKEYUP if wparam.0 as u16 == VK_MENU.0 => {
            let on = msg == WM_SYSKEYDOWN;
            if with_window(|e| e.session.doc.set_preview(on)) == Some(true) {
                invalidate(hwnd);
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            close(hwnd);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Carry out a click on one of the window's own buttons.
fn window_button(hwnd: HWND, b: WindowButton) {
    match b {
        WindowButton::Minimize => unsafe {
            let _ = ShowWindow(hwnd, SW_MINIMIZE);
        },
        WindowButton::Maximize => unsafe {
            let how = if IsZoomed(hwnd).as_bool() {
                SW_RESTORE
            } else {
                SW_MAXIMIZE
            };
            let _ = ShowWindow(hwnd, how);
        },
        WindowButton::Close => close(hwnd),
    }
}

/// What Windows calls `part`, for `WM_NCHITTEST`.
fn hit_code(part: WindowPart) -> u32 {
    match part {
        WindowPart::Client => HTCLIENT,
        WindowPart::Caption => HTCAPTION,
        WindowPart::Button(WindowButton::Minimize) => HTMINBUTTON,
        WindowPart::Button(WindowButton::Maximize) => HTMAXBUTTON,
        WindowPart::Button(WindowButton::Close) => HTCLOSE,
        WindowPart::Edge { x, y } => match (x, y) {
            (0, 0) => HTTOPLEFT,
            (1, 0) => HTTOP,
            (2, 0) => HTTOPRIGHT,
            (0, 1) => HTLEFT,
            (2, 1) => HTRIGHT,
            (0, 2) => HTBOTTOMLEFT,
            (1, 2) => HTBOTTOM,
            _ => HTBOTTOMRIGHT,
        },
    }
}

fn is_button(hit: u32) -> bool {
    matches!(hit, HTMINBUTTON | HTMAXBUTTON | HTCLOSE)
}

/// A non-client message's screen point, in client pixels.
fn screen_to_client(hwnd: HWND, lparam: LPARAM) -> DocPoint {
    let mut p = POINT {
        x: (lparam.0 & 0xFFFF) as i16 as i32,
        y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
    };
    unsafe {
        let _ = ScreenToClient(hwnd, &mut p);
    }
    DocPoint::new(p.x as f32, p.y as f32)
}

/// Ask for `WM_NCMOUSELEAVE`, so a button's hover goes when the pointer
/// leaves it.
fn track_leave(hwnd: HWND) {
    let mut t = TRACKMOUSEEVENT {
        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE | TME_NONCLIENT,
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    unsafe {
        let _ = TrackMouseEvent(&mut t);
    }
}

/// The invisible system frame's thickness, which a maximised window has
/// off the screen's edge.
fn frame_thickness() -> (i32, i32) {
    unsafe {
        let pad = GetSystemMetrics(SM_CXPADDEDBORDER);
        (
            GetSystemMetrics(SM_CXFRAME) + pad,
            GetSystemMetrics(SM_CYFRAME) + pad,
        )
    }
}

/// Keep the system's shadow, rounded corners and resize borders round a
/// window whose frame is all client, and apply the new frame now.
fn use_own_frame(hwnd: HWND) {
    let margins = MARGINS {
        cxLeftWidth: 0,
        cxRightWidth: 0,
        cyTopHeight: 1,
        cyBottomHeight: 0,
    };
    let round = DWMWCP_ROUND;
    unsafe {
        let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &round as *const _ as *const _,
            size_of_val(&round) as u32,
        );
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
        );
    }
}

fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
    let (w, h) = client_size(hwnd);
    with_window(|e| {
        let (pixels, stride) = e.session.doc.render_window();
        blit_from(hdc, pixels, stride, (0, 0), (0, 0), (w, h));
    });
    unsafe {
        let _ = EndPaint(hwnd, &ps);
    }
}

/// Carry out what the session asked for, and repaint if it changed.
fn carry_out(hwnd: HWND, response: Option<Response>) {
    let Some(r) = response else { return };
    for effect in r.effects {
        match effect {
            Effect::CaptureMouse => unsafe {
                SetCapture(hwnd);
            },
            Effect::Window(b) => window_button(hwnd, b),
            Effect::OpenSettings => super::settings_window::post(super::settings_window::NEWS_OPEN),
            Effect::StartDragOut => drag_out(hwnd),
            Effect::Deliver { save, .. } => {
                let mut sw = Stopwatch::start();
                match with_window(|e| e.session.deliverable(save)) {
                    Some(Ok(d)) => {
                        let said = deliver(hwnd, d, &mut sw);
                        sw.report("editor copy");
                        let mut over = RECT::default();
                        if unsafe { GetWindowRect(hwnd, &mut over) }.is_ok() {
                            super::badge::toast(said, over);
                        }
                    }
                    Some(Err(e)) => log::line(format!("copy failed: {e}")),
                    None => {}
                }
            }
            Effect::CopyText(text) => super::editing::copy_text(hwnd, &text),
            // The window's capture holds all the pixels it can grow over,
            // and it neither cancels nor moves on.
            Effect::NeedDesktop(_) | Effect::Cancel | Effect::OpenWindow | Effect::Shared => {}
            Effect::Reveal(path) => super::system::reveal(&path),
            Effect::Log(line) => log::line(&line),
        }
    }
    if r.repaint {
        invalidate(hwnd);
    }
}

/// Drag the finished image out as a file. The window stays open whatever
/// happens.
fn drag_out(hwnd: HWND) {
    let sw = Stopwatch::start();
    let file = with_window(|e| e.session.drag_file(&drag_dir()));
    unsafe {
        let _ = ReleaseCapture();
    }
    let path = match file {
        Some(Ok((path, _))) => path,
        Some(Err(e)) => {
            log::line(format!("drag-out failed: {e}"));
            return;
        }
        None => return,
    };
    log::line(format!("drag-out file ready in {:.1} ms", sw.ms()));
    // The drag loop dispatches messages, so nothing is borrowed here.
    match shell_drag(hwnd, &path) {
        Ok(true) => log::line(format!("dragged out {}", path.display())),
        Ok(false) => log::line("drag-out cancelled"),
        Err(e) => log::line(format!("drag-out failed: {e}")),
    }
    invalidate(hwnd);
}

/// Close the window, keeping its capture in Recents.
fn close(hwnd: HWND) {
    let gone = WINDOW.with(|cell| cell.try_borrow_mut().ok().and_then(|mut c| c.take()));
    if let Some(mut w) = gone {
        log::line(w.session.closing_line());
        super::carry(|c| {
            c.retire(&mut w.session, |line| {
                log::line(&line);
                super::trim_memory();
            })
        });
    }
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
    super::trim_memory();
}
