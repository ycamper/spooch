//! A document as a file: JSON for the marks and layout, with the captured
//! pixels kept beside it. This is what Recents stores, so a capture can be
//! reopened with every mark still editable.

use skia_safe::Contains;
use std::fmt;

use serde::{Deserialize, Serialize};
use skia_safe::{IRect, Point, Rect, image::CachingHint};

use super::{Document, Format, Frame, Mark, Mode, Redaction, Tool};
use crate::{
    Arrow, Boxed, Highlight, HighlightLook, Label, LabelFont, Line, Look, Oval, Plate, Step, Stroke,
};

/// Bumped when the file layout changes in a way older code can't read.
const VERSION: u32 = 1;

/// Why a saved document couldn't be opened.
#[derive(Debug)]
pub struct LoadError(String);

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LoadError {}

type Xy = [f32; 2];
type Rgba = [f32; 4];
type Ltrb = [f32; 4];

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    /// The file name the capture is saved and dragged out under, without
    /// extension.
    name: String,
    scale: f32,
    /// Desktop position of the stored pixels' top-left corner.
    base_at: Xy,
    canvas: [i32; 4],
    tool: String,
    format: String,
    /// The frame round the output and its colour; older files have none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    frame: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    frame_colour: u8,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    frame_title: String,
    /// The drop shadow; older files have Auto.
    #[serde(default, skip_serializing_if = "is_zero")]
    shadow: u8,
    /// "pixel"; none for clean, as older files and unknown modes read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mode: Option<String>,
    #[serde(deserialize_with = "known_marks")]
    marks: Vec<SavedMark>,
}

/// The marks this version knows. A kind from a newer Spooch (synced from
/// another device, say) is left out, so the rest still opens.
fn known_marks<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<SavedMark>, D::Error> {
    let raw = Vec::<serde_json::Value>::deserialize(d)?;
    Ok(raw
        .into_iter()
        .filter_map(|m| serde_json::from_value(m).ok())
        .collect())
}

