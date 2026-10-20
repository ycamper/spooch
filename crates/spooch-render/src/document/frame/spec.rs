//! The frame file format. A frame is a small TOML file: how the window
//! looks (corner radius, shadow, outline, default colour) and the rows of
//! its window top, each a line of items laid out left to right, the
//! growing ones sharing what's left. Or, for frames drawn in an image
//! editor, a PNG and how to stretch it.
//!
//! The built-in frames are written in this format too (`builtin/*.toml`),
//! so they're also the examples. `README` below is the reference people
//! see in their frames folder.

use serde::Deserialize;
use skia_safe::Color4f;

/// One frame, as its file describes it.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameSpec {
    /// The name in the Frame list.
    pub name: String,
    /// Corner radius, in screen pixels.
    #[serde(default = "default_radius")]
    pub radius: f32,
    /// Whether the bar's Shadow choice applies; false never has one.
    #[serde(default = "yes")]
    pub shadow: bool,
    /// A hairline round the window: "auto", "light", "dark", "none" or
    /// "#rrggbb".
    #[serde(default)]
    pub outline: Outline,
    /// How the outline is drawn.
    #[serde(default, rename = "outline-style")]
    pub outline_style: LineStyle,
    /// Edges left, right and below the capture, in the top's colour, in
    /// screen pixels.
    #[serde(default)]
    pub border: f32,
    /// The window top's colour when the colour swatch is on Default:
    /// "light", "dark" or "#rrggbb".
    #[serde(default)]
    pub colour: Option<String>,
    /// Text and icons on the frame's own colour, "#rrggbb"; otherwise (and
    /// on a picked colour) they follow the top.
    #[serde(default)]
    pub ink: Option<String>,
    /// The titles' typeface.
    #[serde(default)]
    pub font: Typeface,
    /// The window top, top row first.
    #[serde(default)]
    pub rows: Vec<Row>,
    /// A frame drawn as an image instead of a window.
    #[serde(default)]
    pub image: Option<ImageFrame>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(try_from = "String")]
pub enum Outline {
    /// Light on light tops, dark on dark ones.
    #[default]
    Auto,
    Light,
    Dark,
    None,
    Colour(Color4f),
}

impl TryFrom<String> for Outline {
    type Error = String;

    fn try_from(s: String) -> Result<Outline, String> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Outline::Auto,
            "light" => Outline::Light,
            "dark" => Outline::Dark,
            "none" => Outline::None,
            _ => Outline::Colour(parse_colour(&s)?),
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LineStyle {
    #[default]
    Solid,
    Dashed,
    /// Two lines, a little apart.
    Double,
    /// Dashed, with a + at each corner.
    Ascii,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Typeface {
    /// The chrome's own.
    #[default]
    Ui,
    /// A system monospace face.
    Mono,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MenuStyle {
    /// Three dots, upright.
    #[default]
    Kebab,
    /// Three lines.
    Hamburger,
}

/// A row of the window top.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    /// In screen pixels.
    pub height: f32,
    #[serde(default)]
    pub background: Fill,
    #[serde(default)]
    pub items: Vec<Item>,
}

/// What a row or an address bar is filled with, all derived from the
/// top's colour so any colour works.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Fill {
    /// The top's colour.
    #[default]
    Top,
    /// Lighter on a light top, darker on a dark one (an address bar).
    Field,
    /// A quieter field, for a field on a field.
    Well,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    #[default]
    Left,
    Center,
}

/// Something on a row. Widths and heights are in screen pixels.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Item {
    /// Empty room.
    Space {
        width: f32,
    },
    /// Back and forward chevrons, and a reload arrow.
    Back,
    Forward,
    Reload,
    /// A + for a new tab.
    Plus,
    /// A box with an arrow out of it.
    Share,
    /// A menu button.
    Menu {
        #[serde(default)]
        style: MenuStyle,
    },
    /// An address bar; it shows the frame's title unless `text = false`.
    Address {
        #[serde(default = "default_address_height")]
        height: f32,
        #[serde(default)]
        lock: bool,
        #[serde(default)]
        align: Align,
        #[serde(default = "field")]
        fill: Fill,
        #[serde(default = "yes")]
        text: bool,
        #[serde(default)]
        width: Option<f32>,
    },
    /// Minimise, maximise and close, monochrome; `size` scales them.
    Buttons {
        #[serde(default = "one")]
        size: f32,
    },
    /// A green >_.
    Prompt,
    /// The frame's title, or `default` when it has none; with neither, a
    /// quiet line unless `placeholder = false`.
    Title {
        #[serde(default)]
        default: String,
        #[serde(default)]
        align: Align,
        #[serde(default = "yes")]
        placeholder: bool,
        #[serde(default)]
        width: Option<f32>,
    },
    /// A browser tab holding the title, with a globe and, if `close`, a
    /// cross: `fraction` of the row, at least `min-width`.
    Tab {
        #[serde(default = "default_fraction")]
        fraction: f32,
        #[serde(default = "default_tab_width", rename = "min-width")]
        min_width: f32,
        #[serde(default)]
        close: bool,
    },
    /// Coloured dots.
    Dots {
        colours: Vec<String>,
    },
    /// Hollow circles, in the ink.
    Circles {
        #[serde(default = "three")]
        count: u8,
    },
    /// A picture from the frames folder, `height` tall (its width
    /// follows).
    Image {
        file: String,
        #[serde(default = "default_image_height")]
        height: f32,
    },
}

