//! Keeping captures in Recents, and the editor window's gallery of them.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use std::time::Instant;

use spooch_platform::recents::{Pixels, Recents};
use spooch_render::skia_safe::Point;
use spooch_render::{GalleryAction, Styles, SyncBadge, WindowButton, thumbnail};

use super::{Effect, Host, Response, Session, Waker};

/// A thread making thumbnails for the gallery, newest first: asked for by
/// id, handed back as they're ready, waking the window each time.
pub(super) struct Thumbs {
    ask: Sender<String>,
    got: Receiver<(String, Pixels)>,
    asked: HashSet<String>,
}

impl Thumbs {
    fn start(recents: Recents, wake: Waker) -> Thumbs {
        let (ask, todo) = mpsc::channel::<String>();
        let (done, got) = mpsc::channel();
        std::thread::spawn(move || {
            for id in todo {
                if let Some(t) = kept_thumb(&recents, &id) {
                    if done.send((id, t)).is_err() {
                        break;
                    }
                    wake();
                }
            }
        });
        Thumbs {
            ask,
            got,
            asked: HashSet::new(),
        }
    }
}

/// Kept capture `id`'s thumbnail, made and stored if it has none.
fn kept_thumb(recents: &Recents, id: &str) -> Option<Pixels> {
    if let Some(t) = recents.thumb(id) {
        return Some(t);
    }
    let mut s = Session::open(recents, id, Vec::new(), Styles::default(), Host::Window).ok()?;
    let t = s.thumb()?;
    let _ = recents.write_thumb(id, &t);
    Some(t)
}

impl Session {
    /// Keep the capture in Recents on a background thread, so closing
    /// never waits on PNG encoding. `done` gets the log line once it's
    /// written, on that thread. A capture nobody used (`wanted`) and
    /// nobody marked is let go.
    pub fn keep_in_background(&mut self, done: impl FnOnce(String) + Send + 'static) {
        let Some(recents) = self.recents.clone() else {
            return;
        };
        if !self.wanted && !self.has_marks() {
            done(format!(
                "not kept in recents: {} was never marked, copied or saved",
                self.id
            ));
            return;
        }
        let (json, pixels) = self.snapshot();
        let thumb = self.thumb();
        let (id, link) = (self.id.clone(), self.link.clone());
        let keeper = recents.clone();
        let sync = self.sync.clone();
        recents.keep_in_background(self.id.clone(), json, pixels, thumb, move |line| {
            note_share(&keeper, &id, link.as_ref());
            sync.kick();
            done(line);
        });
    }

    /// Keep the capture in Recents now. Returns a line for the log.
    pub(super) fn keep_now(&mut self) -> Option<String> {
        let recents = self.recents.clone()?;
        let (json, pixels) = self.snapshot();
        let thumb = self.thumb();
        let written = recents
            .write(&self.id, &json, pixels.as_ref())
            .and_then(|_| {
                if let Some(t) = &thumb {
                    recents.write_thumb(&self.id, t)?;
                    self.doc
                        .set_gallery_thumb(&self.id, t.width, t.height, &t.bgra);
                }
                Ok(())
            });
        note_share(&recents, &self.id, self.link.as_ref());
        self.sync.kick();
        Some(match written {
            Ok(()) => format!("kept in recents as {}", self.id),
            Err(e) => format!("could not keep {} in recents: {e}", self.id),
        })
    }

    /// A thumbnail of the finished image, for the gallery.
    fn thumb(&mut self) -> Option<Pixels> {
        let (w, h, px) = self.export();
        let (width, height, bgra) = thumbnail(w, h, &px)?;
        Some(Pixels {
            width,
            height,
            bgra,
        })
    }

    /// The document and, unless Recents has them already, its pixels.
    fn snapshot(&mut self) -> (String, Option<Pixels>) {
        self.doc.commit_text();
        let json = self.doc.to_json(&self.stem);
        let pixels = (!self.pixels_kept).then(|| {
            let (width, height, bgra) = self.doc.base_pixels();
            Pixels {
                width,
                height,
                bgra,
            }
        });
        self.pixels_kept = true;
        (json, pixels)
    }

