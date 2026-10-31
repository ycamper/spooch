//! Undo history and the edits that act on the selection: duplicate,
//! restack, delete.

use super::*;

impl Document {
    pub(super) fn state(&self) -> State {
        State {
            marks: self.marks.clone(),
            canvas: self.canvas,
        }
    }

    pub(super) fn restore(&mut self, s: State) {
        self.nudged = false;
        self.loupe_at = None;
        self.pending_canvas = None;
        self.marks = s.marks;
        self.canvas = s.canvas;
        self.deselect();
        self.last_drawn = None;
        self.fresh = false;
    }

    /// Save the current state for undo, before changing it.
    pub(super) fn checkpoint(&mut self) {
        self.undo.push(self.state());
        self.redo.clear();
    }
}

impl Document {
    /// Copy the selected marks (Ctrl+D), a little down and right of the
    /// originals, and select the copies. One undo step.
    pub(crate) fn duplicate_selected(&mut self) -> bool {
        if self.typing.is_some() || self.gesture.is_some() {
            return false;
        }
        let all = self.selection();
        if all.is_empty() {
            return false;
        }
        self.checkpoint();
        let d = Point::new(
            DUPLICATE_OFFSET * self.ui_px(),
            DUPLICATE_OFFSET * self.ui_px(),
        );
        let copies: Vec<Mark> = all
            .iter()
            .map(|&i| {
                let mut copy = self.marks[i].clone();
                copy.offset(d);
                copy
            })
            .collect();
        self.add_selected(copies);
        true
    }

    /// Put `marks` on top and select them, as one selection.
    pub(super) fn add_selected(&mut self, marks: Vec<Mark>) {
        let first = self.marks.len();
        self.marks.extend(marks);
        self.last_drawn = None;
        self.selected = Some(first);
        self.also = (first + 1..self.marks.len()).collect();
        self.fresh = false;
        self.chrome_key = None;
    }

    /// Bring the selected marks in front of the others (Ctrl+]), or send
    /// them behind (Ctrl+[), keeping their own order. Redactions stay
    /// under highlights, and those under everything else. One undo step.
    pub(crate) fn restack_selected(&mut self, front: bool) -> bool {
        if self.typing.is_some() || self.gesture.is_some() {
            return false;
        }
        let mut chosen = self.selection();
        chosen.sort_unstable();
        chosen.dedup();
        if chosen.is_empty() {
            return false;
        }
        let rest: Vec<usize> = (0..self.marks.len())
            .filter(|i| !chosen.contains(i))
            .collect();
        let order: Vec<usize> = if front {
            rest.iter().chain(&chosen).copied().collect()
        } else {
            chosen.iter().chain(&rest).copied().collect()
        };
        if order.iter().copied().eq(0..self.marks.len()) {
            return false;
        }
        self.checkpoint();
        let old = std::mem::take(&mut self.marks);
        self.marks = order.iter().map(|&i| old[i].clone()).collect();
        // The selection follows the marks to where they went.
        let at = |i: usize| order.iter().position(|&j| j == i);
        self.selected = self.selected.and_then(at);
        self.also = self.also.iter().filter_map(|&i| at(i)).collect();
        self.fresh = false;
        true
    }

    /// Remove the selected mark. Returns whether there was one.
    pub(crate) fn delete_selected(&mut self) -> bool {
        if self.typing.is_some() || self.gesture.is_some() {
            return false;
        }
        let mut all = self.selection();
        if all.is_empty() {
            return false;
        }
        self.deselect();
        self.checkpoint();
        // From the back, so earlier indexes stay put.
        all.sort_unstable();
        all.dedup();
        for i in all.into_iter().rev() {
            if i < self.marks.len() {
                self.marks.remove(i);
            }
        }
        self.fresh = false;
        true
    }

    /// Discard whatever is in progress, or else step back one edit.
    /// Returns whether anything changed.
    pub(crate) fn undo(&mut self) -> bool {
        self.fresh = false;
        if self.typing.take().is_some() {
            // Undo while editing puts the label back as it was.
            if let Some((i, original)) = self.editing.take() {
                let i = i.min(self.marks.len());
                self.marks.insert(i, Mark::Label(original));
            }
            return true;
        }
        match self.gesture.take() {
            Some(Gesture::Edit {
                index,
                original,
                others,
                ..
            }) => {
                self.marks[index] = original;
                for (j, o) in others {
                    self.marks[j] = o;
                }
                return true;
            }
            Some(Gesture::Canvas {
                original, carry, ..
            }) => {
                self.canvas = original;
                for (i, m) in carry {
                    self.marks[i] = m;
                }
                return true;
            }
            Some(Gesture::Draw { .. }) => return true,
            Some(Gesture::PlaceText | Gesture::SelectText) => return true,
            Some(Gesture::Slide { before, .. }) => {
                self.marks = before.marks;
                self.chrome_key = None;
                return true;
            }
            None => {}
        }
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.state());
        self.restore(prev);
        true
    }

    /// Step forward again after an undo. Returns whether anything changed.
    pub(crate) fn redo(&mut self) -> bool {
        if self.typing.is_some() || self.gesture.is_some() {
            return false;
        }
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.state());
        self.restore(next);
        true
    }
}
