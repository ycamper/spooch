//! The frozen-screen overlay: picking an area over the dimmed desktop,
//! then editing it in place until it's copied, saved, dragged out,
//! moved to the editor window or cancelled.

use super::*;

pub(super) fn doc_point((x, y): (i32, i32)) -> DocPoint {
    DocPoint::new(x as f32, y as f32)
}

/// One capture, from the hotkey until copy or cancel.
pub(super) struct Session {
    /// Desktop size in pixels.
    pub(super) size: (i32, i32),
    pub(super) window: Window,
    pub(super) bright: Pixmap,
    pub(super) dim: Pixmap,
    pub(super) back: Pixmap,
    /// The area being picked, and where the pointer is.
    pub(super) picker: Picker,
    /// The loupe beside the pointer while selecting, from the core.
    pub(super) loupe: Option<Panel>,
    pub(super) editor: Option<EditSession>,
    /// What is currently drawn over the dimmed desktop (selection, border
    /// and chrome), so the next paint can restore it.
    pub(super) shown: Rect,
    pub(super) damage: Rect,
    /// The pointer shape currently shown.
    pub(super) cursor: u32,
    /// A drag-out in progress, with the overlay unmapped.
    pub(super) drag: Option<xdnd::Drag>,
}

impl Session {
    pub(super) fn selection(&self) -> Option<Rect> {
        match &self.editor {
            Some(ed) => Some(region(ed)),
            None => self.picker.selection().map(Rect::from_irect),
        }
    }
}

pub(super) enum Done {
    /// Copied to the clipboard, and also saved to a file when the flag is set.
    Copied(Rect, Counts, bool),
    /// Dragged out as this file and dropped on another app.
    Dragged(Rect, Counts, PathBuf),
    /// Shared, with its link on the clipboard.
    Shared(Rect),
    /// Moved into the editor window to keep editing.
    Window,
    Cancelled,
}

impl App {
    /// Holding Alt previews the framed result; letting go ends it.
    pub(super) fn alt_preview(&mut self, on: bool) {
        let Some(s) = &mut self.session else { return };
        let Some(ed) = &mut s.editor else { return };
        let before = chrome_rect(ed);
        if ed.doc.set_preview(on) {
            s.damage = s.damage.union(&before).union(&chrome_rect(ed));
        }
    }

    pub(super) fn key_release(&mut self, e: KeyPressEvent) {
        let sym = self.keys.sym(e.detail);
        if matches!(sym, keys::XK_ALT_L | keys::XK_ALT_R) {
            self.alt_preview(false);
        }
    }

    pub(super) fn key(&mut self, e: KeyPressEvent) {
        let sym = self.keys.sym(e.detail);
        let ctrl = e.state.contains(KeyButMask::CONTROL);
        let shift = e.state.contains(KeyButMask::SHIFT);
        if self.session.is_some() && matches!(sym, keys::XK_ALT_L | keys::XK_ALT_R) {
            self.alt_preview(true);
            return;
        }
        let Some(s) = &mut self.session else {
            if let Some(action) = self.shortcut_action(&e) {
                let effects = self.flow.shortcut(action, &self.settings, false);
                self.apply(effects);
            }
            return;
        };
        // During a drag-out only Esc means anything: it calls the drag off.
        if s.drag.is_some() {
            if sym == keys::XK_ESCAPE {
                self.drag_cancel(e.time);
            }
            return;
        }
        let alt = e.state.contains(KeyButMask::MOD1);
        let key = editor_key(&self.keys, sym, e.detail);
        let Some(ed) = &mut s.editor else {
            let pick = key.and_then(|k| s.picker.key(k, shift));
            self.picked(pick, e.time);
            return;
        };
        let r = match key {
            Some(k) => ed.key(k, Mods { ctrl, shift, alt }),
            None => Response::default(),
        };
        let mut repaint = r.repaint;
        // Text a key types that the editor didn't use goes into a label,
        // or starts one.
        if !r.used
            && !ctrl
            && !alt
            && let Some(c) = self.keys.char(e.detail, shift)
        {
            repaint |= ed.type_char(c, doc_point(s.picker.pointer())).repaint;
        }
        if repaint {
            s.damage = s.damage.union(&chrome_rect(ed));
        }
        self.carry_out(r.effects, e.time);
    }

