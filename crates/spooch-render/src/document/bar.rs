//! The output bar under the canvas: file name, format, frame, its colour
//! and title, shadow, size, and Copy and Save. Copy and Save are handed to
//! the shell as actions; everything else changes here.

use crate::document::fill;
use skia_safe::{Font, IRect, Paint, Point, RRect, Rect};

use super::Document;
use super::chrome::{Panel, contains, panel};
use super::theme::{
    ACCENT, ACCENT_HOVER, BUTTON, DIVIDER, FIELD, MUTED, SELECTED, TEXT, centred_baseline, chevron,
    draw_list, ellipsize, rounded_panel, swatch, tick_box,
};

/// Something on the bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarItem {
    Name,
    Format,
    /// The frame round the output.
    Frame,
    /// The drop shadow round the output.
    Shadow,
    /// The window frame's colour, for frames with a window top.
    FrameColour,
    /// The text the window frame shows; click to type it.
    FrameTitle,
    Size,
    /// Open the capture in the editor window (the overlay only).
    Window,
    Copy,
    Save,
    Share,
}

/// A list that opens over a bar button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BarMenu {
    /// Every frame, over the Frame button.
    Frame,
    /// Every frame colour, over the colour swatch.
    Colour,
}

impl BarMenu {
    fn button(self) -> BarItem {
        match self {
            BarMenu::Frame => BarItem::Frame,
            BarMenu::Colour => BarItem::FrameColour,
        }
    }

    /// The index of the "Use for every capture" row, the Frame menu's
    /// last; `None` in other menus.
    fn pin_row(self) -> Option<usize> {
        (self == BarMenu::Frame).then(|| super::Frame::all().len())
    }
}

/// What a click on the bar asks the shell to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Copy,
    Save,
    Share,
    /// Move the capture into the editor window.
    OpenWindow,
    /// One of the editor window's own buttons.
    Window(super::view::WindowButton),
    /// A press on the editor window's gallery.
    Gallery(super::GalleryAction),
    /// Open the settings window.
    Settings,
}

const BAR_H: f32 = 32.0;
const ITEM_H: f32 = 24.0;
const PAD: f32 = 4.0;
const GAP: f32 = 4.0;
pub(super) const TEXT_SIZE: f32 = 12.0;
const MARGIN: f32 = 8.0;
/// The least the window's footer cuts the name down to.
const MIN_NAME: f32 = 80.0;
/// A row of a bar list.
const MENU_ROW: f32 = 26.0;
/// The label of the Frame menu's last row.
const PIN: &str = "Use for every capture";
/// Room for a swatch or tick box before a menu row's name.
const ROW_MARK: f32 = 22.0;

/// Which line each item of `widths` goes on, filling each line up to
/// `room` with `gap` between items. An item wider than `room` gets a line
/// of its own.
fn wrap(widths: &[f32], room: f32, gap: f32) -> Vec<usize> {
    let (mut line, mut used) = (0, 0.0);
    widths
        .iter()
        .map(|&w| {
            if used > 0.0 && used + gap + w > room {
                line += 1;
                used = 0.0;
            }
            used += if used > 0.0 { gap + w } else { w };
            line
        })
        .collect()
}

/// Where the bar and its items sit.
pub(super) struct BarLayout {
    pub rect: IRect,
    pub items: Vec<(BarItem, IRect, String)>,
}

impl BarItem {
    pub(super) fn clickable(self) -> bool {
        matches!(
            self,
            BarItem::Format
                | BarItem::Frame
                | BarItem::Shadow
                | BarItem::FrameColour
                | BarItem::FrameTitle
                | BarItem::Window
                | BarItem::Copy
                | BarItem::Save
                | BarItem::Share
        )
    }

