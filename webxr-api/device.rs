/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/// Traits to be implemented by backends
use crate::Error;
use crate::Frame;
use crate::Session;
use crate::SessionBuilder;
use crate::SessionMode;

use euclid::Size2D;

/// A trait for discovering XR devices
pub trait Discovery: 'static {
    fn request_session(&mut self, mode: SessionMode, xr: SessionBuilder) -> Result<Session, Error>;
    fn supports_session(&self, mode: SessionMode) -> bool;
}

/// A trait for using an XR device
pub trait Device {
    /// This method should block waiting for the next frame,
    /// and return the information for it.
    fn wait_for_animation_frame(&mut self) -> Frame;

    /// This method should render a GL texture to the device.
    /// While this method is being called, the device has unique access
    /// to the texture.
    fn render_animation_frame(&mut self, texture_id: u32, size: Size2D<i32>);
}