    pub(super) fn button_press(&mut self, e: ButtonPressEvent) {
        let p = (i32::from(e.event_x), i32::from(e.event_y));
        if e.detail == 3 {
            if self.session.is_some() {
                self.end(Done::Cancelled, e.time);
            }
            return;
        }
        let Some(s) = &mut self.session else { return };
        if e.detail != 1 || s.drag.is_some() {
            return;
        }
        s.picker.motion(p);
        let ctrl = e.state.contains(KeyButMask::CONTROL);
        let Some(ed) = &mut s.editor else {
            s.picker.press(p);
            return;
        };
        let mods = Mods {
            ctrl,
            shift: e.state.contains(KeyButMask::SHIFT),
            alt: false,
        };
        // The overlay's grab already keeps the pointer, so CaptureMouse
        // needs nothing more.
        let r = ed.press(doc_point(p), mods);
        s.damage = s.damage.union(&chrome_rect(ed));
        self.carry_out(r.effects, e.time);
        self.update_cursor();
    }

    pub(super) fn motion(&mut self, e: MotionNotifyEvent) {
        let Some(s) = &mut self.session else { return };
        let p = (i32::from(e.event_x), i32::from(e.event_y));
        s.picker.motion(p);
        if let Some(drag) = s.drag.as_mut() {
            if let Some(offer) = &self.offer {
                drag.motion(
                    &self.conn,
                    &self.xdnd,
                    offer,
                    self.root,
                    (e.root_x, e.root_y),
                    e.time,
                );
            }
            return;
        }
        let mut effects = Vec::new();
        match &mut s.editor {
            Some(ed) => {
                let slop = DRAG_SLOP;
                let mods = keys::mods(e.state);
                let r = ed.motion(doc_point(p), (slop, slop), mods);
                if r.repaint {
                    s.damage = s.damage.union(&chrome_rect(ed));
                }
                effects = r.effects;
            }
            None => {
                if let Some(sel) = s.picker.selection() {
                    s.damage = s.damage.union(&Rect::from_irect(sel).outset(2));
                }
            }
        }
        if s.editor.is_none() {
            if let Some(old) = s.loupe.take() {
                s.damage = s.damage.union(&Rect::from_irect(old.rect));
            }
            s.loupe = read_loupe(&self.conn, &mut self.transfer, s);
            if let Some(new) = &s.loupe {
                s.damage = s.damage.union(&Rect::from_irect(new.rect));
            }
        }
        self.carry_out(effects, e.time);
        self.update_cursor();
    }

    pub(super) fn button_release(&mut self, e: ButtonPressEvent) {
        if e.detail != 1 {
            return;
        }
        let Some(s) = &mut self.session else { return };
        if let Some(drag) = s.drag.as_mut() {
            match drag.release(&self.conn, &self.xdnd, e.time) {
                xdnd::Release::Dropped => {
                    let done = match (&s.editor, &self.offer) {
                        (Some(ed), Some(offer)) => {
                            Done::Dragged(region(ed), ed.doc.counts(), offer.path.clone())
                        }
                        _ => Done::Cancelled,
                    };
                    self.end(done, e.time);
                }
                xdnd::Release::Cancelled => self.drag_cancel(e.time),
            }
            return;
        }
        let p = (i32::from(e.event_x), i32::from(e.event_y));
        s.picker.motion(p);
        if let Some(ed) = &mut s.editor {
            if ed.release(doc_point(p)).repaint {
                s.damage = s.damage.union(&chrome_rect(ed));
            }
            self.update_cursor();
            return;
        }
        let pick = s.picker.release(p);
        self.picked(pick, e.time);
    }

    /// Carry out what picking came to.
    pub(super) fn picked(&mut self, pick: Option<Pick>, time: Timestamp) {
        match pick {
            Some(Pick::Edit(area)) => self.start_editing(area),
            Some(Pick::WarpPointer((x, y))) => {
                // The motion this makes updates the selection and loupe.
                if let Some(s) = &self.session {
                    let _ = self
                        .conn
                        .warp_pointer(NONE, s.window, 0, 0, 0, 0, x as i16, y as i16);
                    let _ = self.conn.flush();
                }
            }
            Some(Pick::Cancel) => self.end(Done::Cancelled, time),
            None => {}
        }
    }

