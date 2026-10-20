//! The editor window's gallery: Recents as thumbnails down the left edge,
//! newest first under day headings. The session fills it; this lays it
//! out, draws it and says what a press asks for.

use std::collections::{HashMap, HashSet};

use skia_safe::{
    Color4f, Data, FilterMode, IRect, Image, MipmapMode, Paint, PathBuilder, Point, Rect,
    SamplingOptions, images, surfaces,
};

use super::chrome::{Panel, contains, panel};
use super::theme::{
    ACCENT, CLOSE_HOVER, DIVIDER, FIELD, HOVER, ICON, MUTED, PANEL, TEXT, centred_baseline,
    chevron, ellipsize, fill, fill_rrect, stroke, with_alpha,
};
use super::theme::{cross, outlined_pill};
use super::{Action, Document};

/// The strip's width, and the parts of it, in screen pixels (times the
/// display scale).
const WIDTH: f32 = 184.0;
const HEAD_H: f32 = 44.0;
const PILL_H: f32 = 26.0;
const DAY_H: f32 = 28.0;
const SIDE: f32 = 12.0;
const THUMB_H: f32 = 96.0;
const LABEL_H: f32 = 20.0;
const CARD_GAP: f32 = 8.0;
const BADGE: f32 = 22.0;
const DELETE_W: f32 = 58.0;
/// The capture keeps at least this much room beside the strip, or the
/// strip stands aside.
const MIN_AREA: f32 = 360.0;
const WHEEL_STEP: f32 = 48.0;

/// The biggest a thumbnail is made: twice its box, so it stays sharp on a
/// display scaled to 200%.
pub const THUMB_MAX: (u32, u32) = (2 * (WIDTH - 2.0 * SIDE) as u32, 2 * THUMB_H as u32);

/// Which captures the gallery shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Range {
    #[default]
    All,
    Today,
    Week,
}

impl Range {
    fn next(self) -> Range {
        match self {
            Range::All => Range::Today,
            Range::Today => Range::Week,
            Range::Week => Range::All,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Range::All => "All captures",
            Range::Today => "Today",
            Range::Week => "Last 7 days",
        }
    }

    fn shows(self, day: i64, today: i64) -> bool {
        match self {
            Range::All => true,
            Range::Today => day == today,
            Range::Week => day > today - 7,
        }
    }
}

/// What a press on the gallery asks the session to do, with the card's
/// index for `Document::gallery_id`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GalleryAction {
    /// A press on a card: opens it if let go there, drags it out as a file
    /// if the pointer leaves first.
    Press(usize),
    /// Show the capture's folder.
    Reveal(usize),
    /// Delete it from Recents (the second click on its delete button).
    Delete(usize),
}

/// A part of the gallery under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GalleryHit {
    Range,
    Card(usize),
    Reveal(usize),
    Delete(usize),
}

struct Entry {
    id: String,
    /// Days since 1970, from the id.
    day: i64,
    /// "18:42".
    time: String,
    thumb: Option<Image>,
    /// Shared, so its card wears a link.
    shared: bool,
    /// Where it is with syncing, if it syncs.
    sync: Option<SyncBadge>,
}

/// Where a capture is with syncing, as its card shows it: a cloud with a
/// check, an arrow up, or an arrow down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncBadge {
    /// In the account as it is here.
    Synced,
    /// New or changed here, waiting to go up.
    Waiting,
    /// In the account but not here yet: the card is a faded preview.
    Remote,
}

/// The gallery's contents and where it's scrolled to. The session moves it
/// from one document to the next when it opens another capture.
#[derive(Default)]
pub struct Gallery {
    /// Newest first.
    entries: Vec<Entry>,
    /// The capture being edited.
    current: String,
    today: i64,
    range: Range,
    /// How far the list is scrolled, in screen pixels.
    scroll: f32,
    /// The card whose delete button was clicked once.
    armed: Option<usize>,
    /// Scroll the current capture into view at the next layout.
    follow: bool,
    /// Bumped on every change, for the chrome cache.
    version: u64,
    /// The capture being downloaded to open, if one is.
    busy: Option<String>,
}

