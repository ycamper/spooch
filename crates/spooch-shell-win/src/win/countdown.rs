//! The 3, 2, 1 badge before a delayed capture, in the middle of the
//! pointer's screen. It's gone, and the desktop redrawn, before the
//! capture.

use windows::Win32::Foundation::{COLORREF, HWND, POINT, SIZE};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, ReleaseDC,
    SelectObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, KillTimer, RegisterClassExW,
    SW_SHOWNOACTIVATE, SetTimer, ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WNDCLASSEXW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::w;

/// The badge on screen and its one-second timer. Which number to show is
/// `spooch_platform::app::Flow`'s business.
pub struct Countdown {
    window: HWND,
    shown: u32,
    timer: usize,
    size: i32,
    at: POINT,
}

impl Countdown {
    /// Show `n` in the middle of the pointer's screen and start a
    /// one-second timer, which arrives as a thread `WM_TIMER` for
    /// `is_timer`.
    pub fn start(n: u32) -> Option<Countdown> {
        extern "system" fn proc(
            hwnd: HWND,
            msg: u32,
            w: windows::Win32::Foundation::WPARAM,
            l: windows::Win32::Foundation::LPARAM,
        ) -> windows::Win32::Foundation::LRESULT {
            unsafe { DefWindowProcW(hwnd, msg, w, l) }
        }
        let scale = unsafe { GetDpiForSystem() } as f32 / 96.0;
        let size = (spooch_render::COUNTDOWN_PX as f32 * scale).round() as i32;
        let mut p = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut p);
        }
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        unsafe {
            let monitor = MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST);
            let _ = GetMonitorInfoW(monitor, &mut info);
        }
        let m = info.rcMonitor;
        let c = spooch_app::centred(
            spooch_render::skia_safe::IRect::from_ltrb(m.left, m.top, m.right, m.bottom),
            size,
            size,
        );
        let at = POINT {
            x: c.left,
            y: c.top,
        };
        let window = unsafe {
            let instance = GetModuleHandleW(None).ok()?;
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(proc),
                hInstance: instance.into(),
                lpszClassName: w!("SpoochCountdown"),
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
                w!("SpoochCountdown"),
                w!("Spooch countdown"),
                WS_POPUP,
                at.x,
                at.y,
                size,
                size,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .ok()?
        };
        // No WDA_EXCLUDEFROMCAPTURE: while a window has it, desktop
        // duplication can hand back a black screen.
        let timer = unsafe { SetTimer(None, 0, 1000, None) };
        let c = Countdown {
            window,
            shown: n,
            timer,
            size,
            at,
        };
        c.show();
        unsafe {
            let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
        }
        Some(c)
    }

    /// Whether a thread `WM_TIMER` with this id is ours.
    pub fn is_timer(&self, id: usize) -> bool {
        id == self.timer
    }

    /// Show `n` instead.
    pub fn set(&mut self, n: u32) {
        if n != self.shown {
            self.shown = n;
            self.show();
        }
    }

    /// Draw the current number into the layered window.
    fn show(&self) {
        let (w, h) = (self.size, self.size);
        let pixels = spooch_render::countdown_badge(self.shown, self.size as u32, None);
        unsafe {
            let screen = GetDC(None);
            if let Some(dib) = super::editing::dib_with(Some(screen), (w, h), &pixels) {
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
                    Some(&SIZE { cx: w, cy: h }),
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
        }
    }
}

impl Drop for Countdown {
    fn drop(&mut self) {
        unsafe {
            let _ = KillTimer(None, self.timer);
            let _ = DestroyWindow(self.window);
            // Wait for the desktop to be redrawn without it.
            let _ = DwmFlush();
        }
    }
}
