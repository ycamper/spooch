//! Drawing the chrome: the tool rail, the style row and its font list,
//! the move tab, tooltips and the loupe, each as a `Panel` the shells
//! blend over the screen.

use crate::document::{fill, stroke};
use skia_safe::{
    BlurStyle, Canvas, Color4f, Font, IRect, MaskFilter, Paint, PaintCap, PaintJoin, PaintStyle,
    Path, PathBuilder, Point, RRect, Rect, Typeface,
};

use super::*;
use crate::document::bar::BarLayout;
use crate::document::style::{PALETTES, SIZES};
use crate::document::theme::{
    ACTIVE, FIELD, HOVER, ICON, PANEL, PANEL_EDGE, TEXT, WHITE, centred_baseline, chevron,
    draw_list, fill_rrect, rounded_panel,
};
use crate::{Arrow, HighlightLook};

impl Document {
    /// The loupe on `at`, from the document's pixels (black off them), with
    /// the canvas's size under it.
    fn draw_loupe(&self, at: (i32, i32)) -> Option<Panel> {
        let area = crate::select::loupe_area(at);
        let side = area.width();
        let mut pixels = vec![0u8; (side * side * 4) as usize];
        let info = crate::bgra_info((side, side), skia_safe::AlphaType::Premul);
        // Skia trims a read that runs off the image, leaving the rest black.
        let from = (
            area.left - self.base.at.x as i32,
            area.top - self.base.at.y as i32,
        );
        self.base.image.read_pixels(
            &info,
            &mut pixels,
            side as usize * 4,
            from,
            skia_safe::image::CachingHint::Allow,
        );
        let c = self.canvas;
        let on_screen = self.to_screen(Point::new(at.0 as f32, at.1 as f32));
        let at_screen = if self.in_window() {
            (on_screen.x as i32, on_screen.y as i32)
        } else {
            at
        };
        crate::select::loupe(
            at_screen,
            crate::select::Readout::Size(c.width(), c.height()),
            &pixels,
            self.screen_at(at_screen),
            self.scale,
        )
    }

    /// The chrome's panels, to blend onto the overlay in order. Cheap when
    /// nothing changed since the last call.
    pub fn chrome(&mut self) -> &[Panel] {
        // Holding Alt: the result alone, in place of everything else.
        if self.preview {
            if self.chrome_key.is_some() || self.chrome_panels.len() != 1 {
                self.chrome_panels = vec![self.draw_preview()];
                self.chrome_key = None;
            }
            return &self.chrome_panels;
        }
        let rail = self.rail_layout();
        let row = self.row_layout();
        let bar = self.bar_layout();
        let picker = self.picker_layout();
        let key = ChromeKey {
            picker: picker.as_ref().map(|k| (k.pill, k.list)),
            stickers: self.styles.stickers,
            sticker_menu: self.sticker_menu,
            bar_menu: self.bar_menu,
            loupe: self.loupe_at,
            maximized: self.window_maximized,
            gallery: self.gallery_key(),
            view: self.view.map(|v| {
                (
                    v.size,
                    (v.zoom * 1000.0) as i32,
                    v.fit,
                    (v.center.x as i32, v.center.y as i32),
                )
            }),
            rail: rail.rect,
            row: row.as_ref().map(|r| r.rect),
            bar: bar.rect,
            bar_text: bar.items.iter().map(|(.., t)| t.clone()).collect(),
            format: self.format,
            tool: self.tool,
            style: self.shown_style(),
            hovered: self.hovered,
            sliding: matches!(self.gesture, Some(Gesture::Slide { .. })),
            canvas: self.canvas,
            moving: matches!(
                self.gesture,
                Some(Gesture::Canvas {
                    grip: crate::document::Grip::Whole,
                    ..
                })
            ),
        };
        if self.chrome_key.as_ref() != Some(&key) {
            let mut panels = Vec::new();
            if let Some(t) = self.top_layout() {
                let hovered = match self.hovered {
                    Some(Hit::Top(item)) => Some(item),
                    _ => None,
                };
                panels.push(self.draw_top(&t, hovered));
            }
            if let Some(g) = self.gallery_layout() {
                let hovered = match self.hovered {
                    Some(Hit::Gallery(h)) => Some(h),
                    _ => None,
                };
                panels.push(self.draw_gallery(&g, hovered));
            }
            panels.push(self.draw_rail(&rail));
            if let Some(row) = &row {
                panels.push(self.draw_row(row));
            }
            let bar_hover = match self.hovered {
                Some(Hit::Bar(item)) => Some(item),
                _ => None,
            };
            panels.push(self.draw_bar(&bar, bar_hover));
            if !self.in_window() {
                panels.push(self.draw_tab());
            }
            if let Some((menu, list, rows)) = self.bar_menu_layout(&bar) {
                let hovered = match self.hovered {
                    Some(Hit::MenuOption(i)) => Some(i),
                    _ => None,
                };
                panels.push(self.draw_bar_menu(menu, list, &rows, hovered));
            }
            if let Some(k) = &picker {
                panels.extend(self.draw_picker(k));
            }
            if let Some((grid, cells)) = self.sticker_grid() {
                panels.push(self.draw_sticker_grid(grid, &cells));
            }
            let sliding = match &self.gesture {
                Some(Gesture::Slide { slider, .. }) => Some(Hit::Slider(*slider)),
                _ => None,
            };
            if let Some(tip) = sliding
                .or(self.hovered)
                .and_then(|h| self.draw_tooltip(&rail, row.as_ref(), &bar, h))
            {
                panels.push(tip);
            }
            if let Some(l) = self.loupe_at.and_then(|at| self.draw_loupe(at)) {
                panels.push(l);
            }
            self.chrome_panels = panels;
            self.chrome_key = Some(key);
        }
        &self.chrome_panels
    }

