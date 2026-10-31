//! Press, drag and release on the canvas: drawing new marks, moving and
//! reshaping selected ones, cropping and extending the canvas, and the
//! pointer shape for each.

use super::*;

/// A drag in progress with the tool it started with.
#[derive(Clone, Copy)]
pub(super) struct Drag {
    pub(super) tool: Tool,
    pub(super) from: Point,
    pub(super) to: Point,
}

/// What the pointer is over on the canvas, for hover feedback.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum HoverMark {
    /// A mark a click would select.
    Mark(usize),
    /// A handle of the selected mark a press would drag.
    Grip(Grip),
}

/// What the pointer is doing between press and release.
pub(super) enum Gesture {
    /// Drawing a new mark. `under` is the mark the press landed on, which
    /// is selected instead if the drag turns out to be a click. `path` is
    /// the pen's samples.
    Draw {
        drag: Drag,
        path: Vec<Point>,
        under: Option<usize>,
    },
    /// Moving the selected mark (`grip` is `None`) or one of its handles.
    /// Positions derive from `original` and the press point `from`, so
    /// nothing drifts over a long drag.
    /// `others` are the rest of a multiple selection, moving along.
    Edit {
        index: usize,
        grip: Option<Grip>,
        from: Point,
        original: Mark,
        others: Vec<(usize, Mark)>,
    },
    /// Dragging an edge or corner of the canvas.
    /// `carry` holds the marks moving with it (Shift on the move tab), as
    /// they were.
    Canvas {
        grip: Grip,
        from: Point,
        original: IRect,
        carry: Vec<(usize, Mark)>,
    },
    /// Placing a new label with the text tool: the caret follows the
    /// pointer until the button is let go.
    PlaceText,
    /// Dragging across the label being typed, selecting text.
    SelectText,
    /// Dragging a style slider. `before` is the state for undo, should the
    /// slide restyle the selected mark.
    Slide {
        slider: style::Slider,
        track: IRect,
        before: State,
    },
}

impl Document {
    /// The pointer shape for `p`: what a press there would do, or what the
    /// drag in progress is doing.
    pub fn cursor_at(&self, p: Point) -> Cursor {
        match &self.gesture {
            Some(Gesture::Draw { .. }) => return Cursor::Draw,
            Some(Gesture::Edit { grip: None, .. }) => return Cursor::Move,
            Some(Gesture::Edit { grip: Some(g), .. }) | Some(Gesture::Canvas { grip: g, .. }) => {
                return g.cursor();
            }
            Some(Gesture::Slide { .. }) => return Cursor::Hand,
            Some(Gesture::PlaceText | Gesture::SelectText) => return Cursor::Text,
            None => {}
        }
        if self.bar_hit(p).is_some() {
            return Cursor::Hand;
        }
        if chrome::contains(&self.tab_rect(), p) {
            return Cursor::Move;
        }
        if self.on_chrome(p) {
            return Cursor::Arrow;
        }
        let p = self.to_doc(p);
        if let Some((_, grip)) = self.grab_selected(p) {
            return grip.map_or(Cursor::Move, Grip::cursor);
        }
        if let Some(g) = self.canvas_grip(p) {
            return g.cursor();
        }
        if self.in_canvas(p) {
            if self.tool == Tool::Text {
                Cursor::Text
            } else {
                Cursor::Draw
            }
        } else {
            Cursor::Arrow
        }
    }