    /// Show the pointer shape for what a press would do where it is.
    pub(super) fn update_cursor(&mut self) {
        let Some(s) = &mut self.session else { return };
        let want = match &s.editor {
            _ if s.drag.is_some() => self.cursors.hand,
            Some(ed) => self.cursors.get(ed.cursor(doc_point(s.picker.pointer()))),
            None => self.cursors.crosshair,
        };
        if want != s.cursor {
            s.cursor = want;
            let _ = self.conn.change_active_pointer_grab(
                want,
                CURRENT_TIME,
                EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
            );
        }
    }

    /// Write the finished image to a file, hide the overlay so the windows
    /// under it can take the drop, and follow the pointer over the desktop.
    pub(super) fn start_drag_out(&mut self, time: Timestamp) {
        let Some(s) = &mut self.session else { return };
        let Some(ed) = &mut s.editor else { return };
        let started = Instant::now();
        let format = ed.doc.format();
        let (path, bytes) = match ed.drag_file(&save::drag_folder()) {
            Ok(p) => p,
            Err(e) => {
                log::line(format!("drag-out failed: {e}"));
                return;
            }
        };
        let started_ms = log::ms_since(started);
        log::line(format!(
            "drag-out file ready in {started_ms:.1} ms: {}",
            path.display()
        ));
        let window = s.window;
        let Some(drag) = self.offer_drag(path, bytes, format.mime(), time) else {
            return;
        };
        // The overlay gets out of the way so the windows under it can
        // take the drop, and the keyboard follows the pointer to the root
        // so Esc still reaches us.
        let _ = self.conn.unmap_window(window);
        let _ = self
            .conn
            .grab_keyboard(false, self.root, time, GrabMode::ASYNC, GrabMode::ASYNC);
        let Some(s) = &mut self.session else { return };
        s.cursor = self.cursors.hand;
        s.drag = Some(drag);
    }

    /// Offer a dragged-out file over XDND and follow the pointer over other
    /// apps until the button comes up. The overlay and the editor window
    /// both drag out this way.
    pub(super) fn offer_drag(
        &mut self,
        path: PathBuf,
        bytes: Vec<u8>,
        mime: &str,
        time: Timestamp,
    ) -> Option<xdnd::Drag> {
        let drag = match xdnd::Drag::start(&self.conn, &self.xdnd, self.clipboard.owner, time) {
            Ok(d) => d,
            Err(e) => {
                log::line(format!("drag-out failed: {e}"));
                return None;
            }
        };
        self.offer = Some(xdnd::Offer::new(&self.conn, path, bytes, mime));
        let _ = self.conn.grab_pointer(
            false,
            self.root,
            EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
            NONE,
            self.cursors.hand,
            time,
        );
        Some(drag)
    }

    /// Put a finished image on the clipboard, and save it when it carries
    /// a file. `mark` gets each step for the log. The overlay and the
    /// editor window both deliver this way.
    pub(super) fn deliver(
        &mut self,
        d: Deliverable,
        time: Timestamp,
        mark: &mut dyn FnMut(String),
    ) {
        let (conn, clipboard) = (&self.conn, &mut self.clipboard);
        d.deliver(
            |c| clipboard.set_image(conn, c.png, time),
            || Ok(save::folder()),
            mark,
        );
    }

    /// Put a share's link on the clipboard.
    pub(super) fn copy_text(&mut self, text: &str, time: Timestamp) {
        match self.clipboard.set_text(&self.conn, text, time) {
            Ok(()) => log::line("share link on the clipboard"),
            Err(e) => log::line(format!("clipboard failed: {e}")),
        }
    }

    /// Take the overlay's share news, if it has a share under way.
    pub(super) fn poll_overlay(&mut self) {
        let Some(s) = &mut self.session else { return };
        let Some(ed) = &mut s.editor else { return };
        let before = chrome_rect(ed);
        let r = ed.poll();
        if r.repaint {
            s.damage = s.damage.union(&before).union(&chrome_rect(ed));
        }
        self.carry_out(r.effects, CURRENT_TIME);
    }

