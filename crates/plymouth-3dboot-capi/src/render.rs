// SPDX-License-Identifier: GPL-3.0-or-later
//! Rendering frames into caller-owned buffers: `p3b_renderer_*`.

use plymouth_3dboot::color::Rgba8;
use plymouth_3dboot::math::Vec3;
use plymouth_3dboot::raster::CullMode;
use plymouth_3dboot::scene::{Camera, Projection};
use plymouth_3dboot::shading::ShadingModel;
use plymouth_3dboot::target::ColorBuffer;
use plymouth_3dboot::{AnimationRenderer, CameraSource, FrameSettings, WrapMode};

use crate::error::{Failure, ffi_guard, p3b_status};
use crate::model::{model_ref, p3b_model};

// Input enumerations are plain `uint32_t` values with named constants, so
// that an out-of-range value from C is a reportable error rather than an
// invalid Rust enum.

/// Layout of the pixels written by `p3b_render_frame` (`P3B_PIXEL_FORMAT_*`).
pub type p3b_pixel_format = u32;
/// Bytes R, G, B, A (straight alpha).
pub const P3B_PIXEL_FORMAT_RGBA8888: p3b_pixel_format = 0;
/// Bytes B, G, R, A (straight alpha).
pub const P3B_PIXEL_FORMAT_BGRA8888: p3b_pixel_format = 1;
/// One native-endian `uint32_t` per pixel, `0xAARRGGBB`, with colour
/// premultiplied by alpha (Plymouth's pixel-buffer format).
pub const P3B_PIXEL_FORMAT_ARGB32_PREMULTIPLIED: p3b_pixel_format = 2;

/// Shading model (`P3B_SHADING_*`).
pub type p3b_shading = u32;
/// Material colours as authored (self-illuminated).
pub const P3B_SHADING_UNLIT: p3b_shading = 0;
/// Diffuse lighting.
pub const P3B_SHADING_LAMBERT: p3b_shading = 1;
/// Diffuse lighting plus specular highlights.
pub const P3B_SHADING_BLINN_PHONG: p3b_shading = 2;

/// How playback time outside a clip is mapped (`P3B_WRAP_*`).
pub type p3b_wrap = u32;
/// Repeat the clip.
pub const P3B_WRAP_LOOP: p3b_wrap = 0;
/// Hold the first/last pose.
pub const P3B_WRAP_CLAMP: p3b_wrap = 1;
/// Play forwards, then backwards.
pub const P3B_WRAP_PING_PONG: p3b_wrap = 2;

/// Where the camera comes from (`P3B_CAMERA_MODE_*`).
pub type p3b_camera_mode = u32;
/// Frame the whole (animated) model, looking along `view_direction`.
pub const P3B_CAMERA_MODE_FRAMED: p3b_camera_mode = 0;
/// Look from `eye` at `target` with `up`.
pub const P3B_CAMERA_MODE_LOOK_AT: p3b_camera_mode = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PixelFormat {
    Rgba,
    Bgra,
    Argb32Premultiplied,
}

fn pixel_format(v: p3b_pixel_format) -> Result<PixelFormat, Failure> {
    match v {
        P3B_PIXEL_FORMAT_RGBA8888 => Ok(PixelFormat::Rgba),
        P3B_PIXEL_FORMAT_BGRA8888 => Ok(PixelFormat::Bgra),
        P3B_PIXEL_FORMAT_ARGB32_PREMULTIPLIED => Ok(PixelFormat::Argb32Premultiplied),
        other => Err((
            p3b_status::InvalidArgument,
            format!("unknown pixel format {other}"),
        )),
    }
}

