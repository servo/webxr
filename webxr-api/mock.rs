/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::DiscoveryAPI;
use crate::Error;
use crate::Floor;
use crate::Handedness;
use crate::Input;
use crate::InputId;
use crate::InputSource;
use crate::Native;
use crate::Receiver;
use crate::Sender;
use crate::TargetRayMode;
use crate::Viewer;
use crate::Views;

use euclid::RigidTransform3D;

#[cfg(feature = "ipc")]
use serde::{Deserialize, Serialize};

/// A trait for discovering mock XR devices
pub trait MockDiscoveryAPI<SwapChains>: 'static {
    fn simulate_device_connection(
        &mut self,
        init: MockDeviceInit,
        receiver: Receiver<MockDeviceMsg>,
    ) -> Result<Box<dyn DiscoveryAPI<SwapChains>>, Error>;
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
pub struct MockDeviceInit {
    pub floor_origin: Option<RigidTransform3D<f32, Floor, Native>>,
    pub supports_immersive: bool,
    pub supports_unbounded: bool,
    pub viewer_origin: Option<RigidTransform3D<f32, Viewer, Native>>,
    pub views: Views,
}

#[derive(Debug)]
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
pub enum MockDeviceMsg {
    SetViewerOrigin(Option<RigidTransform3D<f32, Viewer, Native>>),
    SetFloorOrigin(Option<RigidTransform3D<f32, Floor, Native>>),
    SetViews(Views),
    AddInputSource(MockInputInit),
    MessageInputSource(InputId, MockInputMsg),
    Focus,
    Blur,
    Disconnect(Sender<()>),
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
pub struct MockInputInit {
    pub source: InputSource,
    pub pointer_origin: Option<RigidTransform3D<f32, Input, Native>>,
    pub grip_origin: Option<RigidTransform3D<f32, Input, Native>>,
}

#[derive(Debug)]
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
pub enum MockInputMsg {
    SetHandedness(Handedness),
    SetTargetRayMode(TargetRayMode),
    SetPointerOrigin(Option<RigidTransform3D<f32, Input, Native>>),
    SetGripOrigin(Option<RigidTransform3D<f32, Input, Native>>),
    Disconnect,
    Reconnect,
}
