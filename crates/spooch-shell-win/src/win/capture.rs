//! What a capture backend hands back: one BGRA image per monitor.

use std::fmt;
use std::time::Duration;

use spooch_render::skia_safe::IRect;

/// A BGRA bitmap. Rows may be padded past `width * 4`.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub data: Vec<u8>,
}

impl Image {
    pub fn row(&self, y: u32) -> &[u8] {
        let start = y as usize * self.stride;
        &self.data[start..start + self.width as usize * 4]
    }

    /// Area `r`'s rows, if it's inside.
    pub fn rows_in(&self, r: IRect) -> Option<impl Iterator<Item = &[u8]>> {
        let x = u32::try_from(r.left).ok()?;
        let y = u32::try_from(r.top).ok()?;
        let (w, h) = (r.width() as u32, r.height() as u32);
        if r.is_empty() || x + w > self.width || y + h > self.height {
            return None;
        }
        let cols = x as usize * 4..(x + w) as usize * 4;
        Some((y..y + h).map(move |row| &self.row(row)[cols.clone()]))
    }
}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Image")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("stride", &self.stride)
            .finish_non_exhaustive()
    }
}

/// A monitor as the log and the overlay see it.
#[derive(Clone, Debug, PartialEq)]
pub struct Display {
    /// The device name, like `\\.\DISPLAY1`.
    pub name: String,
    /// Physical pixels per logical one: 1.5 at 150%.
    pub scale: f64,
    pub is_primary: bool,
}

/// One monitor's pixels, frozen at the moment of capture.
#[derive(Debug)]
pub struct CapturedFrame {
    pub display: Display,
    pub image: Image,
    /// How long the backend took.
    pub elapsed: Duration,
}

/// Freezes every monitor at once, for the overlay to show.
pub trait CaptureBackend {
    fn name(&self) -> &'static str;
    fn capture_all(&mut self) -> Result<Vec<CapturedFrame>, String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker(width: u32, height: u32, stride: usize) -> Image {
        let mut data = vec![0u8; stride * height as usize];
        for y in 0..height {
            for x in 0..width {
                let i = y as usize * stride + x as usize * 4;
                data[i] = x as u8;
                data[i + 1] = y as u8;
                data[i + 3] = 255;
            }
        }
        Image {
            width,
            height,
            stride,
            data,
        }
    }

    #[test]
    fn rows_in_handles_padded_rows() {
        let img = checker(8, 6, 8 * 4 + 12);
        let rows: Vec<&[u8]> = img.rows_in(IRect::from_xywh(2, 3, 4, 2)).unwrap().collect();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.len() == 16));
        assert_eq!(&rows[0][..4], &[2, 3, 0, 255]);
        assert_eq!(&rows[1][12..16], &[5, 4, 0, 255]);
    }

    #[test]
    fn rows_in_rejects_out_of_bounds() {
        let img = checker(8, 6, 32);
        assert!(img.rows_in(IRect::from_xywh(6, 0, 4, 2)).is_none());
        assert!(img.rows_in(IRect::from_xywh(-1, 0, 4, 2)).is_none());
        assert!(img.rows_in(IRect::from_xywh(0, 0, 0, 2)).is_none());
    }
}
