/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use webxr_api::Device;
use webxr_api::Discovery;
use webxr_api::Error;
use webxr_api::Floor;
use webxr_api::Frame;
use webxr_api::MockDeviceInit;
use webxr_api::MockDeviceMsg;
use webxr_api::MockDiscovery;
use webxr_api::Native;
use webxr_api::Receiver;
use webxr_api::Session;
use webxr_api::SessionBuilder;
use webxr_api::SessionMode;
use webxr_api::Viewer;
use webxr_api::Views;

use euclid::Size2D;
use euclid::TypedRigidTransform3D;

use gleam::gl::GLsync;
use gleam::gl::GLuint;

pub struct HeadlessMockDiscovery(());

struct HeadlessDiscovery {
    init: MockDeviceInit,
    receiver: Option<Receiver<MockDeviceMsg>>,
}

struct HeadlessDevice {
    floor_transform: TypedRigidTransform3D<f32, Native, Floor>,
    viewer_origin: TypedRigidTransform3D<f32, Native, Viewer>,
    views: Views,
    receiver: Receiver<MockDeviceMsg>,
}

impl MockDiscovery for HeadlessMockDiscovery {
    fn simulate_device_connection(
        &mut self,
        init: MockDeviceInit,
        receiver: Receiver<MockDeviceMsg>,
    ) -> Result<Box<dyn Discovery>, Error> {
        Ok(Box::new(HeadlessDiscovery {
            init,
            receiver: Some(receiver),
        }))
    }
}

impl Discovery for HeadlessDiscovery {
    fn request_session(&mut self, mode: SessionMode, xr: SessionBuilder) -> Result<Session, Error> {
        if !self.supports_session(mode) {
            return Err(Error::NoMatchingDevice);
        }
        let receiver = self.receiver.take().ok_or(Error::NoMatchingDevice)?;
        let viewer_origin = self.init.viewer_origin.clone();
        let floor_transform = self
            .init
            .local_to_floor_level_transform
            .pre_mul(&viewer_origin);
        let views = self.init.views.clone();
        xr.spawn(move || {
            Ok(HeadlessDevice {
                floor_transform,
                viewer_origin,
                views,
                receiver,
            })
        })
    }

    fn supports_session(&self, mode: SessionMode) -> bool {
        mode == SessionMode::Inline || self.init.supports_immersive
    }
}

impl Device for HeadlessDevice {
    fn floor_transform(&self) -> TypedRigidTransform3D<f32, Native, Floor> {
        self.floor_transform.clone()
    }

    fn views(&self) -> Views {
        self.views.clone()
    }

    fn wait_for_animation_frame(&mut self) -> Frame {
        while let Ok(msg) = self.receiver.try_recv() {
            self.handle_msg(msg);
        }
        let transform = self.viewer_origin.inverse();
        Frame { transform }
    }

    fn render_animation_frame(&mut self, _: GLuint, _: Size2D<i32>, _: GLsync) {}
}

impl HeadlessDevice {
    fn handle_msg(&mut self, msg: MockDeviceMsg) {
        match msg {
            MockDeviceMsg::SetViewerOrigin(viewer_origin) => {
                self.viewer_origin = viewer_origin;
            }
            MockDeviceMsg::SetViews(views) => {
                self.views = views;
            }
            MockDeviceMsg::Focus => {
                // TODO
            }
            MockDeviceMsg::Blur => {
                // TODO
            }
        }
    }
}
