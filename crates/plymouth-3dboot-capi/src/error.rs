// SPDX-License-Identifier: GPL-3.0-or-later
//! Status codes, the thread-local error message, and the panic guard.

use std::cell::RefCell;
use std::ffi::{CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Result of a `p3b_*` call. On anything but `P3B_STATUS_OK`,
/// `p3b_last_error()` describes the problem.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum p3b_status {
    /// Success.
    Ok = 0,
    /// A required pointer argument was NULL.
    NullPointer = 1,
    /// An argument was out of range or inconsistent.
    InvalidArgument = 2,
    /// A string argument was not valid UTF-8.
    InvalidUtf8 = 3,
    /// The model format is not supported.
    UnsupportedFormat = 4,
    /// A file could not be read.
    Io = 5,
    /// Model data could not be parsed.
    Parse = 6,
    /// Rendering failed.
    Render = 7,
    /// The output buffer is too small for the requested image.
    BufferTooSmall = 8,
    /// An internal error (a bug): the call was aborted safely.
    Panic = 9,
}

/// An error to report: status plus message.
pub(crate) type Failure = (p3b_status, String);

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

/// Records `message` as this thread's last error.
pub(crate) fn set_error(message: &str) {
    // Interior NULs cannot be represented; replace them.
    let c = CString::new(message.replace('\0', "\u{fffd}")).unwrap_or_default();
    let _ = LAST_ERROR.try_with(|e| *e.borrow_mut() = c);
}

/// Runs `f`, converting failures and panics into a status and recording the
/// error message. Panics never unwind into the caller.
pub(crate) fn ffi_guard(f: impl FnOnce() -> Result<(), Failure>) -> p3b_status {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(())) => {
            set_error("");
            p3b_status::Ok
        }
        Ok(Err((status, message))) => {
            set_error(&message);
            status
        }
        Err(payload) => {
            let what = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_owned());
            set_error(&format!("internal error: {what}"));
            p3b_status::Panic
        }
    }
}

/// The message describing the last failed `p3b_*` call on this thread (an
/// empty string after a successful call). The pointer stays valid until the
/// next `p3b_*` call on the same thread; do not free it.
#[unsafe(no_mangle)]
pub extern "C" fn p3b_last_error() -> *const c_char {
    LAST_ERROR
        .try_with(|e| e.borrow().as_ptr())
        .unwrap_or(c"".as_ptr())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    fn last() -> String {
        // SAFETY: p3b_last_error returns a valid NUL-terminated string.
        unsafe { CStr::from_ptr(p3b_last_error()) }
            .to_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn success_clears_and_failure_sets_the_message() {
        assert_eq!(
            ffi_guard(|| Err((p3b_status::InvalidArgument, "bad width".into()))),
            p3b_status::InvalidArgument
        );
        assert_eq!(last(), "bad width");
        assert_eq!(ffi_guard(|| Ok(())), p3b_status::Ok);
        assert_eq!(last(), "");
    }

    #[test]
    fn panics_are_contained() {
        let status = ffi_guard(|| panic!("boom {}", 42));
        assert_eq!(status, p3b_status::Panic);
        assert_eq!(last(), "internal error: boom 42");
        assert_eq!(ffi_guard(|| panic!("static")), p3b_status::Panic);
        assert_eq!(last(), "internal error: static");
    }

    #[test]
    #[cfg(not(target_os = "emscripten"))] // no threads there
    fn messages_are_per_thread_and_nul_safe() {
        set_error("main");
        std::thread::spawn(|| {
            assert_eq!(last(), "", "a new thread starts clean");
            set_error("worker\0with nul");
            assert_eq!(last(), "worker\u{fffd}with nul");
        })
        .join()
        .unwrap();
        assert_eq!(last(), "main");
    }

    #[test]
    fn status_values_are_stable() {
        assert_eq!(p3b_status::Ok as i32, 0);
        assert_eq!(p3b_status::Panic as i32, 9);
    }
}
