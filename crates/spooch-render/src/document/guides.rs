//! Snapping while moving or resizing marks, like Google Slides: edges,
//! centres and equal gaps line up with other marks and the canvas, with
//! guides showing. Alt places freely; arrow and line ends never snap.

use crate::Label;
use crate::document::fill;
use skia_safe::{Canvas, Color4f, Point, Rect, Typeface};

use super::{Document, Gesture, Grip, Mark, reshape};

/// How close counts as lined up, in screen pixels.
const SNAP: f32 = 6.0;
/// Guides are a bright magenta, which reads on almost any screenshot.
const GUIDE: Color4f = Color4f::new(0.94, 0.22, 0.60, 1.0);

/// A line to draw while dragging, in document pixels: an alignment, or an
/// equal gap (drawn with end ticks).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Guide {
    pub from: Point,
    pub to: Point,
    pub gap: bool,
    /// More than zero for a ring round `from` this big: text wrapped round
    /// a circle.
    pub radius: f32,
}

/// The three lines of a rectangle along one axis: start, middle and end.
fn lines(r: &Rect, horizontal: bool) -> [f32; 3] {
    if horizontal {
        [r.left, r.center_x(), r.right]
    } else {
        [r.top, r.center_y(), r.bottom]
    }
}

impl Document {
    /// What a moved or resized mark lines up with: every other mark's
    /// bounds, and the canvas.
    fn snap_targets(&self, skip: &[usize]) -> Vec<Rect> {
        let tf = self.typeface.as_ref();
        let mut out: Vec<Rect> = self
            .marks
            .iter()
            .enumerate()
            .filter(|(i, _)| !skip.contains(i))
            .filter_map(|(_, m)| m.bounds(tf))
            .collect();
        out.push(Rect::from(self.canvas));
        out
    }

    /// Move a mark by `d` (the pointer's travel since the press) and drag
    /// the selected `others` along. With snapping on, `d` is adjusted so
    /// the moved bounds line up; `self.guides` shows what does.
    pub(super) fn drag_edit(&mut self, p: Point) -> bool {
        let Some(Gesture::Edit {
            index,
            grip,
            from,
            original,
            others,
        }) = &self.gesture
        else {
            return false;
        };
        let (index, grip, from) = (*index, *grip, *from);
        let original = original.clone();
        let others: Vec<(usize, Mark)> = others.clone();
        let tf = self.typeface.as_ref();
        let slop = SNAP * self.ui_px();
        self.guides.clear();

        let m = match grip {
            // A label's bend wraps it round a circle it comes close to.
            Some(Grip::Bend) if matches!(original, Mark::Label(_)) => {
                let bent = reshape(&original, Grip::Bend, p, tf, self.scale, self.shift);
                match (&bent, tf) {
                    (Mark::Label(l), Some(tf)) if !self.alt && l.angle == 0.0 && l.bend != 0.0 => {
                        match self.wrap_round_circle(l, tf, index) {
                            Some((wrapped, centre, radius)) => {
                                self.guides.push(Guide {
                                    from: centre,
                                    to: centre,
                                    gap: false,
                                    radius,
                                });
                                Mark::Label(wrapped)
                            }
                            None => bent,
                        }
                    }
                    _ => bent,
                }
            }
            // A box-like mark's edge or corner snaps to other marks' lines.
            Some(Grip::Edge { x, y }) if snaps_edges(&original) => {
                let mut p = p;
                if !self.alt {
                    let skip: Vec<usize> = std::iter::once(index).collect();
                    let targets = self.snap_targets(&skip);
                    if x != 1 {
                        p.x = self.snap_line(p.x, &targets, true, slop);
                    }
                    if y != 1 {
                        p.y = self.snap_line(p.y, &targets, false, slop);
                    }
                }
                reshape(
                    &original,
                    Grip::Edge { x, y },
                    p,
                    tf,
                    self.scale,
                    self.shift,
                )
            }
            Some(g) => reshape(&original, g, p, tf, self.scale, self.shift),
            None => {
                let mut d = p - from;
                // Shift keeps the move straight.
                if self.shift {
                    if d.x.abs() > d.y.abs() {
                        d.y = 0.0;
                    } else {
                        d.x = 0.0;
                    }
                }
                let moving = std::iter::once(&original)
                    .chain(others.iter().map(|(_, o)| o))
                    .filter_map(|m| m.bounds(tf))
                    .reduce(Rect::join2);
                if let (Some(b), false) = (moving, self.alt) {
                    let skip: Vec<usize> = std::iter::once(index)
                        .chain(others.iter().map(|(j, _)| *j))
                        .collect();
                    let targets = self.snap_targets(&skip);
                    let shift_ok = |horizontal: bool| {
                        !self.shift || {
                            // Straight moves only snap along their own axis.
                            let along_x = d.y == 0.0;
                            horizontal == along_x
                        }
                    };
                    if shift_ok(true) {
                        d.x += self.snap_axis(b.with_offset(d), &targets, true, slop);
                    }
                    if shift_ok(false) {
                        d.y += self.snap_axis(b.with_offset(d), &targets, false, slop);
                    }
                    self.guides = guides_for(b.with_offset(d), &targets);
                }
                let mut m = original.clone();
                m.offset(d);
                for (j, o) in &others {
                    let mut moved = o.clone();
                    moved.offset(d);
                    self.marks[*j] = moved;
                }
                m
            }
        };
        if self.marks[index] == m {
            return false;
        }
        // A resized edge's guides, now its place is known.
        let bending_text = grip == Some(Grip::Bend) && matches!(original, Mark::Label(_));
        if grip.is_some() && !self.alt && !bending_text {
            let skip = [index];
            let targets = self.snap_targets(&skip);
            if let Some(b) = m.bounds(tf) {
                self.guides = guides_for(b, &targets)
                    .into_iter()
                    .filter(|g| !g.gap)
                    .collect();
            }
        }
        self.marks[index] = m;
        self.fresh = false;
        true
    }

