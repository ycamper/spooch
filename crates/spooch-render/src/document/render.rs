//! Drawing the document: the frozen pixels, every mark, the selection's
//! outlines and handles, and the finished image for export.

use super::*;

impl Document {
    /// The base pixels with every redaction applied, positioned like them.
    pub(super) fn backdrop(&mut self) -> Image {
        let key: RedactionKey = self
            .marks
            .iter()
            .filter_map(|m| match m {
                Mark::Redact(r, kind) => Some((*r, *kind)),
                _ => None,
            })
            .collect();
        if let Some((built, image)) = &self.backdrop
            && *built == key
        {
            return image.clone();
        }
        let image = if key.is_empty() {
            self.base.image.clone()
        } else {
            let mut surface = self
                .surface
                .new_surface_with_dimensions(self.base.image.dimensions())
                .expect("a surface like the document's");
            let canvas = surface.canvas();
            canvas.draw_image(&self.base.image, (0, 0), None);
            // Redactions are in desktop pixels; this surface starts at the
            // base's corner.
            canvas.translate(-self.base.at);
            for &(r, kind) in &key {
                Mark::Redact(r, kind).draw(canvas, &self.base, None, self.scale, 0);
            }
            surface.image_snapshot()
        };
        self.backdrop = Some((key, image.clone()));
        image
    }

    /// Render the canvas and return premultiplied BGRA pixels with their
    /// row stride. Cheap when nothing changed since the last call.
    pub fn render(&mut self) -> (&[u8], usize) {
        if !self.fresh {
            let size = (self.canvas.width(), self.canvas.height());
            if (self.surface.width(), self.surface.height()) != size
                && let Some(s) = canvas_surface(size)
            {
                self.surface = s;
            }
            let preview = match &self.gesture {
                Some(Gesture::Draw { drag, path, .. }) => self.shape(*drag, path),
                _ => None,
            };
            let order = self.draw_order();
            let numbers = self.step_numbers();
            // A step being placed gets the next number.
            let next_step = numbers.iter().max().copied().unwrap_or(0) + 1;
            let backdrop = self.backdrop();
            let origin = Point::new(self.canvas.left as f32, self.canvas.top as f32);
            let px = self.ui_px();
            let canvas = self.surface.canvas();
            canvas.save();
            canvas.translate(-origin);
            canvas.draw_image(&backdrop, self.base.at, None);
            let tf = self.typeface.as_ref();
            // Spotlights dim everything else first, marks staying bright.
            let holes: Vec<_> = self
                .marks
                .iter()
                .chain(preview.as_ref())
                .filter_map(|m| match m {
                    Mark::Highlight(h) if h.look == crate::HighlightLook::Spotlight => {
                        Some(h.hole())
                    }
                    _ => None,
                })
                .collect();
            if !holes.is_empty() {
                let area = Rect::from(self.canvas);
                crate::dim_around(canvas, &backdrop, self.base.at, area, &holes);
            }
            // Redactions are already in the backdrop.
            for &i in &order {
                let m = &self.marks[i];
                if !matches!(m, Mark::Redact(..)) {
                    m.draw(canvas, &self.base, tf, self.scale, numbers[i]);
                }
            }
            if let Some(HoverMark::Mark(i)) = self.hover_mark
                && let Some(m) = self.marks.get(i)
            {
                draw_outline(canvas, m, tf, px, true);
            }
            if self.also.is_empty() {
                if let Some(m) = self.selected.and_then(|i| self.marks.get(i)) {
                    let grip = match self.hover_mark {
                        Some(HoverMark::Grip(g)) => Some(g),
                        _ => None,
                    };
                    draw_selection(canvas, m, tf, px, grip);
                }
            } else {
                // Several selected: each outlined, no handles.
                for i in self.selected.iter().chain(&self.also) {
                    if let Some(m) = self.marks.get(*i) {
                        draw_outline(canvas, m, tf, px, false);
                    }
                }
            }
            if let (Some(l), Some(tf)) = (&self.typing, tf) {
                // An auto edge follows the background while typing too.
                let mut l = l.clone();
                if l.look.auto_edge {
                    l.look.edge = Some(Mark::Label(l.clone()).auto_edge(&self.base, Some(tf)));
                }
                let turned = l.angle != 0.0;
                if turned {
                    canvas.save();
                    canvas.rotate(l.angle.to_degrees(), Some(l.pivot(tf)));
                }
                l.draw(canvas, tf, Some(self.caret));
                if turned {
                    canvas.restore();
                }
            }
            // While turning, the angle beside the handle.
            if let Some(Gesture::Edit {
                index,
                grip: Some(Grip::Rotate),
                ..
            }) = &self.gesture
                && let Some(m) = self.marks.get(*index)
                && let Some(ui) = &self.ui_typeface
                && let Some((_, handle)) = grips_of(m, tf, px)
                    .into_iter()
                    .find(|(g, _)| *g == Grip::Rotate)
            {
                let deg = m.angle().unwrap_or(0.0).to_degrees().round() as i32;
                draw_badge(canvas, ui, &format!("{deg}\u{b0}"), handle, px);
            }
            if let Some(m) = preview {
                m.draw(canvas, &self.base, tf, self.scale, next_step);
            }
            guides::draw_guides(canvas, &self.guides, px);
            canvas.restore();
            self.fresh = true;
        }
        let pixmap = self
            .surface
            .peek_pixels()
            .expect("raster surfaces expose their pixels");
        let stride = pixmap.row_bytes();
        (pixmap.bytes().expect("raster pixmaps have bytes"), stride)
    }

