//! The tray menu: a plain override-redirect window drawn with the core
//! font, since the shell has no toolkit. A click on an item picks it; a
//! click anywhere else, or Esc, closes it.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ChangeGCAux, ConnectionExt as _, CreateWindowAux, EventMask, Gcontext, GrabMode, Rectangle,
    Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::{CURRENT_TIME, NONE};

pub use spooch_platform::app::Command;
use spooch_platform::app::MenuItem;

const ROW_H: i32 = 20;
const SEP_H: i32 = 7;
const CHAR_W: i32 = 6;
const PAD: i32 = 10;
const BG: u32 = 0x2b2b2b;
const HOVER: u32 = 0x3078c0;
const TEXT: u32 = 0xffffff;
const DIM: u32 = 0x8a8a8a;

pub struct Menu {
    pub window: Window,
    items: Vec<MenuItem>,
    /// Each item's top and height.
    rows: Vec<(i32, i32)>,
    width: i32,
    height: i32,
    hover: Option<usize>,
    /// Server time of the click that opened the menu. Presses within
    /// `SETTLE_MS` of it are ignored, so a double-click on the tray icon,
    /// or a twitch, can't pick an item by accident.
    opened: u32,
}

const SETTLE_MS: u32 = 400;

impl Menu {
    /// Open the menu at `at`, kept inside `area` (the monitor that was
    /// clicked, as x, y, width, height), and take the pointer and keyboard
    /// so a click elsewhere closes it.
    pub fn open(
        conn: &RustConnection,
        root: Window,
        area: (i32, i32, i32, i32),
        items: Vec<MenuItem>,
        at: (i32, i32),
        opened: u32,
    ) -> Result<Menu, String> {
        let mut rows = Vec::with_capacity(items.len());
        let mut y = 4;
        let mut longest = 0;
        for item in &items {
            let h = match item {
                MenuItem::Entry { label, key, .. } => {
                    longest = longest.max(text(label, key.as_deref()).len() as i32 + 2);
                    ROW_H
                }
                MenuItem::Separator => SEP_H,
            };
            rows.push((y, h));
            y += h;
        }
        let (width, height) = (longest * CHAR_W + 2 * PAD, y + 4);
        // Open towards the middle of the clicked monitor, so a menu from a
        // tray at its top opens downward and one at its bottom upward. With
        // monitors stacked, the middle of the whole desktop can be on a
        // different monitor.
        let (ax, ay, aw, ah) = area;
        let x = (at.0 - width / 2).clamp(ax, (ax + aw - width).max(ax));
        let y = if at.1 > ay + ah / 2 {
            at.1 - height
        } else {
            at.1
        }
        .clamp(ay, (ay + ah - height).max(ay));

        let window = conn.generate_id().map_err(|e| e.to_string())?;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            root,
            x as i16,
            y as i16,
            width as u16,
            height as u16,
            1,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new()
                .override_redirect(1)
                .background_pixel(BG)
                .border_pixel(0x555555)
                .event_mask(
                    EventMask::EXPOSURE
                        | EventMask::BUTTON_PRESS
                        | EventMask::POINTER_MOTION
                        | EventMask::LEAVE_WINDOW
                        | EventMask::KEY_PRESS,
                ),
        )
        .map_err(|e| e.to_string())?;
        conn.map_window(window).map_err(|e| e.to_string())?;
        // Retry briefly: the click that opened us may still hold a grab.
        for _ in 0..50 {
            let ptr = conn
                .grab_pointer(
                    true,
                    window,
                    EventMask::BUTTON_PRESS | EventMask::POINTER_MOTION,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                    NONE,
                    NONE,
                    CURRENT_TIME,
                )
                .map_err(|e| e.to_string())?
                .reply()
                .map_err(|e| e.to_string())?;
            if ptr.status == x11rb::protocol::xproto::GrabStatus::SUCCESS {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let _ = conn.grab_keyboard(true, window, CURRENT_TIME, GrabMode::ASYNC, GrabMode::ASYNC);
        Ok(Menu {
            window,
            items,
            rows,
            width,
            height,
            hover: None,
            opened,
        })
    }

    pub fn close(&self, conn: &RustConnection) {
        let _ = conn.ungrab_pointer(CURRENT_TIME);
        let _ = conn.ungrab_keyboard(CURRENT_TIME);
        let _ = conn.destroy_window(self.window);
        let _ = conn.flush();
    }

    /// The enabled item at `y` inside the window.
    fn item_at(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || x >= self.width {
            return None;
        }
        self.rows
            .iter()
            .position(|&(top, h)| y >= top && y < top + h)
            .filter(|&i| {
                matches!(
                    self.items[i],
                    MenuItem::Entry {
                        command: Some(_),
                        ..
                    }
                )
            })
    }

    /// The pointer moved; returns whether the highlight changed.
    pub fn motion(&mut self, x: i32, y: i32) -> bool {
        let hover = self.item_at(x, y);
        let changed = hover != self.hover;
        self.hover = hover;
        changed
    }

    /// A click at `(x, y)` in the window: `Some(Some(cmd))` picks an item,
    /// `Some(None)` closes the menu, `None` does nothing (a click on a
    /// separator or a greyed-out line).
    pub fn press(&self, x: i32, y: i32, time: u32) -> Option<Option<Command>> {
        if time.wrapping_sub(self.opened) < SETTLE_MS {
            return None;
        }
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return Some(None);
        }
        let i = self.item_at(x, y)?;
        match &self.items[i] {
            MenuItem::Entry {
                command: Some(cmd), ..
            } => Some(Some(cmd.clone())),
            _ => None,
        }
    }

    pub fn draw(&self, conn: &RustConnection, gc: Gcontext) {
        let fill = |color: u32, r: Rectangle| {
            let _ = conn.change_gc(gc, &ChangeGCAux::new().foreground(color));
            let _ = conn.poly_fill_rectangle(self.window, gc, &[r]);
        };
        fill(
            BG,
            Rectangle {
                x: 0,
                y: 0,
                width: self.width as u16,
                height: self.height as u16,
            },
        );
        for (i, (item, &(top, h))) in self.items.iter().zip(&self.rows).enumerate() {
            match item {
                MenuItem::Separator => fill(
                    0x444444,
                    Rectangle {
                        x: PAD as i16,
                        y: (top + h / 2) as i16,
                        width: (self.width - 2 * PAD) as u16,
                        height: 1,
                    },
                ),
                MenuItem::Entry {
                    label,
                    key,
                    command,
                } => {
                    let bg = if self.hover == Some(i) { HOVER } else { BG };
                    fill(
                        bg,
                        Rectangle {
                            x: 2,
                            y: top as i16,
                            width: (self.width - 4) as u16,
                            height: h as u16,
                        },
                    );
                    let fg = if command.is_some() { TEXT } else { DIM };
                    let _ = conn.change_gc(gc, &ChangeGCAux::new().foreground(fg).background(bg));
                    let line = text(label, key.as_deref());
                    let _ = conn.image_text8(
                        self.window,
                        gc,
                        PAD as i16,
                        (top + 14) as i16,
                        &line.as_bytes()[..line.len().min(255)],
                    );
                }
            }
        }
        let _ = conn.flush();
    }
}

/// An entry's line: the label, padded, then its shortcut if it has one.
fn text(label: &str, key: Option<&str>) -> String {
    match key {
        Some(k) => format!("{label:<24}{k}"),
        None => label.to_string(),
    }
}