/// A line of the list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Line {
    /// A day's heading, by the index of its first capture.
    Day(usize),
    Card(usize),
}

/// Where the gallery's parts are, in window pixels.
pub(super) struct GalleryLayout {
    pub rect: IRect,
    pub(super) range: IRect,
    list: IRect,
    /// Each line where it is after scrolling, clipped to nothing.
    pub(super) lines: Vec<(Line, Rect)>,
    /// The whole list's height, unscrolled.
    height: f32,
}

impl Document {
    /// The captures to show, oldest first as Recents lists them, those
    /// that were `shared`, where each is with syncing, the one being edited
    /// (added if it isn't kept yet) and today's date.
    pub fn set_gallery(
        &mut self,
        ids: &[String],
        shared: &HashSet<String>,
        sync: &HashMap<String, SyncBadge>,
        current: &str,
        today: (u16, u8, u8),
    ) {
        let g = &mut self.gallery;
        let mut thumbs: HashMap<String, Image> = g
            .entries
            .drain(..)
            .filter_map(|e| Some((e.id, e.thumb?)))
            .collect();
        let unkept = !current.is_empty() && !ids.iter().any(|id| id == current);
        let newest_first = unkept
            .then_some(current)
            .into_iter()
            .chain(ids.iter().rev().map(String::as_str));
        g.entries = newest_first
            .map(|id| Entry {
                id: id.to_string(),
                day: day_of(id),
                time: id.get(11..16).unwrap_or("").replace('.', ":"),
                thumb: thumbs.remove(id),
                shared: shared.contains(id),
                sync: sync.get(id).copied(),
            })
            .collect();
        if g.current != current {
            g.follow = true;
        }
        g.current = current.to_string();
        g.today = days_from_civil(today.0.into(), today.1.into(), today.2.into());
        g.armed = None;
        g.version += 1;
        self.chrome_key = None;
    }

    /// Show capture `id` downloading, or none.
    pub fn set_gallery_busy(&mut self, id: Option<&str>) {
        let g = &mut self.gallery;
        if g.busy.as_deref() != id {
            g.busy = id.map(str::to_string);
            g.version += 1;
            self.chrome_key = None;
        }
    }

    /// Capture `id`'s thumbnail: straight BGRA, `width` by `height`.
    pub fn set_gallery_thumb(&mut self, id: &str, width: u32, height: u32, bgra: &[u8]) {
        let Some(e) = self.gallery.entries.iter_mut().find(|e| e.id == id) else {
            return;
        };
        let info = crate::bgra_info(
            (width as i32, height as i32),
            skia_safe::AlphaType::Unpremul,
        );
        e.thumb = images::raster_from_data(&info, Data::new_copy(bgra), width as usize * 4)
            .map(|i| i.with_default_mipmaps().unwrap_or(i));
        self.gallery.version += 1;
        self.chrome_key = None;
    }

    /// The captures still without a thumbnail, newest first.
    pub fn gallery_missing(&self) -> Vec<String> {
        let g = &self.gallery;
        g.entries
            .iter()
            .filter(|e| e.thumb.is_none())
            .map(|e| e.id.clone())
            .collect()
    }

    /// The capture card `index` shows.
    pub fn gallery_id(&self, index: usize) -> Option<&str> {
        self.gallery.entries.get(index).map(|e| e.id.as_str())
    }

    /// Hand the gallery over, to give it to the next capture's document.
    pub fn take_gallery(&mut self) -> Gallery {
        self.chrome_key = None;
        std::mem::take(&mut self.gallery)
    }

    pub fn put_gallery(&mut self, gallery: Gallery) {
        self.gallery = gallery;
        self.gallery.follow = true;
        self.chrome_key = None;
    }

