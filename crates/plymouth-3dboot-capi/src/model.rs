// SPDX-License-Identifier: GPL-3.0-or-later
//! Loading models: `p3b_model_*`.

use std::ffi::{CStr, CString, c_char, c_int, c_void};

use plymouth_3dboot::color::Rgba8;
use plymouth_3dboot::io::obj::ObjLoadError;
use plymouth_3dboot::io::{ResolveError, ResourceResolver};
use plymouth_3dboot::math::Vec3;
use plymouth_3dboot::{Format, LoadError, Model};

use crate::error::{Failure, ffi_guard, p3b_status};

/// A loaded model (opaque). Create with `p3b_model_load_file` or
/// `p3b_model_load_memory`; release with `p3b_model_free`.
pub struct p3b_model {
    pub(crate) model: Model,
    warnings: Vec<CString>,
}

impl p3b_model {
    fn new(model: Model) -> Self {
        let warnings = model
            .warnings
            .iter()
            .map(|w| CString::new(w.replace('\0', "\u{fffd}")).unwrap_or_default())
            .collect();
        Self { model, warnings }
    }
}

/// Supplies the contents of a file referenced by a model (e.g. an OBJ's
/// material library) when loading from memory.
///
/// Set `*data` and `*len` and return 0 if `name` exists, or return non-zero
/// if it does not. The data must stay valid until the load call returns;
/// the library copies it.
#[allow(non_camel_case_types)]
pub type p3b_resolve_fn = Option<
    unsafe extern "C" fn(
        user_data: *mut c_void,
        name: *const c_char,
        data: *mut *const u8,
        len: *mut usize,
    ) -> c_int,
>;

/// Adapts a C callback to [`ResourceResolver`].
struct CallbackResolver {
    callback: p3b_resolve_fn,
    user_data: *mut c_void,
}

impl ResourceResolver for CallbackResolver {
    fn resolve(&self, name: &str) -> Result<Vec<u8>, ResolveError> {
        let not_found = || ResolveError::NotFound(name.to_owned());
        let Some(callback) = self.callback else {
            return Err(not_found());
        };
        let c_name = CString::new(name).map_err(|_| ResolveError::InvalidName(name.to_owned()))?;
        let mut data: *const u8 = std::ptr::null();
        let mut len = 0usize;
        // SAFETY: the caller of p3b_model_load_memory guarantees the callback
        // is valid; the out-pointers point to live locals.
        let found =
            unsafe { callback(self.user_data, c_name.as_ptr(), &raw mut data, &raw mut len) } == 0;
        if !found {
            return Err(not_found());
        }
        if len == 0 {
            return Ok(Vec::new());
        }
        if len > isize::MAX as usize {
            return Err(ResolveError::Io {
                name: name.to_owned(),
                message: format!("resolver returned an impossible length {len}"),
            });
        }
        if data.is_null() {
            return Err(ResolveError::Io {
                name: name.to_owned(),
                message: "resolver returned NULL data".into(),
            });
        }
        // SAFETY: the callback contract: `data` points to `len` readable bytes
        // that stay valid until the load call returns (we copy them now).
        Ok(unsafe { std::slice::from_raw_parts(data, len) }.to_vec())
    }
}

fn load_failure(e: &LoadError) -> Failure {
    let status = match e {
        LoadError::UnsupportedFormat(_) => p3b_status::UnsupportedFormat,
        LoadError::Io { .. } | LoadError::Obj(ObjLoadError::Resolve(_)) => p3b_status::Io,
        LoadError::Utf8 => p3b_status::InvalidUtf8,
        _ => p3b_status::Parse,
    };
    (status, e.to_string())
}

