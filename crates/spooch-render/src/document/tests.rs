use super::*;

fn doc() -> Document {
    let (w, h) = (200u32, 100u32);
    // White with alpha 0, the way X11 hands pixels back.
    let data: Vec<u8> = [255u8, 255, 255, 0].repeat((w * h) as usize);
    let mut d =
        with_room_for_chrome(Document::from_bgra(w, h, (w * 4) as usize, &data, 1.0).unwrap());
    // Tests place marks exactly; guides.rs's tests turn snapping on.
    d.set_modifiers(false, true);
    d
}

/// A screen around the canvas with room for the rail outside it, so the
/// rail doesn't sit over the points tests draw at.
fn with_room_for_chrome(mut d: Document) -> Document {
    d.set_screens(vec![IRect::from_ltrb(-400, -400, 800, 800)]);
    d
}

#[test]
fn short_drags_are_not_arrows() {
    let mut d = doc();
    d.begin_drag(Point::new(10.0, 10.0));
    d.drag_to(Point::new(12.0, 11.0));
    assert!(!d.end_drag());
    assert_eq!(d.counts().arrows, 0);
}

#[test]
fn draw_and_undo() {
    let mut d = doc();
    d.begin_drag(Point::new(10.0, 50.0));
    d.drag_to(Point::new(190.0, 50.0));
    assert!(d.end_drag());
    assert_eq!(d.counts().arrows, 1);
    assert!(d.undo());
    assert_eq!(d.counts().arrows, 0);
    assert!(!d.undo());
}

#[test]
fn export_contains_the_arrow() {
    let mut d = doc();
    let white = d.export();
    d.begin_drag(Point::new(10.0, 50.0));
    d.drag_to(Point::new(190.0, 50.0));
    d.end_drag();
    let drawn = d.export();
    assert_eq!(drawn.len(), 200 * 100 * 4);
    assert_ne!(white, drawn);
    // Mid-shaft is red: BGRA with high R, low G.
    let px = &drawn[(50 * 200 + 100) * 4..][..4];
    assert!(px[2] > 150 && px[1] < 100, "pixel was {px:?}");
    assert_eq!(px[3], 255);
}

fn has_red(px: &[u8]) -> bool {
    px.as_chunks::<4>()
        .0
        .iter()
        .any(|p| p[2] > 150 && p[1] < 100 && p[0] < 100)
}

#[test]
fn typed_label_is_drawn_and_exported() {
    let mut d = doc();
    if d.typeface.is_none() {
        eprintln!("no system fonts; skipping");
        return;
    }
    d.begin_text(Point::new(20.0, 50.0));
    assert!(d.type_text("Hi"));
    assert!(d.is_typing());
    // Export keeps the text and finishes the label.
    let out = d.export();
    assert!(!d.is_typing());
    assert_eq!(d.counts().labels, 1);
    assert!(has_red(&out));
}

#[test]
fn the_look_button_puts_labels_on_plates() {
    let mut d = doc();
    let Some(tf) = d.typeface.clone() else {
        return;
    };
    d.set_tool(Tool::Text);
    assert!(d.cycle_look() && d.cycle_look());
    d.begin_text(Point::new(20.0, 50.0));
    d.type_text("Hi");
    assert!(d.commit_text());
    let Some(Mark::Label(l)) = d.marks.last() else {
        panic!("no label")
    };
    assert_eq!(l.plate, crate::Plate::Tag);
    // The tag is wider than the bare text and filled red.
    let plated = l.bounds(&tf);
    let bare = Label {
        plate: crate::Plate::Outlined,
        ..l.clone()
    }
    .bounds(&tf);
    assert!(skia_safe::Contains::contains(&plated, bare) && plated.width() > bare.width());
    let out = d.export();
    let (x, y) = ((plated.left + 2.0) as usize, plated.center_y() as usize);
    let p = &out[(y * 200 + x) * 4..][..4];
    assert!(p[2] > 150 && p[1] < 100 && p[0] < 100, "tag {p:?}");
    // The plate survives saving, and the button restyles the label just
    // drawn.
    let (w, h, px) = d.base_pixels();
    let (back, _) = Document::from_saved(&d.to_json("x"), w, h, &px).unwrap();
    assert!(matches!(back.marks.last(), Some(Mark::Label(l)) if l.plate == crate::Plate::Tag));
    assert!(d.cycle_look());
    assert!(matches!(d.marks.last(), Some(Mark::Label(l)) if l.plate == crate::Plate::Badge));
}

#[test]
fn saved_looks_keep_and_bring_back_a_style() {
    let mut d = doc();
    d.set_tool(Tool::Box);
    let chip = |d: &Document, i: usize| Rect::from(d.row_layout().unwrap().quick[i]).center();
    let saved = |d: &Document| d.styles().quick[style::tool_index(Tool::Box)][0];
    // An empty chip keeps the look in use.
    d.set_colour(3);
    d.step_size(true);
    let kept = d.tool_style();
    d.begin_drag(chip(&d, 0));
    assert_eq!(saved(&d), Some(kept));
    // Later, the chip brings it back.
    d.set_colour(0);
    d.step_size(false);
    assert_ne!(d.tool_style(), kept);
    d.begin_drag(chip(&d, 0));
    assert_eq!(d.tool_style(), kept);
    // Shift+click saves over it.
    d.set_colour(5);
    d.set_modifiers(true, true);
    d.begin_drag(chip(&d, 0));
    d.set_modifiers(false, true);
    assert_eq!(saved(&d), Some(d.tool_style()));
    // Stickers have no saved looks.
    d.set_tool(Tool::Sticker);
    assert!(d.row_layout().unwrap().quick.is_empty());
}

#[test]
fn pixel_mode_draws_whole_cells_and_switches_back() {
    let mut d = doc();
    d.set_tool(Tool::Arrow);
    assert!(drag(&mut d, (20.0, 80.0), (180.0, 20.0)));
    let clean = d.export();
    d.set_mode(Mode::Pixel);
    let pixel = d.export();
    assert_ne!(pixel, clean);
    // Every 3x3 cell, on the grid from the capture's corner, is one colour.
    let at = |x: usize, y: usize| &pixel[(y * 200 + x) * 4..][..4];
    for (cx, cy) in (0..66).flat_map(|x| (0..33).map(move |y| (x, y))) {
        let first = at(cx * 3, cy * 3);
        for (dx, dy) in (0..3).flat_map(|x| (0..3).map(move |y| (x, y))) {
            assert_eq!(at(cx * 3 + dx, cy * 3 + dy), first, "cell {cx},{cy}");
        }
    }
    // Saved with the capture, and lossless back to clean.
    let (w, h, px) = d.base_pixels();
    let (back, _) = Document::from_saved(&d.to_json("x"), w, h, &px).unwrap();
    assert_eq!(back.mode(), Mode::Pixel);
    d.set_mode(Mode::Clean);
    assert_eq!(d.export(), clean);
}

