/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::Input;
use crate::Native;

use euclid::RigidTransform3D;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
pub struct InputId(pub u32);

#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
pub enum Handedness {
    None,
    Left,
    Right,
}

#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
pub enum TargetRayMode {
    Gaze,
    TrackedPointer,
    Screen,
}

#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
pub struct InputSource {
    pub handedness: Handedness,
    pub target_ray_mode: TargetRayMode,
    pub id: InputId,
}

#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
pub struct InputFrame {
    pub id: InputId,
    pub target_ray_origin: Option<RigidTransform3D<f32, Input, Native>>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
pub enum SelectEvent {
    /// Selection started
    Start,
    /// Selection ended *without* it being a contiguous select event
    End,
    /// Selection ended *with* it being a contiguous select event
    Select,
}
