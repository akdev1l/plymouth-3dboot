// SPDX-License-Identifier: GPL-3.0-or-later
//! Render targets: colour and depth buffers.
//!
//! Buffers are row-major with the origin at the top-left pixel and no
//! padding between rows. Callers that need a stride (SDL textures, Plymouth
//! pixel buffers) copy rows out of [`ColorBuffer::as_bytes`].

use crate::color::Rgba8;

/// Largest supported width or height, in pixels.
///
/// The rasterizer's fixed-point setup is sized for this limit (Phase 2).
pub const MAX_DIMENSION: u32 = 16_384;

/// A requested buffer size exceeds [`MAX_DIMENSION`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("buffer size {width}x{height} exceeds the maximum of {MAX_DIMENSION}x{MAX_DIMENSION}")]
pub struct SizeError {
    /// Requested width.
    pub width: u32,
    /// Requested height.
    pub height: u32,
}

fn check_size(width: u32, height: u32) -> Result<usize, SizeError> {
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(SizeError { width, height });
    }
    // At most 2^28 elements, which fits in usize on every supported target.
    Ok(width as usize * height as usize)
}

/// A grid of `T` values, row-major, origin at the top left.
#[derive(Clone, Debug, PartialEq)]
struct Grid<T> {
    width: u32,
    height: u32,
    data: Vec<T>,
}

impl<T: Copy> Grid<T> {
    fn new(width: u32, height: u32, value: T) -> Result<Self, SizeError> {
        let len = check_size(width, height)?;
        Ok(Self {
            width,
            height,
            data: vec![value; len],
        })
    }

    fn index(&self, x: u32, y: u32) -> Option<usize> {
        (x < self.width && y < self.height).then(|| y as usize * self.width as usize + x as usize)
    }

    fn get(&self, x: u32, y: u32) -> Option<T> {
        self.index(x, y).map(|i| self.data[i])
    }

    fn get_mut(&mut self, x: u32, y: u32) -> Option<&mut T> {
        self.index(x, y).map(|i| &mut self.data[i])
    }

    fn row(&self, y: u32) -> Option<&[T]> {
        (y < self.height).then(|| {
            let w = self.width as usize;
            &self.data[y as usize * w..(y as usize + 1) * w]
        })
    }
}

/// An 8-bit sRGB colour buffer ([`Rgba8`] pixels, `R, G, B, A` byte order).
#[derive(Clone, Debug, PartialEq)]
pub struct ColorBuffer(Grid<Rgba8>);

impl ColorBuffer {
    /// Creates a `width × height` buffer filled with `fill`.
    ///
    /// # Errors
    ///
    /// Returns [`SizeError`] if either dimension exceeds [`MAX_DIMENSION`].
    /// Zero-sized buffers are allowed.
    pub fn new(width: u32, height: u32, fill: Rgba8) -> Result<Self, SizeError> {
        Grid::new(width, height, fill).map(Self)
    }

    /// Width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.0.width
    }

    /// Height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.0.height
    }

    /// Sets every pixel to `color`.
    pub fn clear(&mut self, color: Rgba8) {
        self.0.data.fill(color);
    }

    /// The pixel at `(x, y)`, or `None` if out of bounds.
    #[must_use]
    pub fn get(&self, x: u32, y: u32) -> Option<Rgba8> {
        self.0.get(x, y)
    }

    /// Mutable access to the pixel at `(x, y)`, or `None` if out of bounds.
    pub fn get_mut(&mut self, x: u32, y: u32) -> Option<&mut Rgba8> {
        self.0.get_mut(x, y)
    }

    /// Row `y`, or `None` if out of bounds.
    #[must_use]
    pub fn row(&self, y: u32) -> Option<&[Rgba8]> {
        self.0.row(y)
    }

    /// All pixels, row-major.
    #[must_use]
    pub fn pixels(&self) -> &[Rgba8] {
        &self.0.data
    }

    /// All pixels, row-major, mutably.
    pub fn pixels_mut(&mut self) -> &mut [Rgba8] {
        &mut self.0.data
    }

    /// The pixel data as bytes: `width * 4` bytes per row, `R, G, B, A`
    /// order (`SDL_PIXELFORMAT_RGBA32`).
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        bytemuck::cast_slice(&self.0.data)
    }
}

/// A depth buffer of `f32` values in `[0, 1]`, where smaller is closer.
#[derive(Clone, Debug, PartialEq)]
pub struct DepthBuffer(Grid<f32>);

impl DepthBuffer {
    /// The value buffers are cleared to: the far plane.
    pub const FAR: f32 = 1.0;

