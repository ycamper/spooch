//! The editor's keyboard, the same on every platform. The shells turn
//! their own key codes into a `Key` and act on the `KeyOutcome`; text a
//! key types goes to `type_char`.

use skia_safe::Point;

use super::{Document, TextKey, Tool};

/// A key, as the editor sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Enter,
    Escape,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    /// F1 to F24.
    F(u8),
    PrintScreen,
    /// Any other key, by the character it types without Shift (letters
    /// lower case): '1', 'a', '['.
    Char(char),
}

impl Key {
    /// How far an arrow key nudges, in pixels: one, or ten with Shift.
    /// The same while picking the area (the shell moves the pointer) and
    /// in the editor (`Document::key` moves marks or the canvas edge).
    pub fn nudge(self, shift: bool) -> Option<(i32, i32)> {
        let n = if shift { 10 } else { 1 };
        match self {
            Key::Left => Some((-n, 0)),
            Key::Right => Some((n, 0)),
            Key::Up => Some((0, -n)),
            Key::Down => Some((0, n)),
            _ => None,
        }
    }
}

/// Modifier keys held with a key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// The Share shortcut, as `Document::key` binds it.
macro_rules! share_shortcut {
    () => {
        "Ctrl+Shift+U"
    };
}
pub(super) use share_shortcut;

/// The Share shortcut, for telling people about it.
pub const SHARE_SHORTCUT: &str = share_shortcut!();

/// What a key press means for the shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyOutcome {
    /// Not an editor key. If it types text, pass that to `type_char`.
    Ignored,
    /// The editor used it; repaint if `changed`. Any text it would type
    /// must not reach `type_char`.
    Handled { changed: bool },
    /// Finish: copy, and save too when `save`.
    Copy { save: bool },
    /// Share it and put the link on the clipboard.
    Share,
    /// Finish without keeping anything.
    Cancel,
    /// Keep this capture and open the one before (`older`) or after it.
    Recent { older: bool },
}

impl Document {
    /// A key was let go: Space stops moving the shape being drawn.
    pub fn key_up(&mut self, key: Key) {
        if key == Key::Char(' ') {
            self.space = false;
        }
    }

