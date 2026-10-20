//! Small pieces of Windows the tray menu and first-run welcome need: where
//! settings live, launching at sign-in, Snipping Tool's hold on the Print
//! Screen key, opening files, and yes/no questions.

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, HWND};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_DWORD, REG_SZ, RRF_RT_REG_DWORD,
    RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    IDYES, MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO,
    MessageBoxW, SW_SHOWNORMAL,
};
use windows::core::{HSTRING, PCWSTR, w};

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const RUN_VALUE: PCWSTR = w!("Spooch");
const KEYBOARD_KEY: PCWSTR = w!(r"Control Panel\Keyboard");
const SNIPPING_VALUE: PCWSTR = w!("PrintScreenKeyForSnippingEnabled");

/// `%APPDATA%\Spooch`, where settings live.
pub fn config_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Spooch")
}

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

fn read_string(key: HKEY, sub: PCWSTR, value: PCWSTR) -> Option<String> {
    let mut buf = vec![0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let ok = unsafe {
        RegGetValueW(
            key,
            sub,
            value,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    ok.is_ok().then(|| {
        let chars = (len as usize / 2).saturating_sub(1);
        String::from_utf16_lossy(&buf[..chars])
    })
}

fn read_dword(key: HKEY, sub: PCWSTR, value: PCWSTR) -> Option<u32> {
    let mut v = 0u32;
    let mut len = 4u32;
    let ok = unsafe {
        RegGetValueW(
            key,
            sub,
            value,
            RRF_RT_REG_DWORD,
            None,
            Some((&mut v as *mut u32).cast()),
            Some(&mut len),
        )
    };
    ok.is_ok().then_some(v)
}

/// The command the Run key holds for this exe: its full path, quoted.
fn run_command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\"", exe.display()))
}

/// Whether Windows starts this Spooch when the person signs in.
pub fn launch_at_login() -> bool {
    match (
        read_string(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE),
        run_command(),
    ) {
        (Some(have), Some(want)) => have.eq_ignore_ascii_case(&want),
        _ => false,
    }
}

pub fn set_launch_at_login(on: bool) -> Result<(), String> {
    unsafe {
        if on {
            let cmd = run_command().ok_or("can't find Spooch's own path")?;
            let data: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                RUN_VALUE,
                REG_SZ.0,
                Some(data.as_ptr().cast()),
                (data.len() * 2) as u32,
            )
            .ok()
            .map_err(|e| e.to_string())
        } else {
            let r = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE);
            if r.is_ok() || r == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                Err(r.to_hresult().message())
            }
        }
    }
}

/// Whether Windows 11 opens Snipping Tool on Print Screen, which keeps the
/// key from any other app. `None` when the setting doesn't exist (Windows
/// 10, or never touched; Windows 11 then treats it as on).
pub fn snipping_takes_print_screen() -> Option<bool> {
    read_dword(HKEY_CURRENT_USER, KEYBOARD_KEY, SNIPPING_VALUE).map(|v| v != 0)
}

/// Turn Snipping Tool's use of Print Screen on or off. This is the
/// "Use the Print screen key to open screen capture" switch in Windows
/// Settings, so the person can always turn it back there.
pub fn set_snipping_takes_print_screen(on: bool) -> Result<(), String> {
    let v: u32 = on.into();
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            KEYBOARD_KEY,
            SNIPPING_VALUE,
            REG_DWORD.0,
            Some((&v as *const u32).cast()),
            4,
        )
        .ok()
        .map_err(|e| e.to_string())
    }
}

/// Whether this is Windows 11 or later (build 22000 and up), where Print
/// Screen opens Snipping Tool unless that's switched off.
pub fn windows_11() -> bool {
    read_string(
        HKEY_LOCAL_MACHINE,
        w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"),
        w!("CurrentBuildNumber"),
    )
    .and_then(|b| b.trim().parse::<u32>().ok())
    .is_some_and(|b| b >= 22000)
}

/// Show `path` selected in Explorer.
pub fn reveal(path: &Path) {
    use std::os::windows::process::CommandExt;
    // Explorer wants the quotes round the path alone, not the argument.
    if let Err(e) = std::process::Command::new("explorer")
        .raw_arg(format!("/select,\"{}\"", path.display()))
        .spawn()
    {
        super::log::line(format!("could not show {}: {e}", path.display()));
    }
}

/// Open a file or folder the way Explorer would.
pub fn open(path: &Path) {
    let p = wide(path.as_os_str());
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(p.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        );
    }
}

/// A yes/no question, on top of everything. Callable from any thread
/// (with no owner).
pub fn ask(owner: Option<HWND>, text: &str) -> bool {
    unsafe {
        MessageBoxW(
            owner,
            &HSTRING::from(text),
            w!("Spooch"),
            MB_YESNO | MB_ICONQUESTION | MB_TOPMOST | MB_SETFOREGROUND,
        ) == IDYES
    }
}

pub fn tell(owner: Option<HWND>, text: &str) {
    unsafe {
        MessageBoxW(
            owner,
            &HSTRING::from(text),
            w!("Spooch"),
            MB_OK | MB_ICONINFORMATION | MB_TOPMOST | MB_SETFOREGROUND,
        );
    }
}
