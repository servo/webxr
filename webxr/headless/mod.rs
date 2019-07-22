/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use webxr_api::Device;
use webxr_api::Discovery;
use webxr_api::Error;
use webxr_api::Event;
use webxr_api::EventBuffer;
use webxr_api::EventCallback;
use webxr_api::Floor;
use webxr_api::Frame;
use webxr_api::Input;
use webxr_api::InputFrame;
use webxr_api::InputSource;
use webxr_api::MockDeviceInit;
use webxr_api::MockDeviceMsg;
use webxr_api::MockDiscovery;
use webxr_api::MockInputMsg;
use webxr_api::Native;
use webxr_api::Receiver;
use webxr_api::Sender;
use webxr_api::Session;
use webxr_api::SessionBuilder;
use webxr_api::SessionMode;
use webxr_api::Viewer;
use webxr_api::Views;

use euclid::Size2D;
use euclid::TypedRigidTransform3D;

use gleam::gl;
use gleam::gl::GLsync;
use gleam::gl::GLuint;
use gleam::gl::Gl;

use std::rc::Rc;

pub struct HeadlessMockDiscovery {
    gl: Rc<dyn Gl>,
}

struct HeadlessDiscovery {
    gl: Rc<dyn Gl>,
    init: MockDeviceInit,
    receiver: Option<Receiver<MockDeviceMsg>>,
}

struct InputInfo {
    source: InputSource,
    active: bool,
    pointer: TypedRigidTransform3D<f32, Input, Native>,
}

struct HeadlessDevice {
    gl: Rc<dyn Gl>,
    floor_transform: TypedRigidTransform3D<f32, Native, Floor>,
    viewer_origin: TypedRigidTransform3D<f32, Viewer, Native>,
    views: Views,
    receiver: Receiver<MockDeviceMsg>,
    events: EventBuffer,
    inputs: Vec<InputInfo>,
    disconnect_callbacks: Vec<Sender<()>>,
    connected: bool,
}

impl MockDiscovery for HeadlessMockDiscovery {
    fn simulate_device_connection(
        &mut self,
        init: MockDeviceInit,
        receiver: Receiver<MockDeviceMsg>,
    ) -> Result<Box<dyn Discovery>, Error> {
        Ok(Box::new(HeadlessDiscovery {
            gl: self.gl.clone(),
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
        let gl = self.gl.clone();
        let receiver = self.receiver.take().ok_or(Error::NoMatchingDevice)?;
        let viewer_origin = self.init.viewer_origin.clone();
        let floor_transform = self.init.floor_origin.inverse();
        let views = self.init.views.clone();
        xr.run_on_main_thread(move || {
            Ok(HeadlessDevice {
                gl,
                floor_transform,
                viewer_origin,
                views,
                receiver,
                events: Default::default(),
                disconnect_callbacks: vec![],
                connected: true,
                inputs: vec![],
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
        let transform = self.viewer_origin;
        let inputs = self
            .inputs
            .iter()
            .filter(|i| i.active)
            .map(|i| InputFrame {
                id: i.source.id,
                target_ray_origin: i.pointer,
            })
            .collect();
        Frame { transform, inputs }
    }

    fn render_animation_frame(&mut self, _: GLuint, _: Size2D<i32>, sync: GLsync) {
        self.gl.wait_sync(sync, 0, gl::TIMEOUT_IGNORED);
    }

    fn initial_inputs(&self) -> Vec<InputSource> {
        vec![]
    }

    fn set_event_callback(&mut self, callback: Box<dyn EventCallback>) {
        self.events.upgrade(callback)
    }

    fn connected(&mut self) -> bool {
        if self.connected {
            true
        } else {
            for callback in self.disconnect_callbacks.drain(..) {
                let _ = callback.send(());
            }
            false
        }
    }

    fn quit(&mut self) {
        self.connected = false;
        self.events.callback(Event::SessionEnd);
    }
}

impl HeadlessMockDiscovery {
    pub fn new(gl: Rc<dyn Gl>) -> HeadlessMockDiscovery {
        HeadlessMockDiscovery { gl }
    }
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
            MockDeviceMsg::AddInputSource(init) => {
                self.inputs.push(InputInfo {
                    source: init.source,
                    pointer: init.pointer_origin,
                    active: true,
                });
                self.events.callback(Event::AddInput(init.source))
            }
            MockDeviceMsg::MessageInputSource(id, msg) => {
                if let Some(ref mut input) = self.inputs.iter_mut().find(|i| i.source.id == id) {
                    match msg {
                        MockInputMsg::SetHandedness(h) => input.source.handedness = h,
                        MockInputMsg::SetTargetRayMode(t) => input.source.target_ray_mode = t,
                        MockInputMsg::SetPointerOrigin(p) => input.pointer = p,
                        MockInputMsg::Disconnect => input.active = false,
                        MockInputMsg::Reconnect => input.active = true,
                    }
                }
            }
            MockDeviceMsg::Disconnect(sender) => {
                self.connected = false;
                self.disconnect_callbacks.push(sender);
                self.events.callback(Event::SessionEnd);
            }
        }
    }
}
