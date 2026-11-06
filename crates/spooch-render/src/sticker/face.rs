//! Rage-comic-style faces, over the top: thick black lines on a head about
//! 100 by 110, with the tears, steam and sparkles round it.

use skia_safe::{
    BlurStyle, Canvas, Color4f, MaskFilter, Paint, PaintCap, PaintJoin, PaintStyle, Path,
    PathBuilder, PathOp, Point, RRect, Rect,
};

/// The faces there are stickers of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    Fury,
    Lol,
    Sob,
    MindBlown,
    JawDrop,
    HeartEyes,
    Cringe,
    Evil,
    Sweating,
    DeadInside,
    Bruh,
    Smug,
    Cool,
    Derp,
    Bleh,
}

type Pt = (f32, f32);

const fn rgb(r: f32, g: f32, b: f32) -> Color4f {
    Color4f::new(r, g, b, 1.0)
}

const BLACK: Color4f = rgb(0.0, 0.0, 0.0);
const WHITE: Color4f = rgb(1.0, 1.0, 1.0);
const RED: Color4f = rgb(0.88, 0.22, 0.18);
const DARK_RED: Color4f = rgb(0.5, 0.05, 0.05);
const TEARS: Color4f = rgb(0.5, 0.75, 0.98);
const TONGUE: Color4f = rgb(0.95, 0.48, 0.55);
const BLUSH: Color4f = Color4f::new(1.0, 0.5, 0.6, 0.6);
const GREEN: Color4f = rgb(0.7, 0.87, 0.45);
const PALE: Color4f = rgb(0.8, 0.84, 0.9);
const ORANGE: Color4f = rgb(1.0, 0.55, 0.1);
const YELLOW: Color4f = rgb(1.0, 0.86, 0.2);

/// The head, a lumpy oval, going round from the top.
const HEAD: [Pt; 12] = [
    (50.0, 4.0),
    (76.0, 9.0),
    (92.0, 30.0),
    (95.0, 56.0),
    (88.0, 82.0),
    (70.0, 100.0),
    (48.0, 105.0),
    (27.0, 99.0),
    (11.0, 80.0),
    (5.0, 54.0),
    (9.0, 28.0),
    (25.0, 9.0),
];

/// The head, its lower half `stretch` times as long.
fn head(stretch: f32) -> Path {
    let points: Vec<Pt> = HEAD
        .iter()
        .map(|&(x, y)| {
            (
                x,
                if y > 50.0 {
                    50.0 + (y - 50.0) * stretch
                } else {
                    y
                },
            )
        })
        .collect();
    curve(&points, true)
}

/// A smooth curve through `points`, closed back to the first if `closed`.
fn curve(points: &[Pt], closed: bool) -> Path {
    let n = points.len() as isize;
    let at = |i: isize| {
        points[if closed {
            i.rem_euclid(n)
        } else {
            i.clamp(0, n - 1)
        } as usize]
    };
    let mut b = PathBuilder::new();
    b.move_to(points[0]);
    for i in 0..if closed { n } else { n - 1 } {
        let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        b.cubic_to(
            (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0),
            (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0),
            p2,
        );
    }
    if closed {
        b.close();
    }
    b.detach()
}

fn fill(colour: Color4f) -> Paint {
    let mut p = Paint::new(colour, None);
    p.set_anti_alias(true);
    p
}

fn pen(colour: Color4f, width: f32) -> Paint {
    let mut p = fill(colour);
    p.set_style(PaintStyle::Stroke);
    p.set_stroke_width(width);
    p.set_stroke_cap(PaintCap::Round);
    p.set_stroke_join(PaintJoin::Round);
    p
}

fn ink(width: f32) -> Paint {
    pen(BLACK, width)
}

/// Fill `path` in `colour` and outline it.
fn shape(c: &Canvas, path: &Path, colour: Color4f) {
    c.draw_path(path, &fill(colour));
    c.draw_path(path, &ink(3.2));
}