    fn draw_rail(&self, layout: &RailLayout) -> Panel {
        let s = self.scale;
        let r = layout.rect;
        panel(r, |canvas| {
            let bounds = Rect::from_wh(r.width() as f32, r.height() as f32);
            if !self.in_window() {
                rounded_panel(canvas, bounds, 10.0 * s, s);
            }
            let origin = Point::new(r.left as f32, r.top as f32);
            for (tool, b) in &layout.buttons {
                let rect = Rect::from(*b).with_offset(-origin);
                let (bg, fg) = if *tool == self.tool {
                    (Some(ACTIVE), WHITE)
                } else if Some(Hit::Tool(*tool)) == self.hovered {
                    (Some(HOVER), ICON)
                } else {
                    (None, ICON)
                };
                if let Some(bg) = bg {
                    let p = fill(bg);
                    canvas.draw_rrect(RRect::new_rect_xy(rect, 6.0 * s, 6.0 * s), &p);
                }
                draw_icon(
                    canvas,
                    *tool,
                    rect.center(),
                    16.0 * s,
                    fg,
                    self.ui_typeface.as_ref(),
                );
                if GROUP_ENDS.contains(tool) {
                    let y = rect.bottom + (GAP + SEPARATOR) * s / 2.0;
                    let mut p = Paint::new(PANEL_EDGE, None);
                    p.set_stroke_width(s.max(1.0));
                    canvas.draw_line((rect.left + 3.0 * s, y), (rect.right - 3.0 * s, y), &p);
                }
            }
        })
    }

