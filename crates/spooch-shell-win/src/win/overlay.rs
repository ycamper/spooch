//! The frozen-screen overlay: one borderless topmost window per monitor,
//! each showing that monitor's captured pixels, dimmed, with the selection
//! drawn at full brightness.
//!
//! Releasing the selection opens the editor in place (the shared
//! `Session`). Esc or a right-click cancels at any point.
//!
//! Selection is tracked in physical virtual-desktop pixels. Dragging across
//! monitors works because the window that received the mouse-down keeps
//! mouse capture and keeps receiving positions outside its own bounds.
//!
//! The dimmed frame and chrome are drawn with GDI straight from the captured
//! buffers. The selection itself, once it is being edited, is rendered by
//! the shared Skia document in `spooch-render` and blitted in.

use std::cell::RefCell;
use std::path::PathBuf;

use super::capture::{CapturedFrame, Image};
use spooch_app::{Effect, Pick, Picker, Session};
use spooch_render::skia_safe::{IRect, Point as DocPoint};
use spooch_render::{Counts, Format, OUTSIDE_BRIGHTNESS, Panel};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, AlphaBlend, BLENDFUNCTION, BeginPaint, BitBlt,
    CreateCompatibleBitmap, CreateCompatibleDC, CreateSolidBrush, DeleteDC, DeleteObject, EndPaint,
    FillRect, HBITMAP, HDC, HGDIOBJ, InvalidateRect, PAINTSTRUCT, SRCCOPY, SelectObject,
    UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, VK_MENU};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA,
    GetCursorPos, GetWindowLongPtrW, IDC_CROSS, LoadCursorW, MSG, PM_REMOVE, PeekMessageW,
    PostMessageW, RegisterClassExW, SW_HIDE, SW_SHOW, SetCursor, SetCursorPos, SetForegroundWindow,
    SetWindowLongPtrW, ShowWindow, WM_APP, WM_CHAR, WM_ERASEBKGND, WM_KEYDOWN, WM_KEYUP,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT, WM_RBUTTONUP,
    WM_SETCURSOR, WM_SYSKEYDOWN, WM_SYSKEYUP, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};
use windows::core::BOOL;
use windows::core::{PCWSTR, w};

use super::editing::{
    Wheel, blit_from, cursor_id, dib_with, drag_dir, drag_slop, editor_key, mods_now, shell_drag,
};
use super::log::{self, Stopwatch};

/// Posted to the UI thread when the user finishes or cancels a selection.
/// Teardown happens in the message loop, never inside a window procedure.
pub const WM_OVERLAY_DONE: u32 = WM_APP + 1;
/// Posted to an overlay window when a share's upload has news.
const WM_SHARE_NEWS: u32 = WM_APP + 8;

const CLASS_NAME: PCWSTR = w!("SpoochOverlay");

/// What the user chose, in physical virtual-desktop pixels.
#[derive(Clone, Debug)]
pub enum Outcome {
    /// The region, and whether to also save it.
    Region(RECT, bool),
    /// Shared; the link is on the clipboard.
    Shared,
    /// The region was dragged out as this file and dropped on another app.
    Dragged(RECT, Counts, PathBuf),
    /// Keep editing in the editor window (`Finished::editor`).
    Window,
    Cancelled,
}

/// Everything the overlay hands back when it closes.
pub struct Finished {
    pub outcome: Outcome,
    /// The finished image, `(width, height, straight BGRA)`, for copying
    /// and saving; encoded once the overlay is gone.
    pub image: Option<(u32, u32, Vec<u8>)>,
    pub format: Format,
    /// File name without extension, shown in the output bar.
    pub stem: String,
    pub counts: Counts,
    /// For `Outcome::Window`: the capture, to keep editing.
    pub editor: Option<Session>,
}

struct Surface {
    hwnd: HWND,
    /// Monitor bounds on the virtual desktop, physical pixels.
    bounds: RECT,
    /// The frozen frame at full brightness, top-down BGRA.
    image: Image,
    /// The same frame dimmed, same layout.
    dim: Vec<u8>,
    /// Display scale, for sizing annotations.
    scale: f64,
    /// Back buffer for flicker-free selection drawing, created on first drag.
    back: Option<MemBitmap>,
}

