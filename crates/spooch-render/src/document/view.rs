//! The editor window's view: the docked layout (top row, gallery, rail,
//! style row, footer bar), zoom and scrolling, and drawing the window.
//! Points in the window are window pixels; `to_doc` and `to_screen`
//! convert. The overlay has no view: its points are desktop pixels.

use crate::document::{fill, stroke};
use skia_safe::{
    BlurStyle, CubicResampler, Data, FilterMode, IRect, Image, MaskFilter, MipmapMode, Paint,
    PaintStyle, Point, RRect, Rect, SamplingOptions, images,
};

use super::chrome::{Panel, contains, panel};
use super::theme::{
    ACCENT, ACCENT_HOVER, BACKDROP, CLOSE_HOVER, DIVIDER, HOVER, ICON, MUTED, PANEL, TEXT,
    centred_baseline, ellipsize, hex,
};
use super::theme::{cross, outlined_pill};
use super::{Action, Document, canvas_surface};

/// The docked pieces' sizes, in screen pixels (times the display scale).
const TOP_H: f32 = 40.0;
const ROW_H: f32 = 36.0;
const FOOT_H: f32 = 36.0;
/// Room round the capture when it's fitted.
const FIT_PAD: f32 = 24.0;
/// The zoom steps Ctrl+= and Ctrl+- go through.
const ZOOMS: [f32; 15] = [
    0.1, 0.17, 0.25, 0.33, 0.5, 0.67, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0,
];
/// How far one notch of the mouse wheel scrolls, in screen pixels.
const WHEEL_STEP: f32 = 48.0;

/// How the capture is shown in the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct View {
    /// The window's client size, in screen pixels.
    pub size: (i32, i32),
    /// Screen pixels per document pixel.
    pub zoom: f32,
    /// Whether the zoom follows the window, fitting the capture.
    pub fit: bool,
    /// The document point shown at the middle of the canvas area.
    pub center: Point,
}

/// The window's docked areas, in window pixels.
pub(super) struct Dock {
    pub top: IRect,
    /// The gallery down the left edge, when it shows.
    pub gallery: Option<IRect>,
    pub rail: IRect,
    pub row: IRect,
    pub foot: IRect,
    /// What's left, where the capture shows.
    pub area: IRect,
}

/// The window's own buttons, at the right of the top row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowButton {
    Minimize,
    Maximize,
    Close,
}

/// What a point of the editor window is, so the shell can hand moving,
/// resizing and the window buttons to the window manager.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowPart {
    /// The editor's: the capture and the chrome.
    Client,
    /// The empty part of the top row, which moves the window.
    Caption,
    Button(WindowButton),
    /// A resize border: `x` and `y` are 0 for left or top, 1 for neither
    /// and 2 for right or bottom.
    Edge {
        x: u8,
        y: u8,
    },
}

impl WindowPart {
    /// The pointer shape over a resize border, for shells that draw their
    /// own (the window manager's frame does it elsewhere).
    pub fn cursor(self) -> Option<super::Cursor> {
        let WindowPart::Edge { x, y } = self else {
            return None;
        };
        Some(super::edge_cursor(x, y))
    }
}

/// Something on the top row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TopItem {
    /// Shows or hides the gallery.
    Gallery,
    Zoom,
    Copy,
    Window(WindowButton),
}

pub(super) struct TopLayout {
    pub rect: IRect,
    pub gallery: IRect,
    pub zoom: IRect,
    pub copy: IRect,
    /// Minimise, maximise and close, left to right.
    pub buttons: [(WindowButton, IRect); 3],
}

/// The window buttons' width, and the resize border, in screen pixels.
const BUTTON_W: f32 = 46.0;
const RESIZE_BORDER: f32 = 6.0;

impl Document {
    /// Show the editor in a window of `width` by `height` pixels, or tell
    /// it the window was resized. The first call fits the capture.
    pub fn set_window_size(&mut self, width: i32, height: i32) {
        let size = (width.max(1), height.max(1));
        match &mut self.view {
            Some(v) => v.size = size,
            None => {
                self.view = Some(View {
                    size,
                    zoom: 1.0,
                    fit: true,
                    center: Rect::from(self.canvas).center(),
                });
            }
        }
        self.chrome_key = None;
        self.settle();
    }

    /// The window's size and whether it's maximised, for the next
    /// capture's document shown in the same window.
    pub fn window_state(&self) -> Option<((i32, i32), bool)> {
        self.view.map(|v| (v.size, self.window_maximized))
    }

    /// Whether the editor is in a window rather than the overlay.
    pub fn in_window(&self) -> bool {
        self.view.is_some()
    }

    /// Screen pixels per document pixel: 1 in the overlay.
    pub fn zoom(&self) -> f32 {
        self.view.map_or(1.0, |v| v.zoom)
    }