    pub(super) fn tip(self) -> &'static str {
        match self {
            BarItem::Format => "Click to switch between PNG and JPEG",
            BarItem::Frame => "Click to pick a frame; hold Alt to preview",
            BarItem::Shadow => "Click to change the shadow; hold Alt to preview",
            BarItem::FrameColour => "Click to pick the frame's colour",
            BarItem::FrameTitle => "Click to type the frame's title; Enter when done",
            BarItem::Window => "Ctrl+E   open in the editor window",
            BarItem::Copy => "Enter or Ctrl+C",
            BarItem::Save => "Ctrl+S, without a dialog",
            BarItem::Share => concat!(super::keys::share_shortcut!(), "   upload and copy a link"),
            BarItem::Name | BarItem::Size => "",
        }
    }
}

impl Document {
    /// The file name the capture is saved and dragged out under, without
    /// extension. Shown in the bar.
    pub fn set_name(&mut self, name: &str) {
        self.name = name.to_string();
        self.chrome_key = None;
    }

    /// What the Share button says instead of "Share", like "Uploading…";
    /// `None` puts it back.
    pub fn set_share_label(&mut self, label: Option<&str>) {
        self.share_label = label.map(str::to_string);
        self.chrome_key = None;
    }

    /// What the Save button says instead of "Save", like "Show" once the
    /// capture's saved; `None` puts it back.
    pub fn set_save_label(&mut self, label: Option<&str>) {
        self.save_label = label.map(str::to_string);
        self.chrome_key = None;
    }

    /// Whether the Save button shows the saved picture rather than saving.
    pub fn save_shows(&self) -> bool {
        self.save_label.is_some()
    }

    /// The bar action the last press asked for, if any. Each is handed out
    /// once.
    pub fn take_action(&mut self) -> Option<Action> {
        self.action.take()
    }

    pub(super) fn bar_font(&self) -> Option<Font> {
        self.ui_typeface
            .as_ref()
            .map(|tf| Font::from_typeface(tf.clone(), TEXT_SIZE * self.scale))
    }

    pub(super) fn text_width(&self, font: Option<&Font>, text: &str) -> f32 {
        match font {
            Some(f) => f.measure_str(text, None).0,
            None => text.len() as f32 * 7.0 * self.scale,
        }
    }

