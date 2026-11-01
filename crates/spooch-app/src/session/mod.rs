//! One capture being edited, from the first press to copy, save, drag-out
//! or close, wherever it's shown.

mod deliver;
mod effect;
mod kept;
mod share;
#[cfg(test)]
mod tests;

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use spooch_platform::log::ms_since;
use spooch_platform::recents::Recents;
use spooch_platform::save::LocalTime;
use spooch_render::skia_safe::{IRect, Point};
use spooch_render::{Action, Cursor, Document, Key, KeyOutcome, Mods, Styles};

pub use deliver::{Clip, Deliverable};
pub use effect::{DRAG_SLOP, Effect, Response, dragged_line};

use crate::{Sharing, Syncing};
use kept::Thumbs;
use share::Pending;

/// How a background thread tells the shell to call `Session::poll`.
type Waker = Arc<dyn Fn() + Send + Sync>;

/// Where a session's editor is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    /// Over the frozen screen; copying or cancelling ends it.
    Overlay,
    /// In the editor window, which stays open after a copy.
    Window,
}

/// Marks copied with Ctrl+C or Copy while some were selected, for Ctrl+V
/// in this capture or another. Spooch's own: the system clipboard keeps
/// images.
static COPIED_MARKS: Mutex<Option<String>> = Mutex::new(None);

/// One capture being edited.
pub struct Session {
    /// The editor. Shells draw it (`render`, `chrome`) and read its
    /// canvas; input goes through the session.
    pub doc: Document,
    /// The file name without extension, fixed when editing starts.
    stem: String,
    /// The capture's id in Recents; empty without Recents.
    id: String,
    host: Host,
    recents: Option<Recents>,
    /// The monitors (or the window's view), for placing the chrome and
    /// reopened captures.
    screens: Vec<IRect>,
    /// Whether Recents already has the document's pixels, so only the
    /// marks need writing.
    pixels_kept: bool,
    /// Logged once, the first time the whole desktop is handed over.
    supplied_desktop: bool,
    /// Where a Ctrl+press on the image happened; dragging from it far
    /// enough starts a drag-out.
    drag_out_from: Option<Point>,
    /// A press on a gallery card, and where: let go there opens that
    /// capture, a drag takes it out as a file.
    gallery_press: Option<(String, Point)>,
    /// The kept capture being dragged out of the gallery, for `drag_file`.
    drag_recent: Option<String>,
    /// Makes the gallery's missing thumbnails, in the editor window.
    thumbs: Option<Thumbs>,
    waker: Option<Waker>,
    sharing: Sharing,
    sync: Syncing,
    /// The sync change the gallery was last filled at.
    sync_seen: u64,
    /// A capture from the account being downloaded to open, and where the
    /// download says how it went.
    fetching: Option<(String, Receiver<Result<(), String>>)>,
    /// A share under way.
    upload: Option<Pending>,
    /// The link this capture was shared at, once it has been, and how it
    /// looked then (`Document::look`); an edit since lets it go.
    link: Option<(String, String)>,
    /// How the capture looked when it was last saved, while it still does:
    /// the bar's Save then shows the picture instead.
    saved: Option<String>,
    /// Whether closing keeps it in Recents: once it's been copied, saved,
    /// shared, dragged out or taken to the editor window, or when it came
    /// from Recents. A capture cancelled before any of that isn't kept.
    wanted: bool,
}

impl Session {
    /// A fresh capture: `doc` holds the selection's pixels, already placed
    /// on the desktop. With Recents it gets an id there for `when`.
    pub fn new(
        mut doc: Document,
        stem: String,
        recents: Option<Recents>,
        when: &LocalTime,
        screens: Vec<IRect>,
        host: Host,
    ) -> Session {
        let id = recents.as_ref().map(|r| r.new_id(when)).unwrap_or_default();
        doc.set_name(&stem);
        Session::with(doc, stem, id, host, recents, screens, false)
    }

    /// A session round `doc` on `screens`, whose pixels Recents has
    /// (`pixels_kept`) or hasn't yet.
    fn with(
        mut doc: Document,
        stem: String,
        id: String,
        host: Host,
        recents: Option<Recents>,
        screens: Vec<IRect>,
        pixels_kept: bool,
    ) -> Session {
        doc.set_screens(screens.clone());
        Session {
            doc,
            stem,
            id,
            host,
            recents,
            screens,
            pixels_kept,
            supplied_desktop: false,
            drag_out_from: None,
            gallery_press: None,
            drag_recent: None,
            thumbs: None,
            waker: None,
            sharing: Sharing::default(),
            sync: Syncing::default(),
            sync_seen: 0,
            fetching: None,
            upload: None,
            link: None,
            saved: None,
            wanted: pixels_kept,
        }
    }

