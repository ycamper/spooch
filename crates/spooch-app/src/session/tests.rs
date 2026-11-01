use std::sync::mpsc;

use spooch_render::{Format, GalleryAction};

use super::*;

const NONE: Mods = Mods {
    ctrl: false,
    shift: false,
    alt: false,
};

fn when(second: u8) -> LocalTime {
    LocalTime {
        year: 2026,
        month: 10,
        day: 4,
        hour: 12,
        minute: 0,
        second,
    }
}

/// A 400x300 desktop with a 200x100 selection at (100, 100).
fn session(recents: Option<Recents>, second: u8) -> (Session, Vec<u8>) {
    let (w, h) = (400u32, 300u32);
    let mut desktop = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            desktop.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, 0, 255]);
        }
    }
    let mut part = Vec::new();
    for y in 100..200usize {
        part.extend_from_slice(&desktop[(y * w as usize + 100) * 4..][..200 * 4]);
    }
    let mut doc = Document::from_bgra(200, 100, 800, &part, 1.0).unwrap();
    doc.place(100, 100, IRect::from_wh(w as i32, h as i32));
    let screens = vec![IRect::from_wh(w as i32, h as i32)];
    let s = Session::new(
        doc,
        "Spooch test".into(),
        recents,
        &when(second),
        screens,
        Host::Overlay,
    );
    (s, desktop)
}

fn pt(x: f32, y: f32) -> Point {
    Point::new(x, y)
}

#[test]
fn drawing_captures_the_mouse_and_edges_ask_for_the_desktop() {
    let (mut s, desktop) = session(None, 0);
    let r = s.press(pt(150.0, 150.0), NONE);
    assert_eq!(r.effects, vec![Effect::CaptureMouse]);
    assert!(s.motion(pt(250.0, 150.0), (4.0, 4.0), NONE).repaint);
    assert!(s.release(pt(250.0, 150.0)).repaint);
    assert_eq!(s.doc.counts().arrows, 1);
    // The right edge: the whole desktop is wanted, and once supplied
    // the edge can go past the selection.
    let r = s.press(pt(300.0, 150.0), NONE);
    assert_eq!(
        r.effects,
        vec![
            Effect::NeedDesktop(IRect::from_wh(400, 300)),
            Effect::CaptureMouse
        ]
    );
    let line = s.supply_desktop((400, 300, 1600), &desktop, 1.5);
    assert!(line.unwrap().contains("400x300"));
    assert!(s.supply_desktop((400, 300, 1600), &desktop, 1.5).is_none());
    s.motion(pt(350.0, 150.0), (4.0, 4.0), NONE);
    s.release(pt(350.0, 150.0));
    assert_eq!(s.doc.canvas().right, 350);
}

#[test]
fn an_edit_lets_the_shared_link_go_but_picking_a_tool_doesnt() {
    let (mut s, _) = session(None, 0);
    s.link = Some(("https://img.test/x".into(), s.doc.look()));
    s.key(Key::Char('2'), NONE);
    assert!(s.link.is_some());
    // Drawing a box changes it: Share uploads again.
    s.press(pt(150.0, 150.0), NONE);
    s.motion(pt(250.0, 180.0), (4.0, 4.0), NONE);
    assert!(s.release(pt(250.0, 180.0)).repaint);
    assert!(s.link.is_none());
}

#[test]
fn a_shared_capture_s_marks_still_move() {
    for shared in [false, true] {
        let (mut s, _) = session(None, 0);
        s.key(Key::Char('2'), NONE);
        s.press(pt(120.0, 120.0), NONE);
        s.motion(pt(200.0, 170.0), (4.0, 4.0), NONE);
        s.release(pt(200.0, 170.0));
        if shared {
            s.link = Some(("https://img.test/x".into(), s.doc.look()));
            s.doc.set_share_label(Some("Shared \u{b7} link copied"));
        }
        // Click to select it, then drag it down.
        s.press(pt(160.0, 120.0), NONE);
        s.release(pt(160.0, 120.0));
        let before = s.doc.look();
        s.press(pt(160.0, 120.0), NONE);
        s.motion(pt(160.0, 140.0), (4.0, 4.0), NONE);
        s.release(pt(160.0, 140.0));
        assert_ne!(s.doc.look(), before, "shared: {shared}");
    }
}

