/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Traits to be implemented by backends

use crate::EnvironmentBlendMode;
use crate::Error;
use crate::Event;
use crate::Floor;
use crate::Frame;
use crate::InputSource;
use crate::Native;
use crate::Quitter;
use crate::Sender;
use crate::Session;
use crate::SessionBuilder;
use crate::SessionMode;
use crate::Viewport;
use crate::Views;

use euclid::RigidTransform3D;
use euclid::Size2D;

use surfman::platform::generic::universal::surface::Surface;

/// A trait for discovering XR devices
pub trait Discovery: 'static {
    fn request_session(&mut self, mode: SessionMode, xr: SessionBuilder) -> Result<Session, Error>;
    fn supports_session(&self, mode: SessionMode) -> bool;
}

/// A trait for using an XR device
pub trait Device: 'static {
    /// The transform from native coordinates to the floor.
    fn floor_transform(&self) -> RigidTransform3D<f32, Native, Floor>;

    /// The transforms from viewer coordinates to the eyes, and their associated viewports.
    fn views(&self) -> Views;

    /// A resolution large enough to contain all the viewports.
    /// https://immersive-web.github.io/webxr/#native-webgl-framebuffer-resolution
    fn recommended_framebuffer_resolution(&self) -> Size2D<i32, Viewport> {
        let viewport = match self.views() {
            Views::Mono(view) => view.viewport,
            Views::Stereo(left, right) => left.viewport.union(&right.viewport),
        };
        Size2D::new(viewport.max_x(), viewport.max_y())
    }

    /// This method should block waiting for the next frame,
    /// and return the information for it.
    fn wait_for_animation_frame(&mut self) -> Option<Frame>;

    /// This method should render a surface to the device.
    /// While this method is being called, the device has ownership
    /// of the surface, and should return it afterwards.
    fn render_animation_frame(&mut self, surface: Surface) -> Surface;

    /// Inputs registered with the device on initialization. More may be added, which
    /// should be communicated through a yet-undecided event mechanism
    fn initial_inputs(&self) -> Vec<InputSource>;

    /// Sets the event handling channel
    fn set_event_dest(&mut self, dest: Sender<Event>);

    /// Quit the session
    fn quit(&mut self);

    fn set_quitter(&mut self, quitter: Quitter);

    fn update_clip_planes(&mut self, near: f32, far: f32);

    fn environment_blend_mode(&self) -> EnvironmentBlendMode {
        // for VR devices, override for AR
        EnvironmentBlendMode::Opaque
    }
}

impl Discovery for Box<dyn Discovery> {
    fn request_session(&mut self, mode: SessionMode, xr: SessionBuilder) -> Result<Session, Error> {
        (&mut **self).request_session(mode, xr)
    }

    fn supports_session(&self, mode: SessionMode) -> bool {
        (&**self).supports_session(mode)
    }
}