    /// Creates a `width × height` buffer cleared to [`DepthBuffer::FAR`].
    ///
    /// # Errors
    ///
    /// Returns [`SizeError`] if either dimension exceeds [`MAX_DIMENSION`].
    pub fn new(width: u32, height: u32) -> Result<Self, SizeError> {
        Grid::new(width, height, Self::FAR).map(Self)
    }

    /// Width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.0.width
    }

    /// Height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.0.height
    }

    /// Sets every value to `depth`.
    pub fn clear(&mut self, depth: f32) {
        self.0.data.fill(depth);
    }

    /// The depth at `(x, y)`, or `None` if out of bounds.
    #[must_use]
    pub fn get(&self, x: u32, y: u32) -> Option<f32> {
        self.0.get(x, y)
    }

    /// Mutable access to the depth at `(x, y)`, or `None` if out of bounds.
    pub fn get_mut(&mut self, x: u32, y: u32) -> Option<&mut f32> {
        self.0.get_mut(x, y)
    }

    /// All values, row-major.
    #[must_use]
    pub fn values(&self) -> &[f32] {
        &self.0.data
    }
}

/// A colour buffer and a depth buffer of the same size.
#[derive(Clone, Debug, PartialEq)]
pub struct Framebuffer {
    /// Colour attachment.
    pub color: ColorBuffer,
    /// Depth attachment.
    pub depth: DepthBuffer,
}

impl Framebuffer {
    /// Creates a framebuffer with colour cleared to `clear_color` and depth
    /// cleared to [`DepthBuffer::FAR`].
    ///
    /// # Errors
    ///
    /// Returns [`SizeError`] if either dimension exceeds [`MAX_DIMENSION`].
    pub fn new(width: u32, height: u32, clear_color: Rgba8) -> Result<Self, SizeError> {
        Ok(Self {
            color: ColorBuffer::new(width, height, clear_color)?,
            depth: DepthBuffer::new(width, height)?,
        })
    }

    /// Width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.color.width()
    }

    /// Height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.color.height()
    }

    /// Clears colour to `color` and depth to [`DepthBuffer::FAR`].
    pub fn clear(&mut self, color: Rgba8) {
        self.color.clear(color);
        self.depth.clear(DepthBuffer::FAR);
    }
}

