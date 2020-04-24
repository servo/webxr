/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::DeviceAPI;
use crate::Error;
use crate::Event;
use crate::Floor;
use crate::Frame;
use crate::FrameUpdateEvent;
use crate::HitTestId;
use crate::HitTestSource;
use crate::InputSource;
use crate::Native;
use crate::Receiver;
use crate::Sender;
use crate::SwapChainId;
use crate::Viewport;
use crate::Views;

use euclid::RigidTransform3D;
use euclid::Size2D;

use log::warn;

use std::thread;
use std::time::Duration;

use surfman_chains_api::SwapChainAPI;
use surfman_chains_api::SwapChainsAPI;

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

/// https://immersive-web.github.io/webxr/#dictdef-xrsessioninit
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
pub struct SessionInit {
    pub required_features: Vec<String>,
    pub optional_features: Vec<String>,
}

impl SessionInit {
    /// Helper function for validating a list of requested features against
    /// a list of supported features for a given mode
    pub fn validate(&self, mode: SessionMode, supported: &[String]) -> Result<Vec<String>, Error> {
        for f in &self.required_features {
            // viewer and local in immersive are granted by default
            // https://immersive-web.github.io/webxr/#default-features
            if f == "viewer" || (f == "local" && mode != SessionMode::Inline) {
                continue;
            }

            if !supported.contains(f) {
                return Err(Error::UnsupportedFeature(f.into()));
            }
        }
        let mut granted = self.required_features.clone();
        for f in &self.optional_features {
            if f == "viewer"
                || (f == "local" && mode != SessionMode::Inline)
                || supported.contains(f)
            {
                granted.push(f.clone());
            }
        }

        Ok(granted)
    }
}

#[cfg(feature = "profile")]
fn to_ms(ns: u64) -> f64 {
    ns as f64 / 1_000_000.
}

/// https://immersive-web.github.io/webxr-ar-module/#xrenvironmentblendmode-enum
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
pub enum EnvironmentBlendMode {
    Opaque,
    AlphaBlend,
    Additive,
}

// The messages that are sent from the content thread to the session thread.
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
enum SessionMsg {
    SetSwapChain(Option<SwapChainId>),
    SetEventDest(Sender<Event>),
    UpdateClipPlanes(/* near */ f32, /* far */ f32),
    StartRenderLoop,
    RenderAnimationFrame(/* request time */ u64),
    RequestHitTest(HitTestSource),
    CancelHitTest(HitTestId),
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
    floor_transform: Option<RigidTransform3D<f32, Native, Floor>>,
    views: Views,
    resolution: Option<Size2D<i32, Viewport>>,
    sender: Sender<SessionMsg>,
    environment_blend_mode: EnvironmentBlendMode,
    initial_inputs: Vec<InputSource>,
    granted_features: Vec<String>,
    id: SessionId,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(feature = "ipc", derive(Deserialize, Serialize))]
pub struct SessionId(pub(crate) u32);

impl Session {
    pub fn id(&self) -> SessionId {
        self.id
    }

    pub fn floor_transform(&self) -> Option<RigidTransform3D<f32, Native, Floor>> {
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
            .expect("Inline XR sessions should not construct a framebuffer")
    }

    pub fn set_swap_chain(&mut self, swap_chain_id: Option<SwapChainId>) {
        let _ = self.sender.send(SessionMsg::SetSwapChain(swap_chain_id));
    }

    pub fn start_render_loop(&mut self) {
        let _ = self.sender.send(SessionMsg::StartRenderLoop);
    }

    pub fn update_clip_planes(&mut self, near: f32, far: f32) {
        let _ = self.sender.send(SessionMsg::UpdateClipPlanes(near, far));
    }

    pub fn set_event_dest(&mut self, dest: Sender<Event>) {
        let _ = self.sender.send(SessionMsg::SetEventDest(dest));
    }

    pub fn render_animation_frame(&mut self) {
        #[allow(unused)]
        let mut time = 0;
        #[cfg(feature = "profile")]
        {
            time = time::precise_time_ns();
        }
        let _ = self.sender.send(SessionMsg::RenderAnimationFrame(time));
    }

