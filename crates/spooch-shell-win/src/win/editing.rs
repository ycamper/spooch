//! What the overlay and the editor window share on Windows: the key and
//! cursor mapping, the shell's drag loop, blitting the core's pixels and
//! delivering a finished image to the clipboard and Pictures.

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use super::capture::Image;
use spooch_app::{Clip, Deliverable};
use spooch_render::{Cursor, Key, Mods};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS, HBITMAP, HDC,
    SetDIBitsToDevice,
};
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::Ole::{DROPEFFECT_COPY, DROPEFFECT_NONE};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, MAPVK_VK_TO_CHAR, MapVirtualKeyW, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END,
    VK_ESCAPE, VK_HOME, VK_LEFT, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_UP,
};
use windows::Win32::UI::Shell::{
    BHID_DataObject, IShellItem, SHCreateItemFromParsingName, SHDoDragDrop,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, IDC_ARROW, IDC_CROSS, IDC_HAND, IDC_IBEAM, IDC_SIZEALL, IDC_SIZENESW,
    IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE, PostMessageW, SM_CXDRAG, SM_CYDRAG, WM_MOUSEHWHEEL,
};
use windows::core::PCWSTR;

use super::log::{self, Stopwatch};

/// Where drag-out files are written.
pub(super) fn drag_dir() -> PathBuf {
    std::env::temp_dir().join("Spooch").join("drag")
}

/// Put a finished image on the clipboard, and save it to Pictures when it
/// carries a file. `owner` owns the clipboard data; `sw` times each step.
pub(super) fn deliver(owner: HWND, d: Deliverable, sw: &mut Stopwatch) {
    let copy = |c: Clip| {
        let img = Image {
            width: c.width,
            height: c.height,
            stride: c.width as usize * 4,
            data: c.bgra,
        };
        super::clipboard::set_image(owner, &img, &c.png)
    };
    d.deliver(copy, super::save::folder, &mut |line| sw.mark(line));
}

/// Put `text` on the clipboard (`owner` owns it).
pub(super) fn copy_text(owner: HWND, text: &str) {
    match super::clipboard::set_text(owner, text) {
        Ok(()) => log::line("share link on the clipboard"),
        Err(e) => log::line(format!("clipboard failed: {e}")),
    }
}

/// What a background thread calls for a session's news: posts `msg` to
/// `hwnd`, whose window procedure calls `Session::poll`.
pub(super) fn waker(hwnd: HWND, msg: u32) -> impl Fn() + Send + Sync + 'static {
    let raw = hwnd.0 as isize;
    move || unsafe {
        let _ = PostMessageW(Some(HWND(raw as *mut _)), msg, WPARAM(0), LPARAM(0));
    }
}

/// The editor key for virtual key `vk`: a named key, or the character it
/// types without Shift on the current keyboard layout.
pub(super) fn editor_key(vk: u16) -> Option<Key> {
    Some(match vk {
        k if k == VK_RETURN.0 => Key::Enter,
        k if k == VK_ESCAPE.0 => Key::Escape,
        k if k == VK_BACK.0 => Key::Backspace,
        k if k == VK_DELETE.0 => Key::Delete,
        k if k == VK_LEFT.0 => Key::Left,
        k if k == VK_RIGHT.0 => Key::Right,
        k if k == VK_UP.0 => Key::Up,
        k if k == VK_DOWN.0 => Key::Down,
        k if k == VK_HOME.0 => Key::Home,
        k if k == VK_END.0 => Key::End,
        k if k == VK_PRIOR.0 => Key::PageUp,
        k if k == VK_NEXT.0 => Key::PageDown,
        _ => {
            // The top bit marks a dead key.
            let c = unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_CHAR) } & 0x7fff_ffff;
            Key::Char(
                char::from_u32(c)
                    .filter(|c| *c != '\0')?
                    .to_ascii_lowercase(),
            )
        }
    })
}

/// How far the pointer goes before a press becomes a drag, from the
/// system's settings.
pub(super) fn drag_slop() -> (f32, f32) {
    unsafe {
        (
            GetSystemMetrics(SM_CXDRAG) as f32,
            GetSystemMetrics(SM_CYDRAG) as f32,
        )
    }
}

