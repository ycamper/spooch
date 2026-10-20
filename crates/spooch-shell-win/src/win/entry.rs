//! A small window asking for a few lines of text, like a name and a
//! password: a prompt, a labelled field for each, and OK/Cancel. A dialog
//! built in memory, so it needs no resource file. What it asks comes from
//! `spooch_app::Form`.

use std::cell::RefCell;

use spooch_app::Form;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    DLGTEMPLATE, DialogBoxIndirectParamW, EndDialog, GetDlgItem, GetDlgItemTextW, IDCANCEL, IDOK,
    SetDlgItemTextW, WM_COMMAND, WM_INITDIALOG,
};
use windows::core::HSTRING;

const PROMPT: i32 = 100;
/// Field `i` is `FIELD + i`, its label `LABEL + i`.
const FIELD: i32 = 200;
const LABEL: i32 = 300;

// Window and control styles, as the template wants them.
const WS_POPUP: u32 = 0x8000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_BORDER: u32 = 0x0080_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_TABSTOP: u32 = 0x0001_0000;
const WS_EX_TOPMOST: u32 = 0x8;
const DS_SETFONT: u32 = 0x40;
const DS_MODALFRAME: u32 = 0x80;
const DS_SETFOREGROUND: u32 = 0x200;
const DS_CENTER: u32 = 0x800;
const ES_PASSWORD: u32 = 0x20;
const ES_AUTOHSCROLL: u32 = 0x80;
const BS_DEFPUSHBUTTON: u32 = 0x1;
// Predefined control classes.
const BUTTON: u16 = 0x80;
const EDIT: u16 = 0x81;
const STATIC: u16 = 0x82;

/// The dialog's width, and where fields start, in dialog units.
const WIDTH: i16 = 260;
const FIELD_X: i16 = 82;

thread_local! {
    /// The form going in, and the answers coming out.
    static ENTRY: RefCell<(Option<Form>, Option<Vec<String>>)> = const { RefCell::new((None, None)) };
}

/// Show `form` and return what was typed in each field, `None` if
/// cancelled. Blocks until answered, so call it from its own thread.
pub fn ask(form: &Form) -> Option<Vec<String>> {
    // About 62 characters fit a line of the prompt.
    let lines: usize = form
        .prompt
        .split('\n')
        .map(|l| l.chars().count().div_ceil(62).max(1))
        .sum();
    let prompt_h = (lines * 9) as i16;
    let mut items = vec![(
        0,
        STATIC,
        PROMPT,
        [7, 7, WIDTH - 14, prompt_h],
        String::new(),
    )];
    let mut y = 7 + prompt_h + 8;
    for (i, f) in form.fields.iter().enumerate() {
        let i = i as i32;
        let style =
            WS_BORDER | ES_AUTOHSCROLL | WS_TABSTOP | if f.secret { ES_PASSWORD } else { 0 };
        items.push((
            0,
            STATIC,
            LABEL + i,
            [7, y + 2, FIELD_X - 10, 9],
            f.label.to_string(),
        ));
        items.push((
            style,
            EDIT,
            FIELD + i,
            [FIELD_X, y, WIDTH - FIELD_X - 7, 14],
            String::new(),
        ));
        y += 18;
    }
    y += 4;
    items.push((
        BS_DEFPUSHBUTTON | WS_TABSTOP,
        BUTTON,
        IDOK.0,
        [WIDTH - 111, y, 50, 14],
        "OK".into(),
    ));
    items.push((
        WS_TABSTOP,
        BUTTON,
        IDCANCEL.0,
        [WIDTH - 57, y, 50, 14],
        "Cancel".into(),
    ));
    let template = template(form.title, y + 21, &items);
    ENTRY.with(|e| *e.borrow_mut() = (Some(form.clone()), None));
    let ok = unsafe {
        DialogBoxIndirectParamW(
            None,
            template.as_ptr() as *const DLGTEMPLATE,
            None,
            Some(proc),
            LPARAM(0),
        )
    };
    let answers = ENTRY.with(|e| e.borrow_mut().1.take());
    (ok == 1).then_some(answers).flatten()
}

unsafe extern "system" fn proc(dlg: HWND, msg: u32, wparam: WPARAM, _: LPARAM) -> isize {
    unsafe {
        match msg {
            WM_INITDIALOG => {
                let Some(form) = ENTRY.with(|e| e.borrow().0.clone()) else {
                    return 0;
                };
                let _ = SetDlgItemTextW(dlg, PROMPT, &HSTRING::from(form.prompt.as_str()));
                for (i, f) in form.fields.iter().enumerate() {
                    let _ =
                        SetDlgItemTextW(dlg, FIELD + i as i32, &HSTRING::from(f.value.as_str()));
                }
                // The first field still empty takes the typing.
                let first = form
                    .fields
                    .iter()
                    .position(|f| f.value.is_empty())
                    .unwrap_or(0);
                if let Ok(field) = GetDlgItem(Some(dlg), FIELD + first as i32) {
                    let _ = SetFocus(Some(field));
                }
                // Focus was set here.
                0
            }
            WM_COMMAND => match (wparam.0 & 0xFFFF) as i32 {
                id if id == IDOK.0 => {
                    let n = ENTRY.with(|e| e.borrow().0.as_ref().map_or(0, |f| f.fields.len()));
                    let answers = (0..n)
                        .map(|i| {
                            let mut buf = [0u16; 512];
                            let len = GetDlgItemTextW(dlg, FIELD + i as i32, &mut buf) as usize;
                            String::from_utf16_lossy(&buf[..len])
                        })
                        .collect();
                    ENTRY.with(|e| e.borrow_mut().1 = Some(answers));
                    let _ = EndDialog(dlg, 1);
                    1
                }
                id if id == IDCANCEL.0 => {
                    let _ = EndDialog(dlg, 0);
                    1
                }
                _ => 0,
            },
            _ => 0,
        }
    }
}

/// A DLGTEMPLATE `height` dialog units tall with its items: (style, class,
/// id, [x, y, w, h] in dialog units, text). DWORD-aligned, as Windows
/// wants it.
fn template(title: &str, height: i16, items: &[(u32, u16, i32, [i16; 4], String)]) -> Vec<u32> {
    let mut w: Vec<u16> = Vec::new();
    let dword = |w: &mut Vec<u16>, v: u32| w.extend([v as u16, (v >> 16) as u16]);
    let text = |w: &mut Vec<u16>, s: &str| w.extend(s.encode_utf16().chain([0]));
    let style = WS_POPUP
        | WS_CAPTION
        | WS_SYSMENU
        | DS_MODALFRAME
        | DS_SETFONT
        | DS_CENTER
        | DS_SETFOREGROUND;
    dword(&mut w, style);
    dword(&mut w, WS_EX_TOPMOST);
    w.push(items.len() as u16);
    w.extend([0, 0, WIDTH, height].map(|v: i16| v as u16));
    w.extend([0, 0]); // no menu, the default class
    text(&mut w, title);
    w.push(9);
    text(&mut w, "Segoe UI");
    for (item_style, class, id, rect, label) in items {
        if w.len() % 2 == 1 {
            w.push(0);
        }
        dword(&mut w, item_style | WS_CHILD | WS_VISIBLE);
        dword(&mut w, 0);
        w.extend(rect.map(|v| v as u16));
        w.push(*id as u16);
        w.extend([0xFFFF, *class]);
        text(&mut w, label);
        w.push(0); // no creation data
    }
    if w.len() % 2 == 1 {
        w.push(0);
    }
    w.chunks(2)
        .map(|p| u32::from(p[0]) | u32::from(p[1]) << 16)
        .collect()
}