    /// `l`, bent, wrapped snugly round a circle (an ellipse that's nearly
    /// round) when it's close to doing so already: its middle near the
    /// top of the circle as a rainbow, letters standing on it, or near the
    /// bottom as a smile, letters hanging under it. Returns the wrapped
    /// label with the circle its baseline follows.
    fn wrap_round_circle(
        &self,
        l: &Label,
        tf: &Typeface,
        index: usize,
    ) -> Option<(Label, Point, f32)> {
        use crate::curve::sagitta;
        let w = l.bend_width(tf);
        let middle = l.middle(tf);
        let slop = (l.size * 0.8).max(SNAP * 4.0 * self.ui_px());
        let gap = l.size * 0.2;
        self.marks.iter().enumerate().find_map(|(j, m)| {
            let Mark::Oval(o) = m else { return None };
            let (rw, rh) = (o.rect.width().abs(), o.rect.height().abs());
            if j == index || (rw - rh).abs() > 0.12 * rw.max(rh) {
                return None;
            }
            let centre = o.rect.center();
            let outside =
                (rw + rh) / 4.0 + crate::stroke_of(o.width) / 2.0 + crate::edge_of(o.width);
            let above = l.bend < 0.0;
            // The baseline: just off the circle, or a letter's height off
            // it when the letters hang underneath.
            let radius = outside + gap + if above { 0.0 } else { l.size * 0.75 };
            if w > std::f32::consts::TAU * radius * 0.95 {
                return None;
            }
            let bend = if above { -1.0 / radius } else { 1.0 / radius };
            let target = centre + Point::new(0.0, if above { -radius } else { radius });
            let (now, then) = (sagitta(l.bend, w), sagitta(bend, w));
            // Bending moves the middle away as the ends stay put, so a
            // label counts as near from about a line's height plus the
            // bend away.
            let reach = (l.size * 2.5).max(then.abs() + l.size).max(slop);
            let near =
                (middle.x - target.x).abs() < outside * 0.75 && (middle.y - target.y).abs() < reach;
            let alike = (now - then).abs() < (then.abs() * 0.35).max(slop * 0.5);
            (near && alike).then(|| {
                let wrapped = Label {
                    origin: Point::new(centre.x - w / 2.0, target.y),
                    bend,
                    ..l.clone()
                };
                (wrapped, centre, radius)
            })
        })
    }