    /// Show or hide the gallery (Ctrl+H, or the top row's button). The
    /// choice carries on to later captures with the styles.
    pub(crate) fn toggle_gallery(&mut self) -> bool {
        if !self.in_window() {
            return false;
        }
        self.styles.gallery = !self.styles.gallery;
        self.gallery.follow = true;
        self.chrome_key = None;
        self.settle();
        true
    }

    /// Whether the gallery is meant to show (it may still stand aside in a
    /// narrow window).
    pub(super) fn gallery_on(&self) -> bool {
        self.styles.gallery
    }

    /// The strip's width in a window `width` pixels wide, if it shows.
    pub(super) fn gallery_width(&self, width: i32) -> Option<i32> {
        let s = self.scale;
        let w = (WIDTH * s).round() as i32;
        let room = width - w - self.rail_width();
        (self.styles.gallery && room as f32 >= MIN_AREA * s).then_some(w)
    }

    /// The gallery's layout, when it shows.
    pub(super) fn gallery_layout(&self) -> Option<GalleryLayout> {
        let rect = self.dock()?.gallery?;
        let s = self.scale;
        let g = &self.gallery;
        let side = SIDE * s;
        let pill_h = (PILL_H * s).round() as i32;
        let head = (HEAD_H * s).round() as i32;
        let range = IRect::from_xywh(
            rect.left + side as i32,
            rect.top + (head - pill_h) / 2,
            rect.width() - 2 * side as i32,
            pill_h,
        );
        let list = IRect::from_ltrb(rect.left, rect.top + head, rect.right, rect.bottom);
        let mut lines = Vec::new();
        let mut y = 0.0;
        let mut last_day = None;
        for (i, e) in g.entries.iter().enumerate() {
            if !g.range.shows(e.day, g.today) {
                continue;
            }
            if last_day != Some(e.day) {
                lines.push((Line::Day(i), y, DAY_H * s));
                y += DAY_H * s;
                last_day = Some(e.day);
            }
            let h = (THUMB_H + LABEL_H) * s;
            lines.push((Line::Card(i), y, h));
            y += h + CARD_GAP * s;
        }
        let top = list.top as f32 - g.scroll;
        let (l, r) = (rect.left as f32 + side, rect.right as f32 - side);
        let lines = lines
            .into_iter()
            .map(|(line, y, h)| (line, Rect::from_ltrb(l, top + y, r, top + y + h)))
            .collect();
        Some(GalleryLayout {
            rect,
            range,
            list,
            lines,
            height: y,
        })
    }

    /// The part of the gallery under window point `p`.
    pub(super) fn gallery_hit(&self, p: Point) -> Option<GalleryHit> {
        let l = self.gallery_layout()?;
        if !contains(&l.rect, p) {
            return None;
        }
        if contains(&l.range, p) {
            return Some(GalleryHit::Range);
        }
        if !contains(&l.list, p) {
            return None;
        }
        l.lines.iter().find_map(|(line, r)| {
            let Line::Card(i) = *line else { return None };
            if !inside(r, p) {
                return None;
            }
            let (reveal, delete) = self.badges(i, *r);
            Some(if inside(&delete, p) {
                GalleryHit::Delete(i)
            } else if inside(&reveal, p) {
                GalleryHit::Reveal(i)
            } else {
                GalleryHit::Card(i)
            })
        })
    }

    /// Card `i`'s buttons in its top-right corner: show in folder, and
    /// delete (wider once armed).
    pub(super) fn badges(&self, i: usize, card: Rect) -> (Rect, Rect) {
        let s = self.scale;
        let pad = 6.0 * s;
        let b = BADGE * s;
        let w = if self.gallery.armed == Some(i) {
            DELETE_W * s
        } else {
            b
        };
        let delete = Rect::from_xywh(card.right - pad - w, card.top + pad, w, b);
        let reveal = Rect::from_xywh(delete.left - 4.0 * s - b, card.top + pad, b, b);
        (reveal, delete)
    }