fn line(c: &Canvas, points: &[Pt], width: f32) {
    c.draw_path(&curve(points, false), &ink(width));
}

/// A stream of tears along `points`.
fn stream(c: &Canvas, points: &[Pt], width: f32) {
    c.draw_path(&curve(points, false), &pen(TEARS, width));
}

/// An eye: a white circle of radius `r` at `at`, its pupil `p` across
/// from the middle.
fn eye(c: &Canvas, at: Pt, r: f32, p: Pt, pupil: f32) {
    shape(c, &Path::circle(at, r, None), WHITE);
    c.draw_circle((at.0 + p.0, at.1 + p.1), pupil, &fill(BLACK));
}

/// A half-shut eye: its lower half under a heavy lid at `y`, looking `p`
/// across.
fn lidded(c: &Canvas, x: f32, y: f32, p: f32) {
    let lower = Rect::from_ltrb(x - 10.0, y - 8.0, x + 10.0, y + 10.0);
    c.draw_arc(lower, 0.0, 180.0, false, &ink(3.0));
    c.draw_circle((x + p, y + 4.5), 4.0, &fill(BLACK));
    line(c, &[(x - 12.0, y), (x + 12.0, y)], 5.0);
}

/// A drop, its point up, its round end at `at`.
fn drop(c: &Canvas, at: Pt, r: f32) {
    let (x, y) = at;
    let mut b = PathBuilder::new();
    b.move_to((x, y - r * 2.4));
    b.cubic_to(
        (x + r * 0.4, y - r * 1.2),
        (x + r * 1.4, y - r * 0.2),
        (x + r, y + r * 0.3),
    );
    b.cubic_to(
        (x + r * 0.6, y + r * 1.3),
        (x - r * 0.6, y + r * 1.3),
        (x - r, y + r * 0.3),
    );
    b.cubic_to(
        (x - r * 1.4, y - r * 0.2),
        (x - r * 0.4, y - r * 1.2),
        (x, y - r * 2.4),
    );
    b.close();
    let drop = b.detach();
    c.draw_path(&drop, &fill(TEARS));
    c.draw_path(&drop, &pen(rgb(0.2, 0.45, 0.8), 1.5));
}

fn heart(c: &Canvas, at: Pt, s: f32, colour: Color4f) {
    let (x, y) = at;
    let mut b = PathBuilder::new();
    b.move_to((x, y + s * 0.45));
    b.cubic_to(
        (x - s * 0.9, y - s * 0.05),
        (x - s * 0.45, y - s * 0.75),
        (x, y - s * 0.25),
    );
    b.cubic_to(
        (x + s * 0.45, y - s * 0.75),
        (x + s * 0.9, y - s * 0.05),
        (x, y + s * 0.45),
    );
    b.close();
    shape(c, &b.detach(), colour);
}

/// A star of `points` spikes between radii `outer` and `inner`.
fn star(at: Pt, points: usize, outer: f32, inner: f32) -> Path {
    let corners: Vec<Point> = (0..points * 2)
        .map(|i| {
            let r = if i % 2 == 0 { outer } else { inner };
            let a = std::f32::consts::PI * i as f32 / points as f32 - std::f32::consts::FRAC_PI_2;
            Point::new(at.0 + r * a.cos(), at.1 + r * a.sin())
        })
        .collect();
    Path::polygon(&corners, true, None, None)
}

fn sparkle(c: &Canvas, at: Pt, r: f32) {
    shape(c, &star(at, 4, r, r * 0.3), YELLOW);
}

/// A cloud of steam from `circles`.
fn puff(c: &Canvas, circles: &[(Pt, f32)]) {
    let cloud = circles.iter().fold(Path::new(), |acc, &(at, r)| {
        acc.op(&Path::circle(at, r, None), PathOp::Union)
            .unwrap_or(acc)
    });
    shape(c, &cloud, WHITE);
}