#[test]
fn text_editing_and_undo() {
    let mut d = doc();
    d.begin_text(Point::new(20.0, 50.0));
    d.type_text("ab\u{8}c");
    assert!(d.backspace());
    assert!(d.commit_text());
    assert_eq!(d.counts().labels, 1);
    // Blank labels are dropped.
    d.begin_text(Point::new(20.0, 80.0));
    d.type_text("  ");
    assert_eq!(d.counts().labels, 1);
    assert!(!d.commit_text());
    assert_eq!(d.counts().labels, 1);
    // Undo while typing discards the label in progress first.
    d.begin_text(Point::new(20.0, 80.0));
    d.type_text("x");
    // A label in progress counts, since copying keeps it.
    assert_eq!(d.counts().labels, 2);
    assert!(d.undo());
    assert_eq!(d.counts().labels, 1);
    assert!(d.undo());
    assert_eq!(d.counts().labels, 0);
}

#[test]
fn starting_an_arrow_finishes_the_label() {
    let mut d = doc();
    d.begin_text(Point::new(20.0, 50.0));
    d.type_text("note");
    d.begin_drag(Point::new(10.0, 10.0));
    assert!(!d.is_typing());
    assert_eq!(d.counts().labels, 1);
}

#[test]
fn tools_draw_boxes_and_highlights() {
    let mut d = doc();
    assert_eq!(Tool::from_digit('2'), Some(Tool::Box));
    assert_eq!(Tool::from_digit('9'), Some(Tool::Steps));
    assert_eq!(Tool::from_digit('0'), Some(Tool::Redact));
    assert_eq!(Tool::from_digit('x'), None);
    assert!(d.set_tool(Tool::Box));
    d.begin_drag(Point::new(20.0, 20.0));
    d.drag_to(Point::new(120.0, 80.0));
    assert!(d.end_drag());
    d.set_tool(Tool::Highlight);
    d.begin_drag(Point::new(30.0, 40.0));
    d.drag_to(Point::new(180.0, 60.0));
    assert!(d.end_drag());
    // A thin sliver is a stray click, not a highlight.
    d.begin_drag(Point::new(30.0, 40.0));
    d.drag_to(Point::new(180.0, 42.0));
    assert!(!d.end_drag());
    let c = d.counts();
    assert_eq!((c.arrows, c.boxes, c.highlights), (0, 1, 1));

    let out = d.export();
    let at = |x: usize, y: usize| &out[(y * 200 + x) * 4..][..4];
    // The box edge is red; inside the box stays light (only the soft
    // shadow reaches in).
    assert!(
        has_red(at(20, 50)) || has_red(at(21, 50)),
        "edge {:?}",
        at(20, 50)
    );
    // The highlight is a spotlight: inside it stays white, and the rest,
    // inside the box too, dims.
    let lit = at(150, 50);
    assert!(lit[..3].iter().all(|&v| v >= 230), "spotlit {lit:?}");
    let dimmed = at(70, 30);
    assert!(dimmed[..3].iter().all(|&v| v < 150), "dimmed {dimmed:?}");
}

#[test]
fn the_look_button_picks_a_marker_and_it_is_kept() {
    let mut d = doc();
    d.set_tool(Tool::Highlight);
    assert!(d.cycle_look());
    assert!(drag(&mut d, (30.0, 40.0), (180.0, 60.0)));
    let Some(Mark::Highlight(h)) = d.marks.last() else {
        panic!("no highlight")
    };
    assert_eq!(h.look, crate::HighlightLook::Marker);
    // A marker turns white yellow: blue drops, red and green stay, and
    // nothing else dims.
    let out = d.export();
    let at = |x: usize, y: usize| &out[(y * 200 + x) * 4..][..4];
    let h = at(100, 50);
    assert!(h[0] < 100 && h[1] > 180 && h[2] > 200, "marker {h:?}");
    assert!(at(100, 20)[..3].iter().all(|&v| v >= 230));
    let (w, hh, px) = d.base_pixels();
    let (back, _) = Document::from_saved(&d.to_json("x"), w, hh, &px).unwrap();
    assert!(
        matches!(back.marks.last(), Some(Mark::Highlight(h)) if h.look == crate::HighlightLook::Marker)
    );
    // The other tools have no look button.
    d.set_tool(Tool::Arrow);
    assert!(!d.cycle_look());
}

#[test]
fn blur_and_pixelate_hide_detail_and_undo() {
    // A sharp black-and-white checkerboard of 2-pixel squares.
    let (w, h) = (200u32, 100u32);
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let v = if (x / 2 + y / 2) % 2 == 0 { 0 } else { 255 };
            data.extend_from_slice(&[v, v, v, 0]);
        }
    }
    let mut d =
        with_room_for_chrome(Document::from_bgra(w, h, (w * 4) as usize, &data, 1.0).unwrap());
    let sharp = d.export();
    let spread = |img: &[u8], x0: usize, x1: usize| {
        let row = |x: usize| img[(50 * 200 + x) * 4] as i32;
        let vals: Vec<i32> = (x0..x1).map(row).collect();
        vals.iter().max().unwrap() - vals.iter().min().unwrap()
    };
    assert_eq!(spread(&sharp, 20, 40), 255);

    assert_eq!(Tool::from_digit('4'), Some(Tool::Blur));
    assert_eq!(Tool::from_digit('5'), Some(Tool::Pixelate));
    d.set_tool(Tool::Blur);
    d.begin_drag(Point::new(10.0, 10.0));
    d.drag_to(Point::new(90.0, 90.0));
    assert!(d.end_drag());
    d.set_tool(Tool::Pixelate);
    d.begin_drag(Point::new(110.0, 10.0));
    d.drag_to(Point::new(190.0, 90.0));
    assert!(d.end_drag());
    let c = d.counts();
    assert_eq!((c.blurs, c.pixelates), (1, 1));

    let out = d.export();
    // Blur averages the checkerboard into a flat grey.
    assert!(
        spread(&out, 20, 40) < 40,
        "blur spread {}",
        spread(&out, 20, 40)
    );
    // Pixelate turns it into flat blocks: no change within one block.
    let block = (style::STRENGTH[2] * d.scale) as usize;
    let start = 110 + block;
    assert!(spread(&out, start, start + block - 1) < 10);
    // Undo removes the pixelate, then the blur.
    assert!(d.undo());
    assert!(d.undo());
    assert_eq!(d.export(), sharp);
}