fn is_zero<T: Default + PartialEq>(n: &T) -> bool {
    *n == T::default()
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SavedMark {
    Arrow {
        from: Xy,
        to: Xy,
        color: Rgba,
        width: f32,
        #[serde(flatten)]
        look: SavedLook,
        /// Older files have none; they were all straight.
        #[serde(default, skip_serializing_if = "is_straight")]
        bend: f32,
    },
    Line {
        from: Xy,
        to: Xy,
        color: Rgba,
        width: f32,
        #[serde(flatten)]
        look: SavedLook,
        #[serde(default, skip_serializing_if = "is_straight")]
        bend: f32,
    },
    Box {
        rect: Ltrb,
        color: Rgba,
        width: f32,
        #[serde(flatten)]
        look: SavedLook,
        /// Older files have none: the standard slight rounding.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        radius: Option<f32>,
        /// Older files have none; they were all upright.
        #[serde(default, skip_serializing_if = "is_upright")]
        angle: f32,
    },
    Ellipse {
        rect: Ltrb,
        color: Rgba,
        width: f32,
        #[serde(flatten)]
        look: SavedLook,
        /// Older files have none; they were all upright.
        #[serde(default, skip_serializing_if = "is_upright")]
        angle: f32,
    },
    Sticker {
        rect: Ltrb,
        /// Its name in the set; one this version doesn't have shows as the
        /// first sticker.
        name: String,
        #[serde(default, skip_serializing_if = "is_upright")]
        angle: f32,
    },
    Highlight {
        rect: Ltrb,
        /// Older files have none; they were all yellow.
        #[serde(default = "highlight_yellow")]
        color: Rgba,
        /// "spotlight", "marker", "band" or "neon". Older files have none:
        /// they were a plain marker. A name this version doesn't know is
        /// read as a marker too.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        look: Option<String>,
    },
    Label {
        origin: Xy,
        text: String,
        color: Rgba,
        size: f32,
        #[serde(flatten)]
        look: SavedLook,
        /// Older files have none; they were all upright.
        #[serde(default, skip_serializing_if = "is_upright")]
        angle: f32,
        /// "rounded", "pixel", "tiny" or "impact"; older files have none and used
        /// the rounded one. A name this version doesn't know falls back
        /// to it too.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        font: Option<String>,
        /// Older files have none; they were straight.
        #[serde(default, skip_serializing_if = "is_upright")]
        bend: f32,
        /// "bubble", "tag" or "badge". Missing (older files) or unknown
        /// names mean outlined text.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plate: Option<String>,
    },
    Pen {
        points: Vec<Xy>,
        color: Rgba,
        width: f32,
        #[serde(flatten)]
        look: SavedLook,
    },
    Step {
        center: Xy,
        color: Rgba,
        radius: f32,
        #[serde(flatten)]
        look: SavedLook,
    },
    Redact {
        rect: Ltrb,
        /// "blur", "pixelate" or "solid".
        mode: String,
        /// Blur and pixelate strength, 0 to 4; older files have none and
        /// used the middle one.
        #[serde(default = "middle_strength")]
        level: u8,
    },
}

/// A mark's edge and opacity. Older files have neither: they were all
/// white-edged and opaque.
#[derive(Serialize, Deserialize)]
struct SavedLook {
    /// `null` for no edge.
    #[serde(default = "white_edge")]
    edge: Option<Rgba>,
    #[serde(default = "opaque")]
    opacity: f32,
    /// A multiple of the standard edge; older files have the standard one.
    #[serde(default = "opaque", skip_serializing_if = "is_standard_edge")]
    edge_width: f32,
    /// The edge follows the background; `edge` is its fallback.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    auto_edge: bool,
    /// Pixel art's anti-aliasing; older files have none.
    #[serde(default, skip_serializing_if = "is_zero")]
    smooth: f32,
}

impl From<Look> for SavedLook {
    fn from(l: Look) -> Self {
        SavedLook {
            edge: l.edge.map(rgba),
            opacity: l.opacity,
            edge_width: l.edge_width,
            auto_edge: l.auto_edge,
            smooth: l.smooth,
        }
    }
}

impl From<SavedLook> for Look {
    fn from(l: SavedLook) -> Self {
        Look {
            shadow: true,
            edge: l.edge.map(color),
            opacity: l.opacity.clamp(0.0, 1.0),
            edge_width: l.edge_width.clamp(0.1, 10.0),
            auto_edge: l.auto_edge,
            smooth: l.smooth.clamp(0.0, 1.0),
        }
    }
}

fn white_edge() -> Option<Rgba> {
    Some([1.0, 1.0, 1.0, 1.0])
}

fn is_standard_edge(k: &f32) -> bool {
    *k == 1.0
}

fn opaque() -> f32 {
    1.0
}

/// An angle or bend of zero, left out of the file.
fn is_upright(angle: &f32) -> bool {
    *angle == 0.0
}

fn is_straight(bend: &f32) -> bool {
    *bend == 0.0
}

fn middle_strength() -> u8 {
    2
}

fn highlight_yellow() -> Rgba {
    rgba(super::PALETTE[usize::from(super::style::YELLOW)])
}

fn xy(p: Point) -> Xy {
    [p.x, p.y]
}

fn point(a: Xy) -> Point {
    Point::new(a[0], a[1])
}

fn ltrb(r: Rect) -> Ltrb {
    [r.left, r.top, r.right, r.bottom]
}

fn rect(a: Ltrb) -> Rect {
    Rect::from_ltrb(a[0], a[1], a[2], a[3])
}

fn rgba(c: skia_safe::Color4f) -> Rgba {
    [c.r, c.g, c.b, c.a]
}

fn color(a: Rgba) -> skia_safe::Color4f {
    skia_safe::Color4f::new(a[0], a[1], a[2], a[3])
}

impl From<&Mark> for SavedMark {
    fn from(m: &Mark) -> Self {
        match m {
            Mark::Arrow(a) => SavedMark::Arrow {
                from: xy(a.from),
                to: xy(a.to),
                color: rgba(a.color),
                width: a.width,
                look: a.look.into(),
                bend: a.bend,
            },
            Mark::Line(l) => SavedMark::Line {
                from: xy(l.from),
                to: xy(l.to),
                color: rgba(l.color),
                width: l.width,
                look: l.look.into(),
                bend: l.bend,
            },
            Mark::Box(b) => SavedMark::Box {
                rect: ltrb(b.rect),
                color: rgba(b.color),
                width: b.width,
                look: b.look.into(),
                radius: b.radius,
                angle: b.angle,
            },
            Mark::Oval(o) => SavedMark::Ellipse {
                rect: ltrb(o.rect),
                color: rgba(o.color),
                width: o.width,
                look: o.look.into(),
                angle: o.angle,
            },
            Mark::Sticker(s) => SavedMark::Sticker {
                rect: ltrb(s.rect),
                name: s.name.to_string(),
                angle: s.angle,
            },
            Mark::Highlight(h) => SavedMark::Highlight {
                rect: ltrb(h.rect),
                color: rgba(h.color),
                look: Some(h.look.key().to_string()),
            },
            Mark::Label(l) => SavedMark::Label {
                origin: xy(l.origin),
                text: l.text.clone(),
                color: rgba(l.color),
                size: l.size,
                look: l.look.into(),
                angle: l.angle,
                font: (l.font != LabelFont::Rounded).then(|| l.font.key().to_string()),
                bend: l.bend,
                plate: (l.plate != Plate::Outlined).then(|| l.plate.key().to_string()),
            },
            Mark::Stroke(s) => SavedMark::Pen {
                points: s.points.iter().map(|p| xy(*p)).collect(),
                color: rgba(s.color),
                width: s.width,
                look: s.look.into(),
            },
            Mark::Step(s) => SavedMark::Step {
                center: xy(s.center),
                color: rgba(s.color),
                radius: s.radius,
                look: s.look.into(),
            },
            Mark::Redact(r, kind) => {
                let (mode, level) = match kind {
                    Redaction::Blur(l) => ("blur", *l),
                    Redaction::Pixelate(l) => ("pixelate", *l),
                    Redaction::Solid => ("solid", middle_strength()),
                };
                SavedMark::Redact {
                    rect: ltrb(*r),
                    mode: mode.into(),
                    level,
                }
            }
        }
    }
}

impl TryFrom<SavedMark> for Mark {
    type Error = LoadError;

    fn try_from(m: SavedMark) -> Result<Self, LoadError> {
        Ok(match m {
            SavedMark::Arrow {
                from,
                to,
                color: c,
                width,
                look,
                bend,
            } => Mark::Arrow(Arrow {
                from: point(from),
                to: point(to),
                color: color(c),
                width,
                look: look.into(),
                bend,
            }),
            SavedMark::Line {
                from,
                to,
                color: c,
                width,
                look,
                bend,
            } => Mark::Line(Line {
                from: point(from),
                to: point(to),
                color: color(c),
                width,
                look: look.into(),
                bend,
            }),
            SavedMark::Box {
                rect: r,
                color: c,
                width,
                look,
                radius,
                angle,
            } => Mark::Box(Boxed {
                rect: rect(r),
                color: color(c),
                width,
                look: look.into(),
                radius,
                angle,
            }),
            SavedMark::Ellipse {
                rect: r,
                color: c,
                width,
                look,
                angle,
            } => Mark::Oval(Oval {
                rect: rect(r),
                color: color(c),
                width,
                look: look.into(),
                angle,
            }),
            SavedMark::Sticker {
                rect: r,
                name,
                angle,
            } => Mark::Sticker(crate::Sticker {
                rect: rect(r),
                name: crate::sticker::sticker_named(&name).unwrap_or(crate::STICKERS[0].0),
                angle,
            }),
            SavedMark::Highlight {
                rect: r,
                color: c,
                look,
            } => Mark::Highlight(Highlight {
                rect: rect(r),
                color: color(c),
                look: look
                    .as_deref()
                    .and_then(HighlightLook::from_key)
                    .unwrap_or(HighlightLook::Marker),
            }),
            SavedMark::Label {
                origin,
                text,
                color: c,
                size,
                look,
                angle,
                font,
                bend,
                plate,
            } => Mark::Label(Label {
                origin: point(origin),
                text,
                color: color(c),
                size,
                look: look.into(),
                angle,
                font: font
                    .as_deref()
                    .and_then(LabelFont::from_key)
                    .unwrap_or_default(),
                bend: if bend.is_finite() { bend } else { 0.0 },
                plate: plate
                    .as_deref()
                    .and_then(Plate::from_key)
                    .unwrap_or_default(),
            }),
            SavedMark::Pen {
                points,
                color: c,
                width,
                look,
            } => Mark::Stroke(Stroke {
                points: points.into_iter().map(point).collect(),
                color: color(c),
                width,
                look: look.into(),
            }),
            SavedMark::Step {
                center,
                color: c,
                radius,
                look,
            } => Mark::Step(Step {
                center: point(center),
                color: color(c),
                radius,
                look: look.into(),
            }),
            SavedMark::Redact {
                rect: r,
                mode,
                level,
            } => {
                let level = level.min(4);
                let kind = match mode.as_str() {
                    "blur" => Redaction::Blur(level),
                    "pixelate" => Redaction::Pixelate(level),
                    "solid" => Redaction::Solid,
                    other => return Err(LoadError(format!("unknown redaction {other:?}"))),
                };
                Mark::Redact(rect(r), kind)
            }
        })
    }
}

impl Document {
    /// The name a saved document was kept under, which its saved pictures
    /// are named after.
    pub fn name_in(json: &str) -> Option<String> {
        #[derive(Deserialize)]
        struct Named {
            name: String,
        }
        serde_json::from_str::<Named>(json).ok().map(|n| n.name)
    }

    /// The marks and layout as JSON, with `name` (the file name without
    /// extension) kept alongside. The pixels go separately; see
    /// `base_pixels`.
    pub fn to_json(&self, name: &str) -> String {
        serde_json::to_string_pretty(&self.file(name)).expect("plain data serializes")
    }

    /// What the finished image depends on, to tell whether it has changed:
    /// the saved form without the tool and format.
    pub fn look(&self) -> String {
        let mut file = self.file("");
        file.tool.clear();
        file.format.clear();
        serde_json::to_string(&file).expect("plain data serializes")
    }

    fn file(&self, name: &str) -> File {
        let c = self.canvas;
        File {
            version: VERSION,
            name: name.to_string(),
            scale: self.scale,
            base_at: xy(self.base.at),
            canvas: [c.left, c.top, c.right, c.bottom],
            tool: self.tool.name().to_string(),
            format: self.format.name().to_string(),
            frame: match self.frame() {
                Frame::NONE => String::new(),
                f => f.key(),
            },
            frame_colour: self.frame_colour(),
            frame_title: self.frame_title().to_string(),
            shadow: self.shadow(),
            mode: (self.mode() != Mode::Clean).then(|| self.mode().key().to_string()),
            marks: self.marks.iter().map(SavedMark::from).collect(),
        }
    }

    /// The captured pixels the document holds, as tightly packed opaque
    /// BGRA: `(width, height, pixels)`.
    pub fn base_pixels(&self) -> (u32, u32, Vec<u8>) {
        let image = &self.base.image;
        let (w, h) = (image.width(), image.height());
        let info = crate::bgra_info((w, h), skia_safe::AlphaType::Opaque);
        let mut out = vec![0u8; w as usize * h as usize * 4];
        image.read_pixels(&info, &mut out, w as usize * 4, (0, 0), CachingHint::Allow);
        (w as u32, h as u32, out)
    }

    /// Reopen a saved document from its JSON and pixels. Returns it with
    /// its name. The canvas can be cropped and extended again, but only
    /// over the pixels that were kept.
    pub fn from_saved(
        json: &str,
        width: u32,
        height: u32,
        bgra: &[u8],
    ) -> Result<(Document, String), LoadError> {
        let file: File =
            serde_json::from_str(json).map_err(|e| LoadError(format!("document.json: {e}")))?;
        if file.version > VERSION {
            return Err(LoadError(format!(
                "saved by a newer Spooch (format {})",
                file.version
            )));
        }
        let mut doc = Document::from_bgra(width, height, width as usize * 4, bgra, file.scale)
            .ok_or_else(|| LoadError("could not create the editor surface".into()))?;
        doc.base.at = point(file.base_at);
        let base = doc.base_rect();
        let [l, t, r, b] = file.canvas;
        let canvas = IRect::from_ltrb(l, t, r, b);
        doc.canvas = if base.contains(canvas) { canvas } else { base };
        doc.desktop = base;
        // Like `known_marks`: a mark this version can't read is left out.
        doc.marks = file
            .marks
            .into_iter()
            .filter_map(|m| Mark::try_from(m).ok())
            .collect();
        if let Some(t) = Tool::ALL.iter().find(|t| t.name() == file.tool) {
            doc.tool = *t;
        }
        if let Some(f) = Format::ALL.into_iter().find(|f| f.name() == file.format) {
            doc.format = f;
        }
        doc.set_frame(Frame::from_key(&file.frame).unwrap_or_default());
        doc.set_frame_colour(file.frame_colour);
        doc.set_shadow(file.shadow);
        doc.set_mode(
            file.mode
                .as_deref()
                .and_then(Mode::from_key)
                .unwrap_or_default(),
        );
        if !file.frame_title.is_empty() {
            doc.set_frame_title(&file.frame_title);
        }
        doc.name = file.name.clone();
        Ok((doc, file.name))
    }

    /// The selected marks, for pasting into this capture or another with
    /// `paste`; `None` when nothing is selected.
    pub fn copy_selected(&self) -> Option<String> {
        let selection = self.selection();
        if selection.is_empty() {
            return None;
        }
        let marks: Vec<SavedMark> = selection
            .iter()
            .filter_map(|&i| self.marks.get(i))
            .map(SavedMark::from)
            .collect();
        serde_json::to_string(&marks).ok()
    }

    /// Paste marks copied with `copy_selected`: a little down and right of
    /// where they were, or in the middle of the canvas when that's off it.
    /// They come in selected, as one undo step. Returns whether any came.
    pub fn paste(&mut self, copied: &str) -> bool {
        let Ok(saved) = serde_json::from_str::<Vec<SavedMark>>(copied) else {
            return false;
        };
        let Ok(mut marks) = saved
            .into_iter()
            .map(Mark::try_from)
            .collect::<Result<Vec<_>, _>>()
        else {
            return false;
        };
        if marks.is_empty() {
            return false;
        }
        self.commit_text();
        let tf = self.typeface.as_ref();
        let Some(bounds) = marks
            .iter()
            .filter_map(|m| m.bounds(tf))
            .reduce(Rect::join2)
        else {
            return false;
        };
        let step = super::DUPLICATE_OFFSET * self.ui_px();
        let mut d = Point::new(step, step);
        let canvas = Rect::from(self.canvas);
        let moved = bounds.with_offset(d);
        if !canvas.contains(moved) {
            d = canvas.center() - bounds.center();
        }
        for m in &mut marks {
            m.offset(d);
        }
        self.checkpoint();
        self.add_selected(marks);
        true
    }

    /// Keep the canvas to the pixels the document holds, for the editor
    /// window, which has no frozen desktop to grow over.
    pub fn confine(&mut self) {
        self.desktop = self.base_rect();
        self.want_desktop = false;
        self.pending_canvas = None;
    }

    /// Move everything, pixels, canvas and marks, by `(dx, dy)` desktop
    /// pixels. Used when a reopened capture would land off the current
    /// screens.
    pub fn shift(&mut self, dx: i32, dy: i32) {
        let d = Point::new(dx as f32, dy as f32);
        self.base.at += d;
        self.canvas = self.canvas.with_offset((dx, dy));
        self.desktop = self.desktop.with_offset((dx, dy));
        for m in &mut self.marks {
            m.offset(d);
        }
        if let Some(l) = self.typing.as_mut() {
            l.origin += d;
        }
        self.backdrop = None;
        self.fresh = false;
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{click, drag, placed};
    use super::*;

    #[test]
    fn a_saved_document_reopens_with_everything_editable() {
        let (mut d, desktop) = placed();
        d.set_tool(Tool::Box);
        drag(&mut d, (120.0, 110.0), (200.0, 160.0));
        d.set_tool(Tool::Steps);
        click(&mut d, 250.0, 150.0);
        d.set_tool(Tool::Pen);
        drag(&mut d, (130.0, 180.0), (190.0, 185.0));
        d.set_tool(Tool::Pixelate);
        drag(&mut d, (210.0, 110.0), (290.0, 140.0));
        d.begin_text(Point::new(140.0, 130.0));
        d.type_text("Hi there");
        d.commit_text();
        // Crop, then extend over the desktop, so the base has grown.
        d.begin_drag(Point::new(302.0, 150.0));
        d.supply_desktop(400, 300, 400 * 4, &desktop);
        d.drag_to(Point::new(342.0, 150.0));
        d.end_drag();
        d.set_format(Format::Jpeg);
        let before = d.export();

        let json = d.to_json("Spooch 2026-10-03 at 23.10.42");
        let (w, h, pixels) = d.base_pixels();
        assert_eq!((w, h), (400, 300));
        let (mut back, name) = Document::from_saved(&json, w, h, &pixels).unwrap();
        assert_eq!(name, "Spooch 2026-10-03 at 23.10.42");
        assert_eq!(back.canvas(), d.canvas());
        assert_eq!(back.format(), Format::Jpeg);
        assert_eq!(back.counts(), d.counts());
        assert_eq!(back.export(), before);
        // Marks are still objects: the box moves.
        click(&mut back, 140.0, 110.0);
        assert!(back.has_selection());
        assert!(drag(&mut back, (140.0, 110.0), (150.0, 110.0)));
        // The canvas can still extend, but only over the kept pixels.
        assert!(back.wants_desktop().is_none() || back.base_rect() == back.desktop);
    }

    #[test]
    fn shifting_moves_everything_together() {
        let (mut d, _) = placed();
        drag(&mut d, (120.0, 150.0), (280.0, 150.0));
        let before = d.export();
        d.shift(-100, -100);
        assert_eq!(d.canvas(), IRect::from_ltrb(0, 0, 200, 100));
        assert_eq!(d.export(), before);
    }

    #[test]
    fn marks_from_a_newer_spooch_are_left_out() {
        let (mut d, _) = placed();
        d.set_tool(Tool::Box);
        drag(&mut d, (120.0, 110.0), (200.0, 160.0));
        let json = d.to_json("x").replacen(
            "\"marks\": [",
            "\"marks\": [{\"kind\": \"hologram\", \"glow\": 9}, ",
            1,
        );
        assert!(json.contains("hologram"), "{json}");
        let (w, h, px) = d.base_pixels();
        let (back, _) = Document::from_saved(&json, w, h, &px).unwrap();
        assert_eq!(back.counts().boxes, 1);
        assert_eq!(back.marks.len(), 1);
    }

    #[test]
    fn redactions_of_a_newer_mode_are_left_out() {
        let (mut d, _) = placed();
        d.set_tool(Tool::Box);
        drag(&mut d, (120.0, 110.0), (200.0, 160.0));
        d.set_tool(Tool::Blur);
        drag(&mut d, (210.0, 110.0), (260.0, 160.0));
        let json = d
            .to_json("x")
            .replace("\"mode\": \"blur\"", "\"mode\": \"swirl\"");
        assert!(json.contains("swirl"), "{json}");
        let (w, h, px) = d.base_pixels();
        let (back, _) = Document::from_saved(&json, w, h, &px).unwrap();
        assert_eq!(back.counts().boxes, 1);
        assert_eq!(back.marks.len(), 1);
    }

    #[test]
    fn bad_files_are_refused() {
        let (d, _) = placed();
        let (w, h, px) = d.base_pixels();
        assert!(Document::from_saved("{", w, h, &px).is_err());
        let newer = d.to_json("x").replace("\"version\": 1", "\"version\": 99");
        assert!(Document::from_saved(&newer, w, h, &px).is_err());
    }
}
