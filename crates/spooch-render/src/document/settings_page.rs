//! The settings window, drawn by the core like the editor's chrome: rows
//! of shortcuts, switches, steppers and buttons under headings. The app
//! says what the rows are (`SettingsItem`); the page lays them out, draws
//! them, and turns clicks and keys into `SettingsEvent`s naming the row.

use skia_safe::{Font, Point, RRect, Rect, Surface};

use super::theme::{
    ACCENT, ACCENT_HOVER, BUTTON, DIVIDER, FIELD, HOVER, MUTED, OUTLINE, PANEL, TEXT, WHITE,
    centred_baseline, chevron, ellipsize, fill, fill_rrect, stroke,
};
use super::{Key, Mods, canvas_surface};
use crate::Label;

/// One row of the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsItem {
    Heading(String),
    /// A shortcut, set by clicking it and pressing the keys; `clearable`
    /// ones can be none.
    Shortcut {
        label: String,
        keys: Option<String>,
        clearable: bool,
    },
    Switch {
        label: String,
        on: bool,
    },
    /// A number with buttons to lower and raise it.
    Stepper {
        label: String,
        value: String,
    },
    /// A value each click moves on to the next.
    Choice {
        label: String,
        value: String,
    },
    /// A line of text, with buttons at its end.
    Buttons {
        label: String,
        buttons: Vec<String>,
    },
    Link(String),
}

/// What a click or key did, naming the row by its index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsEvent {
    /// A switch, a choice or a link was clicked.
    Click(usize),
    /// A row's button.
    Button(usize, usize),
    /// A stepper went down (-1) or up (1).
    Step(usize, i8),
    /// Keys were pressed for a shortcut, or `None` to clear it.
    Shortcut(usize, Option<(Mods, Key)>),
    /// A shortcut started or stopped listening. Global shortcuts must be
    /// paused meanwhile, or they'd fire instead of reaching the page.
    Recording(bool),
}

/// Sizes in screen pixels, multiplied by the display scale.
const WIDTH: f32 = 440.0;
const PAD: f32 = 20.0;
const ROW: f32 = 40.0;
const HEADING: f32 = 36.0;
const CONTROL_H: f32 = 26.0;
const FIELD_W: f32 = 160.0;
const CHOICE_W: f32 = 120.0;
const SWITCH_W: f32 = 38.0;
const SWITCH_H: f32 = 22.0;
const STEP_VALUE_W: f32 = 74.0;
const BUTTON_GAP: f32 = 8.0;
const NOTE: f32 = 40.0;
const TEXT_SIZE: f32 = 13.0;
const HEADING_SIZE: f32 = 11.0;

/// Something on the page a click can hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    /// Row `i`'s control: its field, switch, choice or link.
    Control(usize),
    Button(usize, usize),
    Step(usize, i8),
}

/// The settings window's contents.
pub struct SettingsPage {
    items: Vec<SettingsItem>,
    scale: f32,
    font: Option<Font>,
    heading_font: Option<Font>,
    hovered: Option<Hit>,
    /// The shortcut row listening for keys.
    recording: Option<usize>,
    /// A line under the rows: a problem, or how recording works.
    note: Option<String>,
    surface: Option<Surface>,
    fresh: bool,
}

impl SettingsPage {
    pub fn new(scale: f32) -> SettingsPage {
        let tf = Label::ui_typeface();
        let font = |size: f32| {
            tf.as_ref()
                .map(|tf| Font::from_typeface(tf.clone(), size * scale))
        };
        SettingsPage {
            items: Vec::new(),
            scale,
            font: font(TEXT_SIZE),
            heading_font: font(HEADING_SIZE),
            hovered: None,
            recording: None,
            note: None,
            surface: None,
            fresh: false,
        }
    }

    /// Show these rows. A shortcut still listening keeps listening.
    pub fn set_items(&mut self, items: Vec<SettingsItem>) {
        if items != self.items {
            self.items = items;
            self.fresh = false;
        }
    }

    /// The line under the rows; `None` clears it.
    pub fn set_note(&mut self, note: Option<String>) {
        if note != self.note {
            self.note = note;
            self.fresh = false;
        }
    }

    /// Whether a shortcut is listening for keys.
    pub fn recording(&self) -> bool {
        self.recording.is_some()
    }