    /// A press on the gallery.
    pub(super) fn press_gallery(&mut self, hit: GalleryHit) {
        let g = &mut self.gallery;
        match hit {
            GalleryHit::Range => {
                g.range = g.range.next();
                g.scroll = 0.0;
                g.follow = true;
            }
            GalleryHit::Card(i) => {
                if g.entries.get(i).is_some_and(|e| e.id != g.current) {
                    self.action = Some(Action::Gallery(GalleryAction::Press(i)));
                }
            }
            GalleryHit::Reveal(i) => self.action = Some(Action::Gallery(GalleryAction::Reveal(i))),
            GalleryHit::Delete(i) if g.armed == Some(i) => {
                self.action = Some(Action::Gallery(GalleryAction::Delete(i)));
            }
            GalleryHit::Delete(_) => {}
        }
        let g = &mut self.gallery;
        g.armed = match hit {
            GalleryHit::Delete(i) if g.armed != Some(i) => Some(i),
            _ => None,
        };
        g.version += 1;
        self.chrome_key = None;
    }

    /// A press anywhere but the gallery's delete buttons: let go of an
    /// armed delete.
    pub(super) fn disarm_gallery(&mut self) {
        if self.gallery.armed.take().is_some() {
            self.gallery.version += 1;
            self.chrome_key = None;
        }
    }

    /// The mouse wheel over the gallery scrolls it. Returns whether the
    /// pointer was over it.
    pub(super) fn gallery_wheel(&mut self, p: Point, notches: f32) -> Option<bool> {
        let l = self.gallery_layout()?;
        if !contains(&l.rect, p) {
            return None;
        }
        let before = self.gallery.scroll;
        let max = (l.height - l.list.height() as f32).max(0.0);
        self.gallery.scroll = (before - notches * WHEEL_STEP * self.scale).clamp(0.0, max);
        let moved = self.gallery.scroll != before;
        if moved {
            self.gallery.version += 1;
            self.chrome_key = None;
        }
        Some(moved)
    }

    /// Keep the scroll in range, and bring the capture being edited into
    /// view after it changes.
    pub(super) fn settle_gallery(&mut self) {
        let Some(l) = self.gallery_layout() else {
            return;
        };
        let g = &self.gallery;
        let max = (l.height - l.list.height() as f32).max(0.0);
        let mut scroll = g.scroll.clamp(0.0, max);
        if g.follow {
            let current = g.entries.iter().position(|e| e.id == g.current);
            let card = l
                .lines
                .iter()
                .find(|(line, _)| Some(*line) == current.map(Line::Card));
            if let Some((_, r)) = card {
                let (top, bottom) = (l.list.top as f32, l.list.bottom as f32);
                if r.top < top {
                    scroll -= top - r.top + DAY_H * self.scale;
                } else if r.bottom > bottom {
                    scroll += r.bottom - bottom + CARD_GAP * self.scale;
                }
                scroll = scroll.clamp(0.0, max);
            }
        }
        let g = &mut self.gallery;
        g.follow = false;
        if scroll != g.scroll {
            g.scroll = scroll;
            g.version += 1;
            self.chrome_key = None;
        }
    }

    /// What the chrome cache needs to know about the gallery.
    pub(super) fn gallery_key(&self) -> Option<(u64, i32)> {
        self.dock()?.gallery?;
        Some((self.gallery.version, self.gallery.scroll as i32))
    }

    pub(super) fn gallery_tip(&self, hit: GalleryHit) -> Option<String> {
        let g = &self.gallery;
        Some(match hit {
            GalleryHit::Range => format!(
                "Showing {}   click for {}",
                g.range.name().to_lowercase(),
                g.range.next().name().to_lowercase()
            ),
            GalleryHit::Card(_) => return None,
            GalleryHit::Reveal(_) => "Show in folder".to_string(),
            GalleryHit::Delete(i) if g.armed == Some(i) => {
                "Click again to delete it for good".to_string()
            }
            GalleryHit::Delete(_) => "Delete from Recents".to_string(),
        })
    }

