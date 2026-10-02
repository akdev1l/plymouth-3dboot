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
Phase 13 addressed this; see below.

## Benchmarks

`just bench` runs the criterion benchmarks in `crates/plymouth-3dboot/benches/`.
`just bench-wasm` prints the median N64 frame time at 640×480 natively and
under node (wasm). Baseline (2026-10-01, Ryzen 7 5800X, before the
optimizations in Phase 13.3):

| Benchmark | Time |
|---|---|
| `triangle_setup` | 29 ns |
| `coverage_256px_triangle_area` | 27 µs |
| `fill_1080p_quad` (2 triangles, every pixel) | 71 ms |
| `n64_frame_640x480` | 6.3 ms |
| `n64_frame_1080p` | 32.8 ms |
| `n64_frame_640x480_aa2` | 33.0 ms |
| `frame_timing`, native / wasm (node) | 6.6 ms / 8.6 ms |

Fragment processing dominates: about 35 ns per pixel for a flat quad.

After Phase 13.3 (same machine, `just bench`):

| Benchmark | Before | After |
|---|---|---|
| `fill_1080p_quad` | 71 ms | 51 ms |
| `n64_frame_640x480` | 6.3 ms | 1.0 ms |
| `n64_frame_1080p` | 32.8 ms | 5.0 ms |
| `n64_frame_640x480_aa2` | 33.0 ms | 6.6 ms |
| `n64_frame_1080p_8_threads` | — | 1.3 ms |
| `frame_timing`, native / wasm (node) | 6.6 / 8.6 ms | 1.0 / 1.4 ms |

The gains came from per-pixel work: sRGB encoding by table lookup (the
transfer function's `powf` dominated), and visiting only the covered span of
each row. Threads (`FrameSettings::threads`, the `parallel` feature) split
the frame into four bands per thread on a pool the renderer owns. A full
1080p frame at 30 fps now uses about 15 % of one core.
