Spooch: an opinionated screenshot tool.  https://spooch.app

Run it
  Windows   spooch.exe (keep the two DLLs next to it). It sits in the tray.
  Linux     ./spooch-x11 on an X11 session. Needs FreeType and Fontconfig,
            which every desktop already has. Wayland isn't supported yet.

Use it
  Ctrl+Shift+2      capture: drag out an area, or click to take a monitor
  Ctrl+Shift+3      capture after a 3 second countdown
  Ctrl+Alt+Shift+2  reopen the last capture, marks and all
  1 to 0            pick a tool; just start typing for a label
  Enter / Ctrl+C    copy        Ctrl+S  copy and save        Esc  cancel
  Ctrl+E            open the capture in the editor window

Settings are in settings.txt, which the tray menu opens.

Spooch is under the MIT or Apache 2.0 license, at your option.

The fonts Spooch draws labels with are under the SIL Open Font License,
and its stickers are Microsoft's Fluent Emoji, under the MIT License;
their licenses are in the licenses folder.
