//! Typing labels: placing the caret, editing and selecting text, and
//! committing the label.

use super::*;

impl Document {
    /// Start a label with its left edge at `p`, vertically centred on it.
    pub fn begin_text(&mut self, p: Point) {
        let style = self.style_for(Tool::Text);
        let size = self.text_size_of(style);
        self.commit_text();
        self.gesture = None;
        self.deselect();
        self.typing = Some(Label {
            origin: Point::new(p.x, p.y + size * 0.35),
            text: String::new(),
            color: self.colour_of(style),
            size,
            look: self.look_of(style),
            angle: 0.0,
            font: crate::LabelFont::nth(style.font),
            bend: 0.0,
            plate: crate::Plate::nth(style.variant),
        });
        self.caret = crate::Caret::default();
        self.fresh = false;
    }

    pub fn is_typing(&self) -> bool {
        self.typing.is_some()
    }

    /// Type into the label being typed, at the caret, replacing any
    /// selected text. Control characters are ignored.
    pub fn type_text(&mut self, text: &str) -> bool {
        let typed: String = text.chars().filter(|c| !c.is_control()).collect();
        if typed.is_empty() {
            return false;
        }
        self.replace_selection(&typed)
    }

    /// Start a new line at the caret (Enter), replacing any selected text.
    pub(super) fn new_line(&mut self) -> bool {
        self.replace_selection("\n")
    }

    /// Delete the character before the caret, or the selected text.
    #[cfg(test)]
    pub(super) fn backspace(&mut self) -> bool {
        self.text_key(TextKey::Backspace, false, false)
    }

    /// Select all the text in the label being typed (Ctrl+A).
    pub(crate) fn select_all_text(&mut self) -> bool {
        let Some(l) = &self.typing else {
            return false;
        };
        let n = l.text.chars().count();
        let before = self.caret;
        self.caret = crate::Caret { at: n, anchor: 0 };
        self.fresh &= before == self.caret;
        before != self.caret
    }

    /// A key that moves the caret or deletes, in the label being typed.
    /// `word` (Ctrl) moves or deletes a word at a time; `extend` (Shift)
    /// selects as the caret moves.
    pub(crate) fn text_key(&mut self, key: TextKey, word: bool, extend: bool) -> bool {
        let Some(l) = &self.typing else {
            return false;
        };
        let lines = l.lines();
        let line = l.line_of(self.caret.at);
        // As near the same place across as line `to` goes, for Up and Down.
        let across = |to: usize| -> usize {
            match &self.typeface {
                Some(tf) => {
                    let x = l.origin.x + l.advance_to(tf, self.caret.at);
                    l.index_at(tf, Point::new(x, l.baseline(to)))
                }
                None => lines[to].1,
            }
        };
        let chars: Vec<char> = l.text.chars().collect();
        let n = chars.len();
        let crate::Caret { at, anchor } = self.caret;
        // The next stop left or right: one character, or a word.
        let step = |from: usize, right: bool| -> usize {
            if !word {
                return if right {
                    (from + 1).min(n)
                } else {
                    from.saturating_sub(1)
                };
            }
            let mut i = from;
            if right {
                while i < n && chars[i].is_whitespace() {
                    i += 1;
                }
                while i < n && !chars[i].is_whitespace() {
                    i += 1;
                }
            } else {
                while i > 0 && chars[i - 1].is_whitespace() {
                    i -= 1;
                }
                while i > 0 && !chars[i - 1].is_whitespace() {
                    i -= 1;
                }
            }
            i
        };
        let to = match key {
            TextKey::Left | TextKey::Right if at != anchor && !extend => {
                // Collapses a selection to its edge.
                let edge = if key == TextKey::Left {
                    at.min(anchor)
                } else {
                    at.max(anchor)
                };
                self.caret = crate::Caret {
                    at: edge,
                    anchor: edge,
                };
                self.fresh = false;
                return true;
            }
            TextKey::Left => step(at, false),
            TextKey::Right => step(at, true),
            TextKey::Home => lines[line].0,
            TextKey::End => lines[line].1,
            TextKey::Up if line == 0 => 0,
            TextKey::Up => across(line - 1),
            TextKey::Down if line + 1 == lines.len() => n,
            TextKey::Down => across(line + 1),
            TextKey::Backspace | TextKey::Delete => {
                if at != anchor {
                    return self.replace_selection("");
                }
                let other = step(at, key == TextKey::Delete);
                if other == at {
                    return false;
                }
                self.caret.anchor = other;
                return self.replace_selection("");
            }
        };
        let caret = crate::Caret {
            at: to,
            anchor: if extend { anchor } else { to },
        };
        let changed = caret != self.caret;
        self.caret = caret;
        self.fresh &= !changed;
        changed
    }

    /// Replace the selected text (or nothing, at the caret) with `text`.
    fn replace_selection(&mut self, text: &str) -> bool {
        let Some(l) = self.typing.as_mut() else {
            return false;
        };
        let (a, b) = (
            self.caret.at.min(self.caret.anchor),
            self.caret.at.max(self.caret.anchor),
        );
        let byte = |i: usize| {
            l.text
                .char_indices()
                .nth(i)
                .map_or(l.text.len(), |(b, _)| b)
        };
        let (start, end) = (byte(a), byte(b));
        if start == end && text.is_empty() {
            return false;
        }
        l.text.replace_range(start..end, text);
        let at = a + text.chars().count();
        self.caret = crate::Caret { at, anchor: at };
        self.fresh = false;
        true
    }

    /// Start editing the text of label `i`: it comes out of `marks` and is
    /// typed into, with the caret where it was clicked (`at`) or at the
    /// end, until it's committed.
    pub(super) fn edit_label(&mut self, i: usize, at: Option<Point>) {
        self.commit_text();
        let Some(Mark::Label(l)) = self.marks.get(i).cloned() else {
            return;
        };
        self.marks.remove(i);
        self.last_drawn = None;
        self.deselect();
        self.tool = Tool::Text;
        let index = match (at, &self.typeface) {
            (Some(p), Some(tf)) => l.index_at(tf, rotate_about(p, -l.angle, l.pivot(tf))),
            _ => l.text.chars().count(),
        };
        self.caret = crate::Caret {
            at: index,
            anchor: index,
        };
        self.typing = Some(l.clone());
        self.editing = Some((i, l));
        self.fresh = false;
    }

    /// Finish the label being typed. Returns whether it was kept (blank
    /// labels are dropped).
    pub fn commit_text(&mut self) -> bool {
        let Some(label) = self.typing.take() else {
            return false;
        };
        self.fresh = false;
        // An edited label goes back where it was, as one undo step; emptied,
        // it's deleted.
        if let Some((i, original)) = self.editing.take() {
            let i = i.min(self.marks.len());
            if label == original {
                self.marks.insert(i, Mark::Label(label));
                self.last_drawn = Some(i);
                return false;
            }
            let mut before = self.state();
            before.marks.insert(i, Mark::Label(original));
            self.undo.push(before);
            self.redo.clear();
            if label.text.trim().is_empty() {
                return false;
            }
            self.marks.insert(i, Mark::Label(label));
            self.last_drawn = Some(i);
            return true;
        }
        if label.text.trim().is_empty() {
            return false;
        }
        self.checkpoint();
        self.marks.push(Mark::Label(label));
        self.last_drawn = Some(self.marks.len() - 1);
        true
    }
}