    pub(in crate::document) fn draw_row(&self, layout: &RowLayout) -> Panel {
        let s = self.scale;
        let r = layout.rect;
        let style = self.shown_style().map(|(st, _)| st);
        // The colour the opacity slider and edge button preview in.
        let colour = style.map_or(self.palette().colours[0], |st| self.colour_of(st));
        let sliding = match &self.gesture {
            Some(Gesture::Slide { slider, .. }) => Some(*slider),
            _ => None,
        };
        panel(r, |canvas| {
            let bounds = Rect::from_wh(r.width() as f32, r.height() as f32);
            if !self.in_window() {
                rounded_panel(canvas, bounds, 10.0 * s, s);
            }
            let origin = Point::new(r.left as f32, r.top as f32);
            let local = |b: &IRect| Rect::from(*b).with_offset(-origin);

            for (i, b) in layout.colours.iter().enumerate() {
                let rect = local(b);
                let c = rect.center();
                let radius = rect.width() / 2.0;
                let chosen = style.is_some_and(|st| usize::from(st.color) == i);
                if chosen || self.hovered == Some(Hit::Colour(i)) {
                    let ring = stroke(if chosen { WHITE } else { ICON }, 2.0 * s);
                    canvas.draw_circle(c, radius + 1.5 * s, &ring);
                }
                let dot = fill(self.palette().colours[i]);
                canvas.draw_circle(c, radius - 1.0 * s, &dot);
                // Black needs an edge to show on the dark panel.
                if i == 6 {
                    let edge = stroke(Color4f::new(0.4, 0.4, 0.42, 1.0), s);
                    canvas.draw_circle(c, radius - 1.0 * s, &edge);
                }
            }
            // The recent stickers, the one the tool stamps first; then the
            // button for all of them, a grid of dots.
            for (i, b) in layout.stickers.iter().enumerate() {
                let rect = local(b);
                if i == 0 || self.hovered == Some(Hit::Sticker(i)) {
                    let p = fill(if i == 0 { ACTIVE } else { HOVER });
                    canvas.draw_rrect(RRect::new_rect_xy(rect, 6.0 * s, 6.0 * s), &p);
                }
                let name = self.recent_stickers()[i];
                crate::sticker::draw_sticker_image(
                    canvas,
                    name,
                    rect.with_inset((2.0 * s, 2.0 * s)),
                    &Paint::default(),
                );
            }
            if let Some(b) = &layout.more {
                let rect = local(b);
                if self.sticker_menu || self.hovered == Some(Hit::StickerMore) {
                    let p = fill(HOVER);
                    canvas.draw_rrect(RRect::new_rect_xy(rect, 6.0 * s, 6.0 * s), &p);
                }
                let dot = fill(ICON);
                for k in 0..9 {
                    let (x, y) = ((k % 3) as f32 - 1.0, (k / 3) as f32 - 1.0);
                    canvas.draw_circle(
                        (rect.center_x() + x * 5.0 * s, rect.center_y() + y * 5.0 * s),
                        1.6 * s,
                        &dot,
                    );
                }
            }
            if let Some(last) = layout.stickers.last() {
                let x = local(last).right + SEPARATOR_GAP * s / 2.0;
                let mut sep = Paint::new(PANEL_EDGE, None);
                sep.set_stroke_width(s.max(1.0));
                canvas.draw_line((x, 7.0 * s), (x, r.height() as f32 - 7.0 * s), &sep);
            }

            // The preset button: three of its colours, round its middle.
            if let Some(b) = &layout.palette {
                let rect = local(b);
                let c = rect.center();
                if self.hovered == Some(Hit::Palette) {
                    let ring = stroke(ICON, 2.0 * s);
                    canvas.draw_circle(c, rect.width() / 2.0 + 1.5 * s, &ring);
                }
                let colours = &self.palette().colours;
                let r = rect.width() * 0.2;
                for (k, slot) in [0usize, 2, 4].into_iter().enumerate() {
                    let a = std::f32::consts::TAU * k as f32 / 3.0 - std::f32::consts::FRAC_PI_2;
                    let at = Point::new(c.x + a.cos() * r * 1.15, c.y + a.sin() * r * 1.15);
                    let dot = fill(colours[slot]);
                    canvas.draw_circle(at, r, &dot);
                }
            }
            // The highlight look or plate: a little picture of it.
            if let (Some(b), Some(st)) = (&layout.look, style) {
                let rect = local(b);
                if self.hovered == Some(Hit::Look) {
                    let p = fill(HOVER);
                    canvas.draw_rrect(RRect::new_rect_xy(rect, 6.0 * s, 6.0 * s), &p);
                }
                let icon = Rect::from_xywh(
                    rect.center_x() - 9.0 * s,
                    rect.center_y() - 6.0 * s,
                    18.0 * s,
                    12.0 * s,
                );
                if self.shows_highlight() {
                    highlight_icon(canvas, icon, HighlightLook::nth(st.variant), colour, s);
                } else {
                    plate_icon(canvas, icon, crate::Plate::nth(st.variant), colour, s);
                }
            }
            if let (Some(last), true) = (
                layout.palette.or(layout.colours.last().copied()),
                layout.size.is_some() || layout.look.is_some(),
            ) {
                let x = local(&last).right + SEPARATOR_GAP * s / 2.0;
                let mut sep = Paint::new(PANEL_EDGE, None);
                sep.set_stroke_width(s.max(1.0));
                canvas.draw_line((x, 7.0 * s), (x, r.height() as f32 - 7.0 * s), &sep);
            }

            let sliders = [
                (Slider::Size, layout.size),
                (Slider::Opacity, layout.opacity),
                (Slider::Outline, layout.outline),
            ];
            for (slider, track) in sliders {
                let Some(track) = track else { continue };
                let rect = local(&track);
                // With no edge, the outline slider has nothing to set.
                let dim = slider == Slider::Outline
                    && style.is_some_and(|st| self.look_of(st).edge.is_none());
                if dim {
                    canvas.save_layer_alpha_f(None, 0.35);
                }
                let active = sliding == Some(slider) || self.hovered == Some(Hit::Slider(slider));
                // The icon before the track.
                let icon = Point::new(
                    rect.left - (SLIDER_ICON_GAP + SLIDER_ICON / 2.0) * s,
                    rect.center_y(),
                );
                draw_slider_icon(canvas, slider, layout.strength, icon, s, colour);
                let knob = KNOB * s;
                let (x0, x1) = (rect.left + knob, rect.right - knob);
                let y = rect.center_y();
                let t = self.slider_value(slider);
                let x = x0 + t * (x1 - x0);
                let mut groove = fill(PANEL_EDGE);
                groove.set_stroke_width(4.0 * s);
                groove.set_stroke_cap(PaintCap::Round);
                canvas.draw_line((x0, y), (x1, y), &groove);
                let mut bar = groove.clone();
                bar.set_color4f(
                    if slider == Slider::Opacity {
                        colour
                    } else {
                        ICON
                    },
                    None,
                );
                canvas.draw_line((x0, y), (x, y), &bar);
                // Ticks for the size steps.
                if slider == Slider::Size {
                    let tick = fill(ICON);
                    for i in 0..SIZES {
                        let tx = x0 + i as f32 / (SIZES - 1) as f32 * (x1 - x0);
                        canvas.draw_circle((tx, y), 1.2 * s, &tick);
                    }
                }
                let k = fill(WHITE);
                canvas.draw_circle((x, y), knob, &k);
                if active {
                    let ring = stroke(Color4f::new(1.0, 1.0, 1.0, 0.35), 3.0 * s);
                    canvas.draw_circle((x, y), knob + 2.0 * s, &ring);
                }
                if dim {
                    canvas.restore();
                }
            }

            if let Some(b) = &layout.edge {
                let rect = local(b);
                if self.hovered == Some(Hit::Edge) {
                    let p = fill(HOVER);
                    canvas.draw_rrect(RRect::new_rect_xy(rect, 6.0 * s, 6.0 * s), &p);
                }
                // A dot in the mark's colour with its edge around it.
                let c = rect.center();
                let look = style.map(|st| self.look_of(st));
                let ring = look.and_then(|l| l.edge);
                if look.is_some_and(|l| l.auto_edge) {
                    // Auto: half white, half black.
                    canvas.draw_circle(c, 7.5 * s, &fill(Color4f::new(1.0, 1.0, 1.0, 1.0)));
                    let half = Rect::from_xywh(c.x - 7.5 * s, c.y - 7.5 * s, 15.0 * s, 15.0 * s);
                    canvas.draw_arc(
                        half,
                        -45.0,
                        180.0,
                        true,
                        &fill(Color4f::new(0.0, 0.0, 0.0, 1.0)),
                    );
                } else if let Some(ring) = ring {
                    let p = fill(ring);
                    canvas.draw_circle(c, 7.5 * s, &p);
                }
                let fill = fill(colour);
                canvas.draw_circle(c, 5.0 * s, &fill);
                if ring.is_none() {
                    // No edge: a faint dashed circle where it would be.
                    let mut p = stroke(ICON, s);
                    p.set_path_effect(skia_safe::PathEffect::dash(&[2.0 * s, 2.0 * s], 0.0));
                    canvas.draw_circle(c, 7.0 * s, &p);
                }
            }

            // The saved looks: each a dot in its colour and size, ringed
            // when it's the look in use; empty ones dashed.
            let saved = self.quick_styles().unwrap_or_default();
            for ((i, b), st) in layout.quick.iter().enumerate().zip(saved) {
                let rect = local(b);
                let rr = RRect::new_rect_xy(rect.with_inset((s, s)), 6.0 * s, 6.0 * s);
                if self.hovered == Some(Hit::Quick(i)) {
                    canvas.draw_rrect(rr, &fill(HOVER));
                }
                let Some(st) = st else {
                    let mut p = stroke(ICON, s);
                    p.set_path_effect(skia_safe::PathEffect::dash(&[2.0 * s, 2.0 * s], 0.0));
                    canvas.draw_rrect(rr, &p);
                    continue;
                };
                let ring = if style == Some(st) { WHITE } else { PANEL_EDGE };
                canvas.draw_rrect(rr, &stroke(ring, 1.5 * s));
                let dot = if layout.strength {
                    ICON
                } else {
                    self.colour_of(st)
                };
                let radius = (2.5 + f32::from(st.size) * 1.1) * s;
                canvas.draw_circle(rect.center(), radius, &fill(dot));
            }

            // "Aa" in the label font.
            if let (Some(b), Some(st)) = (&layout.font, style) {
                let rect = local(b);
                if self.hovered == Some(Hit::Font) {
                    let p = fill(HOVER);
                    canvas.draw_rrect(RRect::new_rect_xy(rect, 6.0 * s, 6.0 * s), &p);
                }
                if let Some(tf) = LabelFont::nth(st.font).typeface() {
                    let font = Font::from_typeface(tf, 15.0 * s);
                    let (w, bounds) = font.measure_str("Aa", None);
                    let p = fill(ICON);
                    canvas.draw_str(
                        "Aa",
                        (
                            rect.center_x() - w / 2.0,
                            crate::document::ink_baseline(&bounds, rect.center_y()),
                        ),
                        &font,
                        &p,
                    );
                }
            }
        })
    }

