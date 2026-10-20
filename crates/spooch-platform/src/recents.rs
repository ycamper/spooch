//! Recents: every capture that reached the editor, kept so it can be
//! reopened with its marks. Each is a folder named by its time holding
//! `document.json`, `source.png` and `thumb.png`. Writes run on a
//! background thread; a lock keeps reads from seeing half a capture.
//!
//! A synced capture's folder also has `sync.txt`. One another device made
//! starts as a stub: its thumbnail and that note, the rest downloaded when
//! it's opened. Pruning turns synced captures back into stubs.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::save::LocalTime;
use crate::write_atomic;

const DOCUMENT: &str = "document.json";
const SOURCE: &str = "source.png";
const THUMB: &str = "thumb.png";
/// The link a capture was shared at, and on the next line how it looked
/// then (the editor's own string), so an edit since shows.
const SHARE: &str = "share.txt";
/// A synced capture's note: its id in the account, its version there, and
/// when its document was last written as that version.
const SYNC: &str = "sync.txt";
/// The account's change number synced up to, in the root.
const CURSOR: &str = "sync-cursor.txt";

/// What a synced capture's `sync.txt` says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncNote {
    /// Its id in the account.
    pub id: String,
    /// The account's version of it this copy is.
    pub version: i64,
    /// The document's modified time (in nanoseconds since 1970) as that
    /// version, so a later edit shows; 0 for a stub.
    pub stamp: u128,
}

/// How long captures are kept, and how much disk they may use together.
pub const KEEP_FOR: Duration = Duration::from_secs(30 * 24 * 60 * 60);
pub const KEEP_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Held while a capture is written or read.
static LOCK: Mutex<()> = Mutex::new(());

/// Folders `new_id` has handed out. A capture's folder is only made when
/// it's kept, so without this the sync thread could be handed the same id
/// as a capture still open in the editor.
static GIVEN: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

/// Captured pixels: width, height and tightly packed BGRA.
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// One kept capture as read back.
pub struct Kept {
    pub json: String,
    pub pixels: Pixels,
}

#[derive(Clone)]
pub struct Recents {
    root: PathBuf,
}

impl Recents {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The folder capture `id` is kept in.
    pub fn dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    /// A fresh id for a capture made at `t`: "2026-10-03 23.10.42", with
    /// " 2", " 3" and so on if that second is taken.
    pub fn new_id(&self, t: &LocalTime) -> String {
        let base = format!("{} {}", t.date(), t.dotted_time());
        let mut given = GIVEN.lock().unwrap_or_else(|e| e.into_inner());
        let given = given.get_or_insert_default();
        let id = std::iter::once(base.clone())
            .chain((2..).map(|n| format!("{base} {n}")))
            .find(|id| {
                let dir = self.root.join(id);
                !dir.exists() && !given.contains(&dir)
            })
            .expect("some suffix is free");
        given.insert(self.root.join(&id));
        id
    }

    /// The newest kept capture, if there is one.
    pub fn latest(&self) -> Option<String> {
        self.list().pop()
    }

    /// Every kept capture, oldest first.
    pub fn list(&self) -> Vec<String> {
        self.listed(|dir| dir.join(DOCUMENT).is_file())
    }

    /// Every kept capture and stub, oldest first: what the gallery shows.
    pub fn list_all(&self) -> Vec<String> {
        self.listed(|dir| dir.join(DOCUMENT).is_file() || dir.join(SYNC).is_file())
    }

