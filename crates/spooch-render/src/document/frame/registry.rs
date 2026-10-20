//! The frames Spooch knows: none, the built-ins, and whatever's in the
//! frames folder. Documents refer to a frame by a small id (`Frame`), kept
//! stable for the life of the process, so the tool styles holding it stay
//! `Copy`; saved documents use its key ("browser-tab", "file:polaroid").

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use skia_safe::{Data, Image};

use super::spec::{FrameSpec, Item, README};

/// The built-in frames, by key, in the order the list shows them.
pub(super) const BUILTIN: [(&str, &str); 15] = [
    (
        "terminal-classic",
        include_str!("builtin/terminal-classic.toml"),
    ),
    (
        "terminal-ascii",
        include_str!("builtin/terminal-ascii.toml"),
    ),
    (
        "terminal-modern",
        include_str!("builtin/terminal-modern.toml"),
    ),
    (
        "terminal-windows",
        include_str!("builtin/terminal-windows.toml"),
    ),
    ("window-mac", include_str!("builtin/window-mac.toml")),
    (
        "window-windows",
        include_str!("builtin/window-windows.toml"),
    ),
    ("window-linux", include_str!("builtin/window-linux.toml")),
    (
        "window-minimal",
        include_str!("builtin/window-minimal.toml"),
    ),
    ("browser-light", include_str!("builtin/browser-light.toml")),
    ("browser-dark", include_str!("builtin/browser-dark.toml")),
    (
        "browser-compact",
        include_str!("builtin/browser-compact.toml"),
    ),
    ("browser-mac", include_str!("builtin/browser-mac.toml")),
    ("tiled", include_str!("builtin/tiled.toml")),
    ("retro-dos", include_str!("builtin/retro-dos.toml")),
    ("rounded", include_str!("builtin/rounded.toml")),
];

/// Frames that were built in once, by key, and what a document saved
/// with one gets now.
const RETIRED: [(&str, &str); 7] = [
    ("browser-address", "browser-light"),
    ("browser-buttons", "browser-mac"),
    ("browser-tab", "browser-light"),
    ("browser-quiet", "browser-mac"),
    ("terminal-prompt", "terminal-modern"),
    ("terminal-buttons", "terminal-classic"),
    ("terminal-bare", "terminal-modern"),
];

/// The key of "no frame", always id 0.
pub(super) const NONE_KEY: &str = "none";

/// A frame ready to draw: its spec and the pictures it uses, decoded.
pub struct Loaded {
    pub spec: FrameSpec,
    pub images: HashMap<String, Image>,
}

struct Registry {
    /// Every key ever seen; a frame's id is its index. Only grows, so ids
    /// held by documents never change meaning.
    keys: Vec<String>,
    frames: HashMap<String, Rc<Loaded>>,
    /// The keys the Frame list shows, in order.
    order: Vec<String>,
}

impl Registry {
    fn new() -> Registry {
        let mut r = Registry {
            keys: vec![NONE_KEY.to_string()],
            frames: HashMap::new(),
            order: vec![NONE_KEY.to_string()],
        };
        for (key, text) in BUILTIN {
            // The built-ins are tested to parse; skip one rather than panic.
            if let Ok(spec) = FrameSpec::parse(text) {
                r.add(key, spec, HashMap::new());
            }
        }
        r
    }

    fn id(&mut self, key: &str) -> u16 {
        if let Some(i) = self.keys.iter().position(|k| k == key) {
            return i as u16;
        }
        self.keys.push(key.to_string());
        (self.keys.len() - 1) as u16
    }

    fn add(&mut self, key: &str, spec: FrameSpec, images: HashMap<String, Image>) {
        self.id(key);
        self.frames
            .insert(key.to_string(), Rc::new(Loaded { spec, images }));
        if !self.order.iter().any(|k| k == key) {
            self.order.push(key.to_string());
        }
    }
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::new());
}

/// The frame with id `id`, unless it's "none" or no longer loaded.
pub(super) fn get(id: u16) -> Option<Rc<Loaded>> {
    REGISTRY.with(|r| {
        let r = r.borrow();
        let key = r.keys.get(usize::from(id))?;
        r.frames.get(key).cloned()
    })
}

pub(super) fn key_of(id: u16) -> String {
    REGISTRY.with(|r| {
        r.borrow()
            .keys
            .get(usize::from(id))
            .cloned()
            .unwrap_or_else(|| NONE_KEY.to_string())
    })
}

/// The id for `key`, if a frame with that key is loaded (or it's "none").
pub(super) fn id_of(key: &str) -> Option<u16> {
    let key = RETIRED
        .iter()
        .find(|(old, _)| *old == key)
        .map_or(key, |(_, now)| now);
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        (key == NONE_KEY || r.frames.contains_key(key)).then(|| r.id(key))
    })
}

/// The Frame list: each frame's id and name, "No frame" first.
pub(super) fn listed() -> Vec<(u16, String)> {
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        let order = r.order.clone();
        order
            .iter()
            .map(|k| {
                let name = match r.frames.get(k) {
                    Some(f) => f.spec.name.clone(),
                    None => super::NO_FRAME.to_string(),
                };
                (r.id(k), name)
            })
            .collect()
    })
}

/// Load the frames in `dir` (`*.toml`), replacing any loaded from it
/// before, and write a README there if it has none. Returns a line per
/// file that couldn't be used, and why.
pub fn load_frames(dir: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    if let Err(e) = std::fs::create_dir_all(dir) {
        return vec![format!("couldn't make {}: {e}", dir.display())];
    }
    let readme = dir.join("README.txt");
    if !readme.exists() {
        let _ = std::fs::write(&readme, README);
    }
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .is_some_and(|x| x.eq_ignore_ascii_case("toml"))
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    let mut loaded = Vec::new();
    for path in files {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let shown = path
            .file_name()
            .map_or(stem.clone(), |n| n.to_string_lossy().into_owned());
        let result = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| FrameSpec::parse(&text))
            .and_then(|spec| {
                let images = load_images(dir, &spec)?;
                Ok((spec, images))
            });
        match result {
            Ok((spec, images)) => loaded.push((format!("file:{stem}"), spec, images)),
            Err(e) => problems.push(format!("frame {shown}: {e}")),
        }
    }
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        // Forget what this folder gave last time; keep the ids.
        r.order.retain(|k| !k.starts_with("file:"));
        r.frames.retain(|k, _| !k.starts_with("file:"));
        for (key, spec, images) in loaded {
            r.add(&key, spec, images);
        }
    });
    problems
}

/// Decode the pictures `spec` names, which must sit next to it.
fn load_images(dir: &Path, spec: &FrameSpec) -> Result<HashMap<String, Image>, String> {
    let mut names: Vec<&str> = Vec::new();
    if let Some(img) = &spec.image {
        names.push(&img.file);
    }
    for row in &spec.rows {
        for item in &row.items {
            if let Item::Image { file, .. } = item {
                names.push(file);
            }
        }
    }
    let mut images = HashMap::new();
    for name in names {
        // Only plain file names: frames can't reach outside their folder.
        if name.contains(['/', '\\']) || name.contains("..") {
            return Err(format!(
                "{name:?}: give just a file name in the frames folder"
            ));
        }
        let bytes = std::fs::read(dir.join(name)).map_err(|e| format!("{name}: {e}"))?;
        let image = Image::from_encoded(Data::new_copy(&bytes))
            .ok_or_else(|| format!("{name}: not an image Spooch can read"))?;
        images.insert(name.to_string(), image);
    }
    Ok(images)
}