    /// The grid of every sticker; the one the tool stamps is marked.
    fn draw_sticker_grid(&self, grid: IRect, cells: &[IRect]) -> Panel {
        let s = self.scale;
        let current = self.sticker();
        let hovered = match self.hovered {
            Some(Hit::StickerOption(i)) => Some(i),
            _ => None,
        };
        panel(grid, |canvas| {
            let bounds = Rect::from_wh(grid.width() as f32, grid.height() as f32);
            rounded_panel(canvas, bounds, 10.0 * s, s);
            let origin = Point::new(grid.left as f32, grid.top as f32);
            for (i, cell) in cells.iter().enumerate() {
                let rect = Rect::from(*cell).with_offset(-origin);
                let (name, _) = crate::STICKERS[i];
                if name == current || hovered == Some(i) {
                    let p = fill(if name == current { ACTIVE } else { HOVER });
                    canvas.draw_rrect(RRect::new_rect_xy(rect, 7.0 * s, 7.0 * s), &p);
                }
                crate::sticker::draw_sticker_image(
                    canvas,
                    name,
                    rect.with_inset((4.0 * s, 4.0 * s)),
                    &Paint::default(),
                );
            }
        })
    }

    /// The font dropdown's pill and, while open, its list: each font's name
    /// written in that font.
    fn draw_picker(&self, layout: &PickerLayout) -> Vec<Panel> {
        let s = self.scale;
        let Some(current) = self.picker_label().map(|l| l.font) else {
            return Vec::new();
        };
        let name = |canvas: &Canvas, f: LabelFont, rect: Rect, colour: Color4f| {
            if let Some(tf) = f.typeface() {
                let font = Font::from_typeface(tf, PICKER_FONT * s);
                let (_, bounds) = font.measure_str(f.name(), None);
                let p = fill(colour);
                canvas.draw_str(
                    f.name(),
                    (
                        rect.left + PICKER_PAD * s,
                        crate::document::ink_baseline(&bounds, rect.center_y()),
                    ),
                    &font,
                    &p,
                );
            }
        };
        let r = layout.pill;
        let open = layout.list.is_some();
        let mut panels = vec![panel(r, |canvas| {
            let bounds = Rect::from_wh(r.width() as f32, r.height() as f32);
            rounded_panel(canvas, bounds, 6.0 * s, s);
            if open || self.hovered == Some(Hit::FontPicker) {
                let p = fill(HOVER);
                let inner = bounds.with_inset((s, s));
                canvas.draw_rrect(RRect::new_rect_xy(inner, 5.0 * s, 5.0 * s), &p);
            }
            name(canvas, current, bounds, WHITE);
            let at = Point::new(
                bounds.right - (PICKER_PAD + CHEVRON / 2.0) * s,
                bounds.center_y(),
            );
            chevron(canvas, at, s, ICON);
        })];
        if let Some(list) = layout.list {
            let hovered = match self.hovered {
                Some(Hit::FontOption(i)) => Some(i),
                _ => None,
            };
            let at = Some(usize::from(current.index()));
            panels.push(panel(list, |canvas| {
                draw_list(
                    canvas,
                    list,
                    &layout.options,
                    at,
                    hovered,
                    s,
                    |i, rect, on| {
                        name(
                            canvas,
                            LabelFont::ALL[i],
                            rect,
                            if on { WHITE } else { ICON },
                        );
                    },
                );
            }));
        }
        panels
    }