    /// The page's size in pixels: what the window's inside should be.
    pub fn size(&self) -> (i32, i32) {
        let rows: f32 = self.items.iter().map(row_height).sum();
        let h = PAD + rows + NOTE;
        (
            (WIDTH * self.scale).round() as i32,
            (h * self.scale).round() as i32,
        )
    }

    /// Each row's rectangle, top to bottom.
    fn rows(&self) -> Vec<Rect> {
        let s = self.scale;
        let mut y = PAD * s / 2.0;
        self.items
            .iter()
            .map(|item| {
                let h = row_height(item) * s;
                let r = Rect::from_xywh(PAD * s, y, (WIDTH - 2.0 * PAD) * s, h);
                y += h;
                r
            })
            .collect()
    }

    /// Everything a click can hit, with where it is.
    fn hits(&self) -> Vec<(Hit, Rect)> {
        let s = self.scale;
        let mut out = Vec::new();
        for (i, (item, row)) in self.items.iter().zip(self.rows()).enumerate() {
            let mid = row.center_y();
            let right = |w: f32, h: f32| {
                Rect::from_xywh(row.right - w * s, mid - h * s / 2.0, w * s, h * s)
            };
            match item {
                SettingsItem::Heading(_) => {}
                SettingsItem::Shortcut { .. } => {
                    out.push((Hit::Control(i), right(FIELD_W, CONTROL_H)))
                }
                SettingsItem::Switch { .. } => {
                    out.push((Hit::Control(i), right(SWITCH_W, SWITCH_H)))
                }
                SettingsItem::Choice { .. } => {
                    out.push((Hit::Control(i), right(CHOICE_W, CONTROL_H)))
                }
                SettingsItem::Stepper { .. } => {
                    let plus = right(CONTROL_H, CONTROL_H);
                    let minus = plus.with_offset((-(CONTROL_H + STEP_VALUE_W) * s, 0.0));
                    out.push((Hit::Step(i, -1), minus));
                    out.push((Hit::Step(i, 1), plus));
                }
                SettingsItem::Buttons { buttons, .. } => {
                    let mut x = row.right;
                    for (b, text) in buttons.iter().enumerate().rev() {
                        let w = self.text_width(text) + 24.0 * s;
                        x -= w;
                        out.push((
                            Hit::Button(i, b),
                            Rect::from_xywh(x, mid - CONTROL_H * s / 2.0, w, CONTROL_H * s),
                        ));
                        x -= BUTTON_GAP * s;
                    }
                }
                SettingsItem::Link(text) => {
                    let w = self.text_width(text);
                    out.push((
                        Hit::Control(i),
                        Rect::from_xywh(row.left, mid - CONTROL_H * s / 2.0, w, CONTROL_H * s),
                    ));
                }
            }
        }
        out
    }

    fn hit(&self, p: Point) -> Option<Hit> {
        self.hits()
            .into_iter()
            .find(|(_, r)| skia_safe::Contains::contains(r, p))
            .map(|(h, _)| h)
    }

    fn text_width(&self, text: &str) -> f32 {
        match &self.font {
            Some(f) => f.measure_str(text, None).0,
            None => text.len() as f32 * 7.0 * self.scale,
        }
    }

    /// The pointer moved to `p`, or left the window with `None`. Returns
    /// whether the page looks different.
    pub fn hover(&mut self, p: Option<Point>) -> bool {
        let hovered = p.and_then(|p| self.hit(p));
        if hovered == self.hovered {
            return false;
        }
        self.hovered = hovered;
        self.fresh = false;
        true
    }

    /// A click at `p`. A click anywhere stops a shortcut listening.
    pub fn press(&mut self, p: Point) -> Vec<SettingsEvent> {
        let was = self.recording;
        let mut out = self.stop_recording();
        match self.hit(p) {
            Some(Hit::Control(i)) => match self.items.get(i) {
                Some(SettingsItem::Shortcut { .. }) => {
                    // A second click on the one listening just stops it.
                    if was == Some(i) {
                        return out;
                    }
                    self.recording = Some(i);
                    self.note = Some(RECORD_HINT.into());
                    out.push(SettingsEvent::Recording(true));
                }
                Some(_) => out.push(SettingsEvent::Click(i)),
                None => {}
            },
            Some(Hit::Button(i, b)) => out.push(SettingsEvent::Button(i, b)),
            Some(Hit::Step(i, d)) => out.push(SettingsEvent::Step(i, d)),
            None => {}
        }
        self.fresh = false;
        out
    }

