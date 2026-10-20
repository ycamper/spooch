//! Where Ctrl+S saves, and the local time captures are named by.

use std::io;
use std::path::PathBuf;

use spooch_platform::save::LocalTime;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Shell::{FOLDERID_Pictures, KF_FLAG_DEFAULT, SHGetKnownFolderPath};

/// `Spooch` in the Pictures known folder, which follows OneDrive and
/// user relocation.
pub fn folder() -> io::Result<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Pictures, KF_FLAG_DEFAULT, None)
            .map_err(|e| io::Error::other(format!("Pictures folder: {e}")))?;
        let path = p.to_string().map(PathBuf::from);
        CoTaskMemFree(Some(p.0 as *const _));
        path.map(|p| p.join("Spooch"))
            .map_err(|e| io::Error::other(format!("Pictures folder: {e}")))
    }
}

pub fn now() -> LocalTime {
    let t = unsafe { GetLocalTime() };
    LocalTime {
        year: t.wYear,
        month: t.wMonth as u8,
        day: t.wDay as u8,
        hour: t.wHour as u8,
        minute: t.wMinute as u8,
        second: t.wSecond as u8,
    }
}
