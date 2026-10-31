//! The styles as a small JSON file, so the last picks survive a restart:
//! each tool's style, the framing and the pinned one, the gallery, the
//! recent stickers and the format. Frames are kept by key and stickers by
//! name, so new built-ins don't shift them; anything unknown or out of
//! range keeps what it was. The palette isn't kept: `settings.txt` picks
//! the starting one.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::frame::{FRAME_COLOURS, Frame, SHADOWS};
use super::style::{
    EDGE_WIDTHS, EDGES, Framing, MIN_OPACITY, PALETTE, RECENT_STICKERS, SIZES, Style, Styles,
    tool_index,
};
use super::{Format, Tool};
use crate::{HighlightLook, LabelFont, Plate, STICKERS};

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Remembered {
    /// By tool name.
    tools: BTreeMap<String, Style>,
    #[serde(flatten)]
    framing: FramingFile,
    /// "Use for every capture".
    pinned: Option<FramingFile>,
    gallery: Option<bool>,
    /// Names, newest first.
    stickers: Vec<String>,
    /// "png" or "jpg".
    format: Option<String>,
}

/// A `Framing` with its frame by key; `None` for no frame.
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct FramingFile {
    frame: Option<String>,
    frame_colour: u8,
    shadow: u8,
}

impl From<Framing> for FramingFile {
    fn from(f: Framing) -> Self {
        FramingFile {
            frame: (f.frame != Frame::NONE).then(|| f.frame.key()),
            frame_colour: f.frame_colour,
            shadow: f.shadow,
        }
    }
}

impl FramingFile {
    /// As a framing, or `None` if its frame is gone or anything's out of
    /// range.
    fn framing(&self) -> Option<Framing> {
        let frame = match &self.frame {
            None => Frame::NONE,
            Some(key) => Frame::from_key(key)?,
        };
        (usize::from(self.frame_colour) < FRAME_COLOURS.len()
            && usize::from(self.shadow) < SHADOWS.len())
        .then_some(Framing {
            frame,
            frame_colour: self.frame_colour,
            shadow: self.shadow,
        })
    }
}

impl Style {
    /// Whether every index is in range.
    fn in_range(&self) -> bool {
        usize::from(self.color) < PALETTE.len()
            && usize::from(self.size) < SIZES
            && usize::from(self.edge) < EDGES
            && (MIN_OPACITY..=100).contains(&self.opacity)
            && usize::from(self.font) < LabelFont::ALL.len()
            && usize::from(self.edge_width) < EDGE_WIDTHS.len()
            && usize::from(self.variant) < HighlightLook::ALL.len().max(Plate::ALL.len())
    }
}

impl Styles {
    /// The styles and `format` as JSON, for `with_remembered`.
    pub fn remembered(&self, format: Format) -> String {
        let file = Remembered {
            tools: Tool::ALL
                .iter()
                .map(|t| (t.name().to_lowercase(), self.tools[tool_index(*t)]))
                .collect(),
            framing: self.framing.into(),
            pinned: self.pinned.map(FramingFile::from),
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
        for (name, style) in &file.tools {
            if let Some(t) = Tool::ALL
                .iter()
                .find(|t| t.name().eq_ignore_ascii_case(name))
                && style.in_range()
            {
                self.tools[tool_index(*t)] = *style;
            }
        }
        if let Some(f) = file.framing.framing() {
            self.framing = f;
        }
        self.pinned = file.pinned.as_ref().and_then(FramingFile::framing);
        if let Some(g) = file.gallery {
            self.gallery = g;
        }
        // Known names, newest first, then the ones they pushed out; each
        // once.
        let named = file
            .stickers
            .iter()
            .filter_map(|name| STICKERS.iter().position(|(n, _)| n == name))
            .map(|i| i as u8);
        let mut stickers = Vec::with_capacity(RECENT_STICKERS);
        for i in named.chain(self.stickers) {
            if stickers.len() == RECENT_STICKERS {
                break;
            }
            if !stickers.contains(&i) {
                stickers.push(i);
            }
        }
        if let Ok(stickers) = stickers.try_into() {
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
        styles.framing = Framing {
            frame: Frame::builtin("terminal-modern"),
            frame_colour: 3,
            shadow: 2,
        };
        styles.pinned = Some(styles.framing);
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
            "pinned": {"frame": "file:gone"},
            "stickers": ["not_a_sticker", "fire", "eyes", "fire"],
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
        assert_eq!(back.framing, base.framing);
        assert_eq!(back.pinned, None);
        // Fire and eyes move to the front, the rest follow in their old
        // order, and none twice.
        let at = |name| STICKERS.iter().position(|(n, _)| *n == name).unwrap() as u8;
        assert_eq!(back.stickers[..2], [at("fire"), at("eyes")]);
        let mut seen = back.stickers.to_vec();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), RECENT_STICKERS);
        assert_eq!(format, None);
        assert!(base.with_remembered("not json").is_err());
    }

    #[test]
    fn more_stickers_than_kept_still_count() {
        let names: Vec<_> = STICKERS.iter().rev().map(|(n, _)| *n).collect();
        let json = serde_json::json!({ "stickers": names }).to_string();
        let (back, _) = Styles::default().with_remembered(&json).unwrap();
        assert_eq!(back.stickers[0], STICKERS.len() as u8 - 1);
    }
}