    pub(super) fn bar_layout(&self) -> BarLayout {
        let s = self.scale;
        let font = self.bar_font();
        let c = self.canvas;
        let texts: Vec<(BarItem, String)> = [
            (
                BarItem::Name,
                format!("{}.{}", self.name, self.format.extension()),
            ),
            (BarItem::Format, self.format.name().to_string()),
            (BarItem::Frame, self.frame().name().to_string()),
            (BarItem::FrameColour, String::new()),
            (
                BarItem::FrameTitle,
                if self.frame_title().is_empty() && !self.editing_title() {
                    "Title".to_string()
                } else {
                    self.frame_title().to_string()
                },
            ),
            (
                BarItem::Shadow,
                super::SHADOWS[usize::from(self.shadow())].0.to_string(),
            ),
            (
                BarItem::Size,
                format!("{} \u{00d7} {}", c.width(), c.height()),
            ),
            (BarItem::Window, "Open".to_string()),
            (BarItem::Copy, "Copy".to_string()),
            (
                BarItem::Save,
                self.save_label.clone().unwrap_or_else(|| "Save".into()),
            ),
            (
                BarItem::Share,
                self.share_label.clone().unwrap_or_else(|| "Share".into()),
            ),
        ]
        .into_iter()
        // A colour for frames with a top or edges, a title for those with
        // a top; Open only over the screen.
        .filter(|(item, _)| match item {
            BarItem::FrameColour => self.frame().has_colour(),
            BarItem::FrameTitle => self.frame().has_title(),
            _ => true,
        })
        .filter(|(item, _)| *item != BarItem::Window || !self.in_window())
        .collect();
        let mut texts = texts;
        let mut widths: Vec<f32> = texts
            .iter()
            .map(|(item, text)| {
                let t = self.text_width(font.as_ref(), text);
                match item {
                    // Room for the chevron.
                    BarItem::Format | BarItem::Frame => t + 30.0 * s,
                    BarItem::FrameColour => 24.0 * s,
                    BarItem::FrameTitle => (t + 20.0 * s).clamp(70.0 * s, 220.0 * s),
                    BarItem::Name => (t + 20.0 * s).max(160.0 * s),
                    // A pixel spare, so rounding doesn't cut it short.
                    BarItem::Size => t + 14.0 * s,
                    BarItem::Shadow => t + 20.0 * s,
                    _ => t + 22.0 * s,
                }
            })
            .collect();
        let inner =
            |widths: &[f32]| widths.iter().sum::<f32>() + GAP * s * (widths.len() - 1) as f32;
        // The window's footer is as wide as the window: when it's narrow,
        // the name gives way first, then the size goes.
        if let Some(f) = self.dock().map(|d| d.foot) {
            let room = f.width() as f32 - 2.0 * PAD * s;
            let short = MIN_NAME * s;
            let name_w = widths[0];
            if inner(&widths) - name_w + short > room
                && let Some(i) = texts.iter().position(|(item, _)| *item == BarItem::Size)
            {
                texts.remove(i);
                widths.remove(i);
            }
            widths[0] = (room - (inner(&widths) - name_w)).clamp(short, name_w);
        }
        let docked = self.dock().map(|d| d.foot);
        let one = (BAR_H * s).round() as i32;
        let lines_h =
            |n: usize| ((2.0 * PAD + n as f32 * ITEM_H + (n - 1) as f32 * GAP) * s).round() as i32;
        // One line, unless there's no room for it beside the rail and the
        // lines still fit below the canvas rather than over it.
        let mut lines = vec![0; widths.len()];
        if docked.is_none() {
            let (lo, hi) = self.panel_span(self.bar_band(one));
            let wrapped = wrap(&widths, (hi - lo) as f32 - 2.0 * PAD * s, GAP * s);
            let n = wrapped.last().map_or(1, |l| l + 1);
            if c.bottom + (MARGIN * s).round() as i32 + lines_h(n) <= self.screen().bottom {
                lines = wrapped;
            }
        }
        let n = lines.last().map_or(1, |l| l + 1);
        let widest = (0..n)
            .map(|l| {
                let ws: Vec<f32> = (0..widths.len())
                    .filter(|&i| lines[i] == l)
                    .map(|i| widths[i])
                    .collect();
                inner(&ws)
            })
            .fold(0.0, f32::max);
        let w = (widest + 2.0 * PAD * s).ceil() as i32;
        let h = lines_h(n);
        let (x, y) = match docked {
            Some(f) => (f.left, f.top + (f.height() - one) / 2),
            None => {
                let band = self.bar_band(h);
                (self.panel_x(w, band), band.0)
            }
        };
        let rect = docked.unwrap_or(IRect::from_xywh(x, y, w, h));

        let mut items = Vec::with_capacity(texts.len());
        let mut left = x as f32 + PAD * s;
        let item_h = (ITEM_H * s).round() as i32;
        for (i, ((item, text), width)) in texts.into_iter().zip(widths).enumerate() {
            if i > 0 && lines[i] != lines[i - 1] {
                left = x as f32 + PAD * s;
            }
            let top = y as f32 + (PAD + lines[i] as f32 * (ITEM_H + GAP)) * s;
            let r = IRect::from_xywh(
                left.round() as i32,
                top.round() as i32,
                width.round() as i32,
                item_h,
            );
            items.push((item, r, text));
            left += width + GAP * s;
        }
        BarLayout { rect, items }
    }

    /// The rows a bar `h` tall takes over the screen: below the canvas,
    /// else just inside its bottom.
    fn bar_band(&self, h: i32) -> (i32, i32) {
        let margin = (MARGIN * self.scale).round() as i32;
        let (c, screen) = (self.canvas, self.screen());
        let below = c.bottom + margin;
        let y = if below + h <= screen.bottom {
            below
        } else {
            c.bottom - margin - h
        };
        (y, y + h)
    }

