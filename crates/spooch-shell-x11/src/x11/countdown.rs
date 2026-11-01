//! The 3, 2, 1 badge before a delayed capture, in the middle of the
//! pointer's screen. A helper thread ticks the event loop.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use x11rb::rust_connection::RustConnection;

use super::badge::{Badge, Screen, behind};

/// The badge's size in pixels.
pub const SIZE: u16 = spooch_render::COUNTDOWN_PX as u16;
/// Marks a wake-up message as a countdown tick.
pub const TICK: u32 = 1;

/// The badge on screen and the thread ticking each second. Which number to
/// show is `spooch_platform::app::Flow`'s business.
pub struct Countdown {
    badge: Badge,
    shown: u32,
    /// Which countdown this is, so a stale tick is ignored.
    pub generation: u32,
    stop: Arc<AtomicBool>,
}

impl Countdown {
    /// Show the badge with its top-left corner at `at` and tick every
    /// second through `wake(TICK, generation)`.
    pub fn start(
        s: Screen,
        at: (i32, i32),
        n: u32,
        generation: u32,
        wake: impl Fn(u32, u32) + Send + 'static,
    ) -> Result<Countdown, String> {
        let badge = Badge::show(s, at, (SIZE, SIZE), &pixels(n))?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                if stopped.load(Ordering::Relaxed) {
                    return;
                }
                wake(TICK, generation);
            }
        });
        Ok(Countdown {
            badge,
            shown: n,
            generation,
            stop,
        })
    }

    /// Show `n` instead.
    pub fn set(&mut self, s: Screen, n: u32) {
        if n != self.shown {
            self.shown = n;
            self.badge.redraw(s, &pixels(n));
        }
    }

    /// Take the badge off the screen and stop the ticks.
    pub fn close(self, conn: &RustConnection) {
        self.stop.store(true, Ordering::Relaxed);
        self.badge.close(conn);
    }
}

fn pixels(n: u32) -> Vec<u8> {
    spooch_render::countdown_badge(n, u32::from(SIZE), Some(behind()))
}