/// A spiral filling the circle of radius `r` at `at`.
fn spiral(c: &Canvas, at: Pt, r: f32) {
    let turns = 2.5;
    let points: Vec<Pt> = (0..=40)
        .map(|i| {
            let t = i as f32 / 40.0;
            let a = t * turns * std::f32::consts::TAU;
            (at.0 + r * t * a.cos(), at.1 + r * t * a.sin())
        })
        .collect();
    line(c, &points, 2.5);
}

/// Draw `inside` clipped to `path`.
fn within<T>(c: &Canvas, path: &Path, inside: impl FnOnce() -> T) {
    c.save();
    c.clip_path(path, None, true);
    inside();
    c.restore();
}

/// A row of `count` teeth across `band`, clipped to the mouth; the top row
/// if `top`.
fn teeth(c: &Canvas, mouth: &Path, band: Rect, count: usize, top: bool) {
    within(c, mouth, || {
        c.draw_rect(band, &fill(WHITE));
        let edge = ink(2.0);
        let y = if top { band.bottom } else { band.top };
        c.draw_line((band.left, y), (band.right, y), &edge);
        for i in 1..count {
            let x = band.left + band.width() * i as f32 / count as f32;
            c.draw_line((x, band.top), (x, band.bottom), &edge);
        }
    });
}

/// A wide-open mouth: black, top teeth down to `teeth_to`, a tongue in
/// the oval `tongue`.
fn yell(c: &Canvas, outline: &[Pt], teeth_to: f32, tongue: Rect) {
    let mouth = curve(outline, true);
    c.draw_path(&mouth, &fill(BLACK));
    within(c, &mouth, || c.draw_oval(tongue, &fill(TONGUE)));
    teeth(
        c,
        &mouth,
        Rect::from_ltrb(0.0, 40.0, 100.0, teeth_to),
        6,
        true,
    );
    c.draw_path(&mouth, &ink(3.2));
}

/// The mirror image of `points` across the face's middle.
fn mirror(points: &[Pt]) -> Vec<Pt> {
    points.iter().map(|&(x, y)| (100.0 - x, y)).collect()
}

/// `points` and their mirror image, as two lines.
fn pair(c: &Canvas, points: &[Pt], width: f32) {
    line(c, points, width);
    line(c, &mirror(points), width);
}

