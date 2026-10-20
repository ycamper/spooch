//! The pointer shapes the overlay and editor window use.

use super::*;

/// Cursor font glyphs.
const XC_CROSSHAIR: u16 = 34;
const XC_FLEUR: u16 = 52;
const XC_HAND2: u16 = 60;
const XC_LEFT_PTR: u16 = 68;
const XC_SB_H_DOUBLE_ARROW: u16 = 108;
const XC_SB_V_DOUBLE_ARROW: u16 = 116;
const XC_TOP_LEFT_CORNER: u16 = 134;
const XC_TOP_RIGHT_CORNER: u16 = 136;
const XC_XTERM: u16 = 152;

/// Font cursors for each pointer shape the editor asks for.
pub(super) struct Cursors {
    pub(super) crosshair: u32,
    pub(super) arrow: u32,
    pub(super) fleur: u32,
    pub(super) hand: u32,
    pub(super) horizontal: u32,
    pub(super) vertical: u32,
    pub(super) diagonal_down: u32,
    pub(super) diagonal_up: u32,
    pub(super) ibeam: u32,
}

impl Cursors {
    pub(super) fn get(&self, c: Cursor) -> u32 {
        match c {
            Cursor::Arrow => self.arrow,
            Cursor::Draw => self.crosshair,
            Cursor::Text => self.ibeam,
            Cursor::Move => self.fleur,
            Cursor::ResizeHorizontal => self.horizontal,
            Cursor::ResizeVertical => self.vertical,
            Cursor::ResizeDiagonalDown => self.diagonal_down,
            Cursor::ResizeDiagonalUp => self.diagonal_up,
            Cursor::Hand => self.hand,
        }
    }
}

impl Cursors {
    /// Every shape, from the X cursor font.
    pub(super) fn load(conn: &RustConnection) -> Result<Cursors, String> {
        let font: Font = conn.generate_id().map_err(|e| e.to_string())?;
        conn.open_font(font, b"cursor").map_err(|e| e.to_string())?;
        let glyph = |shape: u16| -> Result<u32, String> {
            let c = conn.generate_id().map_err(|e| e.to_string())?;
            conn.create_glyph_cursor(
                c,
                font,
                font,
                shape,
                shape + 1,
                0,
                0,
                0,
                0xffff,
                0xffff,
                0xffff,
            )
            .map_err(|e| e.to_string())?;
            Ok(c)
        };
        Ok(Cursors {
            crosshair: glyph(XC_CROSSHAIR)?,
            arrow: glyph(XC_LEFT_PTR)?,
            fleur: glyph(XC_FLEUR)?,
            hand: glyph(XC_HAND2)?,
            horizontal: glyph(XC_SB_H_DOUBLE_ARROW)?,
            vertical: glyph(XC_SB_V_DOUBLE_ARROW)?,
            diagonal_down: glyph(XC_TOP_LEFT_CORNER)?,
            diagonal_up: glyph(XC_TOP_RIGHT_CORNER)?,
            ibeam: glyph(XC_XTERM)?,
        })
    }
}
