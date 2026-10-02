<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# Test fixtures

Fixtures are stored byte-exact (`.gitattributes` marks them `-text`). Some
deliberately use CRLF line endings.

## `n64_logo/`

N64 logo model by **Shadowth117**, from `Readme.txt`: *"No credit needed for
use."* Exported from 3ds Max 2009 (`.obj`/`.mtl` by the guruware OBJ
exporter, `.dae` by ColladaMax 3.05B).

| File | Original name | Changes | SHA-256 of original |
|---|---|---|---|
| `n64_logo.obj` | `N64 Logo.obj` | `mtllib` line now references `n64_logo.mtl` | `0500abe1b841e8a3833e49f4c56488d259c835e4f889400e4fc411a14c29bd1d` |
| `n64_logo.mtl` | `N64 Logo.mtl` | none | `7ff7aecf53def8042f5b47c00fcd4aa3ec45336bcf711c62bdf1a0ab81b27aeb` |
| `n64_logo.dae` | `N64 Logo.DAE` | none | `02ed74c4bba440f1e22f50f4ff66652749b767108c069329c841d92593a3486e` |
| `Readme.txt` | `Readme.txt` | none | `c5b52f3f0162c19ddbf06d99f9bd7084cd59ad229ebec6d5ae7b336b7748616d` |

Known properties, asserted by tests as the loaders land:

- OBJ: 48 vertices, 52 faces (44 quads + 8 triangles, so 96 triangles), 4 materials, no normals or UVs, Y-up, CRLF.
- DAE: 96 triangles in 4 groups (30/50/8/8), 152 normals, `Z_UP`, unit = inch, and **no animation**.
- Reference colours (sRGB, from the Readme): green (6, 147, 48), blue (2, 34, 169), red (255, 24, 19), yellow (255, 192, 1). Displayed unlit ("self illumination full").

## `collada_anim/`

Small synthetic COLLADA 1.4.1 documents (GPL-3.0-or-later, written for this
project). Each animates a single triangle to exercise one feature of the
animation importer:

| File | Feature |
|---|---|
| `rotate_y.dae` | `rotate.ANGLE` channel, LINEAR: one turn about +Y over 4 s |
| `matrix_step.dae` | whole-`matrix` channel (row-major `float4x4`), STEP |
| `translate_x.dae` | `translate.X` member and whole-vector channels, nested `<animation>` |
| `translate_bezier.dae` | BEZIER `translate.X` ease-in-out (control points at one third), 0 → 3 → 0 over 2 s |
| `clips.dae` | `<library_animation_clips>`: `slide` (0–1 s) and `lift` (0.5–2 s) over the `translate_x.dae` animations |