pub(super) fn click(d: &mut Document, x: f32, y: f32) -> bool {
    d.begin_drag(Point::new(x, y));
    d.end_drag()
}

pub(super) fn drag(d: &mut Document, from: (f32, f32), to: (f32, f32)) -> bool {
    d.begin_drag(Point::new(from.0, from.1));
    d.drag_to(Point::new(to.0, to.1));
    d.end_drag()
}

fn box_rect(d: &Document, i: usize) -> Rect {
    d.marks[i].rect().unwrap()
}

#[test]
fn click_selects_and_drag_moves() {
    let mut d = doc();
    d.set_tool(Tool::Box);
    assert!(drag(&mut d, (20.0, 20.0), (120.0, 80.0)));
    assert!(!d.has_selection());
    // Clicking the outline selects; clicking inside the box does not.
    assert!(!click(&mut d, 70.0, 50.0));
    assert!(!d.has_selection());
    assert!(!click(&mut d, 45.0, 21.0));
    assert!(d.has_selection());
    // Dragging the selected box moves it, as one undo step.
    assert!(drag(&mut d, (45.0, 21.0), (65.0, 31.0)));
    assert_eq!(box_rect(&d, 0), Rect::from_ltrb(40.0, 30.0, 140.0, 90.0));
    assert_eq!(d.counts().boxes, 1);
    assert!(d.undo());
    assert_eq!(box_rect(&d, 0), Rect::from_ltrb(20.0, 20.0, 120.0, 80.0));
    assert!(d.redo());
    assert_eq!(box_rect(&d, 0), Rect::from_ltrb(40.0, 30.0, 140.0, 90.0));
    assert!(!d.redo());
    // Clicking empty canvas clears the selection.
    click(&mut d, 180.0, 95.0);
    assert!(!d.has_selection());
}

#[test]
fn dragging_an_unselected_mark_draws() {
    let mut d = doc();
    assert!(drag(&mut d, (10.0, 50.0), (190.0, 50.0)));
    // An arrow starting on the first one is a new arrow, not a move.
    assert!(drag(&mut d, (100.0, 50.0), (100.0, 90.0)));
    assert_eq!(d.counts().arrows, 2);
    assert!(!d.has_selection());
}

#[test]
fn handles_resize_and_reshape() {
    let mut d = doc();
    d.set_tool(Tool::Box);
    drag(&mut d, (20.0, 20.0), (120.0, 80.0));
    click(&mut d, 45.0, 21.0);
    // Bottom-right corner.
    assert!(drag(&mut d, (121.0, 79.0), (150.0, 95.0)));
    assert_eq!(box_rect(&d, 0), Rect::from_ltrb(20.0, 20.0, 150.0, 95.0));
    // The left edge dragged past the right one flips the box.
    assert!(drag(&mut d, (20.0, 57.5), (170.0, 57.0)));
    assert_eq!(box_rect(&d, 0), Rect::from_ltrb(150.0, 20.0, 170.0, 95.0));

    // An arrow's tip.
    let mut d = doc();
    drag(&mut d, (10.0, 50.0), (150.0, 50.0));
    click(&mut d, 80.0, 50.0);
    assert!(d.has_selection());
    assert!(drag(&mut d, (150.0, 50.0), (180.0, 90.0)));
    let Mark::Arrow(a) = &d.marks[0] else {
        panic!("not an arrow")
    };
    assert_eq!(
        (a.from, a.to),
        (Point::new(10.0, 50.0), Point::new(180.0, 90.0))
    );
}

#[test]
fn delete_redo_and_new_edits_clear_redo() {
    let mut d = doc();
    drag(&mut d, (10.0, 50.0), (190.0, 50.0));
    assert!(!d.delete_selected());
    click(&mut d, 100.0, 50.0);
    assert!(d.delete_selected());
    assert_eq!(d.counts().arrows, 0);
    assert!(d.undo());
    assert_eq!(d.counts().arrows, 1);
    assert!(d.redo());
    assert_eq!(d.counts().arrows, 0);
    assert!(d.undo());
    // A new edit after an undo drops the redo history.
    drag(&mut d, (10.0, 20.0), (190.0, 20.0));
    assert!(!d.redo());
    assert_eq!(d.counts().arrows, 2);
}

#[test]
fn labels_can_be_selected_and_moved() {
    let mut d = doc();
    if d.typeface.is_none() {
        eprintln!("no system fonts; skipping");
        return;
    }
    d.begin_text(Point::new(20.0, 50.0));
    d.type_text("Hello");
    d.commit_text();
    click(&mut d, 30.0, 50.0);
    assert!(d.has_selection());
    assert!(drag(&mut d, (30.0, 50.0), (60.0, 70.0)));
    let Mark::Label(l) = &d.marks[0] else {
        panic!("not a label")
    };
    assert_eq!(l.origin.x, 50.0);
}

#[test]
fn handles_are_not_exported() {
    let mut d = doc();
    d.set_tool(Tool::Box);
    drag(&mut d, (20.0, 20.0), (120.0, 80.0));
    let plain = d.export();
    click(&mut d, 45.0, 21.0);
    assert!(d.has_selection());
    // The on-screen render shows the handles...
    let (bytes, stride) = d.render();
    let at = |x: usize, y: usize| bytes[y * stride + x * 4..][..4].to_vec();
    assert_eq!(at(120, 80)[..3], [255, 255, 255]);
    assert_ne!(at(124, 80)[..3], [255, 255, 255]);
    // ...the exported image does not, and the selection survives.
    assert_eq!(d.export(), plain);
    assert!(d.has_selection());
}

#[test]
fn moving_a_blur_moves_what_it_hides() {
    let (w, h) = (200u32, 100u32);
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let v = if (x / 2 + y / 2) % 2 == 0 { 0 } else { 255 };
            data.extend_from_slice(&[v, v, v, 0]);
        }
    }
    let mut d =
        with_room_for_chrome(Document::from_bgra(w, h, (w * 4) as usize, &data, 1.0).unwrap());
    let spread = |img: &[u8], x0: usize, x1: usize| {
        let vals: Vec<i32> = (x0..x1).map(|x| img[(50 * 200 + x) * 4] as i32).collect();
        vals.iter().max().unwrap() - vals.iter().min().unwrap()
    };
    d.set_tool(Tool::Blur);
    drag(&mut d, (10.0, 10.0), (90.0, 90.0));
    click(&mut d, 50.0, 50.0);
    assert!(drag(&mut d, (50.0, 50.0), (150.0, 50.0)));
    let out = d.export();
    assert_eq!(spread(&out, 20, 40), 255);
    assert!(spread(&out, 120, 140) < 40);
}

