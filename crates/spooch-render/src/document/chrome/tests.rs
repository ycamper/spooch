use super::super::tests::placed;
use super::*;
use crate::Cursor;

#[test]
fn the_rail_docks_left_then_right_then_inside() {
    let (mut d, _) = placed();
    // The canvas is at (100, 100) on a 400x300 desktop: room on the left.
    let rail = d.rail_layout().rect;
    assert!(rail.right <= 100 - 8, "{rail:?}");
    // Level with the canvas top, unless that would run off the screen.
    assert_eq!(rail.top, 100.min(300 - rail.height()).max(0));
    // Two monitors side by side with the canvas at the right one's left
    // edge: no room on its left, so the rail goes right of the canvas.
    d.set_screens(vec![
        IRect::from_ltrb(0, 0, 100, 300),
        IRect::from_ltrb(100, 0, 400, 300),
    ]);
    let rail = d.rail_layout().rect;
    assert!(rail.left >= 300, "{rail:?}");
    // No room either side: just inside the canvas.
    d.set_screens(vec![IRect::from_ltrb(100, 0, 300, 300)]);
    let rail = d.rail_layout().rect;
    assert!(rail.left >= 100 && rail.right <= 300, "{rail:?}");
    // Its top always on the screen, and all of it when it fits (the
    // test screen is shorter than the rail).
    assert!(rail.top >= 0, "{rail:?}");
    d.set_screens(vec![IRect::from_ltrb(100, -100, 300, 300)]);
    let rail = d.rail_layout().rect;
    assert!(rail.bottom <= 300 && rail.top >= -100, "{rail:?}");
}

#[test]
fn clicking_a_button_picks_the_tool_without_drawing() {
    let (mut d, _) = placed();
    let layout = d.rail_layout();
    let (tool, b) = layout.buttons[1];
    assert_eq!(tool, Tool::Box);
    let p = Rect::from(b).center();
    assert_eq!(d.cursor_at(p), Cursor::Arrow);
    assert!(!d.begin_drag(p));
    assert!(!d.is_dragging());
    assert_eq!(d.tool(), Tool::Box);
    assert_eq!(d.counts().to_string(), "no marks");
}

#[test]
fn hovering_shows_a_tooltip_and_highlights() {
    let (mut d, _) = placed();
    assert_eq!(d.chrome().len(), 4, "rail, style row, bar and tab");
    let (_, b) = d.rail_layout().buttons[3];
    let p = Rect::from(b).center();
    assert!(d.pointer_moved(p));
    assert!(!d.pointer_moved(p), "no change, no repaint");
    if d.typeface.is_some() {
        let panels = d.chrome();
        assert_eq!(panels.len(), 5, "rail, row, bar, tab and tooltip");
        assert!(panels[4].rect.left > panels[0].rect.right);
    }
    assert!(d.pointer_moved(Point::new(200.0, 150.0)));
    assert_eq!(d.chrome().len(), 4, "the tooltip goes");
}

#[test]
fn the_current_tool_is_red_and_corners_are_clear() {
    let (mut d, _) = placed();
    let layout = d.rail_layout();
    let rail = layout.rect;
    let (_, arrow) = layout.buttons[0];
    let panels = d.chrome().to_vec();
    let p = &panels[0];
    assert_eq!(p.rect, rail);
    let w = rail.width() as usize;
    let at = |x: i32, y: i32| {
        let i = ((y - rail.top) as usize * w + (x - rail.left) as usize) * 4;
        &p.pixels[i..i + 4]
    };
    // The top-left pixel is outside the rounded corner.
    assert_eq!(at(rail.left, rail.top)[3], 0);
    // Just inside the active button's corner is solid red (BGRA).
    let red = at(arrow.left + 2, arrow.top + 3);
    assert!(red[2] > 200 && red[1] < 100 && red[3] == 255, "{red:?}");
}