    /// The finished canvas as tightly packed opaque BGRA, without selection
    /// handles. A label still being typed is kept (without its caret), as
    /// is a mark being moved or an edge being dragged; a mark still being
    /// drawn is not.
    pub fn export(&mut self) -> Vec<u8> {
        self.commit_text();
        self.gesture = None;
        let selected = self.selected.take();
        let also = std::mem::take(&mut self.also);
        let hover = self.hover_mark.take();
        self.fresh = false;
        let (w, h) = (self.width(), self.height());
        let (bytes, stride) = self.render();
        let mut out = tight(bytes, stride, w, h);
        self.selected = selected;
        self.also = also;
        self.hover_mark = hover;
        self.fresh = false;
        // Everything drawn sits on an opaque base, so the result is opaque;
        // make that exact for the clipboard.
        for px in out.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
        out
    }
}

/// `h` rows of `w` BGRA pixels from rows `stride` bytes apart.
pub(super) fn tight(pixels: &[u8], stride: usize, w: u32, h: u32) -> Vec<u8> {
    let row = w as usize * 4;
    let mut out = Vec::with_capacity(row * h as usize);
    for r in pixels.chunks(stride).take(h as usize) {
        out.extend_from_slice(&r[..row]);
    }
    out
}

/// A small dark pill with `text`, just right of `at`.
fn draw_badge(canvas: &Canvas, typeface: &Typeface, text: &str, at: Point, scale: f32) {
    let font = skia_safe::Font::from_typeface(typeface.clone(), 12.0 * scale);
    let (w, _) = font.measure_str(text, None);
    let r = Rect::from_xywh(
        at.x + 12.0 * scale,
        at.y - 10.0 * scale,
        w + 12.0 * scale,
        20.0 * scale,
    );
    let bg = fill(Color4f::new(0.07, 0.07, 0.08, 0.9));
    canvas.draw_rrect(
        skia_safe::RRect::new_rect_xy(r, 6.0 * scale, 6.0 * scale),
        &bg,
    );
    let fg = fill(Color4f::new(1.0, 1.0, 1.0, 1.0));
    canvas.draw_str(
        text,
        (r.left + 6.0 * scale, r.top + 14.0 * scale),
        &font,
        &fg,
    );
}

