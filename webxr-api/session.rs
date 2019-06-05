/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::Device;
use crate::Error;
use crate::Floor;
use crate::Frame;
use crate::Native;
use crate::Views;
use crate::WebGLContextId;

use euclid::TypedRigidTransform3D;

use gleam::gl::Gl;

use std::rc::Rc;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::thread;

use webgl::GLFactory;
use webgl::WebGLExternalImages;

/// https://www.w3.org/TR/webxr/#xrsessionmode-enum
#[derive(Clone, Copy, Debug)]
pub enum SessionMode {
    Inline,
    ImmersiveVR,
    ImmersiveAR,
}

/// https://www.w3.org/TR/hr-time/#dom-domhighrestimestamp
pub type HighResTimeStamp = f64;

/// https://www.w3.org/TR/webxr/#callbackdef-xrframerequestcallback
pub type FrameRequestCallback = Box<'static + FnOnce(HighResTimeStamp, Frame) + Send>;

// The messages that are sent from the content thread to the session thread.
enum SessionMsg {
    RequestAnimationFrame(FrameRequestCallback),
    RenderAnimationFrame(WebGLContextId),
}

/// An object that represents an XR session.
/// This is owned by the content thread.
/// https://www.w3.org/TR/webxr/#xrsession-interface
pub struct Session {
    floor_transform: TypedRigidTransform3D<f32, Native, Floor>,
    views: Views,
    sender: Sender<SessionMsg>,
}

impl Session {
    pub fn floor_transform(&self) -> TypedRigidTransform3D<f32, Native, Floor> {
        self.floor_transform.clone()
    }

    pub fn views(&self) -> Views {
        self.views.clone()
    }

    pub fn request_animation_frame(&mut self, callback: FrameRequestCallback) {
        let _ = self
            .sender
            .send(SessionMsg::RequestAnimationFrame(callback));
    }

    pub fn render_animation_frame(&mut self, webgl: WebGLContextId) {
        let _ = self.sender.send(SessionMsg::RenderAnimationFrame(webgl));
    }
}

/// For devices that want to do their own thread management, the `SessionThread` type is exposed.
pub struct SessionThread<D> {
    receiver: Receiver<SessionMsg>,
    sender: Sender<SessionMsg>,
    images: WebGLExternalImages,
    timestamp: HighResTimeStamp,
    device: D,
}

impl<D: Device> SessionThread<D> {
    pub fn new_session(&mut self) -> Session {
        let floor_transform = self.device.floor_transform();
        let views = self.device.views();
        let sender = self.sender.clone();
        Session {
            floor_transform,
            views,
            sender,
        }
    }

    pub fn run(&mut self) {
        while let Ok(msg) = self.receiver.recv() {
            match msg {
                SessionMsg::RequestAnimationFrame(callback) => {
                    let timestamp = self.timestamp;
                    let frame = self.device.wait_for_animation_frame();
                    self.timestamp += 1.0;
                    callback(timestamp, frame);
                }
                SessionMsg::RenderAnimationFrame(ctx) => {
                    let (texture_id, size) = self.images.lock(ctx);
                    self.device.render_animation_frame(texture_id, size);
                    self.images.unlock(ctx);
                }
            }
        }
    }
}

/// A type for building XR sessions
#[derive(Clone)]
pub struct SessionBuilder {
    images: WebGLExternalImages,
    gl_factory: GLFactory,
}

impl SessionBuilder {
    /// For devices which want to do their own thread management,
    /// e.g. where the session thread has to be run on the main thread.
    pub fn new_thread<D: Device>(self, device: D) -> SessionThread<D> {
        let (sender, receiver) = mpsc::channel();
        let images = self.images.clone();
        let timestamp = 0.0;
        SessionThread {
            sender,
            receiver,
            device,
            images,
            timestamp,
        }
    }

    /// For devices which are happy to hand over thread management to webxr.
    pub fn spawn<D, F>(self, factory: F) -> Result<Session, Error>
    where
        F: 'static + FnOnce() -> Result<D, Error> + Send,
        D: Device,
    {
        let (acks, ackr) = mpsc::channel();
        thread::spawn(move || match factory() {
            Ok(device) => {
                let mut thread = self.new_thread(device);
                let session = thread.new_session();
                let _ = acks.send(Ok(session));
                thread.run();
            }
            Err(err) => {
                let _ = acks.send(Err(err));
            }
        });
        ackr.recv().unwrap_or(Err(Error::CommunicationError))
    }

    pub fn gl(&mut self) -> Rc<Gl> {
        self.gl_factory.build()
    }
}
