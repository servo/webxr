/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! This crate defines the Rust API for WebXR. It is implemented by the `webxr` crate.

mod device;
mod error;
mod events;
mod frame;
mod input;
mod mock;
mod registry;
mod session;
mod view;
mod webgl;

pub use device::Device;
pub use device::Discovery;

pub use error::Error;

pub use events::Event;
pub use events::EventBuffer;
pub use events::EventCallback;

pub use frame::Frame;

pub use input::Handedness;
pub use input::InputFrame;
pub use input::InputId;
pub use input::InputSource;
pub use input::TargetRayMode;

pub use mock::MockDeviceInit;
pub use mock::MockDeviceMsg;
pub use mock::MockDiscovery;

pub use registry::MainThreadRegistry;
pub use registry::Registry;
pub use registry::{MockDeviceCallback, SessionRequestCallback, SessionSupportCallback};

pub use session::FrameRequestCallback;
pub use session::HighResTimeStamp;
pub use session::MainThreadSession;
pub use session::Session;
pub use session::SessionBuilder;
pub use session::SessionMode;
pub use session::SessionThread;

pub use view::Display;
pub use view::Floor;
pub use view::Input;
pub use view::LeftEye;
pub use view::Native;
pub use view::RightEye;
pub use view::View;
pub use view::Viewer;
pub use view::Viewport;
pub use view::Views;

pub use webgl::WebGLExternalImageApi;

#[cfg(feature = "ipc")]
use std::thread;

use std::time::Duration;

#[cfg(feature = "ipc")]
pub use ipc_channel::ipc::IpcSender as Sender;

#[cfg(feature = "ipc")]
pub use ipc_channel::ipc::IpcReceiver as Receiver;

#[cfg(feature = "ipc")]
pub use ipc_channel::ipc::channel;

#[cfg(not(feature = "ipc"))]
pub use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};

#[cfg(not(feature = "ipc"))]
fn channel<T>() -> Result<(Sender<T>, Receiver<T>), ()> {
    Ok(std::sync::mpsc::channel())
}

#[cfg(not(feature = "ipc"))]
pub fn recv_timeout<T>(receiver: &Receiver<T>, timeout: Duration) -> Result<T, RecvTimeoutError> {
    receiver.recv_timeout(timeout)
}

#[cfg(feature = "ipc")]
pub fn recv_timeout<T>(receiver: &Receiver<T>, timeout: Duration) -> Result<T, ipc_channel::Error>
where
    T: serde::Serialize + for<'a> serde::Deserialize<'a>,
{
    // Sigh, polling, sigh.
    let mut delay = timeout / 1000;
    while delay < timeout {
        if let Ok(msg) = receiver.try_recv() {
            return Ok(msg);
        }
        thread::sleep(delay);
        delay = delay * 2;
    }
    receiver.try_recv()
}
