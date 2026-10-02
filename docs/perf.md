<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# Performance

## Boot-time budgets

At boot, the plugin and `libplymouth_3dboot` live in the initramfs and render
on the CPU while the system starts. `just budgets` (part of `just check`)
measures a release build of the C library with
`crates/plymouth-3dboot-capi/examples/budget.c` and enforces these limits:

| Budget | Limit | Baseline |
|---|---|---|
| `libplymouth_3dboot.so`, stripped | < 2 MiB | 0.9 MB (1.15 MB unstripped) |
| Time to first frame at 1920×1080 (load + renderer + render) | < 500 ms | 47 ms |
| Median frame time at 1920×1080 | < 200 ms | 36 ms |

The limits are deliberately loose (about 5× the baseline) so that machine
noise never fails the gate; they catch regressions by an order of magnitude.

## Baseline

Measured 2026-10-01 with the N64 spin model, unlit, release build,
single-threaded, on an AMD Ryzen 7 5800X:

| Resolution | First frame | Steady frame | Max fps |
|---|---|---|---|
| 640×480 | 9.1 ms | 6.9 ms | 145 |
| 1920×1080 | 46.9 ms | 35.5 ms | 28 |
| 3840×2160 | 186.9 ms | 143.0 ms | 7 |

At 1080p the renderer is just below the plugin's default 30 fps. The
time-based animation stays correct; slower machines show fewer frames.
Phase 13 (tiled and parallel rasterization) targets this.
