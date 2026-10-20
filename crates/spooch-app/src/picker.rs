//! Picking the area to capture: the step between the hotkey and the
//! editor, over the frozen screen. Drag an area, or click (or press Enter)
//! for the whole monitor under the pointer; arrows nudge the pointer a
//! pixel at a time; Esc cancels. The shells show the frozen screen and read
//! its pixels; the rules are here, once.

use spooch_platform::recents::Recents;
use spooch_platform::save::LocalTime;
use spooch_render::skia_safe::{IRect, Point};
use spooch_render::{Document, Format, Key, Panel, Styles};

use crate::{Host, Session, Sharing};

/// A drag must cover at least this many pixels both ways to be an area;
/// anything less is a click, which takes the whole monitor.
const MIN_DRAG_PX: i32 = 4;

/// What picking ended in, or asks of the shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// Edit this area, in desktop pixels: read its pixels and call
    /// `Picker::edit`.
    Edit(IRect),
    /// Move the pointer here; the motion it makes comes back as `motion`.
    WarpPointer((i32, i32)),
    Cancel,
}

/// What a new capture starts with, besides its pixels.
pub struct Setup {
    pub styles: Styles,
    pub format: Format,
    pub recents: Option<Recents>,
    pub sharing: Sharing,
    pub sync: crate::Syncing,
    /// When the capture was made: its file name and its id in Recents,
    /// from the one reading of the clock.
    pub when: LocalTime,
}

/// The area being picked over the frozen screen.
#[derive(Clone, Debug)]
pub struct Picker {
    /// Where the drag started, while one is going.
    anchor: Option<(i32, i32)>,
    pointer: (i32, i32),
    /// Each monitor in desktop pixels, and its display scale.
    monitors: Vec<(IRect, f32)>,
    desktop: IRect,
}

impl Picker {
    /// Picking starts with the pointer at `pointer`, over `monitors`
    /// (each with its display scale).
    pub fn new(pointer: (i32, i32), monitors: Vec<(IRect, f32)>) -> Picker {
        let desktop = monitors
            .iter()
            .map(|(r, _)| *r)
            .reduce(|a, b| {
                IRect::from_ltrb(
                    a.left.min(b.left),
                    a.top.min(b.top),
                    a.right.max(b.right),
                    a.bottom.max(b.bottom),
                )
            })
            .unwrap_or(IRect::from_wh(1, 1));
        Picker {
            anchor: None,
            pointer,
            monitors,
            desktop,
        }
    }

    pub fn pointer(&self) -> (i32, i32) {
        self.pointer
    }

    /// The monitors, as the editor's chrome wants them.
    pub fn screens(&self) -> Vec<IRect> {
        self.monitors.iter().map(|(r, _)| *r).collect()
    }

    /// The area dragged out so far, to show.
    pub fn selection(&self) -> Option<IRect> {
        self.anchor.map(|a| rect_between(a, self.pointer))
    }

    /// The monitor `p` is on, else the first.
    fn monitor_at(&self, p: (i32, i32)) -> IRect {
        self.monitor_and_scale(p).0
    }

