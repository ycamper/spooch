//! Frames round the exported image: a rounded card, or a
//! browser or terminal window, for docs and blog posts. Each frame is data
//! in the `spec` format: the built-ins in `builtin/*.toml`, and anything in
//! the frames folder (`registry::load_frames`). `draw` turns a spec into
//! pixels. The editor shows the capture plain; holding Alt shows the framed
//! result in place of all the chrome.
//!
//! Adding a built-in frame is a TOML file and a line in
//! `registry::BUILTIN`; a new kind of item is a variant of `spec::Item`
//! and a match arm in `draw::draw_item`.

mod draw;
mod registry;
mod spec;

use crate::document::fill;
use std::cell::RefCell;
use std::rc::Rc;

use super::theme::hex;
use skia_safe::{ClipOp, Color4f, Data, Font, Paint, Point, Rect, images, surfaces};

pub use draw::SHADOWS;
pub use registry::load_frames;

use super::Document;
use super::chrome::{Panel, panel};
use registry::Loaded;

const LIGHT: Color4f = Color4f::new(0.925, 0.929, 0.941, 1.0);
const DARK: Color4f = Color4f::new(0.114, 0.122, 0.141, 1.0);

/// Frame colour `index`'s own colour, if it has one (Default doesn't).
pub(super) fn swatch(index: u8) -> Option<Color4f> {
    FRAME_COLOURS.get(usize::from(index)).and_then(|(_, c)| *c)
}

/// What the Frame list calls having none.
pub(super) const NO_FRAME: &str = "No frame";

/// Colours for a window frame's top: the frame's own first, then tinted
/// neutrals that sit back behind the screenshot.
pub const FRAME_COLOURS: [(&str, Option<Color4f>); 10] = [
    ("Default", None),
    ("Light", Some(LIGHT)),
    ("Paper", Some(hex(0xf3eee3))),
    ("Dark", Some(DARK)),
    ("Midnight", Some(hex(0x1d263a))),
    ("Mocha", Some(hex(0x222232))),
    ("Nord", Some(hex(0x2d3642))),
    ("Forest", Some(hex(0x223c2e))),
    ("Clay", Some(hex(0x684231))),
    ("Plum", Some(hex(0x41293f))),
];

/// A frame, built in or loaded from the frames folder. A small id, so the
/// tool styles that keep it stay `Copy`; saved documents use its `key`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame(u16);

impl Frame {
    /// No frame: the capture as it is.
    pub const NONE: Frame = Frame(0);

    /// A built-in frame by key ("rounded", "browser-tab", ...), or no frame
    /// if there's no such key.
    pub fn builtin(key: &str) -> Frame {
        Frame::from_key(key).unwrap_or(Frame::NONE)
    }

    /// The frame with `key`, if it's loaded.
    pub fn from_key(key: &str) -> Option<Frame> {
        registry::id_of(key).map(Frame)
    }

    /// How saved documents name it.
    pub fn key(self) -> String {
        registry::key_of(self.0)
    }

    /// The name in the bar and its list.
    pub fn name(self) -> String {
        self.loaded()
            .map_or_else(|| NO_FRAME.to_string(), |f| f.spec.name.clone())
    }

    /// Every frame the list shows, with its name: no frame, the built-ins,
    /// then the frames folder's.
    pub fn all() -> Vec<(Frame, String)> {
        registry::listed()
            .into_iter()
            .map(|(id, name)| (Frame(id), name))
            .collect()
    }

    /// Whether it has a window top or edges, and so a colour.
    pub fn has_colour(self) -> bool {
        self.loaded()
            .is_some_and(|f| !f.spec.rows.is_empty() || f.spec.border > 0.0)
    }

    /// Whether it has a window top, and so a title.
    pub fn has_title(self) -> bool {
        self.loaded().is_some_and(|f| !f.spec.rows.is_empty())
    }

    fn loaded(self) -> Option<Rc<Loaded>> {
        registry::get(self.0)
    }
}

