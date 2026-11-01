# Spooch

A fast, native screenshot markup tool in the spirit of Skitch: press a
hotkey, the screen freezes, mark up the part you want in place, then copy,
save or drag it out. Local use works without an account, and nothing is
uploaded unless you ask. Windows and Linux (X11) for now.

## Layout

| Crate | What it is |
| --- | --- |
| `spooch-platform` | Settings, Recents, saving, the log and the shared app flow (tray menu, shortcuts, countdown) |
| `spooch-render` | The editor on Skia: `Document` (marks, tools, undo, chrome, frames, the editor window's view and gallery) |
| `spooch-app` | `Picker` and `Session`: picking an area and editing one capture, as every shell drives them |
| `spooch-share` | HTTP client for the sharing and sync service, and the keychain device token |
| `spooch-shell-win` | Windows shell (`windows-rs`), bin `spooch` |
| `spooch-shell-x11` | Linux X11 shell (`x11rb`, no toolkit), bin `spooch-x11` |

All editor behaviour lives in `spooch-render` and `spooch-app`; the shells
translate OS input and show pixels.

## Build and run

Windows (MSVC):

```
cargo build --release -p spooch-shell-win
target\release\spooch.exe [--backend=dxgi|gdi] [--no-recents] [--editor] [--welcome]
```

Linux, from an X11 session:

```
sudo apt install libfreetype-dev libfontconfig-dev   # Fedora: freetype-devel fontconfig-devel
cargo run --release -p spooch-shell-x11 [-- --backend=shm|getimage]
```

`skia-safe` downloads prebuilt Skia binaries, which exist for the MSVC and
Linux targets; cross-compiling to `windows-gnu` isn't supported.

## Using it

- **Ctrl+Shift+2** captures: drag an area, or click (or Enter) for the
  monitor under the pointer. Arrow keys nudge the pointer. **Ctrl+Shift+3**
  captures after a countdown.
- In the editor: **1** arrow, **2** box, **3** highlighter, **4** blur,
  **5** pixelate, **6** line, **7** ellipse, **8** pen, **9** numbered
  steps, **0** redact; type for a label. Click a mark to select, move or
  reshape it; drag the capture's edges to crop or extend it.
- **Enter** or **Ctrl+C** copies, **Ctrl+S** also saves to
  `Pictures\Spooch` (`$XDG_PICTURES_DIR/Spooch`), **Ctrl+drag** drops it into
  another app as a file. **Esc** or right-click cancels.
- **Ctrl+E** moves the capture into the editor window, with zoom and a
  gallery of Recents (**Ctrl+H** hides it).
- Every capture is kept in Recents for 30 days or 2 GB. **Ctrl+Alt+Shift+2**
  reopens the newest; PageUp and PageDown step through them.
- **Settings...** in the tray, or the gear in the editor, changes shortcuts
  and the rest. They live in `%APPDATA%\Spooch\settings.txt`
  (`~/.config/spooch/settings.txt`) and reload when saved. Custom frames go
  in the `frames` folder next to it.

The log, with each capture's timings, is `%LOCALAPPDATA%\Spooch\spooch.log`
(`~/.local/state/spooch/spooch.log`).

## Benchmark

```
cargo run --release -p spooch-render --bin spooch-render-bench [screenshot.png] [--out bench.png]
```

Times a full editor frame (screenshot, ten arrows, a blurred region),
single arrows, blur and pixelate over a 2560x1440 desktop, then writes a
PNG to check the arrows by eye.

## Development

```
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT), at your option. The bundled fonts are under the
SIL Open Font License and the stickers are Microsoft's Fluent Emoji (MIT);
their licenses sit next to them in `crates/spooch-render`.