    /// How far to move `b` along one axis to line it up: with another
    /// rectangle's start, middle or end, or into an equal gap between two
    /// neighbours. Zero when nothing is within `slop`.
    fn snap_axis(&self, b: Rect, targets: &[Rect], horizontal: bool, slop: f32) -> f32 {
        let mine = lines(&b, horizontal);
        let mut best: Option<f32> = None;
        let mut consider = |shift: f32| {
            if shift.abs() <= slop && best.is_none_or(|s| shift.abs() < s.abs()) {
                best = Some(shift);
            }
        };
        for t in targets {
            for theirs in lines(t, horizontal) {
                for at in mine {
                    consider(theirs - at);
                }
            }
        }
        let (before, after) = neighbours(&b, targets, horizontal);
        let (start, end) = span(&b, horizontal);
        // Equal gaps either side.
        if let (Some(p), Some(q)) = (before, after) {
            let (_, p_end) = span(&p, horizontal);
            let (q_start, _) = span(&q, horizontal);
            consider((p_end + q_start - (end - start)) / 2.0 - start);
        }
        // The same gap as two other marks side by side.
        for (_, _, gap) in spacings(targets, horizontal) {
            if let Some(p) = before {
                consider(span(&p, horizontal).1 + gap - start);
            }
            if let Some(q) = after {
                consider(span(&q, horizontal).0 - gap - end);
            }
        }
        best.unwrap_or(0.0)
    }

    /// `at` snapped to the nearest line of `targets` along one axis, if one
    /// is within `slop`.
    fn snap_line(&self, at: f32, targets: &[Rect], horizontal: bool, slop: f32) -> f32 {
        targets
            .iter()
            .flat_map(|t| lines(t, horizontal))
            .filter(|t| (t - at).abs() <= slop)
            .min_by(|a, b| (a - at).abs().total_cmp(&(b - at).abs()))
            .unwrap_or(at)
    }
}

/// Draw `guides`; `px` is image pixels per screen pixel. Alignments are
/// thin lines; equal spacing is marked like Google's `|—|`: a heavier
/// line with tall end caps, nudged off any alignment line it would sit on
/// so the two never merge.
pub(super) fn draw_guides(canvas: &Canvas, guides: &[Guide], px: f32) {
    let mut line = fill(GUIDE);
    line.set_stroke_width(px);
    let mut marker = line.clone();
    marker.set_stroke_width(1.5 * px);
    for g in guides.iter().filter(|g| !g.gap) {
        if g.radius > 0.0 {
            let mut ring = line.clone();
            ring.set_style(skia_safe::PaintStyle::Stroke);
            canvas.draw_circle(g.from, g.radius, &ring);
        } else {
            canvas.draw_line(g.from, g.to, &line);
        }
    }
    for g in guides.iter().filter(|g| g.gap) {
        let along = g.to - g.from;
        let len = along.length().max(0.001);
        let dir = Point::new(along.x / len, along.y / len);
        let across = Point::new(-dir.y, dir.x);
        // Off an alignment line running along the same path.
        let on_a_line = guides.iter().any(|o| {
            !o.gap && {
                let (oa, ob) = (o.from, o.to);
                let parallel = (ob - oa).cross(dir).abs() < 0.001 * (ob - oa).length().max(1.0);
                parallel && (g.from - oa).cross(dir).abs() < 0.5
            }
        });
        let shift = if on_a_line {
            across * (8.0 * px)
        } else {
            Point::new(0.0, 0.0)
        };
        // Inset a pixel from the marks' edges, so the caps stand clear.
        let (a, b) = (g.from + dir * px + shift, g.to - dir * px + shift);
        canvas.draw_line(a, b, &marker);
        let cap = across * (6.0 * px);
        for end in [a, b] {
            canvas.draw_line(end - cap, end + cap, &marker);
        }
    }
}

/// Whether a mark's edges snap when it's resized: boxes, ovals,
/// highlights and redactions, unturned.
fn snaps_edges(m: &Mark) -> bool {
    matches!(
        m,
        Mark::Box(_) | Mark::Oval(_) | Mark::Highlight(_) | Mark::Redact(..)
    ) && m.angle().is_none_or(|a| a == 0.0)
}

/// A rectangle's start and end along an axis.
fn span(r: &Rect, horizontal: bool) -> (f32, f32) {
    if horizontal {
        (r.left, r.right)
    } else {
        (r.top, r.bottom)
    }
}

/// Whether `t` is beside `a` along an axis: overlapping it across the axis.
fn beside(a: &Rect, t: &Rect, horizontal: bool) -> bool {
    if horizontal {
        t.top < a.bottom && t.bottom > a.top
    } else {
        t.left < a.right && t.right > a.left
    }
}

