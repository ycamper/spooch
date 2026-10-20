//! Dragging the finished image out to another app with the XDND protocol,
//! as the source side: find the XDND-aware window under the pointer, tell it
//! what is on offer, and hand over the file when it is dropped.
//!
//! The file is offered as `text/uri-list` (what file managers, browsers and
//! chat apps take) and as the image itself for apps that only take image
//! data. The data stays available after the overlay closes, until the
//! target says it has finished or another drag replaces it.

use std::path::{Path, PathBuf};

use x11rb::NONE;
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask, PropMode,
    SELECTION_NOTIFY_EVENT, SelectionNotifyEvent, SelectionRequestEvent, Timestamp, Window,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

/// The XDND version Spooch speaks.
const VERSION: u32 = 5;

pub struct Atoms {
    aware: Atom,
    pub selection: Atom,
    enter: Atom,
    position: Atom,
    pub status: Atom,
    leave: Atom,
    drop: Atom,
    pub finished: Atom,
    action_copy: Atom,
    uri_list: Atom,
    targets: Atom,
}

impl Atoms {
    pub fn intern(conn: &RustConnection) -> Result<Self, String> {
        let get = |name: &[u8]| super::atom(conn, name);
        Ok(Self {
            aware: get(b"XdndAware")?,
            selection: get(b"XdndSelection")?,
            enter: get(b"XdndEnter")?,
            position: get(b"XdndPosition")?,
            status: get(b"XdndStatus")?,
            leave: get(b"XdndLeave")?,
            drop: get(b"XdndDrop")?,
            finished: get(b"XdndFinished")?,
            action_copy: get(b"XdndActionCopy")?,
            uri_list: get(b"text/uri-list")?,
            targets: get(b"TARGETS")?,
        })
    }
}

/// What is being dragged: the file, and the same bytes as image data.
pub struct Offer {
    pub path: PathBuf,
    uri_list: Vec<u8>,
    bytes: Vec<u8>,
    /// `image/png` or `image/jpeg`.
    mime: Atom,
}

impl Offer {
    pub fn new(conn: &RustConnection, path: PathBuf, bytes: Vec<u8>, mime: &str) -> Self {
        let mime = conn
            .intern_atom(false, mime.as_bytes())
            .ok()
            .and_then(|c| c.reply().ok())
            .map_or(NONE, |r| r.atom);
        let uri_list = format!("{}\r\n", file_uri(&path)).into_bytes();
        Self {
            path,
            uri_list,
            bytes,
            mime,
        }
    }
}

/// `file:///...` with everything outside the unreserved set escaped.
pub fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

/// A drag in progress.
pub struct Drag {
    /// Our window that the messages come from and the selection belongs to.
    source: Window,
    /// The XDND-aware window under the pointer, with its protocol version.
    target: Option<(Window, u32)>,
    /// Whether the target said it would take the drop.
    accepted: bool,
    /// A position was sent and its status has not come back yet.
    waiting: bool,
    /// The latest position, held while waiting for a status.
    pending: Option<(i16, i16, Timestamp)>,
}

/// How a release ended the drag.
pub enum Release {
    /// The target took the drop; it will ask for the data.
    Dropped,
    /// Nothing under the pointer would take it.
    Cancelled,
}

