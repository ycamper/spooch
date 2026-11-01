//! The README's images: a framed capture, editing in place, the editor
//! window with Recents, and sharing. Each is drawn through the editor at
//! 2x over a made-up terminal session, and must stay within the budget:
//!
//!     cargo run --release -p spooch-render --example readme_images
//!
//! The images go to `assets/readme/`, or the folder given as argument 1.

mod common;

use std::collections::{HashMap, HashSet};

use skia_safe::{
    AlphaType, Canvas, Color, Color4f, Font, IRect, Image, Paint, Point, RRect, Rect, surfaces,
};
use spooch_render::{
    Document, Exported, Frame, Label, OUTSIDE_BRIGHTNESS, STICKERS, SyncBadge, Tool,
    selection_frame, thumbnail,
};

/// The largest PNG the README takes, in bytes.
const BUDGET: usize = 300_000;
const SCALE: f32 = 2.0;
/// The whole desk, at 2x.
const DESK: (i32, i32) = (1760, 1100);
/// The terminal's content on the desk: what gets captured.
const APP: IRect = IRect {
    left: 180,
    top: 180,
    right: 1580,
    bottom: 960,
};
/// The editor window, just wide enough at 2x for the gallery to show.
const WINDOW: (i32, i32) = (1240, 720);
/// The window when sharing: wide enough for the Share button's label.
const SHARE_WINDOW: (i32, i32) = (1440, 760);
/// Earlier captures in Recents, with their sync state; the last is shared.
const RECENT: [(&str, SyncBadge); 4] = [
    ("2026-10-17 15.37.12", SyncBadge::Synced),
    ("2026-10-18 09.10.00", SyncBadge::Remote),
    ("2026-10-18 10.20.00", SyncBadge::Waiting),
    ("2026-10-18 10.40.00", SyncBadge::Synced),
];
const CURRENT: &str = "2026-10-18 11.00.00";
const TODAY: (u16, u8, u8) = (2026, 10, 18);
/// What the app's Share button says after a share
/// (`spooch-app/src/session/share.rs`).
const SHARED: &str = "Shared \u{b7} link copied";

/// The terminal's text size, line pitch, left margin and table column:
/// a tab moves to the next column.
const TEXT: f32 = 26.0;
const LINE: f32 = 40.0;
const MARGIN: f32 = 40.0;
const TAB: f32 = 150.0;
const PROMPT: &str = "lz@spooch$ ";
const TITLE: &str = "lz@spooch: ~";

#[derive(Clone, Copy)]
enum Ink {
    Prompt,
    Command,
    Out,
    Dim,
    Good,
    Bad,
}