    /// The drag-out was called off: bring the editor back as it was.
    pub(super) fn drag_cancel(&mut self, time: Timestamp) {
        let Some(s) = &mut self.session else { return };
        let Some(mut drag) = s.drag.take() else {
            return;
        };
        drag.cancel(&self.conn, &self.xdnd);
        log::line("drag-out cancelled");
        let _ = self.conn.ungrab_pointer(time);
        let _ = self.conn.ungrab_keyboard(time);
        let _ = self.conn.map_window(s.window);
        let window = s.window;
        s.cursor = self.cursors.crosshair;
        if let Err(e) = self.grab_input(window) {
            log::line(format!("could not take input back after the drag: {e}"));
        }
        self.update_cursor();
    }

    /// The reopen shortcut: freeze the screen, then open the newest kept
    /// capture on it.
    pub(super) fn reopen_latest(&mut self) {
        let Some(id) = self.carry.latest() else {
            log::line("nothing in recents to reopen yet");
            return;
        };
        self.capture();
        if !self.open_recent(&id) {
            self.end(Done::Cancelled, CURRENT_TIME);
        }
    }

    /// Open the kept capture `id` in the editor, replacing any open one.
    pub(super) fn open_recent(&mut self, id: &str) -> bool {
        let Some(s) = &mut self.session else {
            return false;
        };
        let mut ed = match self.carry.reopen(id, s.picker.screens()) {
            Ok((ed, line)) => {
                log::line(&line);
                ed
            }
            Err(e) => {
                log::line(&e);
                return false;
            }
        };
        s.damage = s.damage.union(&chrome_rect(&ed));
        if let Some(l) = s.loupe.take() {
            s.damage = s.damage.union(&Rect::from_irect(l.rect));
        }
        ed.set_waker(super::news_waker(self.clipboard.owner, self.wake));
        s.editor = Some(ed);
        true
    }

    /// Carry out what the editor asked for.
    pub(super) fn carry_out(&mut self, effects: Vec<EditEffect>, time: Timestamp) {
        for effect in effects {
            match effect {
                // The overlay's grab already keeps the pointer.
                EditEffect::CaptureMouse => {}
                EditEffect::NeedDesktop(want) => self.supply_desktop(want),
                EditEffect::StartDragOut => self.start_drag_out(time),
                EditEffect::Deliver { save, .. } => {
                    let Some(ed) = self.session.as_ref().and_then(|s| s.editor.as_ref()) else {
                        return;
                    };
                    let done = Done::Copied(region(ed), ed.doc.counts(), save);
                    self.end(done, time);
                    return;
                }
                EditEffect::CopyText(text) => self.copy_text(&text, time),
                EditEffect::Shared => {
                    let Some(ed) = self.session.as_ref().and_then(|s| s.editor.as_ref()) else {
                        return;
                    };
                    let done = Done::Shared(region(ed));
                    self.end(done, time);
                    return;
                }
                EditEffect::Cancel => {
                    self.end(Done::Cancelled, time);
                    return;
                }
                EditEffect::OpenWindow => {
                    let ed = self.session.as_mut().and_then(|s| s.editor.take());
                    self.end(Done::Window, time);
                    if let Some(ed) = ed {
                        self.open_editor(ed.into_window());
                    }
                    return;
                }
                // The overlay has no window buttons or gallery.
                EditEffect::Window(_) | EditEffect::Reveal(_) => {}
                EditEffect::Log(line) => log::line(&line),
            }
        }
    }

    /// Hand the editor the frozen desktop under `want`, for an edge drag
    /// or nudge past the pixels it holds.
    pub(super) fn supply_desktop(&mut self, want: IRect) {
        let Some(s) = &mut self.session else { return };
        let Some(ed) = &mut s.editor else { return };
        let started = Instant::now();
        let (w, h) = (want.width() as u32, want.height() as u32);
        let read = self.transfer.read(
            &self.conn,
            s.bright,
            (want.left as i16, want.top as i16),
            (w as u16, h as u16),
        );
        match read {
            Ok(pixels) => {
                let ms = log::ms_since(started);
                if let Some(line) = ed.supply_desktop((w, h, w as usize * 4), &pixels, ms) {
                    log::line(&line);
                }
            }
            Err(e) => log::line(format!("could not read the desktop: {e}")),
        }
    }

