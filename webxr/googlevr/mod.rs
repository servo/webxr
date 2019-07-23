/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use gleam::gl::GLsync;

use webxr_api::Device;
use webxr_api::Discovery;
use webxr_api::Error;
use webxr_api::Event;
use webxr_api::EventBuffer;
use webxr_api::EventCallback;
use webxr_api::Floor;
use webxr_api::Frame;
use webxr_api::InputSource;
use webxr_api::Native;
use webxr_api::Quitter;
use webxr_api::Session;
use webxr_api::SessionBuilder;
use webxr_api::SessionMode;
use webxr_api::Views;

use euclid::default::Size2D;
use euclid::RigidTransform3D;

pub struct GoogleVRDiscovery {}

impl GoogleVRDiscovery {
    pub fn new() -> Self {
        GoogleVRDiscovery {}
    }
}
impl Discovery for GoogleVRDiscovery {
    fn request_session(&mut self, mode: SessionMode, xr: SessionBuilder) -> Result<Session, Error> {
        if self.supports_session(mode) {
            xr.spawn(move || GoogleVRDevice::new())
        } else {
            Err(Error::NoMatchingDevice)
        }
    }

    fn supports_session(&self, mode: SessionMode) -> bool {
        mode == SessionMode::ImmersiveVR
    }
}

pub struct GoogleVRDevice {
    events: EventBuffer,
}

impl GoogleVRDevice {
    pub fn new() -> Result<Self, Error> {
        Ok(GoogleVRDevice {
            events: Default::default(),
        })
    }
}

impl Device for GoogleVRDevice {
    fn floor_transform(&self) -> RigidTransform3D<f32, Native, Floor> {
        unimplemented!()
    }

    fn views(&self) -> Views {
        unimplemented!()
    }

    fn wait_for_animation_frame(&mut self) -> Frame {
        unimplemented!()
    }

    fn render_animation_frame(&mut self, _texture_id: u32, _size: Size2D<i32>, _sync: GLsync) {
        unimplemented!()
    }

    fn initial_inputs(&self) -> Vec<InputSource> {
        vec![]
    }

    fn set_event_callback(&mut self, _callback: Box<dyn EventCallback>) {
        unimplemented!()
    }

    fn quit(&mut self) {
        self.events.callback(Event::SessionEnd);
    }

    fn set_quitter(&mut self, _: Quitter) {
        // do nothing for now until we need the quitter
    }
}
