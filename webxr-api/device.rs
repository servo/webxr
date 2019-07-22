/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Traits to be implemented by backends

use crate::Error;
use crate::EventCallback;
use crate::Floor;
use crate::Frame;
use crate::InputSource;
use crate::Native;
use crate::Session;
use crate::SessionBuilder;
use crate::SessionMode;
use crate::Viewport;
use crate::Views;

use euclid::Size2D;
use euclid::TypedRigidTransform3D;
use euclid::TypedSize2D;

use gleam::gl::GLsync;

/// A trait for discovering XR devices
pub trait Discovery: 'static {
    fn request_session(&mut self, mode: SessionMode, xr: SessionBuilder) -> Result<Session, Error>;
    fn supports_session(&self, mode: SessionMode) -> bool;
}

/// A trait for using an XR device
pub trait Device: 'static {
    /// The transform from native coordinates to the floor.
    fn floor_transform(&self) -> TypedRigidTransform3D<f32, Native, Floor>;

    /// The transforms from viewer coordinates to the eyes, and their associated viewports.
    fn views(&self) -> Views;

    /// A resolution large enough to contain all the viewports.
    /// https://immersive-web.github.io/webxr/#native-webgl-framebuffer-resolution
    fn recommended_framebuffer_resolution(&self) -> TypedSize2D<i32, Viewport> {
        let viewport = match self.views() {
            Views::Mono(view) => view.viewport,
            Views::Stereo(left, right) => left.viewport.union(&right.viewport),
        };
        TypedSize2D::new(viewport.max_x(), viewport.max_y())
    }

    /// This method should block waiting for the next frame,
    /// and return the information for it.
    fn wait_for_animation_frame(&mut self) -> Frame;

    /// This method should render a GL texture to the device.
    /// While this method is being called, the device has unique access
    /// to the texture. The texture should be sync'd using glWaitSync before being used.
    fn render_animation_frame(&mut self, texture_id: u32, size: Size2D<i32>, sync: GLsync);

    /// Inputs registered with the device on initialization. More may be added, which
    /// should be communicated through a yet-undecided event mechanism
    fn initial_inputs(&self) -> Vec<InputSource>;

    /// Sets the event handling callback
    fn set_event_callback(&mut self, callback: Box<dyn EventCallback>);

    /// Whether the device is still connected
    fn connected(&mut self) -> bool;

    /// Quit the session
    fn quit(&mut self);
}