#[test]
fn undo_mid_drag_puts_the_mark_back() {
    let mut d = doc();
    drag(&mut d, (10.0, 50.0), (190.0, 50.0));
    click(&mut d, 100.0, 50.0);
    d.begin_drag(Point::new(100.0, 50.0));
    d.drag_to(Point::new(100.0, 80.0));
    assert!(d.undo());
    assert!(!d.is_dragging());
    let Mark::Arrow(a) = &d.marks[0] else {
        panic!("not an arrow")
    };
    assert_eq!(a.from, Point::new(10.0, 50.0));
    assert_eq!(d.counts().arrows, 1);
}

/// A 400x300 desktop with a gradient (so every pixel says where it came
/// from), and a document over (100, 100)-(300, 200) of it.
pub(super) fn placed() -> (Document, Vec<u8>) {
    let (w, h) = (400u32, 300u32);
    let mut desktop = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            desktop.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, 0, 255]);
        }
    }
    let (x0, y0, cw, ch) = (100usize, 100usize, 200usize, 100usize);
    let mut part = Vec::with_capacity(cw * ch * 4);
    for y in y0..y0 + ch {
        part.extend_from_slice(&desktop[(y * w as usize + x0) * 4..][..cw * 4]);
    }
    let mut d = Document::from_bgra(cw as u32, ch as u32, cw * 4, &part, 1.0).unwrap();
    d.place(x0 as i32, y0 as i32, IRect::from_wh(w as i32, h as i32));
    // Tests place marks exactly; guides.rs's tests turn snapping on.
    d.set_modifiers(false, true);
    (d, desktop)
}

#[test]
fn the_wheel_sizes_and_space_moves_the_shape_being_drawn() {
    let (mut d, _) = placed();
    d.set_tool(Tool::Box);
    let at = Point::new(200.0, 150.0);
    let before = d.tool_style().size;
    // A touchpad's half notches add up to one step.
    assert!(!d.wheel(at, 0.5, false, false));
    assert!(d.wheel(at, 0.5, false, false));
    assert_eq!(d.tool_style().size, before + 1);
    assert!(d.wheel(at, -1.0, false, false));
    assert_eq!(d.tool_style().size, before);
    // NaN and infinity are ignored; a huge turn just goes to the end.
    assert!(!d.wheel(at, f32::NAN, false, false));
    assert!(!d.wheel(at, f32::INFINITY, false, false));
    assert!(d.wheel(at, -1.0, false, false));
    assert!(d.wheel(at, -1.0e9, false, false));
    assert_eq!(d.tool_style().size, 0);
    assert!(d.wheel(at, 2.0, false, false));
    assert_eq!(d.tool_style().size, before);
    // Mid-draw the wheel sizes the box being drawn, and Space moves it.
    d.begin_drag(Point::new(120.0, 120.0));
    d.drag_to(Point::new(150.0, 150.0));
    assert!(d.wheel(at, 1.0, false, false));
    let space = d.key(Key::Char(' '), Mods::default());
    assert_eq!(space, KeyOutcome::Handled { changed: false });
    d.drag_to(Point::new(170.0, 160.0));
    d.key_up(Key::Char(' '));
    d.drag_to(Point::new(200.0, 190.0));
    assert!(d.end_drag());
    assert_eq!(box_rect(&d, 0), Rect::from_ltrb(140.0, 130.0, 200.0, 190.0));
    assert_eq!(d.shown_style().unwrap().0.size, before + 1);
}

#[test]
fn edges_crop_and_extend_over_the_desktop() {
    let (mut d, desktop) = placed();
    assert_eq!(d.canvas(), IRect::from_ltrb(100, 100, 300, 200));
    // Points are desktop pixels: an arrow across the middle.
    assert!(drag(&mut d, (120.0, 150.0), (280.0, 150.0)));

    // Crop from the left edge, grabbed just outside it.
    assert_eq!(
        d.cursor_at(Point::new(97.0, 150.0)),
        Cursor::ResizeHorizontal
    );
    assert!(d.begin_drag(Point::new(97.0, 150.0)));
    // The document asks for the whole desktop once an edge moves.
    assert_eq!(d.wants_desktop(), Some(IRect::from_wh(400, 300)));
    assert!(d.drag_to(Point::new(147.0, 150.0)));
    assert!(d.end_drag());
    assert_eq!(d.canvas(), IRect::from_ltrb(150, 100, 300, 200));
    assert_eq!((d.width(), d.height()), (150, 100));
    // Without the desktop's pixels it can't grow past what it has.
    d.begin_drag(Point::new(302.0, 202.0));
    d.drag_to(Point::new(352.0, 252.0));
    d.end_drag();
    assert_eq!(d.canvas(), IRect::from_ltrb(150, 100, 300, 200));

    // With them, the bottom-right corner extends over the desktop.
    assert_eq!(
        d.cursor_at(Point::new(302.0, 202.0)),
        Cursor::ResizeDiagonalDown
    );
    d.begin_drag(Point::new(302.0, 202.0));
    assert!(d.wants_desktop().is_some());
    assert!(d.supply_desktop(400, 300, 400 * 4, &desktop));
    assert!(d.wants_desktop().is_none());
    d.drag_to(Point::new(352.0, 252.0));
    d.end_drag();
    assert_eq!(d.canvas(), IRect::from_ltrb(150, 100, 350, 250));

    // The export is the desktop's pixels under the canvas, with the
    // arrow where it was drawn.
    let out = d.export();
    assert_eq!(out.len(), 200 * 150 * 4);
    let at = |x: usize, y: usize| &out[(y * 200 + x) * 4..][..4];
    assert_eq!(at(180, 140)[..2], [(330 % 256) as u8, 240]);
    assert!(has_red(at(100, 50)), "arrow at {:?}", at(100, 50));

    // Each resize is one undo step, and redo puts it back.
    assert!(d.undo());
    assert_eq!(d.canvas(), IRect::from_ltrb(150, 100, 300, 200));
    assert!(d.undo());
    assert_eq!(d.canvas(), IRect::from_ltrb(100, 100, 300, 200));
    assert_eq!(d.counts().arrows, 1);
    assert!(d.redo());
    assert_eq!(d.canvas(), IRect::from_ltrb(150, 100, 300, 200));
}