    fn draw_tab(&self) -> Panel {
        let s = self.scale;
        let r = self.tab_rect();
        let active = self.hovered == Some(Hit::MoveTab)
            || matches!(
                self.gesture,
                Some(Gesture::Canvas {
                    grip: crate::document::Grip::Whole,
                    ..
                })
            );
        panel(r, |canvas| {
            let bounds = Rect::from_wh(r.width() as f32, r.height() as f32);
            let bg = fill(if active { ACTIVE } else { PANEL });
            let radius = bounds.height() / 2.0;
            let inner = bounds.with_inset((0.5 * s, 0.5 * s));
            canvas.draw_rrect(RRect::new_rect_xy(inner, radius, radius), &bg);
            let edge = stroke(if active { ACTIVE } else { PANEL_EDGE }, s.max(1.0));
            canvas.draw_rrect(RRect::new_rect_xy(inner, radius, radius), &edge);
            // Grip dots: two rows of three.
            let dot = fill(if active { WHITE } else { ICON });
            let c = bounds.center();
            for row in [-1.0, 1.0] {
                for col in [-1.0, 0.0, 1.0] {
                    canvas.draw_circle((c.x + col * 5.0 * s, c.y + row * 2.0 * s), 1.1 * s, &dot);
                }
            }
        })
    }

