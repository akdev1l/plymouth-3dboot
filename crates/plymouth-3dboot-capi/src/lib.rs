// SPDX-License-Identifier: GPL-3.0-or-later
//! C API for `plymouth-3dboot`, built as `libplymouth_3dboot` by cargo-c.
//!
//! All symbols are prefixed `p3b_`. See `docs/c-api.md` for ownership,
//! threading and error-handling rules.

use std::ffi::{CStr, c_char};

/// ABI version: incremented on every incompatible change to the C API
/// (matches the shared library's major version).
pub const P3B_ABI_VERSION: u32 = 0;

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
    }
}
