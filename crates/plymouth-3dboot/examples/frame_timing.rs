// SPDX-License-Identifier: GPL-3.0-or-later
//! Prints the median frame time of the embedded N64 logo at 640x480, so
//! native and WebAssembly (`just bench-wasm`) can be compared.

use std::time::Instant;

use plymouth_3dboot::io::MemResolver;
use plymouth_3dboot::math::Vec3;
use plymouth_3dboot::{Format, FrameSettings, Model, WrapMode};

fn main() {
    const OBJ: &str = include_str!("../../../tests/fixtures/n64_logo/n64_logo.obj");
    const MTL: &[u8] = include_bytes!("../../../tests/fixtures/n64_logo/n64_logo.mtl");
    let resolver = MemResolver::new()
        .with("n64_logo.mtl", MTL)
        .expect("valid name");
    let model = Model::from_source(Format::Obj, OBJ, &resolver)
        .expect("embedded model")
        .with_turntable(Vec3::Y, 6.0);
    let mut renderer = model
        .renderer(Some(0), WrapMode::Loop, FrameSettings::new(640, 480))
        .expect("renderer");
    let mut times: Vec<f64> = (0..60)
        .map(|i| {
            let start = Instant::now();
            renderer.render_at(f64::from(i) * 0.05).expect("frame");
            start.elapsed().as_secs_f64() * 1e3
        })
        .collect();
    times.sort_by(f64::total_cmp);
    println!("640x480 median frame: {:.2} ms", times[times.len() / 2]);
}