    fn draw_tooltip(
        &self,
        rail: &RailLayout,
        row: Option<&RowLayout>,
        bar: &BarLayout,
        hit: Hit,
    ) -> Option<Panel> {
        let tf = self.ui_typeface.as_ref()?;
        let s = self.scale;
        let text = match hit {
            Hit::Tool(tool) => match tool.digit() {
                Some(key) => format!("{}   {key}", tool.name()),
                None => format!("{}   or just type", tool.name()),
            },
            Hit::Colour(i) => format!("Shift+{}", i + 1),
            Hit::Palette => {
                let next = &PALETTES[(PALETTES
                    .iter()
                    .position(|p| p.name == self.palette().name)
                    .unwrap_or(0)
                    + 1)
                    % PALETTES.len()];
                format!("Colours: {}   click for {}", self.palette().name, next.name)
            }
            Hit::Sticker(i) => crate::sticker::sticker_title(self.recent_stickers()[i]),
            Hit::StickerMore => "All stickers".to_string(),
            Hit::StickerOption(i) => crate::sticker::sticker_title(crate::STICKERS.get(i)?.0),
            Hit::Look => {
                let (style, _) = self.shown_style()?;
                let name = |v: u8| {
                    if self.shows_highlight() {
                        HighlightLook::nth(v).name()
                    } else {
                        crate::Plate::nth(v).name()
                    }
                };
                let v = style.variant;
                format!("{}   click for {}", name(v), name(v.wrapping_add(1)))
            }
            Hit::Slider(slider) => {
                let (style, _) = self.shown_style()?;
                match slider {
                    Slider::Size if row.is_some_and(|r| r.strength) => {
                        format!("Strength {} of {SIZES}   [ and ]", style.size + 1)
                    }
                    Slider::Size => format!("Size {} of {SIZES}   [ and ]", style.size + 1),
                    Slider::Opacity => format!("Opacity {}%", style.opacity),
                    Slider::Outline => format!(
                        "Outline {}x",
                        crate::document::style::EDGE_WIDTHS[usize::from(style.edge_width)
                            .min(crate::document::style::EDGE_WIDTHS.len() - 1)]
                    ),
                }
            }
            Hit::Edge => {
                let (style, _) = self.shown_style()?;
                let name =
                    ["White", "Black", "Darker", "No", "Auto"].get(usize::from(style.edge))?;
                format!("{name} edge   click to change")
            }
            Hit::Quick(i) => match self.quick_styles()?.get(i)? {
                Some(_) => "Use this look   Shift+click to save over it".to_string(),
                None => "Save the current look here".to_string(),
            },
            Hit::Font => {
                let (style, _) = self.shown_style()?;
                format!(
                    "Font: {}   click to change",
                    LabelFont::nth(style.font).name()
                )
            }
            Hit::MoveTab => "Drag to move the selection, Shift+drag to bring the marks".to_string(),
            // The dropdown names its fonts itself.
            Hit::FontPicker | Hit::FontOption(_) | Hit::MenuOption(_) => return None,
            Hit::Bar(BarItem::Save) if self.save_shows() => {
                "Show the saved picture in its folder".to_string()
            }
            Hit::Bar(item) => item.tip().to_string(),
            Hit::Top(item) => self.top_tip(item).to_string(),
            Hit::Gallery(g) => self.gallery_tip(g)?,
        };
        let font = Font::from_typeface(tf.clone(), TIP_FONT * s);
        let (advance, _) = font.measure_str(&text, None);
        let w = (advance + 16.0 * s).ceil() as i32;
        let h = (TIP_H * s).round() as i32;
        let rect = match hit {
            // Beside the rail, level with the button.
            Hit::Tool(tool) => {
                let (_, b) = rail.buttons.iter().find(|(t, _)| *t == tool)?;
                IRect::from_xywh(
                    rail.rect.right + (6.0 * s) as i32,
                    (b.top + b.bottom) / 2 - h / 2,
                    w,
                    h,
                )
            }
            Hit::FontPicker | Hit::FontOption(_) | Hit::MenuOption(_) => return None,
            // Under the top row, kept in the window.
            Hit::Top(item) => {
                let t = self.top_layout()?;
                let b = match item {
                    crate::document::view::TopItem::Gallery => t.gallery,
                    crate::document::view::TopItem::Zoom => t.zoom,
                    crate::document::view::TopItem::Copy => t.copy,
                    crate::document::view::TopItem::Window(w) => {
                        t.buttons.iter().find(|(b, _)| *b == w).map(|(_, r)| *r)?
                    }
                };
                let x = ((b.left + b.right) / 2 - w / 2).clamp(
                    t.rect.left + (6.0 * s) as i32,
                    t.rect.right - w - (6.0 * s) as i32,
                );
                IRect::from_xywh(x, t.rect.bottom + (4.0 * s) as i32, w, h)
            }
            // Right of the gallery, level with the button.
            Hit::Gallery(g) => {
                let b = self.gallery_place(g)?;
                let right = self.dock()?.gallery?.right;
                IRect::from_xywh(
                    right + (6.0 * s) as i32,
                    (b.top + b.bottom) / 2 - h / 2,
                    w,
                    h,
                )
            }
            // Beside the tab, so it doesn't cover the bar.
            Hit::MoveTab => {
                let t = self.tab_rect();
                IRect::from_xywh(
                    t.right + (6.0 * s) as i32,
                    (t.top + t.bottom) / 2 - h / 2,
                    w,
                    h,
                )
            }
            // Above the bar, over the button.
            Hit::Bar(item) => {
                let (_, b, _) = bar.items.iter().find(|(i, ..)| *i == item)?;
                IRect::from_xywh(
                    (b.left + b.right) / 2 - w / 2,
                    bar.rect.top - h - (4.0 * s) as i32,
                    w,
                    h,
                )
            }
            // Above the row, over the button.
            // Over the grid, above the sticker.
            Hit::StickerOption(i) => {
                let (_, cells) = self.sticker_grid()?;
                let b = cells.get(i)?;
                IRect::from_xywh(
                    (b.left + b.right) / 2 - w / 2,
                    b.top - h - (2.0 * s) as i32,
                    w,
                    h,
                )
            }
            Hit::Colour(_)
            | Hit::Sticker(_)
            | Hit::StickerMore
            | Hit::Palette
            | Hit::Look
            | Hit::Slider(_)
            | Hit::Edge
            | Hit::Font
            | Hit::Quick(_) => {
                let row = row?;
                let b = row.place_of(hit)?;
                // Above the row, or under it in the window.
                let y = if self.in_window() {
                    row.rect.bottom + (4.0 * s) as i32
                } else {
                    row.rect.top - h - (4.0 * s) as i32
                };
                IRect::from_xywh((b.left + b.right) / 2 - w / 2, y, w, h)
            }
        };
        Some(panel(rect, |canvas| {
            let bounds = Rect::from_wh(w as f32, h as f32);
            fill_rrect(canvas, bounds, 6.0 * s, FIELD);
            let fg = crate::document::theme::fill(TEXT);
            let baseline = centred_baseline(&font, h as f32 / 2.0);
            canvas.draw_str(&text, (8.0 * s, baseline), &font, &fg);
        }))
    }
}