/// The nearest rectangles before and after `b` along an axis, among those
/// beside it.
fn neighbours(b: &Rect, targets: &[Rect], horizontal: bool) -> (Option<Rect>, Option<Rect>) {
    let (start, end) = span(b, horizontal);
    let before = targets
        .iter()
        .filter(|t| beside(b, t, horizontal) && span(t, horizontal).1 <= start + 0.5)
        .max_by(|x, y| span(x, horizontal).1.total_cmp(&span(y, horizontal).1));
    let after = targets
        .iter()
        .filter(|t| beside(b, t, horizontal) && span(t, horizontal).0 >= end - 0.5)
        .min_by(|x, y| span(x, horizontal).0.total_cmp(&span(y, horizontal).0));
    (before.copied(), after.copied())
}

/// The gaps between rectangles side by side along an axis: each one and
/// the nearest beside it after it.
fn spacings(targets: &[Rect], horizontal: bool) -> Vec<(Rect, Rect, f32)> {
    targets
        .iter()
        .filter_map(|a| {
            let (_, next) = neighbours(a, targets, horizontal);
            next.map(|n| (*a, n, span(&n, horizontal).0 - span(a, horizontal).1))
        })
        .filter(|(.., gap)| *gap > 0.5)
        .collect()
}

/// A gap marker from `a`'s end to `c`'s start, across the middle of where
/// they overlap.
fn gap_marker(a: &Rect, c: &Rect, horizontal: bool) -> Guide {
    let (from, to) = if horizontal {
        let y = (a.top.max(c.top) + a.bottom.min(c.bottom)) / 2.0;
        (Point::new(a.right, y), Point::new(c.left, y))
    } else {
        let x = (a.left.max(c.left) + a.right.min(c.right)) / 2.0;
        (Point::new(x, a.bottom), Point::new(x, c.top))
    };
    Guide {
        from,
        to,
        gap: true,
        radius: 0.0,
    }
}

