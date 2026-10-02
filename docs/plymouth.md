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
# Supersampling anti-aliasing, samples per axis (1 = off, up to 8; cost grows
# with its square).
Antialias=1
```

## Installing

Build and install the C library, then the plugin and the demo theme:

```sh
cargo cinstall -p plymouth-3dboot-capi --release --prefix /usr --libdir /usr/lib64   # adjust libdir for your distribution
meson setup build plymouth --prefix /usr && meson install -C build
plymouth-set-default-theme -R 3dboot-n64    # -R rebuilds the initramfs
```

The plugin goes to Plymouth's `pluginsdir`
(`/usr/lib/<multiarch>/plymouth/plymouth-3dboot.so`). The theme goes to
`/usr/share/plymouth/themes/3dboot-n64/`, which holds `3dboot-n64.plymouth`,
the model and the model's Readme.

**Initramfs (dracut).** Plymouth's dracut module copies the selected theme's
directory and plugin. dracut normally pulls in the plugin's library
dependencies automatically. If `libplymouth_3dboot.so.0` is missing from
the initramfs (`lsinitrd | grep plymouth_3dboot`), add it explicitly:

```sh
echo 'install_items+=" /usr/lib64/libplymouth_3dboot.so.0 "' > /etc/dracut.conf.d/plymouth-3dboot.conf
dracut -f
```

**Debugging.** Run `plymouthd --debug --debug-file=/tmp/plymouth.log` (or
boot with `plymouth.debug`); the plugin logs through `ply_trace`, for
example when the model cannot be loaded. To try a theme without rebooting,
run `plymouthd; plymouth show-splash; sleep 10; plymouth quit` as root on a
spare VT.

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

## VM verification checklist

Manual; not automated, and **not yet performed**. Run it in a Fedora VM
(QEMU/KVM with `virtio-gpu`, or `simpledrm` firmware framebuffer) with the
library, plugin and theme installed as described above:

1. `plymouth-set-default-theme -R 3dboot-n64` succeeds, and
   `lsinitrd | grep -E 'plymouth-3dboot|plymouth_3dboot|3dboot-n64'` lists
   the plugin, the library and the theme files.
2. **Boot:** the spinning N64 logo appears on a black background and turns
   smoothly (one revolution per 3.3 s, from the model's own animation) until
   the display manager starts.
3. **LUKS:** with an encrypted root, the password prompt appears below the
   logo, typing shows bullets, wrong passwords are re-prompted, and the
   animation keeps running throughout. Needs a label plugin
   (`plymouth-plugin-label` on Fedora).
4. **Messages:** `plymouth display-message --text="Hello"` shows the text;
   `plymouth hide-message --text="Hello"` hides it.
5. **Shutdown/reboot:** the splash appears with the same animation.
6. **Multiple monitors:** each display shows the logo framed for its own
   size.
7. **Failure modes:** with `ModelFile` pointing to a missing file, boot falls
   back to Plymouth's default text splash, and the debug log contains
   `plymouth-3dboot: cannot load`.
8. **Resources:** `plymouthd` CPU use stays reasonable for the resolution
   (see [perf.md](perf.md)), and no crash appears in the journal.