    /// A window point as a document point; the same point in the overlay.
    pub(super) fn to_doc(&self, p: Point) -> Point {
        match (&self.view, self.dock()) {
            (Some(v), Some(d)) => {
                let a = Rect::from(d.area).center();
                v.center + (p - a) * (1.0 / v.zoom)
            }
            _ => p,
        }
    }

    /// A document point as a window point; the same point in the overlay.
    pub(super) fn to_screen(&self, p: Point) -> Point {
        match (&self.view, self.dock()) {
            (Some(v), Some(d)) => {
                let a = Rect::from(d.area).center();
                a + (p - v.center) * v.zoom
            }
            _ => p,
        }
    }

    /// A document rectangle in window pixels.
    pub(super) fn screen_rect(&self, r: Rect) -> Rect {
        let (a, b) = (
            self.to_screen(Point::new(r.left, r.top)),
            self.to_screen(Point::new(r.right, r.bottom)),
        );
        Rect::from_ltrb(a.x, a.y, b.x, b.y)
    }

    /// The docked areas, in the window.
    pub(super) fn dock(&self) -> Option<Dock> {
        let v = self.view?;
        let s = self.scale;
        let (w, h) = v.size;
        let px = |v: f32| (v * s).round() as i32;
        let top = IRect::from_ltrb(0, 0, w, px(TOP_H).min(h));
        let foot = IRect::from_ltrb(0, (h - px(FOOT_H)).max(top.bottom), w, h);
        let gallery = self
            .gallery_width(w)
            .map(|gw| IRect::from_ltrb(0, top.bottom, gw, foot.top));
        let left = gallery.map_or(0, |g| g.right);
        let rail = IRect::from_ltrb(left, top.bottom, left + self.rail_width(), foot.top);
        let row = IRect::from_ltrb(
            rail.right,
            top.bottom,
            w,
            (top.bottom + px(ROW_H)).min(foot.top),
        );
        let area = IRect::from_ltrb(rail.right, row.bottom, w, foot.top);
        Some(Dock {
            top,
            gallery,
            rail,
            row,
            foot,
            area,
        })
    }

    /// What the window shows, in document pixels: the canvas, and round it
    /// the frame and shadow picked in the bar, so they're seen as they'll
    /// be exported.
    fn shown_rect(&self) -> Rect {
        let c = self.canvas;
        match self.frame_geometry((c.width(), c.height())) {
            Some((w, h, at)) => Rect::from_xywh(
                (c.left - at.left) as f32,
                (c.top - at.top) as f32,
                w as f32,
                h as f32,
            ),
            None => Rect::from(c),
        }
    }

    /// The zoom that fits the capture in the area: never above 100%.
    fn fit_zoom(&self, area: IRect) -> f32 {
        let pad = 2.0 * FIT_PAD * self.scale;
        let c = self.shown_rect();
        let zx = (area.width() as f32 - pad).max(16.0) / c.width().max(1.0);
        let zy = (area.height() as f32 - pad).max(16.0) / c.height().max(1.0);
        zx.min(zy).min(1.0)
    }

    /// Keep the view sensible: fitted when it follows the window, and a
    /// capture smaller than the area centred in it, a bigger one kept from
    /// scrolling away. Not while dragging, so a drag doesn't shift under
    /// the pointer.
    pub(super) fn settle(&mut self) {
        self.settle_gallery();
        let Some(d) = self.dock() else { return };
        let dragging = self.is_dragging();
        let fit = self.fit_zoom(d.area);
        let c = self.shown_rect();
        let Some(v) = self.view.as_mut() else { return };
        if v.fit && !dragging {
            v.zoom = fit;
            v.center = c.center();
        }
        if dragging {
            return;
        }
        let scale = self.scale;
        let pad = FIT_PAD * scale / v.zoom;
        let axis = |centre: f32, lo: f32, hi: f32, room: i32| {
            if overflows(hi - lo, v.zoom, room, scale) {
                let half = room as f32 / 2.0 / v.zoom;
                centre.clamp(lo + half - pad, hi - half + pad)
            } else {
                (lo + hi) / 2.0
            }
        };
        v.center = Point::new(
            axis(v.center.x, c.left, c.right, d.area.width()),
            axis(v.center.y, c.top, c.bottom, d.area.height()),
        );
    }

    /// Zoom to `zoom`, keeping the document point under window point `at`
    /// (or the area's middle) where it is.
    fn zoom_to(&mut self, zoom: f32, at: Option<Point>) -> bool {
        let Some(d) = self.dock() else { return false };
        let at = at.unwrap_or_else(|| Rect::from(d.area).center());
        let under = self.to_doc(at);
        let a = Rect::from(d.area).center();
        let Some(v) = self.view.as_mut() else {
            return false;
        };
        let zoom = zoom.clamp(ZOOMS[0], ZOOMS[ZOOMS.len() - 1]);
        let changed = v.fit || (v.zoom - zoom).abs() > f32::EPSILON;
        v.fit = false;
        v.zoom = zoom;
        v.center = under - (at - a) * (1.0 / zoom);
        self.settle();
        self.fresh = false;
        changed
    }

