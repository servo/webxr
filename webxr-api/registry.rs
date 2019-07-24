/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::Discovery;
use crate::Error;
use crate::MainThreadSession;
use crate::MockDeviceInit;
use crate::MockDeviceMsg;
use crate::MockDiscovery;
use crate::Receiver;
use crate::Sender;
use crate::Session;
use crate::SessionBuilder;
use crate::SessionMode;

#[cfg(feature = "ipc")]
use serde::{Deserialize, Serialize};

#[derive(Clone)]
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
pub struct Registry {
    sender: Sender<RegistryMsg>,
    waker: MainThreadWakerImpl,
}

pub struct MainThreadRegistry {
    discoveries: Vec<Box<dyn Discovery>>,
    sessions: Vec<Box<dyn MainThreadSession>>,
    mocks: Vec<Box<dyn MockDiscovery>>,
    sender: Sender<RegistryMsg>,
    receiver: Receiver<RegistryMsg>,
    waker: MainThreadWakerImpl,
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

#[cfg_attr(feature = "ipc", typetag::serde)]
pub trait SessionSupportCallback: 'static + Send {
    fn callback(&mut self, result: Result<(), Error>);
}

#[cfg_attr(feature = "ipc", typetag::serde)]
pub trait SessionRequestCallback: 'static + Send {
    fn callback(&mut self, result: Result<Session, Error>);
}

#[cfg_attr(feature = "ipc", typetag::serde)]
pub trait MockDeviceCallback: 'static + Send {
    fn callback(&mut self, result: Result<Sender<MockDeviceMsg>, Error>);
}

impl Registry {
    pub fn supports_session<C>(&mut self, mode: SessionMode, callback: C)
    where
        C: SessionSupportCallback,
    {
        let _ = self
            .sender
            .send(RegistryMsg::SupportsSession(mode, Box::new(callback)));
        self.waker.wake();
    }

    pub fn request_session<C>(&mut self, mode: SessionMode, callback: C)
    where
        C: SessionRequestCallback,
    {
        let _ = self
            .sender
            .send(RegistryMsg::RequestSession(mode, Box::new(callback)));
        self.waker.wake();
    }

    pub fn simulate_device_connection<C>(&mut self, init: MockDeviceInit, callback: C)
    where
        C: MockDeviceCallback,
    {
        let _ = self.sender.send(RegistryMsg::SimulateDeviceConnection(
            init,
            Box::new(callback),
        ));
        self.waker.wake();
    }
}

impl MainThreadRegistry {
    pub fn new(waker: Box<dyn MainThreadWaker>) -> Result<MainThreadRegistry, Error> {
        let (sender, receiver) = crate::channel().or(Err(Error::CommunicationError))?;
        let discoveries = Vec::new();
        let sessions = Vec::new();
        let mocks = Vec::new();
        let waker = MainThreadWakerImpl::new(waker)?;
        Ok(MainThreadRegistry {
            discoveries,
            sessions,
            mocks,
            sender,
            receiver,
            waker,
        })
    }

    pub fn registry(&self) -> Registry {
        Registry {
            sender: self.sender.clone(),
            waker: self.waker.clone(),
        }
    }

    pub fn register<D: Discovery>(&mut self, discovery: D) {
        self.discoveries.push(Box::new(discovery));
    }

    pub fn register_mock<D: MockDiscovery>(&mut self, discovery: D) {
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
            RegistryMsg::SupportsSession(mode, mut callback) => {
                callback.callback(self.supports_session(mode));
            }
            RegistryMsg::RequestSession(mode, mut callback) => {
                callback.callback(self.request_session(mode));
            }
            RegistryMsg::SimulateDeviceConnection(init, mut callback) => {
                callback.callback(self.simulate_device_connection(init));
            }
        }
    }

    fn supports_session(&mut self, mode: SessionMode) -> Result<(), Error> {
        for discovery in &self.discoveries {
            if discovery.supports_session(mode) {
                return Ok(());
            }
        }
        Err(Error::NoMatchingDevice)
    }

    fn request_session(&mut self, mode: SessionMode) -> Result<Session, Error> {
        for discovery in &mut self.discoveries {
            let xr = SessionBuilder::new(&mut self.sessions);
            if let Ok(session) = discovery.request_session(mode, xr) {
                return Ok(session);
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
    RequestSession(SessionMode, Box<dyn SessionRequestCallback>),
    SupportsSession(SessionMode, Box<dyn SessionSupportCallback>),
    SimulateDeviceConnection(MockDeviceInit, Box<dyn MockDeviceCallback>),
}