    /// Hotkey to frozen overlay, timed step by step.
    pub(super) fn capture(&mut self) {
        if self.session.is_some() {
            return;
        }
        let mut sw = Stopwatch::start();
        match self.open_session(&mut sw) {
            Ok(session) => {
                self.session = Some(session);
                sw.report("capture");
            }
            Err(e) => log::line(format!("capture failed: {e}")),
        }
    }

    pub(super) fn open_session(&mut self, sw: &mut Stopwatch) -> Result<Session, String> {
        let monitors: Vec<Rect> = self.monitors().into_iter().map(|(_, r)| r).collect();
        sw.mark(format!("monitors ({})", monitors.len()));

        // The frozen desktop stays on the server: copy the root window into
        // a pixmap, then derive the dimmed backdrop from it with RENDER.
        // Nothing crosses the socket until a region is selected.
        let (w, h) = (self.width, self.height);
        let id = || self.conn.generate_id().map_err(|e| e.to_string());
        let (bright, dim, back, window) = (id()?, id()?, id()?, id()?);
        for p in [bright, dim, back] {
            self.conn
                .create_pixmap(self.depth, p, self.root, w, h)
                .map_err(|e| e.to_string())?;
        }
        self.conn
            .copy_area(self.root, bright, self.capture_gc, 0, 0, 0, 0, w, h)
            .map_err(|e| e.to_string())?;
        sync(&self.conn)?;
        sw.mark(format!("capture {w}x{h} (server copy)"));

        self.conn
            .copy_area(bright, dim, self.gc, 0, 0, 0, 0, w, h)
            .map_err(|e| e.to_string())?;
        if let Some(format) = self.dim_format {
            let pic = id()?;
            self.conn
                .render_create_picture(pic, dim, format, &CreatePictureAux::new())
                .map_err(|e| e.to_string())?;
            let shade = render::Color {
                red: 0,
                green: 0,
                blue: 0,
                // Black over the rest, to `OUTSIDE_BRIGHTNESS`.
                alpha: ((100 - u32::from(spooch_render::OUTSIDE_BRIGHTNESS)) * 0xffff / 100) as u16,
            };
            let all = Rectangle {
                x: 0,
                y: 0,
                width: w,
                height: h,
            };
            self.conn
                .render_fill_rectangles(PictOp::OVER, pic, shade, &[all])
                .map_err(|e| e.to_string())?;
            self.conn
                .render_free_picture(pic)
                .map_err(|e| e.to_string())?;
        }
        // The back pixmap needs no fill: paint() copies the dimmed desktop
        // into it for every area it redraws.
        sync(&self.conn)?;
        sw.mark("dim");

        self.conn
            .create_window(
                self.depth,
                window,
                self.root,
                0,
                0,
                w,
                h,
                0,
                WindowClass::INPUT_OUTPUT,
                self.visual,
                &CreateWindowAux::new()
                    .override_redirect(1)
                    .background_pixmap(NONE)
                    .border_pixel(0)
                    .cursor(self.cursors.crosshair)
                    .event_mask(
                        EventMask::EXPOSURE
                            | EventMask::KEY_PRESS
                            | EventMask::KEY_RELEASE
                            | EventMask::BUTTON_PRESS
                            | EventMask::BUTTON_RELEASE
                            | EventMask::POINTER_MOTION,
                    ),
            )
            .map_err(|e| e.to_string())?;
        self.conn.map_window(window).map_err(|e| e.to_string())?;
        // Show the frozen desktop right away rather than waiting for Expose.
        self.conn
            .copy_area(dim, window, self.gc, 0, 0, 0, 0, w, h)
            .map_err(|e| e.to_string())?;
        sync(&self.conn)?;
        sw.mark("map + first paint");

        self.grab_input(window)?;
        sw.mark("grab input");

        let pointer = self.pointer();
        // X11 has no per-monitor scale. Without RandR, the root is one
        // monitor.
        let mut screens: Vec<(IRect, f32)> = monitors.iter().map(|m| (m.irect(), 1.0)).collect();
        if screens.is_empty() {
            screens.push((IRect::from_wh(w.into(), h.into()), 1.0));
        }
        let picker = Picker::new(pointer, screens);
        Ok(Session {
            size: (w.into(), h.into()),
            window,
            bright,
            dim,
            back,
            picker,
            loupe: None,
            editor: None,
            shown: Rect {
                x: 0,
                y: 0,
                w: 0,
                h: 0,
            },
            damage: Rect {
                x: 0,
                y: 0,
                w: 0,
                h: 0,
            },
            cursor: self.cursors.crosshair,
            drag: None,
        })
    }