    /// Where a tooltip for `hit` points from.
    pub(super) fn gallery_place(&self, hit: GalleryHit) -> Option<IRect> {
        let l = self.gallery_layout()?;
        let card = |i: usize| {
            l.lines
                .iter()
                .find(|(line, _)| *line == Line::Card(i))
                .map(|(_, r)| *r)
        };
        let r = match hit {
            GalleryHit::Range => Rect::from(l.range),
            GalleryHit::Card(i) => card(i)?,
            GalleryHit::Reveal(i) => self.badges(i, card(i)?).0,
            GalleryHit::Delete(i) => self.badges(i, card(i)?).1,
        };
        Some(r.round())
    }

    /// The gallery's panel.
    pub(super) fn draw_gallery(&self, l: &GalleryLayout, hovered: Option<GalleryHit>) -> Panel {
        let s = self.scale;
        let g = &self.gallery;
        let r = l.rect;
        let origin = Point::new(r.left as f32, r.top as f32);
        let font = self.bar_font();
        panel(r, |canvas| {
            let (w, h) = (r.width() as f32, r.height() as f32);
            canvas.draw_rect(Rect::from_wh(w, h), &fill(PANEL));
            let hair = s.max(1.0).round();
            canvas.draw_rect(Rect::from_ltrb(w - hair, 0.0, w, h), &fill(DIVIDER));
            let Some(font) = &font else { return };

            // The range pill.
            let pill = Rect::from(l.range).with_offset(-origin);
            outlined_pill(canvas, pill, hovered == Some(GalleryHit::Range), s);
            let mid = centred_baseline(font, pill.center_y());
            canvas.draw_str(
                g.range.name(),
                (pill.left + 12.0 * s, mid),
                font,
                &fill(TEXT),
            );
            chevron(
                canvas,
                Point::new(pill.right - 14.0 * s, pill.center_y()),
                s,
                ICON,
            );

            let list = Rect::from(l.list).with_offset(-origin);
            canvas.save();
            canvas.clip_rect(list, None, false);
            if l.lines.is_empty() {
                let text = if g.range == Range::All {
                    "Nothing kept yet"
                } else {
                    "Nothing in this range"
                };
                let tw = font.measure_str(text, None).0;
                let y = list.top + 40.0 * s;
                canvas.draw_str(text, (list.center_x() - tw / 2.0, y), font, &fill(MUTED));
            }
            for (line, rect) in &l.lines {
                let rect = rect.with_offset(-origin);
                if rect.bottom < list.top || rect.top > list.bottom {
                    continue;
                }
                match *line {
                    Line::Day(i) => {
                        let y = centred_baseline(font, rect.center_y() + 2.0 * s);
                        let text = day_heading(&g.entries[i].id, g.today);
                        canvas.draw_str(&text, (rect.left, y), font, &fill(MUTED));
                    }
                    Line::Card(i) => self.draw_card(canvas, i, rect, hovered, font),
                }
            }
            canvas.restore();
        })
    }