/// A WM_MOUSEWHEEL or WM_MOUSEHWHEEL, as `Session::wheel` takes it.
pub(super) struct Wheel {
    /// Away from the user, or for a tilt, to the right.
    pub notches: f32,
    /// A tilt, or Shift held.
    pub sideways: bool,
    pub ctrl: bool,
}

impl Wheel {
    pub(super) fn from(msg: u32, wparam: WPARAM) -> Wheel {
        const NOTCH: f32 = 120.0;
        const MK_SHIFT: usize = 0x0004;
        const MK_CONTROL: usize = 0x0008;
        let delta = ((wparam.0 >> 16) & 0xFFFF) as i16 as f32 / NOTCH;
        let keys = wparam.0 & 0xFFFF;
        let tilt = msg == WM_MOUSEHWHEEL;
        Wheel {
            // A tilt to the right scrolls right.
            notches: if tilt { -delta } else { delta },
            sideways: tilt || keys & MK_SHIFT != 0,
            ctrl: keys & MK_CONTROL != 0,
        }
    }
}

/// The modifier keys held now.
pub(super) fn mods_now() -> Mods {
    unsafe {
        Mods {
            ctrl: GetKeyState(VK_CONTROL.0 as i32) < 0,
            shift: GetKeyState(VK_SHIFT.0 as i32) < 0,
            alt: GetKeyState(VK_MENU.0 as i32) < 0,
        }
    }
}

pub(super) fn cursor_id(c: Cursor) -> PCWSTR {
    match c {
        Cursor::Arrow => IDC_ARROW,
        Cursor::Draw => IDC_CROSS,
        Cursor::Text => IDC_IBEAM,
        Cursor::Move => IDC_SIZEALL,
        Cursor::ResizeHorizontal => IDC_SIZEWE,
        Cursor::ResizeVertical => IDC_SIZENS,
        Cursor::ResizeDiagonalDown => IDC_SIZENWSE,
        Cursor::ResizeDiagonalUp => IDC_SIZENESW,
        Cursor::Hand => IDC_HAND,
    }
}

/// Run the shell's drag loop for one file. Returns whether it was dropped.
pub(super) fn shell_drag(hwnd: HWND, path: &Path) -> windows::core::Result<bool> {
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None)?;
        let data: IDataObject = item.BindToHandler(None, &BHID_DataObject)?;
        let effect = SHDoDragDrop(Some(hwnd), &data, None, DROPEFFECT_COPY)?;
        Ok(effect != DROPEFFECT_NONE)
    }
}

/// Copy a `size` block from `src` in a top-down BGRA buffer with the given
/// row stride to `dest` on `dc`. The rows are handed to GDI as their own
/// top-down bitmap so the origin rules stay simple.
pub(super) fn blit_from(
    dc: HDC,
    data: &[u8],
    stride: usize,
    src: (i32, i32),
    dest: (i32, i32),
    size: (i32, i32),
) {
    let (w, h) = size;
    if w <= 0 || h <= 0 || src.0 < 0 || src.1 < 0 || !stride.is_multiple_of(4) {
        return;
    }
    let start = src.1 as usize * stride;
    let Some(bits) = data.get(start..start + h as usize * stride) else {
        return;
    };
    let info = top_down_info((stride / 4) as i32, h);
    unsafe {
        SetDIBitsToDevice(
            dc,
            dest.0,
            dest.1,
            w as u32,
            h as u32,
            src.0,
            0,
            0,
            h as u32,
            bits.as_ptr().cast(),
            &info,
            DIB_RGB_COLORS,
        );
    }
}

/// The header of a top-down 32-bit BGRA bitmap, `w` by `h`.
pub(super) fn top_down_info(w: i32, h: i32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// A DIB section holding `pixels`, BGRA `w` by `h`; `None` when the length
/// doesn't match or GDI refuses. The caller deletes it.
pub(super) fn dib_with(dc: Option<HDC>, (w, h): (i32, i32), pixels: &[u8]) -> Option<HBITMAP> {
    if w <= 0 || h <= 0 || pixels.len() != w as usize * h as usize * 4 {
        return None;
    }
    let mut bits = std::ptr::null_mut();
    // SAFETY: for a 32-bit top-down DIB, CreateDIBSection points `bits` at
    // w * h * 4 writable bytes, which is `pixels.len()`.
    unsafe {
        let dib =
            CreateDIBSection(dc, &top_down_info(w, h), DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len());
        Some(dib)
    }
}
