//! Sharing the capture: the upload, its progress on the Share button, and
//! the link it keeps.

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use super::{Effect, Host, Response, Session};

/// A share under way: its link, how the capture looked, and what its
/// upload thread says.
pub(super) struct Pending {
    link: String,
    look: String,
    heard: Receiver<Heard>,
    tell: Sender<Heard>,
}

enum Heard {
    /// The upload worked, or didn't.
    Done(bool),
    /// Time for the overlay to close, a moment after a share worked.
    Close,
}

/// How long the overlay shows "Shared" before it closes.
const SHOW_SHARED: Duration = Duration::from_millis(900);

impl Session {
    /// Share the finished image, or copy the link again if it's unchanged
    /// since it was shared. The link goes out at once as
    /// `Effect::CopyText`; the Share button says how the upload goes.
    pub(super) fn share(&mut self, r: &mut Response) {
        r.repaint = true;
        self.wanted = true;
        self.doc.commit_text();
        self.drop_stale_link();
        if let Some((link, _)) = &self.link {
            r.effects.push(Effect::CopyText(link.clone()));
            self.doc.set_share_label(Some("Link copied"));
            return;
        }
        if self.upload.is_some() {
            return;
        }
        let (tell, heard) = mpsc::channel();
        let look = self.doc.look();
        let note = self.recents.clone().filter(|_| !self.id.is_empty());
        let (id, noted_look) = (self.id.clone(), look.clone());
        let (wake, done_tell) = (self.waker.clone(), tell.clone());
        let image = self.export();
        let started = self.sharing.share(image, move |link| {
            // Noted here, so it lands even if the editor has moved on.
            if let (Some(link), Some(recents)) = (&link, &note)
                && let Err(e) = recents.write_share(&id, link, &noted_look)
            {
                spooch_platform::log::line(format!("couldn't note the share: {e}"));
            }
            let _ = done_tell.send(Heard::Done(link.is_some()));
            if let Some(wake) = wake {
                wake();
            }
        });
        match started {
            Ok(link) => {
                r.effects.push(Effect::CopyText(link.clone()));
                self.doc.set_share_label(Some("Uploading\u{2026}"));
                self.upload = Some(Pending {
                    link,
                    look,
                    heard,
                    tell,
                });
            }
            Err(e) => self.doc.set_share_label(Some(&e)),
        }
    }

    /// Take a share's news since last time.
    pub(super) fn poll_share(&mut self, r: &mut Response) {
        let Some(p) = &self.upload else { return };
        let heard: Vec<Heard> = p.heard.try_iter().collect();
        for h in heard {
            r.repaint = true;
            match h {
                Heard::Done(true) => {
                    self.doc.set_share_label(Some("Shared \u{b7} link copied"));
                    // Kept even if it was edited meanwhile: the next edit
                    // lets it go, so "Shared" shows first.
                    self.link = self
                        .upload
                        .as_ref()
                        .map(|p| (p.link.clone(), p.look.clone()));
                    self.fill_gallery();
                    if self.host == Host::Overlay {
                        self.close_soon();
                    } else {
                        self.upload = None;
                    }
                }
                Heard::Done(false) => {
                    self.doc.set_share_label(Some("Share failed, try again"));
                    self.upload = None;
                }
                Heard::Close => r.effects.push(Effect::Shared),
            }
        }
    }

    /// In the overlay: close once "Shared" has been seen.
    fn close_soon(&self) {
        let (Some(p), Some(wake)) = (&self.upload, self.waker.clone()) else {
            return;
        };
        let tell = p.tell.clone();
        std::thread::spawn(move || {
            std::thread::sleep(SHOW_SHARED);
            let _ = tell.send(Heard::Close);
            wake();
        });
    }

    /// The link a kept capture was shared at, if it still looks as it did
    /// then. `look` is how it looks as kept, before any move onto screen.
    pub(super) fn restore_link(&mut self, link: String, shared_look: &str, look: &str) {
        if shared_look == look {
            self.link = Some((link, self.doc.look()));
            self.doc.set_share_label(Some("Copy link"));
        }
    }

    /// Let go of the link once the capture no longer looks the way it was
    /// shared, so Share uploads it again. Returns whether the bar changed.
    pub(super) fn drop_stale_link(&mut self) -> bool {
        let stale = match &self.link {
            Some((_, look)) => !self.doc.is_typing() && *look != self.doc.look(),
            None => false,
        };
        if stale {
            self.link = None;
            self.doc.set_share_label(None);
        }
        stale
    }
}