/// Rendering options. Start from `p3b_render_options_default()` and change
/// fields as needed.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct p3b_render_options {
    /// Shading model.
    pub shading: p3b_shading,
    /// Background colour as R, G, B, A bytes (straight alpha).
    pub background: [u8; 4],
    /// Non-zero to cull back faces (double-sided materials never are). An
    /// integer rather than `bool`, so that any byte value is valid.
    pub cull_back_faces: u8,
    /// Clip playback wrap mode.
    pub wrap: p3b_wrap,
    /// Camera mode.
    pub camera_mode: p3b_camera_mode,
    /// `P3B_CAMERA_MODE_FRAMED`: direction from the camera towards the model.
    pub view_direction: [f32; 3],
    /// `P3B_CAMERA_MODE_LOOK_AT`: camera position.
    pub eye: [f32; 3],
    /// `P3B_CAMERA_MODE_LOOK_AT`: point looked at.
    pub target: [f32; 3],
    /// `P3B_CAMERA_MODE_LOOK_AT`: up direction.
    pub up: [f32; 3],
    /// Vertical field of view in radians; 0 or less for orthographic
    /// (framed mode only).
    pub fov_y: f32,
    /// `P3B_CAMERA_MODE_LOOK_AT`: near and far clip distances.
    pub z_near: f32,
    /// See `z_near`.
    pub z_far: f32,
    /// Supersampling anti-aliasing: samples per axis, 1 (off) to 8. Cost
    /// grows with its square.
    pub antialias: u8,
}