    /// The open bar menu, over its button: the list and each row.
    pub(super) fn bar_menu_layout(&self, bar: &BarLayout) -> Option<(BarMenu, IRect, Vec<IRect>)> {
        let menu = self.bar_menu?;
        let s = self.scale;
        let (_, button, _) = bar.items.iter().find(|(i, ..)| *i == menu.button())?;
        let font = self.bar_font();
        let names = self.menu_names(menu);
        let pin = menu.pin_row();
        let w = names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let marked = menu == BarMenu::Colour || pin == Some(i);
                self.text_width(font.as_ref(), name) + if marked { ROW_MARK * s } else { 0.0 }
            })
            .fold(0.0f32, f32::max);
        let w = ((w + 28.0 * s).ceil() as i32).max(button.width());
        let row = (MENU_ROW * s).round() as i32;
        let pad = (4.0 * s).round() as i32;
        let n = names.len() as i32;
        let h = n * row + 2 * pad;
        let screen = self.screen();
        // Above the bar, or below it when there's no room.
        let gap = (6.0 * s).round() as i32;
        let top = if bar.rect.top - gap - h >= screen.top {
            bar.rect.top - gap - h
        } else {
            bar.rect.bottom + gap
        };
        let left = button.left.min(screen.right - w).max(screen.left);
        let list = IRect::from_xywh(left, top, w, h);
        let rows = (0..n)
            .map(|i| IRect::from_xywh(left, top + pad + i * row, w, row))
            .collect();
        Some((menu, list, rows))
    }

    /// Each row's name in `menu`.
    fn menu_names(&self, menu: BarMenu) -> Vec<String> {
        match menu {
            BarMenu::Frame => super::Frame::all()
                .into_iter()
                .map(|(_, n)| n)
                .chain([PIN.to_string()])
                .collect(),
            BarMenu::Colour => super::FRAME_COLOURS
                .iter()
                .map(|(n, _)| n.to_string())
                .collect(),
        }
    }

    /// The row of `menu` that's in use now.
    fn menu_current(&self, menu: BarMenu) -> usize {
        match menu {
            BarMenu::Frame => super::Frame::all()
                .iter()
                .position(|(f, _)| *f == self.frame())
                .unwrap_or(0),
            BarMenu::Colour => usize::from(self.frame_colour()),
        }
    }

    /// Pick row `i` of the open menu, and close it.
    pub(super) fn pick_menu(&mut self, i: usize) {
        match self.bar_menu {
            Some(m) if m.pin_row() == Some(i) => self.toggle_pin(),
            Some(BarMenu::Frame) => {
                if let Some((f, _)) = super::Frame::all().get(i) {
                    self.set_frame(*f);
                }
            }
            Some(BarMenu::Colour) => self.set_frame_colour(i as u8),
            None => {}
        }
        self.close_bar_menu();
    }

    /// Open `menu`, or close it if it's open.
    fn toggle_menu(&mut self, menu: BarMenu) {
        self.bar_menu = if self.bar_menu == Some(menu) {
            None
        } else {
            Some(menu)
        };
        self.chrome_key = None;
    }

    pub(super) fn close_bar_menu(&mut self) {
        if self.bar_menu.is_some() {
            self.bar_menu = None;
            self.chrome_key = None;
        }
    }

    /// The open menu's panel: each row's name (a colour's swatch first),
    /// the one in use marked, and under the frames the pin's tick box.
    pub(super) fn draw_bar_menu(
        &self,
        menu: BarMenu,
        list: IRect,
        rows: &[IRect],
        hovered: Option<usize>,
    ) -> Panel {
        let s = self.scale;
        let font = self.bar_font();
        let names = self.menu_names(menu);
        let current = self.menu_current(menu);
        let pin = menu.pin_row();
        let pinned = self.look_pinned();
        panel(list, |canvas| {
            draw_list(
                canvas,
                list,
                rows,
                Some(current),
                hovered,
                s,
                |i, rect, in_use| {
                    let mark = Point::new(rect.left + 17.0 * s, rect.center_y());
                    let is_pin = pin == Some(i);
                    if menu == BarMenu::Colour {
                        swatch(canvas, mark, 7.0 * s, self.frame_colour_shown(i as u8), s);
                    } else if is_pin {
                        let y = rect.top + 0.5 * s;
                        let line = super::theme::stroke(DIVIDER, s);
                        canvas.draw_line((rect.left, y), (rect.right, y), &line);
                        tick_box(canvas, mark, pinned, s);
                    }
                    let on = in_use || (is_pin && pinned);
                    let marked = menu == BarMenu::Colour || is_pin;
                    let indent = if marked { 10.0 + ROW_MARK } else { 10.0 };
                    let text_x = rect.left + indent * s;
                    if let (Some(font), Some(name)) = (&font, names.get(i)) {
                        let p = fill(if on { TEXT } else { MUTED });
                        let baseline = centred_baseline(font, rect.center_y());
                        canvas.draw_str(name, (text_x, baseline), font, &p);
                    }
                },
            );
        })
    }

    /// The clickable bar item under `p`.
    pub(super) fn bar_hit(&self, p: Point) -> Option<BarItem> {
        self.bar_layout()
            .items
            .into_iter()
            .find(|(item, r, _)| item.clickable() && contains(r, p))
            .map(|(item, ..)| item)
    }

    pub(super) fn on_bar(&self, p: Point) -> bool {
        let b = self.bar_layout();
        contains(&b.rect, p)
    }

    /// A click on a bar item.
    pub(super) fn press_bar(&mut self, item: BarItem) {
        match item {
            BarItem::Format => {
                self.format = self.format.next();
                self.chrome_key = None;
            }
            BarItem::Frame => self.toggle_menu(BarMenu::Frame),
            BarItem::Shadow => self.set_shadow(self.shadow() + 1),
            BarItem::FrameColour => self.toggle_menu(BarMenu::Colour),
            BarItem::FrameTitle => self.set_editing_title(!self.editing_title()),
            BarItem::Window => self.action = Some(Action::OpenWindow),
            BarItem::Copy => self.action = Some(Action::Copy),
            BarItem::Save => self.action = Some(Action::Save),
            BarItem::Share => self.action = Some(Action::Share),
            BarItem::Name | BarItem::Size => {}
        }
    }

    pub(super) fn draw_bar(&self, layout: &BarLayout, hovered: Option<BarItem>) -> Panel {
        let s = self.scale;
        let r = layout.rect;
        let font = self.bar_font();
        panel(r, |canvas| {
            rounded_panel(
                canvas,
                Rect::from_wh(r.width() as f32, r.height() as f32),
                10.0 * s,
                s,
            );
            let origin = Point::new(r.left as f32, r.top as f32);
            for (item, ir, text) in &layout.items {
                let rect = Rect::from(*ir).with_offset(-origin);
                let hover = hovered == Some(*item);
                let (bg, fg) = match item {
                    BarItem::Name => (Some(FIELD), TEXT),
                    BarItem::FrameTitle
                        if self.frame_title().is_empty() && !self.editing_title() =>
                    {
                        (Some(FIELD), MUTED)
                    }
                    BarItem::FrameTitle => (Some(FIELD), TEXT),
                    BarItem::Size => (None, MUTED),
                    // The main action stands out.
                    BarItem::Copy => (Some(if hover { ACCENT_HOVER } else { ACCENT }), TEXT),
                    _ => (Some(if hover { SELECTED } else { BUTTON }), TEXT),
                };
                if let Some(bg) = bg {
                    let p = fill(bg);
                    canvas.draw_rrect(RRect::new_rect_xy(rect, 7.0 * s, 7.0 * s), &p);
                }
                let mut text_x = rect.left + 10.0 * s;
                if *item == BarItem::Size {
                    text_x = rect.left + 6.0 * s;
                }
                if let Some(f) = &font {
                    let p = fill(fg);
                    let baseline = centred_baseline(f, rect.center_y());
                    // A long title is cut short rather than run on.
                    let room = rect.right - text_x - 6.0 * s;
                    canvas.draw_str(ellipsize(f, text, room), (text_x, baseline), f, &p);
                }
                if *item == BarItem::FrameTitle && self.editing_title() {
                    // A caret after the text while it's being typed.
                    let w = self.text_width(font.as_ref(), text);
                    let x = (rect.left + 10.0 * s + w + 1.0 * s).min(rect.right - 4.0 * s);
                    let mut p = Paint::new(TEXT, None);
                    p.set_stroke_width(1.5 * s);
                    canvas.draw_line((x, rect.top + 5.0 * s), (x, rect.bottom - 5.0 * s), &p);
                }
                if *item == BarItem::FrameColour {
                    // A dot in the frame's colour.
                    swatch(canvas, rect.center(), 6.5 * s, self.frame_top_colour(), s);
                }
                if matches!(item, BarItem::Format | BarItem::Frame) {
                    // A small chevron after the name.
                    chevron(
                        canvas,
                        Point::new(rect.right - 12.0 * s, rect.center_y()),
                        s,
                        fg,
                    );
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::placed;
    use super::*;
    use crate::Cursor;
    use crate::Format;

    fn centre(r: IRect) -> Point {
        Rect::from(r).center()
    }

    fn item(d: &Document, which: BarItem) -> IRect {
        d.bar_layout()
            .items
            .into_iter()
            .find(|(i, ..)| *i == which)
            .map(|(_, r, _)| r)
            .unwrap()
    }

    #[test]
    fn the_bar_keeps_clear_of_the_rail_and_wraps_only_without_room() {
        let (mut d, _) = placed();
        let overlaps = |a: IRect, b: IRect| {
            a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
        };
        // Room enough: one line from the canvas's left edge.
        d.set_screens(vec![IRect::from_ltrb(0, 0, 1200, 800)]);
        let b = d.bar_layout();
        assert_eq!(b.rect.left, 100);
        assert_eq!(b.rect.height(), BAR_H as i32);
        // Too little beside the rail: it wraps, clear of the rail and on
        // the screen.
        d.set_screens(vec![IRect::from_ltrb(0, 0, 560, 800)]);
        let rail = d.rail_layout().rect;
        let b = d.bar_layout();
        assert!(b.rect.height() > BAR_H as i32, "{:?}", b.rect);
        assert!(!overlaps(b.rect, rail), "{:?} over {rail:?}", b.rect);
        for (item, r, _) in &b.items {
            assert!(r.left >= b.rect.left && r.right <= b.rect.right, "{item:?}");
            assert!(r.top >= b.rect.top && r.bottom <= b.rect.bottom, "{item:?}");
            assert!(r.right <= 560, "{item:?} off the screen");
        }
    }

    #[test]
    fn the_bar_sits_below_the_canvas() {
        let (mut d, _) = placed();
        d.set_screens(vec![IRect::from_ltrb(0, 0, 1000, 800)]);
        d.set_name("Spooch 2026-10-04 at 10.30.00");
        let b = d.bar_layout();
        assert!(b.rect.top >= 200 + 8, "{:?}", b.rect);
        assert_eq!(b.items.len(), 9, "no frame colour without a window frame");
        let (_, _, name) = &b.items[0];
        assert_eq!(name, "Spooch 2026-10-04 at 10.30.00.png");
        let (_, _, frame) = &b.items[2];
        assert_eq!(frame, "No frame");
        let (_, _, shadow) = &b.items[3];
        assert_eq!(shadow, "Auto shadow");
        let (_, _, size) = &b.items[4];
        assert_eq!(size, "200 \u{00d7} 100");
        // A window frame adds its colour swatch.
        d.set_frame(super::super::Frame::builtin("terminal-modern"));
        let b = d.bar_layout();
        assert_eq!(b.items.len(), 11, "plus its colour and title");
        assert_eq!(b.items[3].0, BarItem::FrameColour);
        assert_eq!(b.items[4].0, BarItem::FrameTitle);
        // The panels: rail, style row, bar, tab.
        assert_eq!(d.chrome().len(), 4);
    }

    #[test]
    fn without_room_below_it_goes_inside_the_bottom() {
        let (mut d, _) = placed();
        d.set_screens(vec![IRect::from_ltrb(0, 0, 1000, 210)]);
        let b = d.bar_layout();
        assert!(b.rect.bottom <= 200, "{:?}", b.rect);
    }

    #[test]
    fn clicks_switch_the_format_or_ask_the_shell() {
        let (mut d, _) = placed();
        d.set_screens(vec![IRect::from_ltrb(0, 0, 1000, 800)]);
        let format = centre(item(&d, BarItem::Format));
        assert_eq!(d.cursor_at(format), Cursor::Hand);
        assert!(!d.begin_drag(format));
        assert_eq!(d.format(), Format::Jpeg);
        assert_eq!(d.take_action(), None);

        assert!(!d.begin_drag(centre(item(&d, BarItem::Copy))));
        assert_eq!(d.take_action(), Some(Action::Copy));
        assert_eq!(d.take_action(), None, "handed out once");
        d.begin_drag(centre(item(&d, BarItem::Save)));
        assert_eq!(d.take_action(), Some(Action::Save));
        // The name and size aren't buttons, but still swallow the press.
        let name = centre(item(&d, BarItem::Name));
        assert_eq!(d.cursor_at(name), Cursor::Arrow);
        assert!(!d.begin_drag(name));
        assert_eq!(d.take_action(), None);
    }

    #[test]
    fn the_frame_menu_pins_the_look_for_every_capture() {
        let (mut d, _) = placed();
        d.set_screens(vec![IRect::from_ltrb(0, 0, 1000, 800)]);
        let terminal = super::super::Frame::builtin("terminal-modern");
        d.set_frame(terminal);
        d.set_shadow(2);
        // The pin is the row after the frames.
        let pick = |d: &mut Document, row: usize| {
            d.begin_drag(centre(item(d, BarItem::Frame)));
            let (_, _, rows) = d.bar_menu_layout(&d.bar_layout()).unwrap();
            assert_eq!(rows.len(), super::super::Frame::all().len() + 1);
            d.begin_drag(centre(rows[row]));
        };
        let pin = super::super::Frame::all().len();
        pick(&mut d, pin);
        assert!(d.look_pinned());
        // A different frame lasts for this capture only; the next starts
        // with the pinned one.
        pick(&mut d, 0);
        assert!(!d.look_pinned());
        let next = d.styles().for_next_capture();
        assert_eq!((next.framing.frame, next.framing.shadow), (terminal, 2));
        // Unticking goes back to starting with the last pick.
        d.set_styles(next);
        pick(&mut d, pin);
        assert!(!d.look_pinned());
        d.set_shadow(0);
        assert_eq!(d.styles().for_next_capture().framing.shadow, 0);
    }

    #[test]
    fn hovering_a_button_shows_its_shortcut() {
        let (mut d, _) = placed();
        d.set_screens(vec![IRect::from_ltrb(0, 0, 1000, 800)]);
        let copy = centre(item(&d, BarItem::Copy));
        assert!(d.pointer_moved(copy));
        if d.typeface.is_some() {
            let panels = d.chrome();
            assert_eq!(panels.len(), 5, "rail, row, bar, tab and tooltip");
            let bar = panels[2].rect;
            assert!(panels[4].rect.bottom <= bar.top, "tooltip above the bar");
        }
    }
}
