//! Syncing captures with the account: what changed since a number, pushing
//! a capture's new version, and downloading parts. Parts go straight to
//! and from the bucket through the URLs the service signs.

use std::io::Read;

use serde::Deserialize;

use crate::{Client, Failure, brief, done, random, read};

/// The largest part downloaded: a capture's full image can be big.
const MAX_PART: u64 = 80 << 20;

/// Changes asked for per page. Each carries a presigned thumbnail URL of
/// several hundred bytes, so the service's default of 500 would overrun
/// `MAX_REPLY`.
const CHANGES_PAGE: usize = 50;

/// One capture's change in the account.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Change {
    pub id: String,
    pub version: i64,
    pub seq: i64,
    /// When it was taken, as RFC 3339.
    pub taken: String,
    /// Put in the trash, as RFC 3339; a capture there is gone from devices.
    #[serde(default)]
    pub trashed: Option<String>,
    /// Deleted for good.
    #[serde(default)]
    pub gone: bool,
    /// Its thumbnail, for an hour.
    #[serde(default)]
    pub thumb_url: Option<String>,
}

/// A page of changes, and where to carry on from.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Changes {
    pub captures: Vec<Change>,
    pub next: i64,
    pub more: bool,
}

/// What came of pushing a version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pushed {
    /// It's live, as this version.
    Done(i64),
    /// Another device changed it meanwhile; it's at this version now.
    Conflict(i64),
}

/// A fresh id for a capture in the account.
pub fn new_capture_id() -> String {
    random(16)
}

#[derive(Deserialize)]
struct Begun {
    version: i64,
    uploads: Uploads,
    types: Types,
}

#[derive(Deserialize)]
struct Uploads {
    doc: String,
    source: String,
    thumb: String,
}

#[derive(Deserialize)]
struct Types {
    doc: String,
    source: String,
    thumb: String,
}

#[derive(Deserialize)]
struct Part {
    url: String,
}

#[derive(Deserialize)]
struct Current {
    version: i64,
}

impl Client {
    /// The account's changes after `since`, oldest first.
    pub fn changes(&self, since: i64) -> Result<Changes, String> {
        let path = format!("/captures?since={since}&limit={CHANGES_PAGE}");
        Ok(read(self.api("GET", &path).call())?)
    }

    /// Push a new version of capture `id` on top of `base` (0 for a new
    /// one): its document, full image and thumbnail, uploaded straight to
    /// the bucket. `taken` is when it was taken, as RFC 3339.
    pub fn push_capture(
        &self,
        id: &str,
        base: i64,
        taken: &str,
        (doc, source, thumb): (&[u8], &[u8], &[u8]),
    ) -> Result<Pushed, String> {
        let begun = self
            .api("PUT", &format!("/captures/{id}"))
            .send_json(serde_json::json!({
                "base": base,
                "taken": taken,
                "doc_size": doc.len(),
                "source_size": source.len(),
                "thumb_size": thumb.len(),
            }));
        let begun: Begun = match begun {
            Err(ureq::Error::Status(409, r)) => {
                return match crate::json::<Current>(r) {
                    Ok(c) => Ok(Pushed::Conflict(c.version)),
                    Err(_) => Err("it's in the trash".into()),
                };
            }
            other => read(other)?,
        };
        let u = &begun.uploads;
        let t = &begun.types;
        for (url, kind, bytes) in [
            (&u.doc, &t.doc, doc),
            (&u.source, &t.source, source),
            (&u.thumb, &t.thumb, thumb),
        ] {
            self.agent
                .put(url)
                .set("Content-Type", kind)
                .send_bytes(bytes)
                .map_err(|e| format!("the upload failed: {}", brief(e)))?;
        }
        read::<serde_json::Value>(
            self.api("POST", &format!("/captures/{id}/finish"))
                .send_json(serde_json::json!({"version": begun.version})),
        )?;
        Ok(Pushed::Done(begun.version))
    }

    /// Part `part` ("doc", "source" or "thumb") of capture `id`'s live
    /// version.
    pub fn capture_part(&self, id: &str, part: &str) -> Result<Vec<u8>, String> {
        let p: Part = read(self.api("GET", &format!("/captures/{id}/{part}")).call())?;
        self.download(&p.url)
    }

    /// A signed URL's bytes, like a thumbnail's. No token goes with it.
    pub fn download(&self, url: &str) -> Result<Vec<u8>, String> {
        let r = self
            .agent
            .get(url)
            .call()
            .map_err(|e| format!("the download failed: {}", brief(e)))?;
        let mut bytes = Vec::new();
        r.into_reader()
            .take(MAX_PART + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("the download failed: {e}"))?;
        if bytes.len() as u64 > MAX_PART {
            return Err("the download is too big".into());
        }
        Ok(bytes)
    }

    /// Put capture `id` in the account's trash, for 30 days.
    pub fn trash_capture(&self, id: &str) -> Result<(), String> {
        match done(self.api("DELETE", &format!("/captures/{id}")).call()) {
            // Already gone is as good.
            Err(Failure {
                status: Some(404 | 410),
                ..
            }) => Ok(()),
            other => Ok(other?),
        }
    }
}

impl Client {
    /// Delete capture `id` from the account for good, trash and all.
    pub fn delete_capture_forever(&self, id: &str) -> Result<(), String> {
        Ok(done(
            self.api("DELETE", &format!("/captures/{id}?forever=true"))
                .call(),
        )?)
    }
}