    /// The folders `keep` takes, by id, oldest first.
    fn listed(&self, keep: impl Fn(&Path) -> bool) -> Vec<String> {
        let mut ids: Vec<String> = fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| keep(&e.path()))
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        ids.sort_by(|a, b| id_key(a).cmp(&id_key(b)));
        ids
    }

    /// Whether `id` is a stub: synced, with only its thumbnail here yet.
    pub fn is_stub(&self, id: &str) -> bool {
        let dir = self.root.join(id);
        dir.join(SYNC).is_file() && !dir.join(DOCUMENT).is_file()
    }

    /// The stubs, for the gallery's badge.
    pub fn stubs(&self) -> HashSet<String> {
        self.list_all()
            .into_iter()
            .filter(|id| self.is_stub(id))
            .collect()
    }

    /// Whether kept capture `id` is in the account as it is here: synced,
    /// and not changed since.
    pub fn up_to_date(&self, id: &str) -> bool {
        match (self.sync_note(id), self.stamp(id)) {
            (Some(note), Some(stamp)) => note.stamp == stamp,
            _ => false,
        }
    }

    /// Capture `id`'s sync note, if it's synced.
    pub fn sync_note(&self, id: &str) -> Option<SyncNote> {
        let text = fs::read_to_string(self.root.join(id).join(SYNC)).ok()?;
        let mut lines = text.lines();
        Some(SyncNote {
            id: lines.next()?.trim().to_string(),
            version: lines.next()?.trim().parse().ok()?,
            stamp: lines.next()?.trim().parse().ok()?,
        })
    }

    pub fn write_sync_note(&self, id: &str, note: &SyncNote) -> io::Result<()> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        self.note(id, note)
    }

    /// `write_sync_note` with the lock held.
    fn note(&self, id: &str, note: &SyncNote) -> io::Result<()> {
        let text = format!("{}\n{}\n{}\n", note.id, note.version, note.stamp);
        write_atomic(&self.root.join(id).join(SYNC), text.as_bytes())
    }

    /// When capture `id`'s document was last written, in nanoseconds
    /// since 1970.
    pub fn stamp(&self, id: &str) -> Option<u128> {
        let modified = fs::metadata(self.root.join(id).join(DOCUMENT))
            .and_then(|m| m.modified())
            .ok()?;
        Some(modified.duration_since(UNIX_EPOCH).ok()?.as_nanos())
    }

    /// Capture `id`'s files as they are on disk: document, image and
    /// thumbnail, for uploading.
    pub fn files(&self, id: &str) -> io::Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = self.root.join(id);
        Ok((
            fs::read(dir.join(DOCUMENT))?,
            fs::read(dir.join(SOURCE))?,
            fs::read(dir.join(THUMB))?,
        ))
    }

    /// Make a stub for a capture another device synced: its thumbnail (a
    /// PNG) and its note.
    pub fn write_stub(&self, id: &str, thumb_png: &[u8], note: &SyncNote) -> io::Result<()> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = self.root.join(id);
        fs::create_dir_all(&dir)?;
        write_atomic(&dir.join(THUMB), thumb_png)?;
        let stub = SyncNote {
            stamp: 0,
            ..note.clone()
        };
        self.note(id, &stub)
    }

    /// Fill in synced capture `id` with what was downloaded: its document,
    /// image and thumbnail at `version`. It's then up to date with that
    /// version.
    pub fn write_synced(
        &self,
        id: &str,
        (doc, source, thumb): (&[u8], &[u8], &[u8]),
        note: &SyncNote,
    ) -> io::Result<()> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = self.root.join(id);
        fs::create_dir_all(&dir)?;
        write_atomic(&dir.join(SOURCE), source)?;
        write_atomic(&dir.join(THUMB), thumb)?;
        write_atomic(&dir.join(DOCUMENT), doc)?;
        let modified = fs::metadata(dir.join(DOCUMENT))?.modified()?;
        let stamp = modified
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        self.note(
            id,
            &SyncNote {
                stamp,
                ..note.clone()
            },
        )
    }

    /// The account change number synced up to, 0 before the first.
    pub fn sync_cursor(&self) -> i64 {
        fs::read_to_string(self.root.join(CURSOR))
            .ok()
            .and_then(|t| t.trim().parse().ok())
            .unwrap_or(0)
    }

    pub fn set_sync_cursor(&self, n: i64) -> io::Result<()> {
        fs::create_dir_all(&self.root)?;
        write_atomic(&self.root.join(CURSOR), n.to_string().as_bytes())
    }

    /// The id just before (`older`) or after `current` in time, counting
    /// `current` even if it isn't written yet.
    pub fn neighbour(&self, current: &str, older: bool) -> Option<String> {
        let mut ids = self.list();
        if !ids.iter().any(|id| id == current) {
            ids.push(current.to_string());
            ids.sort_by(|a, b| id_key(a).cmp(&id_key(b)));
        }
        let i = ids.iter().position(|id| id == current)?;
        if older {
            i.checked_sub(1).map(|j| ids[j].clone())
        } else {
            ids.get(i + 1).cloned()
        }
    }

    /// Write a capture: its JSON, and its pixels when given (they don't
    /// change when an already kept capture is only re-marked). Returns the
    /// folder's size in bytes.
    pub fn write(&self, id: &str, json: &str, pixels: Option<&Pixels>) -> io::Result<u64> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = self.root.join(id);
        fs::create_dir_all(&dir)?;
        if let Some(p) = pixels {
            write_atomic(
                &dir.join(SOURCE),
                &crate::encode_png(p.width, p.height, &p.bgra).map_err(io::Error::other)?,
            )?;
        } else if !dir.join(SOURCE).is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{id} has no pixels yet"),
            ));
        }
        write_atomic(&dir.join(DOCUMENT), json.as_bytes())?;
        Ok(folder_size(&dir))
    }

    /// Write capture `id`'s thumbnail: straight BGRA, alpha kept.
    pub fn write_thumb(&self, id: &str, thumb: &Pixels) -> io::Result<()> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let png =
            crate::encode_png(thumb.width, thumb.height, &thumb.bgra).map_err(io::Error::other)?;
        write_atomic(&self.root.join(id).join(THUMB), &png)
    }

    /// Capture `id`'s thumbnail, if it has one yet.
    pub fn thumb(&self, id: &str) -> Option<Pixels> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let bytes = fs::read(self.root.join(id).join(THUMB)).ok()?;
        decode_png(&bytes, true).ok()
    }

    /// Note that kept capture `id` was shared at `link`, looking like
    /// `look` (one line). Does nothing for one not kept yet: keeping it
    /// writes the note then.
    pub fn write_share(&self, id: &str, link: &str, look: &str) -> io::Result<()> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = self.root.join(id);
        if !dir.join(DOCUMENT).is_file() {
            return Ok(());
        }
        write_atomic(&dir.join(SHARE), format!("{link}\n{look}").as_bytes())
    }

    /// The link capture `id` was shared at and how it looked then, if it
    /// was shared.
    pub fn share(&self, id: &str) -> Option<(String, String)> {
        let note = fs::read_to_string(self.root.join(id).join(SHARE)).ok()?;
        let (link, look) = note.split_once('\n')?;
        Some((link.to_string(), look.to_string()))
    }

    /// The captures that have been shared.
    pub fn shared(&self) -> HashSet<String> {
        fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().join(SHARE).is_file())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect()
    }

    /// Remove capture `id` for good.
    pub fn delete(&self, id: &str) -> io::Result<()> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fs::remove_dir_all(self.root.join(id))
    }

    /// Kept capture `id`'s document alone, without its pixels.
    pub fn document(&self, id: &str) -> io::Result<String> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fs::read_to_string(self.root.join(id).join(DOCUMENT))
    }

    pub fn read(&self, id: &str) -> io::Result<Kept> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = self.root.join(id);
        let json = fs::read_to_string(dir.join(DOCUMENT))?;
        let pixels = decode_png(&fs::read(dir.join(SOURCE))?, false)?;
        Ok(Kept { json, pixels })
    }

    /// Remove captures older than `max_age`, then the oldest ones until
    /// the rest fit in `max_bytes`. `keep` is never removed. Returns how
    /// many were removed.
    pub fn prune(&self, now: SystemTime, max_age: Duration, max_bytes: u64, keep: &str) -> usize {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut kept: Vec<(String, SystemTime, u64)> = self
            .list()
            .into_iter()
            .map(|id| {
                let dir = self.root.join(&id);
                let modified = fs::metadata(dir.join(DOCUMENT))
                    .and_then(|m| m.modified())
                    .unwrap_or(now);
                let size = folder_size(&dir);
                (id, modified, size)
            })
            .collect();
        let mut removed = 0;
        // A synced capture is kept in the account: only its document and
        // image go, and it stays as a stub.
        let mut remove = |id: &str| {
            let dir = self.root.join(id);
            let gone = id != keep
                && if dir.join(SYNC).is_file() {
                    fs::remove_file(dir.join(DOCUMENT)).is_ok()
                        && fs::remove_file(dir.join(SOURCE)).is_ok()
                } else {
                    fs::remove_dir_all(&dir).is_ok()
                };
            removed += usize::from(gone);
            gone
        };
        kept.retain(|(id, modified, _)| {
            let old = now.duration_since(*modified).unwrap_or_default() > max_age;
            !(old && remove(id))
        });
        let mut total: u64 = kept.iter().map(|(.., size)| size).sum();
        // Oldest first, so the newest survive.
        for (id, _, size) in &kept {
            if total <= max_bytes {
                break;
            }
            if remove(id) {
                total -= size;
            }
        }
        removed
    }

    /// Write a capture, with its thumbnail when given, and prune on a
    /// background thread. `log` receives one line saying what happened.
    pub fn keep_in_background(
        &self,
        id: String,
        json: String,
        pixels: Option<Pixels>,
        thumb: Option<Pixels>,
        log: impl FnOnce(String) + Send + 'static,
    ) {
        let recents = self.clone();
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let written = recents.write(&id, &json, pixels.as_ref()).and_then(|size| {
                if let Some(t) = &thumb {
                    recents.write_thumb(&id, t)?;
                }
                Ok(size)
            });
            let line = match written {
                Ok(size) => {
                    let removed = recents.prune(SystemTime::now(), KEEP_FOR, KEEP_BYTES, &id);
                    format!(
                        "kept in recents as {id} ({} KB) in {:.0} ms{}",
                        size / 1024,
                        crate::log::ms_since(started),
                        if removed > 0 {
                            format!(", removed {removed} old")
                        } else {
                            String::new()
                        }
                    )
                }
                Err(e) => format!("could not keep {id} in recents: {e}"),
            };
            // Free the pixels before reporting, so a memory trim in
            // `log` can hand them back.
            drop(pixels);
            drop(json);
            log(line);
        });
    }
}