    /// Take the keyboard and pointer so every key and click comes to us.
    /// Another app's grab can linger for a moment after the hotkey, so retry
    /// briefly.
    pub(super) fn grab_input(&self, window: Window) -> Result<(), String> {
        // Everything goes to the overlay, even with the editor window
        // focused: owner_events would hand that window the keys.
        for attempt in 0..50 {
            let kb = self
                .conn
                .grab_keyboard(
                    false,
                    window,
                    CURRENT_TIME,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                )
                .map_err(|e| e.to_string())?
                .reply()
                .map_err(|e| e.to_string())?
                .status;
            let ptr = self
                .conn
                .grab_pointer(
                    false,
                    window,
                    EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                    NONE,
                    self.cursors.crosshair,
                    CURRENT_TIME,
                )
                .map_err(|e| e.to_string())?
                .reply()
                .map_err(|e| e.to_string())?
                .status;
            if kb == GrabStatus::SUCCESS && ptr == GrabStatus::SUCCESS {
                if attempt > 0 {
                    log::line(format!("input grabbed after {attempt} retries"));
                }
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        Err("could not grab keyboard and pointer".into())
    }

    /// Turn the picked `area` into the editor: its frozen pixels become a
    /// document that marks are drawn onto.
    pub(super) fn start_editing(&mut self, area: IRect) {
        let Some(s) = &mut self.session else { return };
        let region = Rect::from_irect(area);
        if region.is_empty() {
            return;
        }
        let started = Instant::now();
        let pixels = match self.transfer.read(
            &self.conn,
            s.bright,
            (region.x as i16, region.y as i16),
            (region.w as u16, region.h as u16),
        ) {
            Ok(p) => p,
            Err(e) => {
                log::line(format!("could not read the selection: {e}"));
                return;
            }
        };
        let setup = self.carry.setup(save::now());
        let (w, h) = (region.w as u32, region.h as u32);
        let Some(mut ed) = s.picker.edit(area, (w, h, w as usize * 4, &pixels), setup) else {
            log::line("could not create the editor surface");
            return;
        };
        log::line(format!(
            "editing {w}x{h} region in {:.1} ms",
            log::ms_since(started)
        ));
        if let Some(l) = s.loupe.take() {
            s.damage = s.damage.union(&Rect::from_irect(l.rect));
        }
        s.damage = s.damage.union(&chrome_rect(&ed));
        ed.set_waker(super::news_waker(self.clipboard.owner, self.wake));
        s.editor = Some(ed);
    }

    pub(super) fn end(&mut self, done: Done, time: Timestamp) {
        let Some(mut s) = self.session.take() else {
            return;
        };
        let mut sw = Stopwatch::start();
        let _ = self.conn.ungrab_keyboard(CURRENT_TIME);
        let _ = self.conn.ungrab_pointer(CURRENT_TIME);
        let _ = self.conn.destroy_window(s.window);
        for p in [s.bright, s.dim, s.back] {
            let _ = self.conn.free_pixmap(p);
        }
        let _ = self.conn.flush();
        sw.mark("close overlay");
        if let Some(mut drag) = s.drag.take()
            && !matches!(done, Done::Dragged(..))
        {
            drag.cancel(&self.conn, &self.xdnd);
        }
        // Every capture that reached the editor is kept, however it closed,
        // so Esc never loses work.
        if let Some(ed) = s.editor.as_mut() {
            self.carry.retire(ed, |line| {
                log::line(&line);
                // The pixels were freed only now, after the editor's own
                // trim, so give the memory back again.
                unsafe {
                    libc::malloc_trim(0);
                }
            });
        }
        let copied = match done {
            Done::Copied(r, ..) | Done::Dragged(r, ..) | Done::Shared(r) => Some(r),
            Done::Cancelled | Done::Window => None,
        };
        let to_window = matches!(done, Done::Window);
        match done {
            Done::Cancelled => log::line("cancelled"),
            Done::Window => log::line("to the editor window"),
            Done::Shared(_) => log::line("shared"),
            Done::Dragged(r, counts, path) => {
                sw.report(&spooch_app::dragged_line(r.irect(), counts, &path))
            }
            Done::Copied(r, counts, save) => {
                let Some(ed) = s.editor.as_mut() else { return };
                // The finished image, with its frame, at its own size.
                let pixels = ed.export();
                sw.mark("render");
                match Deliverable::new(pixels, ed.doc.format(), ed.stem(), save, counts) {
                    Ok(d) => {
                        if let Some(path) = &self.save
                            && let Err(e) = std::fs::write(path, &d.png)
                        {
                            log::line(format!("save {path} failed: {e}"));
                        }
                        self.deliver(d, time, &mut |line| sw.mark(line));
                        sw.report(&format!("copied region at ({}, {})", r.x, r.y));
                    }
                    Err(e) => log::line(format!("encoding failed: {e}")),
                }
            }
        }
        drop(s);
        self.transfer.release(&self.conn);
        // Hand the editor's freed buffers back to the system so an idle
        // Spooch stays small.
        unsafe {
            libc::malloc_trim(0);
        }
        log::line(format!("idle: {}", memory()));
        // In --once mode a copy keeps being served until another app takes
        // the clipboard, unless it was saved to a file.
        if self.once && !to_window && (copied.is_none() || self.save.is_some()) {
            self.quit = true;
        }
    }

    /// Compose the damaged area into the back pixmap, then show it.
    pub(super) fn paint(&mut self) {
        let Some(s) = &mut self.session else { return };
        let selection = s.selection();
        let chrome = s.editor.as_ref().map(chrome_rect);
        let now = selection.map(|sel| covered(sel, chrome));
        // Whenever anything changed, repaint where the selection's chrome
        // was as well as where it is: the chrome can keep the bounding box
        // the same while the selection shrinks inside it.
        let mut damage = s.damage;
        match now {
            Some(now) if now != s.shown || !damage.is_empty() => {
                damage = damage.union(&now).union(&s.shown);
            }
            Some(_) => {}
            None if !s.shown.is_empty() => damage = damage.union(&s.shown),
            None => {}
        }
        // The loupe is blended, so it's always redrawn whole from under it.
        if !damage.is_empty()
            && let Some(l) = &s.loupe
        {
            damage = damage.union(&Rect::from_irect(l.rect));
        }
        let full = Rect {
            x: 0,
            y: 0,
            w: s.size.0,
            h: s.size.1,
        };
        let damage = damage.intersect(&full);
        s.damage = Rect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        };
        if damage.is_empty() {
            return;
        }
        let conn = &self.conn;
        let d = damage.x11();
        let _ = conn.copy_area(
            s.dim, s.back, self.gc, d.x, d.y, d.x, d.y, d.width, d.height,
        );
        if let Some(sel) = selection {
            let inside = sel.intersect(&damage);
            match &mut s.editor {
                Some(ed) => {
                    let r = region(ed);
                    let (pixels, stride) = ed.doc.render();
                    let _ = self.transfer.put(
                        conn,
                        s.back,
                        self.gc,
                        self.depth,
                        (r.x as i16, r.y as i16),
                        (r.w as u16, r.h as u16),
                        |buf| {
                            let row = r.w as usize * 4;
                            for (y, out) in buf.chunks_exact_mut(row).enumerate() {
                                out.copy_from_slice(&pixels[y * stride..y * stride + row]);
                            }
                        },
                    );
                }
                None if !inside.is_empty() => {
                    let i = inside.x11();
                    let _ = conn.copy_area(
                        s.bright, s.back, self.gc, i.x, i.y, i.x, i.y, i.width, i.height,
                    );
                }
                None => {}
            }
            // The outline, and once editing the crop handles, from the core.
            for (r, rgb) in spooch_render::selection_frame(sel.irect(), s.editor.is_some()) {
                let _ = conn.change_gc(self.bar_gc, &ChangeGCAux::new().foreground(rgb));
                let _ = conn.poly_fill_rectangle(s.back, self.bar_gc, &[Rect::from_irect(r).x11()]);
            }
            // The core's chrome (rail, style row, output bar) on top.
            if let (Some(ed), Some(argb), Some(dst)) =
                (s.editor.as_mut(), self.argb_format, self.dim_format)
            {
                composite_panels(conn, self.root, s.back, dst, argb, ed.doc.chrome());
            }
        }
        if let (Some(l), Some(argb), Some(dst)) = (&s.loupe, self.argb_format, self.dim_format) {
            composite_panels(conn, self.root, s.back, dst, argb, [l]);
        }
        let _ = conn.copy_area(
            s.back, s.window, self.gc, d.x, d.y, d.x, d.y, d.width, d.height,
        );
        s.shown = now.unwrap_or(Rect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        });
    }
}