    /// Share through `sharing`.
    pub fn with_sharing(mut self, sharing: Sharing) -> Session {
        self.sharing = sharing;
        self
    }

    /// Sync through `sync`.
    pub fn with_sync(mut self, sync: Syncing) -> Session {
        self.sync = sync;
        self
    }

    /// Reopen the kept capture `id`, with marks editable, on `screens`. One
    /// that would land off them is moved onto the first.
    pub fn open(
        recents: &Recents,
        id: &str,
        screens: Vec<IRect>,
        styles: Styles,
        host: Host,
    ) -> Result<Session, String> {
        let kept = recents.read(id).map_err(|e| e.to_string())?;
        let p = kept.pixels;
        let (mut doc, stem) = Document::from_saved(&kept.json, p.width, p.height, &p.bgra)
            .map_err(|e| e.to_string())?;
        doc.carry_styles(styles);
        // How it looked as kept, before any move onto the screens.
        let look = doc.look();
        let c = doc.canvas();
        let inside = |s: &IRect| {
            c.left >= s.left && c.top >= s.top && c.right <= s.right && c.bottom <= s.bottom
        };
        if !screens.iter().any(inside)
            && let Some(s) = screens.first()
        {
            doc.shift(s.left - c.left, s.top - c.top);
        }
        let mut s = Session::with(
            doc,
            stem,
            id.to_string(),
            host,
            Some(recents.clone()),
            screens,
            true,
        );
        if let Some((link, shared_look)) = recents.share(id) {
            s.restore_link(link, &shared_look, &look);
        }
        Ok(s)
    }

    /// Open kept capture `id`, or the newest, for the editor window.
    /// Returns the session and a line for the log, or why it couldn't.
    pub fn open_editor(
        recents: Option<&Recents>,
        id: Option<String>,
        styles: Styles,
    ) -> Result<(Session, String), String> {
        let recents = recents
            .ok_or_else(|| "the editor needs Recents, which --no-recents turns off".to_string())?;
        let id = id
            .or_else(|| recents.latest())
            .ok_or_else(|| "nothing in recents to open yet".to_string())?;
        let started = Instant::now();
        let session = Session::open(recents, &id, Vec::new(), styles, Host::Window)
            .map_err(|e| format!("could not open {id}: {e}"))?;
        let line = format!(
            "opened {id} {} in the editor in {:.1} ms",
            session.describe(),
            ms_since(started)
        );
        Ok((session, line))
    }

    /// Reopen kept capture `id` over the frozen screen, on `screens`.
    /// Returns the session and a line for the log, or why it couldn't.
    pub fn reopen(
        recents: Option<&Recents>,
        id: &str,
        screens: Vec<IRect>,
        styles: Styles,
    ) -> Result<(Session, String), String> {
        let recents = recents.ok_or_else(|| "Recents are off".to_string())?;
        let started = Instant::now();
        let s = Session::open(recents, id, screens, styles, Host::Overlay)
            .map_err(|e| format!("could not reopen {id}: {e}"))?;
        let line = s.reopened_line(started);
        Ok((s, line))
    }

    fn reopened_line(&self, started: Instant) -> String {
        format!(
            "reopened {} {} in {:.1} ms",
            self.id,
            self.describe(),
            ms_since(started)
        )
    }

    /// Show `next` in this session's editor window instead: this capture
    /// is kept in Recents (`log` gets the line), and `next` takes over the
    /// window's size.
    pub fn replace(&mut self, mut next: Session, log: impl FnOnce(String) + Send + 'static) {
        self.keep_in_background(log);
        next.fit_window_of(&self.doc);
        *self = next;
    }

    /// Size the editor to the window `old` was shown in.
    fn fit_window_of(&mut self, old: &Document) {
        if let Some(((w, h), maximized)) = old.window_state() {
            self.doc.set_maximized(maximized);
            self.doc.set_window_size(w, h);
        }
    }

    /// The log's line for closing the editor window on this capture.
    pub fn closing_line(&self) -> String {
        let kept = if self.recents.is_some() {
            ", keeping it in recents"
        } else {
            ""
        };
        format!(
            "closed the editor window on {} {}{kept}",
            self.id,
            self.describe()
        )
    }

    #[cfg(test)]
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub fn stem(&self) -> &str {
        &self.stem
    }

    #[cfg(test)]
    pub(crate) fn host(&self) -> Host {
        self.host
    }

    /// Size and marks, for the log: "(640x480, 2 arrows)".
    pub fn describe(&self) -> String {
        format!(
            "({}x{}, {})",
            self.doc.width(),
            self.doc.height(),
            self.doc.counts()
        )
    }

    /// The same capture, for the editor window: it keeps editing there,
    /// over the pixels it holds.
    pub fn into_window(mut self) -> Session {
        self.host = Host::Window;
        self.drag_out_from = None;
        self.wanted = true;
        self.doc.confine();
        self
    }

