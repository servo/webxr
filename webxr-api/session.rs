/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::Device;
use crate::Error;
use crate::Event;
use crate::Floor;
use crate::Frame;
use crate::FrameUpdateEvent;
use crate::InputSource;
use crate::Native;
use crate::Receiver;
use crate::Sender;
use crate::SwapChainId;
use crate::Viewport;
use crate::Views;

use euclid::RigidTransform3D;
use euclid::Size2D;

use std::thread;
use std::time::Duration;

use surfman_chains::SwapChain;
use surfman_chains::SwapChains;

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

/// https://immersive-web.github.io/webxr-ar-module/#xrenvironmentblendmode-enum
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
pub enum EnvironmentBlendMode {
    Opaque,
    AlphaBlend,
    Additive,
}

/// https://www.w3.org/TR/hr-time/#dom-domhighrestimestamp
pub type HighResTimeStamp = f64;

// The messages that are sent from the content thread to the session thread.
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
enum SessionMsg {
    SetSwapChain(Option<SwapChainId>),
    SetEventDest(Sender<Event>),
    UpdateClipPlanes(/* near */ f32, /* far */ f32),
    RequestAnimationFrame(Sender<(HighResTimeStamp, Frame)>),
    RenderAnimationFrame,
    Quit,
}

#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
#[derive(Clone)]
pub struct Quitter {
    sender: Sender<SessionMsg>,
}

impl Quitter {
    pub fn quit(&self) {
        let _ = self.sender.send(SessionMsg::Quit);
    }
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
    environment_blend_mode: EnvironmentBlendMode,
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

    pub fn environment_blend_mode(&self) -> EnvironmentBlendMode {
        self.environment_blend_mode
    }

    pub fn recommended_framebuffer_resolution(&self) -> Size2D<i32, Viewport> {
        self.resolution
    }

    pub fn set_swap_chain(&mut self, swap_chain_id: Option<SwapChainId>) {
        let _ = self.sender.send(SessionMsg::SetSwapChain(swap_chain_id));
    }

    pub fn request_animation_frame(&mut self, dest: Sender<(HighResTimeStamp, Frame)>) {
        let _ = self.sender.send(SessionMsg::RequestAnimationFrame(dest));
    }

    pub fn update_clip_planes(&mut self, near: f32, far: f32) {
        let _ = self.sender.send(SessionMsg::UpdateClipPlanes(near, far));
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

    pub fn apply_event(&mut self, event: FrameUpdateEvent) {
        match event {
            FrameUpdateEvent::UpdateViews(views) => self.views = views,
        }
    }
}

/// For devices that want to do their own thread management, the `SessionThread` type is exposed.
pub struct SessionThread<D> {
    receiver: Receiver<SessionMsg>,
    sender: Sender<SessionMsg>,
    swap_chain: Option<SwapChain>,
    swap_chains: SwapChains<SwapChainId>,
    timestamp: HighResTimeStamp,
    running: bool,
    device: D,
}

impl<D: Device> SessionThread<D> {
    pub fn new(
        mut device: D,
        swap_chains: SwapChains<SwapChainId>,
    ) -> Result<SessionThread<D>, Error> {
        let (sender, receiver) = crate::channel().or(Err(Error::CommunicationError))?;
        device.set_quitter(Quitter {
            sender: sender.clone(),
        });
        let timestamp = 0.0;
        let swap_chain = None;
        let running = true;
        Ok(SessionThread {
            sender,
            receiver,
            device,
            swap_chain,
            swap_chains,
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
        let environment_blend_mode = self.device.environment_blend_mode();
        Session {
            floor_transform,
            views,
            resolution,
            sender,
            initial_inputs,
            environment_blend_mode,
        }
    }

    pub fn run(&mut self) {
        loop {
            if let Ok(msg) = self.receiver.recv() {
                if !self.handle_msg(msg) {
                    self.running = false;
                    break;
                }
            } else {
                break;
            }
        }
    }

    fn handle_msg(&mut self, msg: SessionMsg) -> bool {
        match msg {
            SessionMsg::SetSwapChain(swap_chain_id) => {
                self.swap_chain = swap_chain_id.and_then(|id| self.swap_chains.get(id));
            }
            SessionMsg::SetEventDest(dest) => {
                self.device.set_event_dest(dest);
            }
            SessionMsg::RequestAnimationFrame(dest) => {
                let timestamp = self.timestamp;
                match self.device.wait_for_animation_frame() {
                    Some(frame) => {
                        let _ = dest.send((timestamp, frame));
                    }
                    None => {
                        return false;
                    }
                };
            }
            SessionMsg::UpdateClipPlanes(near, far) => self.device.update_clip_planes(near, far),
            SessionMsg::RenderAnimationFrame => {
                self.timestamp += 1.0;
                if let Some(ref swap_chain) = self.swap_chain {
                    if let Some(surface) = swap_chain.take_surface() {
                        let surface = self.device.render_animation_frame(surface);
                        swap_chain.recycle_surface(surface);
                    }
                }
            }
            SessionMsg::Quit => {
                self.device.quit();
            }
        }
        true
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
                self.running = self.handle_msg(msg);
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
    swap_chains: &'a SwapChains<SwapChainId>,
    sessions: &'a mut Vec<Box<dyn MainThreadSession>>,
}

impl<'a> SessionBuilder<'a> {
    pub(crate) fn new(
        swap_chains: &'a SwapChains<SwapChainId>,
        sessions: &'a mut Vec<Box<dyn MainThreadSession>>,
    ) -> SessionBuilder<'a> {
        SessionBuilder {
            swap_chains,
            sessions,
        }
    }

    /// For devices which are happy to hand over thread management to webxr.
    pub fn spawn<D, F>(self, factory: F) -> Result<Session, Error>
    where
        F: 'static + FnOnce() -> Result<D, Error> + Send,
        D: Device,
    {
        let (acks, ackr) = crate::channel().or(Err(Error::CommunicationError))?;
        let swap_chains = self.swap_chains.clone();
        thread::spawn(move || {
            match factory().and_then(|device| SessionThread::new(device, swap_chains)) {
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
        let swap_chains = self.swap_chains.clone();
        let mut session_thread = SessionThread::new(device, swap_chains)?;
        let session = session_thread.new_session();
        self.sessions.push(Box::new(session_thread));
        Ok(session)
    }
}
