//! The X11 editor window: shows `Document::render_window` and feeds input
//! to the session. No decorations (`_MOTIF_WM_HINTS`); caption and border
//! drags go to the window manager (`_NET_WM_MOVERESIZE`).

use spooch_app::{Effect as EditEffect, Response, Session as EditSession};
use spooch_render::skia_safe::Point as DocPoint;
use spooch_render::{Mods, WindowButton, WindowPart};
use x11rb::CURRENT_TIME;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConnectionExt as _,
    CreateWindowAux, EventMask, KeyButMask, PropMode, Timestamp, Window, WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;

use super::{App, keys, log, save, system, xdnd};

/// The window's background while it waits for its first paint.
const BACKDROP: u32 = 0x141518;

/// The open editor window.
pub(super) struct EditorWindow {
    window: Window,
    session: EditSession,
    size: (u16, u16),
    /// The pointer shape shown.
    cursor: u32,
    /// Whether it needs painting.
    dirty: bool,
    /// A drag-out in progress, with the pointer grabbed on the root.
    drag: Option<xdnd::Drag>,
    protocols: Atom,
    delete: Atom,
    frame: FrameAtoms,
    /// When the caption was last pressed, for a double-click to maximise.
    caption_click: Timestamp,
}

/// The atoms an undecorated window needs from the window manager.
struct FrameAtoms {
    moveresize: Atom,
    state: Atom,
    max_vert: Atom,
    max_horz: Atom,
    change_state: Atom,
}

/// `_NET_WM_MOVERESIZE` directions: the eight borders clockwise from the
/// top left, then moving.
const MOVE: u32 = 8;
/// A double-click on the caption, in milliseconds.
const DOUBLE_CLICK_MS: u32 = 400;
/// The wake message's first word when a session has news: thumbnails for
/// the gallery, or a share's progress (`countdown::TICK` is 1, the
/// welcome's answers 0).
pub(super) const NEWS: u32 = 2;

impl App {
    /// Show `session` in the editor window, making the window if it isn't
    /// open. A capture already in it is kept in Recents first.
    pub(super) fn open_editor(&mut self, mut session: EditSession) {
        session.set_waker(super::news_waker(self.clipboard.owner, self.wake));
        if let Some(ew) = &mut self.editor_window {
            ew.session.replace(session, |line| log::line(&line));
            ew.dirty = true;
            let title = ew.session.stem().to_string();
            let window = ew.window;
            self.set_title(window, &title);
            return;
        }
        match self.make_editor(session) {
            Ok(ew) => self.editor_window = Some(ew),
            Err(e) => log::line(format!("editor window: {e}")),
        }
    }

    /// Open the kept capture `id`, or the newest, in the editor window.
    pub(super) fn open_editor_recent(&mut self, id: Option<String>) {
        match self.carry.open_editor(id) {
            Ok((session, line)) => {
                log::line(&line);
                self.open_editor(session);
            }
            Err(e) => log::line(&e),
        }
    }

