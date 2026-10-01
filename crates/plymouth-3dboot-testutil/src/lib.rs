// SPDX-License-Identifier: GPL-3.0-or-later
//! Test support for `plymouth-3dboot` (dev-dependency only).
//!
//! The main entry point is [`golden!`], which compares a rendered
//! [`ColorBuffer`] against a committed PNG:
//!
//! ```no_run
//! # use plymouth_3dboot::{color::Rgba8, target::ColorBuffer};
//! # use plymouth_3dboot_testutil::{golden, Tolerance};
//! let image = ColorBuffer::new(4, 4, Rgba8::BLACK).unwrap();
//! golden!().assert("black_square", &image, Tolerance::EXACT);
//! ```
//!
//! Goldens live in `<crate>/tests/golden/<name>.png`. When a comparison
//! fails, `expected.png`, `actual.png` and `diff.png` are written to
//! `<target>/tmp/golden-failures/<name>/`. Set `UPDATE_GOLDEN=1` to
//! (re)write goldens instead of comparing; review the image diffs before
//! committing them.

#![forbid(unsafe_code)]

use std::fmt;
use std::path::{Path, PathBuf};

use plymouth_3dboot::color::Rgba8;
use plymouth_3dboot::io::png;
use plymouth_3dboot::target::ColorBuffer;

/// How much two images may differ and still match.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tolerance {
    /// Per-channel absolute differences up to this value are ignored.
    pub channel_threshold: u8,
    /// Largest allowed fraction (`0.0..=1.0`) of pixels that differ by more
    /// than `channel_threshold` in any channel.
    pub max_differing_fraction: f64,
}

impl Tolerance {
    /// Images must be identical.
    pub const EXACT: Self = Self {
        channel_threshold: 0,
        max_differing_fraction: 0.0,
    };
}

/// The result of comparing two images.
#[derive(Clone, Debug, PartialEq)]
pub struct Comparison {
    /// Whether the images have the same dimensions (if not, nothing else is
    /// meaningful).
    pub same_size: bool,
    /// Number of pixels that differ by more than the channel threshold.
    pub differing_pixels: usize,
    /// Total number of pixels.
    pub total_pixels: usize,
    /// Largest per-channel difference anywhere in the image.
    pub max_channel_delta: u8,
    /// Whether the comparison is within tolerance.
    pub passed: bool,
}

impl fmt::Display for Comparison {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.same_size {
            return f.write_str("image sizes differ");
        }
        #[allow(clippy::cast_precision_loss)]
        let pct = 100.0 * self.differing_pixels as f64 / self.total_pixels.max(1) as f64;
        write!(
            f,
            "{}/{} pixels differ ({pct:.3}%), max channel delta {}",
            self.differing_pixels, self.total_pixels, self.max_channel_delta
        )
    }
}

fn channel_delta(a: Rgba8, b: Rgba8) -> u8 {
    a.to_array()
        .iter()
        .zip(b.to_array())
        .map(|(x, y)| x.abs_diff(y))
        .max()
        .unwrap_or(0)
}

