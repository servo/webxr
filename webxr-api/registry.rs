/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::Discovery;
use crate::Error;
use crate::MainThreadSession;
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
}

pub struct MainThreadRegistry {
    discoveries: Vec<Box<dyn Discovery>>,
    sessions: Vec<Box<dyn MainThreadSession>>,
    sender: Sender<RegistryMsg>,
    receiver: Receiver<RegistryMsg>,
}

#[cfg_attr(feature = "ipc", typetag::serde)]
pub trait SessionSupportCallback: 'static + Send {
    fn callback(&mut self, result: Result<(), Error>);
}

#[cfg_attr(feature = "ipc", typetag::serde)]
pub trait SessionRequestCallback: 'static + Send {
    fn callback(&mut self, result: Result<Session, Error>);
}

impl Registry {
    pub fn supports_session<C>(&mut self, mode: SessionMode, callback: C)
    where
        C: SessionSupportCallback,
    {
        let _ = self
            .sender
            .send(RegistryMsg::SupportsSession(mode, Box::new(callback)));
    }

    pub fn request_session<C>(&mut self, mode: SessionMode, callback: C)
    where
        C: SessionRequestCallback,
    {
        let _ = self
            .sender
            .send(RegistryMsg::RequestSession(mode, Box::new(callback)));
    }
}

impl MainThreadRegistry {
    pub fn new() -> Result<MainThreadRegistry, Error> {
        let (sender, receiver) = crate::channel().or(Err(Error::CommunicationError))?;
        let discoveries = Vec::new();
        let sessions = Vec::new();
        Ok(MainThreadRegistry {
            discoveries,
            sessions,
            sender,
            receiver,
        })
    }

    pub fn registry(&self) -> Registry {
        Registry {
            sender: self.sender.clone(),
        }
    }

    pub fn register<D: Discovery>(&mut self, discovery: D) {
        self.discoveries.push(Box::new(discovery));
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
                for discovery in &self.discoveries {
                    if discovery.supports_session(mode) {
                        return callback.callback(Ok(()));
                    }
                }
                return callback.callback(Err(Error::NoMatchingDevice));
            }
            RegistryMsg::RequestSession(mode, mut callback) => {
                for discovery in &mut self.discoveries {
                    let xr = SessionBuilder::new(&mut self.sessions);
                    if let Ok(session) = discovery.request_session(mode, xr) {
                        return callback.callback(Ok(session));
                    }
                }
                return callback.callback(Err(Error::NoMatchingDevice));
            }
        }
    }
}

#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
enum RegistryMsg {
    RequestSession(SessionMode, Box<dyn SessionRequestCallback>),
    SupportsSession(SessionMode, Box<dyn SessionSupportCallback>),
}