    fn make_editor(&self, mut session: EditSession) -> Result<EditorWindow, String> {
        let atom = |name: &[u8]| super::atom(&self.conn, name);
        let protocols = atom(b"WM_PROTOCOLS")?;
        let delete = atom(b"WM_DELETE_WINDOW")?;
        let frame = FrameAtoms {
            moveresize: atom(b"_NET_WM_MOVERESIZE")?,
            state: atom(b"_NET_WM_STATE")?,
            max_vert: atom(b"_NET_WM_STATE_MAXIMIZED_VERT")?,
            max_horz: atom(b"_NET_WM_STATE_MAXIMIZED_HORZ")?,
            change_state: atom(b"WM_CHANGE_STATE")?,
        };
        let motif = atom(b"_MOTIF_WM_HINTS")?;
        // Three quarters of the monitor the pointer is on, centred.
        let r = spooch_app::editor_rect(self.monitor_under(self.pointer()));
        let (x, y, w, h) = (r.left, r.top, r.width(), r.height());

        let window = self.conn.generate_id().map_err(|e| e.to_string())?;
        self.conn
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                window,
                self.root,
                x as i16,
                y as i16,
                w as u16,
                h as u16,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::new()
                    .background_pixel(BACKDROP)
                    .event_mask(
                        EventMask::EXPOSURE
                            | EventMask::STRUCTURE_NOTIFY
                            | EventMask::KEY_PRESS
                            | EventMask::KEY_RELEASE
                            | EventMask::BUTTON_PRESS
                            | EventMask::BUTTON_RELEASE
                            | EventMask::POINTER_MOTION,
                    ),
            )
            .map_err(|e| e.to_string())?;
        let _ = self.conn.change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"spooch\0Spooch\0",
        );
        let _ = self.conn.change_property32(
            PropMode::REPLACE,
            window,
            protocols,
            AtomEnum::ATOM,
            &[delete],
        );
        // No window manager decorations: the top row is the title bar.
        // Flags say only the decorations field counts, and it's none.
        let _ =
            self.conn
                .change_property32(PropMode::REPLACE, window, motif, motif, &[2, 0, 0, 0, 0]);
        // So tools (xdotool, the window manager) can tell it's Spooch's.
        let _ = self.conn.change_property32(
            PropMode::REPLACE,
            window,
            atom(b"_NET_WM_PID")?,
            AtomEnum::CARDINAL,
            &[std::process::id()],
        );
        self.set_title(window, session.stem());
        self.conn.map_window(window).map_err(|e| e.to_string())?;
        session.resize(w, h);
        Ok(EditorWindow {
            window,
            session,
            size: (w as u16, h as u16),
            cursor: self.cursors.crosshair,
            dirty: true,
            drag: None,
            protocols,
            delete,
            frame,
            caption_click: 0,
        })
    }

    fn set_title(&self, window: Window, title: &str) {
        let _ = self.conn.change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            title.as_bytes(),
        );
        if let Ok(Ok(utf8)) = self
            .conn
            .intern_atom(false, b"UTF8_STRING")
            .map(|c| c.reply())
            && let Ok(Ok(name)) = self
                .conn
                .intern_atom(false, b"_NET_WM_NAME")
                .map(|c| c.reply())
        {
            let _ = self.conn.change_property8(
                PropMode::REPLACE,
                window,
                name.atom,
                utf8.atom,
                title.as_bytes(),
            );
        }
    }

    /// Events for the editor window. Returns whether it was one.
    pub(super) fn editor_event(&mut self, event: &Event) -> bool {
        let Some(ew) = &mut self.editor_window else {
            return false;
        };
        let w = ew.window;
        // A drag-out has the pointer grabbed on the root.
        if ew.drag.is_some() {
            match event {
                Event::MotionNotify(e) => {
                    if let (Some(drag), Some(offer)) = (ew.drag.as_mut(), &self.offer) {
                        drag.motion(
                            &self.conn,
                            &self.xdnd,
                            offer,
                            self.root,
                            (e.root_x, e.root_y),
                            e.time,
                        );
                    }
                    return true;
                }
                Event::ButtonRelease(e) => {
                    if let Some(mut drag) = ew.drag.take() {
                        match drag.release(&self.conn, &self.xdnd, e.time) {
                            xdnd::Release::Dropped => log::line("dragged out of the editor"),
                            xdnd::Release::Cancelled => {
                                drag.cancel(&self.conn, &self.xdnd);
                                log::line("drag-out cancelled");
                            }
                        }
                    }
                    let _ = self.conn.ungrab_pointer(e.time);
                    return true;
                }
                Event::ClientMessage(e) if e.type_ == self.xdnd.status => {
                    if let Some(drag) = ew.drag.as_mut() {
                        drag.status(&self.conn, &self.xdnd, e);
                    }
                    return true;
                }
                _ => {}
            }
        }
        let point = |x: i16, y: i16| DocPoint::new(f32::from(x), f32::from(y));
        match event {
            Event::Expose(e) if e.window == w => ew.dirty = true,
            Event::ConfigureNotify(e) if e.window == w => {
                if (e.width, e.height) != ew.size {
                    ew.size = (e.width, e.height);
                    ew.session.resize(i32::from(e.width), i32::from(e.height));
                    ew.dirty = true;
                }
                // The window manager may have maximised or restored it.
                let state = self
                    .conn
                    .get_property(false, w, ew.frame.state, AtomEnum::ATOM, 0, 32)
                    .ok()
                    .and_then(|c| c.reply().ok());
                let maximized = state.is_some_and(|r| {
                    let atoms: Vec<u32> = r.value32().into_iter().flatten().collect();
                    atoms.contains(&ew.frame.max_vert) && atoms.contains(&ew.frame.max_horz)
                });
                ew.session.doc.set_maximized(maximized);
                ew.dirty = true;
            }
            Event::ClientMessage(e)
                if e.window == w
                    && e.type_ == ew.protocols
                    && e.data.as_data32()[0] == ew.delete =>
            {
                self.close_editor();
            }
            Event::ButtonPress(e) if e.event == w => {
                let ctrl = e.state.contains(KeyButMask::CONTROL);
                let shift = e.state.contains(KeyButMask::SHIFT);
                let p = point(e.event_x, e.event_y);
                if e.detail == 1 {
                    match ew.session.doc.window_part(p) {
                        WindowPart::Caption => {
                            let double = e.time.wrapping_sub(ew.caption_click) < DOUBLE_CLICK_MS;
                            ew.caption_click = e.time;
                            if double {
                                self.toggle_maximized();
                            } else {
                                self.move_resize(MOVE, (e.root_x, e.root_y), e.time);
                            }
                            return true;
                        }
                        WindowPart::Edge { x, y } => {
                            let direction = match (x, y) {
                                (0, 0) => 0,
                                (1, 0) => 1,
                                (2, 0) => 2,
                                (2, 1) => 3,
                                (2, 2) => 4,
                                (1, 2) => 5,
                                (0, 2) => 6,
                                _ => 7,
                            };
                            self.move_resize(direction, (e.root_x, e.root_y), e.time);
                            return true;
                        }
                        WindowPart::Button(_) | WindowPart::Client => {}
                    }
                }
                let r = match e.detail {
                    1 => ew.session.press(
                        p,
                        Mods {
                            ctrl,
                            shift,
                            alt: false,
                        },
                    ),
                    b => match keys::wheel(b, e.state) {
                        Some((notches, sideways)) => ew.session.wheel(p, notches, sideways, ctrl),
                        None => Response::default(),
                    },
                };
                self.editor_carry_out(r, e.time);
            }
            Event::MotionNotify(e) if e.event == w => {
                let p = point(e.event_x, e.event_y);
                let slop = spooch_app::DRAG_SLOP;
                let mods = keys::mods(e.state);
                let r = ew.session.motion(p, (slop, slop), mods);
                let over = ew.session.doc.window_part(p).cursor();
                let shape = self
                    .cursors
                    .get(over.unwrap_or_else(|| ew.session.cursor(p)));
                if shape != ew.cursor {
                    ew.cursor = shape;
                    let _ = self.conn.change_window_attributes(
                        w,
                        &ChangeWindowAttributesAux::new().cursor(shape),
                    );
                }
                self.editor_carry_out(r, e.time);
            }
            Event::ButtonRelease(e) if e.event == w && e.detail == 1 => {
                let r = ew.session.release(point(e.event_x, e.event_y));
                self.editor_carry_out(r, e.time);
            }
            Event::KeyPress(e) if e.event == w => {
                let sym = self.keys.sym(e.detail);
                if matches!(sym, keys::XK_ALT_L | keys::XK_ALT_R) {
                    ew.dirty |= ew.session.doc.set_preview(true);
                    return true;
                }
                let ctrl = e.state.contains(KeyButMask::CONTROL);
                let shift = e.state.contains(KeyButMask::SHIFT);
                let alt = e.state.contains(KeyButMask::MOD1);
                let mut r = match super::editor_key(&self.keys, sym, e.detail) {
                    Some(k) => ew.session.key(k, Mods { ctrl, shift, alt }),
                    None => Response::default(),
                };
                // Text a key types that the editor didn't use goes into a
                // label, or starts one under the pointer.
                if !r.used
                    && !ctrl
                    && !alt
                    && let Some(c) = self.keys.char(e.detail, shift)
                {
                    let p = point(e.event_x, e.event_y);
                    r.repaint |= ew.session.type_char(c, p).repaint;
                }
                self.editor_carry_out(r, e.time);
            }
            Event::KeyRelease(e) if e.event == w => {
                let sym = self.keys.sym(e.detail);
                if matches!(sym, keys::XK_ALT_L | keys::XK_ALT_R) {
                    ew.dirty |= ew.session.doc.set_preview(false);
                }
                if let Some(k) = super::editor_key(&self.keys, sym, e.detail) {
                    ew.session.key_up(k);
                }
            }
            _ => return false,
        }
        true
    }

    /// A session's background thread has news.
    pub(super) fn news_ready(&mut self) {
        if let Some(ew) = &mut self.editor_window {
            let r = ew.session.poll();
            self.editor_carry_out(r, CURRENT_TIME);
        }
        self.poll_overlay();
    }

    /// Carry out what the session asked for, and repaint if it changed.
    fn editor_carry_out(&mut self, r: Response, time: Timestamp) {
        if r.repaint
            && let Some(ew) = &mut self.editor_window
        {
            ew.dirty = true;
        }
        for effect in r.effects {
            match effect {
                // X holds the pointer for the window while a button is down.
                EditEffect::CaptureMouse => {}
                EditEffect::Window(WindowButton::Minimize) => self.minimize_editor(),
                EditEffect::Window(WindowButton::Maximize) => self.toggle_maximized(),
                EditEffect::Window(WindowButton::Close) => self.close_editor(),
                EditEffect::StartDragOut => self.editor_drag_out(time),
                EditEffect::Deliver { save, .. } => self.editor_deliver(save, time),
                EditEffect::CopyText(text) => self.copy_text(&text, time),
                // The window's capture holds all the pixels it can grow
                // over, and it neither cancels nor moves on.
                EditEffect::NeedDesktop(_)
                | EditEffect::Cancel
                | EditEffect::OpenWindow
                | EditEffect::Shared => {}
                EditEffect::Reveal(path) => system::reveal(&path),
                EditEffect::Log(line) => log::line(&line),
            }
        }
    }

    /// Copy the image, and save it when `save`.
    fn editor_deliver(&mut self, save: bool, time: Timestamp) {
        let Some(ew) = &mut self.editor_window else {
            return;
        };
        match ew.session.deliverable(save) {
            Ok(d) => self.deliver(d, time, &mut |line| log::line(&line)),
            Err(e) => log::line(format!("copy failed: {e}")),
        }
    }

    /// Drag the finished image out as a file. The window stays open.
    fn editor_drag_out(&mut self, time: Timestamp) {
        let Some(ew) = &mut self.editor_window else {
            return;
        };
        let mime = ew.session.doc.format().mime();
        let file = ew.session.drag_file(&save::drag_folder());
        let drag = match file {
            Ok((path, bytes)) => self.offer_drag(path, bytes, mime, time),
            Err(e) => {
                log::line(format!("drag-out failed: {e}"));
                None
            }
        };
        if let Some(ew) = &mut self.editor_window {
            ew.drag = drag;
        }
    }

    /// Hand a move (`MOVE`) or a resize from a border to the window
    /// manager, from the press at root point `at`.
    fn move_resize(&self, direction: u32, at: (i16, i16), time: Timestamp) {
        let Some(ew) = &self.editor_window else {
            return;
        };
        // The window manager takes the pointer from here.
        let _ = self.conn.ungrab_pointer(time);
        let data = [
            at.0 as u32,
            at.1 as u32,
            direction,
            1, // the left button
            1, // from an application
        ];
        self.send_to_root(ew.window, ew.frame.moveresize, data);
    }

    /// Maximise the editor window, or restore it.
    fn toggle_maximized(&self) {
        let Some(ew) = &self.editor_window else {
            return;
        };
        // 2 toggles both maximised states.
        let data = [2, ew.frame.max_vert, ew.frame.max_horz, 1, 0];
        self.send_to_root(ew.window, ew.frame.state, data);
    }

    /// Minimise (iconify) the editor window.
    fn minimize_editor(&self) {
        let Some(ew) = &self.editor_window else {
            return;
        };
        // 3 is IconicState.
        self.send_to_root(ew.window, ew.frame.change_state, [3, 0, 0, 0, 0]);
    }

    /// A request about `window` for the window manager.
    fn send_to_root(&self, window: Window, kind: Atom, data: [u32; 5]) {
        let event = ClientMessageEvent::new(32, window, kind, data);
        let _ = self.conn.send_event(
            false,
            self.root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            event,
        );
        let _ = self.conn.flush();
    }

    /// Paint the editor window if it changed.
    pub(super) fn editor_paint(&mut self) {
        let Some(ew) = &mut self.editor_window else {
            return;
        };
        if !ew.dirty {
            return;
        }
        ew.dirty = false;
        let (w, h) = ew.size;
        let (pixels, stride) = ew.session.doc.render_window();
        let row = usize::from(w) * 4;
        let _ = self.transfer.put(
            &self.conn,
            ew.window,
            self.gc,
            self.depth,
            (0, 0),
            (w, h),
            |buf| {
                for (y, out) in buf.chunks_exact_mut(row).enumerate() {
                    if let Some(src) = pixels.get(y * stride..y * stride + row) {
                        out.copy_from_slice(src);
                    }
                }
            },
        );
    }

    /// Close the window, keeping its capture in Recents.
    fn close_editor(&mut self) {
        let Some(mut ew) = self.editor_window.take() else {
            return;
        };
        log::line(ew.session.closing_line());
        self.carry.retire(&mut ew.session, |line| {
            log::line(&line);
            unsafe {
                libc::malloc_trim(0);
            }
        });
        if ew.drag.is_some() {
            let _ = self.conn.ungrab_pointer(CURRENT_TIME);
        }
        let _ = self.conn.destroy_window(ew.window);
        let _ = self.conn.flush();
    }
}