    /// Fit the capture in the window, and keep fitting it as it resizes.
    pub(crate) fn zoom_fit(&mut self) -> bool {
        let Some(v) = self.view.as_mut() else {
            return false;
        };
        let was = v.fit;
        v.fit = true;
        self.settle();
        !was
    }

    /// Show the capture at its own size.
    pub(crate) fn zoom_actual(&mut self) -> bool {
        self.view.is_some() && self.zoom_to(1.0, None)
    }

    /// The next zoom step up (`larger`) or down, about window point `at`.
    pub(crate) fn zoom_step(&mut self, larger: bool, at: Option<Point>) -> bool {
        let Some(v) = self.view else { return false };
        let next = if larger {
            ZOOMS.iter().copied().find(|z| *z > v.zoom + 0.001)
        } else {
            ZOOMS.iter().rev().copied().find(|z| *z < v.zoom - 0.001)
        };
        match next {
            Some(z) => self.zoom_to(z, at),
            None => false,
        }
    }

    /// The wheel turned `notches` (positive away from the user) at `p`.
    /// Over the capture it steps the size unless the window has something
    /// to scroll. In the window Ctrl zooms about the pointer; otherwise it
    /// scrolls, sideways when `sideways` (Shift, or a sideways wheel).
    pub fn wheel(&mut self, p: Point, notches: f32, sideways: bool, ctrl: bool) -> bool {
        let Some(v) = self.view else {
            return !ctrl && !sideways && !self.on_chrome(p) && self.wheel_size(notches);
        };
        if let Some(moved) = self.gallery_wheel(p, notches) {
            return moved;
        }
        if ctrl {
            let z = v.zoom * 1.15f32.powf(notches);
            return self.zoom_to(z, Some(p));
        }
        if !sideways && !self.on_chrome(p) && !self.scrolls() {
            return self.wheel_size(notches);
        }
        let by = notches * WHEEL_STEP * self.scale / v.zoom;
        let before = v.center;
        if let Some(v) = self.view.as_mut() {
            if sideways {
                v.center.x -= by;
            } else {
                v.center.y -= by;
            }
        }
        self.settle();
        self.view.is_some_and(|v| v.center != before)
    }

    /// Whether the capture overflows the window's area, so it scrolls.
    fn scrolls(&self) -> bool {
        let (Some(d), Some(v)) = (self.dock(), self.view) else {
            return false;
        };
        let c = self.shown_rect();
        overflows(c.width(), v.zoom, d.area.width(), self.scale)
            || overflows(c.height(), v.zoom, d.area.height(), self.scale)
    }

    /// The top row: the zoom pill and Copy, then the window buttons at the
    /// right edge.
    pub(super) fn top_layout(&self) -> Option<TopLayout> {
        let d = self.dock()?;
        let s = self.scale;
        let bw = (BUTTON_W * s).round() as i32;
        let t = d.top;
        let buttons = [
            WindowButton::Minimize,
            WindowButton::Maximize,
            WindowButton::Close,
        ];
        let mut rects = [IRect::new_empty(); 3];
        for (i, r) in rects.iter_mut().enumerate() {
            *r = IRect::from_ltrb(
                t.right - (3 - i as i32) * bw,
                t.top,
                t.right - (2 - i as i32) * bw,
                t.bottom,
            );
        }
        let h = (26.0 * s).round() as i32;
        let y = d.top.top + (d.top.height() - h) / 2;
        let pad = (10.0 * s).round() as i32;
        let copy_w = (68.0 * s).round() as i32;
        let copy = IRect::from_xywh(rects[0].left - pad - copy_w, y, copy_w, h);
        let font = self.bar_font();
        let zoom_w = (self.text_width(font.as_ref(), &self.zoom_text()) + 26.0 * s).ceil() as i32;
        let zoom = IRect::from_xywh(copy.left - (8.0 * s).round() as i32 - zoom_w, y, zoom_w, h);
        let gallery = IRect::from_xywh(
            d.top.left + (8.0 * s).round() as i32,
            y,
            (30.0 * s).round() as i32,
            h,
        );
        Some(TopLayout {
            rect: d.top,
            gallery,
            zoom,
            copy,
            buttons: [
                (buttons[0], rects[0]),
                (buttons[1], rects[1]),
                (buttons[2], rects[2]),
            ],
        })
    }

    /// The window is maximised, or not: the maximise button turns into
    /// restore, and there are no resize borders.
    pub fn set_maximized(&mut self, on: bool) {
        if self.window_maximized != on {
            self.window_maximized = on;
            self.chrome_key = None;
        }
    }

