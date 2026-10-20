//! The editor as the shells drive it, the same on every OS: joins
//! `spooch-render`'s `Document` with `spooch-platform`. A `Picker` is the
//! area being picked, a `Session` one capture being edited. Shells
//! translate OS events and carry out the `Effect`s these hand back.

mod args;
mod carry;
mod config;
mod picker;
mod place;
mod session;
mod sharing;
mod sync;

use spooch_platform::settings::Settings;
use spooch_render::{PALETTES, Palette, Styles};

/// Apply what `settings` says about the editor to the styles the shell
/// keeps between captures: the colour preset. Returns them, and a problem
/// for the log if the setting names no preset.
pub fn apply_settings(styles: Styles, settings: &Settings) -> (Styles, Option<String>) {
    match Palette::by_name(&settings.palette) {
        Some(i) => (styles.with_palette(i), None),
        None => {
            let names: Vec<String> = PALETTES.iter().map(|p| p.name.to_lowercase()).collect();
            let problem = format!(
                "palette {:?} isn't one of {}",
                settings.palette,
                names.join(", ")
            );
            (styles, Some(problem))
        }
    }
}

pub use args::Args;
pub use carry::Carry;
pub use config::{Config, Loaded};
pub use picker::{Pick, Picker, Setup};
pub use place::{EDITOR_MIN, centred, editor_rect, monitor_at};
pub use session::{Clip, DRAG_SLOP, Deliverable, Effect, Host, Response, Session, dragged_line};
pub use sharing::{Field, Form, Sharing, device_label, looks_like_token};
pub use sync::Syncing;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_palette_setting_picks_a_preset() {
        let mut s = Settings {
            palette: "ink".into(),
            ..Settings::default()
        };
        let (styles, problem) = apply_settings(Styles::default(), &s);
        assert!(problem.is_none());
        assert_ne!(styles, Styles::default());
        s.palette = "neon".into();
        let (same, problem) = apply_settings(Styles::default(), &s);
        assert_eq!(same, Styles::default());
        assert!(problem.unwrap().contains("neon"));
    }
}