struct Overlay {
    surfaces: Vec<Surface>,
    /// The area being picked, and where the pointer is.
    picker: Picker,
    /// The loupe beside the pointer while selecting, from the core.
    loupe: Option<Panel>,
    /// Set once the selection is released.
    editor: Option<Session>,
    outcome: Option<Outcome>,
}

fn doc_point(p: POINT) -> DocPoint {
    DocPoint::new(p.x as f32, p.y as f32)
}

thread_local! {
    static OVERLAY: RefCell<Option<Overlay>> = const { RefCell::new(None) };
}

pub fn is_active() -> bool {
    OVERLAY.with(|o| o.borrow().is_some())
}

/// Show the overlay over every captured monitor. Returns once every window
/// has painted its first frame, so the caller can time "overlay visible".
pub fn show(frames: Vec<(RECT, CapturedFrame)>, sw: &mut Stopwatch) -> windows::core::Result<()> {
    register_class()?;
    let scales: Vec<f64> = frames.iter().map(|(_, f)| f.display.scale).collect();
    let images: Vec<(RECT, Image)> = frames.into_iter().map(|(r, f)| (r, f.image)).collect();
    let dims = dimmed_all(&images);
    sw.mark("dimmed frames");

    let mut surfaces = Vec::with_capacity(images.len());
    for (((bounds, image), dim), scale) in images.into_iter().zip(dims).zip(scales) {
        surfaces.push(create_surface(bounds, image, dim, scale)?);
    }
    sw.mark("created windows");
    let mut cursor = POINT::default();
    unsafe { GetCursorPos(&mut cursor)? };

    let hwnds: Vec<HWND> = surfaces.iter().map(|s| s.hwnd).collect();
    let focus = surfaces
        .iter()
        .find(|s| contains(&s.bounds, cursor))
        .or(surfaces.first())
        .map(|s| s.hwnd);

    let monitors = surfaces
        .iter()
        .map(|s| (irect(&s.bounds), s.scale as f32))
        .collect();
    let picker = Picker::new((cursor.x, cursor.y), monitors);
    OVERLAY.with(|o| {
        *o.borrow_mut() = Some(Overlay {
            surfaces,
            picker,
            loupe: None,
            editor: None,
            outcome: None,
        })
    });

    // Show and paint while cloaked, then uncloak every monitor together so
    // nothing appears half-drawn or blank.
    unsafe {
        for (i, hwnd) in hwnds.iter().enumerate() {
            SetWindowLongPtrW(*hwnd, GWLP_USERDATA, i as isize);
            set_cloaked(*hwnd, true);
            let _ = ShowWindow(*hwnd, SW_SHOW);
            let _ = UpdateWindow(*hwnd);
        }
        sw.mark("painted first frame");
        for hwnd in &hwnds {
            set_cloaked(*hwnd, false);
        }
        if let Some(hwnd) = focus {
            let _ = SetForegroundWindow(hwnd);
        }
    }
    Ok(())
}

fn set_cloaked(hwnd: HWND, cloaked: bool) {
    let value = BOOL::from(cloaked);
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CLOAK,
            &value as *const BOOL as *const _,
            size_of::<BOOL>() as u32,
        );
    }
}