/// The look button's picture of highlight look `look` in `colour`.
fn highlight_icon(canvas: &Canvas, icon: Rect, look: HighlightLook, colour: Color4f, s: f32) {
    let spotlight = look == HighlightLook::Spotlight;
    if spotlight {
        canvas.draw_rrect(
            RRect::new_rect_xy(icon.with_outset((2.0 * s, 2.0 * s)), 3.0 * s, 3.0 * s),
            &fill(Color4f::new(0.0, 0.0, 0.0, 0.55)),
        );
        canvas.draw_rrect(
            RRect::new_rect_xy(icon.with_inset((3.0 * s, 2.5 * s)), 2.0 * s, 2.0 * s),
            &fill(WHITE),
        );
    }
    crate::Highlight {
        rect: if spotlight {
            icon.with_inset((3.0 * s, 2.5 * s))
        } else {
            icon
        },
        color: colour,
        look,
    }
    .draw(canvas);
    if !spotlight {
        // A line of "text" under it.
        let line = Rect::from_xywh(icon.left + 3.0 * s, icon.center_y() - s, 12.0 * s, 2.0 * s);
        canvas.draw_rect(line, &fill(ICON));
    }
}

/// The look button's picture of `plate` for labels in `colour`: a line of
/// "text" on it.
fn plate_icon(canvas: &Canvas, icon: Rect, plate: crate::Plate, colour: Color4f, s: f32) {
    let line = Rect::from_xywh(
        icon.left + 4.0 * s,
        icon.center_y() - 1.5 * s,
        10.0 * s,
        3.0 * s,
    );
    match plate.colours(colour) {
        Some((behind, on)) => {
            canvas.draw_rrect(RRect::new_rect_xy(icon, 3.0 * s, 3.0 * s), &fill(behind));
            canvas.draw_rect(line, &fill(on));
        }
        None => {
            canvas.draw_rect(line.with_outset((s, s)), &fill(WHITE));
            canvas.draw_rect(line, &fill(colour));
        }
    }
}

/// The small icon before a slider: two dots for size, a soft dot for
/// strength, a half-filled circle for opacity.
fn draw_slider_icon(
    canvas: &Canvas,
    slider: Slider,
    strength: bool,
    c: Point,
    s: f32,
    colour: Color4f,
) {
    let mut p = fill(ICON);
    match (slider, strength) {
        (Slider::Size, false) => {
            canvas.draw_circle((c.x - 4.0 * s, c.y), 1.8 * s, &p);
            canvas.draw_circle((c.x + 2.5 * s, c.y), 4.0 * s, &p);
        }
        (Slider::Size, true) => {
            p.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 1.8 * s, false));
            canvas.draw_circle(c, 4.5 * s, &p);
        }
        (Slider::Opacity, _) => {
            let r = 5.5 * s;
            let oval = Rect::from_xywh(c.x - r, c.y - r, 2.0 * r, 2.0 * r);
            let half = fill(colour);
            canvas.draw_arc(oval, 90.0, 180.0, false, &half);
            p.set_style(PaintStyle::Stroke);
            p.set_stroke_width(1.2 * s);
            canvas.draw_circle(c, r, &p);
        }
        // Outline: a thin ring and a thick one.
        (Slider::Outline, _) => {
            p.set_style(PaintStyle::Stroke);
            p.set_stroke_width(1.0 * s);
            canvas.draw_circle((c.x - 3.5 * s, c.y), 2.5 * s, &p);
            p.set_stroke_width(2.4 * s);
            canvas.draw_circle((c.x + 3.0 * s, c.y), 3.5 * s, &p);
        }
    }
}