/// Reads a NUL-terminated UTF-8 C string argument.
///
/// # Safety
///
/// `s` must be NULL or point to a NUL-terminated string.
pub(crate) unsafe fn c_str<'a>(s: *const c_char, what: &str) -> Result<&'a str, Failure> {
    if s.is_null() {
        return Err((p3b_status::NullPointer, format!("{what} is NULL")));
    }
    // SAFETY: non-NULL and NUL-terminated per the caller's contract.
    unsafe { CStr::from_ptr(s) }.to_str().map_err(|_| {
        (
            p3b_status::InvalidUtf8,
            format!("{what} is not valid UTF-8"),
        )
    })
}

/// Stores a new model in `*out` (which must be non-NULL), or NULL on error.
fn store(out: *mut *mut p3b_model, load: impl FnOnce() -> Result<Model, Failure>) -> p3b_status {
    if out.is_null() {
        return ffi_guard(|| Err((p3b_status::NullPointer, "out is NULL".into())));
    }
    // SAFETY: `out` is non-NULL and points to writable storage (contract).
    unsafe { out.write(std::ptr::null_mut()) };
    ffi_guard(|| {
        let model = Box::new(p3b_model::new(load()?));
        // SAFETY: as above.
        unsafe { out.write(Box::into_raw(model)) };
        Ok(())
    })
}

/// Loads a model file (`.obj` or `.dae`, chosen by extension); side files
/// are read relative to its directory. On success `*out` receives the model
/// (free it with `p3b_model_free`); on failure it is set to NULL.
///
/// # Safety
///
/// `path` must be a NUL-terminated string and `out` a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn p3b_model_load_file(
    path: *const c_char,
    out: *mut *mut p3b_model,
) -> p3b_status {
    store(out, || {
        // SAFETY: forwarded caller contract.
        let path = unsafe { c_str(path, "path") }?;
        Model::load(path).map_err(|e| load_failure(&e))
    })
}

/// Loads a model from `len` bytes at `data`. `format` is `"obj"` or `"dae"`.
/// Files the model refers to are requested from `resolve` (may be NULL: no
/// side files) with `user_data`. On success `*out` receives the model; on
/// failure it is set to NULL.
///
/// # Safety
///
/// `format` must be a NUL-terminated string, `data` must point to `len`
/// readable bytes (or be NULL with `len == 0`), `resolve` must be NULL or a
/// valid callback honouring its contract, and `out` a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn p3b_model_load_memory(
    format: *const c_char,
    data: *const u8,
    len: usize,
    resolve: p3b_resolve_fn,
    user_data: *mut c_void,
    out: *mut *mut p3b_model,
) -> p3b_status {
    store(out, || {
        // SAFETY: forwarded caller contract.
        let format_name = unsafe { c_str(format, "format") }?;
        let format = Format::from_extension(format_name).ok_or_else(|| {
            (
                p3b_status::UnsupportedFormat,
                format!("unsupported format {format_name:?} (expected \"obj\" or \"dae\")"),
            )
        })?;
        let bytes = match (data.is_null(), len) {
            (_, 0) => &[][..],
            (true, _) => return Err((p3b_status::NullPointer, "data is NULL".into())),
            // SAFETY: `data` points to `len` readable bytes (contract).
            (false, n) => unsafe { std::slice::from_raw_parts(data, n) },
        };
        let resolver = CallbackResolver {
            callback: resolve,
            user_data,
        };
        Model::from_bytes(format, bytes, &resolver).map_err(|e| load_failure(&e))
    })
}

/// Frees a model. NULL is ignored.
///
/// # Safety
///
/// `model` must be NULL or a pointer from a `p3b_model_load_*` call that has
/// not been freed, and no renderer may still use it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn p3b_model_free(model: *mut p3b_model) {
    if !model.is_null() {
        // SAFETY: allocated by Box::into_raw in `store` and not yet freed.
        drop(unsafe { Box::from_raw(model) });
    }
}

/// Borrows a model argument.
///
/// # Safety
///
/// `model` must be NULL or a live model pointer.
pub(crate) unsafe fn model_ref<'a>(model: *const p3b_model) -> Result<&'a p3b_model, Failure> {
    // SAFETY: NULL-checked; otherwise live per the caller's contract.
    unsafe { model.as_ref() }.ok_or_else(|| (p3b_status::NullPointer, "model is NULL".into()))
}