    /// Press at `p`. On the selected mark this starts moving it, or on one
    /// of its handles, reshaping it. On a canvas edge it starts cropping or
    /// extending. Anywhere else on the canvas it starts drawing with the
    /// current tool, so a drag always draws unless a mark was clicked
    /// first. Finishes any label being typed. Returns whether a drag
    /// started; presses off the canvas do nothing.
    pub fn begin_drag(&mut self, p: Point) -> bool {
        self.nudged = false;
        self.guides.clear();
        self.loupe_at = None;
        // The preview hides the editor; nothing under it is pressable.
        if self.preview {
            return false;
        }
        // The chrome picks tools, colours and sizes. A colour or size click
        // keeps a label being typed going, so it can be restyled. A slider
        // press starts a drag.
        if self.press_chrome(p) {
            return self.is_dragging();
        }
        let p = self.to_doc(p);
        self.pointer = p;
        self.last_drawn = None;
        // A press on the label being typed moves the caret (Shift extends
        // the selection), and dragging selects.
        if let (Some(l), Some(tf)) = (&self.typing, &self.typeface) {
            let pad = HIT_SLOP * self.ui_px();
            let p = rotate_about(p, -l.angle, l.pivot(tf));
            if contains(l.bounds(tf).with_outset((pad, pad)), p) {
                let at = l.index_at(tf, p);
                self.caret = crate::Caret {
                    at,
                    anchor: if self.shift { self.caret.anchor } else { at },
                };
                self.gesture = Some(Gesture::SelectText);
                self.fresh = false;
                return true;
            }
        }
        self.commit_text();
        self.fresh = false;
        // Shift+click adds a mark to the selection or takes it out.
        if self.shift
            && self.in_canvas(p)
            && let Some(i) = self.mark_at(p)
        {
            self.toggle_selected(i);
            return false;
        }
        if let Some((index, grip)) = self.grab_selected(p) {
            let others = self
                .selection()
                .into_iter()
                .filter(|&j| j != index)
                .map(|j| (j, self.marks[j].clone()))
                .collect();
            self.gesture = Some(Gesture::Edit {
                index,
                grip,
                from: p,
                original: self.marks[index].clone(),
                others,
            });
            return true;
        }
        if let Some(grip) = self.canvas_grip(p) {
            self.want_desktop = self.base_rect() != self.desktop;
            self.gesture = Some(Gesture::Canvas {
                grip,
                from: p,
                original: self.canvas,
                carry: Vec::new(),
            });
            return true;
        }
        self.deselect();
        if !self.in_canvas(p) {
            return false;
        }
        // The text tool places a caret, which follows the pointer until
        // the button is let go, or selects the mark clicked on (and so
        // switches to its tool).
        if self.tool == Tool::Text {
            if let Some(i) = self.mark_at(p) {
                self.select(Some(i));
                return false;
            }
            self.begin_text(p);
            self.gesture = Some(Gesture::PlaceText);
            return true;
        }
        self.gesture = Some(Gesture::Draw {
            drag: Drag {
                tool: self.tool,
                from: p,
                to: p,
            },
            path: vec![p],
            under: self.mark_at(p),
        });
        true
    }

    /// Move the pointer during a drag. Returns whether anything changed.
    pub fn drag_to(&mut self, p: Point) -> bool {
        let on_screen = p;
        let p = self.to_doc(p);
        // Moving and reshaping marks snap to the others (guides.rs).
        if matches!(self.gesture, Some(Gesture::Edit { .. })) {
            return self.drag_edit(p);
        }
        let ui = self.ui_px();
        let bounds = self.base_rect();
        match self.gesture.as_mut() {
            Some(Gesture::Draw { drag, path, .. }) if drag.to != p => {
                drag.to = p;
                // The pen keeps samples a couple of pixels apart; denser
                // ones only add jitter.
                if drag.tool == Tool::Pen
                    && path
                        .last()
                        .is_none_or(|last| (p - *last).length() >= PEN_STEP * ui)
                {
                    path.push(p);
                }
                self.fresh = false;
                true
            }
            Some(Gesture::Canvas {
                grip,
                from,
                original,
                carry,
            }) => {
                let Some(c) = resized(*original, *grip, p - *from, bounds) else {
                    return false;
                };
                if c == self.canvas {
                    return false;
                }
                // Carried marks keep their place in the selection.
                let d = Point::new(
                    (c.left - original.left) as f32,
                    (c.top - original.top) as f32,
                );
                for (i, m) in carry.iter() {
                    let mut moved = m.clone();
                    moved.offset(d);
                    self.marks[*i] = moved;
                }
                self.canvas = c;
                self.loupe_at = Some(loupe_point(c, *grip, p));
                self.fresh = false;
                true
            }
            Some(Gesture::Slide { slider, track, .. }) => {
                let (slider, track) = (*slider, *track);
                self.slide(slider, chrome::slider_t(track, on_screen, self.scale))
            }
            Some(Gesture::SelectText) => {
                let (Some(l), Some(tf)) = (&self.typing, &self.typeface) else {
                    return false;
                };
                let p = rotate_about(p, -l.angle, l.pivot(tf));
                let at = l.index_at(tf, p);
                if at == self.caret.at {
                    return false;
                }
                self.caret.at = at;
                self.fresh = false;
                true
            }
            Some(Gesture::PlaceText) => {
                let Some(l) = self.typing.as_mut() else {
                    return false;
                };
                // Where begin_text puts it: left edge at the pointer,
                // vertically centred on it.
                let origin = Point::new(p.x, p.y + l.size * 0.35);
                if l.origin == origin {
                    return false;
                }
                l.origin = origin;
                self.fresh = false;
                true
            }
            _ => false,
        }
    }