/// The session, a line at a time, each in runs of one ink.
const SESSION: &[&[(Ink, &str)]] = &[
    &[
        (Ink::Prompt, PROMPT),
        (Ink::Command, "sudo nmap -sV 10.0.4.21"),
    ],
    &[(Ink::Dim, "Starting Nmap 7.94 at 2026-10-18 10:57 UTC")],
    &[(Ink::Out, "Nmap scan report for db-01.internal (10.0.4.21)")],
    &[(Ink::Dim, "PORT\tSTATE\tSERVICE\tVERSION")],
    &[
        (Ink::Out, "22/tcp\t"),
        (Ink::Good, "open"),
        (Ink::Out, "\tssh\tOpenSSH 9.6p1"),
    ],
    &[
        (Ink::Out, "443/tcp\t"),
        (Ink::Good, "open"),
        (Ink::Out, "\thttps\tnginx 1.24.0"),
    ],
    &[
        (Ink::Out, "3306/tcp\t"),
        (Ink::Good, "open"),
        (Ink::Out, "\tmysql\tMySQL 8.0.36"),
    ],
    &[
        (Ink::Out, "6379/tcp\t"),
        (Ink::Good, "open"),
        (Ink::Out, "\tredis\tRedis 7.2.4 "),
        (Ink::Bad, "(protected mode: off)"),
    ],
    &[
        (Ink::Prompt, PROMPT),
        (Ink::Command, "cat ~/.aws/credentials"),
    ],
    &[(Ink::Out, "aws_access_key_id = AKIAIOSFODNN7EXAMPLE")],
    &[(
        Ink::Out,
        "aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
    )],
    &[
        (Ink::Prompt, PROMPT),
        (Ink::Command, "sudo tail -f /var/log/auth.log"),
    ],
    &[
        (Ink::Out, "Oct 18 10:58:02 sshd[4121]: "),
        (Ink::Bad, "Failed password for root from 203.0.113.9"),
    ],
    &[
        (Ink::Out, "Oct 18 10:58:04 sshd[4121]: "),
        (Ink::Bad, "Failed password for root from 203.0.113.9"),
    ],
    &[
        (Ink::Out, "Oct 18 10:58:07 sshd[4127]: "),
        (Ink::Bad, "Invalid user admin from 203.0.113.9"),
    ],
];
const DB: usize = 6;
const REDIS: usize = 7;
const SECRET: usize = 10;
const FIRST_LOG: usize = 12;
const LAST_LOG: usize = 14;

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/readme").into());
    std::fs::create_dir_all(&dir).expect("the output folder");
    let face = Label::ui_typeface().expect("the bundled UI font");
    let term = Term {
        font: Font::from_typeface(face, TEXT),
    };
    let desk = common::pixels(DESK, |c| draw_desk(c, &term));

    let hero = hero(&desk, &term);
    write(&dir, "hero.png", &common::exported(&hero));
    let markup = markup(&desk, &term);
    write(&dir, "markup.png", &markup);
    // Earlier captures for Recents, oldest first: two bits of the desk and
    // these two. All but the hero are opaque, so their pixels are straight.
    let thumbs = [
        (900, 360, crop_desk(&desk, (0, 0), (900, 360))),
        (1000, 300, crop_desk(&desk, (0, 460), (1000, 300))),
        (hero.width, hero.height, hero.bgra),
        (
            DESK.0 as u32,
            DESK.1 as u32,
            common::pixels(DESK, |c| {
                c.draw_image(&markup, (0, 0), None);
            }),
        ),
    ];
    let mut d = capture(&desk, &term);
    write(&dir, "window.png", &window(&mut d, &thumbs));
    write(&dir, "sharing.png", &sharing(&mut d));
}

/// A finished capture, framed as a window with its shadow.
fn hero(desk: &[u8], term: &Term) -> Exported {
    let mut d = capture(desk, term);
    d.set_frame(Frame::from_key("window-mac").expect("the window-mac frame"));
    d.set_frame_title(TITLE);
    d.export_image()
}

/// Editing in place: the desk dimmed round the selection, the marks, the
/// selection's frame and the chrome, painted as the shells paint them.
fn markup(desk: &[u8], term: &Term) -> Image {
    let mut d = placed(desk);
    let (top, bottom) = term.rows(REDIS);
    let right = term.end(REDIS) + 16.0;
    draw(&mut d, Tool::Box, at(26.0, top), at(right, bottom));
    label(&mut d, at(right + 40.0, top - 110.0), "Lock this down");
    draw(
        &mut d,
        Tool::Arrow,
        at(right + 120.0, top - 76.0),
        at(right - 6.0, top - 4.0),
    );
    sticker(&mut d, "stamp_todo", at(1220.0, 470.0));
    // Select the box, so it shows its grips.
    use_tool(&mut d, Tool::Arrow);
    let edge = Point::from(at(26.0, (top + bottom) / 2.0));
    assert!(!d.on_chrome(edge), "{edge:?} is on the chrome");
    d.begin_drag(edge);
    d.end_drag();

    let mut s = surfaces::raster_n32_premul(DESK).expect("a surface");
    let c = s.canvas();
    c.draw_image(common::image(DESK, desk, AlphaType::Premul), (0, 0), None);
    let dim = 1.0 - f32::from(OUTSIDE_BRIGHTNESS) / 100.0;
    c.draw_rect(
        Rect::from_iwh(DESK.0, DESK.1),
        &Paint::new(Color4f::new(0.0, 0.0, 0.0, dim), None),
    );
    let sel = d.canvas();
    let (px, stride) = d.render();
    let shown = common::crop(px, stride, IRect::from_wh(sel.width(), sel.height()));
    c.draw_image(
        common::image((sel.width(), sel.height()), &shown, AlphaType::Premul),
        (sel.left, sel.top),
        None,
    );
    for (r, rgb) in selection_frame(sel, true) {
        let mut p = Paint::default();
        p.set_color(Color::new(rgb | 0xff00_0000));
        c.draw_irect(r, &p);
    }
    for p in d.chrome().to_vec() {
        let size = (p.rect.width(), p.rect.height());
        let panel = common::image(size, &p.pixels, AlphaType::Premul);
        c.draw_image(panel, (p.rect.left, p.rect.top), None);
    }
    s.image_snapshot()
}

