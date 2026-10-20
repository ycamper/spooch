//! Owning the CLIPBOARD selection and serving an image as PNG, or text.
//!
//! X11 has no clipboard storage: the owner hands the data to each app that
//! asks, so Spooch has to keep running after a copy. Most Linux setups run a
//! clipboard manager that takes a copy, but we don't rely on one.

use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt as _, EventMask, PropMode, SELECTION_NOTIFY_EVENT,
    SelectionNotifyEvent, SelectionRequestEvent, Timestamp, Window,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

pub struct Atoms {
    pub clipboard: Atom,
    pub targets: Atom,
    pub png: Atom,
    pub utf8: Atom,
    pub text_plain: Atom,
}

impl Atoms {
    pub fn intern(conn: &RustConnection) -> Result<Self, String> {
        let get = |name: &[u8]| super::atom(conn, name);
        Ok(Self {
            clipboard: get(b"CLIPBOARD")?,
            targets: get(b"TARGETS")?,
            png: get(b"image/png")?,
            utf8: get(b"UTF8_STRING")?,
            text_plain: get(b"text/plain;charset=utf-8")?,
        })
    }
}

pub struct Clipboard {
    pub owner: Window,
    pub atoms: Atoms,
    /// What's on offer: each target and its bytes.
    offers: Vec<(Atom, Vec<u8>)>,
}

impl Clipboard {
    pub fn new(owner: Window, atoms: Atoms) -> Self {
        Self {
            owner,
            atoms,
            offers: Vec::new(),
        }
    }

    pub fn set_image(
        &mut self,
        conn: &RustConnection,
        png: Vec<u8>,
        time: Timestamp,
    ) -> Result<(), String> {
        self.set(conn, vec![(self.atoms.png, png)], time)
    }

    pub fn set_text(
        &mut self,
        conn: &RustConnection,
        text: &str,
        time: Timestamp,
    ) -> Result<(), String> {
        let bytes = text.as_bytes().to_vec();
        let offers = [
            self.atoms.utf8,
            self.atoms.text_plain,
            AtomEnum::STRING.into(),
        ]
        .into_iter()
        .map(|a| (a, bytes.clone()))
        .collect();
        self.set(conn, offers, time)
    }

    fn set(
        &mut self,
        conn: &RustConnection,
        offers: Vec<(Atom, Vec<u8>)>,
        time: Timestamp,
    ) -> Result<(), String> {
        conn.set_selection_owner(self.owner, self.atoms.clipboard, time)
            .map_err(|e| e.to_string())?;
        let owner = conn
            .get_selection_owner(self.atoms.clipboard)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?
            .owner;
        if owner != self.owner {
            return Err("another app kept the clipboard".into());
        }
        self.offers = offers;
        Ok(())
    }

    pub fn cleared(&mut self) {
        self.offers.clear();
    }

    /// Answer another app's request for the clipboard contents.
    pub fn request(&self, conn: &RustConnection, req: &SelectionRequestEvent) -> String {
        // Obsolete clients leave the property unset and expect the target.
        let property = if req.property == u32::from(AtomEnum::NONE) {
            req.target
        } else {
            req.property
        };
        let mut what = "refused".to_string();
        let offer = self.offers.iter().find(|(a, _)| *a == req.target);
        let served = if self.offers.is_empty() || req.selection != self.atoms.clipboard {
            false
        } else if req.target == self.atoms.targets {
            what = "TARGETS".into();
            let targets: Vec<Atom> = std::iter::once(self.atoms.targets)
                .chain(self.offers.iter().map(|(a, _)| *a))
                .collect();
            conn.change_property32(
                PropMode::REPLACE,
                req.requestor,
                property,
                AtomEnum::ATOM,
                &targets,
            )
            .is_ok()
        } else if let Some((target, bytes)) = offer {
            // Bigger than one request would need the INCR protocol.
            if bytes.len() + 64 > conn.maximum_request_bytes() {
                what = format!("too large ({} bytes, no INCR yet)", bytes.len());
                false
            } else {
                what = format!("{} bytes", bytes.len());
                conn.change_property8(PropMode::REPLACE, req.requestor, property, *target, bytes)
                    .is_ok()
            }
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
}
