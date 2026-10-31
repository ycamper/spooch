//! The styles as a small JSON file, so the last picks survive a restart:
//! each tool's style, the frame, its colour and shadow, the gallery, the
//! recent stickers and the format. Frames are kept by key and stickers by
//! name, so new built-ins don't shift them; anything unknown or out of
//! range keeps what it was. The palette isn't kept: `settings.txt` picks
//! the starting one.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::frame::{FRAME_COLOURS, Frame, SHADOWS};
use super::style::{
    EDGE_WIDTHS, EDGES, MIN_OPACITY, PALETTE, RECENT_STICKERS, SIZES, Style, Styles, tool_index,
};
use super::{Format, Tool};
use crate::{HighlightLook, LabelFont, STICKERS};

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Remembered {
    /// By tool name.
    tools: BTreeMap<String, Picked>,
    /// A frame key; `None` for no frame.
    frame: Option<String>,
    frame_colour: u8,
    shadow: u8,
    gallery: Option<bool>,
    /// Names, newest first.
    stickers: Vec<String>,
    /// "png" or "jpg".
    format: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Picked {
    color: u8,
    size: u8,
    edge: u8,
    opacity: u8,
    font: u8,
    edge_width: u8,
    highlight: u8,
}

impl Default for Picked {
    fn default() -> Self {
        Picked::from(Style::default())
    }
}

impl From<Style> for Picked {
    fn from(s: Style) -> Self {
        Picked {
            color: s.color,
            size: s.size,
            edge: s.edge,
            opacity: s.opacity,
            font: s.font,
            edge_width: s.edge_width,
            highlight: s.highlight,
        }
    }
}

impl Picked {
    /// As a style, or `None` if any of it is out of range.
    fn style(&self) -> Option<Style> {
        let ok = usize::from(self.color) < PALETTE.len()
            && usize::from(self.size) < SIZES
            && usize::from(self.edge) < EDGES
            && (MIN_OPACITY..=100).contains(&self.opacity)
            && usize::from(self.font) < LabelFont::ALL.len()
            && usize::from(self.edge_width) < EDGE_WIDTHS.len()
            && usize::from(self.highlight) < HighlightLook::ALL.len();
        ok.then_some(Style {
            color: self.color,
            size: self.size,
            edge: self.edge,
            opacity: self.opacity,
            font: self.font,
            edge_width: self.edge_width,
            highlight: self.highlight,
        })
    }
}

impl Styles {
    /// The styles and `format` as JSON, for `with_remembered`.
    pub fn remembered(&self, format: Format) -> String {
        let file = Remembered {
            tools: Tool::ALL
                .iter()
                .map(|t| {
                    let style = self.tools[tool_index(*t)];
                    (t.name().to_lowercase(), Picked::from(style))
                })
                .collect(),
            frame: (self.frame != Frame::NONE).then(|| self.frame.key()),
            frame_colour: self.frame_colour,
            shadow: self.shadow,
            gallery: Some(self.gallery),
            stickers: self
                .stickers
                .iter()
                .filter_map(|&i| STICKERS.get(usize::from(i)))
                .map(|(name, _)| name.to_string())
                .collect(),
            format: Some(format.extension().to_string()),
        };
        serde_json::to_string_pretty(&file).unwrap_or_default()
    }

    /// These styles with what `json` (from `remembered`) says on top, and
    /// the format it names. Load the custom frames first, so a frame of
    /// theirs can be found.
    pub fn with_remembered(mut self, json: &str) -> Result<(Styles, Option<Format>), String> {
        let file: Remembered = serde_json::from_str(json).map_err(|e| e.to_string())?;
        for (name, picked) in &file.tools {
            if let Some(t) = Tool::ALL
                .iter()
                .find(|t| t.name().eq_ignore_ascii_case(name))
                && let Some(style) = picked.style()
            {
                self.tools[tool_index(*t)] = style;
            }
        }
        match &file.frame {
            None => self.frame = Frame::NONE,
            Some(key) => {
                if let Some(f) = Frame::from_key(key) {
                    self.frame = f;
                }
            }
        }
        if usize::from(file.frame_colour) < FRAME_COLOURS.len() {
            self.frame_colour = file.frame_colour;
        }
        if usize::from(file.shadow) < SHADOWS.len() {
            self.shadow = file.shadow;
        }
        if let Some(g) = file.gallery {
            self.gallery = g;
        }
        // Known names, newest first, then the ones they pushed out.
        let mut picked: Vec<u8> = file
            .stickers
            .iter()
            .filter_map(|name| STICKERS.iter().position(|(n, _)| n == name))
            .map(|i| i as u8)
            .collect();
        picked.dedup();
        for i in self.stickers {
            if picked.len() == RECENT_STICKERS {
                break;
            }
            if !picked.contains(&i) {
                picked.push(i);
            }
        }
        if let Ok(stickers) = <[u8; RECENT_STICKERS]>::try_from(picked) {
            self.stickers = stickers;
        }
        let format = match file.format.as_deref() {
            Some("png") => Some(Format::Png),
            Some("jpg") => Some(Format::Jpeg),
            _ => None,
        };
        Ok((self, format))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_come_back() {
        let mut styles = Styles::default();
        let t = tool_index(Tool::Box);
        styles.tools[t].color = 4;
        styles.tools[t].size = 4;
        styles.frame = Frame::builtin("terminal-modern");
        styles.frame_colour = 3;
        styles.shadow = 2;
        styles.gallery = false;
        styles.stickers[0] = STICKERS.len() as u8 - 1;
        let json = styles.remembered(Format::Jpeg);
        let (back, format) = Styles::default().with_remembered(&json).unwrap();
        assert_eq!(back, styles);
        assert_eq!(format, Some(Format::Jpeg));
    }

    #[test]
    fn unknown_and_out_of_range_keep_what_was() {
        let json = r#"{
            "tools": {"box": {"color": 99}, "laser": {"color": 1}, "arrow": {"color": 3}},
            "frame": "file:gone",
            "shadow": 200,
            "stickers": ["not_a_sticker", "fire"],
            "format": "gif",
            "from_the_future": true
        }"#;
        let base = Styles::default();
        let (back, format) = base.with_remembered(json).unwrap();
        assert_eq!(
            back.tools[tool_index(Tool::Box)],
            base.tools[tool_index(Tool::Box)]
        );
        assert_eq!(back.tools[tool_index(Tool::Arrow)].color, 3);
        assert_eq!((back.frame, back.shadow), (base.frame, base.shadow));
        // Fire moves to the front; the rest follow in their old order.
        let fire = STICKERS.iter().position(|(n, _)| *n == "fire").unwrap() as u8;
        assert_eq!(back.stickers[0], fire);
        assert_eq!(back.stickers.iter().filter(|&&i| i == fire).count(), 1);
        assert_eq!(format, None);
        assert!(base.with_remembered("not json").is_err());
    }
}