    /// Release the pointer. Returns whether anything changed: a new mark
    /// was kept, the selected one moved, or the canvas was resized. A click
    /// without a drag selects the mark under it, or clears the selection.
    pub fn end_drag(&mut self) -> bool {
        self.fresh = false;
        self.guides.clear();
        self.loupe_at = None;
        match self.gesture.take() {
            Some(Gesture::Draw {
                drag,
                mut path,
                under,
            }) => {
                // A click on a mark with the steps or sticker tool selects
                // it rather than dropping one on it.
                let click = (drag.to - drag.from).length() < MIN_ARROW_PX;
                if matches!(drag.tool, Tool::Steps | Tool::Sticker) && click && under.is_some() {
                    self.select(under);
                    return false;
                }
                if drag.tool == Tool::Pen && path.last() != Some(&drag.to) {
                    path.push(drag.to);
                }
                match self.shape(drag, &path) {
                    Some(m) => {
                        self.checkpoint();
                        self.marks.push(m);
                        self.last_drawn = Some(self.marks.len() - 1);
                        true
                    }
                    None => {
                        self.select(under);
                        false
                    }
                }
            }
            Some(Gesture::Edit {
                index,
                grip,
                original,
                others,
                from,
            }) => {
                if self.marks[index] == original {
                    // A click on the rotate handle without turning (or a
                    // double-click on it) stands the mark back upright.
                    if grip == Some(Grip::Rotate) && original.angle().is_some_and(|a| a != 0.0) {
                        self.checkpoint();
                        self.marks[index].set_angle(0.0);
                        return true;
                    }
                    // A click on a bent label's bend handle without bending
                    // straightens it, its ends staying put.
                    if grip == Some(Grip::Bend)
                        && let (Mark::Label(l), Some(tf)) = (&original, &self.typeface)
                        && l.bend != 0.0
                    {
                        let straight = grips::straightened(l, tf);
                        self.checkpoint();
                        self.marks[index] = Mark::Label(straight);
                        return true;
                    }
                    // A click on a label that was already selected (the
                    // second half of a double-click) edits its text.
                    if grip.is_none() && others.is_empty() && matches!(original, Mark::Label(_)) {
                        self.edit_label(index, Some(from));
                        return true;
                    }
                    return false;
                }
                let mut before = self.state();
                before.marks[index] = original;
                for (j, o) in others {
                    before.marks[j] = o;
                }
                self.undo.push(before);
                self.redo.clear();
                true
            }
            Some(Gesture::Canvas {
                original, carry, ..
            }) => {
                if self.canvas == original {
                    return false;
                }
                let mut before = self.state();
                before.canvas = original;
                for (i, m) in carry {
                    before.marks[i] = m;
                }
                self.undo.push(before);
                self.redo.clear();
                true
            }
            // The label is still being typed; it's kept when it's done.
            Some(Gesture::PlaceText | Gesture::SelectText) => false,
            Some(Gesture::Slide { before, .. }) => {
                self.chrome_key = None;
                if before.marks == self.marks {
                    return false;
                }
                self.undo.push(before);
                self.redo.clear();
                true
            }
            None => false,
        }
    }

