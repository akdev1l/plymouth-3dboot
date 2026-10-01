// SPDX-License-Identifier: GPL-3.0-or-later
//! Encoders and decoders.
//!
//! Everything here works on in-memory bytes; reading and writing files is
//! left to the caller, so the same code runs natively and on WebAssembly.

pub mod png;
