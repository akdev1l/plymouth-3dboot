// SPDX-License-Identifier: GPL-3.0-or-later
//! Interactive viewer for `plymouth-3dboot` models.
//!
//! Usage: `plymouth-3dboot-viewer [MODEL.obj|MODEL.dae] [--shading unlit|lambert|blinn-phong]
//! [--turntable SECONDS | --still] [--frames N] [--max-resolution PX]`
//!
//! Without a model the embedded N64 logo is shown. A model's own animation
//! (COLLADA clips) plays if it has one; otherwise it spins on a turntable
//! (one turn per 6 s). `--turntable SECONDS` forces a turntable, `--still`
//! disables animation.
//! Drag with the left mouse button to orbit, scroll to zoom, Space to pause,
//! +/- to change speed, R to reset, Escape or Q to quit.

mod model;
mod viewer;

use plymouth_3dboot::Model;
use plymouth_3dboot::math::Vec3;
use plymouth_3dboot::shading::ShadingModel;
use plymouth_3dboot_sdl::{Presenter, RunOptions, WindowConfig, run};
use viewer::{Viewer, ViewerOptions};

/// Parsed command line.
#[derive(Debug)]
struct Args {
    model: Option<std::path::PathBuf>,
    options: ViewerOptions,
    frames: Option<u64>,
    animation: Animation,
}

/// What to animate.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Animation {
    /// The model's first clip, or a 6 s turntable if it has none.
    Auto,
    /// A turntable with this period in seconds.
    Turntable(f32),
    /// Nothing.
    Still,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            model: None,
            options: ViewerOptions::default(),
            frames: None,
            animation: Animation::Auto,
        }
    }
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut out = Args::default();
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--shading" => {
                out.options.shading = match value("--shading")?.as_str() {
                    "unlit" => ShadingModel::Unlit,
                    "lambert" => ShadingModel::Lambert,
                    "blinn-phong" => ShadingModel::BlinnPhong,
                    other => return Err(format!("unknown shading model {other:?}")),
                }
            }
            "--turntable" => {
                let period: f32 = value("--turntable")?
                    .parse()
                    .map_err(|e| format!("--turntable: {e}"))?;
                if !(period.is_finite() && period > 0.0) {
                    return Err("--turntable needs a positive number of seconds".into());
                }
                out.animation = Animation::Turntable(period);
            }
            "--still" => out.animation = Animation::Still,
            "--frames" => {
                out.frames = Some(
                    value("--frames")?
                        .parse()
                        .map_err(|e| format!("--frames: {e}"))?,
                )
            }
            "--max-resolution" => {
                out.options.max_resolution = value("--max-resolution")?
                    .parse()
                    .map_err(|e| format!("--max-resolution: {e}"))?;
            }
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            path if out.model.is_none() => out.model = Some(path.into()),
            extra => return Err(format!("unexpected argument {extra:?}")),
        }
    }
    Ok(out)
}

fn main() {
    if let Err(e) = try_main() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn try_main() -> Result<(), String> {
    let args = parse_args(std::env::args().skip(1))?;
    let model = match &args.model {
        Some(path) => Model::load(path),
        None => model::embedded_n64(),
    }
    .map_err(|e| e.to_string())?;
    for w in &model.warnings {
        eprintln!("warning: {w}");
    }
    let presenter = Presenter::new(&WindowConfig {
        title: "plymouth-3dboot viewer".into(),
        ..WindowConfig::default()
    })
    .map_err(|e| e.to_string())?;
    let model = match (args.animation, model.clips.is_empty()) {
        (Animation::Turntable(period), _) => model.with_turntable(Vec3::Y, period),
        (Animation::Auto, true) => model.with_turntable(Vec3::Y, 6.0),
        _ => model,
    };
    let clip = match args.animation {
        Animation::Still => None,
        // The model's own clip, or the turntable just added (the last one).
        Animation::Auto => model.clips.first().cloned(),
        Animation::Turntable(_) => model.clips.last().cloned(),
    };
    let scene = model.scene;
    let viewer = Viewer::new(scene, clip, args.options);
    let options = RunOptions {
        max_frames: args.frames,
        ..RunOptions::default()
    };
    run(presenter, viewer, options).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        parse_args(args.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn parses_options_and_model() {
        let a = parse(&[
            "model.obj",
            "--shading",
            "lambert",
            "--frames",
            "3",
            "--max-resolution",
            "200",
        ])
        .unwrap();
        assert_eq!(a.model.as_deref(), Some(std::path::Path::new("model.obj")));
        assert_eq!(
            (a.options.shading, a.frames, a.options.max_resolution),
            (ShadingModel::Lambert, Some(3), 200)
        );
        assert!(parse(&[]).unwrap().model.is_none());
        assert_eq!(parse(&[]).unwrap().animation, Animation::Auto);
        assert_eq!(
            parse(&["--turntable", "2.5"]).unwrap().animation,
            Animation::Turntable(2.5)
        );
        assert_eq!(parse(&["--still"]).unwrap().animation, Animation::Still);
    }

    #[test]
    fn rejects_bad_arguments() {
        for bad in [
            &["--turntable", "0"][..],
            &["--turntable", "x"],
            &["--shading", "toon"],
            &["--frames"],
            &["--frames", "x"],
            &["--bogus"],
            &["a.obj", "b.obj"],
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }
}
