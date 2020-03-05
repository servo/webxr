/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::DiscoveryAPI;
use crate::Error;
use crate::Frame;
use crate::MainThreadSession;
use crate::MockDeviceInit;
use crate::MockDeviceMsg;
use crate::MockDiscoveryAPI;
use crate::Receiver;
use crate::Sender;
use crate::Session;
use crate::SessionBuilder;
use crate::SessionId;
use crate::SessionInit;
use crate::SessionMode;
use crate::SwapChainId;

use log::warn;

use surfman_chains_api::SwapChainsAPI;

#[cfg(feature = "ipc")]
use serde::{Deserialize, Serialize};

#[derive(Clone)]
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
pub struct Registry {
    sender: Sender<RegistryMsg>,
    waker: MainThreadWakerImpl,
}

pub struct MainThreadRegistry<SwapChains> {
    discoveries: Vec<Box<dyn DiscoveryAPI<SwapChains>>>,
    sessions: Vec<Box<dyn MainThreadSession>>,
    mocks: Vec<Box<dyn MockDiscoveryAPI<SwapChains>>>,
    swap_chains: Option<SwapChains>,
    sender: Sender<RegistryMsg>,
    receiver: Receiver<RegistryMsg>,
    waker: MainThreadWakerImpl,
    next_session_id: u32,
}

pub trait MainThreadWaker: 'static + Send {
    fn clone_box(&self) -> Box<dyn MainThreadWaker>;
    fn wake(&self);
}

impl Clone for Box<dyn MainThreadWaker> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

#[derive(Clone)]
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
struct MainThreadWakerImpl {
    #[cfg(feature = "ipc")]
    sender: Sender<()>,
    #[cfg(not(feature = "ipc"))]
    waker: Box<dyn MainThreadWaker>,
}

#[cfg(feature = "ipc")]
impl MainThreadWakerImpl {
    fn new(waker: Box<dyn MainThreadWaker>) -> Result<MainThreadWakerImpl, Error> {
        let (sender, receiver) = crate::channel().or(Err(Error::CommunicationError))?;
        ipc_channel::router::ROUTER
            .add_route(receiver.to_opaque(), Box::new(move |_| waker.wake()));
        Ok(MainThreadWakerImpl { sender })
    }

    fn wake(&self) {
        let _ = self.sender.send(());
    }
}

#[cfg(not(feature = "ipc"))]
impl MainThreadWakerImpl {
    fn new(waker: Box<dyn MainThreadWaker>) -> Result<MainThreadWakerImpl, Error> {
        Ok(MainThreadWakerImpl { waker })
    }

    pub fn wake(&self) {
        self.waker.wake()
    }
}

impl Registry {
    pub fn supports_session(&mut self, mode: SessionMode, dest: Sender<Result<(), Error>>) {
        let _ = self.sender.send(RegistryMsg::SupportsSession(mode, dest));
        self.waker.wake();
    }

    pub fn request_session(
        &mut self,
        mode: SessionMode,
        init: SessionInit,
        dest: Sender<Result<Session, Error>>,
        animation_frame_handler: Sender<Frame>,
    ) {
        let _ = self.sender.send(RegistryMsg::RequestSession(
            mode,
            init,
            dest,
            animation_frame_handler,
        ));
        self.waker.wake();
    }

    pub fn simulate_device_connection(
        &mut self,
        init: MockDeviceInit,
        dest: Sender<Result<Sender<MockDeviceMsg>, Error>>,
    ) {
        let _ = self
            .sender
            .send(RegistryMsg::SimulateDeviceConnection(init, dest));
        self.waker.wake();
    }
}

