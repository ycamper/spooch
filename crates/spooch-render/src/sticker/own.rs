//! Your own stickers: the pictures in the stickers folder, which
//! `load_stickers` reads again whenever the folder changes.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use skia_safe::{Data, Image};

use super::{Art, BUILT_IN};

/// What your stickers' names start with, so they can't clash with the
/// built-in ones.
pub(super) const PREFIX: &str = "own:";
/// The most of yours: the grid's 9 by 9, so it stays its size.
pub(super) const MOST: usize = 81;
/// The biggest picture file read, in bytes.
const MOST_BYTES: u64 = 4 << 20;
/// The longest side a picture can have, in pixels.
const MOST_SIDE: i32 = 2048;
/// The pictures read, by extension.
const KINDS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];

const README: &str = "\
Pictures in this folder show up in Spooch's sticker picker, in the Yours
tab: PNG, JPEG, GIF, WebP or BMP, up to 4 MB and 2048 pixels a side, and up
to 81 of them. Changes show as soon as they're saved.
";

/// One of your pictures: its file, its size, and a number that changes
/// when the file does.
#[derive(Clone)]
pub(super) struct Picture {
    pub bytes: Arc<[u8]>,
    pub size: (i32, i32),
    pub version: u64,
}

struct Own {
    /// Every sticker: the built-in ones, then yours in the order they first
    /// turned up. It only grows, since recent stickers are indexes into it;
    /// one whose file has gone keeps its place, without a picture.
    list: &'static [(&'static str, Art)],
    pictures: BTreeMap<&'static str, Picture>,
}

static OWN: RwLock<Own> = RwLock::new(Own {
    list: BUILT_IN,
    pictures: BTreeMap::new(),
});
/// Goes up whenever your pictures change.
static VERSION: AtomicU64 = AtomicU64::new(1);

fn read() -> RwLockReadGuard<'static, Own> {
    OWN.read().unwrap_or_else(PoisonError::into_inner)
}

fn write() -> RwLockWriteGuard<'static, Own> {
    OWN.write().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn list() -> &'static [(&'static str, Art)] {
    read().list
}

/// Your sticker `name`'s picture, if its file is there.
pub(super) fn picture(name: &str) -> Option<Picture> {
    read().pictures.get(name).cloned()
}

/// A number that changes whenever your pictures do.
pub(crate) fn version() -> u64 {
    VERSION.load(Ordering::Relaxed)
}

/// The sticker called `name`: a built-in one, or one of yours, kept a place
/// while its file isn't there in case it comes back.
pub(super) fn named(name: &str) -> Option<&'static str> {
    let found = read().list.iter().map(|(n, _)| *n).find(|n| *n == name);
    match found {
        Some(n) => Some(n),
        None if name.starts_with(PREFIX) && name.len() <= 255 => write().entry(name),
        None => None,
    }
}

impl Own {
    /// `name`'s place in the list, making one if there's room.
    fn entry(&mut self, name: &str) -> Option<&'static str> {
        if let Some((n, _)) = self.list.iter().find(|(n, _)| *n == name) {
            return Some(n);
        }
        if self.list.len() >= BUILT_IN.len() + MOST {
            return None;
        }
        // Leaked, as the built-in names are static: each name once, and
        // the list only as it grows, so both are bounded by `MOST`.
        let name: &'static str = Box::leak(name.into());
        let mut list = self.list.to_vec();
        list.push((name, Art::Picture));
        self.list = Box::leak(list.into_boxed_slice());
        Some(name)
    }
}