impl Face {
    pub(super) fn title(self) -> &'static str {
        match self {
            Face::Fury => "Rage face",
            Face::Lol => "Laughing out loud",
            Face::Sob => "Sobbing",
            Face::MindBlown => "Mind blown",
            Face::JawDrop => "Jaw drop",
            Face::HeartEyes => "Heart eyes",
            Face::Cringe => "Cringe",
            Face::Evil => "Evil grin",
            Face::Sweating => "Nervous sweating",
            Face::DeadInside => "Dead inside",
            Face::Bruh => "Bruh",
            Face::Smug => "Smug",
            Face::Cool => "Cool shades",
            Face::Derp => "Derp",
            Face::Bleh => "Bleh",
        }
    }

    /// The box it's drawn in.
    pub(super) fn view(self) -> Rect {
        let (l, t, r, b) = match self {
            Face::Fury => (-8.0, -18.0, 108.0, 108.0),
            Face::Lol => (-20.0, -6.0, 120.0, 112.0),
            Face::Sob => (-16.0, 1.0, 116.0, 110.0),
            Face::MindBlown => (-6.0, -42.0, 106.0, 108.0),
            Face::JawDrop => (2.0, -16.0, 98.0, 150.0),
            Face::HeartEyes => (-2.0, -18.0, 102.0, 110.0),
            Face::Cringe => (-16.0, 1.0, 116.0, 108.0),
            Face::Sweating => (-2.0, -4.0, 104.0, 108.0),
            Face::DeadInside => (2.0, -40.0, 98.0, 108.0),
            Face::Smug => (-2.0, -2.0, 104.0, 108.0),
            Face::Bleh => (2.0, 1.0, 98.0, 121.0),
            _ => (2.0, 1.0, 98.0, 108.0),
        };
        Rect::from_ltrb(l, t, r, b)
    }

    pub(super) fn draw(self, c: &Canvas) {
        let skin = match self {
            Face::Fury => RED,
            Face::DeadInside => PALE,
            Face::Bleh => GREEN,
            _ => WHITE,
        };
        let face = head(if self == Face::JawDrop { 1.75 } else { 1.0 });
        // What goes behind the head.
        match self {
            Face::Fury => {
                let left = [
                    ((13.0, 6.0), 8.0),
                    ((4.0, -2.0), 9.0),
                    ((15.0, -8.0), 7.0),
                    ((5.0, -13.0), 6.5),
                ];
                puff(c, &left);
                puff(c, &left.map(|((x, y), r)| ((100.0 - x, y), r)));
            }
            Face::HeartEyes => {
                heart(c, (90.0, 2.0), 13.0, RED);
                heart(c, (11.0, -2.0), 10.0, RED);
                heart(c, (80.0, -11.0), 7.0, RED);
            }
            Face::Smug => {
                sparkle(c, (92.0, 10.0), 9.0);
                sparkle(c, (8.0, 14.0), 6.0);
                sparkle(c, (98.0, 30.0), 5.0);
            }
            Face::Cringe => {
                for (x, bow, reach) in [(-2.0, -5.0, 10.0), (-9.0, -6.0, 14.0)] {
                    pair(
                        c,
                        &[(x, 50.0 - reach), (x + bow, 50.0), (x, 50.0 + reach)],
                        3.0,
                    );
                }
            }
            _ => {}
        }
        if self == Face::Lol {
            c.save();
            c.rotate(-12.0, Some(Point::new(50.0, 55.0)));
        }
        shape(c, &face, skin);
        match self {
            Face::Fury => fury(c),
            Face::Lol => lol(c),
            Face::Sob => sob(c),
            Face::MindBlown => mind_blown(c),
            Face::JawDrop => jaw_drop(c),
            Face::HeartEyes => heart_eyes(c),
            Face::Cringe => cringe(c),
            Face::Evil => evil(c, &face),
            Face::Sweating => sweating(c),
            Face::DeadInside => dead_inside(c),
            Face::Bruh => bruh(c),
            Face::Smug => smug(c),
            Face::Cool => cool(c),
            Face::Derp => derp(c),
            Face::Bleh => bleh(c),
        }
        if self == Face::Lol {
            c.restore();
        }
    }
}

fn fury(c: &Canvas) {
    // The anger mark: four brackets bowed towards each other.
    let mark = pen(DARK_RED, 3.2);
    for (x, y, start) in [
        (44.0, 10.0, 0.0),
        (56.0, 10.0, 90.0),
        (56.0, 22.0, 180.0),
        (44.0, 22.0, 270.0),
    ] {
        c.draw_arc(
            Rect::from_xywh(x - 5.0, y - 5.0, 10.0, 10.0),
            start,
            90.0,
            false,
            &mark,
        );
    }
    eye(c, (33.0, 47.0), 7.0, (0.0, 0.0), 2.0);
    eye(c, (67.0, 47.0), 7.0, (0.0, 0.0), 2.0);
    pair(c, &[(14.0, 28.0), (46.0, 42.0)], 7.0);
    let mouth = curve(
        &[
            (16.0, 58.0),
            (50.0, 63.0),
            (84.0, 58.0),
            (76.0, 82.0),
            (64.0, 97.0),
            (36.0, 97.0),
            (24.0, 82.0),
        ],
        true,
    );
    c.draw_path(&mouth, &fill(BLACK));
    within(c, &mouth, || {
        c.draw_oval(Rect::from_ltrb(30.0, 80.0, 70.0, 110.0), &fill(DARK_RED))
    });
    teeth(c, &mouth, Rect::from_ltrb(10.0, 50.0, 90.0, 68.0), 6, true);
    teeth(
        c,
        &mouth,
        Rect::from_ltrb(10.0, 89.0, 90.0, 100.0),
        5,
        false,
    );
    c.draw_path(&mouth, &ink(3.2));
    for (at, r) in [
        ((7.0, 64.0), 2.6),
        ((93.0, 62.0), 2.6),
        ((3.0, 80.0), 2.0),
        ((97.0, 77.0), 2.0),
    ] {
        drop(c, at, r);
    }
}