    fn draw_card(
        &self,
        canvas: &skia_safe::Canvas,
        i: usize,
        card: Rect,
        hovered: Option<GalleryHit>,
        font: &skia_safe::Font,
    ) {
        let s = self.scale;
        let g = &self.gallery;
        let Some(e) = g.entries.get(i) else { return };
        let current = e.id == g.current;
        let hover = matches!(
            hovered,
            Some(GalleryHit::Card(j) | GalleryHit::Reveal(j) | GalleryHit::Delete(j)) if j == i
        );
        let frame = Rect::from_xywh(card.left, card.top, card.width(), THUMB_H * s);
        let radius = 6.0 * s;
        fill_rrect(canvas, frame, radius, if hover { HOVER } else { FIELD });
        let busy = g.busy.as_deref() == Some(e.id.as_str());
        if let Some(img) = &e.thumb {
            let inner = frame.with_inset((4.0 * s, 4.0 * s));
            let (iw, ih) = (img.width() as f32, img.height() as f32);
            let k = (inner.width() / iw).min(inner.height() / ih);
            let dst = Rect::from_xywh(
                inner.center_x() - iw * k / 2.0,
                inner.center_y() - ih * k / 2.0,
                iw * k,
                ih * k,
            );
            // A preview is a little faded until it's here.
            let mut paint = Paint::default();
            if e.sync == Some(SyncBadge::Remote) {
                paint.set_alpha_f(if busy { 0.35 } else { 0.7 });
            }
            canvas.draw_image_rect_with_sampling_options(
                img,
                None,
                dst,
                SamplingOptions::new(FilterMode::Linear, MipmapMode::Linear),
                &paint,
            );
        }
        if busy {
            let text = "Downloading\u{2026}";
            let tw = font.measure_str(text, None).0;
            let y = centred_baseline(font, frame.center_y());
            canvas.draw_str(text, (frame.center_x() - tw / 2.0, y), font, &fill(TEXT));
        }
        if current {
            let ring = frame.with_outset((s, s));
            canvas.draw_rrect(
                skia_safe::RRect::new_rect_xy(ring, radius + s, radius + s),
                &stroke(ACCENT, 2.0 * s),
            );
        }
        // Syncing in the bottom right corner, sharing in the bottom left.
        let b = BADGE * s;
        let badge = |left: bool, colour: Color4f| {
            let x = if left {
                frame.left + 6.0 * s
            } else {
                frame.right - 6.0 * s - b
            };
            let at = Rect::from_xywh(x, frame.bottom - 6.0 * s - b, b, b);
            fill_rrect(canvas, at, b / 2.0, colour);
            at.center()
        };
        let dark = with_alpha(Color4f::new(0.0, 0.0, 0.0, 1.0), 0.6);
        if let Some(state) = e.sync {
            let colour = match state {
                SyncBadge::Synced => SYNCED,
                SyncBadge::Waiting => WAITING,
                SyncBadge::Remote => dark,
            };
            draw_sync_state(canvas, badge(false, colour), s, state);
        }
        if e.shared {
            draw_link(canvas, badge(true, dark), s);
        }
        let y = centred_baseline(font, frame.bottom + LABEL_H * s / 2.0 + 1.0 * s);
        let label = if current {
            format!("{}  \u{b7}  editing", e.time)
        } else {
            e.time.clone()
        };
        let label = ellipsize(font, &label, card.width());
        canvas.draw_str(
            &label,
            (card.left + 2.0 * s, y),
            font,
            &fill(if current { TEXT } else { MUTED }),
        );

        if !hover {
            return;
        }
        let (reveal, delete) = self.badges(i, card);
        let lit = |h: GalleryHit| hovered == Some(h);
        let disc = |r: Rect, on: bool| {
            let bg = with_alpha(
                Color4f::new(0.0, 0.0, 0.0, 1.0),
                if on { 0.85 } else { 0.6 },
            );
            fill_rrect(canvas, r, r.height() / 2.0, bg);
        };
        disc(reveal, lit(GalleryHit::Reveal(i)));
        draw_folder(canvas, reveal.center(), s);
        if g.armed == Some(i) {
            fill_rrect(canvas, delete, delete.height() / 2.0, CLOSE_HOVER);
            let tw = font.measure_str("Delete", None).0;
            let y = centred_baseline(font, delete.center_y());
            canvas.draw_str(
                "Delete",
                (delete.center_x() - tw / 2.0, y),
                font,
                &fill(TEXT),
            );
        } else {
            disc(delete, lit(GalleryHit::Delete(i)));
            cross(canvas, delete.center(), 4.0 * s, &stroke(TEXT, 1.5 * s));
        }
    }
}

/// A small cloud glyph centred on `c`: two puffs on a flat base.
fn draw_cloud(canvas: &skia_safe::Canvas, c: Point, s: f32) {
    let p = fill(TEXT);
    canvas.draw_circle((c.x - 2.5 * s, c.y + 0.5 * s), 2.8 * s, &p);
    canvas.draw_circle((c.x + 1.5 * s, c.y - 0.8 * s), 3.6 * s, &p);
    fill_rrect(
        canvas,
        Rect::from_ltrb(c.x - 5.5 * s, c.y + 0.2 * s, c.x + 5.5 * s, c.y + 3.3 * s),
        1.5 * s,
        TEXT,
    );
}