/// A frame drawn in an image editor: the PNG is stretched round the
/// capture like a 9-slice, its corners kept and its edges stretched.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageFrame {
    /// The PNG, next to the frame file.
    pub file: String,
    /// How far in from the left, top, right and bottom the stretchable
    /// middle starts, in the image's pixels.
    pub slice: [f32; 4],
    /// Where the capture goes: its distance from the left, top, right and
    /// bottom of the framed result, in the image's pixels.
    pub content: [f32; 4],
}

fn yes() -> bool {
    true
}
fn one() -> f32 {
    1.0
}
fn three() -> u8 {
    3
}
fn field() -> Fill {
    Fill::Field
}
fn default_radius() -> f32 {
    10.0
}
fn default_address_height() -> f32 {
    20.0
}
fn default_fraction() -> f32 {
    0.4
}
fn default_tab_width() -> f32 {
    80.0
}
fn default_image_height() -> f32 {
    16.0
}

impl FrameSpec {
    /// Read a frame file, or say what's wrong with it.
    pub fn parse(text: &str) -> Result<FrameSpec, String> {
        let spec: FrameSpec = toml::from_str(text).map_err(|e| e.message().to_string())?;
        spec.check()?;
        Ok(spec)
    }

    /// Catch values that would make a frame unusable, with a message a
    /// person can act on.
    fn check(&self) -> Result<(), String> {
        let within = |what: &str, v: f32, lo: f32, hi: f32| {
            if (lo..=hi).contains(&v) {
                Ok(())
            } else {
                Err(format!("{what} is {v}; it has to be {lo} to {hi}"))
            }
        };
        if self.name.trim().is_empty() {
            return Err("the name is empty".into());
        }
        within("radius", self.radius, 0.0, 60.0)?;
        within("border", self.border, 0.0, 60.0)?;
        for c in [&self.colour, &self.ink].into_iter().flatten() {
            parse_colour(c)?;
        }
        let mut total = 0.0;
        for (r, row) in self.rows.iter().enumerate() {
            within(&format!("row {} height", r + 1), row.height, 1.0, 200.0)?;
            total += row.height;
            for item in &row.items {
                match item {
                    Item::Space { width } => within("a space's width", *width, 0.0, 2000.0)?,
                    Item::Address { height, width, .. } => {
                        within("an address bar's height", *height, 4.0, row.height)?;
                        if let Some(w) = width {
                            within("an address bar's width", *w, 10.0, 4000.0)?;
                        }
                    }
                    Item::Title { width: Some(w), .. } => {
                        within("a title's width", *w, 10.0, 4000.0)?
                    }
                    Item::Tab {
                        fraction,
                        min_width,
                        ..
                    } => {
                        within("a tab's fraction", *fraction, 0.05, 1.0)?;
                        within("a tab's min-width", *min_width, 10.0, 2000.0)?;
                    }
                    Item::Dots { colours } => {
                        if colours.is_empty() || colours.len() > 12 {
                            return Err("dots need 1 to 12 colours".into());
                        }
                        for c in colours {
                            parse_colour(c)?;
                        }
                    }
                    Item::Image { height, .. } => {
                        within("an image's height", *height, 2.0, row.height)?
                    }
                    Item::Buttons { size } => within("the buttons' size", *size, 0.5, 2.0)?,
                    Item::Circles { count } => within("circles", f32::from(*count), 1.0, 12.0)?,
                    _ => {}
                }
            }
        }
        within("the rows' total height", total, 0.0, 300.0)?;
        if let Some(img) = &self.image {
            if !self.rows.is_empty() {
                return Err("a frame is either an image or rows, not both".into());
            }
            for v in img.slice.iter().chain(&img.content) {
                within("an image frame's slice and content values", *v, 0.0, 2000.0)?;
            }
        }
        Ok(())
    }