/// A tool's icon, `size` pixels square, centred on `c`.
fn draw_icon(
    canvas: &Canvas,
    tool: Tool,
    c: Point,
    size: f32,
    color: Color4f,
    typeface: Option<&Typeface>,
) {
    // Icons are designed on a 16-unit grid.
    let u = size / 16.0;
    let at = |x: f32, y: f32| Point::new(c.x + (x - 8.0) * u, c.y + (y - 8.0) * u);
    let r = |l: f32, t: f32, rt: f32, b: f32| {
        Rect::from_ltrb(at(l, t).x, at(l, t).y, at(rt, b).x, at(rt, b).y)
    };
    let solid = fill(color);
    let mut line = stroke(color, 1.6 * u);
    line.set_stroke_cap(PaintCap::Round);
    line.set_stroke_join(PaintJoin::Round);
    match tool {
        Tool::Arrow => {
            let a = Arrow {
                from: at(2.5, 13.5),
                to: at(13.5, 2.5),
                color,
                width: 2.6 * u,
                look: crate::Look::default(),
                bend: 0.0,
            };
            canvas.draw_path(&a.path(), &solid);
        }
        Tool::Text => {
            // A serif-less T: crossbar and stem.
            canvas.draw_rect(r(2.5, 2.5, 13.5, 4.8), &solid);
            canvas.draw_rect(r(6.85, 2.5, 9.15, 13.5), &solid);
        }
        Tool::Box => {
            canvas.draw_rrect(
                RRect::new_rect_xy(r(2.5, 3.5, 13.5, 12.5), 1.5 * u, 1.5 * u),
                &line,
            );
        }
        Tool::Highlight => {
            let wash = fill(Color4f::new(color.r, color.g, color.b, 0.45));
            canvas.draw_rect(r(1.5, 5.5, 14.5, 10.5), &wash);
            let mut text = line.clone();
            text.set_stroke_width(1.2 * u);
            canvas.draw_line(at(3.5, 8.0), at(12.5, 8.0), &text);
        }
        Tool::Blur => {
            let mut soft = solid.clone();
            soft.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 1.6 * u, false));
            canvas.draw_circle(c, 4.5 * u, &soft);
        }
        Tool::Pixelate => {
            for row in 0..3 {
                for col in 0..3 {
                    let alpha = if (row + col) % 2 == 0 { 1.0 } else { 0.45 };
                    let p = fill(Color4f::new(color.r, color.g, color.b, alpha));
                    let (x, y) = (2.5 + col as f32 * 3.8, 2.5 + row as f32 * 3.8);
                    canvas.draw_rect(r(x, y, x + 3.2, y + 3.2), &p);
                }
            }
        }
        Tool::Line => {
            canvas.draw_line(at(3.0, 13.0), at(13.0, 3.0), &line);
        }
        Tool::Ellipse => {
            canvas.draw_oval(r(1.5, 4.0, 14.5, 12.0), &line);
        }
        Tool::Pen => {
            let mut pb = PathBuilder::new();
            pb.move_to(at(2.0, 11.0))
                .cubic_to(at(5.0, 2.0), at(8.0, 15.0), at(14.0, 4.5));
            canvas.draw_path(&pb.detach(), &line);
        }
        Tool::Steps => {
            canvas.draw_circle(c, 6.5 * u, &line);
            if let Some(tf) = typeface {
                let font = Font::from_typeface(tf.clone(), 9.5 * u);
                let (adv, _) = font.measure_str("1", None);
                let path = Path::from_str("1", Point::new(c.x - adv / 2.0, c.y + 3.4 * u), &font);
                canvas.draw_path(&path, &solid);
            }
        }
        Tool::Sticker => {
            // A smiley.
            canvas.draw_circle(c, 6.5 * u, &line);
            canvas.draw_circle(at(5.8, 6.5), 1.1 * u, &solid);
            canvas.draw_circle(at(10.2, 6.5), 1.1 * u, &solid);
            let mut pb = PathBuilder::new();
            pb.move_to(at(4.8, 9.6))
                .quad_to(at(8.0, 13.2), at(11.2, 9.6));
            canvas.draw_path(&pb.detach(), &line);
        }
        Tool::Redact => {
            canvas.draw_rect(r(2.0, 5.0, 14.0, 11.0), &solid);
            let mut gap = line.clone();
            gap.set_stroke_width(1.0 * u);
            canvas.draw_rect(r(1.0, 4.0, 15.0, 12.0), &gap);
        }
    }
}