/// The loupe for where the pointer is now, drawn by the core from the
/// frozen pixels round it, read back from the server.
pub(super) fn read_loupe(
    conn: &RustConnection,
    transfer: &mut Transfer,
    s: &Session,
) -> Option<Panel> {
    s.picker.loupe(|r| {
        transfer
            .read(
                conn,
                s.bright,
                (r.left as i16, r.top as i16),
                (r.width() as u16, r.height() as u16),
            )
            .ok()
    })
}

/// Everything a selection draws: the region, its border and, once
/// editing, the chrome.
pub(super) fn covered(sel: Rect, chrome: Option<Rect>) -> Rect {
    match chrome {
        Some(chrome) => sel.outset(2).union(&chrome),
        None => sel.outset(2),
    }
}

/// Blend the core's chrome panels (premultiplied BGRA, rounded corners
/// transparent) onto `back` with RENDER.
pub(super) fn composite_panels<'a>(
    conn: &RustConnection,
    root: Window,
    back: Pixmap,
    back_format: render::Pictformat,
    argb: render::Pictformat,
    panels: impl IntoIterator<Item = &'a spooch_render::Panel>,
) {
    let mut panels = panels
        .into_iter()
        .filter(|p| {
            let (w, h) = (p.rect.width(), p.rect.height());
            w > 0 && h > 0 && p.pixels.len() == w as usize * h as usize * 4
        })
        .peekable();
    if panels.peek().is_none() {
        return;
    }
    // The destination and GC serve every panel; each panel needs only its
    // own pixmap and picture.
    let (Ok(dst), Ok(gc)) = (conn.generate_id(), conn.generate_id()) else {
        return;
    };
    let _ = conn.render_create_picture(dst, back, back_format, &CreatePictureAux::new());
    let mut gc_made = false;
    for panel in panels {
        let (w, h) = (panel.rect.width() as u16, panel.rect.height() as u16);
        let (Ok(pixmap), Ok(src)) = (conn.generate_id(), conn.generate_id()) else {
            break;
        };
        let _ = conn.create_pixmap(32, pixmap, root, w, h);
        if !gc_made {
            let _ = conn.create_gc(gc, pixmap, &CreateGCAux::new().graphics_exposures(0));
            gc_made = true;
        }
        let _ = conn.put_image(
            x11rb::protocol::xproto::ImageFormat::Z_PIXMAP,
            pixmap,
            gc,
            w,
            h,
            0,
            0,
            0,
            32,
            &panel.pixels,
        );
        let _ = conn.render_create_picture(src, pixmap, argb, &CreatePictureAux::new());
        let _ = conn.render_composite(
            PictOp::OVER,
            src,
            NONE,
            dst,
            0,
            0,
            0,
            0,
            panel.rect.left as i16,
            panel.rect.top as i16,
            w,
            h,
        );
        let _ = conn.render_free_picture(src);
        let _ = conn.free_pixmap(pixmap);
    }
    if gc_made {
        let _ = conn.free_gc(gc);
    }
    let _ = conn.render_free_picture(dst);
}