fn lol(c: &Canvas) {
    pair(c, &[(18.0, 24.0), (30.0, 14.0), (42.0, 22.0)], 4.0);
    pair(c, &[(22.0, 31.0), (38.0, 38.0), (22.0, 45.0)], 5.0);
    stream(c, &[(20.0, 40.0), (6.0, 34.0), (-12.0, 37.0)], 7.0);
    stream(c, &[(80.0, 40.0), (94.0, 34.0), (112.0, 37.0)], 7.0);
    drop(c, (-15.0, 47.0), 3.0);
    drop(c, (115.0, 47.0), 3.0);
    yell(
        c,
        &[
            (14.0, 52.0),
            (50.0, 56.0),
            (86.0, 52.0),
            (80.0, 78.0),
            (64.0, 98.0),
            (36.0, 98.0),
            (20.0, 78.0),
        ],
        62.0,
        Rect::from_ltrb(26.0, 78.0, 74.0, 112.0),
    );
}

fn sob(c: &Canvas) {
    let left = [(22.0, 46.0), (4.0, 40.0), (-8.0, 60.0), (-10.0, 96.0)];
    stream(c, &left, 8.0);
    stream(c, &mirror(&left), 8.0);
    drop(c, (-10.0, 104.0), 4.0);
    drop(c, (110.0, 104.0), 4.0);
    pair(c, &[(14.0, 34.0), (28.0, 30.0), (42.0, 20.0)], 5.0);
    pair(c, &[(21.0, 42.0), (32.0, 48.0), (43.0, 42.0)], 5.0);
    yell(
        c,
        &[
            (22.0, 66.0),
            (36.0, 61.0),
            (50.0, 66.0),
            (64.0, 61.0),
            (78.0, 66.0),
            (72.0, 88.0),
            (58.0, 96.0),
            (42.0, 96.0),
            (28.0, 88.0),
        ],
        70.0,
        Rect::from_ltrb(32.0, 84.0, 68.0, 108.0),
    );
    // A snot bubble.
    c.draw_circle((55.0, 56.0), 7.0, &fill(Color4f::new(0.75, 0.92, 0.7, 0.6)));
    c.draw_circle((55.0, 56.0), 7.0, &ink(1.8));
    c.draw_circle((52.5, 53.5), 2.0, &fill(WHITE));
}

fn mind_blown(c: &Canvas) {
    shape(c, &star((50.0, -2.0), 14, 42.0, 22.0), ORANGE);
    shape(c, &star((50.0, -2.0), 12, 26.0, 13.0), YELLOW);
    for x in [31.0, 69.0] {
        shape(c, &Path::circle((x, 50.0), 13.0, None), WHITE);
        spiral(c, (x, 50.0), 11.0);
    }
    shape(
        c,
        &Path::oval(Rect::from_ltrb(37.0, 72.0, 63.0, 100.0), None),
        BLACK,
    );
}

fn jaw_drop(c: &Canvas) {
    pair(c, &[(14.0, -6.0), (28.0, -12.0), (42.0, -6.0)], 4.0);
    // The eyes pop out past the head.
    eye(c, (25.0, 38.0), 20.0, (0.0, 0.0), 2.5);
    eye(c, (75.0, 38.0), 20.0, (0.0, 0.0), 2.5);
    let mouth = Path::oval(Rect::from_ltrb(36.0, 64.0, 64.0, 138.0), None);
    c.draw_path(&mouth, &fill(BLACK));
    within(c, &mouth, || {
        c.draw_oval(Rect::from_ltrb(38.0, 118.0, 62.0, 150.0), &fill(TONGUE))
    });
    c.draw_path(&mouth, &ink(3.2));
}