#[test]
fn ctrl_drag_drags_out_past_the_threshold() {
    let (mut s, _) = session(None, 0);
    let ctrl = Mods { ctrl: true, ..NONE };
    assert_eq!(
        s.press(pt(150.0, 150.0), ctrl).effects,
        vec![Effect::CaptureMouse]
    );
    assert_eq!(s.cursor(pt(150.0, 150.0)), Cursor::Hand);
    assert!(
        s.motion(pt(152.0, 151.0), (4.0, 4.0), NONE)
            .effects
            .is_empty()
    );
    assert_eq!(
        s.motion(pt(160.0, 150.0), (4.0, 4.0), NONE).effects,
        vec![Effect::StartDragOut]
    );
    let dir = std::env::temp_dir().join(format!("spooch-app-drag-{}", std::process::id()));
    let (path, bytes) = s.drag_file(&dir).unwrap();
    assert_eq!(&bytes[1..4], b"PNG");
    assert!(path.ends_with("Spooch test.png"));
    assert!(std::fs::metadata(&path).unwrap().len() > 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keys_deliver_cancel_or_are_left_for_typing() {
    let (mut s, _) = session(None, 0);
    let enter = s.key(Key::Enter, NONE);
    assert_eq!(
        enter.effects,
        vec![Effect::Deliver {
            save: false,
            close: true
        }]
    );
    assert!(enter.used);
    assert_eq!(s.key(Key::Escape, NONE).effects, vec![Effect::Cancel]);
    let h = s.key(Key::Char('h'), NONE);
    assert!(!h.used);
    assert!(s.type_char('H', pt(150.0, 150.0)).repaint);
    // The window stays open: a copy doesn't close it, Esc does nothing.
    s.host = Host::Window;
    s.doc.commit_text();
    assert_eq!(
        s.key(Key::Enter, NONE).effects,
        vec![Effect::Deliver {
            save: false,
            close: false
        }]
    );
    assert!(s.key(Key::Escape, NONE).effects.is_empty());
}

#[test]
fn the_window_copies_selected_marks_and_ctrl_v_pastes_them() {
    let (overlay, _) = session(None, 0);
    let ctrl = Mods { ctrl: true, ..NONE };
    let mut overlay = overlay;
    assert_eq!(
        overlay.key(Key::Char('e'), ctrl).effects,
        vec![Effect::OpenWindow]
    );
    let mut s = overlay.into_window();
    assert_eq!(s.host(), Host::Window);
    s.resize(1200, 900);
    // Draw an arrow in window pixels (the capture sits mid-area, right of
    // the gallery and rail), then select it.
    let a = s.doc.render_window().1;
    assert_eq!(a, 1200 * 4);
    let from = pt(690.0, 450.0);
    s.press(from, NONE);
    s.motion(pt(770.0, 450.0), (4.0, 4.0), NONE);
    s.release(pt(770.0, 450.0));
    assert_eq!(s.doc.counts().arrows, 1);
    // Nothing selected: Ctrl+C copies the image, and the window stays.
    assert_eq!(
        s.key(Key::Char('c'), ctrl).effects,
        vec![Effect::Deliver {
            save: false,
            close: false
        }]
    );
    s.press(pt(730.0, 450.0), NONE);
    s.release(pt(730.0, 450.0));
    assert!(s.doc.has_selection());
    let r = s.key(Key::Char('c'), ctrl);
    assert!(
        matches!(r.effects.as_slice(), [Effect::Log(_)]),
        "{:?}",
        r.effects
    );
    assert!(s.key(Key::Char('v'), ctrl).repaint);
    assert_eq!(s.doc.counts().arrows, 2);
    // The wheel zooms with Ctrl.
    assert!(s.wheel(pt(730.0, 450.0), 2.0, false, true).repaint);
    assert!(s.doc.zoom() > 1.0);
}

#[test]
fn delivering_encodes_png_and_the_saved_format() {
    let (mut s, _) = session(None, 0);
    let d = s.deliverable(false).unwrap();
    assert_eq!((d.width, d.height), (200, 100));
    assert_eq!(&d.png[1..4], b"PNG");
    assert!(d.file.is_none());
    s.doc.set_format(Format::Jpeg);
    let d = s.deliverable(true).unwrap();
    let (stem, ext, bytes) = d.file.unwrap();
    assert_eq!((stem.as_str(), ext), ("Spooch test", "jpg"));
    assert_eq!(&bytes[..2], &[0xff, 0xd8]);
}

#[test]
fn a_reopened_capture_keeps_its_own_frame() {
    let dir = std::env::temp_dir().join(format!("spooch-app-frame-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let recents = Recents::new(dir.clone());
    let (mut s, _) = session(Some(recents.clone()), 1);
    let frame = spooch_render::Frame::builtin("terminal-modern");
    s.doc.set_frame(frame);
    s.doc.set_shadow(3);
    s.keep_now();
    // The styles carried from elsewhere have no frame.
    let again = Session::open(&recents, s.id(), vec![], Styles::default(), Host::Window).unwrap();
    assert_eq!((again.doc.frame(), again.doc.shadow()), (frame, 3));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_shared_capture_keeps_its_link_until_edited_wherever_it_reopens() {
    let dir = std::env::temp_dir().join(format!("spooch-app-link-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let recents = Recents::new(dir.clone());
    let (mut s, _) = session(Some(recents.clone()), 1);
    s.link = Some(("https://img.test/x".into(), s.doc.look()));
    s.keep_now();
    // Reopened on screens that don't hold it, it's moved, and still the
    // same capture: Share copies the link again.
    let elsewhere = vec![IRect::from_ltrb(1000, 1000, 2000, 2000)];
    let mut again = Session::open(
        &recents,
        s.id(),
        elsewhere,
        Styles::default(),
        Host::Overlay,
    )
    .unwrap();
    assert_eq!(again.doc.canvas().left, 1000);
    let mut r = Response::default();
    again.share(&mut r);
    assert_eq!(
        r.effects,
        vec![Effect::CopyText("https://img.test/x".into())]
    );
    // Kept with a mark it didn't have when shared: no link.
    s.key(Key::Char('2'), NONE);
    s.press(pt(120.0, 120.0), NONE);
    s.motion(pt(200.0, 170.0), (4.0, 4.0), NONE);
    s.release(pt(200.0, 170.0));
    s.keep_now();
    let mut edited =
        Session::open(&recents, s.id(), vec![], Styles::default(), Host::Window).unwrap();
    let mut r = Response::default();
    edited.share(&mut r);
    assert!(r.effects.is_empty(), "{:?}", r.effects);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_reopened_shared_capture_s_marks_can_be_selected() {
    let dir = std::env::temp_dir().join(format!("spooch-app-shared-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let recents = Recents::new(dir.clone());
    let (mut s, _) = session(Some(recents.clone()), 1);
    s.press(pt(150.0, 150.0), NONE);
    s.motion(pt(250.0, 150.0), (4.0, 4.0), NONE);
    s.release(pt(250.0, 150.0));
    s.keep_now();
    for shared in [false, true] {
        if shared {
            recents
                .write_share(s.id(), "https://img.test/x", "")
                .unwrap();
        }
        let screens = vec![IRect::from_wh(400, 300)];
        let mut r =
            Session::open(&recents, s.id(), screens, Styles::default(), Host::Overlay).unwrap();
        r.press(pt(200.0, 150.0), NONE);
        r.release(pt(200.0, 150.0));
        assert!(r.doc.has_selection(), "shared: {shared}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn in_the_window_a_shared_capture_s_marks_can_be_selected() {
    let dir = std::env::temp_dir().join(format!("spooch-app-win-shared-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let recents = Recents::new(dir.clone());
    let (mut kept, _) = session(Some(recents.clone()), 1);
    kept.keep_now();
    for shared in [false, true] {
        if shared {
            recents
                .write_share(kept.id(), "https://img.test/x", "")
                .unwrap();
        }
        let id = Some(kept.id().to_string());
        let (mut s, _) = Session::open_editor(Some(&recents), id, Styles::default()).unwrap();
        s.resize(1200, 900);
        s.press(pt(690.0, 450.0), NONE);
        s.motion(pt(770.0, 450.0), (4.0, 4.0), NONE);
        s.release(pt(770.0, 450.0));
        assert_eq!(s.doc.counts().arrows, 1, "shared: {shared}");
        s.press(pt(730.0, 450.0), NONE);
        s.release(pt(730.0, 450.0));
        assert!(s.doc.has_selection(), "shared: {shared}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_reopened_label_can_be_selected_in_the_window() {
    let dir = std::env::temp_dir().join(format!("spooch-app-label-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let recents = Recents::new(dir.clone());
    let (kept, _) = session(Some(recents.clone()), 1);
    let mut w = kept.into_window();
    w.resize(1200, 900);
    for c in "Hello there".chars() {
        w.type_char(c, pt(700.0, 450.0));
    }
    w.key(Key::Enter, NONE);
    assert_eq!(w.doc.counts().labels, 1);
    w.keep_now();
    let hits = |s: &mut Session| {
        (0..40)
            .filter(|i| {
                let p = pt(690.0 + *i as f32 * 3.0, 450.0);
                s.press(p, NONE);
                s.release(p);
                let hit = s.doc.has_selection();
                s.key(Key::Escape, NONE);
                hit
            })
            .count()
    };
    let before = hits(&mut w);
    recents
        .write_share(w.id(), "https://img.test/x", "")
        .unwrap();
    let id = Some(w.id().to_string());
    let (mut s, _) = Session::open_editor(Some(&recents), id, Styles::default()).unwrap();
    s.resize(1200, 900);
    assert!(before > 0);
    assert_eq!(hits(&mut s), before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recents_keep_reopen_and_page_through() {
    let dir = std::env::temp_dir().join(format!("spooch-app-recents-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let recents = Recents::new(dir.clone());
    let (mut older, _) = session(Some(recents.clone()), 1);
    older.press(pt(150.0, 150.0), NONE);
    older.motion(pt(250.0, 150.0), (4.0, 4.0), NONE);
    older.release(pt(250.0, 150.0));
    assert!(older.keep_now().unwrap().contains("kept"));
    let (mut newer, _) = session(Some(recents.clone()), 2);
    assert!(newer.keep_now().is_some());
    // PageUp from the newer one opens the older, marks and all.
    let r = newer.key(Key::PageUp, NONE);
    assert!(r.repaint, "{:?}", r.effects);
    assert_eq!(newer.id(), older.id());
    assert_eq!(newer.doc.counts().arrows, 1);
    // Reopened off every screen, it's moved onto the first.
    let s = Session::open(
        &recents,
        older.id(),
        vec![IRect::from_ltrb(1000, 1000, 2000, 2000)],
        Styles::default(),
        Host::Window,
    )
    .unwrap();
    assert_eq!(s.doc.canvas().left, 1000);
    // The editor opens the newest when no id is given.
    let (s, line) = Session::open_editor(Some(&recents), None, Styles::default()).unwrap();
    assert_eq!(s.host(), Host::Window);
    assert!(line.contains("in the editor"), "{line}");
    assert!(s.closing_line().contains(s.id()));
    assert!(Session::open_editor(None, None, Styles::default()).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_gallery_gets_thumbnails_opens_drags_and_deletes() {
    let dir = std::env::temp_dir().join(format!("spooch-gallery-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let recents = Recents::new(dir.clone());
    // Two kept captures; the older one from before thumbnails.
    let (mut older, _) = session(Some(recents.clone()), 1);
    older.keep_now();
    std::fs::remove_file(recents.dir(older.id()).join("thumb.png")).unwrap();
    let (mut newer, _) = session(Some(recents.clone()), 2);
    newer.keep_now();
    let (mut s, _) = Session::open_editor(Some(&recents), None, Styles::default()).unwrap();
    s.resize(1200, 900);
    assert_eq!(s.id(), newer.id());
    let (tx, rx) = mpsc::channel();
    s.set_waker(move || {
        let _ = tx.send(());
    });
    assert_eq!(s.doc.gallery_id(0), Some(newer.id()));
    assert_eq!(s.doc.gallery_id(1), Some(older.id()));
    // The older one's thumbnail is made in the background, and kept.
    rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
    assert!(s.poll().repaint);
    assert!(s.doc.gallery_missing().is_empty());
    assert!(recents.dir(older.id()).join("thumb.png").is_file());

    // A press and release on the older card opens it in place.
    let at = pt(50.0, 300.0);
    let mut r = Response::default();
    s.gallery_action(GalleryAction::Press(1), at, &mut r);
    assert_eq!(r.effects, vec![Effect::CaptureMouse]);
    assert!(s.release(at).repaint);
    assert_eq!(s.id(), older.id());
    // In the same window, at the same size, chrome and all.
    assert_eq!(s.doc.window_state(), Some(((1200, 900), false)));
    assert_eq!(s.doc.gallery_id(0), Some(newer.id()));

    // A press that moves drags that capture out as a file.
    let mut r = Response::default();
    s.gallery_action(GalleryAction::Press(0), at, &mut r);
    let r = s.motion(pt(80.0, 300.0), (4.0, 4.0), NONE);
    assert_eq!(r.effects, vec![Effect::StartDragOut]);
    let (path, _) = s.drag_file(&dir.join("drag")).unwrap();
    assert!(path.is_file());

    // Reveal names its folder; delete removes it from Recents.
    let mut r = Response::default();
    s.gallery_action(GalleryAction::Reveal(0), at, &mut r);
    assert_eq!(r.effects, vec![Effect::Reveal(recents.dir(newer.id()))]);
    let mut r = Response::default();
    s.gallery_action(GalleryAction::Delete(0), at, &mut r);
    assert_eq!(recents.list(), vec![older.id().to_string()]);
    assert_eq!(s.doc.gallery_id(1), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn after_saving_in_the_window_save_shows_the_picture_until_an_edit() {
    static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    let dir = std::env::temp_dir().join(format!("spooch-saved-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Spooch test.png"), b"png").unwrap();
    let _ = DIR.set(dir.clone());
    spooch_platform::save::set_folder(|| DIR.get().cloned());

    let (overlay, _) = session(None, 0);
    let mut s = overlay.into_window();
    s.resize(1200, 900);
    let ctrl = Mods { ctrl: true, ..NONE };
    let save = Effect::Deliver {
        save: true,
        close: false,
    };
    assert_eq!(s.key(Key::Char('s'), ctrl).effects, vec![save.clone()]);
    // Saved: the bar's Save shows the picture.
    assert!(s.doc.save_shows());
    assert_eq!(
        s.save_or_show(),
        Effect::Reveal(dir.join("Spooch test.png"))
    );
    // An arrow later, it saves again.
    s.press(pt(690.0, 450.0), NONE);
    s.motion(pt(770.0, 450.0), (4.0, 4.0), NONE);
    s.release(pt(770.0, 450.0));
    assert!(!s.doc.save_shows());
    assert_eq!(s.save_or_show(), save);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn only_a_capture_that_was_used_is_kept() {
    let dir = std::env::temp_dir().join(format!("spooch-app-wanted-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let recents = Recents::new(dir.clone());
    let keep = |s: &mut Session| {
        let (tx, rx) = mpsc::channel();
        s.keep_in_background(move |line| {
            let _ = tx.send(line);
        });
        rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap()
    };
    // Cancelled without copying: let go.
    let (mut cancelled, _) = session(Some(recents.clone()), 1);
    assert!(keep(&mut cancelled).starts_with("not kept"));
    assert!(recents.list().is_empty());
    // Copied: kept.
    let (mut copied, _) = session(Some(recents.clone()), 2);
    let ctrl = Mods { ctrl: true, ..NONE };
    assert!(
        copied
            .key(Key::Char('c'), ctrl)
            .effects
            .iter()
            .any(|e| matches!(e, Effect::Deliver { .. }))
    );
    assert!(keep(&mut copied).starts_with("kept in recents"));
    assert_eq!(recents.list(), vec![copied.id().to_string()]);
    // Marked, then cancelled: kept, so the work isn't lost.
    let (mut marked, _) = session(Some(recents.clone()), 4);
    arrow(&mut marked);
    assert!(keep(&mut marked).starts_with("kept in recents"));
    // Taken to the editor window: kept.
    let (s, _) = session(Some(recents.clone()), 3);
    let mut window = s.into_window();
    assert!(keep(&mut window).starts_with("kept in recents"));
    // Reopened from Recents and cancelled: still there, kept as before.
    let (mut reopened, _) = Session::reopen(
        Some(&recents),
        copied.id(),
        vec![IRect::from_wh(400, 300)],
        Styles::default(),
    )
    .unwrap();
    assert!(keep(&mut reopened).starts_with("kept in recents"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Draw an arrow across the selection.
fn arrow(s: &mut Session) {
    s.press(pt(120.0, 120.0), NONE);
    s.motion(pt(180.0, 160.0), (4.0, 4.0), NONE);
    s.release(pt(180.0, 160.0));
    assert_eq!(s.doc.counts().arrows, 1);
}

#[test]
fn a_right_click_cancels_only_before_the_first_mark() {
    let (mut s, _) = session(None, 0);
    assert_eq!(s.right_click().effects, vec![Effect::Cancel]);
    arrow(&mut s);
    assert!(s.right_click().effects.is_empty());
    // Esc still cancels.
    assert_eq!(s.key(Key::Escape, NONE).effects, vec![Effect::Cancel]);
}
