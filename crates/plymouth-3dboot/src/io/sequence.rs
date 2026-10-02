// SPDX-License-Identifier: GPL-3.0-or-later
//! Writing numbered image sequences (`frame0000.png`, `frame0001.png`, ...).

use std::path::{Path, PathBuf};

use super::png::{PngError, encode};
use crate::target::ColorBuffer;

/// Writing a sequence frame failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SequenceError {
    /// The frame could not be encoded.
    #[error(transparent)]
    Png(#[from] PngError),
    /// The file could not be written.
    #[error("writing {path}: {source}")]
    Io {
        /// Destination file.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
}

/// Writes frames as zero-padded numbered PNG files into a directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PngSequence {
    dir: PathBuf,
    prefix: String,
    digits: usize,
}

impl PngSequence {
    /// A sequence of `<dir>/<prefix><index>.png` files with at least
    /// `digits` digits per index (4 is a common choice).
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>, prefix: impl Into<String>, digits: usize) -> Self {
        Self {
            dir: dir.into(),
            prefix: prefix.into(),
            digits,
        }
    }

    /// The path of frame `index`.
    #[must_use]
    pub fn path(&self, index: u64) -> PathBuf {
        self.dir.join(format!(
            "{}{index:0width$}.png",
            self.prefix,
            width = self.digits
        ))
    }

    /// The output directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Encodes `frame` and writes it as frame `index` (creating the
    /// directory if needed). Returns the file's path.
    ///
    /// # Errors
    ///
    /// Returns [`SequenceError`] if encoding or writing fails.
    pub fn write(&self, index: u64, frame: &ColorBuffer) -> Result<PathBuf, SequenceError> {
        let path = self.path(index);
        let bytes = encode(frame)?;
        let io = |source| SequenceError::Io {
            path: path.clone(),
            source,
        };
        std::fs::create_dir_all(&self.dir).map_err(io)?;
        std::fs::write(&path, bytes).map_err(io)?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Rgba8;

    #[test]
    fn names_are_zero_padded() {
        let s = PngSequence::new("out", "frame", 4);
        assert_eq!(s.path(7), Path::new("out/frame0007.png"));
        assert_eq!(s.path(12345), Path::new("out/frame12345.png"));
        assert_eq!(PngSequence::new("o", "f", 0).path(3), Path::new("o/f3.png"));
    }

    #[test]
    fn writes_frames_that_decode_back() {
        let dir = std::env::temp_dir().join(format!("p3b-sequence-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let seq = PngSequence::new(dir.join("nested"), "f", 3);
        let frames: Vec<ColorBuffer> = (0..3u8)
            .map(|i| ColorBuffer::new(4, 2, Rgba8::new(i * 50, 0, 0, 255)).unwrap())
            .collect();
        for (i, f) in frames.iter().enumerate() {
            seq.write(i as u64, f).unwrap();
        }
        for (i, f) in frames.iter().enumerate() {
            let back =
                super::super::png::decode(&std::fs::read(seq.path(i as u64)).unwrap()).unwrap();
            assert_eq!(&back, f);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn errors_name_the_file() {
        // A file where the directory should be.
        let file = std::env::temp_dir().join(format!("p3b-sequence-file-{}", std::process::id()));
        std::fs::write(&file, b"x").unwrap();
        let err = PngSequence::new(&file, "f", 1)
            .write(0, &ColorBuffer::new(1, 1, Rgba8::BLACK).unwrap())
            .unwrap_err();
        assert!(matches!(err, SequenceError::Io { .. }));
        let _ = std::fs::remove_file(&file);
    }
}