fn heart_eyes(c: &Canvas) {
    c.draw_oval(Rect::from_ltrb(12.0, 58.0, 30.0, 68.0), &fill(BLUSH));
    c.draw_oval(Rect::from_ltrb(70.0, 58.0, 88.0, 68.0), &fill(BLUSH));
    for x in [32.0, 68.0] {
        heart(c, (x, 45.0), 26.0, RED);
        c.draw_circle((x - 6.0, 39.0), 2.5, &fill(WHITE));
    }
    yell(
        c,
        &[
            (26.0, 66.0),
            (50.0, 68.0),
            (74.0, 66.0),
            (66.0, 86.0),
            (50.0, 92.0),
            (34.0, 86.0),
        ],
        72.0,
        Rect::from_ltrb(36.0, 80.0, 64.0, 100.0),
    );
    stream(c, &[(68.0, 82.0), (70.0, 94.0), (70.0, 102.0)], 4.0);
    drop(c, (70.0, 107.0), 3.0);
}

fn cringe(c: &Canvas) {
    pair(c, &[(18.0, 30.0), (42.0, 36.0)], 5.0);
    pair(c, &[(24.0, 40.0), (38.0, 46.0), (24.0, 52.0)], 5.0);
    let mouth = Path::rrect(
        RRect::new_rect_xy(Rect::from_ltrb(20.0, 64.0, 80.0, 86.0), 8.0, 8.0),
        None,
    );
    within(c, &mouth, || {
        c.draw_rect(Rect::from_ltrb(0.0, 0.0, 100.0, 100.0), &fill(WHITE));
        let edge = ink(2.0);
        c.draw_line((20.0, 75.0), (80.0, 75.0), &edge);
        for i in 1..8 {
            let x = 20.0 + 60.0 * i as f32 / 8.0;
            c.draw_line((x, 64.0), (x, 86.0), &edge);
        }
    });
    c.draw_path(&mouth, &ink(3.5));
    drop(c, (86.0, 26.0), 5.0);
    drop(c, (14.0, 24.0), 4.0);
    drop(c, (92.0, 48.0), 4.0);
}

fn evil(c: &Canvas, face: &Path) {
    within(c, face, || {
        let shade = curve(
            &[
                (-5.0, -5.0),
                (105.0, -5.0),
                (105.0, 46.0),
                (50.0, 58.0),
                (-5.0, 46.0),
            ],
            true,
        );
        c.draw_path(&shade, &fill(Color4f::new(0.2, 0.14, 0.3, 0.92)));
    });
    c.draw_path(face, &ink(3.2));
    let left = [(17.0, 34.0), (45.0, 45.0), (40.0, 49.0), (22.0, 45.0)];
    let mut glow = fill(Color4f::new(1.0, 0.15, 0.1, 0.9));
    glow.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, 4.0, None));
    for points in [left.to_vec(), mirror(&left)] {
        let slit = curve(&points, true);
        c.draw_path(&slit, &glow);
        c.draw_path(&slit, &fill(rgb(1.0, 0.95, 0.5)));
    }
    let lip = |x: f32| 56.0 + 14.0 * (1.0 - ((x - 50.0) / 38.0).powi(2));
    let chin = |x: f32| 92.0 - 30.0 * ((x - 50.0) / 38.0).powi(2);
    let mouth = curve(
        &[
            (12.0, 56.0),
            (30.0, 66.0),
            (50.0, 70.0),
            (70.0, 66.0),
            (88.0, 56.0),
            (76.0, 80.0),
            (50.0, 92.0),
            (24.0, 80.0),
        ],
        true,
    );
    c.draw_path(&mouth, &fill(BLACK));
    within(c, &mouth, || {
        let w = 76.0 / 9.0;
        for i in 0..9 {
            let x = 12.0 + i as f32 * w;
            let m = x + w / 2.0;
            for (base, tip) in [(40.0, lip(m) + 10.0), (100.0, chin(m) - 9.0)] {
                let fang = Path::polygon(
                    &[
                        Point::new(x, base),
                        Point::new(x + w, base),
                        Point::new(m, tip),
                    ],
                    true,
                    None,
                    None,
                );
                c.draw_path(&fang, &fill(WHITE));
                c.draw_path(&fang, &ink(1.5));
            }
        }
    });
    c.draw_path(&mouth, &ink(3.2));
}