/// An outline around a mark: faint and dashed (`hover`) for one a click
/// would select, solid for one of several selected.
fn draw_outline(canvas: &Canvas, m: &Mark, typeface: Option<&Typeface>, scale: f32, hover: bool) {
    let mut p = Paint::new(
        theme::with_alpha(theme::SELECTION, if hover { 0.75 } else { 1.0 }),
        None,
    );
    p.set_anti_alias(true);
    p.set_style(PaintStyle::Stroke);
    p.set_stroke_width(scale.max(1.0));
    if hover {
        p.set_path_effect(skia_safe::PathEffect::dash(
            &[4.0 * scale, 3.0 * scale],
            0.0,
        ));
    }
    match m {
        // A line or arrow gets a soft band along its length.
        Mark::Arrow(Arrow {
            from,
            to,
            width,
            bend,
            ..
        })
        | Mark::Line(Line {
            from,
            to,
            width,
            bend,
            ..
        }) => {
            let mut band = Paint::new(
                theme::with_alpha(theme::SELECTION, if hover { 0.22 } else { 0.4 }),
                None,
            );
            band.set_anti_alias(true);
            band.set_style(PaintStyle::Stroke);
            band.set_stroke_cap(PaintCap::Round);
            band.set_stroke_width(width * 1.6 + 8.0 * scale);
            canvas.draw_path(&crate::curve_path(*from, *to, *bend), &band);
        }
        _ => {
            if let Some(r) = m.outline(typeface) {
                let pad = LABEL_PAD * scale;
                m.turned(canvas, typeface, || {
                    canvas.draw_rect(r.with_outset((pad, pad)), &p);
                });
            }
        }
    }
}

/// The selected mark's handles, and an outline for marks without corner
/// handles, drawn at a constant screen size. `hot` is the handle under the
/// pointer, filled in.
fn draw_selection(
    canvas: &Canvas,
    m: &Mark,
    typeface: Option<&Typeface>,
    scale: f32,
    hot: Option<Grip>,
) {
    let blue = theme::SELECTION;
    let line = stroke(blue, scale);
    if let Some(r) = m.outline(typeface) {
        let pad = if matches!(m, Mark::Label(_)) {
            LABEL_PAD * scale
        } else {
            0.0
        };
        m.turned(canvas, typeface, || {
            canvas.draw_rect(r.with_outset((pad, pad)), &line);
        });
    }
    let white = fill(Color4f::new(1.0, 1.0, 1.0, 1.0));
    let hot_fill = fill(blue);
    let half = HANDLE_HALF * scale;
    for (g, p) in grips_of(m, typeface, scale) {
        // The bend and rounding handles are round and blue, to read as
        // "shape", not "resize".
        if matches!(g, Grip::Bend | Grip::Round | Grip::Rotate) {
            if g == Grip::Rotate
                && let Some((foot, _)) = rotate_stick(m, typeface, scale)
            {
                let foot = match m.spin(typeface) {
                    Some((a, c)) => rotate_about(foot, a, c),
                    None => foot,
                };
                canvas.draw_line(foot, p, &line);
            }
            let r = half * if Some(g) == hot { 1.7 } else { 1.3 };
            canvas.draw_circle(p, r, &hot_fill);
            let mut ring = white.clone();
            ring.set_style(PaintStyle::Stroke);
            ring.set_stroke_width(1.5 * scale);
            canvas.draw_circle(p, r, &ring);
            continue;
        }
        let (r, f) = if Some(g) == hot {
            let h = half * 1.4;
            (
                Rect::from_ltrb(p.x - h, p.y - h, p.x + h, p.y + h),
                &hot_fill,
            )
        } else {
            (
                Rect::from_ltrb(p.x - half, p.y - half, p.x + half, p.y + half),
                &white,
            )
        };
        canvas.draw_rect(r, f);
        canvas.draw_rect(r, &line);
    }
}