/// A finished image: straight (not premultiplied) BGRA rows, tightly
/// packed. Opaque unless it has a frame.
pub struct Exported {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

thread_local! {
    /// The title the window frames show (an address, a tab's title, a
    /// terminal's name), kept from one capture to the next.
    static LAST_TITLE: RefCell<String> = const { RefCell::new(String::new()) };
}

pub(super) fn last_title() -> String {
    LAST_TITLE.with(|t| t.borrow().clone())
}

impl Document {
    pub fn frame(&self) -> Frame {
        self.styles.frame
    }

    pub fn set_frame(&mut self, frame: Frame) {
        self.styles.frame = frame;
        self.chrome_key = None;
    }

    /// The title the window frames show; empty for a placeholder.
    pub(crate) fn frame_title(&self) -> &str {
        &self.frame_title
    }

    /// Set the frame's title, which the next capture starts with too.
    pub fn set_frame_title(&mut self, title: &str) {
        self.frame_title = title.to_string();
        LAST_TITLE.with(|t| *t.borrow_mut() = self.frame_title.clone());
        self.chrome_key = None;
    }

    /// Whether keys are typing into the frame's title in the bar.
    pub(crate) fn editing_title(&self) -> bool {
        self.editing_title
    }

    pub(super) fn set_editing_title(&mut self, on: bool) {
        if self.editing_title != on {
            self.editing_title = on;
            self.chrome_key = None;
        }
    }

    /// The window frames' colour, into `FRAME_COLOURS`.
    pub fn frame_colour(&self) -> u8 {
        self.styles.frame_colour
    }

    pub(crate) fn set_frame_colour(&mut self, index: u8) {
        self.styles.frame_colour = index % FRAME_COLOURS.len() as u8;
        self.chrome_key = None;
    }

    /// The drop shadow, into `SHADOWS`.
    pub fn shadow(&self) -> u8 {
        self.styles.shadow
    }

    pub fn set_shadow(&mut self, level: u8) {
        self.styles.shadow = level % SHADOWS.len() as u8;
        self.chrome_key = None;
    }

    /// What frame colour `index` looks like on the frame in use: its own
    /// colour for Default.
    pub(super) fn frame_colour_shown(&self, index: u8) -> Color4f {
        match self.frame().loaded() {
            Some(f) => draw::top_colour(&f, index),
            None => swatch(index).unwrap_or(LIGHT),
        }
    }

    /// The colour the frame's top is drawn in now, for the bar's swatch.
    pub(super) fn frame_top_colour(&self) -> Color4f {
        self.frame_colour_shown(self.frame_colour())
    }

    /// The finished image with its frame: what copy, save and drag-out
    /// produce. Straight BGRA, with transparency round a frame.
    pub fn export_image(&mut self) -> Exported {
        let (w, h) = (self.width() as i32, self.height() as i32);
        let content = self.export();
        let Some((mut surface, _)) = self.draw_framed(&content, (w, h)) else {
            return Exported {
                width: w as u32,
                height: h as u32,
                bgra: content,
            };
        };
        let (fw, fh) = (surface.width(), surface.height());
        let straight = crate::bgra_info((fw, fh), skia_safe::AlphaType::Unpremul);
        let mut bgra = vec![0u8; (fw * fh * 4) as usize];
        surface.read_pixels(&straight, &mut bgra, fw as usize * 4, (0, 0));
        Exported {
            width: fw as u32,
            height: fh as u32,
            bgra,
        }
    }

    /// Where the frame and shadow put content of `w` by `h`: the framed
    /// size and where the content sits in it. `None` with neither.
    pub(super) fn frame_geometry(
        &self,
        (w, h): (i32, i32),
    ) -> Option<(i32, i32, skia_safe::IRect)> {
        let (frame, sh) = self.framing()?;
        let g = draw::geometry(frame.as_deref(), sh, w, h, self.scale);
        Some((g.width, g.height, g.content))
    }

    /// The frame and shadow in use, or `None` with neither.
    pub(super) fn framing(&self) -> Option<(Option<Rc<Loaded>>, draw::Shadow)> {
        let frame = self.frame().loaded();
        let sh = draw::Shadow::of(self.shadow(), frame.as_deref());
        (frame.is_some() || !sh.is_none()).then_some((frame, sh))
    }

