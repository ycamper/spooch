//! What Spooch needs around the editor that isn't tied to one OS:
//! settings, Recents, save file naming, the log, PNG encoding and the
//! shared app flow (tray menu, shortcuts, countdown).

pub mod app;
pub mod log;
pub mod recents;
pub mod save;
pub mod settings;

use std::path::Path;
use std::{fs, io};

/// Writes `path` through a temporary file, so readers never see half of it.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

/// Straight (not premultiplied), tightly packed BGRA as a PNG with its
/// alpha. Fast compression: this is on the hot path between release and
/// paste.
pub fn encode_png(width: u32, height: u32, bgra: &[u8]) -> Result<Vec<u8>, String> {
    let mut rgba = Vec::with_capacity(bgra.len());
    for px in bgra.as_chunks::<4>().0 {
        rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(out)
}