    /// A key while the window has focus. Only a listening shortcut takes
    /// keys: Esc stops it, Backspace or Delete clears a clearable one, and
    /// a key with Ctrl or Alt held (or an F key alone) becomes it.
    pub fn key(&mut self, key: Key, mods: Mods) -> Vec<SettingsEvent> {
        let Some(i) = self.recording else {
            return Vec::new();
        };
        let clearable = matches!(
            self.items.get(i),
            Some(SettingsItem::Shortcut {
                clearable: true,
                ..
            })
        );
        let set = match key {
            Key::Escape => None,
            Key::Backspace | Key::Delete if clearable => Some(None),
            Key::Char(c) if c.is_ascii_alphanumeric() && (mods.ctrl || mods.alt) => {
                Some(Some((mods, key)))
            }
            Key::F(_) | Key::PrintScreen => Some(Some((mods, key))),
            _ => return Vec::new(),
        };
        let mut out = self.stop_recording();
        if let Some(keys) = set {
            out.push(SettingsEvent::Shortcut(i, keys));
        }
        out
    }

    /// Stop a shortcut listening, saying so if one was.
    fn stop_recording(&mut self) -> Vec<SettingsEvent> {
        if self.recording.take().is_none() {
            return Vec::new();
        }
        if self.note.as_deref() == Some(RECORD_HINT) {
            self.note = None;
        }
        self.fresh = false;
        vec![SettingsEvent::Recording(false)]
    }

    /// The page as premultiplied BGRA rows, and their stride; nothing if
    /// Skia couldn't make a surface.
    pub fn render(&mut self) -> (&[u8], usize) {
        let size = self.size();
        if self
            .surface
            .as_ref()
            .is_none_or(|s| (s.width(), s.height()) != size)
        {
            self.surface = canvas_surface(size);
            self.fresh = false;
        }
        if !self.fresh {
            self.draw();
            self.fresh = true;
        }
        let Some(pixmap) = self.surface.as_mut().and_then(|s| s.peek_pixels()) else {
            return (&[], 0);
        };
        let stride = pixmap.row_bytes();
        (pixmap.bytes().unwrap_or_default(), stride)
    }

    fn draw(&mut self) {
        if let Some(mut surface) = self.surface.take() {
            self.paint(surface.canvas());
            self.surface = Some(surface);
        }
    }

