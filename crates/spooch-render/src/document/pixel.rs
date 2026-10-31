//! Pixel mode: marks drawn as pixel art on a grid of cells over the
//! untouched capture. Each mark is drawn at one pixel per cell into a
//! layer just big enough for it, its cells cut to inked or clear and
//! outlined, then scaled up with nearest-neighbour sampling. The grid is
//! anchored to the capture, so cropping doesn't shift it.

use skia_safe::{
    BlendMode, Canvas, Color4f, ColorFilter, FilterMode, MipmapMode, Paint, Point, Rect,
    SamplingOptions, color_filters, surfaces,
};

crate::choices::choices! {
    /// How the marks are drawn. Each capture has its own; the geometry is
    /// the same in every mode, so switching is lossless.
    pub enum Mode {
        #[default]
        Clean => ("Clean", "clean"),
        Pixel => ("Pixel", "pixel"),
    }
}

/// Screen pixels per cell at 100% scale.
pub(super) const CELL: f32 = 3.0;

/// The grid of cells pixel art is drawn on.
#[derive(Clone, Copy)]
pub(super) struct Grid {
    /// A point the grid lines pass through: the capture's corner.
    origin: Point,
    /// A cell's side, in desktop pixels.
    cell: f32,
    /// Where drawing shows: the canvas.
    clip: Rect,
}

impl Grid {
    /// Run `draw` (in desktop pixels) at one pixel per cell over `area`,
    /// cut every cell to fully inked or clear, outline the ink one cell
    /// wide in `edge`, and put the cells onto `onto`, each a crisp square.
    pub(super) fn draw(
        &self,
        onto: &Canvas,
        area: Rect,
        edge: Option<Color4f>,
        draw: impl FnOnce(&Canvas),
    ) {
        const CLEAR: Color4f = Color4f::new(0.0, 0.0, 0.0, 0.0);
        let c = self.cell;
        // A cell spare on each side for the outline.
        let mut area = area.with_outset((c, c));
        if !area.intersect(self.clip) {
            return;
        }
        let snap = |v: f32, o: f32| o + ((v - o) / c).floor() * c;
        let corner = Point::new(
            snap(area.left, self.origin.x),
            snap(area.top, self.origin.y),
        );
        let cols = ((area.right - corner.x) / c).ceil() as i32;
        let rows = ((area.bottom - corner.y) / c).ceil() as i32;
        let Some(mut surface) = surfaces::raster_n32_premul((cols.max(1), rows.max(1))) else {
            return;
        };
        let s = surface.canvas();
        s.clear(CLEAR);
        s.save();
        s.scale((1.0 / c, 1.0 / c));
        s.translate(-corner);
        draw(s);
        s.restore();
        let soft = surface.image_snapshot();
        let s = surface.canvas();
        s.clear(CLEAR);
        let mut cut = Paint::default();
        cut.set_color_filter(alpha_cut());
        s.draw_image(&soft, (0, 0), Some(&cut));
        if let Some(e) = edge {
            // The ink in the edge colour, a cell every way round it.
            let hard = surface.image_snapshot();
            let s = surface.canvas();
            let mut ring = Paint::default();
            ring.set_color_filter(color_filters::blend(e.to_color(), BlendMode::SrcIn));
            ring.set_blend_mode(BlendMode::DstOver);
            for (dx, dy) in [
                (-1, -1),
                (0, -1),
                (1, -1),
                (-1, 0),
                (1, 0),
                (-1, 1),
                (0, 1),
                (1, 1),
            ] {
                s.draw_image(&hard, (dx, dy), Some(&ring));
            }
        }
        let cells = surface.image_snapshot();
        onto.draw_image_rect_with_sampling_options(
            &cells,
            None,
            Rect::from_point_and_size(corner, (cols as f32 * c, rows as f32 * c)),
            SamplingOptions::new(FilterMode::Nearest, MipmapMode::None),
            &Paint::default(),
        );
    }
}

/// A filter that makes every cell fully opaque or fully clear, so edges
/// are hard instead of soft.
fn alpha_cut() -> Option<ColorFilter> {
    let mut table = [0u8; 256];
    for (i, a) in table.iter_mut().enumerate() {
        *a = if i >= 128 { 255 } else { 0 };
    }
    color_filters::table_argb(&table, None, None, None)
}

impl super::Document {
    /// How the marks are drawn.
    pub fn mode(&self) -> Mode {
        self.styles.mode
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if self.styles.mode != mode {
            self.styles.mode = mode;
            self.fresh = false;
            self.chrome_key = None;
        }
    }

    /// The grid pixel art is drawn on, anchored at the capture's corner;
    /// `None` in clean mode.
    pub(super) fn grid(&self) -> Option<Grid> {
        (self.mode() == Mode::Pixel).then(|| Grid {
            origin: self.base.at,
            cell: CELL * self.scale,
            clip: Rect::from(self.canvas),
        })
    }
}
