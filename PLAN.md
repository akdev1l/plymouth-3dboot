# Execution Plan: plymouth-3dboot

A Rust library for rasterizing animated 3D models, built for native targets and WebAssembly and presented through SDL.

Status: **approved baseline, v3** · Last updated: 2026-10-01

## 0. Goal & scope

The library loads 3D models, evaluates their animation at any time `t`, and
rasterizes each frame on the CPU into an RGBA framebuffer. The result is
deterministic and tested at every layer. Frames are presented through **SDL**,
and everything builds for both:

- **native**: `x86_64-unknown-linux-gnu`, with SDL from the system
- **wasm**: `wasm32-unknown-emscripten`, with SDL from Emscripten's port (`-sUSE_SDL=3`)

**In scope:** a CPU software rasterizer, OBJ/MTL and COLLADA loaders, a node
hierarchy, keyframed transform animation (TRS), cameras, and basic shading
(unlit/emissive, Lambert, Blinn-Phong). Also an SDL presenter with a main loop
that works natively and under Emscripten, a viewer app for both targets, and
PNG / PNG-sequence / GIF output. It also exposes a **stable C ABI** (shared and
static library, header, pkg-config file) and ships a **Plymouth splash plugin
and theme** built on that ABI.

**Deferred (decide when we reach it, see Phase 14):** glTF 2.0, skinning, morph
targets, textures.

**Out of scope:** GPU rendering (SDL is only used to blit the finished
framebuffer), ray tracing.

**Licence:** GPL-3.0-or-later (SPDX `GPL-3.0-or-later`). Every dependency must be GPL-3.0-compatible (MIT, Apache-2.0,
BSD, Zlib, ISC). `cargo-deny` enforces this.

### Architecture

```
┌────────────────────────────┐   RGBA8 framebuffer   ┌───────────────────────────────┐
│ plymouth-3dboot  (core)    │ ────────────────────▶ │ plymouth-3dboot-sdl           │
│ pure Rust, no C deps,      │                       │ SDL presenter + main loop     │
│ #![forbid(unsafe_code)],   │                       │ (native loop | emscripten     │
│ builds natively + wasm     │                       │  main loop), unsafe isolated  │
└─────────────┬──────────────┘                       └───────────────┬───────────────┘
              │                                                      │
              │                                      ┌───────────────▼───────────────┐
              │                                      │ apps/viewer (bin)             │
              │                                      │ native window / browser canvas│
              │                                      └───────────────────────────────┘
┌─────────────▼──────────────┐   C ABI (p3b_*)       ┌───────────────────────────────┐
│ plymouth-3dboot-capi       │ ────────────────────▶ │ plymouth/ splash plugin (C)   │
│ cdylib + staticlib, header │                       │ + theme; renders into         │
│ (cbindgen), pkg-config     │                       │ ply_pixel_buffer (ARGB32)     │
└────────────────────────────┘                       └───────────────────────────────┘
```

Plymouth runs inside the initramfs, early in boot, often without a GPU. That is
why rendering is CPU-only. It also constrains the C library: no panics or
unwinding across the ABI, no threads by default, a small binary, fast start-up,
and load from a theme directory or from memory.

The core never touches SDL, the filesystem, threads or the clock directly.
That keeps it fully testable, and its output identical on both targets. Assets
reach the core as bytes through a `ResourceResolver` trait: filesystem natively,
and embedded or preloaded files on wasm.

### Facts about the sample model (`/tmp/N64 Logo`)

