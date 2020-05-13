/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Traits to be implemented by backends

use crate::EnvironmentBlendMode;
use crate::Error;
use crate::Event;
use crate::Floor;
use crate::Frame;
use crate::HitTestId;
use crate::HitTestSource;
use crate::InputSource;
use crate::Native;
use crate::Quitter;
use crate::Sender;
use crate::Session;
use crate::SessionBuilder;
use crate::SessionInit;
use crate::SessionMode;
use crate::Viewport;

use euclid::RigidTransform3D;
use euclid::Size2D;

/// A trait for discovering XR devices
pub trait DiscoveryAPI<SwapChains>: 'static {
    fn request_session(
        &mut self,
        mode: SessionMode,
        init: &SessionInit,
        xr: SessionBuilder<SwapChains>,
    ) -> Result<Session, Error>;
    fn supports_session(&self, mode: SessionMode) -> bool;
}

/// A trait for using an XR device
pub trait DeviceAPI<Surface>: 'static {
    /// The transform from native coordinates to the floor.
    fn floor_transform(&self) -> Option<RigidTransform3D<f32, Native, Floor>>;

    /// A resolution large enough to contain all the viewports.
    /// https://immersive-web.github.io/webxr/#native-webgl-framebuffer-resolution
    fn recommended_framebuffer_resolution(&self) -> Option<Size2D<i32, Viewport>>;

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

    fn granted_features(&self) -> &[String];

    fn request_hit_test(&mut self, _source: HitTestSource) {
        panic!("This device does not support requesting hit tests");
    }

    fn cancel_hit_test(&mut self, _id: HitTestId) {
        panic!("This device does not support hit tests");
    }
}

impl<SwapChains: 'static> DiscoveryAPI<SwapChains> for Box<dyn DiscoveryAPI<SwapChains>> {
    fn request_session(
        &mut self,
        mode: SessionMode,
        init: &SessionInit,
        xr: SessionBuilder<SwapChains>,
    ) -> Result<Session, Error> {
        (&mut **self).request_session(mode, init, xr)
    }

    fn supports_session(&self, mode: SessionMode) -> bool {
        (&**self).supports_session(mode)
    }
}