/// Number of animation clips in the model (0 for NULL).
///
/// # Safety
///
/// `model` must be NULL or a live model.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn p3b_model_clip_count(model: *const p3b_model) -> usize {
    // SAFETY: forwarded caller contract.
    unsafe { model.as_ref() }.map_or(0, |m| m.model.clips.len())
}

/// Stores the duration in seconds of clip `index` in `*seconds`.
///
/// # Safety
///
/// `model` must be NULL or a live model; `seconds` a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn p3b_model_clip_duration(
    model: *const p3b_model,
    index: usize,
    seconds: *mut f32,
) -> p3b_status {
    ffi_guard(|| {
        // SAFETY: forwarded caller contract.
        let m = unsafe { model_ref(model) }?;
        let clip = m.model.clips.get(index).ok_or_else(|| {
            (
                p3b_status::InvalidArgument,
                format!("clip {index} out of range ({} clips)", m.model.clips.len()),
            )
        })?;
        if seconds.is_null() {
            return Err((p3b_status::NullPointer, "seconds is NULL".into()));
        }
        // SAFETY: non-NULL and writable (contract).
        unsafe { seconds.write(clip.duration()) };
        Ok(())
    })
}

/// Adds a clip (as the last one) spinning the whole model one turn about the
/// axis `(x, y, z)` through the origin every `period` seconds.
///
/// # Safety
///
/// `model` must be NULL or a live model not used by a renderer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn p3b_model_add_turntable(
    model: *mut p3b_model,
    x: f32,
    y: f32,
    z: f32,
    period: f32,
) -> p3b_status {
    ffi_guard(|| {
        // SAFETY: NULL-checked; otherwise live and exclusively ours (contract).
        let m = unsafe { model.as_mut() }
            .ok_or_else(|| (p3b_status::NullPointer, "model is NULL".into()))?;
        let axis = Vec3::new(x, y, z);
        if !(axis.is_finite() && axis.length_squared() > 0.0 && period.is_finite() && period > 0.0)
        {
            return Err((
                p3b_status::InvalidArgument,
                "the axis must be non-zero and the period positive".into(),
            ));
        }
        let model = std::mem::replace(
            &mut m.model,
            Model {
                scene: Default::default(),
                clips: Vec::new(),
                warnings: Vec::new(),
            },
        );
        m.model = model.with_turntable(axis, period);
        Ok(())
    })
}

/// Stores the model's bounding box at rest (world units, without a floor
/// added by `p3b_model_add_floor`) in `min[3]` and `max[3]`.
///
/// # Safety
///
/// `model` must be NULL or a live model; `min` and `max` must point to three
/// writable floats each.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn p3b_model_bounds(
    model: *const p3b_model,
    min: *mut f32,
    max: *mut f32,
) -> p3b_status {
    ffi_guard(|| {
        // SAFETY: forwarded caller contract.
        let m = unsafe { model_ref(model) }?;
        if min.is_null() || max.is_null() {
            return Err((p3b_status::NullPointer, "min or max is NULL".into()));
        }
        let scene = &m.model.scene;
        let b = scene.model_bounds_with(&scene.world_matrices());
        if b.is_empty() {
            return Err((
                p3b_status::InvalidArgument,
                "the model has no geometry".into(),
            ));
        }
        // SAFETY: each points to three writable floats (contract).
        unsafe {
            std::ptr::copy_nonoverlapping(b.min.to_array().as_ptr(), min, 3);
            std::ptr::copy_nonoverlapping(b.max.to_array().as_ptr(), max, 3);
        }
        Ok(())
    })
}

/// Number of warnings produced while loading (0 for NULL).
///
/// # Safety
///
/// `model` must be NULL or a live model.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn p3b_model_warning_count(model: *const p3b_model) -> usize {
    // SAFETY: forwarded caller contract.
    unsafe { model.as_ref() }.map_or(0, |m| m.warnings.len())
}

