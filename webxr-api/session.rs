/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::Device;
use crate::Error;
use crate::Event;
use crate::Floor;
use crate::Frame;
use crate::InputSource;
use crate::Native;
use crate::Receiver;
use crate::Sender;
use crate::Viewport;
use crate::Views;
use crate::WebGLContextId;
use crate::WebGLExternalImageApi;
use crate::WebGLTextureId;

use euclid::default::Size2D as UntypedSize2D;
use euclid::RigidTransform3D;
use euclid::Size2D;

use gleam::gl::GLsizei;

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

// The messages that are sent from the content thread to the session thread.
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
enum SessionMsg {
    SetTexture(WebGLContextId, WebGLTextureId, UntypedSize2D<GLsizei>),
    SetEventDest(Sender<Event>),
    RequestAnimationFrame(Sender<(HighResTimeStamp, Frame)>),
    RenderAnimationFrame,
    Quit,
}

/// An object that represents an XR session.
/// This is owned by the content thread.
/// https://www.w3.org/TR/webxr/#xrsession-interface
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
pub struct Session {
    floor_transform: RigidTransform3D<f32, Native, Floor>,
    views: Views,
    resolution: Size2D<i32, Viewport>,
    sender: Sender<SessionMsg>,
    initial_inputs: Vec<InputSource>,
}

impl Session {
    pub fn floor_transform(&self) -> RigidTransform3D<f32, Native, Floor> {
        self.floor_transform.clone()
    }

    pub fn initial_inputs(&self) -> &[InputSource] {
        &self.initial_inputs
    }

    pub fn views(&self) -> Views {
        self.views.clone()
    }

    pub fn recommended_framebuffer_resolution(&self) -> Size2D<i32, Viewport> {
        self.resolution
    }

    pub fn set_texture(
        &mut self,
        ctxt: WebGLContextId,
        txt: WebGLTextureId,
        size: UntypedSize2D<GLsizei>,
    ) {
        let _ = self.sender.send(SessionMsg::SetTexture(ctxt, txt, size));
    }

    pub fn request_animation_frame(&mut self, dest: Sender<(HighResTimeStamp, Frame)>) {
        let _ = self.sender.send(SessionMsg::RequestAnimationFrame(dest));
    }

    pub fn set_event_dest(&mut self, dest: Sender<Event>) {
        let _ = self.sender.send(SessionMsg::SetEventDest(dest));
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
    webgl: Box<dyn WebGLExternalImageApi>,
    texture: Option<(WebGLContextId, WebGLTextureId, UntypedSize2D<GLsizei>)>,
    timestamp: HighResTimeStamp,
    running: bool,
    device: D,
}

impl<D: Device> SessionThread<D> {
    pub fn new(
        device: D,
        webgl: Box<dyn WebGLExternalImageApi>,
    ) -> Result<SessionThread<D>, Error> {
        let (sender, receiver) = crate::channel().or(Err(Error::CommunicationError))?;

        let timestamp = 0.0;
        let texture = None;
        let running = true;
        Ok(SessionThread {
            sender,
            receiver,
            device,
            webgl,
            texture,
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
        while self.running {
            if let Ok(msg) = self.receiver.recv() {
                self.handle_msg(msg);
            } else {
                break;
            }
        }
    }

    fn handle_msg(&mut self, msg: SessionMsg) {
        match msg {
            SessionMsg::SetTexture(ctxt, txt, size) => {
                self.texture = Some((ctxt, txt, size));
            }
            SessionMsg::SetEventDest(dest) => {
                self.device.set_event_dest(dest);
            }
            SessionMsg::RequestAnimationFrame(dest) => {
                let timestamp = self.timestamp;
                let frame = self.device.wait_for_animation_frame();
                let _ = dest.send((timestamp, frame));
            }
            SessionMsg::RenderAnimationFrame => {
                self.timestamp += 1.0;
                if let Some((ctxt, txt, size)) = self.texture {
                    let sync = self.webgl.lock(ctxt);
                    self.device.render_animation_frame(txt, size, sync);
                    self.webgl.unlock(ctxt);
                }
            }
            SessionMsg::Quit => {
                self.running = false;
                self.device.quit();
            }
        }
        self.running = self.running && self.device.connected();
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
            } else {
                break;
            }
        }
    }

    fn running(&self) -> bool {
        self.running
    }
}

/// A type for building XR sessions
pub struct SessionBuilder<'a> {
    webgl: &'a dyn WebGLExternalImageApi,
    sessions: &'a mut Vec<Box<dyn MainThreadSession>>,
}

impl<'a> SessionBuilder<'a> {
    pub(crate) fn new(
        webgl: &'a dyn WebGLExternalImageApi,
        sessions: &'a mut Vec<Box<dyn MainThreadSession>>,
    ) -> SessionBuilder<'a> {
        SessionBuilder { webgl, sessions }
    }

    /// For devices which are happy to hand over thread management to webxr.
    pub fn spawn<D, F>(self, factory: F) -> Result<Session, Error>
    where
        F: 'static + FnOnce() -> Result<D, Error> + Send,
        D: Device,
    {
        let (acks, ackr) = crate::channel().or(Err(Error::CommunicationError))?;
        let webgl = self.webgl.clone_box();
        thread::spawn(move || {
            match factory().and_then(|device| SessionThread::new(device, webgl)) {
                Ok(mut thread) => {
                    let session = thread.new_session();
                    let _ = acks.send(Ok(session));
                    thread.run();
                }
                Err(err) => {
                    let _ = acks.send(Err(err));
                }
            }
        });
        ackr.recv().unwrap_or(Err(Error::CommunicationError))
    }

    /// For devices that need to run on the main thread.
    pub fn run_on_main_thread<D, F>(self, factory: F) -> Result<Session, Error>
    where
        F: 'static + FnOnce() -> Result<D, Error>,
        D: Device,
    {
        let device = factory()?;
        let webgl = self.webgl.clone_box();
        let mut session_thread = SessionThread::new(device, webgl)?;
        let session = session_thread.new_session();
        self.sessions.push(Box::new(session_thread));
        Ok(session)
    }
}
