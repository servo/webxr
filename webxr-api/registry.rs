/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::Discovery;
use crate::Error;
use crate::Session;
use crate::SessionBuilder;
use crate::SessionMode;

use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::thread;

#[derive(Clone)]
pub struct Registry {
    sender: Sender<RegistryMsg>,
}

pub struct RegistryThread {
    discoveries: Vec<Box<dyn Discovery>>,
    receiver: Receiver<RegistryMsg>,
}

pub type SessionSupportCallback = Box<dyn 'static + FnOnce(Result<(), Error>) + Send>;

pub type SessionRequestCallback = Box<dyn 'static + FnOnce(Result<Session, Error>) + Send>;

impl Registry {
    pub fn new() -> Registry {
        let (sender, receiver) = mpsc::channel();
        let discoveries = Vec::new();
        let mut thread = RegistryThread {
            receiver,
            discoveries,
        };
        let registry = Registry { sender };
        thread::spawn(move || thread.run());
        registry
    }

    pub fn register<D: Discovery>(&mut self, discovery: D) {
        let _ = self.sender.send(RegistryMsg::Register(Box::new(discovery)));
    }

    pub fn supports_session(&mut self, mode: SessionMode, callback: SessionSupportCallback) {
        let _ = self
            .sender
            .send(RegistryMsg::SupportsSession(mode, callback));
    }

    pub fn request_session(&mut self, mode: SessionMode, callback: SessionRequestCallback) {
        let _ = self
            .sender
            .send(RegistryMsg::RequestSession(mode, callback));
    }
}

impl RegistryThread {
    fn run(&mut self) {
        while let Ok(msg) = self.receiver.recv() {
            match msg {
                RegistryMsg::Register(discovery) => {
                    self.discoveries.push(discovery);
                }
                RegistryMsg::SupportsSession(mode, callback) => {
                    for discovery in &self.discoveries {
                        if discovery.supports_session(mode) {
                            return callback(Ok(()));
                        }
                    }
                    return callback(Err(Error::NoMatchingDevice));
                }
                RegistryMsg::RequestSession(mode, callback) => {
                    for discovery in &mut self.discoveries {
                        let xr = SessionBuilder::new();
                        if let Ok(session) = discovery.request_session(mode, xr) {
                            return callback(Ok(session));
                        }
                    }
                    return callback(Err(Error::NoMatchingDevice));
                }
            }
        }
    }
}

enum RegistryMsg {
    Register(Box<dyn Discovery>),
    RequestSession(SessionMode, SessionRequestCallback),
    SupportsSession(SessionMode, SessionSupportCallback),
}