/// Warning `index` as a NUL-terminated string owned by the model (valid
/// until it is freed), or NULL if out of range.
///
/// # Safety
///
/// `model` must be NULL or a live model.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn p3b_model_warning(model: *const p3b_model, index: usize) -> *const c_char {
    // SAFETY: forwarded caller contract.
    unsafe { model.as_ref() }
        .and_then(|m| m.warnings.get(index))
        .map_or(std::ptr::null(), |w| w.as_ptr())
}

/// Adds a floor under the model: a square of the sRGB colour `rgb` (3
/// bytes) just below the model's lowest point over its clips, with a half
/// extent of `size` (> 0) times the model's horizontal radius. Add it after
/// any turntable. The floor stays fixed while the model moves, is ignored
/// when framing the camera, and costs little per frame.
///
/// # Safety
///
/// `model` must be NULL or a live model not used by a renderer; `rgb` must
/// point to 3 readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn p3b_model_add_floor(
    model: *mut p3b_model,
    rgb: *const u8,
    size: f32,
) -> p3b_status {
    ffi_guard(|| {
        // SAFETY: NULL-checked; otherwise live and exclusively ours (contract).
        let m = unsafe { model.as_mut() }
            .ok_or_else(|| (p3b_status::NullPointer, "model is NULL".into()))?;
        if rgb.is_null() {
            return Err((p3b_status::NullPointer, "rgb is NULL".into()));
        }
        if !(size.is_finite() && size > 0.0) {
            return Err((
                p3b_status::InvalidArgument,
                "the size must be positive".into(),
            ));
        }
        // SAFETY: non-NULL and 3 readable bytes (contract).
        let [r, g, b] = unsafe { rgb.cast::<[u8; 3]>().read_unaligned() };
        let model = std::mem::replace(
            &mut m.model,
            Model {
                scene: Default::default(),
                clips: Vec::new(),
                warnings: Vec::new(),
            },
        );
        m.model = model.with_floor(Rgba8::new(r, g, b, 255), size);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::p3b_last_error;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/n64_logo");

    fn last_error() -> String {
        // SAFETY: always a valid C string.
        unsafe { CStr::from_ptr(p3b_last_error()) }
            .to_string_lossy()
            .into_owned()
    }

    fn load_file(path: &str) -> (p3b_status, *mut p3b_model) {
        let c = CString::new(path).unwrap();
        let mut out = std::ptr::dangling_mut::<p3b_model>();
        // SAFETY: valid string and out pointer.
        let status = unsafe { p3b_model_load_file(c.as_ptr(), &raw mut out) };
        (status, out)
    }

    #[test]
    fn loads_files_and_reports_counts() {
        let (status, m) = load_file(&format!("{FIXTURES}/n64_logo_spin.dae"));
        assert_eq!(status, p3b_status::Ok, "{}", last_error());
        // SAFETY: `m` is a live model in all calls below until freed.
        unsafe {
            assert_eq!(p3b_model_clip_count(m), 1);
            let mut d = 0.0;
            assert_eq!(p3b_model_clip_duration(m, 0, &raw mut d), p3b_status::Ok);
            assert!((d - 10.0 / 3.0).abs() < 1e-5);
            assert_eq!(
                p3b_model_clip_duration(m, 1, &raw mut d),
                p3b_status::InvalidArgument
            );
            assert_eq!(
                p3b_model_add_turntable(m, 0.0, 1.0, 0.0, 4.0),
                p3b_status::Ok
            );
            assert_eq!(p3b_model_clip_count(m), 2);
            assert_eq!(
                p3b_model_add_turntable(m, 0.0, 0.0, 0.0, 4.0),
                p3b_status::InvalidArgument
            );
            let (mut lo0, mut hi0) = ([0.0f32; 3], [0.0f32; 3]);
            assert_eq!(
                p3b_model_bounds(m, lo0.as_mut_ptr(), hi0.as_mut_ptr()),
                p3b_status::Ok
            );
            let grey = [200u8, 200, 200];
            assert_eq!(
                p3b_model_add_floor(m, grey.as_ptr(), 0.0),
                p3b_status::InvalidArgument
            );
            assert_eq!(
                p3b_model_add_floor(m, std::ptr::null(), 4.0),
                p3b_status::NullPointer
            );
            assert_eq!(
                p3b_model_add_floor(std::ptr::null_mut(), grey.as_ptr(), 4.0),
                p3b_status::NullPointer
            );
            let nodes = (*m).model.scene.nodes().len();
            assert_eq!(p3b_model_add_floor(m, grey.as_ptr(), 4.0), p3b_status::Ok);
            assert_eq!((*m).model.scene.nodes().len(), nodes + 1);
            assert_eq!(p3b_model_clip_count(m), 2, "a floor adds no clip");
            let (mut lo1, mut hi1) = ([0.0f32; 3], [0.0f32; 3]);
            assert_eq!(
                p3b_model_bounds(m, lo1.as_mut_ptr(), hi1.as_mut_ptr()),
                p3b_status::Ok
            );
            assert_eq!(
                (lo0, hi0),
                (lo1, hi1),
                "the floor is not part of the model's bounds"
            );
            let (mut lo, mut hi) = ([0.0f32; 3], [0.0f32; 3]);
            assert_eq!(
                p3b_model_bounds(m, lo.as_mut_ptr(), hi.as_mut_ptr()),
                p3b_status::Ok
            );
            assert!(hi[1] > lo[1]);
            assert_eq!(p3b_model_warning_count(m), 0);
            assert!(p3b_model_warning(m, 0).is_null());
            p3b_model_free(m);
        }
    }

    unsafe extern "C" fn resolve_mtl(
        user: *mut c_void,
        name: *const c_char,
        data: *mut *const u8,
        len: *mut usize,
    ) -> c_int {
        // SAFETY: the test passes a &Vec<u8> as user data and valid pointers.
        unsafe {
            let mtl = &*(user as *const Vec<u8>);
            if CStr::from_ptr(name).to_str() != Ok("n64_logo.mtl") {
                return 1;
            }
            data.write(mtl.as_ptr());
            len.write(mtl.len());
        }
        0
    }

    #[test]
    fn loads_from_memory_with_a_resolver_callback() {
        let obj = std::fs::read(format!("{FIXTURES}/n64_logo.obj")).unwrap();
        let mtl = std::fs::read(format!("{FIXTURES}/n64_logo.mtl")).unwrap();
        let mut out = std::ptr::null_mut();
        // SAFETY: valid arguments; `mtl` outlives the call.
        let status = unsafe {
            p3b_model_load_memory(
                c"obj".as_ptr(),
                obj.as_ptr(),
                obj.len(),
                Some(resolve_mtl),
                (&raw const mtl).cast_mut().cast(),
                &raw mut out,
            )
        };
        assert_eq!(status, p3b_status::Ok, "{}", last_error());
        // SAFETY: `out` is a live model.
        unsafe {
            assert_eq!((*out).model.scene.materials.len(), 4);
            // Only the unsupported directives warn (not a missing library).
            for i in 0..p3b_model_warning_count(out) {
                let w = CStr::from_ptr(p3b_model_warning(out, i)).to_string_lossy();
                assert!(w.contains("ignored directive"), "{w}");
            }
            p3b_model_free(out);
        }
        // Without a resolver the library is missing: a warning, not an error.
        let mut out = std::ptr::null_mut();
        // SAFETY: valid arguments.
        let status = unsafe {
            p3b_model_load_memory(
                c"OBJ".as_ptr(),
                obj.as_ptr(),
                obj.len(),
                None,
                std::ptr::null_mut(),
                &raw mut out,
            )
        };
        assert_eq!(status, p3b_status::Ok);
        // SAFETY: live model.
        unsafe {
            assert!((0..p3b_model_warning_count(out)).any(|i| {
                CStr::from_ptr(p3b_model_warning(out, i))
                    .to_string_lossy()
                    .contains("not found")
            }));
            p3b_model_free(out);
        }
    }

    #[test]
    fn errors_set_status_message_and_null_output() {
        let (status, m) = load_file("/nonexistent/model.obj");
        assert_eq!((status, m), (p3b_status::Io, std::ptr::null_mut()));
        assert!(last_error().contains("model.obj"));
        assert_eq!(load_file("model.gltf").0, p3b_status::UnsupportedFormat);

        let mut out = std::ptr::null_mut();
        let bad = b"v 1 x 3\n";
        // SAFETY: valid arguments.
        unsafe {
            assert_eq!(
                p3b_model_load_memory(
                    c"obj".as_ptr(),
                    bad.as_ptr(),
                    bad.len(),
                    None,
                    std::ptr::null_mut(),
                    &raw mut out
                ),
                p3b_status::Parse
            );
            assert!(out.is_null());
            assert!(last_error().contains("line 1"));
            let truncated = b"<COLLADA><library_geometries>";
            assert_eq!(
                p3b_model_load_memory(
                    c"dae".as_ptr(),
                    truncated.as_ptr(),
                    truncated.len(),
                    None,
                    std::ptr::null_mut(),
                    &raw mut out
                ),
                p3b_status::Parse
            );
            assert_eq!(
                p3b_model_load_memory(
                    c"ply".as_ptr(),
                    bad.as_ptr(),
                    bad.len(),
                    None,
                    std::ptr::null_mut(),
                    &raw mut out
                ),
                p3b_status::UnsupportedFormat
            );
            assert_eq!(
                p3b_model_load_memory(
                    std::ptr::null(),
                    bad.as_ptr(),
                    bad.len(),
                    None,
                    std::ptr::null_mut(),
                    &raw mut out
                ),
                p3b_status::NullPointer
            );
            assert_eq!(
                p3b_model_load_memory(
                    c"obj".as_ptr(),
                    std::ptr::null(),
                    5,
                    None,
                    std::ptr::null_mut(),
                    &raw mut out
                ),
                p3b_status::NullPointer
            );
            assert_eq!(
                p3b_model_load_memory(
                    c"obj".as_ptr(),
                    bad.as_ptr(),
                    bad.len(),
                    None,
                    std::ptr::null_mut(),
                    std::ptr::null_mut()
                ),
                p3b_status::NullPointer
            );
            let invalid = [0x6fu8, 0xff, 0];
            assert_eq!(
                p3b_model_load_file(invalid.as_ptr().cast(), &raw mut out),
                p3b_status::InvalidUtf8
            );
            assert_eq!(
                p3b_model_load_memory(
                    c"obj".as_ptr(),
                    [0xffu8].as_ptr(),
                    1,
                    None,
                    std::ptr::null_mut(),
                    &raw mut out
                ),
                p3b_status::InvalidUtf8
            );
            // NULL models are handled everywhere.
            p3b_model_free(std::ptr::null_mut());
            assert_eq!(p3b_model_clip_count(std::ptr::null()), 0);
            assert_eq!(p3b_model_warning_count(std::ptr::null()), 0);
            assert!(p3b_model_warning(std::ptr::null(), 0).is_null());
            let mut d = 0.0;
            assert_eq!(
                p3b_model_clip_duration(std::ptr::null(), 0, &raw mut d),
                p3b_status::NullPointer
            );
            assert_eq!(
                p3b_model_add_turntable(std::ptr::null_mut(), 0.0, 1.0, 0.0, 1.0),
                p3b_status::NullPointer
            );
            assert_eq!(
                p3b_model_bounds(std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut()),
                p3b_status::NullPointer
            );
        }
    }
}