    /// The editor window is `width` by `height` pixels.
    pub fn resize(&mut self, width: i32, height: i32) -> Response {
        self.doc.set_window_size(width, height);
        Response {
            repaint: true,
            ..Response::default()
        }
    }

    /// The mouse wheel, as `Document::wheel` takes it.
    pub fn wheel(&mut self, p: Point, notches: f32, sideways: bool, ctrl: bool) -> Response {
        Response {
            repaint: self.doc.wheel(p, notches, sideways, ctrl),
            ..Response::default()
        }
    }

    /// The mouse button went down at `p`.
    pub fn press(&mut self, p: Point, mods: Mods) -> Response {
        let mut r = Response {
            repaint: true,
            ..Response::default()
        };
        // Ctrl+drag on the image drags it out.
        if mods.ctrl && self.doc.on_canvas(p) && !self.doc.on_chrome(p) {
            self.drag_out_from = Some(p);
            r.effects.push(Effect::CaptureMouse);
            return r;
        }
        self.doc.set_modifiers(mods.shift, mods.alt);
        let started = self.doc.begin_drag(p);
        match self.doc.take_action() {
            Some(Action::Copy) => {
                r.effects.push(self.copy());
                return r;
            }
            Some(Action::Save) => {
                r.effects.push(self.save_or_show());
                return r;
            }
            Some(Action::Share) => {
                self.share(&mut r);
                return r;
            }
            Some(Action::OpenWindow) => {
                r.effects.push(Effect::OpenWindow);
                return r;
            }
            Some(Action::Window(b)) => {
                r.effects.push(Effect::Window(b));
                return r;
            }
            Some(Action::Settings) => {
                r.effects.push(Effect::OpenSettings);
                return r;
            }
            Some(Action::Gallery(a)) => {
                self.gallery_action(a, p, &mut r);
                return r;
            }
            None => {}
        }
        // Cropping needs nothing more, but extending needs the rest of the
        // frozen desktop.
        self.want_desktop(&mut r);
        if started {
            r.effects.push(Effect::CaptureMouse);
        }
        r
    }

    /// The pointer moved to `p`, with `mods` held. A drag-out starts once
    /// it's gone further than `slop` (the OS's drag threshold) from the
    /// press.
    pub fn motion(&mut self, p: Point, slop: (f32, f32), mods: Mods) -> Response {
        let mut r = Response::default();
        self.doc.set_modifiers(mods.shift, mods.alt);
        let far = |from: Point| (p.x - from.x).abs() > slop.0 || (p.y - from.y).abs() > slop.1;
        if let Some(from) = self.drag_out_from {
            if far(from) {
                self.drag_out_from = None;
                self.wanted = true;
                r.effects.push(Effect::StartDragOut);
            }
            return r;
        }
        if let Some((id, from)) = &self.gallery_press {
            if far(*from) {
                self.drag_recent = Some(id.clone());
                self.gallery_press = None;
                r.effects.push(Effect::StartDragOut);
            }
            return r;
        }
        let moved = self.doc.is_dragging() && self.doc.drag_to(p);
        // Hovering highlights a button and shows its tooltip.
        let hover = self.doc.pointer_moved(p);
        r.repaint = moved || hover;
        r
    }

    /// The mouse button came up at `p`.
    pub fn release(&mut self, p: Point) -> Response {
        let mut r = self.released(p);
        r.repaint |= self.drop_stale_link() | self.drop_stale_save();
        r
    }

    fn released(&mut self, p: Point) -> Response {
        let mut r = Response::default();
        // A gallery card let go of where it was pressed opens.
        if let Some((id, _)) = self.gallery_press.take() {
            // One only in the account downloads first.
            if self.recents.as_ref().is_some_and(|rec| rec.is_stub(&id)) {
                self.fetch(id);
                r.repaint = true;
            } else {
                r.repaint = self.open_in_place(&id, true, &mut r.effects);
            }
            return r;
        }
        // A Ctrl+click without dragging does nothing.
        if self.drag_out_from.take().is_none() && self.doc.is_dragging() {
            self.doc.drag_to(p);
            self.doc.end_drag();
            r.repaint = true;
        }
        r
    }

    /// A key was let go.
    pub fn key_up(&mut self, key: Key) {
        self.doc.key_up(key);
    }

    /// A key, as `Document::key` takes it, plus the session's own: Ctrl+V
    /// pastes copied marks, Ctrl+E moves the overlay's capture into the
    /// window, and in the window Ctrl+C copies the selected marks when
    /// there are some.
    pub fn key(&mut self, key: Key, mods: Mods) -> Response {
        let mut r = self.keyed(key, mods);
        r.repaint |= self.drop_stale_link() | self.drop_stale_save();
        r
    }