/// The default options: unlit shading on opaque black, back-face culling,
/// looping, and a camera framing the model from the front right, slightly
/// above, with a 0.7 rad field of view.
#[unsafe(no_mangle)]
pub extern "C" fn p3b_render_options_default() -> p3b_render_options {
    p3b_render_options {
        shading: P3B_SHADING_UNLIT,
        background: [0, 0, 0, 255],
        cull_back_faces: 1,
        wrap: P3B_WRAP_LOOP,
        camera_mode: P3B_CAMERA_MODE_FRAMED,
        view_direction: [-0.6, -0.45, -1.0],
        eye: [0.0, 0.0, 5.0],
        target: [0.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        fov_y: 0.7,
        z_near: 0.1,
        z_far: 100.0,
        antialias: 1,
    }
}

fn settings(width: u32, height: u32, o: &p3b_render_options) -> Result<FrameSettings, Failure> {
    let finite = |v: [f32; 3]| v.iter().all(|c| c.is_finite());
    let invalid = |what: &str| (p3b_status::InvalidArgument, what.to_owned());
    let camera = match o.camera_mode {
        P3B_CAMERA_MODE_FRAMED => {
            if !finite(o.view_direction)
                || Vec3::from(o.view_direction).length_squared() == 0.0
                || !o.fov_y.is_finite()
                || o.fov_y >= std::f32::consts::PI
            {
                return Err(invalid(
                    "view_direction must be non-zero and fov_y below pi",
                ));
            }
            CameraSource::Framed {
                direction: Vec3::from(o.view_direction),
                fov_y: (o.fov_y > 0.0).then_some(o.fov_y),
            }
        }
        P3B_CAMERA_MODE_LOOK_AT => {
            let (eye, target, up) = (Vec3::from(o.eye), Vec3::from(o.target), Vec3::from(o.up));
            if !(finite(o.eye) && finite(o.target) && finite(o.up))
                || eye == target
                || up.cross(target - eye).length_squared() == 0.0
            {
                return Err(invalid(
                    "eye, target and up must be finite, distinct and not aligned",
                ));
            }
            if !(o.fov_y > 0.0
                && o.fov_y < std::f32::consts::PI
                && o.z_near > 0.0
                && o.z_far > o.z_near
                && o.z_far.is_finite())
            {
                return Err(invalid(
                    "look-at cameras need 0 < fov_y < pi and 0 < z_near < z_far",
                ));
            }
            let projection = Projection::Perspective {
                fov_y: o.fov_y,
                z_near: o.z_near,
                z_far: o.z_far,
            };
            CameraSource::Fixed(Camera::look_at(eye, target, up, projection))
        }
        other => return Err(invalid(&format!("unknown camera mode {other}"))),
    };
    let shading = match o.shading {
        P3B_SHADING_UNLIT => ShadingModel::Unlit,
        P3B_SHADING_LAMBERT => ShadingModel::Lambert,
        P3B_SHADING_BLINN_PHONG => ShadingModel::BlinnPhong,
        other => return Err(invalid(&format!("unknown shading model {other}"))),
    };
    Ok(FrameSettings {
        camera,
        shading,
        cull: if o.cull_back_faces != 0 {
            CullMode::Back
        } else {
            CullMode::None
        },
        background: Rgba8::from(o.background),
        antialias: u32::from(o.antialias),
        ..FrameSettings::new(width, height)
    })
}

/// Largest renderer width or height. Colour plus depth take 8 bytes per
/// pixel, so 8192 x 8192 needs 512 MiB; allocation failure would abort the
/// host process, so sizes are bounded well below the core library's limit.
pub const P3B_MAX_SIZE: u32 = 8192;
use P3B_MAX_SIZE as MAX_SIZE;

/// Renders frames of one model (opaque). Create with `p3b_renderer_new`;
/// release with `p3b_renderer_free`. A renderer must only be used by one
/// thread at a time.
pub struct p3b_renderer {
    // Borrows the model: the caller keeps the model alive and unmodified
    // until the renderer is freed (documented contract of p3b_renderer_new).
    inner: AnimationRenderer<'static>,
}

/// Creates a renderer for `model` producing `width × height` images and
/// playing clip `clip` (pass `SIZE_MAX`, i.e. `(size_t)-1`, for no
/// animation). `options` may be NULL for the defaults.
///
/// The model must stay alive, and must not be modified (e.g. by
/// `p3b_model_add_turntable`), until the renderer is freed.
///
/// # Safety
///
/// `model` must be NULL or a live model satisfying the above; `options`
/// NULL or valid; `out` a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn p3b_renderer_new(
    model: *const p3b_model,
    clip: usize,
    width: u32,
    height: u32,
    options: *const p3b_render_options,
    out: *mut *mut p3b_renderer,
) -> p3b_status {
    if out.is_null() {
        return ffi_guard(|| Err((p3b_status::NullPointer, "out is NULL".into())));
    }
    // SAFETY: `out` is non-NULL and writable (contract).
    unsafe { out.write(std::ptr::null_mut()) };
    ffi_guard(|| {
        // SAFETY: the caller guarantees the model outlives the renderer, so
        // extending the borrow to 'static is sound under that contract.
        let model: &'static p3b_model = unsafe { model_ref(model) }?;
        // SAFETY: NULL or valid per the contract.
        let options = unsafe { options.as_ref() }
            .copied()
            .unwrap_or_else(|| p3b_render_options_default());
        if width == 0 || height == 0 || width > MAX_SIZE || height > MAX_SIZE {
            return Err((
                p3b_status::InvalidArgument,
                format!("width and height must be between 1 and {MAX_SIZE}"),
            ));
        }
        let clip = match clip {
            usize::MAX => None,
            i => Some(model.model.clips.get(i).ok_or_else(|| {
                (
                    p3b_status::InvalidArgument,
                    format!("clip {i} out of range ({} clips)", model.model.clips.len()),
                )
            })?),
        };
        let wrap = match options.wrap {
            P3B_WRAP_LOOP => WrapMode::Loop,
            P3B_WRAP_CLAMP => WrapMode::Clamp,
            P3B_WRAP_PING_PONG => WrapMode::PingPong,
            other => {
                return Err((
                    p3b_status::InvalidArgument,
                    format!("unknown wrap mode {other}"),
                ));
            }
        };
        let inner = AnimationRenderer::new(
            &model.model.scene,
            clip.map(|c| (c, wrap)),
            settings(width, height, &options)?,
        )
        .map_err(|e| (p3b_status::InvalidArgument, e.to_string()))?;
        // SAFETY: as above.
        unsafe { out.write(Box::into_raw(Box::new(p3b_renderer { inner }))) };
        Ok(())
    })
}

/// Frees a renderer. NULL is ignored.
///
/// # Safety
///
/// `renderer` must be NULL or a live renderer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn p3b_renderer_free(renderer: *mut p3b_renderer) {
    if !renderer.is_null() {
        // SAFETY: allocated by Box::into_raw in p3b_renderer_new.
        drop(unsafe { Box::from_raw(renderer) });
    }
}