    /// The frame and shadow drawn round `content` (`w` by `h`, opaque BGRA
    /// rows) on a fresh premultiplied surface, and where the content sits
    /// on it. The export and the editor window both draw frames this way.
    pub(super) fn draw_framed(
        &self,
        content: &[u8],
        (w, h): (i32, i32),
    ) -> Option<(skia_safe::Surface, skia_safe::IRect)> {
        let (frame, sh) = self.framing()?;
        let g = draw::geometry(frame.as_deref(), sh, w, h, self.scale);
        let premul = crate::bgra_info((g.width, g.height), skia_safe::AlphaType::Premul);
        let mut surface = surfaces::raster(&premul, None, None)?;
        let canvas = surface.canvas();
        canvas.clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
        draw::draw(
            canvas,
            frame.as_deref(),
            sh,
            self.styles.frame_colour,
            &self.frame_title,
            content,
            (w, h),
            self.scale,
        );
        Some((surface, g.content))
    }

    /// Hold Alt to see the result: all the chrome gives way to one panel
    /// covering the screen, with the framed image in the middle.
    pub fn set_preview(&mut self, on: bool) -> bool {
        // Mid-drag, Alt means "don't snap", not the preview.
        if self.preview == on || (on && self.is_dragging()) {
            return false;
        }
        self.preview = on;
        self.chrome_key = None;
        self.fresh = false;
        true
    }