    fn paint(&self, canvas: &skia_safe::Canvas) {
        let s = self.scale;
        let rows = self.rows();
        let hits = self.hits();
        let h = self.size().1;
        canvas.clear(PANEL);
        let place = |hit: Hit| hits.iter().find(|(h, _)| *h == hit).map(|(_, r)| *r);
        let text = |canvas: &skia_safe::Canvas, t: &str, x: f32, mid: f32, colour| {
            if let Some(f) = &self.font {
                canvas.draw_str(t, (x, centred_baseline(f, mid)), f, &fill(colour));
            }
        };
        let centred = |canvas: &skia_safe::Canvas, t: &str, r: Rect, colour| {
            let x = r.center_x() - self.text_width(t) / 2.0;
            text(canvas, t, x, r.center_y(), colour);
        };
        let button = |canvas: &skia_safe::Canvas, r: Rect, hover: bool| {
            fill_rrect(canvas, r, 6.0 * s, if hover { HOVER } else { BUTTON });
        };
        for (i, (item, row)) in self.items.iter().zip(&rows).enumerate() {
            let mid = row.center_y();
            let hover = |hit: Hit| self.hovered == Some(hit);
            match item {
                SettingsItem::Heading(t) => {
                    // A rule above every heading but the first.
                    if i > 0 {
                        let y = row.top + 6.0 * s;
                        canvas.draw_line((row.left, y), (row.right, y), &stroke(DIVIDER, s));
                    }
                    if let Some(f) = &self.heading_font {
                        let y = row.bottom - 9.0 * s;
                        canvas.draw_str(t.to_uppercase(), (row.left, y), f, &fill(MUTED));
                    }
                }
                SettingsItem::Shortcut { label, keys, .. } => {
                    text(canvas, label, row.left, mid, TEXT);
                    let Some(r) = place(Hit::Control(i)) else {
                        continue;
                    };
                    let listening = self.recording == Some(i);
                    fill_rrect(canvas, r, 6.0 * s, FIELD);
                    let edge = if listening {
                        ACCENT
                    } else if hover(Hit::Control(i)) {
                        OUTLINE
                    } else {
                        DIVIDER
                    };
                    let rr = RRect::new_rect_xy(r.with_inset((0.5 * s, 0.5 * s)), 6.0 * s, 6.0 * s);
                    canvas.draw_rrect(rr, &stroke(edge, if listening { 2.0 * s } else { s }));
                    let (t, colour) = match (listening, keys) {
                        (true, _) => ("Press keys\u{2026}", ACCENT_HOVER),
                        (false, Some(k)) => (k.as_str(), TEXT),
                        (false, None) => ("None", MUTED),
                    };
                    centred(canvas, t, r, colour);
                }
                SettingsItem::Switch { label, on } => {
                    text(canvas, label, row.left, mid, TEXT);
                    let Some(r) = place(Hit::Control(i)) else {
                        continue;
                    };
                    let track = if *on {
                        if hover(Hit::Control(i)) {
                            ACCENT_HOVER
                        } else {
                            ACCENT
                        }
                    } else if hover(Hit::Control(i)) {
                        OUTLINE
                    } else {
                        BUTTON
                    };
                    fill_rrect(canvas, r, r.height() / 2.0, track);
                    let k = r.height() / 2.0 - 3.0 * s;
                    let x = if *on {
                        r.right - r.height() / 2.0
                    } else {
                        r.left + r.height() / 2.0
                    };
                    canvas.draw_circle((x, r.center_y()), k, &fill(WHITE));
                }
                SettingsItem::Stepper { label, value } => {
                    text(canvas, label, row.left, mid, TEXT);
                    for d in [-1i8, 1] {
                        let Some(r) = place(Hit::Step(i, d)) else {
                            continue;
                        };
                        button(canvas, r, hover(Hit::Step(i, d)));
                        // A minus, and a plus.
                        let p = stroke(TEXT, 1.6 * s);
                        let (c, a) = (r.center(), 5.0 * s);
                        canvas.draw_line((c.x - a, c.y), (c.x + a, c.y), &p);
                        if d > 0 {
                            canvas.draw_line((c.x, c.y - a), (c.x, c.y + a), &p);
                        }
                    }
                    if let (Some(m), Some(p)) = (place(Hit::Step(i, -1)), place(Hit::Step(i, 1))) {
                        centred(
                            canvas,
                            value,
                            Rect::from_ltrb(m.right, m.top, p.left, m.bottom),
                            TEXT,
                        );
                    }
                }
                SettingsItem::Choice { label, value } => {
                    text(canvas, label, row.left, mid, TEXT);
                    let Some(r) = place(Hit::Control(i)) else {
                        continue;
                    };
                    button(canvas, r, hover(Hit::Control(i)));
                    text(canvas, value, r.left + 12.0 * s, mid, TEXT);
                    chevron(canvas, Point::new(r.right - 13.0 * s, mid), s, MUTED);
                }
                SettingsItem::Buttons { label, buttons } => {
                    let first = place(Hit::Button(i, 0)).map_or(row.right, |r| r.left);
                    if let Some(f) = &self.font {
                        let fit = ellipsize(f, label, first - row.left - 12.0 * s);
                        text(canvas, &fit, row.left, mid, MUTED);
                    }
                    for (b, t) in buttons.iter().enumerate() {
                        let Some(r) = place(Hit::Button(i, b)) else {
                            continue;
                        };
                        button(canvas, r, hover(Hit::Button(i, b)));
                        centred(canvas, t, r, TEXT);
                    }
                }
                SettingsItem::Link(t) => {
                    let colour = if hover(Hit::Control(i)) {
                        ACCENT_HOVER
                    } else {
                        ACCENT
                    };
                    text(canvas, t, row.left, mid, colour);
                    if let Some(r) = place(Hit::Control(i)) {
                        let y = mid + 7.0 * s;
                        canvas.draw_line((r.left, y), (r.right, y), &stroke(colour, s));
                    }
                }
            }
        }
        if let Some(note) = &self.note
            && let Some(f) = &self.font
        {
            let mid = h as f32 - NOTE * s / 2.0;
            let fit = ellipsize(f, note, (WIDTH - 2.0 * PAD) * s);
            text(canvas, &fit, PAD * s, mid, MUTED);
        }
    }
}

