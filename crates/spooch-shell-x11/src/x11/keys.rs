//! Keycode to keysym lookup for the handful of keys Spooch reacts to.

use spooch_platform::settings::{Key, Shortcut};
use spooch_render::{Key as EditorKey, Mods};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt as _, KeyButMask, Keycode, ModMask};
use x11rb::rust_connection::RustConnection;

pub const XK_DELETE: u32 = 0xffff;
pub const XK_KP_DELETE: u32 = 0xff9f;
pub const XK_PRIOR: u32 = 0xff55;
pub const XK_NEXT: u32 = 0xff56;
pub const XK_KP_PRIOR: u32 = 0xff9a;
pub const XK_KP_NEXT: u32 = 0xff9b;
pub const XK_PRINT: u32 = 0xff61;
pub const XK_RETURN: u32 = 0xff0d;
pub const XK_KP_ENTER: u32 = 0xff8d;
pub const XK_ESCAPE: u32 = 0xff1b;
pub const XK_BACKSPACE: u32 = 0xff08;
pub const XK_LEFT: u32 = 0xff51;
pub const XK_UP: u32 = 0xff52;
pub const XK_RIGHT: u32 = 0xff53;
pub const XK_DOWN: u32 = 0xff54;
pub const XK_HOME: u32 = 0xff50;
pub const XK_END: u32 = 0xff57;
pub const XK_KP_LEFT: u32 = 0xff96;
pub const XK_KP_UP: u32 = 0xff97;
pub const XK_KP_RIGHT: u32 = 0xff98;
pub const XK_KP_DOWN: u32 = 0xff99;
pub const XK_KP_HOME: u32 = 0xff95;
pub const XK_KP_END: u32 = 0xff9c;
pub const XK_ALT_L: u32 = 0xffe9;
pub const XK_ALT_R: u32 = 0xffea;

pub struct Keymap {
    min: Keycode,
    per: usize,
    syms: Vec<u32>,
}

impl Keymap {
    pub fn load(conn: &RustConnection) -> Result<Self, String> {
        let setup = conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let reply = conn
            .get_keyboard_mapping(min, max - min + 1)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            min,
            per: reply.keysyms_per_keycode as usize,
            syms: reply.keysyms,
        })
    }

    /// The unshifted keysym, lowercased for letters.
    pub fn sym(&self, code: Keycode) -> u32 {
        let i = code.wrapping_sub(self.min) as usize * self.per;
        let sym = self.syms.get(i).copied().unwrap_or(0);
        if (0x41..=0x5a).contains(&sym) {
            sym + 0x20
        } else {
            sym
        }
    }

    /// The character a key types, honouring Shift, for keys that type one.
    pub fn char(&self, code: Keycode, shift: bool) -> Option<char> {
        let i = code.wrapping_sub(self.min) as usize * self.per;
        let at = |level: usize| {
            (level < self.per)
                .then(|| self.syms.get(i + level).copied())
                .flatten()
                .filter(|&s| s != 0)
        };
        let sym = if shift {
            at(1).or_else(|| at(0).map(upper))
        } else {
            at(0)
        }?;
        match sym {
            // Latin-1 keysyms are their own code points.
            0x20..=0x7e | 0xa0..=0xff => char::from_u32(sym),
            // Unicode keysyms carry the code point in the low 24 bits.
            0x0100_0000..=0x0110_ffff => char::from_u32(sym - 0x0100_0000),
            _ => None,
        }
    }

    pub fn codes(&self, sym: u32) -> Vec<Keycode> {
        (0..self.syms.len() / self.per.max(1))
            .map(|i| self.min + i as u8)
            .filter(|&c| self.sym(c) == sym)
            .collect()
    }
}

fn upper(sym: u32) -> u32 {
    if (0x61..=0x7a).contains(&sym) {
        sym - 0x20
    } else {
        sym
    }
}

/// The editor key for keysym `sym` (key code `detail`): a named key, or
/// the character it types without Shift.
pub(super) fn editor_key(keymap: &Keymap, sym: u32, detail: u8) -> Option<EditorKey> {
    match sym {
        XK_RETURN | XK_KP_ENTER => Some(EditorKey::Enter),
        XK_ESCAPE => Some(EditorKey::Escape),
        XK_BACKSPACE => Some(EditorKey::Backspace),
        XK_DELETE | XK_KP_DELETE => Some(EditorKey::Delete),
        XK_LEFT | XK_KP_LEFT => Some(EditorKey::Left),
        XK_RIGHT | XK_KP_RIGHT => Some(EditorKey::Right),
        XK_UP | XK_KP_UP => Some(EditorKey::Up),
        XK_DOWN | XK_KP_DOWN => Some(EditorKey::Down),
        XK_HOME | XK_KP_HOME => Some(EditorKey::Home),
        XK_END | XK_KP_END => Some(EditorKey::End),
        XK_PRIOR | XK_KP_PRIOR => Some(EditorKey::PageUp),
        XK_NEXT | XK_KP_NEXT => Some(EditorKey::PageDown),
        XK_PRINT => Some(EditorKey::PrintScreen),
        0xffbe..=0xffd5 => Some(EditorKey::F((sym - 0xffbe + 1) as u8)),
        // Anything else by the character it types without Shift.
        _ => keymap
            .char(detail, false)
            .map(|c| EditorKey::Char(c.to_ascii_lowercase())),
    }
}

/// The keysym a shortcut's key types, as the keymap reports it (letters
/// lower case).
pub(super) fn keysym(k: Key) -> u32 {
    match k {
        Key::Char(c) => c.to_ascii_lowercase() as u32,
        Key::F(n) => 0xffbe + u32::from(n) - 1,
        Key::PrintScreen => XK_PRINT,
    }
}

/// The modifiers that count for shortcuts; Lock and NumLock don't.
pub(super) const SHORTCUT_MODS: u16 = 0x04 | 0x01 | 0x08 | 0x40;

pub(super) fn modmask(s: &Shortcut) -> u16 {
    let mut m = 0;
    for (on, bit) in [
        (s.ctrl, ModMask::CONTROL),
        (s.shift, ModMask::SHIFT),
        (s.alt, ModMask::M1),
        (s.logo, ModMask::M4),
    ] {
        if on {
            m |= u16::from(bit);
        }
    }
    m
}

/// The modifiers held in an event's `state`.
pub fn mods(state: KeyButMask) -> Mods {
    Mods {
        ctrl: state.contains(KeyButMask::CONTROL),
        shift: state.contains(KeyButMask::SHIFT),
        alt: state.contains(KeyButMask::MOD1),
    }
}

/// Wheel button `button` (4 and 5 up and down, 6 and 7 tilts) as
/// `Session::wheel` takes it: notches away from the user or to the right,
/// and whether it's sideways (a tilt, or Shift held).
pub fn wheel(button: u8, state: KeyButMask) -> Option<(f32, bool)> {
    let (notches, tilt) = match button {
        4 => (1.0, false),
        5 => (-1.0, false),
        6 => (1.0, true),
        7 => (-1.0, true),
        _ => return None,
    };
    Some((notches, tilt || state.contains(KeyButMask::SHIFT)))
}