/// The editor window with Recents: earlier captures from `thumbs`, as
/// straight BGRA, the last one shared.
fn window(d: &mut Document, thumbs: &[(u32, u32, Vec<u8>)]) -> Image {
    let current = d.export_image();
    d.set_name("db-01 scan");
    d.set_window_size(WINDOW.0, WINDOW.1);
    let ids: Vec<String> = RECENT.iter().map(|(id, _)| id.to_string()).collect();
    let sync: HashMap<String, SyncBadge> = RECENT.iter().map(|&(id, b)| (id.into(), b)).collect();
    let shared = HashSet::from([ids[3].clone()]);
    d.set_gallery(&ids, &shared, &sync, CURRENT, TODAY);
    for (id, (w, h, px)) in ids.iter().zip(thumbs) {
        set_thumb(d, id, *w, *h, px);
    }
    set_thumb(d, CURRENT, current.width, current.height, &current.bgra);
    assert!(d.gallery_missing().is_empty(), "a card without a thumbnail");
    shot(d, IRect::from_wh(WINDOW.0, WINDOW.1))
}

/// After a share: a wider window's lower part, with the gallery's cards
/// and the bottom bar.
fn sharing(d: &mut Document) -> Image {
    d.set_share_label(Some(SHARED));
    let (w, h) = SHARE_WINDOW;
    d.set_window_size(w, h);
    shot(d, IRect::from_ltrb(0, h - 380, w, h))
}

/// The terminal's content with the hero's marks on it: the exposed
/// database, the open Redis, the leaked key, and who is knocking.
fn capture(desk: &[u8], term: &Term) -> Document {
    let mut d = placed(desk);
    let (top, bottom) = term.rows(DB);
    draw(
        &mut d,
        Tool::Box,
        at(26.0, top),
        at(term.end(DB) + 16.0, bottom),
    );
    let step = |row: usize| at(term.end(row) + 54.0, term.baseline(row) - 9.0);
    let steps = [step(DB), step(SECRET), step(FIRST_LOG)];

    use_tool(&mut d, Tool::Highlight);
    // Spotlight, Marker, Band, then Neon, which reads on a dark terminal.
    for _ in 0..3 {
        assert!(d.cycle_look(), "another highlight look");
    }
    let (left, right) = term.span(REDIS, "(protected mode: off)");
    let (top, bottom) = term.rows(REDIS);
    draw(
        &mut d,
        Tool::Highlight,
        at(left - 6.0, top),
        at(right + 6.0, bottom),
    );
    sticker(&mut d, "stamp_nope", at(1210.0, 300.0));

    let (left, right) = term.span(SECRET, "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");
    let (top, bottom) = term.rows(SECRET);
    draw(
        &mut d,
        Tool::Blur,
        at(left - 8.0, top),
        at(right + 8.0, bottom),
    );

    for p in steps {
        click(&mut d, Tool::Steps, p);
    }
    let (_, ip) = term.span(LAST_LOG, "203.0.113.9");
    let below = term.rows(LAST_LOG).1;
    label(&mut d, at(ip - 380.0, below + 66.0), "Who is 203.0.113.9?");
    draw(
        &mut d,
        Tool::Arrow,
        at(ip + 70.0, below + 50.0),
        at(ip + 10.0, below - 6.0),
    );
    use_tool(&mut d, Tool::Arrow);
    d
}

/// The terminal's text laid out: where each line and each run goes, in
/// pixels from the corner of the terminal's content.
struct Term {
    font: Font,
}

