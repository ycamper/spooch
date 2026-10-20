//! The tray icon, using the freedesktop system tray protocol (XEmbed): a
//! small window we ask the panel to swallow. GNOME shows these through its
//! AppIndicator extension; KDE, Xfce and most other panels show them
//! natively. Spooch works without a tray too; the shortcuts don't need it.
//!
//! When the tray offers a 32-bit visual the icon is drawn with real
//! transparency; otherwise it is blended over a dark panel colour.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ColormapAlloc, ConnectionExt as _, CreateGCAux,
    CreateWindowAux, EventMask, Gcontext, ImageFormat, PropMode, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{CURRENT_TIME, NONE};

/// SYSTEM_TRAY_REQUEST_DOCK.
const REQUEST_DOCK: u32 = 0;
/// The colour the icon is blended over when the tray has no alpha.
const PANEL: [u8; 3] = [0x30, 0x30, 0x30];

pub struct Tray {
    pub window: Window,
    gc: Gcontext,
    depth: u8,
    size: (u16, u16),
    selection: Atom,
    opcode: Atom,
    pub manager_atom: Atom,
    docked: bool,
}

fn atom(conn: &RustConnection, name: &str) -> Result<Atom, String> {
    super::atom(conn, name.as_bytes())
}

impl Tray {
    pub fn new(conn: &RustConnection, screen_num: usize) -> Result<Tray, String> {
        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;
        let selection = atom(conn, &format!("_NET_SYSTEM_TRAY_S{screen_num}"))?;
        let opcode = atom(conn, "_NET_SYSTEM_TRAY_OPCODE")?;
        let manager_atom = atom(conn, "MANAGER")?;
        let visual_atom = atom(conn, "_NET_SYSTEM_TRAY_VISUAL")?;
        let xembed_info = atom(conn, "_XEMBED_INFO")?;

        // Use the tray's own visual when it offers a 32-bit one.
        let manager = owner(conn, selection);
        let offered = (manager != NONE)
            .then(|| {
                conn.get_property(false, manager, visual_atom, AtomEnum::VISUALID, 0, 1)
                    .ok()?
                    .reply()
                    .ok()?
                    .value32()?
                    .next()
            })
            .flatten();
        let argb = offered.and_then(|v| {
            screen
                .allowed_depths
                .iter()
                .find(|d| d.depth == 32 && d.visuals.iter().any(|vt| vt.visual_id == v))
                .map(|_| v)
        });
        let (depth, visual) = match argb {
            Some(v) => (32, v),
            None => (screen.root_depth, screen.root_visual),
        };

        let id = || conn.generate_id().map_err(|e| e.to_string());
        let window = id()?;
        let mut aux = CreateWindowAux::new()
            .border_pixel(0)
            .background_pixel(0)
            .event_mask(
                EventMask::EXPOSURE | EventMask::STRUCTURE_NOTIFY | EventMask::BUTTON_PRESS,
            );
        if depth == 32 {
            let colormap = id()?;
            conn.create_colormap(ColormapAlloc::NONE, colormap, root, visual)
                .map_err(|e| e.to_string())?;
            aux = aux.colormap(colormap);
        }
        conn.create_window(
            depth,
            window,
            root,
            0,
            0,
            22,
            22,
            0,
            WindowClass::INPUT_OUTPUT,
            visual,
            &aux,
        )
        .map_err(|e| e.to_string())?;
        // XEmbed version 0, and "map me".
        conn.change_property32(PropMode::REPLACE, window, xembed_info, xembed_info, &[0, 1])
            .map_err(|e| e.to_string())?;
        conn.change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"spooch\0Spooch\0",
        )
        .map_err(|e| e.to_string())?;
        conn.change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            b"Spooch",
        )
        .map_err(|e| e.to_string())?;
        let gc = id()?;
        conn.create_gc(gc, window, &CreateGCAux::new().graphics_exposures(0))
            .map_err(|e| e.to_string())?;
        // Hear about a tray that starts after us.
        conn.change_window_attributes(
            root,
            &x11rb::protocol::xproto::ChangeWindowAttributesAux::new()
                .event_mask(EventMask::STRUCTURE_NOTIFY),
        )
        .map_err(|e| e.to_string())?;

        let mut tray = Tray {
            window,
            gc,
            depth,
            size: (22, 22),
            selection,
            opcode,
            manager_atom,
            docked: false,
        };
        tray.dock(conn);
        Ok(tray)
    }

    pub fn is_docked(&self) -> bool {
        self.docked
    }

    /// Ask the tray, if there is one, to take the icon.
    pub fn dock(&mut self, conn: &RustConnection) -> bool {
        let manager = owner(conn, self.selection);
        if manager == NONE {
            return false;
        }
        let event = ClientMessageEvent::new(
            32,
            manager,
            self.opcode,
            [CURRENT_TIME, REQUEST_DOCK, self.window, 0, 0],
        );
        self.docked = conn
            .send_event(false, manager, EventMask::NO_EVENT, event)
            .is_ok();
        let _ = conn.flush();
        self.docked
    }

    /// A MANAGER message on the root window: a tray started.
    pub fn manager_appeared(&mut self, conn: &RustConnection, e: &ClientMessageEvent) -> bool {
        e.data.as_data32()[1] == self.selection && self.dock(conn)
    }

    pub fn resized(&mut self, w: u16, h: u16) {
        self.size = (w.max(1), h.max(1));
    }

    /// Draw the icon, centred in whatever size the tray gave us.
    pub fn draw(&self, conn: &RustConnection) {
        let (w, h) = self.size;
        let side = w.min(h).clamp(8, 256) as u32;
        let icon = spooch_render::app_icon(side);
        let mut px = Vec::with_capacity(icon.len());
        for p in icon.as_chunks::<4>().0 {
            let a = u16::from(p[3]);
            if self.depth == 32 {
                // ARGB visuals want premultiplied alpha.
                let pm = |c: u8| (u16::from(c) * a / 255) as u8;
                px.extend_from_slice(&[pm(p[0]), pm(p[1]), pm(p[2]), p[3]]);
            } else {
                let mix =
                    |c: u8, bg: u8| ((u16::from(c) * a + u16::from(bg) * (255 - a)) / 255) as u8;
                px.extend_from_slice(&[
                    mix(p[0], PANEL[2]),
                    mix(p[1], PANEL[1]),
                    mix(p[2], PANEL[0]),
                    255,
                ]);
            }
        }
        let _ = conn.clear_area(false, self.window, 0, 0, 0, 0);
        let _ = conn.put_image(
            ImageFormat::Z_PIXMAP,
            self.window,
            self.gc,
            side as u16,
            side as u16,
            ((u32::from(w) - side) / 2) as i16,
            ((u32::from(h) - side) / 2) as i16,
            0,
            self.depth,
            &px,
        );
    }
}

fn owner(conn: &RustConnection, selection: Atom) -> Window {
    conn.get_selection_owner(selection)
        .ok()
        .and_then(|c| c.reply().ok())
        .map_or(NONE, |r| r.owner)
}