| File | Content | Implications |
|---|---|---|
| `N64 Logo.obj` | 48 vertices; 52 faces (44 quads + 8 triangles; the header comment's "44 polygons" is misleading); smoothing groups; 4 materials; **no normals/UVs**; Y-up; **CRLF line endings** | Triangulate the quads (→ 96 triangles, the same as the DAE). Handle CRLF. Generate normals. |
| `N64 Logo.mtl` | CRLF; 4 materials; `Kd` = `Ke` (green/blue/red/yellow) | The Readme says "self-illumination full", so the reference look is **unlit/emissive**. |
| `N64 Logo.DAE` | COLLADA 1.4.1; 96 triangles; per-corner normals; 4 Blinn effects; `Z_UP`; unit = inch; node TRS + pivot `<matrix>`; scene time 0–3.33 s @ 30 fps; **no `<library_animations>`** | Needs up-axis and unit conversion and a node hierarchy. Animation tests need synthetic fixtures. |
| `Readme.txt` | Reference sRGB colours; "No credit needed for use" | Safe to vendor into `tests/fixtures/` with the Readme alongside. |

## 1. Working agreements

### 1.1 Change protocol (every step)

1. **Before:** `git status` is clean, and `just check` passes in the container on HEAD.
2. **Write tests first** where practical. They must fail, or fail to compile, for the right reason.
3. Implement until they pass.
4. **After:** `just check` passes. That means fmt, clippy `-D warnings`, native tests, doc build, the wasm build, wasm tests under node, and `cargo deny`. `git status` shows only the intended files.
5. Commit using **Conventional Commits**, with no AI attribution trailers.
6. Make one logical change per commit, and one commit per sub-step.

### 1.2 Test strategy

| Layer | Kind | Tooling |
|---|---|---|
| Math, interpolation, parsing | Unit tests, exact or epsilon | `#[test]`, `approx` |
| Rasterizer invariants | Property tests: watertightness, no double coverage, determinism | `proptest` |
| Loaders | Fixture tests: counts, bounds, materials, malformed input | `tests/fixtures/` |
| Rendering | **Golden images.** Compare against a committed PNG within a tolerance (max channel delta and % of differing pixels). On failure, write `actual.png` and `diff.png` to `target/golden-failures/`. Regenerate only with `UPDATE_GOLDEN=1`. | own harness + `png` |
| Cross-target | The **same golden tests run on wasm** under node (`NODERAWFS`), so native and wasm renders must agree | Cargo runner = node |
| SDL presenter | Headless (`SDL_VIDEODRIVER=dummy`, software renderer). Upload a frame, read it back, compare bytes. | SDL |
| Browser | Smoke test: headless Chromium loads the viewer, screenshots the canvas, and checks the palette | `just browser-smoke` |
| Animation | Sample at known `t`; golden frames at fixed times | unit + golden |
| C ABI | Rust-side FFI tests (null/invalid inputs, panic containment). **C test programs** compiled against the generated header and linked to both the `.so` and the `.a`, run under **valgrind** with zero leaks or errors. Header drift check: regenerating gives an identical header. | `cc`, `valgrind`, `cbindgen` |
| Plymouth plugin | Compiled against the real Plymouth headers. A C harness `dlopen`s the plugin and drives its interface with a fake pixel buffer, checked against a golden. Then a manual VM boot test. | `libplymouth-dev`, harness |
| Performance | Benchmarks, native, plus wasm frame timing; not gating | `criterion` |

**Determinism rule:** the same input gives byte-identical output on a given target.
Golden comparisons allow a small tolerance across targets.

### 1.3 Conventions (`docs/conventions.md`)

- Right-handed, **Y-up** internally. Loaders convert up-axis and units to metres.
- Column vectors, `glam` (`Mat4`/`Vec3`/`Quat`, f32). No SIMD-dependent
  behaviour that differs between native and wasm (verified by the cross-target goldens).
- Clip space is OpenGL-style (`-w..w`). Depth buffer `[0,1]`, smaller is closer.
- CCW front faces; culling is configurable.
- Pixel centres at `(x+0.5, y+0.5)`, **top-left fill rule**, origin at the top left.
- Colour is linear `f32` internally and encoded to sRGB `u8`. The framebuffer
  byte order is `R,G,B,A`, presented through SDL as `SDL_PIXELFORMAT_ABGR8888`
  on little-endian hosts. Byte order is tested in Phase 6.

## 2. Phases & steps

Each step lists **Deliverable / Tests / Done when / Commit**.

---

### Phase 0: Development environment & skeleton

**0.1 Containerfile (native toolchain)**
- Base image: `docker.io/library/rust:1.98.1-slim-trixie`.
- Rust components: `rustfmt`, `clippy`. Cargo tools, installed with `--locked` and pinned: `just`, `cargo-nextest`, `cargo-deny`, `cargo-llvm-cov`.
- System packages: `git`, `pkg-config`, `build-essential`, `cmake`, `libsdl3-dev`, `python3`, `valgrind`, `libplymouth-dev`.
- Also install `cargo-c` (pinned, `--locked`) for C library builds.
- Non-root user, `WORKDIR /work`.
- *Tests:* `podman build` succeeds; the tool versions print as pinned; `pkg-config --modversion sdl3` works.
- *Commit:* `build: add Containerfile for native toolchain`

**0.2 Emscripten in the container**
- Install a **pinned** emsdk version, clone at a tag, `emsdk install/activate <ver>`. The version is chosen in the 0.6 spike to match rustc 1.98's LLVM.
- `rustup target add wasm32-unknown-emscripten`.
- Emsdk ships node, which serves as the wasm test runner.
- *Tests:* `emcc --version` and `node --version` print as pinned.
- *Commit:* `build: add pinned emscripten SDK to dev container`

**0.3 Dev wrapper script**
- `scripts/dev.sh <cmd>` runs the command in the container.
- It mounts the repo at `/work:Z` for SELinux and uses named volumes for the cargo and emscripten caches.
- *Tests:* `scripts/dev.sh cargo --version`.
- *Commit:* `build: add dev container wrapper script`

**0.4 Cargo workspace skeleton**
- Workspace members: `crates/plymouth-3dboot` (core), `crates/plymouth-3dboot-sdl` (presenter, empty for now), and `apps/viewer` (bin, empty).
- Edition 2024; `rust-toolchain.toml` pinned to 1.98.1 with the emscripten target.
- Core crate: `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`.
- Add `LICENSE` (GPL-3.0 text) and `license = "GPL-3.0-or-later"` in the workspace package.
- Add SPDX headers (`// SPDX-License-Identifier: GPL-3.0-or-later`) to source files.
- Add `.gitignore`, `rustfmt.toml`, and workspace `[lints]`.
- *Tests:* a smoke test in core (`version()`).
- *Commit:* `chore: initialize cargo workspace under GPL-3.0-or-later`

**0.5 Wasm build configuration**
- Configure `.cargo/config.toml` for `wasm32-unknown-emscripten`:
  - runner = `node`
  - link args `-sALLOW_MEMORY_GROWTH=1`, plus `-sNODERAWFS=1` for test binaries only
  - panic and exception strategy as determined in the 0.6 spike
- *Tests:* `cargo test -p plymouth-3dboot --target wasm32-unknown-emscripten` runs the smoke test under node.
- *Commit:* `build: configure wasm32-unknown-emscripten target and node runner`

**0.6 Toolchain spike: SDL on both targets** (throwaway code in `apps/viewer`)
- Open an SDL window and fill it with a solid colour from a CPU buffer, in two variants:
  - native: run headless with the dummy driver
  - wasm: `-sUSE_SDL=3` with `emscripten_set_main_loop`
- Lock down the emsdk version, the `sdl3`/`sdl3-sys` crate versions and features (system SDL natively, Emscripten port on wasm), and the linker flags. Record them in `docs/toolchain.md`.
- *Tests:* the native binary exits 0 under `SDL_VIDEODRIVER=dummy`; `emcc` produces `.html/.js/.wasm`.
- *Commit:* `build: verify SDL builds natively and with emscripten`

**0.7 `justfile` quality gate**
- `just check` runs `fmt --check`, clippy (both targets), native `nextest`, doctests, `doc -D warnings`, the wasm build, wasm tests, and `cargo deny`. From Phase 11 it also runs the C-ABI tests (`just test-c`), and from Phase 12 the Plymouth harness (`just test-plymouth`).
- Also add `just fmt|test|test-wasm|golden-update|cov|viewer|viewer-web|browser-smoke`.
- *Done when:* `scripts/dev.sh just check` is green.
- *Commit:* `build: add justfile quality gate for native and wasm`

**0.8 cargo-deny policy**
- `deny.toml` allows only GPL-3.0-compatible licences and denies known advisories.
- *Commit:* `build: add cargo-deny licence and advisory policy`

**0.9 Vendor the fixture**
- Copy the sample to `tests/fixtures/n64_logo/`, renaming to remove spaces and fixing the `mtllib` reference.
- Add provenance to `tests/fixtures/README.md`.
- *Commit:* `test: vendor N64 logo sample model as fixture`

**0.10 Docs**
- `README.md`: purpose, the container, both targets, and the change protocol.
- `docs/conventions.md`: the conventions from §1.3.
- *Commit:* `docs: add README and conventions`

---

### Phase 1: Foundations

**1.1 Math helpers (`math`)**
- Re-export `glam`. Add `perspective_rh_gl`, `look_at_rh`, the viewport matrix, and `Aabb` (transform, union).
- *Tests:* known projections; near and far planes map to depth 0 and 1; a transformed AABB contains the transformed corners (proptest).
- *Commit:* `feat(math): add projection, viewport and AABB helpers`

**1.2 Colour (`color`)**
- `LinearRgba` with sRGB encode and decode, and `to_rgba8`.
- *Tests:* all 256 values round-trip; reference values; clamping.
- *Commit:* `feat(color): add linear/sRGB colour types and conversions`

**1.3 Framebuffer (`target`)**
- `ColorBuffer` (RGBA8, presentation-ready, `as_bytes()` for SDL) and `DepthBuffer` (f32).
- *Tests:* indexing, clear values, byte layout, bounds.
- *Commit:* `feat(target): add colour and depth buffers`

**1.4 PNG I/O**
- Encode to and decode from `Vec<u8>`, plus a path helper (native only, `cfg`-gated).
- *Tests:* a 2×2 pattern round-trips.
- *Commit:* `feat(io): encode framebuffer to PNG`

**1.5 Golden-image harness (`tests/common/golden.rs`)**
- Tolerance, diff images, `UPDATE_GOLDEN`.
- It must work under node through `NODERAWFS`.
- *Tests:* identical images pass; a change within tolerance passes; a change beyond tolerance fails; a missing golden fails. All of this runs on both targets.
- *Commit:* `test: add golden-image comparison harness`

---

### Phase 2: Rasterization core (2D)

**2.1 Fixed-point edge setup**
- 24.8 subpixel snapping, `i64` edge equations, and a documented maximum resolution.
- *Tests:* sign tests, snapping, no overflow at the extremes.
- *Commit:* `feat(raster): add fixed-point edge function setup`

**2.2 Coverage with top-left rule**
- *Tests:*
  - exact pixel sets for small triangles
  - degenerate triangles cover 0 pixels
  - **property:** a quad split into two triangles covers every pixel exactly once
  - **property:** a fan never double-covers a pixel
  - offscreen and partially offscreen triangles are handled
- *Commit:* `feat(raster): rasterize triangle coverage with top-left fill rule`

**2.3 Winding & culling**
- *Tests:* CW and CCW triangles under each `CullMode`.
- *Commit:* `feat(raster): add winding detection and face culling`

**2.4 Barycentric interpolation**
- *Tests:* one-hot at the vertices; ⅓ at the centroid; the weights sum to 1 (proptest).
- *Commit:* `feat(raster): interpolate barycentric attributes`

**2.5 Golden: 2D primitives**
- `raster_rgb_triangle.png` and `raster_overlap.png`.
- *Commit:* `test(raster): add golden images for 2D triangle rendering`

---

### Phase 3: 3D pipeline

**3.1 Vertex stage**
- Transform by MVP into clip space.
- *Tests:* known values.
- *Commit:* `feat(pipeline): add vertex transform stage`

**3.2 Near-plane clipping in homogeneous space**
- *Tests:*
  - inside and outside triangles
  - 1 vertex behind the near plane → 2 triangles; 2 behind → 1
  - attribute correctness on the clipped edge
  - no `w <= 0` vertex reaches the divide (proptest)
- *Commit:* `feat(pipeline): clip triangles against near plane in clip space`

**3.3 Perspective divide & viewport**
- *Tests:* NDC corners map to framebuffer corners; depth maps to `[0,1]`.
- *Commit:* `feat(pipeline): add perspective divide and viewport mapping`

**3.4 Perspective-correct interpolation**
- *Tests:* a receding checker matches the analytic values; an affine control case differs.
- *Commit:* `feat(raster): perspective-correct varying interpolation`

**3.5 Depth test**
- *Tests:* output is independent of draw order (byte-identical); the closer triangle wins; equal-depth behaviour.
- *Commit:* `feat(pipeline): add depth buffer testing`

**3.6 Renderer + `Shader` trait** (generic, not `dyn`, in hot loops)
- *Tests:* golden `pipeline_cube.png`.
- *Commit:* `feat(pipeline): expose renderer and shader trait`

---

### Phase 4: Scene model, cameras, shading

**4.1 Mesh & material types**
- Validation errors are returned as `Err`, never as panics.
- *Commit:* `feat(scene): add mesh and material types`

**4.2 Normal generation**
- Flat normals, and smooth normals by angle or smoothing group.
- *Tests:* cube and sphere checks.
- *Commit:* `feat(scene): generate flat and smooth vertex normals`

**4.3 Polygon triangulation**
- Fan for convex polygons, ear clipping for concave ones.
- *Tests:* the area is preserved; no triangle is inverted.
- *Commit:* `feat(scene): triangulate convex and concave polygons`

**4.4 Node hierarchy with world transforms**
- *Tests:* composition and non-uniform scale.
- *Commit:* `feat(scene): add node hierarchy with world transform evaluation`

**4.5 Cameras and `frame_aabb()`**
- *Tests:* all the AABB corners land inside NDC.
- *Commit:* `feat(scene): add perspective/orthographic cameras with auto-framing`

**4.6 Shaders: unlit, Lambert, Blinn-Phong**
- *Tests:* hand-computed fragments; sphere goldens.
- *Commits:* `feat(shading): add unlit shader`, `feat(shading): add lambert shader`, `feat(shading): add blinn-phong shader`

---

### Phase 5: OBJ/MTL loader → first render of the sample

**5.1 `ResourceResolver` trait**
- `resolve(name) -> Result<Vec<u8>>` with implementations `FsResolver` (native) and `MemResolver` (a map of embedded bytes).
- *Tests:* `MemResolver` lookups and missing resources.
- *Commit:* `feat(io): add resource resolver abstraction`

**5.2 OBJ parser** (input is `&str`)
- Handles every face format, negative indices, `g`/`o`/`usemtl`/`s`/`mtllib`, warnings for unknown directives, and line-numbered errors.
- *Tests:* inline cases; **CRLF and LF parse identically**; the N64 sample gives **48 vertices, 52 faces (44 quads, 8 triangles), and 4 materials**.
- *Commit:* `feat(io/obj): parse wavefront OBJ geometry`

**5.3 MTL parser**
- *Tests:* the N64 materials match the Readme colours ÷255 (±1/255).
- *Commit:* `feat(io/obj): parse MTL materials`

**5.4 OBJ → `Scene`** (through the resolver)
- *Tests:* **96 triangles**; AABB ≈ x∈[-29.98, 29.97], y∈[0, 57.46], z∈[-30.26, 29.87].
- *Commit:* `feat(io/obj): convert OBJ/MTL to scene`

**5.5 First golden render**
- `n64_obj_unlit.png` at 256×256, from a fixed 3/4 view.
- *Tests:* the **exact palette check** passes: every pixel is one of the 4 Readme colours or the background. This also runs on wasm.
- *Commit:* `test: add golden render of N64 logo from OBJ`

---

### Phase 6: SDL presenter & viewer (native + wasm)

**6.1 Presenter**
- `plymouth-3dboot-sdl::Presenter` owns the window, renderer and streaming texture.
- `present(&ColorBuffer)` uploads a frame; the texture is recreated when the frame size changes.
- *Tests (headless, dummy driver, software renderer):*
  - present a known pattern, read it back with `SDL_RenderReadPixels`, and check the bytes are identical
  - **the RGBA byte order is correct** (a red pixel stays red)
  - resizing works
- *Commit:* `feat(sdl): add framebuffer presenter`

**6.2 Main-loop abstraction**
- `run(app)`: natively a `loop` with vsync or a frame cap; on Emscripten, `emscripten_set_main_loop_arg`. The FFI is isolated in one small `unsafe` module.
- The app logic is a pure state machine, `App::update(events, dt) -> Control`, so it is unit-testable without SDL.
- *Tests:* state-machine tests for quit, pause, and time accumulation; a native headless run exits after N frames.
- *Commit:* `feat(sdl): add native/emscripten main loop abstraction`

**6.3 Input mapping**
- Map SDL events to app events: quit, pause, orbit drag, zoom, and model reset.
- *Tests:* mapping tests built from synthetic SDL events.
- *Commit:* `feat(sdl): map SDL input events to viewer actions`

**6.4 Viewer app (native)**
- `apps/viewer`: load the model from a path, orbit the camera, render, present.
- *Tests:* a headless smoke run renders N frames and exits 0.
- *Commit:* `feat(viewer): add native SDL model viewer`

**6.5 Viewer app (web)**
- Build with `-sUSE_SDL=3`, embed assets with `--preload-file` (or `MemResolver` with `include_bytes!`), and use a minimal HTML shell.
- *Tests:* the build produces `.html/.js/.wasm`; the size budget is recorded.
- *Commit:* `feat(viewer): build viewer for the web with emscripten`

**6.6 Browser smoke test**
- `just browser-smoke`: serve the build, take a screenshot with headless Chromium after a virtual-time budget, and check the palette (N64 colours are present on the canvas).
- Chromium is added to the container.
- *Commit:* `test(viewer): add headless browser smoke test`

---

### Phase 7: COLLADA loader (static)

**7.1 XML layer**
- `roxmltree`, `#id` resolution, number arrays.
- *Commit:* `feat(io/collada): add XML document and id resolution layer`

**7.2 Geometry**
- `<triangles>`/`<polylist>`/`<polygons>` with multiple inputs and offsets.
- *Tests:* **48 positions, 152 normals, 4 groups (30/50/8/8 = 96 triangles)**; the normals are unit length.
- *Commit:* `feat(io/collada): parse mesh geometry`

**7.3 Materials and effects**
- `blinn`/`phong`/`lambert`/`constant` and the FCOLLADA `emission_level`.
- *Tests:* colours match the Readme.
- *Commit:* `feat(io/collada): resolve materials and effects`

**7.4 Visual scene**
- TRS and `<matrix>` in document order, plus `<unit>` and `Z_UP→Y_UP` conversion.
- *Tests:* the hand-computed world matrix matches; the model stands upright.
- *Commit:* `feat(io/collada): build node hierarchy with unit and up-axis conversion`

**7.5 Cross-format consistency**
- *Tests:* the DAE and OBJ renders have the same palette and silhouette IoU ≥ 0.95; golden `n64_dae_unlit.png`.
- *Commit:* `test: cross-check COLLADA and OBJ renders of N64 logo`

**7.6 Viewer can open `.dae`**
- *Commit:* `feat(viewer): load COLLADA models`

---

### Phase 8: Animation system

**8.1 Keyframe tracks**
- Step / Linear / CubicSpline interpolation, with shortest-arc slerp for rotations.
- *Tests:* clamping, exact keys, a cubic polynomial, unit-length quaternions (proptest), rejection of invalid time arrays.
- *Commit:* `feat(anim): add keyframe tracks with step/linear/cubic interpolation`

**8.2 Channels, clips, `WrapMode`** (Clamp / Loop / PingPong)
- *Commit:* `feat(anim): add animation channels and clips`

**8.3 Pose evaluation**
- `scene.pose_at(&clip, t)`.
- *Tests:* analytic results for a 2-level hierarchy.
- *Commit:* `feat(anim): evaluate scene pose at time t`

**8.4 Procedural clip builders**
- `turntable` and `bounce`.
- *Tests:* a 90° rotation at period/4.
- *Commit:* `feat(anim): add procedural clip builders`

**8.5 Animated camera nodes**
- *Tests:* orbit distance stays constant.
- *Commit:* `feat(anim): support animated camera nodes`

**8.6 Frame-sequence rendering**
- Times are computed from the integer frame index.
- *Tests:* 100 frames for 3.333 s @ 30 fps; no time drift; determinism.
- *Commit:* `feat(render): render animation clips to frame sequences`

**8.7 Golden turntable frames**
- Frames 0, 25, 50 and 75; under looping, frame 100 equals frame 0. Runs on both targets.
- *Commit:* `test(anim): add golden frames for N64 turntable`

**8.8 Real-time playback in the viewer**
- Animation time comes from the main-loop clock (pause and speed controls). Rendering is still a pure function of `t`.
- *Tests:* state-machine tests for clock, pause and speed.
- *Commit:* `feat(viewer): play animation clips in real time`

---

### Phase 9: COLLADA animation import

**9.1 Samplers and channels**
- Targets such as `node/rotateY.ANGLE` and `node/transform`.
- *Tests:* synthetic fixtures. The N64 DAE gives 0 clips and parses cleanly.
- *Commit:* `feat(io/collada): parse animation samplers and channels`

**9.2 BEZIER interpolation (HERMITE approximated)**
- *Tests:* synthetic tangent fixture.
- *Commit:* `feat(io/collada): support bezier interpolation`

**9.3 `<library_animation_clips>`**
- *Tests:* multi-clip fixture.
- *Commit:* `feat(io/collada): parse animation clips`

**9.4 Animated N64 fixture**
- `n64_logo_spin.dae` must match the 8.7 goldens within tolerance.
- *Commit:* `test(io/collada): add animated N64 fixture and cross-check against procedural clip`

---

### Phase 10: Output formats & public API

**10.1 PNG sequence writer**
- Native, `cfg`-gated.
- *Commit:* `feat(io): write PNG frame sequences`

**10.2 Animated GIF**
- Feature `gif`; encodes to bytes, so it works on both targets.
- *Commit:* `feat(io): encode animated GIF output`

**10.3 High-level API**
- `Model::from_resolver`, `Model::load(path)` (native only), the renderer builder, and error enums.
- *Tests:* integration tests through the public API only, plus doctests.
- *Commit:* `feat: add high-level load-and-render API`

**10.4 Offline render example**
- `examples/animate.rs` renders a clip to PNGs or a GIF without SDL.
- *Commit:* `docs(examples): add offline animation render example`

**10.5 API review**
- `#[non_exhaustive]`, minimal public surface.
- *Commit:* `refactor: tighten public API surface`

---

### Phase 11: C interface (`crates/plymouth-3dboot-capi`)

The ABI is designed for C callers such as a Plymouth plugin:
- Opaque handles: `p3b_scene`, `p3b_renderer`.
- Every function returns a `p3b_status` enum. `p3b_last_error()` gives a thread-local message.
- **No panic crosses the boundary:** every entry point is wrapped in `catch_unwind` and returns `P3B_STATUS_PANIC`.
- No Rust types appear in the header. All sizes are explicit (`uint32_t`, `size_t`).
- Version functions: `p3b_version()` and `p3b_abi_version()`.
- The C library is native-only (Linux `cdylib` + `staticlib`).
- The shared library is `libplymouth_3dboot.so.0`, with soname and pkg-config file `plymouth-3dboot.pc` generated by `cargo-c`.

**11.1 Crate skeleton & build**
- Use `cargo-c` (`cargo cbuild`/`cinstall`) for soname, pkg-config and header installation. `cbindgen.toml` sets the `p3b_` prefix, include guards and the SPDX header.
- *Tests:* `cargo cbuild` produces the `.so`, `.a`, `.h` and `.pc`. `readelf -d` shows the expected soname. `nm -D` exports only `p3b_*` symbols (checked by a script).
- *Commit:* `build(capi): add C API crate with cargo-c and cbindgen`

**11.2 Status codes, error reporting, panic guard**
- `p3b_status`, `p3b_last_error`, `p3b_version`, `p3b_abi_version`, and an internal `ffi_guard(|| …)` helper.
- *Tests:* a deliberate panic in a test-only entry point returns `PANIC` and does not abort. The error message is readable from the same thread and isolated between threads.
- *Commit:* `feat(capi): add status codes, error reporting and panic containment`

**11.3 Scene loading**
- `p3b_scene_load_file(path, out)` and `p3b_scene_load_memory(format, data, len, resolver_cb, user_data, out)`. The callback resolves MTL and other side files from memory.
- `p3b_scene_free`, `p3b_scene_clip_count`, `p3b_scene_clip_duration`, `p3b_scene_bounds`.
- *Tests:* the N64 sample loads from a file and from memory; NULL pointers, bad UTF-8, an unknown format and truncated data each give the right status and never crash. Calling free with NULL is a no-op.
- *Commit:* `feat(capi): load scenes from file or memory`

**11.4 Renderer & frame output**
- `p3b_renderer_new(width, height, const p3b_render_options*)`. Options: shading mode, background colour, cull mode, AA level, camera override (eye, target, fov) or auto-frame.
- `p3b_renderer_free`.
- `p3b_render_frame(renderer, scene, clip_index, double time_s, uint8_t* dst, uint32_t stride_bytes, p3b_pixel_format fmt)`. Formats:
  - `RGBA8888` bytes
  - `ARGB32_PREMULTIPLIED` native-endian `uint32_t`, which is Plymouth's pixel-buffer format
  - `BGRA8888` bytes
- Calls are re-entrant. A renderer is not shared between threads (documented).
- *Tests:* each format against hand-converted expected pixels; stride larger than `width*4` leaves the padding bytes untouched; a buffer that is too small is rejected (a length parameter is required); `clip_index` out of range returns an error; the rendered frame matches the Rust golden for the same `t`.
- *Commit:* `feat(capi): render animation frames into caller buffers`

**11.5 Header & ABI stability checks**
- Commit the generated `include/plymouth-3dboot.h`. `just check` regenerates it and fails on any diff.
- Add `docs/c-api.md` (ownership rules, threading, error handling, versioning policy).
- *Commit:* `docs(capi): commit generated header and document C API contract`

**11.6 C test suite**
- `capi/tests/*.c`: load → render → compare checksum or golden, error paths, and repeated load/free in a loop. Each test is built twice, against the shared and the static library.
- `just test-c` builds them with `cc -Wall -Wextra -Werror` and runs them under `valgrind --error-exitcode=1 --leak-check=full`.
- *Commit:* `test(capi): add C test programs run under valgrind`

**11.7 C example**
- `examples/c/render_png.c`: renders a frame and writes a PPM or PNG, using only the public header and pkg-config.
- *Commit:* `docs(examples): add C API usage example`

---

### Phase 12: Plymouth splash plugin & theme (`plymouth/`)

**12.1 Integration spike**
- Read the installed Plymouth headers (`ply-boot-splash-plugin.h`, `ply-pixel-buffer.h`, `ply-pixel-display.h`) to confirm the plugin interface, pixel format, event-loop timer API and display callbacks.
- Write the findings to `docs/plymouth.md`, including the Plymouth version range we support.
- *Commit:* `docs(plymouth): document splash plugin integration points`

**12.2 Plugin skeleton**
- A C splash plugin (`plymouth-3dboot.so`, exporting `ply_boot_splash_plugin_get_interface`) built with meson against `ply-splash-core` and `libplymouth_3dboot`.
- It reads `ModelFile`, `Clip`, `Background`, `Shading` and `Fps` from the theme's `.plymouth` keyfile.
- *Tests:* builds with `-Werror`; `nm -D` exports the entry symbol.
- *Commit:* `feat(plymouth): add splash plugin skeleton`

**12.3 Rendering loop**
- On each display, create a `p3b_renderer` sized to the display.
- Drive frames from Plymouth's event-loop timer: time comes from a monotonic clock, frames that fall behind are dropped, and nothing renders while hidden.
- Render directly into the display's `ply_pixel_buffer` through `ARGB32_PREMULTIPLIED`, and handle multiple heads and resolution changes.
- *Commit:* `feat(plymouth): render animated model each frame`

**12.4 Boot-splash features**
- Password and question prompts, plus message and progress display. At minimum, defer these to Plymouth's built-in label/entry controls, as the two-step plugin does.
- Handle the boot / shutdown / update modes.
- *Commit:* `feat(plymouth): support prompts, messages and boot modes`

**12.5 Test harness**
- `plymouth/tests/harness.c` `dlopen`s the plugin, provides a fake event loop and pixel display, ticks N frames at fixed times, and dumps the pixel buffer.
- It compares the dump with the core goldens (after converting from ARGB32) and runs under valgrind.
- *Commit:* `test(plymouth): add headless plugin harness with golden comparison`

**12.6 Theme & packaging**
- `plymouth/theme/plymouth-3dboot.plymouth` with the N64 model as the demo.
- Install rules (meson), and a dracut note: add the library, model files and plugin to the initramfs (`install_items`).
- `docs/plymouth.md` covers install, `plymouth-set-default-theme -R`, and debugging (`plymouthd --debug`).
- *Commit:* `feat(plymouth): add demo theme and install rules`

**12.7 Boot-environment budgets**
- Measure the library and plugin size, the time to first frame, and the frame time at 1080p for the N64 scene. Record them in `docs/perf.md` and set limits that tests check.
- *Commit:* `perf(plymouth): record boot-time size and latency budgets`

**12.8 Manual VM verification** (documented, not automated)
- Boot a Fedora VM (virtio-gpu / simpledrm) with the theme in the initramfs and check boot, shutdown and LUKS password prompts.
- Record the checklist in `docs/plymouth.md`.
- *Commit:* `docs(plymouth): add VM verification checklist`

---

### Phase 13: Quality & performance

**13.1 Supersampling anti-aliasing (SSAA), then MSAA**
- *Tests:* goldens are updated together with an edge-quality test.
- *Commit:* `feat(raster): add supersampling anti-aliasing`

**13.2 Benchmarks**
- `criterion` natively, plus wasm frame timing for the N64 scene under node. Record a baseline in `docs/perf.md`, with a frame-time budget for the viewer at its target resolution.
- *Commit:* `perf: add native and wasm benchmarks`

**13.3 Tiled rasterization**
- A single-threaded tiled path (better cache use on both targets) and a `rayon` feature that is **native only**. Wasm stays single-threaded unless pthreads/COOP-COEP is adopted later.
- *Tests:* tiled output equals untiled output, and parallel equals serial, byte for byte.
- *Commit:* `perf(raster): add tiled and parallel rasterization`

**13.4 ≥85 % line coverage on core**
- *Commit:* `test: raise coverage of core modules`

---

### Phase 14: Deferred (decide at this point)

Candidates: glTF 2.0 import, skeletal skinning, morph targets, textures. This
phase is reviewed and planned in detail when Phase 13 is complete.

## 3. Dependency budget

| Crate | Purpose | Where | Licence |
|---|---|---|---|
| `glam` | math | core | MIT/Apache-2.0 |
| `png` | PNG | core | MIT/Apache-2.0 |
| `thiserror` | errors | core | MIT/Apache-2.0 |
| `roxmltree` | COLLADA XML | core | MIT/Apache-2.0 |
| `gif` (feature) | GIF output | core | MIT/Apache-2.0 |
| `rayon` (feature, native) | parallel tiles | core | MIT/Apache-2.0 |
| `sdl3` / `sdl3-sys` | presenter | sdl crate | Zlib (SDL), MIT/Apache (bindings) |
| build: `cbindgen` (via `cargo-c`) | C header | capi crate | MPL-2.0 (build tool only, not linked) |
| system: `ply-splash-core` | Plymouth plugin API | plymouth/ | GPL-2.0-or-later (compatible with GPL-3.0-or-later) |
| dev: `proptest`, `approx`, `criterion` | tests/bench | dev | MIT/Apache-2.0 |

The OBJ/MTL parser is written in-house.

## 4. Repository layout

```
Containerfile  justfile  deny.toml  rust-toolchain.toml  LICENSE
.cargo/config.toml
scripts/dev.sh
crates/
  plymouth-3dboot/        core: math, color, target, raster, pipeline, scene,
                          shading, anim, io/{png,obj,mtl,collada,gif}
    tests/                integration + golden tests (tests/golden/*.png)
  plymouth-3dboot-sdl/    presenter, main loop (native | emscripten), input
  plymouth-3dboot-capi/   C ABI (cdylib + staticlib), cbindgen.toml
    include/plymouth-3dboot.h   generated, committed
    tests/*.c             C tests (valgrind)
apps/viewer/              SDL viewer binary (native + web shell.html)
plymouth/                 C splash plugin (meson), theme/, tests/harness.c
tests/fixtures/           n64_logo/, collada_anim/
docs/                     conventions.md, toolchain.md, perf.md, c-api.md, plymouth.md
```

## 5. Risks & mitigations

| Risk | Mitigation |
|---|---|
| The emsdk ↔ rustc LLVM version or flags mismatch | The 0.6 spike pins a known-good combination, recorded in `docs/toolchain.md`. The container makes it reproducible. |
| SDL crate support on Emscripten is fragile | `sdl3-sys` documents Emscripten builds. SDL is isolated in its own crate, so the core never depends on it. Fallback: raw `sdl3-sys` FFI only for the few calls we need. |
| Native and wasm renders diverge | The core forbids target-specific code paths. The golden suite runs on both targets. |
| Wasm performance at the target resolution | Benchmarks (13.2), tiling (13.3), and a configurable internal resolution scaled up by SDL. |
| Golden-image flakiness | Run in the pinned container with tolerance comparison; regenerate only explicitly, reviewing the diff images. |
| Edge cracks or double hits | Fixed-point arithmetic plus the top-left rule, guarded by property tests. |
| Near-plane division by zero | Clip before the divide, guarded by a proptest. |
| A panic or UB across the C ABI takes down plymouthd (and the boot splash) | `catch_unwind` on every entry point; all C inputs are validated; fuzz/property tests on FFI inputs; C tests under valgrind. |
| The Plymouth plugin API differs between versions or distros | The 12.1 spike pins a supported version range; the harness compiles against real headers in the container. |
| Initramfs size and boot latency | Size and latency budgets (12.7); `panic = "abort"` is **not** used in the C library (it would prevent containment), but release builds use LTO and strip symbols. |
| COLLADA's large spec surface | Support a documented subset; warn on unknown elements; synthetic fixtures for each supported feature. |
| The sample has no animation | Procedural clips, synthetic COLLADA fixtures, and the animated N64 derivative. |

## 6. Decisions log

| Decision | Choice |
|---|---|
| Crate name | `plymouth-3dboot` (lib `plymouth_3dboot`) |
| Licence | GPL-3.0-or-later |
| Branch | `main` |
| Math | `glam` |
| Presentation | **SDL3**: the system library natively, Emscripten's port (`-sUSE_SDL=3`) on wasm |
| Consumer | Plymouth theme, through a C ABI (`p3b_*`) and a C splash plugin |
| Targets | `x86_64-unknown-linux-gnu`, `wasm32-unknown-emscripten` |
| Phase 14 scope | deferred until Phase 13 is done |