#[test]
fn canvas_stays_inside_the_desktop_and_above_a_minimum() {
    let (mut d, desktop) = placed();
    d.begin_drag(Point::new(98.0, 98.0));
    d.supply_desktop(400, 300, 400 * 4, &desktop);
    d.drag_to(Point::new(-500.0, -500.0));
    d.end_drag();
    assert_eq!(d.canvas(), IRect::from_ltrb(0, 0, 300, 200));
    // The right edge dragged past the left one stops at the minimum.
    d.begin_drag(Point::new(302.0, 100.0));
    d.drag_to(Point::new(-100.0, 100.0));
    d.end_drag();
    assert_eq!(d.canvas(), IRect::from_ltrb(0, 0, MIN_CANVAS_PX, 200));
}

#[test]
fn presses_off_the_canvas_do_nothing() {
    let (mut d, _) = placed();
    assert_eq!(d.cursor_at(Point::new(20.0, 20.0)), Cursor::Arrow);
    assert!(!d.begin_drag(Point::new(20.0, 20.0)));
    assert!(!d.is_dragging());
    assert_eq!(d.cursor_at(Point::new(200.0, 150.0)), Cursor::Draw);
    // A selected mark shows the move cursor, its handles a resize one.
    d.set_tool(Tool::Box);
    drag(&mut d, (150.0, 120.0), (250.0, 180.0));
    click(&mut d, 175.0, 120.0);
    assert_eq!(d.cursor_at(Point::new(175.0, 120.0)), Cursor::Move);
    assert_eq!(
        d.cursor_at(Point::new(250.0, 120.0)),
        Cursor::ResizeDiagonalUp
    );
    assert_eq!(
        d.cursor_at(Point::new(200.0, 180.0)),
        Cursor::ResizeVertical
    );
}

#[test]
fn redactions_follow_the_desktop_when_the_base_grows() {
    let (mut d, desktop) = placed();
    d.set_tool(Tool::Pixelate);
    drag(&mut d, (110.0, 110.0), (190.0, 190.0));
    let before = d.export();
    // Supplying the desktop moves the base's origin; the pixelated
    // area must not move with it.
    d.begin_drag(Point::new(302.0, 150.0));
    d.supply_desktop(400, 300, 400 * 4, &desktop);
    d.end_drag();
    let after = d.export();
    // Filtering may now pull in a pixel or two from beyond the area's
    // edge, which shifts colours slightly; a moved area would differ by
    // tens of levels everywhere.
    let worst = before
        .iter()
        .zip(&after)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    assert!(worst <= 6, "pixelate moved: worst difference {worst}");
}

#[test]
fn jpeg_encodes() {
    let (w, h) = (64u32, 32u32);
    let bgra = [40u8, 80, 200, 255].repeat((w * h) as usize);
    let jpg = crate::encode_jpeg(w, h, &bgra, 90).unwrap();
    assert_eq!(&jpg[..3], &[0xff, 0xd8, 0xff]);
    assert_eq!(Format::Png.next(), Format::Jpeg);
    assert_eq!(Format::Jpeg.extension(), "jpg");
}

#[test]
fn lines_ellipses_and_pen_strokes() {
    let mut d = doc();
    d.set_tool(Tool::Line);
    assert!(drag(&mut d, (10.0, 20.0), (190.0, 20.0)));
    d.set_tool(Tool::Ellipse);
    assert!(drag(&mut d, (20.0, 40.0), (120.0, 90.0)));
    d.set_tool(Tool::Pen);
    d.begin_drag(Point::new(140.0, 50.0));
    for i in 1..=20 {
        d.drag_to(Point::new(
            140.0 + i as f32 * 2.0,
            50.0 + (i as f32 * 0.5).sin() * 10.0,
        ));
    }
    assert!(d.end_drag());
    // A pen click is a stray click, not a dot.
    assert!(!click(&mut d, 60.0, 65.0));
    let c = d.counts();
    assert_eq!((c.lines, c.ellipses, c.strokes), (1, 1, 1));
    assert_eq!(c.to_string(), "1 line, 1 ellipse, 1 pen stroke");

    let out = d.export();
    let at = |x: usize, y: usize| &out[(y * 200 + x) * 4..][..4];
    assert!(has_red(at(100, 20)), "line {:?}", at(100, 20));
    // The ellipse's leftmost point is red; its middle is untouched.
    assert!(has_red(at(20, 65)) || has_red(at(21, 65)), "ellipse edge");
    assert!(at(70, 65)[..3].iter().all(|&v| v >= 230), "ellipse inside");

    // Each can be selected and moved; the line by its end too.
    click(&mut d, 100.0, 20.0);
    assert!(drag(&mut d, (190.0, 20.0), (190.0, 30.0)));
    let Mark::Line(l) = &d.marks[0] else {
        panic!("not a line")
    };
    assert_eq!(l.to, Point::new(190.0, 30.0));
    // On the curve, away from the handles at the box's corners and
    // edge midpoints.
    click(&mut d, 105.4, 47.3);
    assert!(d.has_selection());
    assert!(drag(&mut d, (105.4, 47.3), (115.4, 47.3)));
    assert_eq!(box_rect(&d, 1), Rect::from_ltrb(30.0, 40.0, 130.0, 90.0));
    click(&mut d, 160.0, 50.0 + (5.0f32).sin() * 10.0);
    assert!(d.has_selection());
    assert!(d.delete_selected());
    assert_eq!(d.counts().strokes, 0);
}

#[test]
fn steps_count_up_and_renumber() {
    let mut d = doc();
    d.set_tool(Tool::Steps);
    assert!(click(&mut d, 30.0, 50.0));
    assert!(click(&mut d, 100.0, 50.0));
    assert!(click(&mut d, 170.0, 50.0));
    assert_eq!(d.counts().steps, 3);
    assert_eq!(d.step_numbers(), vec![1, 2, 3]);
    // Clicking an existing step selects it instead of adding one.
    assert!(!click(&mut d, 100.0, 50.0));
    assert!(d.has_selection());
    assert_eq!(d.counts().steps, 3);
    // Deleting the middle one renumbers the last.
    assert!(d.delete_selected());
    assert_eq!(d.step_numbers(), vec![1, 2]);
    // Steps are drawn as filled circles in the annotation colour.
    let out = d.export();
    assert!(has_red(&out[(50 * 200 + 30 - 10) * 4..][..4]));
}

