/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::Device;
use crate::Error;
use crate::Floor;
use crate::Frame;
use crate::Native;
use crate::Receiver;
use crate::Sender;
use crate::Views;
use crate::WebGLExternalImageApi;

use euclid::TypedRigidTransform3D;

use std::cell::Cell;
use std::thread;

#[cfg(feature = "ipc")]
use serde::{Deserialize, Serialize};

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
}

/// An object that represents an XR session.
/// This is owned by the content thread.
/// https://www.w3.org/TR/webxr/#xrsession-interface
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
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
}

/// For devices that want to do their own thread management, the `SessionThread` type is exposed.
pub struct SessionThread<D> {
    receiver: Receiver<SessionMsg>,
    sender: Sender<SessionMsg>,
    images: Option<Box<dyn WebGLExternalImageApi>>,
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
                SessionMsg::UpdateWebGLExternalImageApi(images) => {
                    self.images = Some(images);
                }
                SessionMsg::RequestAnimationFrame(mut callback) => {
                    let timestamp = self.timestamp;
                    let frame = self.device.wait_for_animation_frame();
                    self.timestamp += 1.0;
                    callback.callback(timestamp, frame);
                }
                SessionMsg::RenderAnimationFrame => {
                    if let Some(ref images) = self.images {
                        if let Ok((texture_id, size, sync)) = images.lock() {
                            self.device.render_animation_frame(texture_id, size, sync);
                            images.unlock();
                        }
                    }
                }
            }
        }
    }
}

/// A type for building XR sessions
pub struct SessionBuilder {
    // This field is just used to make the type !Sync and !Send, for futureproofing
    #[allow(dead_code)]
    not_sync_or_send: Cell<()>,
}

impl SessionBuilder {
    pub(crate) fn new() -> SessionBuilder {
        SessionBuilder {
            not_sync_or_send: Cell::new(()),
        }
    }

    /// For devices which want to do their own thread management,
    /// e.g. where the session thread has to be run on the main thread.
    pub fn new_thread<D: Device>(self, device: D) -> Result<SessionThread<D>, Error> {
        let (sender, receiver) = crate::channel().or(Err(Error::CommunicationError))?;
        let timestamp = 0.0;
        let images = None;
        Ok(SessionThread {
            sender,
            receiver,
            device,
            images,
            timestamp,
        })
    }

    /// For devices which are happy to hand over thread management to webxr.
    pub fn spawn<D, F>(self, factory: F) -> Result<Session, Error>
    where
        F: 'static + FnOnce() -> Result<D, Error> + Send,
        D: Device,
    {
        let (acks, ackr) = crate::channel().or(Err(Error::CommunicationError))?;
        thread::spawn(
            move || match factory().and_then(|device| self.new_thread(device)) {
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
}
