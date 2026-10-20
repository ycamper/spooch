//! Put an image on the clipboard in the two formats apps actually read:
//! a registered "PNG" format (Slack, browsers, Office) and CF_DIBV5 with
//! alpha (everything older). Text goes as CF_UNICODETEXT.

use super::capture::Image;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::Graphics::Gdi::{BI_BITFIELDS, BITMAPV5HEADER};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW,
    SetClipboardData,
};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::Win32::System::Ole::{CF_DIBV5, CF_UNICODETEXT};
use windows::core::w;

/// 'sRGB' as a little-endian four-character code.
const LCS_SRGB: u32 = 0x7352_4742;
const LCS_GM_IMAGES: u32 = 4;

/// `png` is `image` already encoded as a PNG.
pub fn set_image(owner: HWND, image: &Image, png: &[u8]) -> Result<(), String> {
    let dib = encode_dibv5(image);

    unsafe {
        OpenClipboard(Some(owner)).map_err(|e| format!("OpenClipboard: {e}"))?;
        let result = (|| {
            EmptyClipboard().map_err(|e| format!("EmptyClipboard: {e}"))?;
            let png_format = RegisterClipboardFormatW(w!("PNG"));
            put(png_format, png)?;
            put(u32::from(CF_DIBV5.0), &dib)?;
            Ok(())
        })();
        let _ = CloseClipboard();
        result
    }
}

/// Put `text` on the clipboard.
pub fn set_text(owner: HWND, text: &str) -> Result<(), String> {
    let utf16: Vec<u8> = text
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    unsafe {
        OpenClipboard(Some(owner)).map_err(|e| format!("OpenClipboard: {e}"))?;
        let result = EmptyClipboard()
            .map_err(|e| format!("EmptyClipboard: {e}"))
            .and_then(|()| put(u32::from(CF_UNICODETEXT.0), &utf16));
        let _ = CloseClipboard();
        result
    }
}

/// The clipboard's text, if it holds some.
pub fn text(owner: HWND) -> Option<String> {
    unsafe {
        OpenClipboard(Some(owner)).ok()?;
        let text = (|| {
            let mem = HGLOBAL(GetClipboardData(u32::from(CF_UNICODETEXT.0)).ok()?.0);
            let ptr = GlobalLock(mem) as *const u16;
            if ptr.is_null() {
                return None;
            }
            // Up to the nul, and never past the block's end.
            let units = std::slice::from_raw_parts(ptr, GlobalSize(mem) / 2);
            let len = units.iter().position(|&u| u == 0).unwrap_or(units.len());
            let text = String::from_utf16_lossy(&units[..len]);
            let _ = GlobalUnlock(mem);
            Some(text)
        })();
        let _ = CloseClipboard();
        text
    }
}

/// Empty the clipboard.
pub fn clear(owner: HWND) {
    unsafe {
        if OpenClipboard(Some(owner)).is_ok() {
            let _ = EmptyClipboard();
            let _ = CloseClipboard();
        }
    }
}

/// Copy bytes into a movable global block and hand it to the clipboard,
/// which takes ownership on success.
unsafe fn put(format: u32, bytes: &[u8]) -> Result<(), String> {
    unsafe {
        let mem: HGLOBAL =
            GlobalAlloc(GMEM_MOVEABLE, bytes.len()).map_err(|e| format!("GlobalAlloc: {e}"))?;
        let ptr = GlobalLock(mem) as *mut u8;
        if ptr.is_null() {
            let _ = GlobalFree(Some(mem));
            return Err("GlobalLock failed".into());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        let _ = GlobalUnlock(mem);
        if let Err(e) = SetClipboardData(format, Some(HANDLE(mem.0))) {
            let _ = GlobalFree(Some(mem));
            return Err(format!("SetClipboardData: {e}"));
        }
        Ok(())
    }
}

/// A bottom-up 32-bit BITMAPV5 with an explicit alpha mask, which is the
/// DIB layout the widest range of Windows apps paste correctly.
fn encode_dibv5(image: &Image) -> Vec<u8> {
    let row_bytes = image.width as usize * 4;
    let pixels = row_bytes * image.height as usize;
    let header = BITMAPV5HEADER {
        bV5Size: size_of::<BITMAPV5HEADER>() as u32,
        bV5Width: image.width as i32,
        bV5Height: image.height as i32,
        bV5Planes: 1,
        bV5BitCount: 32,
        bV5Compression: BI_BITFIELDS,
        bV5SizeImage: pixels as u32,
        bV5RedMask: 0x00FF_0000,
        bV5GreenMask: 0x0000_FF00,
        bV5BlueMask: 0x0000_00FF,
        bV5AlphaMask: 0xFF00_0000,
        bV5CSType: LCS_SRGB,
        bV5Intent: LCS_GM_IMAGES,
        ..Default::default()
    };
    let header_bytes = unsafe {
        std::slice::from_raw_parts(
            &header as *const _ as *const u8,
            size_of::<BITMAPV5HEADER>(),
        )
    };
    let mut out = Vec::with_capacity(header_bytes.len() + pixels);
    out.extend_from_slice(header_bytes);
    for y in (0..image.height).rev() {
        out.extend_from_slice(image.row(y));
    }
    out
}