/// Downsamples `src` by `factor` in each direction into `dst` (which must be
/// `src` size / `factor`), averaging each `factor × factor` block in linear
/// light (alpha averaged linearly). This is the resolve step of
/// supersampling anti-aliasing.
///
/// # Panics
///
/// Panics if the sizes do not match.
pub fn downsample(src: &ColorBuffer, factor: u32, dst: &mut ColorBuffer) {
    assert!(factor >= 1, "factor must be at least 1");
    assert_eq!(
        (src.width(), src.height()),
        (dst.width() * factor, dst.height() * factor),
        "size mismatch"
    );
    if factor == 1 {
        dst.pixels_mut().copy_from_slice(src.pixels());
        return;
    }
    let decode = crate::color::srgb8_decode_table();
    let (f, sw) = (factor as usize, src.width() as usize);
    #[allow(clippy::cast_precision_loss)]
    let inv = 1.0 / (f * f) as f32;
    let dw = dst.width() as usize;
    for (dy, row) in dst.pixels_mut().chunks_exact_mut(dw.max(1)).enumerate() {
        for (dx, out) in row.iter_mut().enumerate() {
            let mut sum = [0.0f32; 4];
            for sy in dy * f..(dy + 1) * f {
                for p in &src.pixels()[sy * sw + dx * f..][..f] {
                    sum[0] += decode[usize::from(p.r)];
                    sum[1] += decode[usize::from(p.g)];
                    sum[2] += decode[usize::from(p.b)];
                    sum[3] += f32::from(p.a);
                }
            }
            *out = Rgba8::new(
                crate::color::encode_srgb8(sum[0] * inv),
                crate::color::encode_srgb8(sum[1] * inv),
                crate::color::encode_srgb8(sum[2] * inv),
                crate::color::unorm8(sum[3] * inv / 255.0),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgba8 = Rgba8::new(255, 0, 0, 255);

    #[test]
    fn new_buffers_have_size_and_clear_values() {
        let fb = Framebuffer::new(4, 3, RED).unwrap();
        assert_eq!((fb.width(), fb.height()), (4, 3));
        assert_eq!(fb.color.pixels().len(), 12);
        assert!(fb.color.pixels().iter().all(|&p| p == RED));
        assert!(fb.depth.values().iter().all(|&d| d == DepthBuffer::FAR));
    }

    #[test]
    fn clear_resets_colour_and_depth() {
        let mut fb = Framebuffer::new(2, 2, RED).unwrap();
        *fb.color.get_mut(1, 1).unwrap() = Rgba8::WHITE;
        *fb.depth.get_mut(1, 1).unwrap() = 0.25;
        fb.clear(Rgba8::BLACK);
        assert!(fb.color.pixels().iter().all(|&p| p == Rgba8::BLACK));
        assert!(fb.depth.values().iter().all(|&d| d == 1.0));
    }

    #[test]
    fn indexing_is_row_major_from_top_left() {
        let mut c = ColorBuffer::new(3, 2, Rgba8::TRANSPARENT).unwrap();
        *c.get_mut(2, 0).unwrap() = Rgba8::new(1, 2, 3, 4);
        *c.get_mut(0, 1).unwrap() = Rgba8::new(5, 6, 7, 8);
        assert_eq!(c.pixels()[2], Rgba8::new(1, 2, 3, 4));
        assert_eq!(c.pixels()[3], Rgba8::new(5, 6, 7, 8));
        assert_eq!(c.row(1).unwrap()[0], Rgba8::new(5, 6, 7, 8));
        assert_eq!(c.get(2, 0), Some(Rgba8::new(1, 2, 3, 4)));
    }

    #[test]
    fn bytes_are_rgba_rows_without_padding() {
        let mut c = ColorBuffer::new(2, 2, Rgba8::TRANSPARENT).unwrap();
        *c.get_mut(1, 0).unwrap() = Rgba8::new(10, 20, 30, 40);
        *c.get_mut(0, 1).unwrap() = Rgba8::new(50, 60, 70, 80);
        let b = c.as_bytes();
        assert_eq!(b.len(), 2 * 2 * 4);
        assert_eq!(&b[4..8], &[10, 20, 30, 40]);
        assert_eq!(&b[8..12], &[50, 60, 70, 80]);
    }

    #[test]
    fn out_of_bounds_access_returns_none() {
        let mut c = ColorBuffer::new(3, 2, RED).unwrap();
        let mut d = DepthBuffer::new(3, 2).unwrap();
        assert_eq!(c.get(3, 0), None);
        assert_eq!(c.get(0, 2), None);
        assert!(c.get_mut(u32::MAX, 0).is_none());
        assert!(c.row(2).is_none());
        assert_eq!(d.get(3, 1), None);
        assert!(d.get_mut(0, 2).is_none());
    }

    #[test]
    fn zero_sized_buffers_are_allowed() {
        let fb = Framebuffer::new(0, 5, RED).unwrap();
        assert!(fb.color.pixels().is_empty());
        assert!(fb.color.as_bytes().is_empty());
        assert_eq!(fb.color.get(0, 0), None);
    }

    #[test]
    fn downsampling_averages_in_linear_light() {
        let mut src = ColorBuffer::new(4, 2, Rgba8::new(10, 20, 30, 255)).unwrap();
        // Left 2x2 block: half black, half white (alpha 0 and 255).
        *src.get_mut(0, 0).unwrap() = Rgba8::new(0, 0, 0, 0);
        *src.get_mut(1, 0).unwrap() = Rgba8::new(255, 255, 255, 255);
        *src.get_mut(0, 1).unwrap() = Rgba8::new(0, 0, 0, 0);
        *src.get_mut(1, 1).unwrap() = Rgba8::new(255, 255, 255, 255);
        let mut dst = ColorBuffer::new(2, 1, Rgba8::TRANSPARENT).unwrap();
        downsample(&src, 2, &mut dst);
        // 50% linear is sRGB 188, not 128.
        assert_eq!(dst.get(0, 0), Some(Rgba8::new(188, 188, 188, 128)));
        // A uniform block is unchanged.
        assert_eq!(dst.get(1, 0), Some(Rgba8::new(10, 20, 30, 255)));
        let mut same = ColorBuffer::new(4, 2, Rgba8::BLACK).unwrap();
        downsample(&src, 1, &mut same);
        assert_eq!(same, src);
    }

    #[test]
    fn oversized_buffers_are_rejected() {
        let err = ColorBuffer::new(MAX_DIMENSION + 1, 1, RED).unwrap_err();
        assert_eq!(
            err,
            SizeError {
                width: MAX_DIMENSION + 1,
                height: 1
            }
        );
        assert!(DepthBuffer::new(1, u32::MAX).is_err());
        assert!(Framebuffer::new(MAX_DIMENSION, MAX_DIMENSION + 1, RED).is_err());
        assert!(err.to_string().contains("16384"));
    }
}