/// Load the pictures in `dir` as your stickers, replacing any loaded from
/// it before, and write a README there if it has none. Returns a line per
/// file that couldn't be used, and why.
pub fn load_stickers(dir: &Path) -> Vec<String> {
    if let Err(e) = std::fs::create_dir_all(dir) {
        return vec![format!("couldn't make {}: {e}", dir.display())];
    }
    let readme = dir.join("README.txt");
    if !readme.exists() {
        let _ = std::fs::write(&readme, README);
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .is_some_and(|x| KINDS.iter().any(|k| x.eq_ignore_ascii_case(k)))
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    let mut problems = Vec::new();
    let mut found = Vec::new();
    for path in files {
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if found.len() == MOST {
            problems.push(format!("stickers: only the first {MOST} are used"));
            break;
        }
        match read_picture(&path) {
            Ok((bytes, size)) => found.push((format!("{PREFIX}{file}"), bytes, size)),
            Err(e) => problems.push(format!("sticker {file}: {e}")),
        }
    }
    let mut own = write();
    let mut pictures = BTreeMap::new();
    for (name, bytes, size) in found {
        let Some(name) = own.entry(&name) else {
            problems.push(format!(
                "sticker {}: no room for more until Spooch restarts",
                &name[PREFIX.len()..]
            ));
            continue;
        };
        let version = match own.pictures.get(name) {
            Some(p) if *p.bytes == *bytes => p.version,
            _ => VERSION.fetch_add(1, Ordering::Relaxed) + 1,
        };
        let bytes = bytes.into();
        pictures.insert(
            name,
            Picture {
                bytes,
                size,
                version,
            },
        );
    }
    let same = |a: &BTreeMap<&str, Picture>, b: &BTreeMap<&str, Picture>| {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|((n, p), (m, q))| n == m && p.version == q.version)
    };
    if !same(&own.pictures, &pictures) {
        VERSION.fetch_add(1, Ordering::Relaxed);
    }
    own.pictures = pictures;
    problems
}

/// The picture file at `path`, and its size, if Spooch can read it and
/// it's not too big.
fn read_picture(path: &Path) -> Result<(Vec<u8>, (i32, i32)), String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(MOST_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MOST_BYTES {
        return Err(format!("bigger than {} MB", MOST_BYTES >> 20));
    }
    let image = Image::from_encoded(Data::new_copy(&bytes))
        .ok_or_else(|| "not a picture Spooch can read".to_string())?;
    let (w, h) = (image.width(), image.height());
    if !(1..=MOST_SIDE).contains(&w) || !(1..=MOST_SIDE).contains(&h) {
        return Err(format!("bigger than {MOST_SIDE} pixels a side"));
    }
    Ok((bytes, (w, h)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `w` by `h` red PNG.
    fn png(w: i32, h: i32) -> Vec<u8> {
        let mut s = skia_safe::surfaces::raster_n32_premul((w, h)).unwrap();
        s.canvas()
            .clear(skia_safe::Color4f::new(1.0, 0.0, 0.0, 1.0));
        let img = s.image_snapshot();
        let data = img
            .encode(None, skia_safe::EncodedImageFormat::PNG, None)
            .unwrap();
        data.as_bytes().to_vec()
    }

    #[test]
    fn the_folder_s_pictures_are_stickers_while_they_re_there() {
        let dir = std::env::temp_dir().join(format!("spooch-own-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("rage.png"), png(40, 20)).unwrap();
        std::fs::write(dir.join("broken.png"), b"not a png").unwrap();
        std::fs::write(dir.join("notes.txt"), b"ignored").unwrap();
        let problems = load_stickers(&dir);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("broken.png"));
        assert!(dir.join("README.txt").exists());

        let name = super::super::sticker_named("own:rage.png").expect("loaded");
        assert_eq!(super::super::art_of(name), Some(Art::Picture));
        assert_eq!(super::super::sticker_title(name), "rage");
        assert_eq!(super::super::sticker_aspect(name), 2.0);
        let img = super::super::sticker_image(name).expect("decoded");
        assert_eq!((img.width(), img.height()), (40, 20));

        // Changed, it's drawn anew; gone, it keeps its place without a
        // picture.
        let before = version();
        std::fs::write(dir.join("rage.png"), png(20, 20)).unwrap();
        load_stickers(&dir);
        assert!(version() > before);
        assert_eq!(super::super::sticker_image(name).unwrap().width(), 20);
        std::fs::remove_file(dir.join("rage.png")).unwrap();
        load_stickers(&dir);
        assert!(picture(name).is_none());
        assert_eq!(super::super::sticker_named("own:rage.png"), Some(name));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn huge_pictures_are_turned_down() {
        let dir = std::env::temp_dir().join(format!("spooch-huge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wide.png");
        std::fs::write(&path, png(MOST_SIDE + 1, 1)).unwrap();
        assert!(read_picture(&path).unwrap_err().contains("pixels"));
        std::fs::write(&path, vec![0u8; MOST_BYTES as usize + 1]).unwrap();
        assert!(read_picture(&path).unwrap_err().contains("MB"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
