//! What a session hands back to the shell.

use std::path::{Path, PathBuf};

use spooch_render::skia_safe::IRect;
use spooch_render::{Counts, WindowButton};

/// Something the shell has to do for the editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Keep the mouse until the button is let go, so a drag can leave the
    /// window it started in.
    CaptureMouse,
    /// Hand over the frozen desktop's pixels in this rectangle with
    /// `supply_desktop`, for cropping or extending past the selection.
    NeedDesktop(IRect),
    /// The pointer has gone far enough: start dragging the image out, as
    /// the file `drag_file` writes.
    StartDragOut,
    /// Copy the image (`deliverable`), save it too when `save`, and close
    /// the editor when `close`.
    Deliver {
        save: bool,
        close: bool,
    },
    /// Put this text on the clipboard: a share's link.
    CopyText(String),
    /// A share from the overlay is done: close it.
    Shared,
    /// Close the editor without delivering anything.
    Cancel,
    /// Move this capture into the editor window (`into_window`).
    OpenWindow,
    /// One of the editor window's own buttons was clicked.
    Window(WindowButton),
    /// Show this folder (a kept capture's) in the file manager.
    Reveal(PathBuf),
    Log(String),
}

/// What an input did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Response {
    /// Whether the editor looks different.
    pub repaint: bool,
    /// Whether a key was used, so any text it types must not reach
    /// `type_char`.
    pub used: bool,
    pub effects: Vec<Effect>,
}

/// How far, in pixels, the pointer goes before a press becomes a drag, on
/// systems that don't say (Windows does).
pub const DRAG_SLOP: f32 = 4.0;

/// The log's line for a capture of `area` dragged out as `path`.
pub fn dragged_line(area: IRect, counts: Counts, path: &Path) -> String {
    format!(
        "dragged out {}x{} region at ({}, {}) with {counts} as {}",
        area.width(),
        area.height(),
        area.left,
        area.top,
        path.display()
    )
}