    fn keyed(&mut self, key: Key, mods: Mods) -> Response {
        let mut r = Response::default();
        if mods.ctrl && !mods.alt && !self.doc.is_typing() {
            match key {
                Key::Char('v') => {
                    let copied = COPIED_MARKS
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone();
                    r.repaint = copied.is_some_and(|m| self.doc.paste(&m));
                    r.used = true;
                    return r;
                }
                Key::Char('e') if self.host == Host::Overlay => {
                    r.effects.push(Effect::OpenWindow);
                    r.used = true;
                    return r;
                }
                Key::Char('c') if !mods.shift && self.host == Host::Window => {
                    r.effects.push(self.copy());
                    r.used = true;
                    return r;
                }
                _ => {}
            }
        }
        match self.doc.key(key, mods) {
            KeyOutcome::Ignored => return r,
            KeyOutcome::Handled { changed } => {
                r.repaint = changed;
                // A nudge past the selection's pixels needs the rest of the
                // desktop, like an edge drag.
                self.want_desktop(&mut r);
            }
            KeyOutcome::Copy { save: true } => r.effects.push(self.save()),
            KeyOutcome::Copy { save: false } => r.effects.push(self.deliver(false)),
            KeyOutcome::Share => self.share(&mut r),
            KeyOutcome::Cancel => {
                if self.host == Host::Overlay {
                    r.effects.push(Effect::Cancel);
                }
            }
            KeyOutcome::Recent { older } => r.repaint = self.switch_recent(older, &mut r.effects),
        }
        r.used = true;
        r
    }

    /// Text a key typed that `key` didn't use: into a label, or starting
    /// one at `pointer`.
    pub fn type_char(&mut self, c: char, pointer: Point) -> Response {
        Response {
            repaint: self.doc.type_char(c, pointer),
            ..Response::default()
        }
    }

    /// The pointer shape for `p`.
    pub fn cursor(&self, p: Point) -> Cursor {
        if self.drag_out_from.is_some() {
            Cursor::Hand
        } else {
            self.doc.cursor_at(p)
        }
    }

    /// The frozen desktop's pixels, as asked for by `Effect::NeedDesktop`,
    /// read in `ms`. Returns a line for the log the first time.
    pub fn supply_desktop(
        &mut self,
        (width, height, stride): (u32, u32, usize),
        data: &[u8],
        ms: f64,
    ) -> Option<String> {
        let ok = self.doc.supply_desktop(width, height, stride, data);
        // Recents now needs the bigger pixels.
        if ok {
            self.pixels_kept = false;
        }
        if self.supplied_desktop {
            return None;
        }
        self.supplied_desktop = true;
        Some(format!(
            "desktop {width}x{height} handed to the editor in {ms:.1} ms{}",
            if ok { "" } else { " (refused)" }
        ))
    }

    /// Copy: in the window, the selected marks when there are some; else
    /// the image.
    fn copy(&mut self) -> Effect {
        if self.host == Host::Window
            && let Some(marks) = self.doc.copy_selected()
        {
            *COPIED_MARKS.lock().unwrap_or_else(|e| e.into_inner()) = Some(marks);
            return Effect::Log("copied the selected marks".into());
        }
        self.deliver(false)
    }

    /// The bar's Save: show the saved picture when the capture still looks
    /// as it did when saved, else save it.
    fn save_or_show(&mut self) -> Effect {
        if self.saved.is_some()
            && !self.doc.is_typing()
            && self.saved == Some(self.doc.look())
            && let Some(path) =
                spooch_platform::save::find_saved(&self.stem).or_else(spooch_platform::save::folder)
        {
            return Effect::Reveal(path);
        }
        self.save()
    }

    /// Copy and save; in the window, Save then shows the picture.
    fn save(&mut self) -> Effect {
        if self.host == Host::Window {
            self.saved = Some(self.doc.look());
            self.doc.set_save_label(Some("Show"));
        }
        self.deliver(true)
    }

    /// Go back to saving once the capture no longer looks as it was saved.
    /// Returns whether the bar changed.
    fn drop_stale_save(&mut self) -> bool {
        let stale = match &self.saved {
            Some(look) => !self.doc.is_typing() && *look != self.doc.look(),
            None => false,
        };
        if stale {
            self.saved = None;
            self.doc.set_save_label(None);
        }
        stale
    }

    fn deliver(&mut self, save: bool) -> Effect {
        self.wanted = true;
        Effect::Deliver {
            save,
            close: self.host == Host::Overlay,
        }
    }

    fn want_desktop(&self, r: &mut Response) {
        if let Some(want) = self.doc.wants_desktop() {
            r.effects.push(Effect::NeedDesktop(want));
        }
    }
}
