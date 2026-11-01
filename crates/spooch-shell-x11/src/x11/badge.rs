//! Windows that float over everything and that the window manager leaves
//! alone: the countdown badge, and the toast after a copy.

use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt as _, CreateWindowAux, Gcontext, ImageFormat, Pixmap, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;

use spooch_render::skia_safe::Color4f;

/// The dark a badge's corners show over: an X window is square.
const BEHIND: u32 = 0x141519;

/// `BEHIND`, for drawing.
pub fn behind() -> Color4f {
    Color4f::new(
        ((BEHIND >> 16) & 0xff) as f32 / 255.0,
        ((BEHIND >> 8) & 0xff) as f32 / 255.0,
        (BEHIND & 0xff) as f32 / 255.0,
        1.0,
    )
}

/// One such window, showing pixels of its size.
pub struct Badge {
    window: Window,
    pixmap: Pixmap,
    size: (u16, u16),
}

/// What drawing a badge needs from the shell.
#[derive(Clone, Copy)]
pub struct Screen<'a> {
    pub conn: &'a RustConnection,
    pub root: Window,
    pub depth: u8,
    pub gc: Gcontext,
}

impl Badge {
    /// Show `pixels`, BGRA rows `size` big, with the top-left corner at
    /// `at`.
    pub fn show(
        s: Screen,
        at: (i32, i32),
        size: (u16, u16),
        pixels: &[u8],
    ) -> Result<Badge, String> {
        let pixmap = s.conn.generate_id().map_err(|e| e.to_string())?;
        s.conn
            .create_pixmap(s.depth, pixmap, s.root, size.0, size.1)
            .map_err(|e| e.to_string())?;
        let window = s.conn.generate_id().map_err(|e| e.to_string())?;
        s.conn
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                window,
                s.root,
                at.0 as i16,
                at.1 as i16,
                size.0,
                size.1,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::new()
                    .override_redirect(1)
                    .background_pixmap(pixmap),
            )
            .map_err(|e| e.to_string())?;
        let b = Badge {
            window,
            pixmap,
            size,
        };
        b.put(s, pixels);
        s.conn.map_window(window).map_err(|e| e.to_string())?;
        s.conn.flush().map_err(|e| e.to_string())?;
        Ok(b)
    }

    /// Show `pixels` instead.
    pub fn redraw(&self, s: Screen, pixels: &[u8]) {
        self.put(s, pixels);
        let _ = s.conn.clear_area(false, self.window, 0, 0, 0, 0);
        let _ = s.conn.flush();
    }

    fn put(&self, s: Screen, pixels: &[u8]) {
        let _ = s.conn.put_image(
            ImageFormat::Z_PIXMAP,
            self.pixmap,
            s.gc,
            self.size.0,
            self.size.1,
            0,
            0,
            0,
            s.depth,
            pixels,
        );
    }

    /// Take it off the screen, then wait for the server (and a
    /// compositor) to have redrawn without it, so a capture right after
    /// doesn't catch it.
    pub fn close(self, conn: &RustConnection) {
        let _ = conn.destroy_window(self.window);
        let _ = conn.free_pixmap(self.pixmap);
        if let Ok(c) = conn.get_input_focus() {
            let _ = c.reply();
        }
        std::thread::sleep(Duration::from_millis(120));
    }
}
