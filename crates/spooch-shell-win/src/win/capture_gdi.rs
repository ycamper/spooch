//! GDI BitBlt capture: the old, universally available baseline. It is the
//! fallback when desktop duplication is unavailable, and needs no GPU device.
//!
//! It misses protected content and has no HDR support. Monitors are
//! captured in parallel, one thread each.

use std::time::Instant;

use super::capture::{CaptureBackend, CapturedFrame, Display, Image};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap,
    CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC,
    SRCCOPY, SelectObject,
};

use super::monitors::Monitor;

pub struct GdiCapture {
    pub monitors: Vec<Monitor>,
}

impl CaptureBackend for GdiCapture {
    fn name(&self) -> &'static str {
        "gdi-bitblt"
    }

    fn capture_all(&mut self) -> Result<Vec<CapturedFrame>, String> {
        // Monitor handles are not Send; the threads only need bounds and metadata.
        let jobs: Vec<(RECT, Display)> = self
            .monitors
            .iter()
            .map(|m| (m.px, m.display.clone()))
            .collect();
        std::thread::scope(|s| {
            let handles: Vec<_> = jobs
                .into_iter()
                .map(|(px, display)| s.spawn(move || capture_monitor(px, display)))
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join()
                        .unwrap_or_else(|_| Err("capture thread panicked".to_string()))
                })
                .collect()
        })
    }
}

/// Blit the screen into a device bitmap, then read it straight into one
/// top-down buffer: a single host-memory copy per monitor.
fn capture_monitor(px: RECT, display: Display) -> Result<CapturedFrame, String> {
    let started = Instant::now();
    let (w, h) = (px.right - px.left, px.bottom - px.top);
    let mut data = vec![0u8; w as usize * h as usize * 4];
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bitmap.into());
        let blit = BitBlt(
            mem,
            0,
            0,
            w,
            h,
            Some(screen),
            px.left,
            px.top,
            SRCCOPY | CAPTUREBLT,
        );
        // GetDIBits needs the bitmap deselected.
        SelectObject(mem, old);

        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let rows = GetDIBits(
            mem,
            bitmap,
            0,
            h as u32,
            Some(data.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        );

        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        blit.map_err(|e| format!("BitBlt: {e}"))?;
        if rows != h {
            return Err(format!("GetDIBits returned {rows} of {h} rows"));
        }
    }

    // GDI leaves alpha undefined.
    for px in data.as_chunks_mut::<4>().0 {
        px[3] = 255;
    }

    Ok(CapturedFrame {
        display,
        image: Image {
            width: w as u32,
            height: h as u32,
            stride: w as usize * 4,
            data,
        },
        elapsed: started.elapsed(),
    })
}