fn sweating(c: &Canvas) {
    pair(c, &[(20.0, 32.0), (30.0, 28.0), (42.0, 22.0)], 4.0);
    eye(c, (32.0, 44.0), 12.0, (7.5, 1.0), 2.5);
    eye(c, (68.0, 44.0), 12.0, (7.5, 1.0), 2.5);
    line(
        c,
        &[
            (24.0, 72.0),
            (30.0, 76.0),
            (36.0, 72.0),
            (42.0, 76.0),
            (48.0, 72.0),
            (54.0, 76.0),
            (60.0, 72.0),
            (66.0, 76.0),
            (72.0, 72.0),
            (78.0, 67.0),
        ],
        3.5,
    );
    for (at, r) in [
        ((86.0, 22.0), 6.0),
        ((12.0, 30.0), 5.0),
        ((95.0, 50.0), 5.0),
        ((20.0, 64.0), 4.0),
        ((68.0, 10.0), 4.0),
        ((92.0, 74.0), 4.0),
    ] {
        drop(c, at, r);
    }
}

fn dead_inside(c: &Canvas) {
    let bags = Color4f::new(0.42, 0.36, 0.52, 0.55);
    c.draw_oval(Rect::from_ltrb(18.0, 35.0, 46.0, 60.0), &fill(bags));
    c.draw_oval(Rect::from_ltrb(54.0, 35.0, 82.0, 60.0), &fill(bags));
    eye(c, (32.0, 46.0), 8.0, (0.0, 0.0), 1.4);
    eye(c, (68.0, 46.0), 8.0, (0.0, 0.0), 1.4);
    pair(c, &[(22.0, 60.0), (32.0, 63.0), (42.0, 60.0)], 2.0);
    line(c, &[(40.0, 80.0), (60.0, 81.0)], 3.0);
    // The soul, leaving.
    let soul = curve(
        &[
            (46.0, 6.0),
            (36.0, -6.0),
            (36.0, -24.0),
            (48.0, -36.0),
            (62.0, -32.0),
            (66.0, -18.0),
            (60.0, -6.0),
            (54.0, 6.0),
        ],
        true,
    );
    c.draw_path(&soul, &fill(Color4f::new(1.0, 1.0, 1.0, 0.8)));
    c.draw_path(&soul, &pen(Color4f::new(0.3, 0.35, 0.45, 0.8), 1.6));
    c.draw_circle((45.0, -22.0), 2.2, &fill(BLACK));
    c.draw_circle((56.0, -22.0), 2.2, &fill(BLACK));
    c.draw_oval(Rect::from_ltrb(48.0, -16.0, 53.0, -10.0), &ink(1.8));
}

fn bruh(c: &Canvas) {
    line(c, &[(18.0, 26.0), (30.0, 14.0), (44.0, 22.0)], 4.0);
    line(c, &[(56.0, 36.0), (80.0, 36.0)], 5.0);
    lidded(c, 32.0, 40.0, 0.0);
    lidded(c, 68.0, 43.0, 0.0);
    line(c, &[(50.0, 50.0), (46.0, 62.0), (53.0, 63.0)], 3.0);
    line(c, &[(44.0, 80.0), (60.0, 80.0), (74.0, 77.0)], 3.5);
}

