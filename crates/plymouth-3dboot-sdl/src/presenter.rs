// SPDX-License-Identifier: GPL-3.0-or-later
//! Showing rendered frames in an SDL3 window.

use plymouth_3dboot::color::Rgba8;
use plymouth_3dboot::target::ColorBuffer;
use sdl3::pixels::{Color, PixelFormat};
use sdl3::render::{FRect, Texture, WindowCanvas};
use sdl3_sys::pixels::SDL_PixelFormat;

/// An SDL call failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("SDL: {0}")]
pub struct SdlError(pub String);

fn err(e: impl std::fmt::Display) -> SdlError {
    SdlError(e.to_string())
}

/// Window settings for [`Presenter::new`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowConfig {
    /// Window title.
    pub title: String,
    /// Initial width in (logical) pixels.
    pub width: u32,
    /// Initial height in (logical) pixels.
    pub height: u32,
    /// Whether the user may resize the window.
    pub resizable: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "plymouth-3dboot".into(),
            width: 640,
            height: 480,
            resizable: true,
        }
    }
}

struct StreamingTexture {
    texture: Texture,
    width: u32,
    height: u32,
}

/// Owns an SDL window and renderer and shows [`ColorBuffer`]s in it.
///
/// Frames are uploaded to a streaming `RGBA32` texture (byte order
/// `R, G, B, A` on every host, matching [`ColorBuffer::as_bytes`]) that is
/// reused while the frame size stays the same, then scaled to the window
/// with their aspect ratio preserved (letterboxed with the clear colour).
pub struct Presenter {
    // Field order matters: `texture` must be destroyed before `canvas`.
    texture: Option<StreamingTexture>,
    canvas: WindowCanvas,
    sdl: sdl3::Sdl,
    clear: Rgba8,
}

impl std::fmt::Debug for Presenter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Presenter")
            .field("renderer", &self.canvas.renderer_name)
            .finish_non_exhaustive()
    }
}

impl Presenter {
    /// Initializes SDL video and opens a window.
    ///
    /// Must be called on the main thread. Honours `SDL_VIDEO_DRIVER` /
    /// `SDL_RENDER_DRIVER` (e.g. `dummy` / `software` for headless use).
    ///
    /// # Errors
    ///
    /// Returns [`SdlError`] if SDL, the window or the renderer cannot be
    /// created.
    pub fn new(config: &WindowConfig) -> Result<Self, SdlError> {
        let sdl = sdl3::init().map_err(err)?;
        let video = sdl.video().map_err(err)?;
        let mut builder = video.window(&config.title, config.width, config.height);
        if config.resizable {
            builder.resizable();
        }
        let window = builder.build().map_err(err)?;
        Ok(Self {
            texture: None,
            canvas: window.into_canvas(),
            sdl,
            clear: Rgba8::BLACK,
        })
    }

    /// The SDL context (for event handling).
    #[must_use]
    pub fn sdl(&self) -> &sdl3::Sdl {
        &self.sdl
    }

    /// Name of the SDL render driver in use (e.g. `software`, `opengl`).
    #[must_use]
    pub fn renderer_name(&self) -> &str {
        &self.canvas.renderer_name
    }

    /// Size of the drawable area in physical pixels; a good frame size.
    ///
    /// # Errors
    ///
    /// Returns [`SdlError`] if SDL cannot report the size.
    pub fn output_size(&self) -> Result<(u32, u32), SdlError> {
        self.canvas.output_size().map_err(err)
    }

    /// Colour of the letterbox bars.
    pub fn set_clear_color(&mut self, color: Rgba8) {
        self.clear = color;
    }