    /// A key press in the editor.
    ///
    /// While a label is being typed, arrows, Home and End move the caret
    /// (Ctrl by word, Shift selecting), Backspace and Delete delete, Ctrl+A
    /// selects all, Enter starts a new line and Esc finishes the label
    /// (as does clicking away from it). Other Ctrl
    /// shortcuts still work (Ctrl+C copies with the label, Ctrl+Z drops it).
    ///
    /// Otherwise: Esc cancels; Enter or Ctrl+C copies; Ctrl+S saves too;
    /// Ctrl+Shift+U shares;
    /// Ctrl+Z undoes, Ctrl+Y or Ctrl+Shift+Z redoes; Ctrl+D duplicates;
    /// Ctrl+] and Ctrl+[ restack; Del or Backspace deletes; PageUp and
    /// PageDown step through recents; Shift+1 to 8 pick a colour; [ and ]
    /// step the size; digits pick the tool. Arrows nudge the selected
    /// marks, or with none selected the canvas edge nearest the pointer, a
    /// pixel at a time (ten with Shift). Space held while drawing moves the
    /// shape instead of stretching it.
    pub fn key(&mut self, key: Key, mods: Mods) -> KeyOutcome {
        let handled = |changed| KeyOutcome::Handled { changed };
        // Space while drawing moves the shape until it's let go.
        if key == Key::Char(' ') && self.drawing().is_some() {
            self.space = true;
            return handled(false);
        }
        // A run of nudges is one undo step; anything else ends the run.
        if key.nudge(false).is_none() {
            self.nudged = false;
            self.loupe_at = None;
        }
        // Typing the frame's title in the bar: Enter or Esc finishes it.
        if self.editing_title() && !mods.ctrl {
            return match key {
                Key::Enter | Key::Escape => {
                    self.set_editing_title(false);
                    handled(true)
                }
                Key::Backspace => {
                    let mut t = self.frame_title().to_string();
                    let changed = t.pop().is_some();
                    self.set_frame_title(&t);
                    handled(changed)
                }
                _ => KeyOutcome::Ignored,
            };
        }
        if self.is_typing() {
            let text_key = match key {
                Key::Left => Some(TextKey::Left),
                Key::Right => Some(TextKey::Right),
                Key::Up => Some(TextKey::Up),
                Key::Down => Some(TextKey::Down),
                Key::Home => Some(TextKey::Home),
                Key::End => Some(TextKey::End),
                Key::Backspace => Some(TextKey::Backspace),
                Key::Delete => Some(TextKey::Delete),
                _ => None,
            };
            if let Some(k) = text_key {
                return handled(self.text_key(k, mods.ctrl, mods.shift));
            }
            if mods.ctrl && key == Key::Char('a') {
                return handled(self.select_all_text());
            }
            if !mods.ctrl {
                return match key {
                    // Enter starts a new line; Esc finishes the label.
                    Key::Enter => handled(self.new_line()),
                    // Repaint even when nothing was kept, to drop the caret.
                    Key::Escape => {
                        self.commit_text();
                        handled(true)
                    }
                    // Letters and the like type into the label.
                    _ => KeyOutcome::Ignored,
                };
            }
        }
        if mods.ctrl {
            return match key {
                Key::Char('c') => KeyOutcome::Copy { save: false },
                Key::Char('u') if mods.shift => KeyOutcome::Share,
                Key::Char('s') => KeyOutcome::Copy { save: true },
                Key::Char('z') if mods.shift => handled(self.redo()),
                Key::Char('z') => handled(self.undo()),
                Key::Char('y') => handled(self.redo()),
                Key::Char('d') => handled(self.duplicate_selected()),
                Key::Char(']') => handled(self.restack_selected(true)),
                Key::Char('[') => handled(self.restack_selected(false)),
                // In the editor window: Ctrl+0 fits, Ctrl+1 is 100%, and
                // Ctrl+= and Ctrl+- step the zoom.
                Key::Char('0') if self.in_window() => handled(self.zoom_fit()),
                Key::Char('h') if self.in_window() => handled(self.toggle_gallery()),
                Key::Char('1') if self.in_window() => handled(self.zoom_actual()),
                Key::Char('=') if self.in_window() => handled(self.zoom_step(true, None)),
                Key::Char('-') if self.in_window() => handled(self.zoom_step(false, None)),
                _ => KeyOutcome::Ignored,
            };
        }
        match key {
            Key::Escape => KeyOutcome::Cancel,
            Key::Enter => KeyOutcome::Copy { save: false },
            Key::PageUp => KeyOutcome::Recent { older: true },
            Key::PageDown => KeyOutcome::Recent { older: false },
            Key::Delete | Key::Backspace => handled(self.delete_selected()),
            _ if mods.alt => KeyOutcome::Ignored,
            Key::Left | Key::Right | Key::Up | Key::Down => {
                handled(key.nudge(mods.shift).is_some_and(|d| self.nudge(d)))
            }
            Key::Char(d @ '1'..='8') if mods.shift => {
                handled(self.set_colour(d as usize - '1' as usize))
            }
            Key::Char(c @ ('[' | ']')) => handled(self.step_size(c == ']')),
            Key::Char(c) if !mods.shift => match Tool::from_digit(c) {
                Some(tool) => handled(self.set_tool(tool)),
                None => KeyOutcome::Ignored,
            },
            _ => KeyOutcome::Ignored,
        }
    }