    /// What window point `p` is: a resize border (unless maximised), a
    /// window button, the caption (the top row's empty part) or the
    /// editor's own.
    pub fn window_part(&self, p: Point) -> WindowPart {
        let Some(v) = self.view else {
            return WindowPart::Client;
        };
        if !self.window_maximized {
            let band = RESIZE_BORDER * self.scale;
            let (w, h) = (v.size.0 as f32, v.size.1 as f32);
            let side = |at: f32, end: f32| {
                if at < band {
                    0
                } else if at >= end - band {
                    2
                } else {
                    1
                }
            };
            let (x, y) = (side(p.x, w), side(p.y, h));
            if (x, y) != (1, 1) {
                return WindowPart::Edge { x, y };
            }
        }
        match self.top_hit(p) {
            Some(TopItem::Window(b)) => WindowPart::Button(b),
            Some(_) => WindowPart::Client,
            None if self.top_layout().is_some_and(|t| contains(&t.rect, p)) => WindowPart::Caption,
            None => WindowPart::Client,
        }
    }

    /// "Fit · 63%" while fitted, else "63%".
    fn zoom_text(&self) -> String {
        let Some(v) = self.view else {
            return String::new();
        };
        let pct = (v.zoom * 100.0).round() as i32;
        if v.fit {
            format!("Fit \u{b7} {pct}%")
        } else {
            format!("{pct}%")
        }
    }

    /// The top row item under window point `p`.
    pub(super) fn top_hit(&self, p: Point) -> Option<TopItem> {
        let t = self.top_layout()?;
        if let Some((b, _)) = t.buttons.iter().find(|(_, r)| contains(r, p)) {
            Some(TopItem::Window(*b))
        } else if contains(&t.gallery, p) {
            Some(TopItem::Gallery)
        } else if contains(&t.zoom, p) {
            Some(TopItem::Zoom)
        } else if contains(&t.copy, p) {
            Some(TopItem::Copy)
        } else {
            None
        }
    }

    /// A click on the top row.
    pub(super) fn press_top(&mut self, item: TopItem) {
        match item {
            // Fit and 100% are the two views people want; the wheel and
            // keys do the rest.
            TopItem::Zoom => {
                if self.view.is_some_and(|v| v.fit) {
                    self.zoom_actual();
                } else {
                    self.zoom_fit();
                }
            }
            TopItem::Gallery => {
                self.toggle_gallery();
            }
            TopItem::Copy => self.action = Some(Action::Copy),
            TopItem::Window(b) => self.action = Some(Action::Window(b)),
        }
        self.chrome_key = None;
    }