impl Term {
    fn baseline(&self, row: usize) -> f32 {
        60.0 + row as f32 * LINE
    }

    /// The top and bottom of a box round `row`.
    fn rows(&self, row: usize) -> (f32, f32) {
        let b = self.baseline(row);
        (b - 30.0, b + 13.0)
    }

    /// Each piece of `row`'s runs, split at tabs, with its left edge.
    fn runs(&self, row: usize) -> Vec<(Ink, &'static str, f32)> {
        let mut x = MARGIN;
        let mut out = Vec::new();
        for &(ink, text) in SESSION[row] {
            for (k, piece) in text.split('\t').enumerate() {
                if k > 0 {
                    x = MARGIN + (((x - MARGIN) / TAB).floor() + 1.0) * TAB;
                }
                out.push((ink, piece, x));
                x += self.width(piece);
            }
        }
        out
    }

    fn width(&self, s: &str) -> f32 {
        self.font.measure_str(s, None).0
    }

    /// Where `row`'s text ends.
    fn end(&self, row: usize) -> f32 {
        let (_, piece, x) = *self.runs(row).last().expect("a line with text");
        x + self.width(piece)
    }

    /// The left and right of `needle` on `row`.
    fn span(&self, row: usize, needle: &str) -> (f32, f32) {
        self.runs(row)
            .into_iter()
            .find_map(|(_, piece, x)| {
                let i = piece.find(needle)?;
                let left = x + self.width(&piece[..i]);
                Some((left, left + self.width(needle)))
            })
            .expect("the text on that line")
    }
}

/// The terminal's content captured on the desk, so the chrome sits round
/// it as it would on a screen.
fn placed(desk: &[u8]) -> Document {
    let px = common::crop(desk, DESK.0 as usize * 4, APP);
    let (w, h) = (APP.width(), APP.height());
    let mut d =
        Document::from_bgra(w as u32, h as u32, w as usize * 4, &px, SCALE).expect("a document");
    let screen = IRect::from_wh(DESK.0, DESK.1);
    d.place(APP.left, APP.top, screen);
    d.set_screens(vec![screen]);
    d
}

/// A point on the terminal's content, in desk pixels.
fn at(x: f32, y: f32) -> (f32, f32) {
    (APP.left as f32 + x, APP.top as f32 + y)
}

fn use_tool(d: &mut Document, tool: Tool) {
    d.set_tool(tool);
    assert_eq!(d.tool(), tool);
}

/// One mark, dragged out with `tool`.
fn draw(d: &mut Document, tool: Tool, from: (f32, f32), to: (f32, f32)) {
    use_tool(d, tool);
    adds_a_mark(d, from, |d| common::drag(d, from, to));
}

fn click(d: &mut Document, tool: Tool, at: (f32, f32)) {
    use_tool(d, tool);
    adds_a_mark(d, at, |d| {
        d.begin_drag(Point::from(at));
        d.end_drag();
    });
}

fn label(d: &mut Document, at: (f32, f32), text: &str) {
    use_tool(d, Tool::Text);
    adds_a_mark(d, at, |d| {
        d.begin_text(Point::from(at));
        d.type_text(text);
        d.commit_text();
    });
}

fn sticker(d: &mut Document, name: &str, at: (f32, f32)) {
    use_tool(d, Tool::Sticker);
    let i = STICKERS
        .iter()
        .position(|(n, _)| *n == name)
        .expect("a known sticker");
    d.pick_sticker(i);
    click(d, Tool::Sticker, at);
}

/// Run `act`, a press at `at`, and check it made a mark rather than
/// hitting the chrome.
fn adds_a_mark(d: &mut Document, at: (f32, f32), act: impl FnOnce(&mut Document)) {
    assert!(!d.on_chrome(Point::from(at)), "{at:?} is on the chrome");
    let before = d.counts();
    act(d);
    assert_ne!(d.counts(), before, "no mark at {at:?}");
}

fn set_thumb(d: &mut Document, id: &str, w: u32, h: u32, bgra: &[u8]) {
    let (tw, th, px) = thumbnail(w, h, bgra).expect("a thumbnail");
    d.set_gallery_thumb(id, tw, th, &px);
}

