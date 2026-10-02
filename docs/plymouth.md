<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# Plymouth integration

The theme is a Plymouth **splash plugin**: a shared object that plymouthd
loads from `/usr/lib/<multiarch>/plymouth/`, plus a theme directory under
`/usr/share/plymouth/themes/`. The plugin renders through `libplymouth_3dboot`
([C API](c-api.md)).

Supported Plymouth: **24.x** (developed against 24.004.60, Debian trixie;
`ply-splash-core` and `ply-splash-graphics`, soname 5).

## Plugin interface (`ply-boot-splash-plugin.h`)

The module exports `ply_boot_splash_plugin_interface_t *
ply_boot_splash_plugin_get_interface(void)`, which returns a static table of
callbacks:

| Callback | What the plugin does |
|---|---|
| `create_plugin(key_file)` | Read the theme keys (below) and load the model. Return NULL on failure; plymouthd then falls back to another splash. |
| `add_pixel_display` / `remove_pixel_display` | Keep one *view* per display (head): a `p3b_renderer` sized to the display and an ARGB32 frame buffer. Register a draw handler with `ply_pixel_display_set_draw_handler`. |
| `show_splash_screen(loop, boot_buffer, mode)` | Record the start time (`ply_get_timestamp()`) and start the frame timer `ply_event_loop_watch_for_timeout(loop, 1/fps, …)`. |
| frame timer | Ask every display to redraw (`ply_pixel_display_draw_area`), then re-arm. Animation time is `now - start`, so slow frames drop rather than slowing the animation down. |
| draw handler `(user, pixel_buffer, x, y, w, h, display)` | Render the frame for the current time with `p3b_render_frame(…, P3B_PIXEL_FORMAT_ARGB32_PREMULTIPLIED)` into the view buffer, then `ply_pixel_buffer_fill_with_argb32_data`. Plymouth clips to the update area. Draw prompt or message labels on top. |
| `hide_splash_screen` | Stop the timer and unset the draw handlers. |
| `display_message` / `hide_message` | Show or hide a line of text (`ply_label`). |
| `display_password(prompt, bullets)` / `display_question(prompt, text)` / `display_prompt` | Show the prompt and the entered text (bullets for secrets) with `ply_label`s; `display_normal` hides them. |
| `become_idle(trigger)` | Pull the trigger: there is nothing to finish. |
| keyboard, text display, progress, boot output | Ignored (optional callbacks may be NULL). |

All boot modes (boot, shutdown, reboot, updates) show the same animation.
Prompt and message text is drawn with `ply_label`, which needs a Plymouth
label plugin (`label-freetype` or `label-pango`) at run time. Without one,
the text stays invisible but the animation keeps running. The VM checklist
covers the visual check.

## Pixels

`ply_pixel_buffer` stores `uint32_t` pixels `0xAARRGGBB` with premultiplied
alpha, which is `P3B_PIXEL_FORMAT_ARGB32_PREMULTIPLIED`.
`ply_pixel_display_get_width/height` give the display size in logical pixels.
Frames are rendered at that size. HiDPI scaling (`device_scale`) is left to
Plymouth for now.

## Theme file

```ini
[Plymouth Theme]
Name=3D Boot (N64)
Description=Spinning N64 logo rendered with plymouth-3dboot
ModuleName=plymouth-3dboot

[plymouth-3dboot]
# Absolute path of the model (.obj or .dae).
ModelFile=/usr/share/plymouth/themes/3dboot-n64/n64_logo.dae
# Seconds per turntable revolution, used when the model has no animation
# (0 = still).
TurntablePeriod=6
# unlit | lambert | blinn-phong
Shading=unlit
# Background colour, RRGGBB.
BackgroundColor=000000
# Frames per second (1–60).
FramesPerSecond=30
```

## Testing

- **Build:** the plugin compiles against the real headers in the dev
  container with `-Werror`, and exports `ply_boot_splash_plugin_get_interface`.
- **Headless harness:** a C program `dlopen`s the plugin and drives it with
  a real `ply_event_loop` and `ply_pixel_buffer`. Through ELF symbol
  interposition, the harness provides its own `ply_pixel_display_*`
  functions (a fake display) and `ply_get_timestamp` (virtual time). It
  compares the drawn frames with `p3b_render_frame` output for the same
  time, and runs under valgrind.
- **Manual:** a VM boot, following the checklist below once it is written.