    pub(super) fn top_tip(&self, item: TopItem) -> &'static str {
        match item {
            TopItem::Gallery if self.gallery_on() => "Hide the gallery   Ctrl+H",
            TopItem::Gallery => "Show the gallery   Ctrl+H",
            TopItem::Zoom => "Fit or 100%   Ctrl+scroll, Ctrl+0, Ctrl+1",
            TopItem::Copy => "Ctrl+C   copies the selected marks when some are selected",
            TopItem::Window(WindowButton::Minimize) => "Minimise",
            TopItem::Window(WindowButton::Maximize) if self.window_maximized => "Restore",
            TopItem::Window(WindowButton::Maximize) => "Maximise",
            TopItem::Window(WindowButton::Close) => "Close   the capture stays in Recents",
        }
    }

    /// The top row's panel: the capture's name, quiet, then zoom and Copy.
    pub(super) fn draw_top(&self, t: &TopLayout, hovered: Option<TopItem>) -> Panel {
        let s = self.scale;
        let r = t.rect;
        let font = self.bar_font();
        let origin = Point::new(r.left as f32, r.top as f32);
        let zoom_text = self.zoom_text();
        panel(r, |canvas| {
            let Some(font) = &font else { return };
            let p = fill(MUTED);
            let mid = centred_baseline(font, r.height() as f32 / 2.0);
            // The gallery button: a sidebar glyph, its side filled while
            // the gallery shows.
            let gb = Rect::from(t.gallery).with_offset(-origin);
            if hovered == Some(TopItem::Gallery) {
                canvas.draw_rrect(
                    RRect::new_rect_xy(gb, 6.0 * s, 6.0 * s),
                    &Paint::new(HOVER, None),
                );
            }
            let c = gb.center();
            let glyph = Rect::from_ltrb(c.x - 7.0 * s, c.y - 5.5 * s, c.x + 7.0 * s, c.y + 5.5 * s);
            let side = Rect::from_ltrb(glyph.left, glyph.top, glyph.left + 5.0 * s, glyph.bottom);
            if self.gallery_on() {
                canvas.draw_rect(side, &Paint::new(ICON, None));
            }
            let g = stroke(ICON, s);
            canvas.draw_rrect(RRect::new_rect_xy(glyph, 2.0 * s, 2.0 * s), &g);
            canvas.draw_line((side.right, glyph.top), (side.right, glyph.bottom), &g);

            // The name, cut short before the zoom pill.
            let name_x = gb.right + 10.0 * s;
            let room = (t.zoom.left - r.left) as f32 - name_x - 14.0 * s;
            canvas.draw_str(ellipsize(font, &self.name, room), (name_x, mid), font, &p);

            let zoom = Rect::from(t.zoom).with_offset(-origin);
            outlined_pill(canvas, zoom, hovered == Some(TopItem::Zoom), s);
            let text = fill(TEXT);
            let tw = self.text_width(Some(font), &zoom_text);
            canvas.draw_str(&zoom_text, (zoom.center_x() - tw / 2.0, mid), font, &text);

            let copy = Rect::from(t.copy).with_offset(-origin);
            let bg = if hovered == Some(TopItem::Copy) {
                ACCENT_HOVER
            } else {
                ACCENT
            };
            let b = fill(bg);
            canvas.draw_rrect(RRect::new_rect_xy(copy, 6.0 * s, 6.0 * s), &b);
            let tw = self.text_width(Some(font), "Copy");
            canvas.draw_str("Copy", (copy.center_x() - tw / 2.0, mid), font, &text);

            // The window buttons: a quiet glyph each, the hovered one
            // lit (close in red, as Windows does).
            for (b, rect) in &t.buttons {
                let rect = Rect::from(*rect).with_offset(-origin);
                let lit = hovered == Some(TopItem::Window(*b));
                if lit {
                    let bg = if *b == WindowButton::Close {
                        CLOSE_HOVER
                    } else {
                        HOVER
                    };
                    canvas.draw_rect(rect, &Paint::new(bg, None));
                }
                let mut g = Paint::new(ICON, None);
                if lit && *b == WindowButton::Close {
                    g.set_color4f(TEXT, None);
                }
                g.set_anti_alias(true);
                g.set_style(PaintStyle::Stroke);
                g.set_stroke_width(s);
                let c = rect.center();
                let k = 5.0 * s;
                match b {
                    WindowButton::Minimize => {
                        canvas.draw_line((c.x - k, c.y), (c.x + k, c.y), &g);
                    }
                    WindowButton::Maximize if self.window_maximized => {
                        let back =
                            Rect::from_ltrb(c.x - k + 2.0 * s, c.y - k, c.x + k, c.y + k - 2.0 * s);
                        canvas.draw_rect(back, &g);
                        let front =
                            Rect::from_ltrb(c.x - k, c.y - k + 2.0 * s, c.x + k - 2.0 * s, c.y + k);
                        canvas.draw_rect(front, &Paint::new(PANEL, None));
                        canvas.draw_rect(front, &g);
                    }
                    WindowButton::Maximize => {
                        canvas.draw_rect(Rect::from_ltrb(c.x - k, c.y - k, c.x + k, c.y + k), &g);
                    }
                    WindowButton::Close => {
                        cross(canvas, c, k, &g);
                    }
                }
            }
        })
    }

    /// The whole window, ready to show: the docked chrome, and the capture
    /// at the current zoom with its outline and crop handles. Returns
    /// top-down premultiplied BGRA rows and their stride.
    pub fn render_window(&mut self) -> (&[u8], usize) {
        self.settle();
        let Some(v) = self.view else {
            return self.render();
        };
        let Some(d) = self.dock() else {
            return self.render();
        };
        let size = v.size;
        let stale = !self.fresh;
        let (w, h) = (self.canvas.width(), self.canvas.height());
        let framed = self.frame_geometry((w, h)).is_some();
        // Picking a frame, its colour, the shadow or the title changes the
        // picture as much as drawing does.
        let frame_key = Some((self.styles.framing, self.frame_title().to_string()));
        if stale || self.window_image.is_none() || frame_key != self.window_frame_key {
            self.window_frame_key = frame_key;
            let wants_frame = self.framing().is_some();
            let (pixels, stride) = self.render();
            // With a frame or shadow picked, show it as it'll be exported.
            // Without, skip copying the canvas out.
            let framed_image = if wants_frame {
                let content = super::render::tight(pixels, stride, w as u32, h as u32);
                self.draw_framed(&content, (w, h))
                    .map(|(mut surface, _)| surface.image_snapshot())
            } else {
                None
            };
            let snapshot = framed_image.unwrap_or_else(|| self.surface.image_snapshot());
            self.window_framed = framed;
            // Mipmaps keep a shrunk capture sharp.
            self.window_image = Some(snapshot.with_default_mipmaps().unwrap_or(snapshot));
        }
        let mut surface = match self.window_surface.take() {
            Some(s) if (s.width(), s.height()) == size => s,
            _ => match canvas_surface(size) {
                Some(s) => s,
                None => return self.render(),
            },
        };
        let dest = self.screen_rect(Rect::from(self.canvas));
        let shown = self.screen_rect(self.shown_rect());
        let image = self.window_image.clone();
        let panels: Vec<(IRect, Option<Image>)> = self
            .chrome()
            .iter()
            .map(|p| (p.rect, panel_image(p)))
            .collect();
        let s = self.scale;
        {
            let canvas = surface.canvas();
            canvas.clear(BACKDROP);
            // The capture, with a soft shadow, in the area.
            canvas.save();
            canvas.clip_rect(Rect::from(d.area), None, false);
            // A frame brings its own shadow, or none.
            if !self.window_framed {
                let mut shadow = Paint::new(skia_safe::Color4f::new(0.0, 0.0, 0.0, 0.45), None);
                shadow.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 10.0 * s, false));
                canvas.draw_rect(dest.with_offset((0.0, 4.0 * s)), &shadow);
            }
            if let Some(image) = &image {
                let sampling = if v.zoom >= 2.0 {
                    // Big zooms are for looking at pixels.
                    SamplingOptions::new(FilterMode::Nearest, MipmapMode::None)
                } else if v.zoom < 1.0 {
                    SamplingOptions::new(FilterMode::Linear, MipmapMode::Linear)
                } else {
                    SamplingOptions::from(CubicResampler::mitchell())
                };
                canvas.draw_image_rect_with_sampling_options(
                    image,
                    None,
                    shown,
                    sampling,
                    &Paint::default(),
                );
            }
            // The crop handles only when they're wanted: the pointer on an
            // edge, or an edge being dragged.
            let cropping = matches!(self.gesture, Some(super::Gesture::Canvas { .. }));
            if self.edge_hover || cropping {
                let frame = IRect::from_ltrb(
                    dest.left.round() as i32,
                    dest.top.round() as i32,
                    dest.right.round() as i32,
                    dest.bottom.round() as i32,
                );
                for (r, rgb) in crate::select::selection_frame(frame, true) {
                    canvas.draw_rect(Rect::from(r), &Paint::new(hex(rgb), None));
                }
            }
            canvas.restore();

            // The docked strips, with hairlines between them and the area.
            let bg = Paint::new(PANEL, None);
            for r in [d.top, d.rail, d.row, d.foot] {
                canvas.draw_rect(Rect::from(r), &bg);
            }
            let line = Paint::new(DIVIDER, None);
            let hair = s.max(1.0).round();
            for r in [
                Rect::from_ltrb(
                    0.0,
                    d.top.bottom as f32 - hair,
                    v.size.0 as f32,
                    d.top.bottom as f32,
                ),
                Rect::from_ltrb(
                    d.rail.right as f32 - hair,
                    d.rail.top as f32,
                    d.rail.right as f32,
                    d.rail.bottom as f32,
                ),
                Rect::from_ltrb(
                    d.row.left as f32,
                    d.row.bottom as f32 - hair,
                    d.row.right as f32,
                    d.row.bottom as f32,
                ),
                Rect::from_ltrb(
                    0.0,
                    d.foot.top as f32,
                    v.size.0 as f32,
                    d.foot.top as f32 + hair,
                ),
            ] {
                canvas.draw_rect(r, &line);
            }
            for (r, image) in &panels {
                if let Some(image) = image {
                    canvas.draw_image(image, (r.left, r.top), None);
                }
            }
        }
        self.window_surface = Some(surface);
        let pixmap = self
            .window_surface
            .as_mut()
            .and_then(|s| s.peek_pixels())
            .expect("raster surfaces expose their pixels");
        let stride = pixmap.row_bytes();
        (pixmap.bytes().expect("raster pixmaps have bytes"), stride)
    }
}