impl Drag {
    pub fn start(
        conn: &RustConnection,
        atoms: &Atoms,
        source: Window,
        time: Timestamp,
    ) -> Result<Self, String> {
        conn.set_selection_owner(source, atoms.selection, time)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            source,
            target: None,
            accepted: false,
            waiting: false,
            pending: None,
        })
    }

    /// The pointer moved to `(x, y)` on the root window.
    pub fn motion(
        &mut self,
        conn: &RustConnection,
        atoms: &Atoms,
        offer: &Offer,
        root: Window,
        (x, y): (i16, i16),
        time: Timestamp,
    ) {
        let under = aware_window_at(conn, atoms, root, x, y);
        if under.map(|t| t.0) != self.target.map(|t| t.0) {
            if let Some((old, _)) = self.target.take() {
                self.send(conn, old, atoms.leave, [self.source, 0, 0, 0, 0]);
            }
            self.accepted = false;
            self.waiting = false;
            if let Some((w, version)) = under {
                let types = [atoms.uri_list, offer.mime, NONE];
                self.send(
                    conn,
                    w,
                    atoms.enter,
                    [self.source, version << 24, types[0], types[1], types[2]],
                );
                self.target = Some((w, version));
            }
        }
        if self.target.is_none() {
            return;
        }
        if self.waiting {
            self.pending = Some((x, y, time));
        } else {
            self.position(conn, atoms, x, y, time);
        }
    }

    fn position(&mut self, conn: &RustConnection, atoms: &Atoms, x: i16, y: i16, time: Timestamp) {
        let Some((w, _)) = self.target else { return };
        let packed = ((x as u16 as u32) << 16) | (y as u16 as u32);
        self.send(
            conn,
            w,
            atoms.position,
            [self.source, 0, packed, time, atoms.action_copy],
        );
        self.waiting = true;
        self.pending = None;
    }

    /// The target answered a position.
    pub fn status(&mut self, conn: &RustConnection, atoms: &Atoms, e: &ClientMessageEvent) {
        let data = e.data.as_data32();
        if self.target.map(|t| t.0) != Some(data[0]) {
            return;
        }
        self.accepted = data[1] & 1 == 1;
        self.waiting = false;
        if let Some((x, y, time)) = self.pending.take() {
            self.position(conn, atoms, x, y, time);
        }
    }

    /// The button came up: drop on the target if it accepted, else leave.
    pub fn release(&mut self, conn: &RustConnection, atoms: &Atoms, time: Timestamp) -> Release {
        let Some((w, _)) = self.target.take() else {
            return Release::Cancelled;
        };
        if self.accepted {
            self.send(conn, w, atoms.drop, [self.source, 0, time, 0, 0]);
            Release::Dropped
        } else {
            self.send(conn, w, atoms.leave, [self.source, 0, 0, 0, 0]);
            Release::Cancelled
        }
    }

    /// Esc during the drag.
    pub fn cancel(&mut self, conn: &RustConnection, atoms: &Atoms) {
        if let Some((w, _)) = self.target.take() {
            self.send(conn, w, atoms.leave, [self.source, 0, 0, 0, 0]);
        }
    }

    fn send(&self, conn: &RustConnection, to: Window, kind: Atom, data: [u32; 5]) {
        let event = ClientMessageEvent::new(32, to, kind, data);
        let _ = conn.send_event(false, to, EventMask::NO_EVENT, event);
        let _ = conn.flush();
    }
}

/// The deepest-first XDND-aware window under `(x, y)`: walk down from the
/// root through the mapped child under the point, and take the first one
/// that advertises XDND (with a reparenting window manager that is the
/// app's own window inside the frame).
fn aware_window_at(
    conn: &RustConnection,
    atoms: &Atoms,
    root: Window,
    x: i16,
    y: i16,
) -> Option<(Window, u32)> {
    let mut w = root;
    for _ in 0..32 {
        let child = conn
            .translate_coordinates(root, w, x, y)
            .ok()?
            .reply()
            .ok()?
            .child;
        if child == NONE {
            return None;
        }
        w = child;
        let version = conn
            .get_property(false, w, atoms.aware, AtomEnum::ATOM, 0, 1)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().and_then(|mut v| v.next()));
        if let Some(v) = version {
            return Some((w, v.min(VERSION)));
        }
    }
    None
}

/// Answer the drop target's request for the dragged data. Returns what was
/// served, for the log.
pub fn request(
    conn: &RustConnection,
    atoms: &Atoms,
    offer: &Offer,
    req: &SelectionRequestEvent,
) -> String {
    let property = if req.property == u32::from(AtomEnum::NONE) {
        req.target
    } else {
        req.property
    };
    let mut what = "refused".to_string();
    let served = if req.target == atoms.targets {
        what = "TARGETS".into();
        conn.change_property32(
            PropMode::REPLACE,
            req.requestor,
            property,
            AtomEnum::ATOM,
            &[atoms.targets, atoms.uri_list, offer.mime],
        )
        .is_ok()
    } else if req.target == atoms.uri_list {
        what = format!("text/uri-list {}", offer.path.display());
        conn.change_property8(
            PropMode::REPLACE,
            req.requestor,
            property,
            atoms.uri_list,
            &offer.uri_list,
        )
        .is_ok()
    } else if req.target == offer.mime && offer.bytes.len() + 64 <= conn.maximum_request_bytes() {
        what = format!("image data ({} bytes)", offer.bytes.len());
        conn.change_property8(
            PropMode::REPLACE,
            req.requestor,
            property,
            offer.mime,
            &offer.bytes,
        )
        .is_ok()
    } else {
        false
    };
    let event = SelectionNotifyEvent {
        response_type: SELECTION_NOTIFY_EVENT,
        sequence: 0,
        time: req.time,
        requestor: req.requestor,
        selection: req.selection,
        target: req.target,
        property: if served {
            property
        } else {
            AtomEnum::NONE.into()
        },
    };
    let _ = conn.send_event(false, req.requestor, EventMask::NO_EVENT, event);
    let _ = conn.flush();
    what
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_escape_spaces_and_keep_slashes() {
        assert_eq!(
            file_uri(Path::new(
                "/home/user/.cache/spooch/drag/Spooch 2026-10-03 at 22.53.10.png"
            )),
            "file:///home/user/.cache/spooch/drag/Spooch%202026-10-03%20at%2022.53.10.png"
        );
    }
}