    pub fn end_session(&mut self) {
        let _ = self.sender.send(SessionMsg::Quit);
    }

    pub fn apply_event(&mut self, event: FrameUpdateEvent) {
        match event {
            FrameUpdateEvent::UpdateViews(views) => self.views = views,
            FrameUpdateEvent::UpdateFloorTransform(floor) => self.floor_transform = floor,
            FrameUpdateEvent::HitTestSourceAdded(_) => (),
        }
    }

    pub fn granted_features(&self) -> &[String] {
        &self.granted_features
    }

    pub fn request_hit_test(&self, source: HitTestSource) {
        let _ = self.sender.send(SessionMsg::RequestHitTest(source));
    }

    pub fn cancel_hit_test(&self, id: HitTestId) {
        let _ = self.sender.send(SessionMsg::CancelHitTest(id));
    }
}

/// For devices that want to do their own thread management, the `SessionThread` type is exposed.
pub struct SessionThread<Device, SwapChains: SwapChainsAPI<SwapChainId>> {
    receiver: Receiver<SessionMsg>,
    sender: Sender<SessionMsg>,
    swap_chain: Option<SwapChains::SwapChain>,
    swap_chains: SwapChains,
    frame_count: u64,
    frame_sender: Sender<Frame>,
    running: bool,
    device: Device,
    id: SessionId,
}