fn smug(c: &Canvas) {
    pair(c, &[(16.0, 26.0), (28.0, 12.0), (44.0, 20.0)], 5.0);
    lidded(c, 32.0, 42.0, 4.0);
    lidded(c, 68.0, 42.0, 4.0);
    line(
        c,
        &[(28.0, 72.0), (50.0, 77.0), (70.0, 73.0), (84.0, 58.0)],
        4.0,
    );
    line(c, &[(40.0, 95.0), (50.0, 98.0), (60.0, 95.0)], 3.0);
}

fn cool(c: &Canvas) {
    let black = fill(BLACK);
    for (l, t, r, b) in [
        (12.0, 34.0, 88.0, 38.0),
        (16.0, 38.0, 44.0, 42.0),
        (20.0, 42.0, 40.0, 46.0),
        (56.0, 38.0, 84.0, 42.0),
        (60.0, 42.0, 80.0, 46.0),
    ] {
        c.draw_rect(Rect::from_ltrb(l, t, r, b), &black);
    }
    for x in [20.0, 60.0] {
        c.draw_rect(Rect::from_xywh(x, 38.0, 4.0, 4.0), &fill(WHITE));
        c.draw_rect(Rect::from_xywh(x + 4.0, 42.0, 4.0, 4.0), &fill(WHITE));
    }
    line(c, &[(36.0, 76.0), (52.0, 78.0), (67.0, 71.0)], 3.5);
}

fn derp(c: &Canvas) {
    line(c, &[(19.0, 22.0), (43.0, 21.0)], 3.5);
    line(c, &[(58.0, 33.0), (77.0, 30.0)], 3.5);
    eye(c, (32.0, 39.0), 13.0, (-5.0, -5.0), 4.5);
    eye(c, (68.0, 45.0), 8.0, (3.0, 3.0), 3.2);
    let mouth = curve(
        &[
            (24.0, 66.0),
            (52.0, 70.0),
            (78.0, 60.0),
            (70.0, 77.0),
            (42.0, 86.0),
        ],
        true,
    );
    shape(c, &mouth, BLACK);
    teeth(c, &mouth, Rect::from_ltrb(20.0, 55.0, 85.0, 70.0), 5, true);
    c.draw_path(&mouth, &ink(3.2));
    shape(
        c,
        &curve(
            &[
                (50.0, 78.0),
                (66.0, 72.0),
                (68.0, 90.0),
                (58.0, 97.0),
                (50.0, 90.0),
            ],
            true,
        ),
        TONGUE,
    );
    line(c, &[(59.0, 78.0), (59.0, 89.0)], 2.0);
}

fn bleh(c: &Canvas) {
    line(
        c,
        &[(20.0, 30.0), (26.0, 26.0), (32.0, 30.0), (38.0, 26.0)],
        3.0,
    );
    line(
        c,
        &[(62.0, 26.0), (68.0, 30.0), (74.0, 26.0), (80.0, 30.0)],
        3.0,
    );
    eye(c, (32.0, 43.0), 10.0, (0.0, -6.0), 3.5);
    line(c, &[(61.0, 36.0), (75.0, 50.0)], 4.0);
    line(c, &[(75.0, 36.0), (61.0, 50.0)], 4.0);
    shape(
        c,
        &curve(
            &[
                (34.0, 70.0),
                (50.0, 66.0),
                (66.0, 70.0),
                (62.0, 82.0),
                (38.0, 82.0),
            ],
            true,
        ),
        BLACK,
    );
    let tongue = [
        (40.0, 76.0),
        (60.0, 76.0),
        (62.0, 92.0),
        (56.0, 104.0),
        (60.0, 114.0),
        (48.0, 118.0),
        (42.0, 108.0),
        (44.0, 94.0),
    ];
    shape(c, &curve(&tongue, true), TONGUE);
    line(c, &[(51.0, 80.0), (50.0, 98.0), (52.0, 110.0)], 2.0);
}
