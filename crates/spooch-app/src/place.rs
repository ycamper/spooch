//! Where things go on screen: the editor window, the countdown badge.

use spooch_render::skia_safe::IRect;

/// The smallest the editor window goes, in pixels at 100%.
pub const EDITOR_MIN: (i32, i32) = (560, 380);

/// Where a new editor window goes on a monitor whose work area is `work`:
/// three quarters of it, centred, and never below `EDITOR_MIN`.
pub fn editor_rect(work: IRect) -> IRect {
    let w = (work.width() * 3 / 4).max(EDITOR_MIN.0);
    let h = (work.height() * 3 / 4).max(EDITOR_MIN.1);
    centred(work, w, h)
}

/// A `w` by `h` rectangle in the middle of `area`.
pub fn centred(area: IRect, w: i32, h: i32) -> IRect {
    IRect::from_xywh(
        area.left + (area.width() - w) / 2,
        area.top + (area.height() - h) / 2,
        w,
        h,
    )
}

/// The monitor `p` is on, if it's on one.
pub fn monitor_at(monitors: &[IRect], (x, y): (i32, i32)) -> Option<IRect> {
    monitors
        .iter()
        .copied()
        .find(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_editor_window_goes_in_the_middle_and_never_too_small() {
        let r = editor_rect(IRect::from_xywh(-2560, 717, 2560, 1400));
        assert_eq!((r.width(), r.height()), (1920, 1050));
        assert_eq!((r.left, r.top), (-2560 + 320, 717 + 175));
        let tiny = editor_rect(IRect::from_wh(600, 400));
        assert_eq!((tiny.width(), tiny.height()), EDITOR_MIN);
    }

    #[test]
    fn centres_and_finds_monitors() {
        let a = IRect::from_xywh(-1920, 200, 1920, 1080);
        let b = IRect::from_wh(2560, 1440);
        assert_eq!(centred(b, 100, 40), IRect::from_xywh(1230, 700, 100, 40));
        assert_eq!(monitor_at(&[a, b], (-5, 300)), Some(a));
        assert_eq!(monitor_at(&[a, b], (2560, 0)), None);
    }
}