impl<Device, SwapChains> SessionThread<Device, SwapChains>
where
    Device: DeviceAPI<SwapChains::Surface>,
    SwapChains: SwapChainsAPI<SwapChainId>,
{
    pub fn new(
        mut device: Device,
        swap_chains: SwapChains,
        frame_sender: Sender<Frame>,
        id: SessionId,
    ) -> Result<Self, Error> {
        let (sender, receiver) = crate::channel().or(Err(Error::CommunicationError))?;
        device.set_quitter(Quitter {
            sender: sender.clone(),
        });
        let frame_count = 0;
        let swap_chain = None;
        let running = true;
        Ok(SessionThread {
            sender,
            receiver,
            device,
            swap_chain,
            swap_chains,
            frame_count,
            frame_sender,
            running,
            id,
        })
    }

    pub fn new_session(&mut self) -> Session {
        let floor_transform = self.device.floor_transform();
        let views = self.device.views();
        let resolution = self.device.recommended_framebuffer_resolution();
        let sender = self.sender.clone();
        let initial_inputs = self.device.initial_inputs();
        let environment_blend_mode = self.device.environment_blend_mode();
        let granted_features = self.device.granted_features().into();
        Session {
            floor_transform,
            views,
            resolution,
            sender,
            initial_inputs,
            environment_blend_mode,
            granted_features,
            id: self.id,
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
            SessionMsg::RequestHitTest(source) => {
                self.device.request_hit_test(source);
            }
            SessionMsg::CancelHitTest(id) => {
                self.device.cancel_hit_test(id);
            }
            SessionMsg::StartRenderLoop => {
                let frame = match self.device.wait_for_animation_frame() {
                    Some(frame) => frame,
                    None => {
                        warn!("Device stopped providing frames, exiting");
                        return false;
                    }
                };

                let _ = self.frame_sender.send(frame);
            }
            SessionMsg::UpdateClipPlanes(near, far) => self.device.update_clip_planes(near, far),
            SessionMsg::RenderAnimationFrame(_sent_time) => {
                self.frame_count += 1;
                #[cfg(feature = "profile")]
                let mut render_start = None;
                if let Some(ref swap_chain) = self.swap_chain {
                    if let Some(surface) = swap_chain.take_surface() {
                        #[cfg(feature = "profile")]
                        {
                            render_start = Some(time::precise_time_ns());
                            println!(
                                "WEBXR PROFILING [raf transmitted]:\t{}ms",
                                to_ms(render_start.unwrap() - _sent_time)
                            );
                        }
                        let surface = self.device.render_animation_frame(surface);
                        swap_chain.recycle_surface(surface);
                    } else {
                        warn!("no surface; not rendering");
                    }
                }
                #[cfg(feature = "profile")]
                let wait_start = time::precise_time_ns();
                #[cfg(feature = "profile")]
                {
                    if let Some(render_start) = render_start {
                        println!(
                            "WEBXR PROFILING [raf render]:\t{}ms",
                            to_ms(wait_start - render_start)
                        );
                    }
                }
                #[allow(unused_mut)]
                let mut frame = match self.device.wait_for_animation_frame() {
                    Some(frame) => frame,
                    None => {
                        warn!("Device stopped providing frames, exiting");
                        return false;
                    }
                };
                #[cfg(feature = "profile")]
                {
                    let wait_end = time::precise_time_ns();
                    println!(
                        "WEBXR PROFILING [raf wait]:\t{}ms",
                        to_ms(wait_end - wait_start)
                    );
                    frame.sent_time = wait_end;
                }
                let _ = self.frame_sender.send(frame);
            }
            SessionMsg::Quit => {
                self.device.quit();
                return false;
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

impl<Device, SwapChains> MainThreadSession for SessionThread<Device, SwapChains>
where
    Device: DeviceAPI<SwapChains::Surface>,
    SwapChains: SwapChainsAPI<SwapChainId>,
{
    fn run_one_frame(&mut self) {
        let frame_count = self.frame_count;
        #[cfg(feature = "profile")]
        let start_run = time::precise_time_ns();
        while frame_count == self.frame_count && self.running {
            if let Ok(msg) = crate::recv_timeout(&self.receiver, TIMEOUT) {
                self.running = self.handle_msg(msg);
            } else {
                break;
            }
        }
        #[cfg(feature = "profile")]
        {
            let end_run = time::precise_time_ns();
            println!(
                "WEBXR PROFILING [run_one_frame]:\t{}ms",
                to_ms(end_run - start_run)
            );
        }
    }

    fn running(&self) -> bool {
        self.running
    }
}

/// A type for building XR sessions
pub struct SessionBuilder<'a, SwapChains: 'a> {
    swap_chains: &'a SwapChains,
    sessions: &'a mut Vec<Box<dyn MainThreadSession>>,
    frame_sender: Sender<Frame>,
    id: SessionId,
}

impl<'a, SwapChains> SessionBuilder<'a, SwapChains>
where
    SwapChains: SwapChainsAPI<SwapChainId>,
{
    pub fn id(&self) -> SessionId {
        self.id
    }

    pub(crate) fn new(
        swap_chains: &'a SwapChains,
        sessions: &'a mut Vec<Box<dyn MainThreadSession>>,
        frame_sender: Sender<Frame>,
        id: SessionId,
    ) -> Self {
        SessionBuilder {
            swap_chains,
            sessions,
            frame_sender,
            id,
        }
    }

    /// For devices which are happy to hand over thread management to webxr.
    pub fn spawn<Device, Factory>(self, factory: Factory) -> Result<Session, Error>
    where
        Factory: 'static + FnOnce() -> Result<Device, Error> + Send,
        Device: DeviceAPI<SwapChains::Surface>,
    {
        let (acks, ackr) = crate::channel().or(Err(Error::CommunicationError))?;
        let swap_chains = self.swap_chains.clone();
        let frame_sender = self.frame_sender.clone();
        let id = self.id;
        thread::spawn(move || {
            match factory()
                .and_then(|device| SessionThread::new(device, swap_chains, frame_sender, id))
            {
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
    pub fn run_on_main_thread<Device, Factory>(self, factory: Factory) -> Result<Session, Error>
    where
        Factory: 'static + FnOnce() -> Result<Device, Error>,
        Device: DeviceAPI<SwapChains::Surface>,
    {
        let device = factory()?;
        let swap_chains = self.swap_chains.clone();
        let frame_sender = self.frame_sender.clone();
        let mut session_thread = SessionThread::new(device, swap_chains, frame_sender, self.id)?;
        let session = session_thread.new_session();
        self.sessions.push(Box::new(session_thread));
        Ok(session)
    }
}
