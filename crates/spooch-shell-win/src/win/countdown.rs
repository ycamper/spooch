//! The 3, 2, 1 badge before a delayed capture, in the middle of the
//! pointer's screen. It's gone, and the desktop redrawn, before the
//! capture.

use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, KillTimer, SetTimer};

use super::badge::Badge;

/// The badge on screen and its one-second timer. Which number to show is
/// `spooch_platform::app::Flow`'s business.
pub struct Countdown {
    badge: Badge,
    shown: u32,
    timer: usize,
    size: u32,
}

impl Countdown {
    /// Show `n` in the middle of the pointer's screen and start a
    /// one-second timer, which arrives as a thread `WM_TIMER` for
    /// `is_timer`.
    pub fn start(n: u32) -> Option<Countdown> {
        let size = (spooch_render::COUNTDOWN_PX as f32 * super::badge::scale()).round() as i32;
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
        let badge = Badge::new(
            POINT {
                x: c.left,
                y: c.top,
            },
            (size, size),
        )?;
        let timer = unsafe { SetTimer(None, 0, 1000, None) };
        let c = Countdown {
            badge,
            shown: n,
            timer,
            size: size as u32,
        };
        c.show();
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

    fn show(&self) {
        self.badge
            .show(&spooch_render::countdown_badge(self.shown, self.size, None));
    }
}

impl Drop for Countdown {
    fn drop(&mut self) {
        // The badge goes after this, waiting for the redraw.
        unsafe {
            let _ = KillTimer(None, self.timer);
        }
    }
}