    pub fn is_dragging(&self) -> bool {
        self.gesture.is_some()
    }

    /// The annotation a drag makes, or `None` for a stray click. `path` is
    /// the pen's samples.
    pub(super) fn shape(&self, d: Drag, path: &[Point]) -> Option<Mark> {
        let style = self.style_for(d.tool);
        let (color, width, look) = (
            self.colour_of(style),
            self.width_of(style),
            self.look_of(style),
        );
        let rect = Rect::from_ltrb(d.from.x, d.from.y, d.to.x, d.to.y).sorted();
        let big = rect.width() >= MIN_BOX_PX && rect.height() >= MIN_BOX_PX;
        match d.tool {
            Tool::Arrow if (d.to - d.from).length() >= MIN_ARROW_PX => Some(Mark::Arrow(Arrow {
                from: d.from,
                to: d.to,
                color,
                width,
                look,
                bend: 0.0,
            })),
            Tool::Box if big => Some(Mark::Box(Boxed {
                rect,
                color,
                width,
                look,
                radius: None,
                angle: 0.0,
            })),
            Tool::Highlight if big => Some(Mark::Highlight(Highlight {
                rect,
                color,
                look: style::highlight_of(style),
            })),
            Tool::Blur if big => Some(Mark::Redact(rect, Redaction::Blur(style.size))),
            Tool::Pixelate if big => Some(Mark::Redact(rect, Redaction::Pixelate(style.size))),
            Tool::Redact if big => Some(Mark::Redact(rect, Redaction::Solid)),
            Tool::Line if (d.to - d.from).length() >= MIN_ARROW_PX => Some(Mark::Line(Line {
                from: d.from,
                to: d.to,
                color,
                width,
                look,
                bend: 0.0,
            })),
            Tool::Ellipse if big => Some(Mark::Oval(Oval {
                rect,
                color,
                width,
                look,
                angle: 0.0,
            })),
            Tool::Pen => {
                let length: f32 = path.windows(2).map(|w| (w[1] - w[0]).length()).sum();
                (length >= MIN_ARROW_PX).then(|| {
                    Mark::Stroke(Stroke {
                        points: path.to_vec(),
                        color,
                        width,
                        look,
                    })
                })
            }
            // A click stamps a sticker there; a drag sizes it, square, from
            // the press.
            Tool::Sticker => {
                let d2 = d.to - d.from;
                let side = d2.x.abs().max(d2.y.abs());
                let rect = if side < STICKER_DRAG_PX * self.scale {
                    let half = STICKER_PX * self.scale / 2.0;
                    Rect::from_ltrb(
                        d.from.x - half,
                        d.from.y - half,
                        d.from.x + half,
                        d.from.y + half,
                    )
                } else {
                    let x = if d2.x < 0.0 {
                        d.from.x - side
                    } else {
                        d.from.x
                    };
                    let y = if d2.y < 0.0 {
                        d.from.y - side
                    } else {
                        d.from.y
                    };
                    Rect::from_xywh(x, y, side, side)
                };
                Some(Mark::Sticker(crate::Sticker {
                    rect,
                    name: self.sticker(),
                    angle: 0.0,
                }))
            }
            // A step goes where the press was, whether or not it moved.
            Tool::Steps => Some(Mark::Step(Step {
                center: d.from,
                color,
                radius: self.text_size_of(style) * super::style::STEP_RADIUS,
                look,
            })),
            _ => None,
        }
    }
}