/// Compares `actual` against `expected` within `tolerance`.
#[must_use]
pub fn compare(expected: &ColorBuffer, actual: &ColorBuffer, tolerance: Tolerance) -> Comparison {
    let total_pixels = expected.pixels().len();
    if (expected.width(), expected.height()) != (actual.width(), actual.height()) {
        return Comparison {
            same_size: false,
            differing_pixels: total_pixels,
            total_pixels,
            max_channel_delta: 255,
            passed: false,
        };
    }
    let mut differing_pixels = 0;
    let mut max_channel_delta = 0;
    for (&e, &a) in expected.pixels().iter().zip(actual.pixels()) {
        let d = channel_delta(e, a);
        max_channel_delta = max_channel_delta.max(d);
        if d > tolerance.channel_threshold {
            differing_pixels += 1;
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let fraction = differing_pixels as f64 / total_pixels.max(1) as f64;
    Comparison {
        same_size: true,
        differing_pixels,
        total_pixels,
        max_channel_delta,
        passed: fraction <= tolerance.max_differing_fraction,
    }
}

/// Visualizes differences: pixels beyond the threshold are red (brighter
/// for larger deltas), all others are a dimmed grey copy of `expected`.
///
/// Returns `None` if the images have different sizes.
#[must_use]
pub fn diff_image(
    expected: &ColorBuffer,
    actual: &ColorBuffer,
    tolerance: Tolerance,
) -> Option<ColorBuffer> {
    if (expected.width(), expected.height()) != (actual.width(), actual.height()) {
        return None;
    }
    let mut out = ColorBuffer::new(expected.width(), expected.height(), Rgba8::BLACK).ok()?;
    for ((o, &e), &a) in out
        .pixels_mut()
        .iter_mut()
        .zip(expected.pixels())
        .zip(actual.pixels())
    {
        let d = channel_delta(e, a);
        *o = if d > tolerance.channel_threshold {
            Rgba8::new(128u8.saturating_add(d / 2), 0, 0, 255)
        } else {
            let luma = (u16::from(e.r) + u16::from(e.g) + u16::from(e.b)) / 3;
            let dim = u8::try_from(luma / 4).unwrap_or(u8::MAX);
            Rgba8::new(dim, dim, dim, 255)
        };
    }
    Some(out)
}

/// Whether [`Golden`] compares against or rewrites golden files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Compare; a missing golden is a failure.
    Verify,
    /// Write the actual image as the new golden.
    Update,
}

impl Mode {
    /// [`Mode::Update`] if the `UPDATE_GOLDEN` environment variable is set to
    /// a non-empty value other than `0`, else [`Mode::Verify`].
    #[must_use]
    pub fn from_env() -> Self {
        match std::env::var("UPDATE_GOLDEN") {
            Ok(v) if !v.is_empty() && v != "0" => Self::Update,
            _ => Self::Verify,
        }
    }
}

/// A directory of golden images plus a directory for failure artefacts.
#[derive(Clone, Debug)]
pub struct Golden {
    golden_dir: PathBuf,
    failure_dir: PathBuf,
    mode: Mode,
}

/// Creates a [`Golden`] for the calling crate's integration tests:
/// goldens in `<crate>/tests/golden`, failures under Cargo's target tmp
/// directory (or the system temp directory outside integration tests), and
/// the mode from `UPDATE_GOLDEN`.
#[macro_export]
macro_rules! golden {
    () => {
        $crate::Golden::new(
            ::std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden"),
            // Cargo sets CARGO_TARGET_TMPDIR for integration tests only.
            match option_env!("CARGO_TARGET_TMPDIR") {
                Some(dir) => ::std::path::PathBuf::from(dir),
                None => ::std::env::temp_dir(),
            }
            .join("golden-failures"),
            $crate::Mode::from_env(),
        )
    };
}

impl Golden {
    /// Creates a golden store.
    #[must_use]
    pub fn new(
        golden_dir: impl Into<PathBuf>,
        failure_dir: impl Into<PathBuf>,
        mode: Mode,
    ) -> Self {
        Self {
            golden_dir: golden_dir.into(),
            failure_dir: failure_dir.into(),
            mode,
        }
    }

    /// Path of the golden image `name`.
    #[must_use]
    pub fn path(&self, name: &str) -> PathBuf {
        self.golden_dir.join(format!("{name}.png"))
    }

    /// Checks `actual` against the golden image `name`, returning a
    /// description of the failure instead of panicking.
    ///
    /// # Errors
    ///
    /// Returns a message if the golden is missing or unreadable, or if the
    /// images differ beyond `tolerance` (after writing failure artefacts).
    pub fn check(
        &self,
        name: &str,
        actual: &ColorBuffer,
        tolerance: Tolerance,
    ) -> Result<(), String> {
        let path = self.path(name);
        if self.mode == Mode::Update {
            write_png(&path, actual)?;
            eprintln!("golden: wrote {}", path.display());
            return Ok(());
        }
        let bytes = std::fs::read(&path).map_err(|e| {
            format!(
                "golden {name}: cannot read {}: {e}\n\
                 Run `scripts/dev.sh just golden-update` to create it, then review it.",
                path.display()
            )
        })?;
        let expected =
            png::decode(&bytes).map_err(|e| format!("golden {name}: {}: {e}", path.display()))?;
        let cmp = compare(&expected, actual, tolerance);
        if cmp.passed {
            return Ok(());
        }
        let dir = self.failure_dir.join(name);
        write_png(&dir.join("expected.png"), &expected)?;
        write_png(&dir.join("actual.png"), actual)?;
        if let Some(diff) = diff_image(&expected, actual, tolerance) {
            write_png(&dir.join("diff.png"), &diff)?;
        }
        Err(format!(
            "golden {name} mismatch: {cmp} (tolerance {tolerance:?}); artefacts in {}",
            dir.display()
        ))
    }

    /// Like [`Golden::check`], but panics on failure.
    ///
    /// # Panics
    ///
    /// Panics with a description if the check fails.
    #[track_caller]
    pub fn assert(&self, name: &str, actual: &ColorBuffer, tolerance: Tolerance) {
        if let Err(msg) = self.check(name, actual, tolerance) {
            panic!("{msg}");
        }
    }
}

fn write_png(path: &Path, image: &ColorBuffer) -> Result<(), String> {
    let bytes = png::encode(image).map_err(|e| format!("encoding {}: {e}", path.display()))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("writing {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker(w: u32, h: u32) -> ColorBuffer {
        let mut c = ColorBuffer::new(w, h, Rgba8::BLACK).unwrap();
        for y in 0..h {
            for x in 0..w {
                if (x + y) % 2 == 0 {
                    *c.get_mut(x, y).unwrap() = Rgba8::new(200, 100, 50, 255);
                }
            }
        }
        c
    }

    /// A fresh scratch directory per test (tests may run in parallel).
    fn scratch(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("p3b-golden-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn store(test: &str, mode: Mode) -> Golden {
        let dir = scratch(test);
        Golden::new(dir.join("golden"), dir.join("failures"), mode)
    }

    #[test]
    fn identical_images_pass_exact_comparison() {
        let cmp = compare(&checker(8, 8), &checker(8, 8), Tolerance::EXACT);
        assert!(cmp.passed && cmp.same_size);
        assert_eq!((cmp.differing_pixels, cmp.max_channel_delta), (0, 0));
    }

    #[test]
    fn small_deltas_within_threshold_pass() {
        let mut a = checker(8, 8);
        a.get_mut(3, 3).unwrap().g += 2;
        let tol = Tolerance {
            channel_threshold: 2,
            max_differing_fraction: 0.0,
        };
        let cmp = compare(&checker(8, 8), &a, tol);
        assert!(cmp.passed);
        assert_eq!(cmp.max_channel_delta, 2);
        assert!(!compare(&checker(8, 8), &a, Tolerance::EXACT).passed);
    }

    #[test]
    fn differing_fraction_is_enforced() {
        let mut a = checker(10, 10);
        *a.get_mut(0, 0).unwrap() = Rgba8::WHITE;
        let one_pct = Tolerance {
            channel_threshold: 0,
            max_differing_fraction: 0.01,
        };
        assert!(compare(&checker(10, 10), &a, one_pct).passed);
        *a.get_mut(1, 0).unwrap() = Rgba8::WHITE;
        let cmp = compare(&checker(10, 10), &a, one_pct);
        assert!(!cmp.passed);
        assert_eq!(cmp.differing_pixels, 2);
        assert!(cmp.to_string().contains("2/100 pixels differ"));
    }

    #[test]
    fn size_mismatch_fails() {
        let cmp = compare(
            &checker(4, 4),
            &checker(4, 5),
            Tolerance {
                channel_threshold: 255,
                max_differing_fraction: 1.0,
            },
        );
        assert!(!cmp.passed && !cmp.same_size);
        assert!(diff_image(&checker(4, 4), &checker(5, 4), Tolerance::EXACT).is_none());
    }

    #[test]
    fn diff_image_marks_differing_pixels_red() {
        let mut a = checker(4, 4);
        *a.get_mut(1, 2).unwrap() = Rgba8::WHITE;
        let d = diff_image(&checker(4, 4), &a, Tolerance::EXACT).unwrap();
        let marked = d.get(1, 2).unwrap();
        assert!(marked.r >= 128 && marked.g == 0 && marked.b == 0);
        assert!(d.pixels().iter().filter(|p| p.g == 0 && p.r >= 128).count() == 1);
    }

    #[test]
    fn update_mode_writes_golden_then_verify_passes() {
        let image = checker(6, 3);
        let upd = store("update", Mode::Update);
        upd.assert("img", &image, Tolerance::EXACT);
        assert!(upd.path("img").is_file());
        let verify = Golden {
            mode: Mode::Verify,
            ..upd
        };
        verify.assert("img", &image, Tolerance::EXACT);
    }

    #[test]
    fn missing_golden_fails_in_verify_mode() {
        let g = store("missing", Mode::Verify);
        let err = g
            .check("nope", &checker(2, 2), Tolerance::EXACT)
            .unwrap_err();
        assert!(err.contains("golden-update"), "{err}");
        assert!(
            !g.path("nope").exists(),
            "verify mode must not create goldens"
        );
    }

    #[test]
    fn mismatch_fails_and_writes_artefacts() {
        let upd = store("mismatch", Mode::Update);
        upd.assert("img", &checker(4, 4), Tolerance::EXACT);
        let verify = Golden {
            mode: Mode::Verify,
            ..upd
        };
        let mut other = checker(4, 4);
        *other.get_mut(0, 0).unwrap() = Rgba8::WHITE;
        let err = verify.check("img", &other, Tolerance::EXACT).unwrap_err();
        assert!(err.contains("1/16 pixels differ"), "{err}");
        for f in ["expected.png", "actual.png", "diff.png"] {
            assert!(
                verify.failure_dir.join("img").join(f).is_file(),
                "missing {f}"
            );
        }
    }

    #[test]
    #[should_panic(expected = "mismatch")]
    fn assert_panics_on_mismatch() {
        let upd = store("panics", Mode::Update);
        upd.assert("img", &checker(4, 4), Tolerance::EXACT);
        Golden {
            mode: Mode::Verify,
            ..upd
        }
        .assert(
            "img",
            &checker(4, 4).clone_with_white_corner(),
            Tolerance::EXACT,
        );
    }

    trait WhiteCorner {
        fn clone_with_white_corner(&self) -> Self;
    }

    impl WhiteCorner for ColorBuffer {
        fn clone_with_white_corner(&self) -> Self {
            let mut c = self.clone();
            *c.get_mut(0, 0).unwrap() = Rgba8::WHITE;
            c
        }
    }
}