#[test]
fn the_style_row_sits_above_and_picks_colours_and_sizes() {
    let (mut d, _) = placed();
    let row = d.row_layout().expect("arrows have a style");
    assert!(row.rect.bottom <= 100 - 8, "{:?}", row.rect);
    assert_eq!(row.colours.len(), 8);
    let swatch = Rect::from(row.colours[3]).center();
    assert!(d.on_chrome(swatch));
    assert!(!d.begin_drag(swatch));
    assert_eq!(d.tool_style().color, 3);
    // Sliding to the left end picks the smallest size, as one step.
    let track = row.size.unwrap();
    let left = Point::new(track.left as f32 + 1.0, Rect::from(track).center_y());
    assert!(d.begin_drag(left), "a slider press starts a drag");
    assert!(!d.end_drag());
    assert_eq!(d.tool_style().size, 0);
    // Hovering a swatch shows its shortcut above the row.
    assert!(d.pointer_moved(swatch));
    if d.typeface.is_some() {
        let panels = d.chrome();
        assert_eq!(panels.len(), 5, "rail, row, bar, tab and tooltip");
        assert!(panels[4].rect.bottom <= panels[1].rect.top);
    }
    // Opacity slider and edge button sit after the size slider.
    let row = d.row_layout().unwrap();
    let (opacity, edge) = (row.opacity.unwrap(), row.edge.unwrap());
    assert!(opacity.left > track.right && edge.left > opacity.right);
    assert!(row.rect.right >= edge.right);
    assert!(!d.begin_drag(Rect::from(edge).center()));
    assert_eq!(
        d.tool_style().edge,
        0,
        "the edge button cycles from auto to white"
    );
    // Blur has strength only, the highlighter colours only, and solid
    // redact no row at all.
    d.set_tool(Tool::Blur);
    let row = d.row_layout().unwrap();
    assert!(row.strength && row.colours.is_empty() && row.opacity.is_none());
    let track = row.size.unwrap();
    let right = Point::new(track.right as f32 - 1.0, Rect::from(track).center_y());
    assert!(d.begin_drag(right));
    d.end_drag();
    assert_eq!(d.tool_style().size, 4);
    d.set_tool(Tool::Highlight);
    let row = d.row_layout().unwrap();
    assert!(row.size.is_none() && row.edge.is_none());
    d.set_tool(Tool::Redact);
    assert!(d.row_layout().is_none());
}

#[test]
fn dragging_a_slider_restyles_the_mark_as_one_undo_step() {
    let (mut d, _) = placed();
    assert!(super::super::tests::drag(
        &mut d,
        (120.0, 150.0),
        (280.0, 150.0)
    ));
    super::super::tests::click(&mut d, 200.0, 150.0);
    let track = d.row_layout().unwrap().opacity.unwrap();
    let y = Rect::from(track).center_y();
    let at = |t: f32| {
        let knob = KNOB * d.scale;
        let (x0, x1) = (track.left as f32 + knob, track.right as f32 - knob);
        Point::new(x0 + t * (x1 - x0), y)
    };
    let (right, mid, left) = (at(1.0), at(0.5), at(0.0));
    assert!(d.begin_drag(right));
    assert!(!d.drag_to(right), "no change at 100%");
    assert!(d.drag_to(mid));
    assert!(d.drag_to(left));
    assert!(d.end_drag());
    let crate::document::Mark::Arrow(a) = &d.marks[0] else {
        panic!("not an arrow")
    };
    assert!((a.look.opacity - 0.1).abs() < 0.001);
    assert_eq!(d.tool_style().opacity, 10);
    assert!(d.undo());
    let crate::document::Mark::Arrow(a) = &d.marks[0] else {
        panic!("not an arrow")
    };
    assert_eq!(a.look.opacity, 1.0, "one undo takes back the whole drag");
}