impl<SwapChains> MainThreadRegistry<SwapChains>
where
    SwapChains: SwapChainsAPI<SwapChainId>,
{
    pub fn new(waker: Box<dyn MainThreadWaker>) -> Result<Self, Error> {
        let (sender, receiver) = crate::channel().or(Err(Error::CommunicationError))?;
        let discoveries = Vec::new();
        let sessions = Vec::new();
        let mocks = Vec::new();
        let swap_chains = None;
        let waker = MainThreadWakerImpl::new(waker)?;
        Ok(MainThreadRegistry {
            discoveries,
            sessions,
            mocks,
            swap_chains,
            sender,
            receiver,
            waker,
            next_session_id: 0,
        })
    }

    pub fn registry(&self) -> Registry {
        Registry {
            sender: self.sender.clone(),
            waker: self.waker.clone(),
        }
    }

    pub fn register<D: DiscoveryAPI<SwapChains>>(&mut self, discovery: D) {
        self.discoveries.push(Box::new(discovery));
    }

    pub fn register_mock<D: MockDiscoveryAPI<SwapChains>>(&mut self, discovery: D) {
        self.mocks.push(Box::new(discovery));
    }

    pub fn run_on_main_thread<S: MainThreadSession>(&mut self, session: S) {
        self.sessions.push(Box::new(session));
    }

    pub fn run_one_frame(&mut self) {
        while let Ok(msg) = self.receiver.try_recv() {
            self.handle_msg(msg);
        }
        for session in &mut self.sessions {
            session.run_one_frame();
        }
        self.sessions.retain(|session| session.running());
    }

    pub fn running(&self) -> bool {
        self.sessions.iter().any(|session| session.running())
    }

    fn handle_msg(&mut self, msg: RegistryMsg) {
        match msg {
            RegistryMsg::SupportsSession(mode, dest) => {
                let _ = dest.send(self.supports_session(mode));
            }
            RegistryMsg::RequestSession(mode, init, dest, raf_sender) => {
                let _ = dest.send(self.request_session(mode, init, raf_sender));
            }
            RegistryMsg::SimulateDeviceConnection(init, dest) => {
                let _ = dest.send(self.simulate_device_connection(init));
            }
        }
    }

    pub fn set_swap_chains(&mut self, swap_chains: SwapChains) {
        self.swap_chains = Some(swap_chains);
    }

    fn supports_session(&mut self, mode: SessionMode) -> Result<(), Error> {
        for discovery in &self.discoveries {
            if discovery.supports_session(mode) {
                return Ok(());
            }
        }
        Err(Error::NoMatchingDevice)
    }

    fn request_session(
        &mut self,
        mode: SessionMode,
        init: SessionInit,
        raf_sender: Sender<Frame>,
    ) -> Result<Session, Error> {
        let swap_chains = self.swap_chains.as_mut().ok_or(Error::NoMatchingDevice)?;
        for discovery in &mut self.discoveries {
            if discovery.supports_session(mode) {
                let id = SessionId(self.next_session_id);
                self.next_session_id += 1;
                let xr =
                    SessionBuilder::new(swap_chains, &mut self.sessions, raf_sender.clone(), id);
                match discovery.request_session(mode, &init, xr) {
                    Ok(session) => return Ok(session),
                    Err(err) => warn!("XR device error {:?}", err),
                }
            }
        }
        Err(Error::NoMatchingDevice)
    }

    fn simulate_device_connection(
        &mut self,
        init: MockDeviceInit,
    ) -> Result<Sender<MockDeviceMsg>, Error> {
        for mock in &mut self.mocks {
            let (sender, receiver) = crate::channel().or(Err(Error::CommunicationError))?;
            if let Ok(discovery) = mock.simulate_device_connection(init.clone(), receiver) {
                self.discoveries.insert(0, discovery);
                return Ok(sender);
            }
        }
        Err(Error::NoMatchingDevice)
    }
}

#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
enum RegistryMsg {
    RequestSession(
        SessionMode,
        SessionInit,
        Sender<Result<Session, Error>>,
        Sender<Frame>,
    ),
    SupportsSession(SessionMode, Sender<Result<(), Error>>),
    SimulateDeviceConnection(MockDeviceInit, Sender<Result<Sender<MockDeviceMsg>, Error>>),
}
