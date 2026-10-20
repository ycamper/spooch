//! The 3, 2, 1 badge before a delayed capture, in the middle of the
//! pointer's screen. A helper thread ticks the event loop.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt as _, CreateWindowAux, Gcontext, ImageFormat, Pixmap, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;

/// The badge's size in pixels.
pub const SIZE: u16 = spooch_render::COUNTDOWN_PX as u16;
/// The dark the badge's corners show over: an X window is square.
const BEHIND: u32 = 0x141519;
/// Marks a wake-up message as a countdown tick.
pub const TICK: u32 = 1;

/// The badge on screen and the thread ticking each second. Which number to
/// show is `spooch_platform::app::Flow`'s business.
pub struct Countdown {
    window: Window,
    pixmap: Pixmap,
    shown: u32,
    /// Which countdown this is, so a stale tick is ignored.
    pub generation: u32,
    stop: Arc<AtomicBool>,
}

impl Countdown {
    /// Show the badge with its top-left corner at `at` and tick every
    /// second through `wake(TICK, generation)`.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        conn: &RustConnection,
        root: Window,
        depth: u8,
        gc: Gcontext,
        at: (i32, i32),
        n: u32,
        generation: u32,
        wake: impl Fn(u32, u32) + Send + 'static,
    ) -> Result<Countdown, String> {
        let pixmap = conn.generate_id().map_err(|e| e.to_string())?;
        conn.create_pixmap(depth, pixmap, root, SIZE, SIZE)
            .map_err(|e| e.to_string())?;
        let window = conn.generate_id().map_err(|e| e.to_string())?;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            root,
            at.0 as i16,
            at.1 as i16,
            SIZE,
            SIZE,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new()
                .override_redirect(1)
                .background_pixmap(pixmap),
        )
        .map_err(|e| e.to_string())?;
        let c = Countdown {
            window,
            pixmap,
            shown: n,
            generation,
            stop: Arc::new(AtomicBool::new(false)),
        };
        c.draw(conn, gc, depth);
        conn.map_window(window).map_err(|e| e.to_string())?;
        conn.flush().map_err(|e| e.to_string())?;
        let stop = c.stop.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                wake(TICK, generation);
            }
        });
        Ok(c)
    }

    /// Show `n` instead.
    pub fn set(&mut self, conn: &RustConnection, gc: Gcontext, depth: u8, n: u32) {
        if n == self.shown {
            return;
        }
        self.shown = n;
        self.draw(conn, gc, depth);
        let _ = conn.clear_area(false, self.window, 0, 0, 0, 0);
        let _ = conn.flush();
    }

    fn draw(&self, conn: &RustConnection, gc: Gcontext, depth: u8) {
        let behind = spooch_render::skia_safe::Color4f::new(
            ((BEHIND >> 16) & 0xff) as f32 / 255.0,
            ((BEHIND >> 8) & 0xff) as f32 / 255.0,
            (BEHIND & 0xff) as f32 / 255.0,
            1.0,
        );
        let pixels = spooch_render::countdown_badge(self.shown, u32::from(SIZE), Some(behind));
        let _ = conn.put_image(
            ImageFormat::Z_PIXMAP,
            self.pixmap,
            gc,
            SIZE,
            SIZE,
            0,
            0,
            0,
            depth,
            &pixels,
        );
    }

    /// Take the badge off the screen and stop the ticks, then wait for the
    /// server (and a compositor) to have redrawn without it.
    pub fn close(self, conn: &RustConnection) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = conn.destroy_window(self.window);
        let _ = conn.free_pixmap(self.pixmap);
        if let Ok(c) = conn.get_input_focus() {
            let _ = c.reply();
        }
        std::thread::sleep(Duration::from_millis(120));
    }
}