/// Tear down the overlay and return what the user chose, plus the finished
/// image: the region's frozen pixels with any marks drawn on them.
/// Dropping the overlay destroys its windows and frees its bitmaps.
pub fn finish() -> Option<Finished> {
    let mut overlay = OVERLAY.with(|o| o.borrow_mut().take())?;
    let outcome = overlay.outcome.take().unwrap_or(Outcome::Cancelled);
    let image = match (&outcome, overlay.editor.as_mut()) {
        (Outcome::Cancelled | Outcome::Dragged(..) | Outcome::Window, _) => None,
        (Outcome::Region(..), Some(ed)) => Some(ed.export()),
        (Outcome::Shared, _) => None,
        (Outcome::Region(r, _), None) => {
            stitch(&overlay.surfaces, *r).map(|i| (i.width, i.height, i.data))
        }
    };
    if let Some(ed) = overlay.editor.as_ref() {
        super::carry(|c| c.remember(ed));
    }
    // Moving to the window, the capture goes along and is kept there.
    let editor = match outcome {
        Outcome::Window => overlay.editor.take(),
        _ => None,
    };
    // Every capture that reached the editor is kept, however it closed,
    // so Esc never loses work.
    if let Some(ed) = overlay.editor.as_mut() {
        super::carry(|c| {
            c.retire(ed, |line| {
                log::line(&line);
                // The pixels were freed only now, after the editor's own
                // trim, so give the memory back again.
                super::trim_memory();
            })
        });
    }
    let (format, stem, counts) = match &overlay.editor {
        Some(ed) => (ed.doc.format(), ed.stem().to_string(), ed.doc.counts()),
        None => (
            super::carry(|c| c.format),
            spooch_platform::save::file_stem(&super::save::now()),
            Counts::default(),
        ),
    };
    Some(Finished {
        outcome,
        image,
        format,
        stem,
        counts,
        editor,
    })
}

/// Register the window class ahead of the first capture.
pub fn prepare() -> windows::core::Result<()> {
    register_class()
}