/// `area` of the editor window.
fn shot(d: &mut Document, area: IRect) -> Image {
    let (px, stride) = d.render_window();
    let px = common::crop(px, stride, area);
    common::image((area.width(), area.height()), &px, AlphaType::Premul)
}

/// A `size` piece of the terminal's content, `at` from its corner.
fn crop_desk(desk: &[u8], at: (i32, i32), size: (i32, i32)) -> Vec<u8> {
    let area = IRect::from_xywh(APP.left + at.0, APP.top + at.1, size.0, size.1);
    common::crop(desk, DESK.0 as usize * 4, area)
}

/// Encode `img`, check it is within the budget, and write it to `dir`.
fn write(dir: &str, name: &str, img: &Image) {
    let png = common::png(img);
    assert!(
        png.size() <= BUDGET,
        "{name} is {} bytes, over the {BUDGET} byte budget",
        png.size()
    );
    let path = format!("{dir}/{name}");
    std::fs::write(&path, png.as_bytes()).expect("written");
    println!(
        "{path}: {}x{}, {} bytes",
        img.width(),
        img.height(),
        png.size()
    );
}

/// The made-up desk: a dark backdrop, a taskbar, and a terminal window
/// whose content is `APP`.
fn draw_desk(c: &Canvas, term: &Term) {
    let fill = |r: f32, g: f32, b: f32| {
        let mut p = Paint::new(Color4f::new(r, g, b, 1.0), None);
        p.set_anti_alias(true);
        p
    };
    let (dw, dh) = (DESK.0 as f32, DESK.1 as f32);
    c.clear(Color4f::new(0.16, 0.2, 0.3, 1.0));
    c.draw_rect(
        Rect::from_xywh(0.0, dh - 72.0, dw, 72.0),
        &fill(0.08, 0.09, 0.12),
    );
    for i in 0..5 {
        let r = Rect::from_xywh(dw / 2.0 - 170.0 + i as f32 * 70.0, dh - 58.0, 44.0, 44.0);
        c.draw_rrect(RRect::new_rect_xy(r, 10.0, 10.0), &fill(0.24, 0.28, 0.38));
    }

    // The window: a title bar above the content.
    let (x, y) = (APP.left as f32, APP.top as f32);
    let (w, h) = (APP.width() as f32, APP.height() as f32);
    let outer = Rect::from_xywh(x, y - 64.0, w, h + 64.0);
    c.draw_rrect(
        RRect::new_rect_xy(outer, 16.0, 16.0),
        &fill(0.2, 0.21, 0.24),
    );
    let width = term.width(TITLE);
    c.draw_str(
        TITLE,
        (x + (w - width) / 2.0, y - 22.0),
        &term.font,
        &fill(0.7, 0.72, 0.76),
    );
    c.draw_rect(Rect::from_xywh(x, y, w, h), &fill(0.07, 0.08, 0.1));

    let paint = |ink: Ink| match ink {
        Ink::Prompt => fill(0.4, 0.85, 0.5),
        Ink::Command => fill(0.95, 0.96, 0.98),
        Ink::Out => fill(0.75, 0.78, 0.82),
        Ink::Dim => fill(0.5, 0.53, 0.58),
        Ink::Good => fill(0.45, 0.85, 0.55),
        Ink::Bad => fill(0.98, 0.45, 0.4),
    };
    for row in 0..SESSION.len() {
        let baseline = y + term.baseline(row);
        for (ink, piece, left) in term.runs(row) {
            c.draw_str(piece, (x + left, baseline), &term.font, &paint(ink));
        }
    }
    // A last prompt and the cursor.
    let baseline = y + term.baseline(SESSION.len());
    c.draw_str(
        PROMPT,
        (x + MARGIN, baseline),
        &term.font,
        &paint(Ink::Prompt),
    );
    let cursor = x + MARGIN + term.width(PROMPT);
    c.draw_rect(
        Rect::from_xywh(cursor, baseline - 24.0, 14.0, 30.0),
        &fill(0.85, 0.87, 0.9),
    );
}
