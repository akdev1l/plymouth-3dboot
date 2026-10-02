// SPDX-License-Identifier: GPL-3.0-or-later
//! Renders a model's animation offline to PNG frames or an animated GIF.
//!
//! ```text
//! cargo run --example animate -- MODEL --out OUTPUT [--frames N] [--fps F]
//!     [--size WxH] [--turntable SECONDS] [--shading unlit|lambert|blinn-phong]
//! ```
//!
//! `OUTPUT` ending in `.gif` writes an animated GIF; anything else is a
//! directory receiving `frame0000.png`, `frame0001.png`, ... The model's
//! first clip is played (or a turntable if it has none, or if
//! `--turntable` is given). Without `--frames`, one full clip is rendered.

use std::path::PathBuf;

use plymouth_3dboot::io::sequence::PngSequence;
use plymouth_3dboot::math::Vec3;
use plymouth_3dboot::render::{frame_count, frame_time};
use plymouth_3dboot::shading::ShadingModel;
use plymouth_3dboot::{FrameSettings, Model, WrapMode};

struct Args {
    model: PathBuf,
    out: PathBuf,
    frames: Option<u64>,
    fps: f64,
    size: (u32, u32),
    turntable: Option<f32>,
    shading: ShadingModel,
}

fn parse() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let (mut model, mut out) = (None, None);
    let mut parsed = Args {
        model: PathBuf::new(),
        out: PathBuf::new(),
        frames: None,
        fps: 30.0,
        size: (256, 256),
        turntable: None,
        shading: ShadingModel::Unlit,
    };
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--out" => out = Some(PathBuf::from(value()?)),
            "--frames" => {
                parsed.frames = Some(value()?.parse().map_err(|e| format!("--frames: {e}"))?)
            }
            "--fps" => parsed.fps = value()?.parse().map_err(|e| format!("--fps: {e}"))?,
            "--turntable" => {
                parsed.turntable = Some(value()?.parse().map_err(|e| format!("--turntable: {e}"))?)
            }
            "--size" => {
                let v = value()?;
                let (w, h) = v.split_once('x').ok_or("--size expects WxH")?;
                parsed.size = (
                    w.parse().map_err(|e| format!("--size: {e}"))?,
                    h.parse().map_err(|e| format!("--size: {e}"))?,
                );
            }
            "--shading" => {
                parsed.shading = match value()?.as_str() {
                    "unlit" => ShadingModel::Unlit,
                    "lambert" => ShadingModel::Lambert,
                    "blinn-phong" => ShadingModel::BlinnPhong,
                    other => return Err(format!("unknown shading model {other:?}")),
                }
            }
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            path => model = Some(PathBuf::from(path)),
        }
    }
    parsed.model = model.ok_or("missing MODEL")?;
    parsed.out = out.ok_or("missing --out OUTPUT")?;
    if !(parsed.fps.is_finite() && parsed.fps > 0.0) {
        return Err("--fps must be positive".into());
    }
    Ok(parsed)
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse()?;
    let mut model = Model::load(&args.model)?;
    for w in &model.warnings {
        eprintln!("warning: {w}");
    }
    let clip_index = if let Some(period) = args
        .turntable
        .or_else(|| model.clips.is_empty().then_some(6.0))
    {
        model = model.with_turntable(Vec3::Y, period);
        model.clips.len() - 1
    } else {
        0
    };
    let duration = f64::from(model.clips[clip_index].duration());
    let count = args
        .frames
        .unwrap_or_else(|| frame_count(duration, args.fps));
    let settings = FrameSettings {
        shading: args.shading,
        ..FrameSettings::new(args.size.0, args.size.1)
    };
    let mut renderer = model.renderer(Some(clip_index), WrapMode::Loop, settings)?;

    if args
        .out
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("gif"))
    {
        let frames = (0..count)
            .map(|i| renderer.render_at(frame_time(0.0, i, args.fps)).cloned())
            .collect::<Result<Vec<_>, _>>()?;
        std::fs::write(
            &args.out,
            plymouth_3dboot::io::gif::encode(&frames, args.fps)?,
        )?;
        println!("wrote {count} frames to {}", args.out.display());
    } else {
        let sequence = PngSequence::new(&args.out, "frame", 4);
        for i in 0..count {
            sequence.write(i, renderer.render_at(frame_time(0.0, i, args.fps))?)?;
        }
        println!("wrote {count} frames to {}", args.out.display());
    }
    Ok(())
}
