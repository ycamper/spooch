//! Rage-comic-style faces: thick black lines on a white (or red) head,
//! drawn in a 100 by 110 box.

use skia_safe::{Canvas, Color4f, Paint, PaintCap, PaintJoin, PaintStyle, Path, PathBuilder, Rect};

/// The faces there are stickers of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    Scream,
    Smug,
    Sad,
    Blank,
    Shocked,
    Derp,
    Laugh,
    Disgust,
}

const BLACK: Color4f = Color4f::new(0.0, 0.0, 0.0, 1.0);
const WHITE: Color4f = Color4f::new(1.0, 1.0, 1.0, 1.0);
const RED: Color4f = Color4f::new(0.88, 0.22, 0.18, 1.0);
const TEARS: Color4f = Color4f::new(0.5, 0.75, 0.98, 1.0);
const TONGUE: Color4f = Color4f::new(0.95, 0.48, 0.55, 1.0);

/// The head, a lumpy oval, going round from the top.
const HEAD: [(f32, f32); 12] = [
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

/// A smooth curve through `points`, closed back to the first if `closed`.
fn curve(points: &[(f32, f32)], closed: bool) -> Path {
    let n = points.len();
    let at = |i: isize| {
        let i = if closed {
            i.rem_euclid(n as isize)
        } else {
            i.clamp(0, n as isize - 1)
        };
        points[i as usize]
    };
    let mut b = PathBuilder::new();
    b.move_to(points[0]);
    let segments = if closed { n } else { n - 1 };
    for i in 0..segments as isize {
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

fn ink(width: f32) -> Paint {
    let mut p = fill(BLACK);
    p.set_style(PaintStyle::Stroke);
    p.set_stroke_width(width);
    p.set_stroke_cap(PaintCap::Round);
    p.set_stroke_join(PaintJoin::Round);
    p
}

/// Fill `path` in `colour` and outline it.
fn shape(canvas: &Canvas, path: &Path, colour: Color4f) {
    canvas.draw_path(path, &fill(colour));
    canvas.draw_path(path, &ink(3.2));
}

fn line(canvas: &Canvas, points: &[(f32, f32)], width: f32) {
    canvas.draw_path(&curve(points, false), &ink(width));
}

/// An eye: a white circle of radius `r` at `c`, its pupil `p` across
/// from the middle.
fn eye(canvas: &Canvas, c: (f32, f32), r: f32, p: (f32, f32), pupil: f32) {
    shape(canvas, &Path::circle(c, r, None), WHITE);
    canvas.draw_circle((c.0 + p.0, c.1 + p.1), pupil, &fill(BLACK));
}

/// A drop, its point up, its round end at `c`.
fn drop(canvas: &Canvas, c: (f32, f32), r: f32) {
    let mut b = PathBuilder::new();
    b.move_to((c.0, c.1 - r * 2.4));
    b.cubic_to(
        (c.0 + r * 0.4, c.1 - r * 1.2),
        (c.0 + r * 1.4, c.1 - r * 0.2),
        (c.0 + r, c.1 + r * 0.3),
    );
    b.cubic_to(
        (c.0 + r * 0.6, c.1 + r * 1.3),
        (c.0 - r * 0.6, c.1 + r * 1.3),
        (c.0 - r, c.1 + r * 0.3),
    );
    b.cubic_to(
        (c.0 - r * 1.4, c.1 - r * 0.2),
        (c.0 - r * 0.4, c.1 - r * 1.2),
        (c.0, c.1 - r * 2.4),
    );
    b.close();
    canvas.draw_path(&b.detach(), &fill(TEARS));
}

/// Draw `inside` clipped to `path`.
fn within(canvas: &Canvas, path: &Path, inside: impl FnOnce()) {
    canvas.save();
    canvas.clip_path(path, None, true);
    inside();
    canvas.restore();
}

/// A row of teeth across `band`, clipped to the mouth.
fn teeth(canvas: &Canvas, mouth: &Path, band: Rect, count: usize) {
    within(canvas, mouth, || {
        canvas.draw_rect(band, &fill(WHITE));
        let edge = ink(2.0);
        let y = if band.top < 70.0 {
            band.bottom
        } else {
            band.top
        };
        canvas.draw_line((band.left, y), (band.right, y), &edge);
        for i in 1..count {
            let x = band.left + band.width() * i as f32 / count as f32;
            canvas.draw_line((x, band.top), (x, band.bottom), &edge);
        }
    });
}

impl Face {
    pub(super) fn title(self) -> &'static str {
        match self {
            Face::Scream => "Rage face",
            Face::Smug => "Smug face",
            Face::Sad => "Crying face",
            Face::Blank => "Blank face",
            Face::Shocked => "Shocked face",
            Face::Derp => "Derp face",
            Face::Laugh => "Laughing face",
            Face::Disgust => "Disgusted face",
        }
    }

    /// The box it's drawn in.
    pub(super) fn view(self) -> Rect {
        Rect::from_ltrb(2.0, 1.0, 98.0, 108.0)
    }

    pub(super) fn draw(self, canvas: &Canvas) {
        let head = curve(&HEAD, true);
        shape(
            canvas,
            &head,
            if self == Face::Scream { RED } else { WHITE },
        );
        let c = canvas;
        match self {
            Face::Scream => {
                line(c, &[(20.0, 28.0), (44.0, 40.0)], 6.0);
                line(c, &[(80.0, 28.0), (56.0, 40.0)], 6.0);
                line(c, &[(27.0, 44.0), (42.0, 48.0)], 4.0);
                line(c, &[(73.0, 44.0), (58.0, 48.0)], 4.0);
                // The anger mark: four brackets bowed towards each other.
                let mut mark = ink(3.0);
                mark.set_color4f(Color4f::new(0.45, 0.05, 0.05, 1.0), None);
                for (cx, cy, start) in [
                    (70.0, 12.0, 0.0),
                    (82.0, 12.0, 90.0),
                    (82.0, 24.0, 180.0),
                    (70.0, 24.0, 270.0),
                ] {
                    c.draw_arc(
                        Rect::from_xywh(cx - 5.0, cy - 5.0, 10.0, 10.0),
                        start,
                        90.0,
                        false,
                        &mark,
                    );
                }
                let mouth = curve(
                    &[
                        (20.0, 58.0),
                        (50.0, 62.0),
                        (80.0, 58.0),
                        (66.0, 92.0),
                        (34.0, 92.0),
                    ],
                    true,
                );
                c.draw_path(&mouth, &fill(BLACK));
                teeth(c, &mouth, Rect::from_ltrb(15.0, 50.0, 85.0, 68.0), 6);
                teeth(c, &mouth, Rect::from_ltrb(15.0, 84.0, 85.0, 97.0), 4);
                c.draw_path(&mouth, &ink(3.2));
            }
            Face::Smug => {
                line(c, &[(20.0, 32.0), (31.0, 21.0), (44.0, 27.0)], 4.0);
                line(c, &[(57.0, 31.0), (69.0, 28.0), (81.0, 31.0)], 4.0);
                for cx in [32.0, 68.0] {
                    let cy = 44.0;
                    let lower = Rect::from_ltrb(cx - 10.0, cy - 6.0, cx + 10.0, cy + 7.0);
                    c.draw_arc(lower, 0.0, 180.0, false, &ink(3.0));
                    c.draw_circle((cx + 4.0, cy + 2.5), 3.5, &fill(BLACK));
                    line(c, &[(cx - 11.0, cy), (cx + 11.0, cy - 1.0)], 4.0);
                }
                line(
                    c,
                    &[(26.0, 70.0), (50.0, 76.0), (72.0, 70.0), (82.0, 59.0)],
                    3.5,
                );
            }
            Face::Sad => {
                within(c, &head, || {
                    for x in [28.0, 72.0] {
                        let stream = [
                            (x - 3.0, 50.0),
                            (x + 3.0, 50.0),
                            (x + 4.0, 110.0),
                            (x - 4.0, 110.0),
                        ];
                        c.draw_path(&curve(&stream, true), &fill(TEARS));
                    }
                });
                c.draw_path(&head, &ink(3.2));
                line(c, &[(21.0, 34.0), (31.0, 31.0), (41.0, 25.0)], 4.0);
                line(c, &[(59.0, 25.0), (69.0, 31.0), (79.0, 34.0)], 4.0);
                for cx in [32.0, 68.0] {
                    eye(c, (cx, 44.0), 10.0, (0.0, 2.0), 5.0);
                    c.draw_circle((cx - 2.0, 43.0), 1.8, &fill(WHITE));
                }
                line(
                    c,
                    &[
                        (34.0, 84.0),
                        (40.0, 77.0),
                        (45.0, 79.0),
                        (50.0, 75.0),
                        (55.0, 79.0),
                        (60.0, 77.0),
                        (66.0, 84.0),
                    ],
                    3.5,
                );
            }
            Face::Blank => {
                line(c, &[(26.0, 34.0), (41.0, 34.0)], 3.5);
                line(c, &[(59.0, 34.0), (74.0, 34.0)], 3.5);
                c.draw_circle((34.0, 45.0), 3.8, &fill(BLACK));
                c.draw_circle((66.0, 45.0), 3.8, &fill(BLACK));
                line(c, &[(38.0, 77.0), (62.0, 77.0)], 3.5);
            }
            Face::Shocked => {
                line(c, &[(19.0, 26.0), (31.0, 14.0), (44.0, 22.0)], 4.0);
                line(c, &[(56.0, 22.0), (69.0, 14.0), (81.0, 26.0)], 4.0);
                eye(c, (32.0, 42.0), 12.0, (0.0, 0.0), 2.6);
                eye(c, (68.0, 42.0), 12.0, (0.0, 0.0), 2.6);
                shape(
                    c,
                    &Path::oval(Rect::from_ltrb(40.0, 66.0, 60.0, 97.0), None),
                    BLACK,
                );
                drop(c, (88.0, 36.0), 4.5);
            }
            Face::Derp => {
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
                teeth(c, &mouth, Rect::from_ltrb(20.0, 55.0, 85.0, 70.0), 5);
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
            Face::Laugh => {
                line(c, &[(19.0, 27.0), (31.0, 20.0), (43.0, 26.0)], 4.0);
                line(c, &[(57.0, 26.0), (69.0, 20.0), (81.0, 27.0)], 4.0);
                line(c, &[(22.0, 47.0), (32.0, 36.0), (42.0, 47.0)], 4.5);
                line(c, &[(58.0, 47.0), (68.0, 36.0), (78.0, 47.0)], 4.5);
                drop(c, (13.0, 50.0), 3.5);
                drop(c, (87.0, 50.0), 3.5);
                let mouth = curve(
                    &[
                        (22.0, 59.0),
                        (50.0, 60.0),
                        (78.0, 59.0),
                        (70.0, 88.0),
                        (50.0, 99.0),
                        (30.0, 88.0),
                    ],
                    true,
                );
                c.draw_path(&mouth, &fill(BLACK));
                teeth(c, &mouth, Rect::from_ltrb(15.0, 50.0, 85.0, 67.0), 6);
                within(c, &mouth, || {
                    c.draw_oval(Rect::from_ltrb(32.0, 82.0, 68.0, 110.0), &fill(TONGUE));
                });
                c.draw_path(&mouth, &ink(3.2));
            }
            Face::Disgust => {
                line(c, &[(18.0, 26.0), (30.0, 15.0), (44.0, 25.0)], 4.0);
                line(c, &[(55.0, 37.0), (80.0, 31.0)], 4.5);
                eye(c, (32.0, 41.0), 10.0, (2.0, 1.0), 3.8);
                line(c, &[(58.0, 45.0), (68.0, 43.0), (79.0, 44.0)], 4.0);
                line(
                    c,
                    &[
                        (28.0, 80.0),
                        (40.0, 73.0),
                        (52.0, 79.0),
                        (64.0, 70.0),
                        (77.0, 73.0),
                    ],
                    3.5,
                );
            }
        }
    }
}
