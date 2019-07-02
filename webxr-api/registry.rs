/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::Discovery;
use crate::Error;
use crate::Receiver;
use crate::Sender;
use crate::Session;
use crate::SessionBuilder;
use crate::SessionMode;

use std::thread;

#[cfg(feature = "ipc")]
use serde::{Deserialize, Serialize};

#[derive(Clone)]
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
pub struct Registry {
    sender: Sender<RegistryMsg>,
}

pub struct RegistryThread {
    discoveries: Vec<Box<dyn Discovery>>,
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
    pub fn new() -> Result<Registry, Error> {
        let (sender, receiver) = crate::channel().or(Err(Error::CommunicationError))?;
        let discoveries = Vec::new();
        let mut thread = RegistryThread {
            receiver,
            discoveries,
        };
        let registry = Registry { sender };
        thread::spawn(move || thread.run());
        Ok(registry)
    }

    pub fn register<D: Discovery>(&mut self, discovery: D) {
        let _ = self.sender.send(RegistryMsg::Register(Box::new(discovery)));
    }

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

impl RegistryThread {
    fn run(&mut self) {
        while let Ok(msg) = self.receiver.recv() {
            match msg {
                RegistryMsg::Register(discovery) => {
                    self.discoveries.push(discovery);
                }
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
                        let xr = SessionBuilder::new();
                        if let Ok(session) = discovery.request_session(mode, xr) {
                            return callback.callback(Ok(session));
                        }
                    }
                    return callback.callback(Err(Error::NoMatchingDevice));
                }
            }
        }
    }
}

#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
enum RegistryMsg {
    Register(Box<dyn Discovery>),
    RequestSession(SessionMode, Box<dyn SessionRequestCallback>),
    SupportsSession(SessionMode, Box<dyn SessionSupportCallback>),
}
