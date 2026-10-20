//! The finished image: for the clipboard, a file, or a drag-out.

use std::io;
use std::path::{Path, PathBuf};

use spooch_render::{Counts, Format};

use super::Session;

/// The finished image, ready for the clipboard and a file.
pub struct Deliverable {
    pub width: u32,
    pub height: u32,
    /// Straight BGRA, tightly packed; transparent round a frame.
    pub bgra: Vec<u8>,
    /// The PNG, for the clipboard (and the file, when saving as PNG).
    pub png: Vec<u8>,
    /// When saving: the file's stem and extension, and its bytes in the
    /// format picked in the bar.
    pub file: Option<(String, &'static str, Vec<u8>)>,
    pub counts: Counts,
}

impl Deliverable {
    /// Encode straight BGRA pixels for delivery, saving in `format` too
    /// when `save`.
    pub fn new(
        (width, height, bgra): (u32, u32, Vec<u8>),
        format: Format,
        stem: &str,
        save: bool,
        counts: Counts,
    ) -> Result<Deliverable, String> {
        let png = spooch_platform::encode_png(width, height, &bgra)?;
        let file = match (save, format) {
            (false, _) => None,
            (true, Format::Png) => Some((stem.to_string(), format.extension(), png.clone())),
            (true, f) => Some((
                stem.to_string(),
                f.extension(),
                encode(f, width, height, &bgra)?,
            )),
        };
        Ok(Deliverable {
            width,
            height,
            bgra,
            png,
            file,
            counts,
        })
    }

    /// Put the image on the clipboard with `copy`, then, when it carries a
    /// file, save it into `folder` without overwriting anything. `mark`
    /// gets each step for the log. Every shell delivers this way.
    pub fn deliver(
        self,
        copy: impl FnOnce(Clip) -> Result<(), String>,
        folder: impl FnOnce() -> io::Result<PathBuf>,
        mark: &mut dyn FnMut(String),
    ) {
        mark(format!("png {} KB", self.png.len() / 1024));
        let (w, h, counts) = (self.width, self.height, self.counts);
        let clip = Clip {
            width: self.width,
            height: self.height,
            bgra: self.bgra,
            png: self.png,
        };
        match copy(clip) {
            Ok(()) => mark(format!("copied {w}x{h} with {counts} to the clipboard")),
            Err(e) => mark(format!("clipboard failed: {e}")),
        }
        if let Some((stem, ext, bytes)) = self.file {
            let saved =
                folder().and_then(|dir| spooch_platform::save::write_new(&dir, &stem, ext, &bytes));
            match saved {
                Ok(path) => {
                    spooch_platform::save::remember(&path);
                    mark(format!("saved {}", path.display()))
                }
                Err(e) => mark(format!("save failed: {e}")),
            }
        }
    }
}

/// What goes on the clipboard: the image as straight BGRA, tightly
/// packed, and as PNG. Each shell puts up what its clipboard takes.
pub struct Clip {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    pub png: Vec<u8>,
}

/// Straight BGRA pixels in `format`.
pub fn encode(format: Format, width: u32, height: u32, bgra: &[u8]) -> Result<Vec<u8>, String> {
    match format {
        Format::Png => spooch_platform::encode_png(width, height, bgra),
        Format::Jpeg => spooch_render::encode_jpeg(width, height, bgra, 90)
            .ok_or_else(|| "JPEG encoding failed".to_string()),
    }
}

impl Session {
    /// The finished image, framed: `(width, height, straight BGRA)`. Cheap
    /// next to encoding, so a shell can close first and encode after.
    pub fn export(&mut self) -> (u32, u32, Vec<u8>) {
        let out = self.doc.export_image();
        (out.width, out.height, out.bgra)
    }

    /// The finished image, encoded for the clipboard, and for a file when
    /// `save`.
    pub fn deliverable(&mut self, save: bool) -> Result<Deliverable, String> {
        Deliverable::new(
            self.export(),
            self.doc.format(),
            &self.stem,
            save,
            self.doc.counts(),
        )
    }

    /// Write the finished image into `dir` for a drag-out, under its file
    /// name. Returns the file and its bytes, for drag protocols that hand
    /// over the data as well.
    pub fn drag_file(&mut self, dir: &Path) -> Result<(PathBuf, Vec<u8>), String> {
        // A card dragged out of the gallery: that capture, as it was kept.
        if let Some(id) = self.drag_recent.take() {
            let recents = self.recents.as_ref().ok_or("Recents are off")?;
            let mut kept = Session::open(recents, &id, Vec::new(), self.doc.styles(), self.host)?;
            return kept.drag_file(dir);
        }
        let out = self.doc.export_image();
        let bytes = encode(self.doc.format(), out.width, out.height, &out.bgra)?;
        let path = spooch_platform::save::write_drag_file(dir, &self.file_name(), &bytes)
            .map_err(|e| e.to_string())?;
        Ok((path, bytes))
    }

    /// The name it's saved and dragged out under.
    fn file_name(&self) -> String {
        format!("{}.{}", self.stem, self.doc.format().extension())
    }
}