#[test]
fn redact_is_solid_black_and_hides_everything() {
    let mut d = doc();
    d.set_tool(Tool::Redact);
    assert!(drag(&mut d, (20.0, 20.0), (80.0, 60.0)));
    assert_eq!(d.counts().redactions, 1);
    let out = d.export();
    for y in 20..60 {
        for x in 20..80 {
            let px = &out[(y * 200 + x) * 4..][..3];
            assert_eq!(px, [0, 0, 0], "at ({x}, {y})");
        }
    }
    // Like the other redactions it can be moved, and it moves what it
    // hides.
    click(&mut d, 50.0, 40.0);
    assert!(drag(&mut d, (50.0, 40.0), (150.0, 40.0)));
    let out = d.export();
    assert_eq!(out[(40 * 200 + 50) * 4..][..3], [255, 255, 255]);
    assert_eq!(out[(40 * 200 + 150) * 4..][..3], [0, 0, 0]);
}

#[test]
fn export_drops_an_unfinished_arrow() {
    let mut d = doc();
    let white = d.export();
    d.begin_drag(Point::new(10.0, 50.0));
    d.drag_to(Point::new(190.0, 50.0));
    assert_eq!(d.export(), white);
}

#[test]
fn the_middle_handle_bends_arrows_and_lines() {
    let (mut d, _) = tests::placed();
    assert!(tests::drag(&mut d, (120.0, 150.0), (280.0, 150.0)));
    tests::click(&mut d, 160.0, 150.0);
    // The bend handle sits in the middle, with a hand cursor.
    let mid = Point::new(200.0, 150.0);
    assert_eq!(d.cursor_at(mid), Cursor::Hand);
    assert!(d.begin_drag(mid));
    d.drag_to(Point::new(200.0, 120.0));
    assert!(d.end_drag());
    let Mark::Arrow(a) = &d.marks[0] else {
        panic!("not an arrow")
    };
    // Up the screen is to the left of a left-to-right arrow: negative.
    assert!((a.bend + 30.0).abs() < 0.01, "{}", a.bend);
    // The curve, not the old straight line, is what clicks hit.
    assert!(d.marks[0].hit(Point::new(200.0, 120.0), 2.0, None));
    assert!(!d.marks[0].hit(Point::new(200.0, 150.0), 2.0, None));
    // Moving an end keeps the bend.
    let tip = Point::new(280.0, 150.0);
    d.begin_drag(tip);
    d.drag_to(Point::new(280.0, 170.0));
    d.end_drag();
    let Mark::Arrow(a) = &d.marks[0] else {
        panic!("not an arrow")
    };
    let a = *a;
    assert!((a.bend + 30.0).abs() < 0.01);
    // Saved and loaded, it's still bent.
    let json = d.to_json("bent");
    let (w, h, pixels) = d.base_pixels();
    let (back, _) = Document::from_saved(&json, w, h, &pixels).unwrap();
    let Mark::Arrow(b) = &back.marks[0] else {
        panic!("not an arrow")
    };
    assert!((b.bend + 30.0).abs() < 0.01);
    // Dragged back near the straight line, it snaps straight.
    let handle = crate::bend_point(a.from, a.to, a.bend);
    d.begin_drag(handle);
    d.drag_to(crate::bend_point(a.from, a.to, 3.0));
    d.end_drag();
    let Mark::Arrow(a) = &d.marks[0] else {
        panic!("not an arrow")
    };
    assert_eq!(a.bend, 0.0);
}

#[test]
fn the_corner_handle_rounds_boxes() {
    let (mut d, _) = tests::placed();
    d.set_tool(Tool::Box);
    assert!(tests::drag(&mut d, (120.0, 110.0), (280.0, 190.0)));
    tests::click(&mut d, 120.0, 150.0);
    let Mark::Box(b) = d.marks[0].clone() else {
        panic!("not a box")
    };
    let handle = Point::new(
        120.0 + ROUND_INSET + b.corner_radius(),
        110.0 + ROUND_INSET + b.corner_radius(),
    );
    assert_eq!(d.cursor_at(handle), Cursor::Hand);
    // Dragging it in rounds the corners.
    assert!(d.begin_drag(handle));
    d.drag_to(Point::new(
        120.0 + ROUND_INSET + 25.0,
        110.0 + ROUND_INSET + 25.0,
    ));
    assert!(d.end_drag());
    let Mark::Box(b) = &d.marks[0] else {
        panic!("not a box")
    };
    assert_eq!(b.radius, Some(25.0));
    // Never rounder than half the short side.
    let handle = Point::new(120.0 + ROUND_INSET + 25.0, 110.0 + ROUND_INSET + 25.0);
    d.begin_drag(handle);
    d.drag_to(Point::new(250.0, 250.0));
    d.end_drag();
    let Mark::Box(b) = &d.marks[0] else {
        panic!("not a box")
    };
    assert_eq!(b.corner_radius(), 40.0);
    // Back to the corner, it snaps square.
    let r = b.corner_radius();
    d.begin_drag(Point::new(120.0 + ROUND_INSET + r, 110.0 + ROUND_INSET + r));
    d.drag_to(Point::new(120.0 + ROUND_INSET, 110.0 + ROUND_INSET));
    d.end_drag();
    let Mark::Box(b) = &d.marks[0] else {
        panic!("not a box")
    };
    assert_eq!(b.radius, Some(0.0));
    assert!(d.undo());
}

fn rot_handle(d: &Document) -> Point {
    let i = d.selected.unwrap();
    grips_of(&d.marks[i], d.typeface.as_ref(), d.scale)
        .into_iter()
        .find(|(g, _)| *g == Grip::Rotate)
        .unwrap()
        .1
}