/// Whether `len` image pixels at `zoom`, with the fit padding each side,
/// are more than `room` window pixels.
fn overflows(len: f32, zoom: f32, room: i32, scale: f32) -> bool {
    len * zoom + 2.0 * FIT_PAD * scale > room as f32
}

fn panel_image(p: &Panel) -> Option<Image> {
    let (w, h) = (p.rect.width(), p.rect.height());
    if w <= 0 || h <= 0 || p.pixels.len() < (w * h * 4) as usize {
        return None;
    }
    let info = crate::bgra_info((w, h), skia_safe::AlphaType::Premul);
    images::raster_from_data(&info, Data::new_copy(&p.pixels), w as usize * 4)
}

#[cfg(test)]
mod tests {
    use super::super::GalleryAction;
    use super::super::tests::{drag, placed};
    use super::*;

    #[test]
    fn a_big_capture_fits_and_points_map_both_ways() {
        let (mut d, _) = placed();
        // A window too small for the 200x100 capture at 100%.
        d.set_window_size(220, 220);
        let v = d.view.unwrap();
        assert!(v.fit && v.zoom < 1.0, "{v:?}");
        let mid = Point::new(200.0, 150.0);
        let on_screen = d.to_screen(mid);
        let back = d.to_doc(on_screen);
        assert!((back - mid).length() < 0.01);
        // The capture's middle is the area's middle.
        let area = Rect::from(d.dock().unwrap().area);
        assert!((on_screen - area.center()).length() < 0.5);
        // A roomy window shows it at 100%, never larger.
        d.set_window_size(1200, 900);
        assert_eq!(d.zoom(), 1.0);
    }