/// What the note says while a shortcut listens.
const RECORD_HINT: &str = "Press the new shortcut. Esc cancels, Backspace clears.";

fn row_height(item: &SettingsItem) -> f32 {
    match item {
        SettingsItem::Heading(_) => HEADING,
        _ => ROW,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> SettingsPage {
        let mut p = SettingsPage::new(1.0);
        p.set_items(vec![
            SettingsItem::Heading("Shortcuts".into()),
            SettingsItem::Shortcut {
                label: "Capture".into(),
                keys: Some("Ctrl+Shift+2".into()),
                clearable: false,
            },
            SettingsItem::Shortcut {
                label: "Quit".into(),
                keys: None,
                clearable: true,
            },
            SettingsItem::Switch {
                label: "Print Screen".into(),
                on: false,
            },
            SettingsItem::Stepper {
                label: "Countdown".into(),
                value: "3 seconds".into(),
            },
            SettingsItem::Buttons {
                label: "Signed out".into(),
                buttons: vec!["Sign in".into(), "Create account".into()],
            },
            SettingsItem::Link("Open settings.txt".into()),
        ]);
        p
    }

    fn centre(p: &SettingsPage, hit: Hit) -> Point {
        p.hits()
            .into_iter()
            .find(|(h, _)| *h == hit)
            .map(|(_, r)| r.center())
            .unwrap()
    }

    #[test]
    fn clicks_name_their_row() {
        let mut p = page();
        assert_eq!(
            p.press(centre(&p, Hit::Control(3))),
            [SettingsEvent::Click(3)]
        );
        assert_eq!(
            p.press(centre(&p, Hit::Step(4, 1))),
            [SettingsEvent::Step(4, 1)]
        );
        assert_eq!(
            p.press(centre(&p, Hit::Button(5, 1))),
            [SettingsEvent::Button(5, 1)]
        );
        assert_eq!(
            p.press(centre(&p, Hit::Control(6))),
            [SettingsEvent::Click(6)]
        );
        assert!(p.press(Point::new(1.0, 1.0)).is_empty());
        let (w, h) = p.size();
        assert_eq!(w, 440);
        assert!(p.render().0.len() >= (w * h * 4) as usize);
    }

    #[test]
    fn shortcuts_record_the_keys_pressed() {
        let mut p = page();
        let ctrl_alt = Mods {
            ctrl: true,
            alt: true,
            shift: false,
        };
        assert_eq!(
            p.press(centre(&p, Hit::Control(1))),
            [SettingsEvent::Recording(true)]
        );
        assert!(p.recording());
        // A plain letter or a lone modifier isn't a shortcut.
        assert!(p.key(Key::Char('s'), Mods::default()).is_empty());
        // Backspace doesn't clear Capture, which must have one.
        assert!(p.key(Key::Backspace, Mods::default()).is_empty());
        assert_eq!(
            p.key(Key::Char('s'), ctrl_alt),
            [
                SettingsEvent::Recording(false),
                SettingsEvent::Shortcut(1, Some((ctrl_alt, Key::Char('s'))))
            ]
        );
        assert!(!p.recording());
        // Quit can be cleared; Esc just stops.
        p.press(centre(&p, Hit::Control(2)));
        assert_eq!(
            p.key(Key::Delete, Mods::default()),
            [
                SettingsEvent::Recording(false),
                SettingsEvent::Shortcut(2, None)
            ]
        );
        p.press(centre(&p, Hit::Control(2)));
        assert_eq!(
            p.key(Key::Escape, Mods::default()),
            [SettingsEvent::Recording(false)]
        );
        // A click elsewhere stops it too, and still clicks.
        p.press(centre(&p, Hit::Control(2)));
        assert_eq!(
            p.press(centre(&p, Hit::Control(3))),
            [SettingsEvent::Recording(false), SettingsEvent::Click(3)]
        );
    }
}