#[test]
fn boxes_turn_with_the_rotate_handle() {
    let (mut d, _) = tests::placed();
    d.set_tool(Tool::Box);
    assert!(tests::drag(&mut d, (140.0, 120.0), (260.0, 180.0)));
    tests::click(&mut d, 140.0, 150.0);
    // The handle sits on a stick above the middle of the top edge.
    let h = rot_handle(&d);
    assert_eq!(h, Point::new(200.0, 120.0 - ROTATE_STICK));
    assert_eq!(d.cursor_at(h), Cursor::Hand);
    // Dragging it round to the right turns the box a quarter turn.
    assert!(d.begin_drag(h));
    d.drag_to(Point::new(300.0, 151.0));
    assert!(d.end_drag());
    let Mark::Box(b) = &d.marks[0] else {
        panic!("not a box")
    };
    assert!(
        (b.angle.to_degrees() - 90.0).abs() < 0.01,
        "{}",
        b.angle.to_degrees()
    );
    // Turned on its side, its old top edge is now on the right: a click
    // there hits it, and the old left edge (now the top) does too.
    assert!(d.marks[0].hit(Point::new(230.0, 150.0), 2.0, None));
    assert!(d.marks[0].hit(Point::new(200.0, 90.0), 2.0, None));
    assert!(!d.marks[0].hit(Point::new(140.0, 150.0), 2.0, None));
    // Shift snaps to 15 degrees.
    d.set_modifiers(true, true);
    let h = rot_handle(&d);
    assert!(d.begin_drag(h));
    d.drag_to(Point::new(200.0 + 100.0 * 0.5, 150.0 - 100.0 * 0.86));
    d.end_drag();
    d.set_modifiers(false, true);
    let Mark::Box(b) = &d.marks[0] else {
        panic!("not a box")
    };
    assert!(
        (b.angle.to_degrees() - 30.0).abs() < 0.01,
        "{}",
        b.angle.to_degrees()
    );
    // A click on the handle without turning stands it upright.
    let h = rot_handle(&d);
    d.begin_drag(h);
    assert!(d.end_drag());
    let Mark::Box(b) = &d.marks[0] else {
        panic!("not a box")
    };
    assert_eq!(b.angle, 0.0);
    // And each turn is one undo step.
    assert!(d.undo());
    let Mark::Box(b) = &d.marks[0] else {
        panic!("not a box")
    };
    assert!((b.angle.to_degrees() - 30.0).abs() < 0.01);
}

#[test]
fn a_turned_box_resizes_along_its_own_sides() {
    let (mut d, _) = tests::placed();
    d.set_tool(Tool::Box);
    assert!(tests::drag(&mut d, (170.0, 135.0), (230.0, 165.0)));
    tests::click(&mut d, 170.0, 150.0);
    d.begin_drag(rot_handle(&d));
    d.drag_to(Point::new(300.0, 150.0));
    d.end_drag();
    // On its side the box spans y 120..180 on screen. Its own right edge
    // (x 2, y 1) is now at the bottom: drag it down 20.
    let edge = grips_of(&d.marks[0], None, d.scale)
        .into_iter()
        .find(|(g, _)| *g == Grip::Edge { x: 2, y: 1 })
        .unwrap()
        .1;
    assert!(
        (edge - Point::new(200.0, 180.0)).length() < 0.01,
        "{edge:?}"
    );
    d.begin_drag(edge);
    d.drag_to(edge + Point::new(0.0, 10.0));
    d.end_drag();
    let Mark::Box(b) = &d.marks[0] else {
        panic!("not a box")
    };
    // 10 longer along its own width, and its far end didn't move.
    assert!((b.rect.width() - 70.0).abs() < 0.01, "{:?}", b.rect);
    let far = rotate_about(
        Point::new(b.rect.left, b.rect.center_y()),
        b.angle,
        b.rect.center(),
    );
    assert!((far - Point::new(200.0, 120.0)).length() < 0.01, "{far:?}");
}

#[test]
fn shift_on_a_corner_keeps_the_proportions() {
    let (mut d, _) = tests::placed();
    d.set_tool(Tool::Ellipse);
    assert!(tests::drag(&mut d, (140.0, 120.0), (240.0, 170.0)));
    tests::click(&mut d, 140.0, 145.0);
    d.set_modifiers(true, true);
    d.begin_drag(Point::new(240.0, 170.0));
    d.drag_to(Point::new(290.0, 175.0));
    d.end_drag();
    let Mark::Oval(o) = &d.marks[0] else {
        panic!("not an ellipse")
    };
    assert_eq!((o.rect.width(), o.rect.height()), (150.0, 75.0));
}

#[test]
fn duplicate_and_restack() {
    let (mut d, _) = tests::placed();
    d.set_tool(Tool::Box);
    assert!(tests::drag(&mut d, (120.0, 110.0), (180.0, 150.0)));
    assert!(tests::drag(&mut d, (200.0, 110.0), (260.0, 150.0)));
    tests::click(&mut d, 120.0, 130.0);
    // Ctrl+D: a copy, a little down and right, selected.
    assert!(d.duplicate_selected());
    assert_eq!(d.marks.len(), 3);
    assert_eq!(d.selection(), vec![2]);
    let Mark::Box(b) = &d.marks[2] else {
        panic!("not a box")
    };
    assert_eq!(b.rect.left, 120.0 + DUPLICATE_OFFSET);
    // Ctrl+[ sends it to the back; the selection follows.
    assert!(d.restack_selected(false));
    assert_eq!(d.selection(), vec![0]);
    let Mark::Box(b) = &d.marks[0] else {
        panic!("not a box")
    };
    assert_eq!(b.rect.left, 120.0 + DUPLICATE_OFFSET);
    assert!(!d.restack_selected(false), "already at the back");
    // Ctrl+] brings it to the front again.
    assert!(d.restack_selected(true));
    assert_eq!(d.selection(), vec![2]);
    // Each is one undo step.
    assert!(d.undo());
    assert!(d.undo());
    assert!(d.undo());
    assert_eq!(d.marks.len(), 2);
}

