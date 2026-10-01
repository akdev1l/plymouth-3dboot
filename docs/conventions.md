<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# Conventions

These conventions are fixed early because tests and golden images depend on
them. Changing one means a deliberate, reviewed update of the affected
goldens.

## Coordinate systems

| Space | Convention |
|---|---|
| World / model | Right-handed, **Y-up**, units in **metres**. Loaders convert the source up-axis and units (e.g. COLLADA `Z_UP`, `<unit meter="0.0254">`). |
| View | Camera looks down **−Z**, +Y up (`glam::camera::rh`). |
| Clip | OpenGL style: visible when `-w ≤ x, y, z ≤ w`, with `w > 0`. |
| NDC → depth buffer | NDC `z ∈ [-1, 1]` maps to depth `[0, 1]`. **Smaller is closer.** The buffer is cleared to `1.0`, and the default test is `Less`. |
| Framebuffer | Origin at the **top left**, +x right, +y down. |

Math uses `glam` (`f32`, column vectors, `Mat4 * Vec4`), built with
`scalar-math` and `libm`, so no SIMD-specific or platform-libm behaviour can
creep in. Transcendental functions (`sin`, `cos`, `acos`, `powf`, …) come
from the `libm` crate, never the platform `f32`/`f64` methods; `clippy.toml`
enforces this. As a result, native and wasm renders are **bit-identical** and
compared against the same goldens exactly.

## Triangles

- **Front faces are counter-clockwise** in NDC (looking at the screen). Culling
  is configurable (`None`, `Back`, `Front`).
- Pixel centres are sampled at `(x + 0.5, y + 0.5)`.
- **Top-left fill rule:** a pixel centre exactly on an edge belongs to the
  triangle only if that edge is a top or left edge. Triangles that share an edge
  therefore cover each pixel exactly once.
- Vertices are snapped to a fixed-point subpixel grid before setup (see Phase 2).

## Colour

- Shading is done in **linear** `f32` RGBA.
- Output is **sRGB-encoded** 8-bit `R, G, B, A` in byte order, row-major,
  with no padding unless a stride is given. That is SDL's
  `SDL_PIXELFORMAT_RGBA32`.
- Material colours from files (OBJ `Kd`, COLLADA `<color>`) are treated as
  sRGB-encoded values and converted to linear on load. For unlit rendering
  this reproduces the authored 8-bit colours exactly.
- The C API can also write premultiplied ARGB32 in native-endian `u32`, which
  is Plymouth's pixel-buffer format.

## Time

- Animation time is `f64` seconds. Frame `i` of a sequence at `fps` is
  sampled at `start + i / fps`, computed from the integer index so that time
  does not drift.
- Rendering is a pure function of (scene, clip, time, camera, options). Wall
  clocks live only in the frontends (SDL main loop, Plymouth plugin).