    /// Text a key typed, after `key` ignored it: into the label being
    /// typed, or else it starts a label at `pointer` when that's on the
    /// canvas. Returns whether anything changed.
    pub fn type_char(&mut self, c: char, pointer: Point) -> bool {
        if c.is_control() {
            return false;
        }
        if self.editing_title() {
            let mut t = self.frame_title().to_string();
            t.push(c);
            self.set_frame_title(&t);
            return true;
        }
        let mut buf = [0; 4];
        let text = c.encode_utf8(&mut buf);
        if self.is_typing() {
            return self.type_text(text);
        }
        let pointer = self.to_doc(pointer);
        if c.is_whitespace() || !self.in_canvas(pointer) {
            return false;
        }
        self.begin_text(pointer);
        self.type_text(text)
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{click, drag, placed};
    use super::*;
    use skia_safe::IRect;

    const NONE: Mods = Mods {
        ctrl: false,
        shift: false,
        alt: false,
    };
    const CTRL: Mods = Mods {
        ctrl: true,
        shift: false,
        alt: false,
    };
    const SHIFT: Mods = Mods {
        ctrl: false,
        shift: true,
        alt: false,
    };

    #[test]
    fn keys_pick_tools_colours_and_sizes() {
        let (mut d, _) = placed();
        assert_eq!(
            d.key(Key::Char('2'), NONE),
            KeyOutcome::Handled { changed: true }
        );
        assert_eq!(d.tool(), Tool::Box);
        d.key(Key::Char('5'), SHIFT);
        assert_eq!(d.tool_style().color, 4);
        d.key(Key::Char(']'), NONE);
        assert_eq!(d.tool_style().size, 3);
        // A letter isn't a shortcut: it types.
        assert_eq!(d.key(Key::Char('h'), NONE), KeyOutcome::Ignored);
        assert!(d.type_char('H', Point::new(150.0, 150.0)));
        assert!(d.is_typing());
    }

    #[test]
    fn typing_keys_edit_the_label_and_shortcuts_still_work() {
        let (mut d, _) = placed();
        d.type_char('H', Point::new(150.0, 150.0));
        // Digits and brackets type while a label is open.
        assert_eq!(d.key(Key::Char('2'), NONE), KeyOutcome::Ignored);
        d.type_char('2', Point::new(0.0, 0.0));
        assert_eq!(d.typing.as_ref().unwrap().text, "H2");
        d.key(Key::Left, NONE);
        d.key(Key::Backspace, NONE);
        assert_eq!(d.typing.as_ref().unwrap().text, "2");
        d.key(Key::Char('a'), CTRL);
        assert_eq!(d.caret.anchor, 0);
        // Esc finishes the label rather than cancelling.
        assert_eq!(
            d.key(Key::Escape, NONE),
            KeyOutcome::Handled { changed: true }
        );
        assert!(!d.is_typing());
        assert_eq!(d.key(Key::Escape, NONE), KeyOutcome::Cancel);
        // Ctrl+C copies even mid-label.
        d.type_char('x', Point::new(150.0, 180.0));
        assert_eq!(
            d.key(Key::Char('c'), CTRL),
            KeyOutcome::Copy { save: false }
        );
    }

    #[test]
    fn edit_shortcuts() {
        let (mut d, _) = placed();
        assert!(drag(&mut d, (120.0, 150.0), (280.0, 150.0)));
        click(&mut d, 200.0, 150.0);
        assert_eq!(
            d.key(Key::Char('d'), CTRL),
            KeyOutcome::Handled { changed: true }
        );
        assert_eq!(d.marks.len(), 2);
        assert_eq!(
            d.key(Key::Delete, NONE),
            KeyOutcome::Handled { changed: true }
        );
        assert_eq!(d.marks.len(), 1);
        d.key(Key::Char('z'), CTRL);
        assert_eq!(d.marks.len(), 2);
        d.key(
            Key::Char('z'),
            Mods {
                shift: true,
                ..CTRL
            },
        );
        assert_eq!(d.marks.len(), 1);
        assert_eq!(d.key(Key::Char('s'), CTRL), KeyOutcome::Copy { save: true });
        let ctrl_shift = Mods {
            shift: true,
            ..CTRL
        };
        assert_eq!(d.key(Key::Char('u'), ctrl_shift), KeyOutcome::Share);
        assert_eq!(d.key(Key::Char('u'), CTRL), KeyOutcome::Ignored);
        assert_eq!(d.key(Key::PageUp, NONE), KeyOutcome::Recent { older: true });
        assert_eq!(d.key(Key::Enter, NONE), KeyOutcome::Copy { save: false });
    }

    #[test]
    fn arrows_nudge_marks_then_the_canvas_edge() {
        let (mut d, desktop) = placed();
        assert!(drag(&mut d, (120.0, 150.0), (280.0, 150.0)));
        click(&mut d, 200.0, 150.0);
        let before = d.marks[0].clone();
        d.key(Key::Right, NONE);
        d.key(Key::Right, SHIFT);
        d.key(Key::Up, NONE);
        let mut moved = before.clone();
        moved.offset(Point::new(11.0, -1.0));
        assert!(d.marks[0] == moved);
        // The run of nudges is one undo step.
        d.key(Key::Char('z'), CTRL);
        assert!(d.marks[0] == before);

        // Nothing selected: the edge nearest the pointer. Inward crops at
        // once; outward waits for the desktop, then lands.
        assert!(!d.has_selection());
        d.pointer_moved(Point::new(290.0, 190.0));
        assert_eq!(d.canvas(), IRect::from_ltrb(100, 100, 300, 200));
        d.key(Key::Left, NONE);
        assert_eq!(d.canvas(), IRect::from_ltrb(100, 100, 299, 200));
        d.key(Key::Down, SHIFT);
        assert_eq!(d.wants_desktop(), Some(IRect::from_wh(400, 300)));
        assert_eq!(d.canvas().bottom, 200, "not until the pixels come");
        assert!(d.supply_desktop(400, 300, 400 * 4, &desktop));
        assert_eq!(d.canvas(), IRect::from_ltrb(100, 100, 299, 210));
        // Pointer on the left: the left edge moves.
        d.pointer_moved(Point::new(105.0, 150.0));
        d.key(Key::Left, SHIFT);
        assert_eq!(d.canvas().left, 90);
        d.key(Key::Char('z'), CTRL);
        assert_eq!(d.canvas(), IRect::from_ltrb(100, 100, 300, 200));
    }

    #[test]
    fn the_loupe_follows_edge_drags_and_nudges() {
        let (mut d, desktop) = placed();
        let count = |d: &mut Document| d.chrome().len();
        let plain = count(&mut d);
        // Dragging the right edge: a loupe on the edge's pixel, inside the
        // chrome's bounds, gone on release.
        d.begin_drag(Point::new(300.0, 150.0));
        d.supply_desktop(400, 300, 400 * 4, &desktop);
        d.drag_to(Point::new(320.0, 160.0));
        assert_eq!(d.loupe_at, Some((319, 160)));
        assert_eq!(count(&mut d), plain + 1);
        let l = d.chrome().last().unwrap().rect;
        let b = d.chrome_bounds();
        assert!(IRect::intersect(&b, &l) == Some(l), "{l:?} in {b:?}");
        d.end_drag();
        assert_eq!(count(&mut d), plain);
        // A nudge shows it on the nudged edge until the pointer moves.
        d.pointer_moved(Point::new(310.0, 190.0));
        d.key(Key::Down, NONE);
        assert_eq!(d.loupe_at, Some((310, 200)));
        assert!(!d.pointer_moved(Point::new(310.4, 190.0)) || d.loupe_at.is_some());
        assert!(d.loupe_at.is_some(), "a sub-pixel wobble keeps it");
        assert!(d.pointer_moved(Point::new(330.0, 190.0)));
        assert!(d.loupe_at.is_none());
    }
}