/// The guides for `b` where it is now: a line through each start, middle
/// or end it shares with a target, and gap markers when it sits an equal
/// gap between two neighbours.
fn guides_for(b: Rect, targets: &[Rect]) -> Vec<Guide> {
    let mut out = Vec::new();
    for horizontal in [true, false] {
        let mine = lines(&b, horizontal);
        for t in targets {
            for theirs in lines(t, horizontal) {
                if mine.iter().any(|m| (m - theirs).abs() < 0.5) {
                    let (from, to) = if horizontal {
                        (
                            Point::new(theirs, b.top.min(t.top)),
                            Point::new(theirs, b.bottom.max(t.bottom)),
                        )
                    } else {
                        (
                            Point::new(b.left.min(t.left), theirs),
                            Point::new(b.right.max(t.right), theirs),
                        )
                    };
                    out.push(Guide {
                        from,
                        to,
                        gap: false,
                        radius: 0.0,
                    });
                }
            }
        }
        let (before, after) = neighbours(&b, targets, horizontal);
        let (start, end) = span(&b, horizontal);
        let gap_before = before.map(|p| start - span(&p, horizontal).1);
        let gap_after = after.map(|q| span(&q, horizontal).0 - end);
        // Equal gaps either side.
        if let (Some(p), Some(q), Some(g1), Some(g2)) = (before, after, gap_before, gap_after)
            && g1 > 0.5
            && (g1 - g2).abs() < 0.5
        {
            out.push(gap_marker(&p, &b, horizontal));
            out.push(gap_marker(&b, &q, horizontal));
        }
        // The same gap as two other marks side by side: mark both.
        for (a, c, gap) in spacings(targets, horizontal) {
            if let (Some(p), Some(g)) = (before, gap_before)
                && g > 0.5
                && (g - gap).abs() < 0.5
            {
                out.push(gap_marker(&p, &b, horizontal));
                out.push(gap_marker(&a, &c, horizontal));
            }
            if let (Some(q), Some(g)) = (after, gap_after)
                && g > 0.5
                && (g - gap).abs() < 0.5
            {
                out.push(gap_marker(&b, &q, horizontal));
                out.push(gap_marker(&a, &c, horizontal));
            }
        }
    }
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::super::tests::{click, drag, placed};
    use super::*;
    use crate::Tool;

    /// Highlights (filled, so a click anywhere grabs them): one at
    /// 120..160 across, 120..140 down, one at 200..260, 150..190. Snapping
    /// on.
    fn two_marks() -> Document {
        let (mut d, _) = placed();
        d.set_tool(Tool::Highlight);
        assert!(drag(&mut d, (120.0, 120.0), (160.0, 140.0)));
        assert!(drag(&mut d, (200.0, 150.0), (260.0, 190.0)));
        d.set_modifiers(false, false);
        d
    }

    fn rect(d: &Document, i: usize) -> Rect {
        d.marks[i].rect().unwrap()
    }

    #[test]
    fn a_move_snaps_left_edges_and_shows_a_guide() {
        let mut d = two_marks();
        click(&mut d, 230.0, 170.0);
        d.begin_drag(Point::new(230.0, 170.0));
        // Its left lands 3 px off the first's.
        d.drag_to(Point::new(153.0, 170.0));
        assert_eq!(rect(&d, 1).left, 120.0, "snapped");
        assert!(d.guides.iter().any(|g| !g.gap && g.from.x == 120.0));
        d.end_drag();
        assert!(d.guides.is_empty(), "guides go when the drag ends");
    }

    #[test]
    fn alt_places_freely_and_shift_keeps_the_move_straight() {
        let mut d = two_marks();
        click(&mut d, 230.0, 170.0);
        d.begin_drag(Point::new(230.0, 170.0));
        d.set_modifiers(false, true);
        d.drag_to(Point::new(153.0, 170.0));
        assert_eq!(rect(&d, 1).left, 123.0, "no snap with Alt");
        assert!(d.guides.is_empty());
        d.set_modifiers(true, false);
        d.drag_to(Point::new(290.0, 175.0));
        assert_eq!(rect(&d, 1).top, 150.0, "straight across with Shift");
        d.end_drag();
    }

    #[test]
    fn an_equal_gap_between_two_neighbours_snaps() {
        let (mut d, _) = placed();
        d.set_tool(Tool::Highlight);
        assert!(drag(&mut d, (110.0, 150.0), (130.0, 170.0)));
        assert!(drag(&mut d, (250.0, 150.0), (270.0, 170.0)));
        // A third, 40 wide, 2 px left of the middle of the gap.
        assert!(drag(&mut d, (168.0, 145.0), (208.0, 185.0)));
        d.set_modifiers(false, false);
        click(&mut d, 188.0, 165.0);
        d.begin_drag(Point::new(188.0, 165.0));
        d.drag_to(Point::new(188.5, 165.0));
        let r = rect(&d, 2);
        assert_eq!(r.left, 170.0, "{r:?}");
        assert!(d.guides.iter().any(|g| g.gap), "{:?}", d.guides);
        d.end_drag();
    }

    #[test]
    fn a_mark_snaps_to_the_spacing_two_others_already_have() {
        let (mut d, _) = placed();
        d.set_tool(Tool::Highlight);
        // Two marks 20 apart.
        assert!(drag(&mut d, (110.0, 150.0), (130.0, 170.0)));
        assert!(drag(&mut d, (150.0, 150.0), (170.0, 170.0)));
        // A bigger third after them, 2 px further than 20 from the second.
        assert!(drag(&mut d, (192.0, 140.0), (232.0, 180.0)));
        d.set_modifiers(false, false);
        click(&mut d, 212.0, 160.0);
        d.begin_drag(Point::new(212.0, 160.0));
        d.drag_to(Point::new(212.5, 160.0));
        assert_eq!(rect(&d, 2).left, 190.0, "the same 20 px gap");
        let gaps: Vec<&Guide> = d.guides.iter().filter(|g| g.gap).collect();
        assert!(
            gaps.iter().any(|g| g.from.x == 130.0 && g.to.x == 150.0),
            "the first pair's gap is marked: {gaps:?}"
        );
        assert!(gaps.iter().any(|g| g.from.x == 170.0 && g.to.x == 190.0));
        d.end_drag();
    }

    #[test]
    fn an_edge_snaps_but_arrow_ends_never_do() {
        let mut d = two_marks();
        click(&mut d, 230.0, 170.0);
        // The right edge, dragged near the canvas's right.
        d.begin_drag(Point::new(260.0, 170.0));
        d.drag_to(Point::new(296.0, 170.0));
        assert_eq!(rect(&d, 1).right, 300.0);
        d.end_drag();
        // An arrow whose end lands near a mark's edge stays where it's put.
        d.set_tool(Tool::Arrow);
        assert!(drag(&mut d, (130.0, 195.0), (161.0, 195.0)));
        let Some(Mark::Arrow(a)) = d.marks.last() else {
            panic!("not an arrow")
        };
        assert_eq!(a.to.x, 161.0);
    }
}
