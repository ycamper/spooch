//! Windows that float over everything, let clicks through and never take
//! focus: the countdown badge, and the toast after a copy.

use std::cell::RefCell;

use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
    ReleaseDC, SelectObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, KillTimer, RegisterClassExW, SW_SHOWNOACTIVATE,
    SetTimer, ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WNDCLASSEXW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::w;

use spooch_render::skia_safe::IRect;

/// One such window, showing premultiplied pixels of its size. Dropping it
/// takes it off the screen and waits for the desktop to be redrawn
/// without it, so a capture right after doesn't catch it.
pub(super) struct Badge {
    window: HWND,
    at: POINT,
    size: (i32, i32),
}

impl Badge {
    /// A badge `size` pixels big with its top-left corner at `at`, hidden
    /// until `show`.
    pub(super) fn new(at: POINT, size: (i32, i32)) -> Option<Badge> {
        extern "system" fn proc(
            hwnd: HWND,
            msg: u32,
            w: windows::Win32::Foundation::WPARAM,
            l: windows::Win32::Foundation::LPARAM,
        ) -> windows::Win32::Foundation::LRESULT {
            unsafe { DefWindowProcW(hwnd, msg, w, l) }
        }
        let window = unsafe {
            let instance = GetModuleHandleW(None).ok()?;
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(proc),
                hInstance: instance.into(),
                lpszClassName: w!("SpoochBadge"),
                ..Default::default()
            };
            // Fails harmlessly when it's already registered.
            RegisterClassExW(&class);
            CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TRANSPARENT
                    | WS_EX_NOACTIVATE,
                w!("SpoochBadge"),
                w!("Spooch"),
                WS_POPUP,
                at.x,
                at.y,
                size.0,
                size.1,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .ok()?
        };
        // No WDA_EXCLUDEFROMCAPTURE: while a window has it, desktop
        // duplication can hand back a black screen.
        Some(Badge { window, at, size })
    }

    /// Show `pixels`, premultiplied BGRA rows of the badge's size.
    pub(super) fn show(&self, pixels: &[u8]) {
        unsafe {
            let screen = GetDC(None);
            if let Some(dib) = super::editing::dib_with(Some(screen), self.size, pixels) {
                let src = CreateCompatibleDC(Some(screen));
                let old = SelectObject(src, dib.into());
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                let zero = POINT::default();
                let _ = UpdateLayeredWindow(
                    self.window,
                    Some(screen),
                    Some(&self.at),
                    Some(&SIZE {
                        cx: self.size.0,
                        cy: self.size.1,
                    }),
                    Some(src),
                    Some(&zero),
                    COLORREF(0),
                    Some(&blend),
                    ULW_ALPHA,
                );
                SelectObject(src, old);
                let _ = DeleteDC(src);
                let _ = DeleteObject(dib.into());
            }
            ReleaseDC(None, screen);
            let _ = ShowWindow(self.window, SW_SHOWNOACTIVATE);
        }
    }
}

impl Drop for Badge {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.window);
            let _ = DwmFlush();
        }
    }
}

/// The display scale, as the badges use it.
pub(super) fn scale() -> f32 {
    unsafe { GetDpiForSystem() as f32 / 96.0 }
}

thread_local! {
    /// The toast on screen, and the timer that takes it down.
    static TOAST: RefCell<Option<(Badge, usize)>> = const { RefCell::new(None) };
}

/// Say `text` in a toast centred on `over`, in screen pixels, for a
/// moment. It replaces any toast already up.
pub(super) fn toast(text: &str, over: RECT) {
    hide_toast();
    let ((w, h), pixels) = spooch_render::toast(text, scale(), None);
    let c = spooch_app::centred(
        IRect::from_ltrb(over.left, over.top, over.right, over.bottom),
        w,
        h,
    );
    let Some(badge) = Badge::new(
        POINT {
            x: c.left,
            y: c.top,
        },
        (w, h),
    ) else {
        return;
    };
    badge.show(&pixels);
    // A timer procedure, so it fires even inside a modal loop.
    let timer = unsafe { SetTimer(None, 0, spooch_render::TOAST_MS, Some(expired)) };
    TOAST.with(|t| *t.borrow_mut() = Some((badge, timer)));
}

/// Take the toast down, if one is up.
pub(super) fn hide_toast() {
    if let Some((badge, timer)) = TOAST.with(|t| t.borrow_mut().take()) {
        unsafe {
            let _ = KillTimer(None, timer);
        }
        drop(badge);
    }
}

extern "system" fn expired(_: HWND, _: u32, _: usize, _: u32) {
    hide_toast();
}