    /// The colour the window top has when the swatch is on Default.
    pub fn default_colour(&self) -> Color4f {
        self.colour
            .as_deref()
            .and_then(|c| parse_colour(c).ok())
            .unwrap_or(super::LIGHT)
    }

    /// The window top's height, in screen pixels.
    pub(crate) fn top_height(&self) -> f32 {
        self.rows.iter().map(|r| r.height).sum()
    }

    /// The ink on the frame's own colour, if the file sets one.
    pub fn own_ink(&self) -> Option<Color4f> {
        self.ink.as_deref().and_then(|c| parse_colour(c).ok())
    }
}

/// "light", "dark", or "#rrggbb".
pub fn parse_colour(c: &str) -> Result<Color4f, String> {
    match c.trim().to_ascii_lowercase().as_str() {
        "light" => return Ok(super::LIGHT),
        "dark" => return Ok(super::DARK),
        _ => {}
    }
    let hex = c.trim().trim_start_matches('#');
    let bad = || format!("{c:?} isn't a colour: use light, dark or #rrggbb");
    if hex.len() != 6 {
        return Err(bad());
    }
    let n = u32::from_str_radix(hex, 16).map_err(|_| bad())?;
    Ok(super::super::theme::hex(n))
}

/// The reference written into the frames folder.
pub const README: &str = r##"Spooch frames
=============

Put a frame file here (name.toml) and it shows up in the output bar's
Frame list as soon as it's saved. A file with a mistake is skipped and
the reason logged.

A window frame:

    name = "My terminal"
    radius = 10            # corner radius (default 10)
    shadow = true          # false: never a shadow, whatever the bar says
    outline = "auto"       # auto, light, dark, none or #rrggbb
    outline-style = "solid"  # solid, dashed, double or ascii (dashed, + corners)
    border = 3             # edges left, right and below, in the top's colour
    colour = "#1d2a3a"     # the top's default colour: light, dark or #rrggbb
    ink = "#e6e6e6"        # text and icons on that colour (default: follows it)
    font = "mono"          # the titles' typeface: ui or mono

    [[rows]]               # the window top: one or more rows
    height = 28
    background = "top"     # top, field or well (all follow the colour)
    items = [
        { type = "space", width = 10 },
        { type = "prompt" },
        { type = "title", default = "terminal", placeholder = false },
        { type = "buttons" },
    ]

Items go left to right; title and address grow to fill what's left.

    space    width
    back, forward, reload, plus, share
    menu     style (kebab|hamburger)
    address  height, lock, align (left|center), fill (field|well),
             text (shows the title; default true), width
    buttons  minimise, maximise and close; size (default 1)
    prompt   a green >_
    title    default, align, placeholder (a quiet line when empty), width
    tab      fraction (of the row), min-width, close (a cross)
    dots     colours = ["#ff5f57", "#febc2e", "#28c840"]
    circles  count (hollow, in the ink)
    image    file (a PNG next to this file), height

A frame drawn in an image editor:

    name = "Polaroid"
    shadow = false
    [image]
    file = "polaroid.png"
    slice = [40, 40, 40, 140]    # left, top, right, bottom: corners kept,
                                 # the middle stretched to fit
    content = [30, 30, 30, 130]  # where the capture sits, from each edge

The built-in frames are written the same way; see
https://github.com/ycamper/spooch/tree/main/crates/spooch-render/src/document/frame/builtin
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builtins_parse() {
        for (key, text) in super::super::registry::BUILTIN {
            let spec = FrameSpec::parse(text).unwrap_or_else(|e| panic!("{key}: {e}"));
            assert!(!spec.name.is_empty());
        }
    }

    #[test]
    fn mistakes_say_what_is_wrong() {
        assert!(FrameSpec::parse("radius = 4").unwrap_err().contains("name"));
        let e = FrameSpec::parse("name = \"x\"\nradius = 999").unwrap_err();
        assert!(e.contains("radius"), "{e}");
        let e = FrameSpec::parse("name = \"x\"\ncolour = \"teal\"").unwrap_err();
        assert!(e.contains("colour"), "{e}");
        let e = FrameSpec::parse(
            "name = \"x\"\n[[rows]]\nheight = 20\nitems = [{ type = \"wobble\" }]",
        )
        .unwrap_err();
        assert!(e.contains("wobble"), "{e}");
        assert_eq!(
            parse_colour("#ff8000").unwrap(),
            Color4f::new(1.0, 128.0 / 255.0, 0.0, 1.0)
        );
    }
}