/// Writes `image` into `dst` (rows `stride` bytes apart) in `format`; bytes
/// past each row's pixels are not touched.
fn write_pixels(image: &ColorBuffer, dst: &mut [u8], stride: usize, format: PixelFormat) {
    let w = image.width() as usize;
    for (y, row) in image.pixels().chunks_exact(w.max(1)).enumerate() {
        let out = &mut dst[y * stride..][..w * 4];
        for (px, o) in row.iter().zip(out.as_chunks_mut::<4>().0) {
            *o = match format {
                PixelFormat::Rgba => px.to_array(),
                PixelFormat::Bgra => [px.b, px.g, px.r, px.a],
                PixelFormat::Argb32Premultiplied => {
                    let pm = |c: u8| u32::from((u16::from(c) * u16::from(px.a) + 127) / 255);
                    (u32::from(px.a) << 24 | pm(px.r) << 16 | pm(px.g) << 8 | pm(px.b))
                        .to_ne_bytes()
                }
            };
        }
    }
}

/// Renders the frame at playback time `time` (seconds) into `dst`, which
/// holds `dst_len` bytes with rows `stride` bytes apart (at least
/// `width * 4`), in `format`. Bytes beyond each row's `width * 4` are left
/// untouched.
///
/// # Safety
///
/// `renderer` must be NULL or a live renderer (whose model is still alive)
/// not used concurrently; `dst` must point to `dst_len` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn p3b_render_frame(
    renderer: *mut p3b_renderer,
    time: f64,
    dst: *mut u8,
    dst_len: usize,
    stride: usize,
    format: p3b_pixel_format,
) -> p3b_status {
    ffi_guard(|| {
        // SAFETY: NULL-checked; otherwise live and exclusively used (contract).
        let r = unsafe { renderer.as_mut() }
            .ok_or_else(|| (p3b_status::NullPointer, "renderer is NULL".into()))?;
        let format = pixel_format(format)?;
        if dst.is_null() {
            return Err((p3b_status::NullPointer, "dst is NULL".into()));
        }
        let (w, h) = (
            r.inner.settings().width as usize,
            r.inner.settings().height as usize,
        );
        if stride < w * 4 {
            return Err((
                p3b_status::InvalidArgument,
                format!("stride {stride} is less than width * 4 = {}", w * 4),
            ));
        }
        let Some(needed) = stride.checked_mul(h - 1).and_then(|n| n.checked_add(w * 4)) else {
            return Err((
                p3b_status::InvalidArgument,
                "stride * height overflows".into(),
            ));
        };
        if dst_len < needed {
            return Err((
                p3b_status::BufferTooSmall,
                format!("buffer holds {dst_len} bytes, {needed} needed"),
            ));
        }
        if !time.is_finite() {
            return Err((p3b_status::InvalidArgument, "time must be finite".into()));
        }
        let image = r
            .inner
            .render_at(time)
            .map_err(|e| (p3b_status::Render, e.to_string()))?;
        // SAFETY: `dst` points to `dst_len >= needed` writable bytes
        // (contract). The slice covers only the bytes we write, a real
        // allocation size unlike a caller's possibly oversized `dst_len`.
        let dst = unsafe { std::slice::from_raw_parts_mut(dst, needed) };
        write_pixels(image, dst, stride, format);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{p3b_model_free, p3b_model_load_file};
    use std::ffi::CString;

    fn load(name: &str) -> *mut p3b_model {
        let path = CString::new(format!(
            "{}/../../tests/fixtures/n64_logo/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let mut m = std::ptr::null_mut();
        // SAFETY: valid arguments.
        let status = unsafe { p3b_model_load_file(path.as_ptr(), &raw mut m) };
        assert_eq!(status, p3b_status::Ok);
        m
    }

    #[test]
    fn pixel_formats() {
        let mut img = ColorBuffer::new(2, 1, Rgba8::TRANSPARENT).unwrap();
        *img.get_mut(0, 0).unwrap() = Rgba8::new(10, 20, 30, 255);
        *img.get_mut(1, 0).unwrap() = Rgba8::new(200, 100, 50, 128);
        let mut out = [0u8; 8];
        write_pixels(&img, &mut out, 8, PixelFormat::Rgba);
        assert_eq!(out, [10, 20, 30, 255, 200, 100, 50, 128]);
        write_pixels(&img, &mut out, 8, PixelFormat::Bgra);
        assert_eq!(out, [30, 20, 10, 255, 50, 100, 200, 128]);
        write_pixels(&img, &mut out, 8, PixelFormat::Argb32Premultiplied);
        let px = |i: usize| u32::from_ne_bytes(out[i * 4..][..4].try_into().unwrap());
        assert_eq!(px(0), 0xFF0A_141E);
        // 200 * 128 / 255 = 100.4 -> 100, 100 -> 50.2 -> 50, 50 -> 25.1 -> 25.
        assert_eq!(px(1), 0x8064_3219);
    }

    #[test]
    fn frames_match_the_rust_renderer_and_respect_stride() {
        let m = load("n64_logo_spin.dae");
        let mut r = std::ptr::null_mut();
        // SAFETY: live model; valid out pointer; model outlives renderer.
        unsafe {
            assert_eq!(
                p3b_renderer_new(m, 0, 32, 24, std::ptr::null(), &raw mut r),
                p3b_status::Ok
            );
            let stride = 32 * 4 + 12;
            let mut buf = vec![0xAAu8; stride * 24];
            assert_eq!(
                p3b_render_frame(
                    r,
                    0.5,
                    buf.as_mut_ptr(),
                    buf.len(),
                    stride,
                    P3B_PIXEL_FORMAT_RGBA8888
                ),
                p3b_status::Ok
            );
            // Same as the Rust API.
            let model = &(*m).model;
            let mut expected = model
                .renderer(Some(0), WrapMode::Loop, FrameSettings::new(32, 24))
                .unwrap();
            let image = expected.render_at(0.5).unwrap();
            for y in 0..24 {
                assert_eq!(
                    &buf[y * stride..][..32 * 4],
                    &image.as_bytes()[y * 128..][..128],
                    "row {y}"
                );
                assert!(
                    buf[y * stride + 128..][..12].iter().all(|&b| b == 0xAA),
                    "padding of row {y} untouched"
                );
            }
            p3b_renderer_free(r);
            p3b_model_free(m);
        }
    }

    #[test]
    fn invalid_arguments_are_rejected() {
        let m = load("n64_logo.obj");
        let mut r = std::ptr::null_mut();
        // SAFETY: arguments are valid or deliberately NULL.
        unsafe {
            assert_eq!(
                p3b_renderer_new(m, 0, 16, 16, std::ptr::null(), &raw mut r),
                p3b_status::InvalidArgument,
                "no clips"
            );
            assert!(r.is_null());
            assert_eq!(
                p3b_renderer_new(m, usize::MAX, 0, 16, std::ptr::null(), &raw mut r),
                p3b_status::InvalidArgument
            );
            assert_eq!(
                p3b_renderer_new(
                    std::ptr::null(),
                    usize::MAX,
                    16,
                    16,
                    std::ptr::null(),
                    &raw mut r
                ),
                p3b_status::NullPointer
            );
            let mut bad = p3b_render_options_default();
            bad.view_direction = [0.0; 3];
            assert_eq!(
                p3b_renderer_new(m, usize::MAX, 16, 16, &raw const bad, &raw mut r),
                p3b_status::InvalidArgument
            );
            let mut look = p3b_render_options {
                camera_mode: P3B_CAMERA_MODE_LOOK_AT,
                ..p3b_render_options_default()
            };
            look.eye = look.target;
            assert_eq!(
                p3b_renderer_new(m, usize::MAX, 16, 16, &raw const look, &raw mut r),
                p3b_status::InvalidArgument
            );
            assert_eq!(
                p3b_renderer_new(m, usize::MAX, 1 << 20, 16, std::ptr::null(), &raw mut r),
                p3b_status::InvalidArgument
            );

            assert_eq!(
                p3b_renderer_new(m, usize::MAX, 16, 16, std::ptr::null(), &raw mut r),
                p3b_status::Ok
            );
            let mut buf = vec![0u8; 16 * 16 * 4];
            assert_eq!(
                p3b_render_frame(
                    r,
                    0.0,
                    buf.as_mut_ptr(),
                    buf.len() - 1,
                    64,
                    P3B_PIXEL_FORMAT_RGBA8888
                ),
                p3b_status::BufferTooSmall
            );
            assert_eq!(
                p3b_render_frame(
                    r,
                    0.0,
                    buf.as_mut_ptr(),
                    buf.len(),
                    60,
                    P3B_PIXEL_FORMAT_RGBA8888
                ),
                p3b_status::InvalidArgument
            );
            assert_eq!(
                p3b_render_frame(
                    r,
                    f64::NAN,
                    buf.as_mut_ptr(),
                    buf.len(),
                    64,
                    P3B_PIXEL_FORMAT_RGBA8888
                ),
                p3b_status::InvalidArgument
            );
            assert_eq!(
                p3b_render_frame(
                    r,
                    0.0,
                    std::ptr::null_mut(),
                    buf.len(),
                    64,
                    P3B_PIXEL_FORMAT_RGBA8888
                ),
                p3b_status::NullPointer
            );
            assert_eq!(
                p3b_render_frame(
                    std::ptr::null_mut(),
                    0.0,
                    buf.as_mut_ptr(),
                    buf.len(),
                    64,
                    P3B_PIXEL_FORMAT_RGBA8888
                ),
                p3b_status::NullPointer
            );
            assert_eq!(
                p3b_render_frame(
                    r,
                    0.0,
                    buf.as_mut_ptr(),
                    buf.len(),
                    64,
                    P3B_PIXEL_FORMAT_ARGB32_PREMULTIPLIED
                ),
                p3b_status::Ok
            );
            assert!(buf.iter().any(|&b| b != 0));
            assert_eq!(
                p3b_render_frame(r, 0.0, buf.as_mut_ptr(), buf.len(), 64, 7),
                p3b_status::InvalidArgument,
                "unknown format"
            );
            p3b_renderer_free(r);
            for bad in [
                p3b_render_options {
                    shading: 9,
                    ..p3b_render_options_default()
                },
                p3b_render_options {
                    wrap: 9,
                    ..p3b_render_options_default()
                },
                p3b_render_options {
                    camera_mode: 9,
                    ..p3b_render_options_default()
                },
                p3b_render_options {
                    antialias: 0,
                    ..p3b_render_options_default()
                },
                p3b_render_options {
                    antialias: 9,
                    ..p3b_render_options_default()
                },
            ] {
                assert_eq!(
                    p3b_renderer_new(m, usize::MAX, 16, 16, &raw const bad, &raw mut r),
                    p3b_status::InvalidArgument
                );
            }
            p3b_renderer_free(std::ptr::null_mut());
            p3b_model_free(m);
        }
    }

    #[test]
    fn look_at_camera_renders() {
        let m = load("n64_logo.obj");
        let mut r = std::ptr::null_mut();
        let options = p3b_render_options {
            camera_mode: P3B_CAMERA_MODE_LOOK_AT,
            eye: [30.0, 40.0, 120.0],
            target: [0.0, 28.0, 0.0],
            z_far: 1000.0,
            shading: P3B_SHADING_LAMBERT,
            ..p3b_render_options_default()
        };
        // SAFETY: valid arguments.
        unsafe {
            assert_eq!(
                p3b_renderer_new(m, usize::MAX, 32, 32, &raw const options, &raw mut r),
                p3b_status::Ok
            );
            let mut buf = vec![0u8; 32 * 32 * 4];
            assert_eq!(
                p3b_render_frame(
                    r,
                    0.0,
                    buf.as_mut_ptr(),
                    buf.len(),
                    128,
                    P3B_PIXEL_FORMAT_BGRA8888
                ),
                p3b_status::Ok
            );
            let centre = &buf[(16 * 32 + 16) * 4..][..4];
            assert_ne!(centre, &[0, 0, 0, 255], "the model is in view");
            p3b_renderer_free(r);
            p3b_model_free(m);
        }
    }
}