fn register_class() -> windows::core::Result<()> {
    thread_local!(static REGISTERED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
    if REGISTERED.with(|r| r.get()) {
        return Ok(());
    }
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_CROSS)?,
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

fn create_surface(
    bounds: RECT,
    image: Image,
    dim: Vec<u8>,
    scale: f64,
) -> windows::core::Result<Surface> {
    let (w, h) = (bounds.right - bounds.left, bounds.bottom - bounds.top);
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            CLASS_NAME,
            w!("Spooch"),
            WS_POPUP,
            bounds.left,
            bounds.top,
            w,
            h,
            None,
            None,
            Some(GetModuleHandleW(None)?.into()),
            None,
        )?
    };
    Ok(Surface {
        hwnd,
        bounds,
        image,
        dim,
        scale,
        back: None,
    })
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_SETCURSOR => {
            let mut p = POINT::default();
            if unsafe { GetCursorPos(&mut p) }.is_ok() && set_cursor_at(p) {
                return LRESULT(1);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_LBUTTONDOWN => {
            let p = to_virtual(hwnd, lparam);
            let mods = mods_now();
            let after = with_overlay(|o| {
                o.picker.motion((p.x, p.y));
                let Some(ed) = o.editor.as_mut() else {
                    o.picker.press((p.x, p.y));
                    return After {
                        capture: true,
                        ..After::default()
                    };
                };
                let r = ed.press(doc_point(p), mods);
                carry_out(o, r.effects)
            })
            .unwrap_or_default();
            if after.capture {
                unsafe { SetCapture(hwnd) };
            }
            invalidate_all();
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let p = to_virtual(hwnd, lparam);
            let slop = drag_slop();
            let (changed, after) = with_overlay(|o| {
                o.picker.motion((p.x, p.y));
                match o.editor.as_mut() {
                    Some(ed) => {
                        let r = ed.motion(doc_point(p), slop, mods_now());
                        (r.repaint, carry_out(o, r.effects))
                    }
                    None => {
                        update_loupe(o);
                        (true, After::default())
                    }
                }
            })
            .unwrap_or_default();
            if after.drag_out {
                drag_out(hwnd);
                return LRESULT(0);
            }
            set_cursor_at(p);
            if changed {
                invalidate_all();
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let p = to_virtual(hwnd, lparam);
            unsafe {
                let _ = ReleaseCapture();
            }
            with_overlay(|o| {
                o.picker.motion((p.x, p.y));
                if let Some(ed) = o.editor.as_mut() {
                    ed.release(doc_point(p));
                    return;
                }
                let pick = o.picker.release((p.x, p.y));
                picked(o, pick);
            });
            invalidate_all();
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let w = Wheel::from(msg, wparam);
            let changed = with_overlay(|o| {
                let (x, y) = o.picker.pointer();
                let p = DocPoint::new(x as f32, y as f32);
                o.editor
                    .as_mut()
                    .is_some_and(|ed| ed.wheel(p, w.notches, w.sideways, w.ctrl).repaint)
            });
            if changed == Some(true) {
                invalidate_all();
            }
            LRESULT(0)
        }
        WM_KEYUP => {
            if let Some(k) = editor_key(wparam.0 as u16) {
                with_overlay(|o| o.editor.as_mut().map(|ed| ed.key_up(k)));
            }
            LRESULT(0)
        }
        WM_CHAR => {
            // Text a key typed, which the editor's `key` didn't use.
            let changed = char::from_u32(wparam.0 as u32).and_then(|c| {
                with_overlay(|o| {
                    let (x, y) = o.picker.pointer();
                    let p = DocPoint::new(x as f32, y as f32);
                    o.editor.as_mut().map(|ed| ed.type_char(c, p).repaint)
                })
                .flatten()
            });
            if changed == Some(true) {
                invalidate_all();
            }
            LRESULT(0)
        }
        WM_SHARE_NEWS => {
            with_overlay(|o| {
                if let Some(ed) = o.editor.as_mut() {
                    let r = ed.poll();
                    carry_out(o, r.effects);
                }
            });
            invalidate_all();
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            with_overlay(|o| o.outcome = Some(Outcome::Cancelled));
            post_done();
            LRESULT(0)
        }
        // Holding Alt previews the framed result. Plain Alt would otherwise
        // go to the (missing) menu bar, so it stops here; Alt+F4 and the
        // like still reach Windows.
        WM_SYSKEYDOWN | WM_SYSKEYUP if wparam.0 as u16 == VK_MENU.0 => {
            let on = msg == WM_SYSKEYDOWN;
            let changed =
                with_overlay(|o| o.editor.as_mut().is_some_and(|ed| ed.doc.set_preview(on)));
            if changed == Some(true) {
                invalidate_all();
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let vk = wparam.0 as u16;
            let mods = mods_now();
            let response = with_overlay(|o| {
                let Some(ed) = o.editor.as_mut() else {
                    let pick = editor_key(vk).and_then(|k| o.picker.key(k, mods.shift));
                    picked(o, pick);
                    return None;
                };
                let r = ed.key(editor_key(vk)?, mods);
                carry_out(o, r.effects.clone());
                Some(r)
            })
            .flatten();
            if let Some(r) = response {
                if r.used {
                    // The key was a shortcut: don't let the "!" or "]" it
                    // also types reach WM_CHAR and start a label.
                    let mut m = MSG::default();
                    unsafe {
                        let _ = PeekMessageW(&mut m, Some(hwnd), WM_CHAR, WM_CHAR, PM_REMOVE);
                    }
                }
                if r.repaint {
                    invalidate_all();
                }
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Run `f` against the overlay state if it is present and not already
/// borrowed (window procedures can re-enter through synchronous messages).
fn with_overlay<R>(f: impl FnOnce(&mut Overlay) -> R) -> Option<R> {
    OVERLAY.with(|o| o.try_borrow_mut().ok()?.as_mut().map(f))
}

/// Carry out what picking came to.
fn picked(o: &mut Overlay, pick: Option<Pick>) {
    match pick {
        Some(Pick::Edit(area)) => start_editing(o, area),
        Some(Pick::WarpPointer((x, y))) => {
            // The mouse move this makes updates the selection.
            unsafe {
                let _ = SetCursorPos(x, y);
            }
        }
        Some(Pick::Cancel) => {
            o.outcome = Some(Outcome::Cancelled);
            post_done();
        }
        None => {}
    }
}

/// Turn the picked area into the editor: its frozen pixels become a
/// document that marks are drawn onto.
fn start_editing(o: &mut Overlay, area: IRect) {
    let setup = super::carry(|c| c.setup(super::save::now()));
    let session = stitch(&o.surfaces, rect(area)).and_then(|px| {
        o.picker
            .edit(area, (px.width, px.height, px.stride, &px.data), setup)
    });
    match session {
        Some(session) => {
            o.loupe = None;
            edit(o, session);
        }
        None => {
            o.outcome = Some(Outcome::Cancelled);
            post_done();
        }
    }
}

/// Edit `session` here, its news coming back as WM_SHARE_NEWS.
fn edit(o: &mut Overlay, mut session: Session) {
    if let Some(s) = o.surfaces.first() {
        session.set_waker(super::editing::waker(s.hwnd, WM_SHARE_NEWS));
    }
    o.editor = Some(session);
}

/// Open the kept capture `id` in the editor, replacing any open one.
fn open_recent(o: &mut Overlay, id: &str) -> bool {
    match super::carry(|c| c.reopen(id, o.picker.screens())) {
        Ok((session, line)) => {
            log::line(&line);
            edit(o, session);
            true
        }
        Err(e) => {
            log::line(&e);
            false
        }
    }
}

/// Reopen the kept capture `id` in the overlay that was just shown.
pub fn reopen(id: &str) {
    let opened = with_overlay(|o| open_recent(o, id)) == Some(true);
    if !opened {
        with_overlay(|o| o.outcome = Some(Outcome::Cancelled));
        post_done();
    }
    invalidate_all();
}

/// Blend one of the core's chrome panels (premultiplied BGRA, rounded
/// corners transparent) onto a monitor's back buffer.
fn blend_panel(dc: HDC, panel: &Panel, monitor: &RECT) {
    let (w, h) = (panel.rect.width(), panel.rect.height());
    if w <= 0 || h <= 0 {
        return;
    }
    let (x, y) = (panel.rect.left - monitor.left, panel.rect.top - monitor.top);
    // Skip panels on other monitors.
    if x >= monitor.right - monitor.left
        || y >= monitor.bottom - monitor.top
        || x + w <= 0
        || y + h <= 0
    {
        return;
    }
    let Some(dib) = dib_with(Some(dc), (w, h), &panel.pixels) else {
        return;
    };
    unsafe {
        let src = CreateCompatibleDC(Some(dc));
        let old = SelectObject(src, dib.into());
        let _ = AlphaBlend(
            dc,
            x,
            y,
            w,
            h,
            src,
            0,
            0,
            w,
            h,
            BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            },
        );
        SelectObject(src, old);
        let _ = DeleteDC(src);
        let _ = DeleteObject(dib.into());
    }
}

/// What the overlay has to do after carrying out the session's effects.
#[derive(Clone, Copy, Debug, Default)]
struct After {
    /// Take the mouse until the button is let go.
    capture: bool,
    /// Run the drag-out loop, once the overlay isn't borrowed.
    drag_out: bool,
}

/// Carry out what the session asked for.
fn carry_out(o: &mut Overlay, effects: Vec<Effect>) -> After {
    let mut after = After::default();
    for effect in effects {
        match effect {
            Effect::CaptureMouse => after.capture = true,
            Effect::NeedDesktop(want) => supply_desktop(o, want),
            Effect::StartDragOut => after.drag_out = true,
            Effect::Deliver { save, .. } => {
                if let Some(ed) = &o.editor {
                    o.outcome = Some(Outcome::Region(rect(ed.doc.canvas()), save));
                }
                post_done();
            }
            Effect::CopyText(text) => {
                if let Some(s) = o.surfaces.first() {
                    super::editing::copy_text(s.hwnd, &text);
                }
            }
            Effect::Shared => {
                o.outcome = Some(Outcome::Shared);
                post_done();
            }
            Effect::Cancel => {
                o.outcome = Some(Outcome::Cancelled);
                post_done();
            }
            Effect::OpenWindow => {
                o.outcome = Some(Outcome::Window);
                post_done();
            }
            // The overlay has no window buttons or gallery.
            Effect::Window(_) | Effect::Reveal(_) => {}
            Effect::Log(line) => log::line(&line),
        }
    }
    after
}

/// Hand the editor the frozen desktop under `want`.
fn supply_desktop(o: &mut Overlay, want: IRect) {
    let sw = Stopwatch::start();
    let Some(pixels) = stitch(&o.surfaces, rect(want)) else {
        return;
    };
    let Some(ed) = o.editor.as_mut() else {
        return;
    };
    let size = (pixels.width, pixels.height, pixels.stride);
    if let Some(line) = ed.supply_desktop(size, &pixels.data, sw.ms()) {
        log::line(&line);
    }
}

/// Map the document's cursor to a system one and show it. Returns whether
/// the editor chose one; outside the editor the class cursor applies.
fn set_cursor_at(p: POINT) -> bool {
    let id = with_overlay(|o| Some(cursor_id(o.editor.as_ref()?.cursor(doc_point(p))))).flatten();
    let Some(id) = id else {
        return false;
    };
    unsafe {
        if let Ok(c) = LoadCursorW(None, id) {
            SetCursor(Some(c));
        }
    }
    true
}

/// Drag the finished image out as a file: write it to a temporary folder,
/// hide the overlay so the windows under it can take the drop, and run the
/// shell's drag loop. A drop finishes the capture; a cancelled drag brings
/// the editor back as it was.
fn drag_out(hwnd: HWND) {
    let sw = Stopwatch::start();
    let prepared = with_overlay(|o| {
        let ed = o.editor.as_mut()?;
        match ed.drag_file(&drag_dir()) {
            Ok((path, _)) => Some((
                path,
                rect(ed.doc.canvas()),
                ed.doc.counts(),
                o.surfaces.iter().map(|s| s.hwnd).collect::<Vec<_>>(),
            )),
            Err(e) => {
                log::line(format!("drag-out failed: {e}"));
                None
            }
        }
    })
    .flatten();
    let Some((path, region, counts, hwnds)) = prepared else {
        unsafe {
            let _ = ReleaseCapture();
        }
        return;
    };
    log::line(format!(
        "drag-out file ready in {:.1} ms: {}",
        sw.ms(),
        path.display()
    ));
    unsafe {
        let _ = ReleaseCapture();
        for h in &hwnds {
            let _ = ShowWindow(*h, SW_HIDE);
        }
    }
    // The drag loop dispatches messages, so no borrow of the overlay may be
    // held here.
    match shell_drag(hwnd, &path) {
        Ok(true) => {
            with_overlay(|o| o.outcome = Some(Outcome::Dragged(region, counts, path)));
            post_done();
        }
        result => {
            match result {
                Err(e) => log::line(format!("drag-out failed: {e}")),
                _ => log::line("drag-out cancelled"),
            }
            unsafe {
                for h in &hwnds {
                    let _ = ShowWindow(*h, SW_SHOW);
                }
                let _ = SetForegroundWindow(hwnd);
            }
            invalidate_all();
        }
    }
}

fn post_done() {
    unsafe {
        let _ = PostMessageW(None, WM_OVERLAY_DONE, WPARAM(0), LPARAM(0));
    }
}

fn invalidate_all() {
    let hwnds: Vec<HWND> = OVERLAY.with(|o| {
        o.try_borrow()
            .ok()
            .and_then(|o| {
                o.as_ref()
                    .map(|o| o.surfaces.iter().map(|s| s.hwnd).collect())
            })
            .unwrap_or_default()
    });
    for hwnd in hwnds {
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
}

fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
    let index = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as usize;
    with_overlay(|o| {
        let Overlay {
            surfaces,
            picker,
            editor,
            loupe,
            ..
        } = o;
        let selection = match editor.as_ref() {
            Some(ed) => Some(rect(ed.doc.canvas())),
            None => picker.selection().map(rect),
        };
        if let Some(s) = surfaces.get_mut(index) {
            paint_surface(hdc, s, selection, editor.as_mut(), loupe.as_ref());
        }
    });
    unsafe {
        let _ = EndPaint(hwnd, &ps);
    }
}

fn paint_surface(
    hdc: HDC,
    s: &mut Surface,
    selection: Option<RECT>,
    mut editor: Option<&mut Session>,
    loupe: Option<&Panel>,
) {
    let (w, h) = (
        s.bounds.right - s.bounds.left,
        s.bounds.bottom - s.bounds.top,
    );
    let local = selection.and_then(|sel| {
        intersect(&sel, &s.bounds).map(|r| (sel, offset(&r, -s.bounds.left, -s.bounds.top)))
    });
    let loupe = loupe.filter(|l| {
        let r = &l.rect;
        let r = RECT {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        };
        intersect(&r, &s.bounds).is_some()
    });
    if local.is_none() && loupe.is_none() {
        // Nothing on this monitor: the dimmed frame goes straight to the
        // window. This is the first paint, so keep it to one copy.
        blit_whole(hdc, &s.dim, &s.image, w);
        return;
    }

    if s.back.is_none() {
        s.back = MemBitmap::compatible(hdc, w, h);
    }
    let Some(back) = s.back.as_ref() else {
        return;
    };
    unsafe {
        let back = back.dc;
        blit_whole(back, &s.dim, &s.image, w);
        let Some((sel, local)) = local else {
            // Only the loupe is here.
            if let Some(l) = loupe {
                blend_panel(back, l, &s.bounds);
            }
            let _ = BitBlt(hdc, 0, 0, w, h, Some(back), 0, 0, SRCCOPY);
            return;
        };
        let (lw, lh) = (local.right - local.left, local.bottom - local.top);
        match editor.as_deref_mut() {
            // While editing, the selection comes from the document, which
            // covers the whole region; this monitor shows its part.
            Some(ed) => {
                let (pixels, stride) = ed.doc.render();
                blit_from(
                    back,
                    pixels,
                    stride,
                    (
                        s.bounds.left + local.left - sel.left,
                        s.bounds.top + local.top - sel.top,
                    ),
                    (local.left, local.top),
                    (lw, lh),
                );
            }
            None => blit_from(
                back,
                &s.image.data,
                s.image.width as usize * 4,
                (local.left, local.top),
                (local.left, local.top),
                (lw, lh),
            ),
        }

        // The outline, and once editing the crop handles, from the core.
        let on_surface = offset(&sel, -s.bounds.left, -s.bounds.top);
        for (r, rgb) in spooch_render::selection_frame(irect(&on_surface), editor.is_some()) {
            // COLORREF is 0x00BBGGRR.
            let bgr = ((rgb & 0xff) << 16) | (rgb & 0xff00) | (rgb >> 16);
            let brush = CreateSolidBrush(COLORREF(bgr));
            FillRect(back, &rect(r), brush);
            let _ = DeleteObject(brush.into());
        }
        if editor.is_none()
            && let Some(l) = loupe
        {
            // While selecting, the core's loupe shows the size.
            blend_panel(back, l, &s.bounds);
        }

        // The editor's chrome (rail, style row, output bar), drawn by the
        // core, goes on top of everything.
        if let Some(ed) = editor {
            for panel in ed.doc.chrome() {
                blend_panel(back, panel, &s.bounds);
            }
        }

        let _ = BitBlt(hdc, 0, 0, w, h, Some(back), 0, 0, SRCCOPY);
    }
}

/// `data`, the size of `image`, onto `dc` from the top left, `w` wide.
fn blit_whole(dc: HDC, data: &[u8], image: &Image, w: i32) {
    let size = (w, image.height as i32);
    blit_from(dc, data, image.width as usize * 4, (0, 0), (0, 0), size);
}

fn to_virtual(hwnd: HWND, lparam: LPARAM) -> POINT {
    // Client coordinates are signed 16-bit and can be negative while the
    // mouse is captured and outside the window.
    let x = (lparam.0 & 0xFFFF) as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
    let index = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as usize;
    let origin = OVERLAY.with(|o| {
        o.try_borrow()
            .ok()
            .and_then(|o| {
                o.as_ref()
                    .and_then(|o| o.surfaces.get(index).map(|s| (s.bounds.left, s.bounds.top)))
            })
            .unwrap_or((0, 0))
    });
    POINT {
        x: x + origin.0,
        y: y + origin.1,
    }
}

/// The loupe for where the pointer is now, drawn by the core from the
/// frozen pixels round it (stitched across monitors, black between them).
fn update_loupe(o: &mut Overlay) {
    let loupe = o
        .picker
        .loupe(|r| stitch(&o.surfaces, rect(r)).map(|img| img.data));
    o.loupe = loupe;
}

fn rect(r: IRect) -> RECT {
    RECT {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    }
}

fn irect(r: &RECT) -> IRect {
    IRect::from_ltrb(r.left, r.top, r.right, r.bottom)
}

/// Copy the selected region out of each monitor's frozen frame into one image.
/// Monitors are joined in physical pixels; mixed-DPI resampling comes later.
fn stitch(surfaces: &[Surface], sel: RECT) -> Option<Image> {
    if sel.right <= sel.left || sel.bottom <= sel.top {
        return None;
    }
    let (w, h) = ((sel.right - sel.left) as u32, (sel.bottom - sel.top) as u32);
    let stride = w as usize * 4;
    let mut data = vec![0u8; stride * h as usize];
    for s in surfaces {
        let Some(part) = intersect(&sel, &s.bounds) else {
            continue;
        };
        let local = offset(&part, -s.bounds.left, -s.bounds.top);
        let rows = s.image.rows_in(IRect::from_ltrb(
            local.left,
            local.top,
            local.right,
            local.bottom,
        ))?;
        let (dx, dy) = (
            (part.left - sel.left) as usize,
            (part.top - sel.top) as usize,
        );
        for (i, src) in rows.enumerate() {
            let dst = (dy + i) * stride + dx * 4;
            data[dst..dst + src.len()].copy_from_slice(src);
        }
    }
    Some(Image {
        width: w,
        height: h,
        stride,
        data,
    })
}

/// Dim every frame to `OUTSIDE_BRIGHTNESS`, spread across all cores. The output
/// buffers are fresh allocations, so their page faults are spread too.
fn dimmed_all(images: &[(RECT, Image)]) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = images
        .iter()
        .map(|(_, img)| vec![0u8; img.data.len()])
        .collect();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|s| {
        for ((_, img), dst) in images.iter().zip(out.iter_mut()) {
            // Whole pixels per chunk, at least one chunk.
            let per = img.data.len().div_ceil(threads).next_multiple_of(4).max(4);
            for (src, dst) in img.data.chunks(per).zip(dst.chunks_mut(per)) {
                s.spawn(move || {
                    for (d, p) in dst
                        .as_chunks_mut::<4>()
                        .0
                        .iter_mut()
                        .zip(src.as_chunks::<4>().0)
                    {
                        let keep = |c: u8| (u16::from(c) * OUTSIDE_BRIGHTNESS / 100) as u8;
                        *d = [keep(p[0]), keep(p[1]), keep(p[2]), p[3]];
                    }
                });
            }
        }
    });
    out
}

fn contains(r: &RECT, p: POINT) -> bool {
    p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom
}

fn intersect(a: &RECT, b: &RECT) -> Option<RECT> {
    let r = RECT {
        left: a.left.max(b.left),
        top: a.top.max(b.top),
        right: a.right.min(b.right),
        bottom: a.bottom.min(b.bottom),
    };
    (r.right > r.left && r.bottom > r.top).then_some(r)
}

fn offset(r: &RECT, dx: i32, dy: i32) -> RECT {
    RECT {
        left: r.left + dx,
        top: r.top + dy,
        right: r.right + dx,
        bottom: r.bottom + dy,
    }
}

/// A bitmap selected into its own memory DC.
struct MemBitmap {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
}

impl MemBitmap {
    /// A device-compatible bitmap, which normally lives in video memory
    /// rather than this process's heap.
    fn compatible(like: HDC, w: i32, h: i32) -> Option<Self> {
        unsafe {
            let bitmap = CreateCompatibleBitmap(like, w, h);
            if bitmap.is_invalid() {
                return None;
            }
            let dc = CreateCompatibleDC(Some(like));
            let previous = SelectObject(dc, bitmap.into());
            Some(Self {
                dc,
                bitmap,
                previous,
            })
        }
    }
}

impl Drop for MemBitmap {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