#[test]
fn the_move_tab_moves_the_selection_and_leaves_marks_alone() {
    let (mut d, desktop) = placed();
    assert!(super::super::tests::drag(
        &mut d,
        (120.0, 150.0),
        (280.0, 150.0)
    ));
    let before = d.canvas;
    let tab = d.tab_rect();
    // Centred on the bottom edge, half in and half out.
    assert!(tab.top < before.bottom && tab.bottom > before.bottom);
    let p = Rect::from(tab).center();
    assert_eq!(d.cursor_at(p), Cursor::Move);
    assert!(d.begin_drag(p), "a press on the tab starts a drag");
    // It asks for the whole desktop, as an edge drag does.
    assert!(d.wants_desktop().is_some());
    assert!(d.supply_desktop(400, 300, 1600, &desktop));
    assert!(d.drag_to(p + Point::new(-60.0, 40.0)));
    assert!(d.end_drag());
    assert_eq!(
        (d.canvas.left, d.canvas.top),
        (before.left - 60, before.top + 40)
    );
    assert_eq!(d.canvas.size(), before.size());
    let crate::document::Mark::Arrow(a) = &d.marks[0] else {
        panic!("not an arrow")
    };
    assert_eq!(
        a.from,
        Point::new(120.0, 150.0),
        "marks stay on the picture"
    );
    // Never off the pixels there are.
    let p = Rect::from(d.tab_rect()).center();
    d.begin_drag(p);
    d.drag_to(p + Point::new(-1000.0, 1000.0));
    d.end_drag();
    assert!(
        d.canvas.left >= 0 && d.canvas.bottom <= 300,
        "{:?}",
        d.canvas
    );
    // One undo per move.
    assert!(d.undo());
    assert!(d.undo());
    assert_eq!(d.canvas, before);
    // Shift+drag brings the marks along, as one undo step.
    d.set_modifiers(true, true);
    let p = Rect::from(d.tab_rect()).center();
    assert!(d.begin_drag(p));
    d.drag_to(p + Point::new(30.0, 20.0));
    d.end_drag();
    let crate::document::Mark::Arrow(a) = &d.marks[0] else {
        panic!("not an arrow")
    };
    assert_eq!(a.from, Point::new(150.0, 170.0));
    assert!(d.undo());
    let crate::document::Mark::Arrow(a) = &d.marks[0] else {
        panic!("not an arrow")
    };
    assert_eq!((a.from, d.canvas), (Point::new(120.0, 150.0), before));
}

#[test]
fn hovering_outlines_what_a_click_would_grab() {
    let (mut d, _) = placed();
    assert!(super::super::tests::drag(
        &mut d,
        (120.0, 150.0),
        (280.0, 150.0)
    ));
    assert!(d.pointer_moved(Point::new(200.0, 150.0)));
    assert_eq!(d.hover_mark, Some(super::super::HoverMark::Mark(0)));
    assert!(d.pointer_moved(Point::new(200.0, 120.0)));
    assert_eq!(d.hover_mark, None);
    // Selected, its handles light up instead.
    super::super::tests::click(&mut d, 200.0, 150.0);
    d.pointer_moved(Point::new(280.0, 150.0));
    assert!(matches!(
        d.hover_mark,
        Some(super::super::HoverMark::Grip(_))
    ));
    // Never in the exported image.
    d.pointer_moved(Point::new(200.0, 150.0));
    let with = d.export();
    d.pointer_moved(Point::new(110.0, 110.0));
    assert_eq!(d.export(), with);
}

#[test]
fn a_colour_click_keeps_typing_and_restyles_the_label() {
    let (mut d, _) = placed();
    d.begin_text(Point::new(150.0, 150.0));
    d.type_text("Hi");
    let swatch = Rect::from(d.row_layout().unwrap().colours[4]).center();
    d.begin_drag(swatch);
    assert!(d.is_typing(), "a colour pick doesn't end the label");
    assert!(d.type_text("!"));
    d.commit_text();
    assert_eq!(d.counts().labels, 1);
}
