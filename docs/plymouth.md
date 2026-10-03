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
| `show_splash_screen(loop, boot_buffer, mode)` | Record the start time (`ply_get_timestamp()`), redraw every display, and start the frame timer (`ply_event_loop_watch_for_timeout`). |
| frame timer | For each display, render the frame for the current time into the view buffer with `p3b_render_frame_incremental(…, P3B_PIXEL_FORMAT_ARGB32_PREMULTIPLIED, &damage)` and redraw only `damage` (`ply_pixel_display_draw_area`). Re-arm for the next due time: frames are due every 1/fps from the start, so the time spent drawing does not lower the frame rate, and a late frame skips to the next due time. Animation time is `now - start`, so slow frames drop rather than slowing the animation down. |
| draw handler `(user, pixel_buffer, x, y, w, h, display)` | Copy the view buffer with `ply_pixel_buffer_fill_with_argb32_data` (rendering a first frame if there is none). Plymouth clips the copy, and its flush to the screen, to the requested area. Draw prompt or message labels on top. |
| `hide_splash_screen` | Stop the timer and unset the draw handlers. |
| `display_message` / `hide_message` | Show or hide a line of text (`ply_label`). |
| `display_password(prompt, bullets)` / `display_question(prompt, text)` / `display_prompt` | Show the prompt and the entered text (bullets for secrets) with `ply_label`s; `display_normal` hides them. |
| `become_idle(trigger)` | Pull the trigger: there is nothing to finish. |
| `update_status(status)` | Ignored. systemd sends one per unit during boot. **Required:** plymouthd asserts it is not NULL (as for `show_splash_screen` and `hide_splash_screen`), and the harness checks all three. |
| keyboard, text display, progress, boot output | Ignored (these optional callbacks may be NULL). |

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
# Background colour, RRGGBB (sky blue in the demo theme).
BackgroundColor=87CEEB
# Floor colour, RRGGBB; leave out for no floor. The floor sits just below
# the model, stays fixed while it moves, and costs little per frame.
FloorColor=C8C8C8
# Floor half size, in multiples of the model's horizontal radius (> 0).
FloorSize=4
# Frames per second (1–60).
FramesPerSecond=30
# Supersampling anti-aliasing, samples per axis (1 = off, up to 8; cost grows
# with its square).
Antialias=1
# Rendering threads (1–64); the image does not depend on it.
Threads=2
```

## Installing

### Fedora (RPM)

`packaging/plymouth-3dboot.spec` builds four packages: `plymouth-3dboot`
(the library), `plymouth-3dboot-devel`, `plymouth-plugin-3dboot` and
`plymouth-theme-3dboot-n64`. Build them from the working tree in the Fedora
container (see [toolchain.md](toolchain.md)):

```sh
DEV_CONTAINER=fedora scripts/dev.sh just rpm
```

Each build's release contains its UTC time and commit (with `.dirty` for
uncommitted changes), so a newer build always upgrades an older one.

**Fedora Workstation** (and other mutable variants):

```sh
sudo dnf install dist/rpm/plymouth-3dboot-0*.x86_64.rpm \
    dist/rpm/plymouth-plugin-3dboot-0*.rpm dist/rpm/plymouth-theme-3dboot-n64-*.rpm
sudo plymouth-set-default-theme -R 3dboot-n64    # -R rebuilds the initramfs
```

**Fedora Atomic** (Silverblue, Kinoite, uBlue images): `/usr` is read-only
and the initramfs comes with the image.

- *Try it without rebooting.* This is lost on the next reboot:

  ```sh
  sudo rpm-ostree usroverlay               # temporarily writable /usr
  sudo rpm -Uvh dist/rpm/plymouth-3dboot-0*.x86_64.rpm \
      dist/rpm/plymouth-plugin-3dboot-0*.rpm dist/rpm/plymouth-theme-3dboot-n64-*.rpm
  ```

  Then, as root on a spare VT (Ctrl+Alt+F3), run
  `plymouthd; plymouth show-splash; sleep 15; plymouth quit`. To test the
  prompt, run `plymouth ask-for-password --prompt=Test` while the splash is
  showing. Select the theme first; see *Selecting the theme* below.
- *Install it for boot.* Layer the packages, select the theme, and
  regenerate the initramfs locally:

  ```sh
  sudo rpm-ostree install dist/rpm/plymouth-3dboot-0*.x86_64.rpm \
      dist/rpm/plymouth-plugin-3dboot-0*.rpm dist/rpm/plymouth-theme-3dboot-n64-*.rpm
  printf '[Daemon]\nTheme=3dboot-n64\n' | sudo tee /etc/plymouth/plymouthd.conf
  sudo rpm-ostree initramfs --enable
  systemctl reboot
  ```

  To undo, pick the previous deployment in the boot menu, or run
  `sudo rpm-ostree rollback`. To remove it for good, run
  `rpm-ostree uninstall` on the three packages and `rpm-ostree initramfs --disable`,
  and set `Theme=` back to the previous theme (`bgrt` on Fedora).
- *Selecting the theme.* Plymouth reads `Theme=` from
  `/etc/plymouth/plymouthd.conf` (`[Daemon]` section). Note the current value
  before changing it (`plymouth-set-default-theme` prints it).

Updating an installed build: run `rpm -Uvh` with the new files. On Atomic,
replace layered packages in one step with
`sudo rpm-ostree uninstall plymouth-3dboot plymouth-plugin-3dboot plymouth-theme-3dboot-n64 --install <new .rpm files>`.

### From source

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
example when the model cannot be loaded. Every 5 s it also logs its frame
statistics: the frame rate achieved, the time per frame spent rendering and
in Plymouth (copying and flushing to the screen), and the share of the screen
redrawn:

```
plymouth-3dboot: 30.0 fps; per frame: render 9.8 ms, display 3.1 ms, 18% of the screen redrawn
```

To try a theme without rebooting, run
`plymouthd --debug --debug-file=/tmp/plymouth.log; plymouth show-splash; sleep 20; plymouth quit`
as root on a spare VT, then `grep plymouth-3dboot /tmp/plymouth.log`.

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
