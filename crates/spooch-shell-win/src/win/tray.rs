//! The notification-area icon and its menu. The icon belongs to the hidden
//! owner window; clicks on it are posted to the UI thread's message loop,
//! which builds the menu from the current settings and Recents.

use spooch_platform::app::{Command, MenuItem};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos,
    GetSystemMetrics, HICON, ICONINFO, MF_DISABLED, MF_GRAYED, MF_SEPARATOR, MF_STRING,
    PostMessageW, SM_CXSMICON, SetForegroundWindow, TPM_BOTTOMALIGN, TPM_NONOTIFY, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP, WM_NULL,
};
use windows::core::HSTRING;

/// The icon's callback message to the owner window.
pub const WM_TRAY: u32 = WM_APP + 2;
/// Posted to the UI thread when the icon is clicked.
pub const WM_TRAY_MENU: u32 = WM_APP + 3;

const ICON_ID: u32 = 1;

pub struct Tray {
    owner: HWND,
    icon: HICON,
}

impl Tray {
    pub fn new(owner: HWND) -> Option<Tray> {
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as u32;
        let icon = make_icon(size)?;
        let tray = Tray { owner, icon };
        tray.add().then_some(tray)
    }

    /// Show the icon; again after Explorer restarts.
    pub fn add(&self) -> bool {
        let mut data = self.data();
        data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        data.uCallbackMessage = WM_TRAY;
        data.hIcon = self.icon;
        let tip: Vec<u16> = "Spooch".encode_utf16().collect();
        data.szTip[..tip.len()].copy_from_slice(&tip);
        unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool()
    }

    fn data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.owner,
            uID: ICON_ID,
            ..Default::default()
        }
    }

    /// Show `items` at the pointer and return the chosen command.
    pub fn show_menu(&self, items: &[MenuItem]) -> Option<Command> {
        let mut commands: Vec<Command> = Vec::new();
        unsafe {
            let m = CreatePopupMenu().ok()?;
            for item in items {
                let (flags, id, text) = match item {
                    MenuItem::Separator => (MF_SEPARATOR, 0, String::new()),
                    MenuItem::Entry {
                        label,
                        key,
                        command,
                    } => {
                        let text = match key {
                            Some(k) => format!("{label}\t{k}"),
                            None => label.clone(),
                        };
                        match command {
                            Some(c) => {
                                commands.push(c.clone());
                                (MF_STRING, commands.len(), text)
                            }
                            None => (MF_STRING | MF_DISABLED | MF_GRAYED, 0, text),
                        }
                    }
                };
                let _ = AppendMenuW(m, flags, id, &HSTRING::from(text));
            }

            let mut p = POINT::default();
            let _ = GetCursorPos(&mut p);
            // Without this the menu doesn't close when clicking elsewhere.
            let _ = SetForegroundWindow(self.owner);
            let chosen = TrackPopupMenu(
                m,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
                p.x,
                p.y,
                None,
                self.owner,
                None,
            );
            // Documented workaround so the next click on the icon works.
            let _ = PostMessageW(
                Some(self.owner),
                WM_NULL,
                Default::default(),
                Default::default(),
            );
            let _ = DestroyMenu(m);
            let i = chosen.0 as usize;
            (i > 0).then(|| commands.get(i - 1).cloned()).flatten()
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.data());
            let _ = DestroyIcon(self.icon);
        }
    }
}

/// The app icon as an HICON, drawn by the shared renderer.
fn make_icon(size: u32) -> Option<HICON> {
    let pixels = spooch_render::app_icon(size);
    unsafe {
        let color = super::editing::dib_with(None, (size as i32, size as i32), &pixels)?;
        // An all-zero mask: the colour bitmap's alpha does the masking.
        let mask = CreateBitmap(size as i32, size as i32, 1, 1, None);
        let icon = CreateIconIndirect(&ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        });
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon.ok()
    }
}
