// SPDX-License-Identifier: GPL-3.0-or-later
//! C API for `plymouth-3dboot`, built as `libplymouth_3dboot` by cargo-c.

#![allow(non_camel_case_types)] // C naming for the exported types

//!
//! All symbols are prefixed `p3b_`. See `docs/c-api.md` for ownership,
//! threading and error-handling rules.

// Panics are caught at the C boundary (`ffi_guard`); with panic=abort an
// internal bug would instead kill the host process (e.g. plymouthd).
#[cfg(panic = "abort")]
compile_error!("plymouth-3dboot-capi requires panic=unwind so panics can be contained");

mod error;
mod model;
mod render;

pub use error::{p3b_last_error, p3b_status};
pub use model::{
    p3b_model, p3b_model_add_turntable, p3b_model_bounds, p3b_model_clip_count,
    p3b_model_clip_duration, p3b_model_free, p3b_model_load_file, p3b_model_load_memory,
    p3b_model_warning, p3b_model_warning_count, p3b_resolve_fn,
};
pub use render::*;

use std::ffi::{CStr, c_char};

/// ABI version: the major version, incremented on every incompatible change
/// to the C API; the shared library is `libplymouth_3dboot.so.<major>`.
/// release-please updates the value (and the header's, which comes from
/// cbindgen.toml) with the version.
pub const P3B_ABI_VERSION: AbiVersion = 0; // x-release-please-major

/// The type of [`P3B_ABI_VERSION`] (an alias without digits, so that
/// release-please's major-version update finds the value).
type AbiVersion = u32;

/// The library version as a NUL-terminated string (e.g. `"0.1.0"`). The
/// string is static: do not free it.
#[unsafe(no_mangle)]
pub extern "C" fn p3b_version() -> *const c_char {
    const VERSION: &CStr =
        match CStr::from_bytes_with_nul(concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes()) {
            Ok(v) => v,
            Err(_) => panic!("version contains NUL"),
        };
    VERSION.as_ptr()
}

/// The ABI version of the loaded library; compare with
/// `P3B_ABI_VERSION` from the header to detect mismatches.
#[unsafe(no_mangle)]
pub extern "C" fn p3b_abi_version() -> u32 {
    P3B_ABI_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        // SAFETY: p3b_version returns a static NUL-terminated string.
        let v = unsafe { CStr::from_ptr(p3b_version()) };
        assert_eq!(v.to_str().unwrap(), plymouth_3dboot::version());
        assert_eq!(p3b_abi_version(), P3B_ABI_VERSION);
        assert_eq!(
            P3B_ABI_VERSION.to_string(),
            env!("CARGO_PKG_VERSION_MAJOR"),
            "the ABI follows the major version"
        );
    }
}
