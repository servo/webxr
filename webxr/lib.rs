/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! This crate defines the Rust implementation of WebXR for various devices.

#[cfg(feature = "glwindow")]
pub mod glwindow;

#[cfg(feature = "headless")]
pub mod headless;

#[cfg(feature = "googlevr")]
pub mod googlevr;

#[cfg(feature = "googlevr")]
mod gl {
    include!(concat!(env!("OUT_DIR"), "/gles_bindings.rs"));
}