#[test]
fn labels_bend_by_their_handle_and_wrap_round_circles() {
    let (w, h) = (600u32, 500u32);
    let data = [255u8, 255, 255, 255].repeat((w * h) as usize);
    let mut d =
        with_room_for_chrome(Document::from_bgra(w, h, (w * 4) as usize, &data, 1.0).unwrap());
    d.set_modifiers(false, false);
    let tf = d.typeface.clone().unwrap();
    let last = |d: &Document| match d.marks.last() {
        Some(Mark::Label(l)) => l.clone(),
        _ => panic!("no label"),
    };
    let type_label = |d: &mut Document, at: Point, text: &str| {
        d.set_tool(Tool::Text);
        d.begin_text(at);
        d.type_text(text);
        d.commit_text();
        d.set_tool(Tool::Arrow);
    };
    let pull = |d: &mut Document, l: &Label, by: f32| {
        let m = l.middle(&tf);
        if !d.has_selection() {
            click(d, m.x, m.y - 8.0);
        }
        assert!(d.has_selection());
        let h = grips::label_bend_handle(l, &tf);
        assert!(drag(d, (h.x, h.y), (h.x, h.y + by)));
    };

    type_label(&mut d, Point::new(60.0, 60.0), "HELLO THERE");
    let straight = last(&d);
    // Down: a smile, its middle following the handle.
    pull(&mut d, &straight, 30.0);
    let smile = last(&d);
    assert!(smile.bend > 0.0);
    assert!((smile.middle(&tf).y - straight.middle(&tf).y - 30.0).abs() < 0.5);
    // Kept in a saved file.
    let (bw, bh, px) = d.base_pixels();
    let (back, _) = Document::from_saved(&d.to_json("x"), bw, bh, &px).unwrap();
    assert!(matches!(back.marks.last(), Some(Mark::Label(l)) if l.bend == smile.bend));
    // Back to within a few pixels of straight: straight.
    pull(&mut d, &smile, -28.0);
    assert_eq!(last(&d).bend, 0.0);
    // A click on the handle straightens it again.
    let now = last(&d);
    pull(&mut d, &now, 25.0);
    assert!(last(&d).bend > 0.0);
    let h = grips::label_bend_handle(&last(&d), &tf);
    click(&mut d, h.x, h.y);
    assert_eq!(last(&d).bend, 0.0);
    assert!((last(&d).middle(&tf).y - straight.middle(&tf).y).abs() < 0.5);
    // Twice the size, the same shape twice as big.
    let mut big = smile.clone();
    big.size *= 2.0;
    big.bend /= 2.0;
    let (a, b) = (smile.bounds(&tf), big.bounds(&tf));
    assert!((b.width() / a.width() - 2.0).abs() < 0.05);

    // A label just above a circle, bent up about as far as the circle
    // curves, wraps round it, centred.
    d.set_tool(Tool::Ellipse);
    assert!(drag(&mut d, (300.0, 200.0), (500.0, 400.0)));
    type_label(&mut d, Point::new(330.0, 160.0), "ON TOP");
    let on_top = last(&d);
    pull(&mut d, &on_top, -12.0);
    let wrapped = last(&d);
    assert!(wrapped.bend < 0.0);
    assert!((wrapped.middle(&tf).x - 400.0).abs() < 0.5);
    let radius = -1.0 / wrapped.bend;
    assert!((radius - (wrapped.middle(&tf).y - 300.0).abs()).abs() < 0.5);
    assert!(radius > 100.0 && radius < 120.0, "{radius}");
}

#[test]
fn a_marker_over_a_dark_page_lightens_it() {
    let (w, h) = (200u32, 100u32);
    let data = [30u8, 30, 30, 255].repeat((w * h) as usize);
    let mut d =
        with_room_for_chrome(Document::from_bgra(w, h, (w * 4) as usize, &data, 1.0).unwrap());
    d.set_modifiers(false, true);
    d.set_tool(Tool::Highlight);
    assert!(d.cycle_look());
    assert!(drag(&mut d, (180.0, 60.0), (30.0, 40.0)));
    let out = d.export();
    // BGRA: yellow ink screened in lifts red and green well above the page.
    let p = &out[(50 * 200 + 100) * 4..][..4];
    assert!(p[2] > 120 && p[1] > 100 && p[0] < p[2], "marker {p:?}");
}

#[test]
fn stickers_stamp_size_swap_and_keep() {
    let (w, h) = (600u32, 400u32);
    let data = [255u8, 255, 255, 255].repeat((w * h) as usize);
    let mut d =
        with_room_for_chrome(Document::from_bgra(w, h, (w * 4) as usize, &data, 1.0).unwrap());
    d.set_modifiers(false, true);
    let last = |d: &Document| match d.marks.last() {
        Some(Mark::Sticker(s)) => *s,
        _ => panic!("no sticker"),
    };
    d.set_tool(Tool::Sticker);
    // A click stamps the sticker picked last, centred, at the standard size.
    assert_eq!(d.sticker(), "thumbs_up");
    click(&mut d, 150.0, 150.0);
    let s = last(&d);
    assert_eq!(s.name, "thumbs_up");
    assert_eq!(s.rect, Rect::from_xywh(102.0, 102.0, 96.0, 96.0));
    // A drag sizes it, square, from the press.
    assert!(drag(&mut d, (300.0, 100.0), (420.0, 160.0)));
    assert_eq!(last(&d).rect, Rect::from_xywh(300.0, 100.0, 120.0, 120.0));
    assert_eq!(d.counts().stickers, 2);

    // Picking one moves it to the front of the row, and the tool stamps it.
    let fire = crate::STICKERS
        .iter()
        .position(|(n, _)| *n == "fire")
        .unwrap();
    let rocket = crate::STICKERS
        .iter()
        .position(|(n, _)| *n == "rocket")
        .unwrap();
    assert!(d.pick_sticker(rocket));
    assert_eq!(d.recent_stickers()[..3], ["rocket", "thumbs_up", "fire"]);
    assert!(d.pick_sticker(fire));
    assert_eq!(d.recent_stickers()[..3], ["fire", "rocket", "thumbs_up"]);
    // With a sticker selected, picking swaps its picture, as one undo step.
    d.set_tool(Tool::Arrow);
    click(&mut d, 360.0, 160.0);
    assert!(d.has_selection());
    assert!(d.pick_sticker(rocket));
    assert_eq!(last(&d).name, "rocket");
    assert!(d.undo());
    assert_eq!(last(&d).name, "thumbs_up");

    // A corner keeps it square.
    click(&mut d, 360.0, 160.0);
    assert!(drag(&mut d, (420.0, 220.0), (460.0, 230.0)));
    let r = last(&d).rect;
    assert!(
        (r.width() - r.height()).abs() < 0.01 && r.width() > 120.0,
        "{r:?}"
    );

    // Kept in a saved file, picture and all.
    let (bw, bh, px) = d.base_pixels();
    let (back, _) = Document::from_saved(&d.to_json("x"), bw, bh, &px).unwrap();
    assert!(
        matches!(back.marks.last(), Some(Mark::Sticker(s)) if s.name == "thumbs_up" && s.rect == r)
    );
}

#[test]
fn escape_on_a_blank_label_repaints_to_drop_the_caret() {
    let mut d = doc();
    d.begin_text(Point::new(20.0, 50.0));
    d.type_text("  ");
    let out = d.key(Key::Escape, Mods::default());
    assert!(matches!(out, KeyOutcome::Handled { changed: true }));
    assert!(!d.is_typing());
    assert_eq!(d.counts().labels, 0);
}