    /// The preview panel: the screen darkened, the framed image centred
    /// (shrunk to fit if it has to be), and a line saying what it is.
    pub(super) fn draw_preview(&mut self) -> Panel {
        let screen = self.screen();
        let out = self.export_image();
        let s = self.scale;
        let ui = self.ui_typeface.clone();
        let frame = self.frame();
        panel(screen, |canvas| {
            canvas.clear(Color4f::new(0.075, 0.08, 0.09, 0.96));
            let (sw, sh) = (screen.width() as f32, screen.height() as f32);
            let room = (sw - 80.0 * s, sh - 120.0 * s);
            let fit = (room.0 / out.width as f32)
                .min(room.1 / out.height as f32)
                .min(1.0);
            let (dw, dh) = (out.width as f32 * fit, out.height as f32 * fit);
            let at = Point::new((sw - dw) / 2.0, (sh - dh) / 2.0 - 16.0 * s);
            let info = crate::bgra_info(
                (out.width as i32, out.height as i32),
                skia_safe::AlphaType::Unpremul,
            );
            if let Some(image) =
                images::raster_from_data(&info, Data::new_copy(&out.bgra), out.width as usize * 4)
            {
                // A light checkerboard behind, so transparency shows.
                let back = Rect::from_xywh(at.x, at.y, dw, dh);
                canvas.save();
                canvas.clip_rect(back, ClipOp::Intersect, false);
                let cell = 12.0 * s;
                let light = Paint::new(Color4f::new(0.86, 0.87, 0.89, 1.0), None);
                let dark = Paint::new(Color4f::new(0.78, 0.79, 0.82, 1.0), None);
                canvas.draw_rect(back, &light);
                let mut y = 0;
                while (y as f32) * cell < dh {
                    let mut x = y % 2;
                    while (x as f32) * cell < dw {
                        canvas.draw_rect(
                            Rect::from_xywh(
                                at.x + x as f32 * cell,
                                at.y + y as f32 * cell,
                                cell,
                                cell,
                            ),
                            &dark,
                        );
                        x += 2;
                    }
                    y += 1;
                }
                canvas.restore();
                let sampling = skia_safe::SamplingOptions::new(
                    skia_safe::FilterMode::Linear,
                    skia_safe::MipmapMode::Linear,
                );
                canvas.draw_image_rect_with_sampling_options(
                    &image,
                    None,
                    Rect::from_xywh(at.x, at.y, dw, dh),
                    sampling,
                    &Paint::default(),
                );
            }
            if let Some(tf) = ui {
                let font = Font::from_typeface(tf, 13.0 * s);
                let text = format!(
                    "Preview: {}, {} \u{00d7} {}. Let go of Alt to keep editing.",
                    frame.name(),
                    out.width,
                    out.height
                );
                let (w, _) = font.measure_str(&text, None);
                let p = fill(Color4f::new(0.85, 0.86, 0.88, 1.0));
                canvas.draw_str(&text, ((sw - w) / 2.0, sh - 40.0 * s), &font, &p);
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::placed;
    use super::*;
    use skia_safe::IRect;

    #[test]
    fn no_frame_exports_the_canvas_as_it_is() {
        let (mut d, _) = placed();
        let plain = d.export();
        let out = d.export_image();
        assert_eq!((out.width, out.height), (200, 100));
        assert_eq!(out.bgra, plain);
    }

    #[test]
    fn every_builtin_adds_room_its_top_and_transparency() {
        let (mut d, _) = placed();
        let plain = d.export();
        for (frame, name) in Frame::all().into_iter().skip(1) {
            d.set_frame(frame);
            let out = d.export_image();
            let spec = &frame.loaded().unwrap().spec;
            let (bar, b) = (spec.top_height() as u32, spec.border as u32);
            assert_eq!(out.width, 200 + 2 * b + 64, "{name}");
            assert_eq!(out.height, 100 + bar + b + 64 + 6, "{name}");
            let at = |x: u32, y: u32| {
                let i = ((y * out.width + x) * 4) as usize;
                &out.bgra[i..i + 4]
            };
            // The corner is transparent; the content's middle is the
            // capture, untouched.
            assert_eq!(at(0, 0)[3], 0, "{name}");
            let (cx, cy) = (32 + b + 100, 32 + bar + 50);
            let i = ((50 * 200 + 100) * 4) as usize;
            assert_eq!(at(cx, cy), &plain[i..i + 4], "{name}");
        }
        // The frame is kept with the styles, for the next capture.
        assert_eq!(d.styles().frame, Frame::builtin("rounded"));
        // A document saved with a frame that's since been retired gets
        // its nearest one.
        assert_eq!(
            Frame::from_key("terminal-prompt"),
            Some(Frame::builtin("terminal-modern"))
        );
    }

    #[test]
    fn shadows_work_with_or_without_a_frame() {
        let (mut d, _) = placed();
        let plain = d.export();
        // Auto, the default: none without a frame.
        assert_eq!(d.shadow(), 0);
        assert_eq!(d.export_image().bgra, plain);
        // No frame, a soft shadow: room round the capture, see-through
        // corners, the capture untouched.
        d.set_shadow(2);
        let out = d.export_image();
        let m = (8.0f32 * 8.0 / 3.0).round() as u32;
        assert_eq!((out.width, out.height), (200 + 2 * m, 100 + 2 * m + 4));
        assert_eq!(out.bgra[3], 0);
        let i = (((m + 50) * out.width + m + 100) * 4) as usize;
        let j = ((50 * 200 + 100) * 4) as usize;
        assert_eq!(&out.bgra[i..i + 4], &plain[j..j + 4]);
        // With a frame, Auto is the standard shadow, None sits snug, and
        // Deep leaves more room.
        d.set_frame(Frame::builtin("rounded"));
        d.set_shadow(0);
        let auto = d.export_image().width;
        d.set_shadow(3);
        assert_eq!(d.export_image().width, auto);
        d.set_shadow(1);
        let edges = 2 * Frame::builtin("rounded").loaded().unwrap().spec.border as u32;
        assert_eq!(d.export_image().width, 200 + edges);
        d.set_shadow(4);
        assert!(d.export_image().width > auto);
        // It steps round.
        d.set_shadow(5);
        assert_eq!(d.shadow(), 0);
    }

    #[test]
    fn alt_previews_the_result_instead_of_the_chrome() {
        let (mut d, _) = placed();
        d.set_frame(Frame::builtin("rounded"));
        let normal = d.chrome().len();
        assert!(normal > 1);
        assert!(d.set_preview(true));
        assert!(!d.set_preview(true));
        let panels = d.chrome();
        assert_eq!(panels.len(), 1, "one preview panel, nothing else");
        assert_eq!(panels[0].rect, IRect::from_wh(400, 300));
        assert!(d.set_preview(false));
        assert_eq!(d.chrome().len(), normal);
        // Default colours come from each frame's file; any colour works.
        let quiet = Frame::builtin("browser-mac").loaded().unwrap();
        let prompt = Frame::builtin("terminal-modern").loaded().unwrap();
        assert!(draw::is_light(draw::top_colour(&quiet, 0)));
        assert!(!draw::is_light(draw::top_colour(&prompt, 0)));
        assert_eq!(draw::top_colour(&prompt, 4), FRAME_COLOURS[4].1.unwrap());
    }

    #[test]
    fn frames_load_from_a_folder_and_bad_ones_are_reported() {
        let dir = std::env::temp_dir().join(format!("spooch-frames-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("mine.toml"),
            "name = \"Mine\"\ncolour = \"#336699\"\n[[rows]]\nheight = 24\n\
             items = [{ type = \"dots\", colours = [\"#ff0000\"] }, { type = \"title\" }]\n",
        )
        .unwrap();
        std::fs::write(dir.join("broken.toml"), "name = \"Broken\"\nradius = 500\n").unwrap();
        let problems = load_frames(&dir);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("broken.toml") && problems[0].contains("radius"));
        assert!(dir.join("README.txt").exists(), "a README for people");
        let mine = Frame::from_key("file:mine").expect("loaded");
        assert_eq!(mine.name(), "Mine");
        assert!(Frame::all().iter().any(|(f, _)| *f == mine));
        let (mut d, _) = placed();
        d.set_frame(mine);
        let out = d.export_image();
        assert_eq!(out.height, 100 + 24 + 64 + 6);
        // Taken out of the folder, it's gone from the list on reload, and
        // a document holding it exports plain rather than failing.
        std::fs::remove_file(dir.join("mine.toml")).unwrap();
        load_frames(&dir);
        assert!(Frame::from_key("file:mine").is_none());
        assert_eq!(d.export_image().height, 100);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_frame_list_and_the_title() {
        use super::super::bar::BarItem;
        use super::super::{Key, KeyOutcome, Mods};
        let (mut d, _) = placed();
        d.set_screens(vec![IRect::from_ltrb(0, 0, 1000, 800)]);
        let item = |d: &Document, which: BarItem| {
            d.bar_layout()
                .items
                .into_iter()
                .find(|(i, ..)| *i == which)
                .map(|(_, r, _)| Rect::from(r).center())
        };
        // The Frame button opens a list of every frame; a row picks one.
        d.begin_drag(item(&d, BarItem::Frame).unwrap());
        let (_, _, rows) = d.bar_menu_layout(&d.bar_layout()).expect("open");
        assert_eq!(rows.len(), Frame::all().len());
        d.begin_drag(Rect::from(rows[6]).center());
        assert_eq!(d.frame(), Frame::all()[6].0);
        assert!(d.bar_menu_layout(&d.bar_layout()).is_none(), "closed");
        // The colour swatch opens a list of every colour; a row picks one.
        d.begin_drag(item(&d, BarItem::FrameColour).expect("a swatch"));
        let (_, _, rows) = d.bar_menu_layout(&d.bar_layout()).expect("open");
        assert_eq!(rows.len(), FRAME_COLOURS.len());
        d.begin_drag(Rect::from(rows[4]).center());
        assert_eq!(d.frame_colour(), 4);
        assert!(d.bar_menu_layout(&d.bar_layout()).is_none(), "closed");
        // A window frame shows a title box; typing fills it, Enter ends.
        d.begin_drag(item(&d, BarItem::FrameTitle).expect("a title box"));
        assert!(d.editing_title());
        for c in "zsh".chars() {
            assert_eq!(d.key(Key::Char(c), Mods::default()), KeyOutcome::Ignored);
            assert!(d.type_char(c, Point::new(0.0, 0.0)));
        }
        d.key(Key::Backspace, Mods::default());
        d.type_char('!', Point::new(0.0, 0.0));
        assert_eq!(
            d.key(Key::Enter, Mods::default()),
            KeyOutcome::Handled { changed: true }
        );
        assert!(!d.editing_title());
        assert_eq!(d.frame_title(), "zs!");
        // The next capture starts with it, and it's saved.
        let (next, _) = placed();
        assert_eq!(next.frame_title(), "zs!");
        let json = d.to_json("t");
        assert!(json.contains("zs!"));
        // No title box or colour without a window top or edges.
        d.set_frame(Frame::builtin("rounded"));
        assert!(item(&d, BarItem::FrameTitle).is_none());
        assert!(item(&d, BarItem::FrameColour).is_none());
    }
}