/// Sort ids by time, then by their " 2", " 3" suffix as a number.
fn id_key(id: &str) -> (&str, u32) {
    // The time part is fixed width: "2026-10-03 23.10.42" is 19 bytes.
    match id.get(..19) {
        Some(time) => (time, id[19..].trim().parse().unwrap_or(1)),
        None => (id, 0),
    }
}

/// Write through a temporary file, so a crash never leaves half a file.
fn folder_size(dir: &Path) -> u64 {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

/// Decode a PNG to straight BGRA, dropping any alpha unless `alpha`:
/// captures are opaque, thumbnails of framed captures aren't.
fn decode_png(bytes: &[u8], alpha: bool) -> io::Result<Pixels> {
    let mut decoder = png::Decoder::new(io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().map_err(io::Error::other)?;
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader.next_frame(&mut buf).map_err(io::Error::other)?;
    let (w, h) = (info.width, info.height);
    let channels = info.color_type.samples();
    let mut bgra = Vec::with_capacity(w as usize * h as usize * 4);
    for px in buf[..info.buffer_size()].chunks_exact(channels) {
        // Colour, or grey; ALPHA always adds an alpha channel last.
        let a = if alpha { px[channels - 1] } else { 255 };
        if channels >= 3 {
            bgra.extend_from_slice(&[px[2], px[1], px[0], a]);
        } else {
            bgra.extend_from_slice(&[px[0], px[0], px[0], a]);
        }
    }
    Ok(Pixels {
        width: w,
        height: h,
        bgra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: LocalTime = LocalTime {
        year: 2026,
        month: 10,
        day: 3,
        hour: 23,
        minute: 10,
        second: 42,
    };

    #[test]
    fn thumbnails_keep_alpha_shares_are_noted_and_delete_removes_the_folder() {
        let r = scratch("thumb");
        let id = r.new_id(&T);
        r.write(&id, "{}", Some(&pixels(1))).unwrap();
        assert!(r.thumb(&id).is_none());
        let t = Pixels {
            width: 2,
            height: 1,
            bgra: vec![10, 20, 30, 0, 40, 50, 60, 255],
        };
        r.write_thumb(&id, &t).unwrap();
        assert!(r.shared().is_empty());
        r.write_share("not-kept", "https://img.test/x", "{}")
            .unwrap();
        assert!(
            r.shared().is_empty(),
            "a capture that isn't kept got a note"
        );
        r.write_share(&id, "https://img.test/abc", "{\"look\":1}")
            .unwrap();
        assert_eq!(r.shared(), HashSet::from([id.clone()]));
        assert_eq!(
            r.share(&id),
            Some(("https://img.test/abc".into(), "{\"look\":1}".into()))
        );
        let back = r.thumb(&id).unwrap();
        assert_eq!((back.width, back.height), (2, 1));
        assert_eq!(back.bgra, t.bgra);
        assert_eq!(r.dir(&id), r.root().join(&id));
        r.delete(&id).unwrap();
        assert!(r.list().is_empty());
        let _ = fs::remove_dir_all(r.root());
    }

    fn scratch(name: &str) -> Recents {
        let dir = std::env::temp_dir().join(format!("spooch-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Recents::new(dir)
    }

    fn pixels(seed: u8) -> Pixels {
        let (width, height) = (8, 4);
        let bgra = (0..width * height)
            .flat_map(|i| [seed, i as u8, 7, 255])
            .collect();
        Pixels {
            width,
            height,
            bgra,
        }
    }

    #[test]
    fn synced_captures_stubs_and_pruning_back_to_stubs() {
        let r = scratch("recents-sync");
        let note = SyncNote {
            id: "s1".into(),
            version: 3,
            stamp: 0,
        };
        // A stub: listed for the gallery, not for reopening.
        let stub = r.new_id(&T);
        r.write_stub(&stub, b"thumb", &note).unwrap();
        assert!(r.is_stub(&stub));
        assert_eq!(r.list(), Vec::<String>::new());
        assert_eq!(r.list_all(), vec![stub.clone()]);
        assert_eq!(r.sync_note(&stub), Some(note.clone()));
        // Downloaded, it's a capture like any, stamped as that version.
        r.write_synced(&stub, (b"{}", b"png", b"thumb2"), &note)
            .unwrap();
        assert!(!r.is_stub(&stub));
        assert_eq!(r.list(), vec![stub.clone()]);
        let synced = r.sync_note(&stub).unwrap();
        assert_eq!(synced.stamp, r.stamp(&stub).unwrap());
        assert_eq!(r.files(&stub).unwrap().2, b"thumb2");
        // Pruned, a synced capture goes back to a stub; an unsynced one goes.
        let local = r.new_id(&LocalTime { second: 50, ..T });
        r.write(&local, "{}", Some(&pixels(1))).unwrap();
        let later = SystemTime::now() + KEEP_FOR * 2;
        assert_eq!(r.prune(later, KEEP_FOR, KEEP_BYTES, ""), 2);
        assert_eq!(r.list_all(), vec![stub.clone()]);
        assert!(r.is_stub(&stub));
        // The cursor starts at 0.
        assert_eq!(r.sync_cursor(), 0);
        r.set_sync_cursor(42).unwrap();
        assert_eq!(r.sync_cursor(), 42);
    }

    #[test]
    fn writes_reads_and_lists_in_time_order() {
        let r = scratch("recents-rw");
        let a = r.new_id(&T);
        assert_eq!(a, "2026-10-03 23.10.42");
        r.write(&a, "{\"a\":1}", Some(&pixels(1))).unwrap();
        // The same second again gets a suffix and sorts after.
        let b = r.new_id(&T);
        assert_eq!(b, "2026-10-03 23.10.42 2");
        // Handed out but not kept yet is taken too.
        assert_eq!(r.new_id(&T), "2026-10-03 23.10.42 3");
        r.write(&b, "{\"b\":2}", Some(&pixels(2))).unwrap();
        let earlier = LocalTime { second: 41, ..T };
        let c = r.new_id(&earlier);
        r.write(&c, "{}", Some(&pixels(3))).unwrap();
        assert_eq!(r.list(), vec![c.clone(), a.clone(), b.clone()]);

        let kept = r.read(&a).unwrap();
        assert_eq!(kept.json, "{\"a\":1}");
        assert_eq!(kept.pixels.bgra, pixels(1).bgra);
        // Re-marking keeps the pixels.
        r.write(&a, "{\"a\":2}", None).unwrap();
        assert_eq!(r.read(&a).unwrap().pixels.bgra, pixels(1).bgra);

        assert_eq!(r.neighbour(&a, true), Some(c.clone()));
        assert_eq!(r.neighbour(&a, false), Some(b.clone()));
        assert_eq!(r.neighbour(&c, true), None);
        // An id not written yet counts as the newest.
        let fresh = r.new_id(&LocalTime { minute: 11, ..T });
        assert_eq!(r.neighbour(&fresh, true), Some(b.clone()));
        fs::remove_dir_all(r.root()).unwrap();
    }

    #[test]
    fn prunes_by_age_then_size_but_never_the_current_one() {
        let r = scratch("recents-prune");
        let ids: Vec<String> = (0..4)
            .map(|s| {
                let id = r.new_id(&LocalTime { second: s, ..T });
                r.write(&id, "{}", Some(&pixels(s))).unwrap();
                id
            })
            .collect();
        let now = SystemTime::now();
        // Nothing is old yet, and everything fits.
        assert_eq!(r.prune(now, KEEP_FOR, KEEP_BYTES, &ids[3]), 0);
        // A month from now everything is old, except the current one.
        let later = now + KEEP_FOR + Duration::from_secs(60);
        assert_eq!(r.prune(later, KEEP_FOR, KEEP_BYTES, &ids[3]), 3);
        assert_eq!(r.list(), vec![ids[3].clone()]);
        // A size cap removes the oldest first.
        let more: Vec<String> = (10..13)
            .map(|s| {
                let id = r.new_id(&LocalTime { second: s, ..T });
                r.write(&id, "{}", Some(&pixels(s))).unwrap();
                id
            })
            .collect();
        let one = folder_size(&r.root().join(&more[0]));
        assert_eq!(r.prune(now, KEEP_FOR, one * 2, &more[2]), 2);
        assert_eq!(r.list(), vec![more[1].clone(), more[2].clone()]);
        fs::remove_dir_all(r.root()).unwrap();
    }

    #[test]
    fn writing_without_pixels_needs_them_already_kept() {
        let r = scratch("recents-nopix");
        assert!(r.write("x", "{}", None).is_err());
        let _ = fs::remove_dir_all(r.root());
    }
}