    /// `wake` is how a background thread (a share's upload, or in the
    /// editor window the gallery's thumbnails) gets the shell to call
    /// `poll`. In the window it also fills the gallery.
    pub fn set_waker(&mut self, wake: impl Fn() + Send + Sync + 'static) -> Response {
        let wake: Waker = Arc::new(wake);
        self.sync.watch(&wake);
        if self.host == Host::Window {
            self.thumbs = self.recents.clone().map(|r| Thumbs::start(r, wake.clone()));
            self.fill_gallery();
        }
        self.waker = Some(wake);
        Response {
            repaint: true,
            ..Response::default()
        }
    }

    /// Take what background threads have done since last time: gallery
    /// thumbnails and a share's progress.
    pub fn poll(&mut self) -> Response {
        let mut r = Response::default();
        if let Some(t) = &self.thumbs {
            for (id, p) in t.got.try_iter() {
                self.doc.set_gallery_thumb(&id, p.width, p.height, &p.bgra);
                r.repaint = true;
            }
        }
        self.poll_share(&mut r);
        self.poll_sync(&mut r);
        r
    }

    /// Sync's news: the gallery looks again when Recents changed, and a
    /// capture downloaded to open opens.
    fn poll_sync(&mut self, r: &mut Response) {
        if self.host == Host::Window && self.sync.changes() != self.sync_seen {
            self.sync_seen = self.sync.changes();
            self.fill_gallery();
            r.repaint = true;
        }
        let Some(result) = self
            .fetching
            .as_ref()
            .and_then(|(_, rx)| match rx.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                // The job never ran: no sync thread, or it died.
                Err(TryRecvError::Disconnected) => Some(Err("sync isn't running".to_string())),
            })
        else {
            return;
        };
        let (id, _) = self.fetching.take().expect("checked above");
        self.doc.set_gallery_busy(None);
        r.repaint = true;
        match result {
            Ok(()) => r.repaint = self.open_in_place(&id, true, &mut r.effects),
            Err(e) => r
                .effects
                .push(Effect::Log(format!("could not download {id}: {e}"))),
        }
    }

    /// Download capture `id` from the account, then open it (`poll_sync`).
    pub(super) fn fetch(&mut self, id: String) {
        if self.fetching.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let wake = self.waker.clone();
        self.sync.fetch(&id, move |result| {
            let _ = tx.send(result);
            if let Some(wake) = wake {
                wake();
            }
        });
        self.doc.set_gallery_busy(Some(&id));
        self.fetching = Some((id, rx));
    }

    /// Show Recents in the window's gallery, and ask for thumbnails it
    /// lacks. The one being edited is drawn from the document itself.
    pub(super) fn fill_gallery(&mut self) {
        let Some(recents) = self.recents.clone() else {
            return;
        };
        if self.host != Host::Window {
            return;
        }
        let t = spooch_platform::save::now();
        let shared: HashSet<String> = recents.shared();
        let today = (t.year, t.month, t.day);
        let ids = recents.list_all();
        let syncing = self.sync.is_on();
        let sync: HashMap<String, SyncBadge> = ids
            .iter()
            .filter_map(|id| {
                let badge = if recents.is_stub(id) {
                    SyncBadge::Remote
                } else if recents.up_to_date(id) {
                    SyncBadge::Synced
                } else if syncing {
                    SyncBadge::Waiting
                } else {
                    return None;
                };
                Some((id.clone(), badge))
            })
            .collect();
        self.doc.set_gallery(&ids, &shared, &sync, &self.id, today);
        let Some(thumbs) = &mut self.thumbs else {
            return;
        };
        let own = thumbs.asked.insert(self.id.clone());
        for id in self.doc.gallery_missing() {
            if thumbs.asked.insert(id.clone()) {
                let _ = thumbs.ask.send(id);
            }
        }
        if own && let Some(t) = self.thumb() {
            self.doc
                .set_gallery_thumb(&self.id.clone(), t.width, t.height, &t.bgra);
        }
    }

    /// What a press on the gallery asked for.
    pub(super) fn gallery_action(&mut self, action: GalleryAction, p: Point, r: &mut Response) {
        let i = match action {
            GalleryAction::Press(i) | GalleryAction::Reveal(i) | GalleryAction::Delete(i) => i,
        };
        let (Some(id), Some(recents)) = (
            self.doc.gallery_id(i).map(str::to_owned),
            self.recents.clone(),
        ) else {
            return;
        };
        match action {
            GalleryAction::Press(_) => {
                self.gallery_press = Some((id, p));
                r.effects.push(Effect::CaptureMouse);
            }
            GalleryAction::Reveal(_) => {
                let saved = recents
                    .document(&id)
                    .ok()
                    .and_then(|json| spooch_render::Document::name_in(&json))
                    .and_then(|stem| spooch_platform::save::find_saved(&stem));
                r.effects
                    .push(Effect::Reveal(saved.unwrap_or_else(|| recents.dir(&id))));
            }
            GalleryAction::Delete(_) => {
                // The one being edited gives way to the one before it (or
                // after), unkept; with none left the window closes.
                let next = (id == self.id).then(|| {
                    recents
                        .neighbour(&id, true)
                        .or_else(|| recents.neighbour(&id, false))
                });
                // Synced, it goes to the account's trash too.
                self.sync.trash(&recents, &id);
                if let Err(e) = recents.delete(&id) {
                    r.effects
                        .push(Effect::Log(format!("could not delete {id}: {e}")));
                    return;
                }
                r.effects
                    .push(Effect::Log(format!("deleted {id} from recents")));
                match next {
                    None => self.fill_gallery(),
                    Some(Some(next)) => {
                        r.repaint = self.open_in_place(&next, false, &mut r.effects)
                    }
                    Some(None) => {
                        self.recents = None;
                        r.effects.push(Effect::Window(WindowButton::Close));
                    }
                }
            }
        }
    }

    /// PageUp or PageDown: keep this capture and open the one before
    /// (`older`) or after it in its place. Returns whether it changed.
    pub(super) fn switch_recent(&mut self, older: bool, log: &mut Vec<Effect>) -> bool {
        if self.doc.is_dragging() || self.doc.is_typing() {
            return false;
        }
        match self
            .recents
            .as_ref()
            .and_then(|r| r.neighbour(&self.id, older))
        {
            Some(next) => self.open_in_place(&next, true, log),
            None => false,
        }
    }

    /// Keep this capture (when `keep`) and open kept capture `next` in its
    /// place, with the gallery, sharing and waker carried over. Returns
    /// whether it changed.
    pub(super) fn open_in_place(&mut self, next: &str, keep: bool, log: &mut Vec<Effect>) -> bool {
        let Some(recents) = self.recents.clone() else {
            return false;
        };
        let started = Instant::now();
        if keep && let Some(line) = self.keep_now() {
            log.push(Effect::Log(line));
        }
        let styles = self.doc.styles();
        match Session::open(&recents, next, self.screens.clone(), styles, self.host) {
            Ok(mut s) => {
                // The same window shows it: its size, and the gallery.
                s.fit_window_of(&self.doc);
                s.doc.put_gallery(self.doc.take_gallery());
                s.thumbs = self.thumbs.take();
                s.waker = self.waker.take();
                s.sharing = self.sharing.clone();
                s.sync = self.sync.clone();
                s.sync_seen = self.sync.changes();
                *self = s;
                self.fill_gallery();
                log.push(Effect::Log(self.reopened_line(started)));
                true
            }
            Err(e) => {
                log.push(Effect::Log(format!("could not reopen {next}: {e}")));
                false
            }
        }
    }
}

/// Note in Recents the link a kept capture was shared at, if it has one.
fn note_share(recents: &Recents, id: &str, link: Option<&(String, String)>) {
    if let Some((link, look)) = link
        && let Err(e) = recents.write_share(id, link, look)
    {
        spooch_platform::log::line(format!("couldn't note the share of {id}: {e}"));
    }
}
