//! Monitor enumeration under Per-Monitor-V2 DPI awareness.
//!
//! With PMv2 every coordinate Windows hands us is in physical pixels on the
//! virtual desktop, so monitor rectangles are exact pixel rectangles.

use super::capture::Display;
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::core::BOOL;

const MONITORINFOF_PRIMARY: u32 = 1;

/// One monitor with its native handle and physical-pixel bounds.
#[derive(Clone, Debug)]
pub struct Monitor {
    pub handle: HMONITOR,
    /// Bounds on the virtual desktop, in physical pixels.
    pub px: RECT,
    pub display: Display,
}

impl Monitor {
    pub fn width(&self) -> u32 {
        (self.px.right - self.px.left) as u32
    }

    pub fn height(&self) -> u32 {
        (self.px.bottom - self.px.top) as u32
    }
}

pub fn enumerate() -> Vec<Monitor> {
    unsafe extern "system" fn collect(hmon: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let handles = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
        handles.push(hmon);
        true.into()
    }

    let mut handles: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut handles as *mut _ as isize),
        );
    }
    handles.into_iter().filter_map(describe).collect()
}

fn describe(handle: HMONITOR) -> Option<Monitor> {
    let mut info = MONITORINFOEXW {
        monitorInfo: MONITORINFO {
            cbSize: size_of::<MONITORINFOEXW>() as u32,
            ..Default::default()
        },
        ..Default::default()
    };
    unsafe { GetMonitorInfoW(handle, &mut info.monitorInfo as *mut MONITORINFO) }
        .as_bool()
        .then_some(())?;

    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
    unsafe {
        let _ = GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
    }
    let scale = f64::from(dpi_x) / 96.0;

    let name_len = info
        .szDevice
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(info.szDevice.len());
    let name = String::from_utf16_lossy(&info.szDevice[..name_len]);
    let px = info.monitorInfo.rcMonitor;

    Some(Monitor {
        handle,
        px,
        display: Display {
            name,
            scale,
            is_primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        },
    })
}