/// A synced capture's dot: green.
const SYNCED: Color4f = Color4f::new(0.13, 0.62, 0.36, 1.0);
/// One waiting to go up: amber.
const WAITING: Color4f = Color4f::new(0.90, 0.58, 0.10, 1.0);

/// What syncing is at, on its dot: a check (synced), an arrow up (waiting
/// to go up), or a cloud with an arrow down (in the account, not here yet).
fn draw_sync_state(canvas: &skia_safe::Canvas, c: Point, s: f32, state: SyncBadge) {
    let mut pen = stroke(TEXT, 1.6 * s);
    pen.set_stroke_cap(skia_safe::PaintCap::Round);
    pen.set_stroke_join(skia_safe::PaintJoin::Round);
    match state {
        SyncBadge::Synced => {
            let mut pb = PathBuilder::new();
            pb.move_to((c.x - 3.6 * s, c.y + 0.2 * s))
                .line_to((c.x - 1.0 * s, c.y + 2.8 * s))
                .line_to((c.x + 3.8 * s, c.y - 2.6 * s));
            canvas.draw_path(&pb.detach(), &pen);
        }
        SyncBadge::Waiting => {
            canvas.draw_line((c.x, c.y + 3.8 * s), (c.x, c.y - 3.6 * s), &pen);
            let mut pb = PathBuilder::new();
            pb.move_to((c.x - 3.0 * s, c.y - 0.6 * s))
                .line_to((c.x, c.y - 3.6 * s))
                .line_to((c.x + 3.0 * s, c.y - 0.6 * s));
            canvas.draw_path(&pb.detach(), &pen);
        }
        SyncBadge::Remote => {
            draw_cloud(canvas, Point::new(c.x, c.y - 2.0 * s), s);
            let (x, top, tip) = (c.x + 0.5 * s, c.y + 1.5 * s, c.y + 5.0 * s);
            pen.set_stroke_width(1.4 * s);
            canvas.draw_line((x, top), (x, tip), &pen);
            canvas.draw_line((x - 2.0 * s, tip - 2.0 * s), (x, tip), &pen);
            canvas.draw_line((x + 2.0 * s, tip - 2.0 * s), (x, tip), &pen);
        }
    }
}

/// Two links of a chain, on a slant: shared.
fn draw_link(canvas: &skia_safe::Canvas, c: Point, s: f32) {
    let pen = stroke(TEXT, 1.4 * s);
    canvas.save();
    canvas.rotate(-45.0, Some(c));
    for dx in [-2.4, 2.4] {
        let r = Rect::from_xywh(c.x + dx * s - 3.2 * s, c.y - 1.8 * s, 6.4 * s, 3.6 * s);
        canvas.draw_rrect(skia_safe::RRect::new_rect_xy(r, 1.8 * s, 1.8 * s), &pen);
    }
    canvas.restore();
}

/// A small folder glyph centred on `c`.
fn draw_folder(canvas: &skia_safe::Canvas, c: Point, s: f32) {
    let (w, h) = (12.0 * s, 9.0 * s);
    let (l, t) = (c.x - w / 2.0, c.y - h / 2.0 + 0.5 * s);
    let mut pb = PathBuilder::new();
    pb.move_to((l, t + h))
        .line_to((l, t))
        .line_to((l + 4.5 * s, t))
        .line_to((l + 6.0 * s, t + 1.5 * s))
        .line_to((l + w, t + 1.5 * s))
        .line_to((l + w, t + h))
        .close();
    let mut p = stroke(TEXT, 1.2 * s);
    p.set_stroke_join(skia_safe::PaintJoin::Round);
    canvas.draw_path(&pb.detach(), &p);
}

/// Whether `p` is inside `r`, edges left and top included.
fn inside(r: &Rect, p: Point) -> bool {
    p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom
}

