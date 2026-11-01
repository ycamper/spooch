//! The editor window's gallery with a card in each sync state, and one
//! shared, to see the badges.
//!
//!     cargo run --release -p spooch-render --example gallery_badges -- gallery-badges.png

use std::collections::{HashMap, HashSet};

mod common;

use skia_safe::AlphaType;
use spooch_render::{Document, SyncBadge};

fn main() {
    let out = common::out("gallery-badges.png");
    let (w, h) = (640u32, 400u32);
    let px: Vec<u8> = (0..w * h)
        .flat_map(|i| {
            let (x, y) = (i % w, i / w);
            [(x / 3) as u8, (y / 2) as u8, 160, 255]
        })
        .collect();
    let mut d = Document::from_bgra(w, h, w as usize * 4, &px, 1.0).unwrap();
    d.set_window_size(1000, 760);
    let ids: Vec<String> = [
        "2026-10-17 15.37.12",
        "2026-10-18 09.10.00",
        "2026-10-18 10.20.00",
        "2026-10-18 10.40.00",
    ]
    .map(String::from)
    .to_vec();
    let sync = HashMap::from([
        (ids[0].clone(), SyncBadge::Synced),
        (ids[1].clone(), SyncBadge::Remote),
        (ids[2].clone(), SyncBadge::Waiting),
        (ids[3].clone(), SyncBadge::Synced),
    ]);
    let shared = HashSet::from([ids[3].clone()]);
    d.set_gallery(&ids, &shared, &sync, "2026-10-18 11.00.00", (2026, 10, 18));
    let thumb: Vec<u8> = (0..160 * 100)
        .flat_map(|i| [(i % 160) as u8, 120, 200, 255])
        .collect();
    for id in ids.iter().chain(["2026-10-18 11.00.00".to_string()].iter()) {
        d.set_gallery_thumb(id, 160, 100, &thumb);
    }
    let (px, _) = d.render_window();
    common::save(&out, &common::image((1000, 760), px, AlphaType::Premul));
}