    #[test]
    fn ctrl_wheel_zooms_about_the_pointer_and_keys_fit_or_reset() {
        let (mut d, _) = placed();
        // Zoomed past the window, the point under the pointer stays put. (A
        // capture that still fits stays centred instead.)
        d.set_window_size(400, 300);
        let at = d.to_screen(Point::new(180.0, 140.0));
        assert!(d.wheel(at, 6.0, false, true));
        assert!(d.zoom() > 2.0);
        // Plain scrolling moves the view, and back.
        assert!(d.wheel(at, -1.0, false, false));
        assert!(d.wheel(at, 1.0, false, false));
        let still = d.to_doc(at);
        assert!(
            (still - Point::new(180.0, 140.0)).length() < 1.0,
            "{still:?}"
        );
        assert!(d.zoom_actual());
        assert_eq!(d.zoom(), 1.0);
        assert!(d.zoom_step(true, None));
        assert_eq!(d.zoom(), 1.25);
        assert!(d.zoom_fit());
        assert!(d.view.unwrap().fit);
    }

    #[test]
    fn the_wheel_sizes_until_there_is_something_to_scroll() {
        let (mut d, _) = placed();
        d.set_window_size(1200, 900);
        let at = d.to_screen(Point::new(180.0, 140.0));
        let before = d.tool_style().size;
        assert!(d.wheel(at, 1.0, false, false));
        assert_eq!(d.tool_style().size, before + 1);
        // Zoomed past the window, it scrolls instead.
        d.set_window_size(400, 300);
        let at = d.to_screen(Point::new(180.0, 140.0));
        assert!(d.wheel(at, 6.0, false, true));
        assert!(d.wheel(at, -1.0, false, false));
        assert_eq!(d.tool_style().size, before + 1);
    }

    #[test]
    fn drawing_in_the_window_lands_in_document_pixels() {
        let (mut d, _) = placed();
        d.set_window_size(1200, 900);
        let a = d.to_screen(Point::new(120.0, 150.0));
        let b = d.to_screen(Point::new(280.0, 150.0));
        assert!(drag(&mut d, (a.x, a.y), (b.x, b.y)));
        assert_eq!(d.counts().arrows, 1);
        // The top row's Copy asks the shell to copy; the chrome is docked.
        let t = d.top_layout().unwrap();
        let c = Rect::from(t.copy).center();
        d.begin_drag(c);
        assert_eq!(d.take_action(), Some(Action::Copy));
        // The rail sits right of the gallery, or at the edge without it.
        let gallery = d.dock().unwrap().gallery.unwrap();
        assert_eq!(gallery.left, 0);
        assert_eq!(d.rail_layout().rect.left, gallery.right);
        assert!(d.on_chrome(Point::new(5.0, 300.0)));
        assert!(d.toggle_gallery());
        assert!(d.dock().unwrap().gallery.is_none());
        assert_eq!(d.rail_layout().rect.left, 0);
        // The whole window renders at its size.
        let (px, stride) = d.render_window();
        assert_eq!(stride, 1200 * 4);
        assert_eq!(px.len(), 1200 * 4 * 900);
    }