/// A thumbnail of a finished capture, at most `THUMB_MAX`, both straight
/// BGRA. `None` if Skia can't make the surface.
pub fn thumbnail(width: u32, height: u32, bgra: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let (mw, mh) = THUMB_MAX;
    let k = (mw as f32 / width as f32)
        .min(mh as f32 / height as f32)
        .min(1.0);
    let tw = ((width as f32 * k).round() as i32).max(1);
    let th = ((height as f32 * k).round() as i32).max(1);
    let straight = |w: i32, h: i32| crate::bgra_info((w, h), skia_safe::AlphaType::Unpremul);
    let src = images::raster_from_data(
        &straight(width as i32, height as i32),
        Data::new_copy(bgra),
        width as usize * 4,
    )?;
    let src = src.with_default_mipmaps().unwrap_or(src);
    let premul = crate::bgra_info((tw, th), skia_safe::AlphaType::Premul);
    let mut surface = surfaces::raster(&premul, None, None)?;
    let canvas = surface.canvas();
    canvas.clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
    canvas.draw_image_rect_with_sampling_options(
        &src,
        None,
        Rect::from_iwh(tw, th),
        SamplingOptions::new(FilterMode::Linear, MipmapMode::Linear),
        &Paint::default(),
    );
    let mut out = vec![0u8; (tw * th * 4) as usize];
    surface
        .read_pixels(&straight(tw, th), &mut out, tw as usize * 4, (0, 0))
        .then_some((tw as u32, th as u32, out))
}

/// Days since 1970-01-01 of a capture id starting "YYYY-MM-DD".
fn day_of(id: &str) -> i64 {
    let part = |r: std::ops::Range<usize>| id.get(r).and_then(|v| v.parse::<i64>().ok());
    match (part(0..4), part(5..7), part(8..10)) {
        (Some(y), Some(m), Some(d)) => days_from_civil(y, m, d),
        _ => 0,
    }
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The heading for capture `id`'s day: "Today", "Yesterday", or
/// "Fri 2 Oct".
fn day_heading(id: &str, today: i64) -> String {
    const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let day = day_of(id);
    match today - day {
        0 => return "Today".into(),
        1 => return "Yesterday".into(),
        _ => {}
    }
    let part = |r: std::ops::Range<usize>| id.get(r).and_then(|v| v.parse::<usize>().ok());
    match (part(5..7), part(8..10)) {
        (Some(m @ 1..=12), Some(d)) => format!(
            "{} {d} {}",
            WEEKDAYS[day.rem_euclid(7) as usize],
            MONTHS[m - 1]
        ),
        _ => id.get(..10).unwrap_or(id).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_headings() {
        assert_eq!(day_of("1970-01-01 00.00.00"), 0);
        let today = day_of("2026-10-04 18.42.56");
        assert_eq!(day_heading("2026-10-04 09.00.00", today), "Today");
        assert_eq!(day_heading("2026-10-03 23.59.59", today), "Yesterday");
        assert_eq!(day_heading("2026-10-02 09.00.00", today), "Fri 2 Oct");
        assert!(Range::Week.shows(today - 6, today));
        assert!(!Range::Week.shows(today - 7, today));
        assert!(!Range::Today.shows(today - 1, today));
    }

    #[test]
    fn thumbnails_fit_the_box_and_keep_alpha() {
        let (w, h) = (1000u32, 300u32);
        let mut px = vec![255u8; (w * h * 4) as usize];
        px[3] = 0;
        let (tw, th, out) = thumbnail(w, h, &px).unwrap();
        assert!(tw <= THUMB_MAX.0 && th <= THUMB_MAX.1);
        assert_eq!(tw, THUMB_MAX.0);
        assert_eq!(out.len(), (tw * th * 4) as usize);
        assert!(out[3] < 255);
        // Small captures aren't blown up.
        let (sw, sh, _) = thumbnail(40, 20, &vec![255u8; 40 * 20 * 4]).unwrap();
        assert_eq!((sw, sh), (40, 20));
    }
}
