/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::Device;
use crate::Error;
use crate::Floor;
use crate::Frame;
use crate::InputSource;
use crate::Native;
use crate::Receiver;
use crate::Sender;
use crate::Viewport;
use crate::Views;
use crate::WebGLExternalImageApi;

use euclid::TypedRigidTransform3D;
use euclid::TypedSize2D;

use std::thread;
use std::time::Duration;

#[cfg(feature = "ipc")]
use serde::{Deserialize, Serialize};

// How long to wait for an rAF.
static TIMEOUT: Duration = Duration::from_millis(5);

/// https://www.w3.org/TR/webxr/#xrsessionmode-enum
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
pub enum SessionMode {
    Inline,
    ImmersiveVR,
    ImmersiveAR,
}

/// https://www.w3.org/TR/hr-time/#dom-domhighrestimestamp
pub type HighResTimeStamp = f64;

/// https://www.w3.org/TR/webxr/#callbackdef-xrframerequestcallback
#[cfg_attr(feature = "ipc", typetag::serde)]
pub trait FrameRequestCallback: 'static + Send {
    fn callback(&mut self, time: HighResTimeStamp, frame: Frame);
}

// The messages that are sent from the content thread to the session thread.
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
enum SessionMsg {
    UpdateWebGLExternalImageApi(Box<dyn WebGLExternalImageApi>),
    RequestAnimationFrame(Box<dyn FrameRequestCallback>),
    RenderAnimationFrame,
    Quit,
}

/// An object that represents an XR session.
/// This is owned by the content thread.
/// https://www.w3.org/TR/webxr/#xrsession-interface
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
pub struct Session {
    floor_transform: TypedRigidTransform3D<f32, Native, Floor>,
    views: Views,
    resolution: TypedSize2D<i32, Viewport>,
    sender: Sender<SessionMsg>,
    initial_inputs: Vec<InputSource>,
}

impl Session {
    pub fn floor_transform(&self) -> TypedRigidTransform3D<f32, Native, Floor> {
        self.floor_transform.clone()
    }

    pub fn initial_inputs(&self) -> &[InputSource] {
        &self.initial_inputs
    }

    pub fn views(&self) -> Views {
        self.views.clone()
    }

    pub fn recommended_framebuffer_resolution(&self) -> TypedSize2D<i32, Viewport> {
        self.resolution
    }

    pub fn update_webgl_external_image_api<I>(&mut self, images: I)
    where
        I: WebGLExternalImageApi,
    {
        let _ = self
            .sender
            .send(SessionMsg::UpdateWebGLExternalImageApi(Box::new(images)));
    }

    pub fn request_animation_frame<C>(&mut self, callback: C)
    where
        C: FrameRequestCallback,
    {
        let _ = self
            .sender
            .send(SessionMsg::RequestAnimationFrame(Box::new(callback)));
    }

    pub fn render_animation_frame(&mut self) {
        let _ = self.sender.send(SessionMsg::RenderAnimationFrame);
    }

    pub fn end_session(&mut self) {
        let _ = self.sender.send(SessionMsg::Quit);
    }
}

/// For devices that want to do their own thread management, the `SessionThread` type is exposed.
pub struct SessionThread<D> {
    receiver: Receiver<SessionMsg>,
    sender: Sender<SessionMsg>,
    images: Option<Box<dyn WebGLExternalImageApi>>,
    timestamp: HighResTimeStamp,
    running: bool,
    device: D,
}

impl<D: Device> SessionThread<D> {
    pub fn new(device: D) -> Result<SessionThread<D>, Error> {
        let (sender, receiver) = crate::channel().or(Err(Error::CommunicationError))?;
        let timestamp = 0.0;
        let images = None;
        let running = true;
        Ok(SessionThread {
            sender,
            receiver,
            device,
            images,
            timestamp,
            running,
        })
    }

    pub fn new_session(&mut self) -> Session {
        let floor_transform = self.device.floor_transform();
        let views = self.device.views();
        let resolution = self.device.recommended_framebuffer_resolution();
        let sender = self.sender.clone();
        let initial_inputs = self.device.initial_inputs();
        Session {
            floor_transform,
            views,
            resolution,
            sender,
            initial_inputs,
        }
    }

    pub fn run(&mut self) {
        while let Ok(msg) = self.receiver.recv() {
            self.handle_msg(msg);
        }
    }

    fn handle_msg(&mut self, msg: SessionMsg) {
        match msg {
            SessionMsg::UpdateWebGLExternalImageApi(images) => {
                self.images = Some(images);
            }
            SessionMsg::RequestAnimationFrame(mut callback) => {
                let timestamp = self.timestamp;
                let frame = self.device.wait_for_animation_frame();
                callback.callback(timestamp, frame);
            }
            SessionMsg::RenderAnimationFrame => {
                self.timestamp += 1.0;
                if let Some(ref images) = self.images {
                    if let Ok((texture_id, size, sync)) = images.lock() {
                        self.device.render_animation_frame(texture_id, size, sync);
                        images.unlock();
                    }
                }
            }
            SessionMsg::Quit => {
                self.running = false;
            }
        }
    }
}

/// Devices that need to can run sessions on the main thread.
pub trait MainThreadSession: 'static {
    fn run_one_frame(&mut self);
    fn running(&self) -> bool;
}

impl<D: Device> MainThreadSession for SessionThread<D> {
    fn run_one_frame(&mut self) {
        let timestamp = self.timestamp;
        while timestamp == self.timestamp && self.running {
            if let Ok(msg) = crate::recv_timeout(&self.receiver, TIMEOUT) {
                self.handle_msg(msg);
            }
        }
        while let Ok(msg) = self.receiver.try_recv() {
            self.handle_msg(msg);
        }
    }

    fn running(&self) -> bool {
        self.running
    }
}

/// A type for building XR sessions
pub struct SessionBuilder<'a> {
    sessions: &'a mut Vec<Box<dyn MainThreadSession>>,
}

impl<'a> SessionBuilder<'a> {
    pub(crate) fn new(sessions: &'a mut Vec<Box<dyn MainThreadSession>>) -> SessionBuilder {
        SessionBuilder { sessions }
    }

    /// For devices which are happy to hand over thread management to webxr.
    pub fn spawn<D, F>(self, factory: F) -> Result<Session, Error>
    where
        F: 'static + FnOnce() -> Result<D, Error> + Send,
        D: Device,
    {
        let (acks, ackr) = crate::channel().or(Err(Error::CommunicationError))?;
        thread::spawn(
            move || match factory().and_then(|device| SessionThread::new(device)) {
                Ok(mut thread) => {
                    let session = thread.new_session();
                    let _ = acks.send(Ok(session));
                    thread.run();
                }
                Err(err) => {
                    let _ = acks.send(Err(err));
                }
            },
        );
        ackr.recv().unwrap_or(Err(Error::CommunicationError))
    }

    /// For devices that need to run on the main thread.
    pub fn run_on_main_thread<D, F>(self, factory: F) -> Result<Session, Error>
    where
        F: 'static + FnOnce() -> Result<D, Error>,
        D: Device,
    {
        let device = factory()?;
        let mut session_thread = SessionThread::new(device)?;
        let session = session_thread.new_session();
        self.sessions.push(Box::new(session_thread));
        Ok(session)
    }
}