    #[test]
    fn the_gallery_opens_reveals_and_deletes_on_the_second_click() {
        let (mut d, _) = placed();
        d.set_window_size(1200, 900);
        let ids: Vec<String> = [
            "2026-10-02 09.00.00",
            "2026-10-04 10.00.00",
            "2026-10-04 11.00.00",
        ]
        .map(String::from)
        .to_vec();
        // The one being edited isn't kept yet, so it comes first.
        d.set_gallery(
            &ids,
            &Default::default(),
            &Default::default(),
            "2026-10-04 12.00.00",
            (2026, 10, 4),
        );
        assert_eq!(d.gallery_id(0), Some("2026-10-04 12.00.00"));
        assert_eq!(d.gallery_id(3), Some("2026-10-02 09.00.00"));
        assert_eq!(d.gallery_missing().len(), 4);
        d.set_gallery_thumb("2026-10-04 11.00.00", 2, 2, &[255; 16]);
        assert_eq!(d.gallery_missing().len(), 3);

        let l = d.gallery_layout().unwrap();
        let card = |d: &Document, i: usize| {
            d.gallery_layout()
                .unwrap()
                .lines
                .iter()
                .find(|(line, _)| *line == super::super::gallery::Line::Card(i))
                .map(|(_, r)| *r)
                .unwrap()
        };
        // Today's three under one heading, then the 2nd under its own.
        assert_eq!(l.lines.len(), 6);
        // A click on the capture being edited does nothing; on another,
        // it asks the session.
        d.begin_drag(card(&d, 0).center());
        assert_eq!(d.take_action(), None);
        d.begin_drag(card(&d, 1).center());
        assert_eq!(
            d.take_action(),
            Some(Action::Gallery(GalleryAction::Press(1)))
        );
        // Its buttons: reveal at once, delete only on the second click.
        let (reveal, delete) = d.badges(1, card(&d, 1));
        d.begin_drag(reveal.center());
        assert_eq!(
            d.take_action(),
            Some(Action::Gallery(GalleryAction::Reveal(1)))
        );
        d.begin_drag(delete.center());
        assert_eq!(d.take_action(), None);
        let (_, delete) = d.badges(1, card(&d, 1));
        d.begin_drag(delete.center());
        assert_eq!(
            d.take_action(),
            Some(Action::Gallery(GalleryAction::Delete(1)))
        );
        // The range pill narrows it to today.
        d.begin_drag(Rect::from(l.range).center());
        assert_eq!(d.gallery_layout().unwrap().lines.len(), 4);
    }

    #[test]
    fn a_frame_shows_round_the_capture_and_fit_makes_room_for_it() {
        let (mut d, _) = placed();
        d.set_window_size(400, 300);
        let plain = d.zoom();
        d.set_frame(super::super::Frame::builtin("terminal-modern"));
        let shown = d.shown_rect();
        let c = Rect::from(d.canvas);
        assert!(shown.left < c.left && shown.top < c.top && shown.bottom > c.bottom);
        d.render_window();
        assert!(d.window_framed);
        assert!(d.zoom() < plain, "fit shrinks to fit the frame too");
        // A new colour shows at once, without touching the capture.
        let before = d.window_frame_key.clone();
        d.set_frame_colour(d.frame_colour() + 1);
        d.render_window();
        assert_ne!(d.window_frame_key, before);
        // The crop handles show only on an edge.
        let inside = d.to_screen(Point::new(150.0, 150.0));
        d.pointer_moved(inside);
        assert!(!d.edge_hover);
        let edge = d.to_screen(Point::new(300.0, 150.0));
        assert!(d.pointer_moved(edge));
        assert!(d.edge_hover);
        // Points still land on the capture itself.
        let p = d.to_screen(Point::new(150.0, 150.0));
        assert!((d.to_doc(p) - Point::new(150.0, 150.0)).length() < 0.01);
    }

    #[test]
    fn window_parts_resize_move_and_buttons() {
        let (mut d, _) = placed();
        d.set_window_size(1000, 700);
        assert_eq!(
            d.window_part(Point::new(2.0, 2.0)),
            WindowPart::Edge { x: 0, y: 0 }
        );
        assert_eq!(
            d.window_part(Point::new(500.0, 698.0)),
            WindowPart::Edge { x: 1, y: 2 }
        );
        assert_eq!(d.window_part(Point::new(200.0, 20.0)), WindowPart::Caption);
        assert_eq!(d.window_part(Point::new(500.0, 400.0)), WindowPart::Client);
        let t = d.top_layout().unwrap();
        let close = Rect::from(t.buttons[2].1).center();
        assert_eq!(
            d.window_part(close),
            WindowPart::Button(WindowButton::Close)
        );
        // Clicking it asks the shell to close.
        d.begin_drag(close);
        assert_eq!(d.take_action(), Some(Action::Window(WindowButton::Close)));
        // Maximised: no resize borders.
        d.set_maximized(true);
        assert_eq!(d.window_part(Point::new(2.0, 400.0)), WindowPart::Client);
        assert_eq!(
            WindowPart::Edge { x: 2, y: 0 }.cursor(),
            Some(super::super::Cursor::ResizeDiagonalUp)
        );
    }

    #[test]
    fn copied_marks_paste_offset_and_selected_as_one_undo_step() {
        let (mut d, _) = placed();
        assert!(d.copy_selected().is_none());
        assert!(drag(&mut d, (120.0, 150.0), (280.0, 150.0)));
        super::super::tests::click(&mut d, 200.0, 150.0);
        let copied = d.copy_selected().unwrap();
        assert!(d.paste(&copied));
        assert_eq!(d.counts().arrows, 2);
        assert!(d.has_selection());
        assert!(d.undo());
        assert_eq!(d.counts().arrows, 1);
        // Into another capture: kept on its canvas.
        let (mut other, _) = placed();
        other.shift(500, 500);
        assert!(other.paste(&copied));
        assert_eq!(other.counts().arrows, 1);
        assert!(!other.paste("not marks"));
    }
}