    /// Uploads `frame` and draws it to the back buffer (without showing it;
    /// see [`Presenter::present`]).
    ///
    /// # Errors
    ///
    /// Returns [`SdlError`] if the texture cannot be created or updated, or
    /// drawing fails.
    pub fn draw(&mut self, frame: &ColorBuffer) -> Result<(), SdlError> {
        let (w, h) = (frame.width(), frame.height());
        let c = self.clear;
        self.canvas.set_draw_color(Color::RGBA(c.r, c.g, c.b, c.a));
        self.canvas.clear();
        if w == 0 || h == 0 {
            return Ok(());
        }
        if self
            .texture
            .as_ref()
            .is_none_or(|t| (t.width, t.height) != (w, h))
        {
            self.destroy_texture();
            let format = PixelFormat::try_from(SDL_PixelFormat::RGBA32).map_err(err)?;
            let texture = self
                .canvas
                .create_texture_streaming(format, w, h)
                .map_err(err)?;
            self.texture = Some(StreamingTexture {
                texture,
                width: w,
                height: h,
            });
        }
        let tex = &mut self.texture.as_mut().expect("created above").texture;
        tex.update(None, frame.as_bytes(), w as usize * 4)
            .map_err(|e| err(format!("{e:?}")))?;
        let dst = letterbox((w, h), self.canvas.output_size().map_err(err)?);
        self.canvas.copy(tex, None, Some(dst)).map_err(err)
    }

    /// Shows the back buffer.
    pub fn present(&mut self) {
        self.canvas.present();
    }

    /// [`Presenter::draw`] followed by [`Presenter::present`].
    ///
    /// # Errors
    ///
    /// See [`Presenter::draw`].
    pub fn show(&mut self, frame: &ColorBuffer) -> Result<(), SdlError> {
        self.draw(frame)?;
        self.present();
        Ok(())
    }

    /// Reads back the current back buffer (after [`Presenter::draw`], before
    /// [`Presenter::present`]). Slow; meant for tests and screenshots.
    ///
    /// # Errors
    ///
    /// Returns [`SdlError`] if reading or converting the pixels fails.
    pub fn read_back(&self) -> Result<ColorBuffer, SdlError> {
        let surface = self.canvas.read_pixels(None).map_err(err)?;
        let format = PixelFormat::try_from(SDL_PixelFormat::RGBA32).map_err(err)?;
        let surface = surface.convert_format(format).map_err(err)?;
        let (w, h, pitch) = (surface.width(), surface.height(), surface.pitch() as usize);
        let mut out = ColorBuffer::new(w, h, Rgba8::TRANSPARENT).map_err(err)?;
        surface.with_lock(|bytes| {
            for (y, row) in out
                .pixels_mut()
                .chunks_exact_mut(w.max(1) as usize)
                .enumerate()
            {
                let src = &bytes[y * pitch..][..w as usize * 4];
                for (px, b) in row.iter_mut().zip(src.as_chunks::<4>().0) {
                    *px = Rgba8::from(*b);
                }
            }
        });
        Ok(out)
    }

    fn destroy_texture(&mut self) {
        if let Some(old) = self.texture.take() {
            // SAFETY: the texture was created by `self.canvas`, which is
            // still alive (we hold `&mut self`), so destroying it is valid.
            unsafe { old.texture.destroy() };
        }
    }
}

impl Drop for Presenter {
    fn drop(&mut self) {
        self.destroy_texture();
    }
}

/// The largest rectangle with the frame's aspect ratio centred in the
/// output.
fn letterbox((fw, fh): (u32, u32), (ow, oh): (u32, u32)) -> FRect {
    #[allow(clippy::cast_precision_loss)]
    let (fw, fh, ow, oh) = (fw as f32, fh as f32, ow as f32, oh as f32);
    let scale = (ow / fw).min(oh / fh);
    let (w, h) = (fw * scale, fh * scale);
    FRect::new((ow - w) * 0.5, (oh - h) * 0.5, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letterbox_preserves_aspect_and_centres() {
        let r = letterbox((100, 50), (400, 400));
        assert_eq!((r.x, r.y, r.w, r.h), (0.0, 100.0, 400.0, 200.0));
        let r = letterbox((50, 100), (400, 400));
        assert_eq!((r.x, r.y, r.w, r.h), (100.0, 0.0, 200.0, 400.0));
        let r = letterbox((64, 48), (64, 48));
        assert_eq!((r.x, r.y, r.w, r.h), (0.0, 0.0, 64.0, 48.0));
    }
}
