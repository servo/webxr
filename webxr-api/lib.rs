/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! This crate defines the Rust API for WebXR. It is implemented by the `webxr` crate.

extern crate euclid;
extern crate gleam;

mod device;
mod error;
mod frame;
mod session;
mod webgl;

pub use device::Device;
pub use device::Discovery;

pub use error::Error;

pub use frame::Frame;

pub use session::FrameRequestCallback;
pub use session::HighResTimeStamp;
pub use session::Session;
pub use session::SessionBuilder;
pub use session::SessionMode;
pub use session::SessionThread;

pub use webgl::WebGLContextId;