    fn monitor_and_scale(&self, (x, y): (i32, i32)) -> (IRect, f32) {
        self.monitors
            .iter()
            .copied()
            .find(|(r, _)| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
            .or_else(|| self.monitors.first().copied())
            .unwrap_or((self.desktop, 1.0))
    }

    /// The button went down at `p`: a drag starts.
    pub fn press(&mut self, p: (i32, i32)) {
        self.pointer = p;
        self.anchor = Some(p);
    }

    /// The pointer moved to `p`.
    pub fn motion(&mut self, p: (i32, i32)) {
        self.pointer = p;
    }

    /// The button came up at `p`: the area dragged out, or for a click the
    /// whole monitor under the pointer.
    pub fn release(&mut self, p: (i32, i32)) -> Option<Pick> {
        self.pointer = p;
        let a = self.anchor.take()?;
        let dragged = (p.0 - a.0).abs() >= MIN_DRAG_PX && (p.1 - a.1).abs() >= MIN_DRAG_PX;
        let area = if dragged {
            IRect::intersect(&rect_between(a, p), &self.desktop)
        } else {
            None
        };
        Some(Pick::Edit(area.unwrap_or_else(|| self.monitor_at(p))))
    }

    /// A key while picking: arrows nudge the pointer a pixel (ten with
    /// Shift) for an exact corner, Enter takes the monitor under the
    /// pointer (not mid-drag) and Esc cancels.
    pub fn key(&mut self, key: Key, shift: bool) -> Option<Pick> {
        if let Some(to) = spooch_render::nudge_pointer(key, shift, self.pointer, self.desktop) {
            return Some(Pick::WarpPointer(to));
        }
        match key {
            Key::Escape => Some(Pick::Cancel),
            Key::Enter if self.anchor.is_none() => Some(Pick::Edit(self.monitor_at(self.pointer))),
            _ => None,
        }
    }

    /// The loupe beside the pointer, from the frozen pixels `read` gives
    /// for a desktop rectangle.
    pub fn loupe(&self, read: impl FnOnce(IRect) -> Option<Vec<u8>>) -> Option<Panel> {
        let (screen, scale) = self.monitor_and_scale(self.pointer);
        spooch_render::selecting_loupe(self.pointer, self.anchor, self.desktop, screen, scale, read)
    }

    /// Start the editor on `area`, whose frozen pixels are `pixels`
    /// (width, height, row stride, BGRA). Marks are sized for the densest
    /// monitor the area touches.
    pub fn edit(
        &self,
        area: IRect,
        (width, height, stride, data): (u32, u32, usize, &[u8]),
        setup: Setup,
    ) -> Option<Session> {
        let scale = self
            .monitors
            .iter()
            .filter(|(r, _)| IRect::intersects(r, &area))
            .map(|(_, s)| *s)
            .fold(1.0, f32::max);
        let mut doc = Document::from_bgra(width, height, stride, data, scale)?;
        doc.place(area.left, area.top, self.desktop);
        doc.set_format(setup.format);
        doc.set_styles(setup.styles);
        // Arrows nudge the corner the drag let go on.
        doc.pointer_moved(Point::new(self.pointer.0 as f32, self.pointer.1 as f32));
        Some(
            Session::new(
                doc,
                spooch_platform::save::file_stem(&setup.when),
                setup.recents,
                &setup.when,
                self.screens(),
                Host::Overlay,
            )
            .with_sharing(setup.sharing)
            .with_sync(setup.sync),
        )
    }
}

/// The rectangle with corners `a` and `b`.
fn rect_between(a: (i32, i32), b: (i32, i32)) -> IRect {
    IRect::from_ltrb(a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two monitors, the second to the left and lower down.
    fn picker() -> Picker {
        Picker::new(
            (100, 100),
            vec![
                (IRect::from_xywh(0, 0, 2560, 1440), 1.0),
                (IRect::from_xywh(-2560, 717, 2560, 1440), 1.5),
            ],
        )
    }

    #[test]
    fn a_drag_is_an_area_and_a_click_is_the_monitor() {
        let mut p = picker();
        p.press((300, 200));
        p.motion((100, 100));
        assert_eq!(p.selection(), Some(IRect::from_ltrb(100, 100, 300, 200)));
        assert_eq!(
            p.release((100, 100)),
            Some(Pick::Edit(IRect::from_ltrb(100, 100, 300, 200)))
        );
        // Too thin either way is a click: the monitor under the pointer.
        p.press((-500, 1000));
        assert_eq!(
            p.release((-300, 1002)),
            Some(Pick::Edit(IRect::from_xywh(-2560, 717, 2560, 1440)))
        );
        // A release with no press does nothing.
        assert_eq!(p.release((5, 5)), None);
    }

    #[test]
    fn keys_nudge_take_the_monitor_or_cancel() {
        let mut p = picker();
        assert_eq!(p.key(Key::Left, true), Some(Pick::WarpPointer((90, 100))));
        assert_eq!(
            p.key(Key::Enter, false),
            Some(Pick::Edit(IRect::from_xywh(0, 0, 2560, 1440)))
        );
        // Not mid-drag.
        p.press((10, 10));
        assert_eq!(p.key(Key::Enter, false), None);
        assert_eq!(p.key(Key::Escape, false), Some(Pick::Cancel));
    }

    #[test]
    fn editing_starts_a_session_sized_for_the_densest_monitor() {
        let p = picker();
        let area = IRect::from_ltrb(-10, 800, 10, 820);
        let pixels = vec![200u8; 20 * 20 * 4];
        let setup = Setup {
            styles: Styles::default(),
            format: Format::Jpeg,
            recents: None,
            sharing: Sharing::default(),
            sync: crate::Syncing::default(),
            when: LocalTime {
                year: 2026,
                month: 10,
                day: 4,
                hour: 12,
                minute: 0,
                second: 0,
            },
        };
        let s = p.edit(area, (20, 20, 80, &pixels), setup).unwrap();
        assert_eq!(s.doc.canvas(), area);
        assert_eq!(s.doc.format(), Format::Jpeg);
        assert_eq!(s.host(), Host::Overlay);
        assert_eq!(s.stem(), "Spooch 2026-10-04 at 12.00.00");
    }
}
