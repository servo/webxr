/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::Floor;
use crate::InputFrame;
use crate::Native;
use crate::Viewer;
use crate::Views;

use euclid::RigidTransform3D;

/// The per-frame data that is provided by the device.
/// https://www.w3.org/TR/webxr/#xrframe
// TODO: other fields?
#[derive(Clone, Debug)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
pub struct Frame {
    /// The transform from the viewer to native coordinates
    ///
    /// This is equivalent to the pose of the viewer in native coordinates.
    /// This is the inverse of the view matrix.
    pub transform: Option<RigidTransform3D<f32, Viewer, Native>>,

    /// Frame information for each connected input source
    pub inputs: Vec<InputFrame>,

    /// Events that occur with the frame.
    pub events: Vec<FrameUpdateEvent>,

    /// Value of time::precise_time_ns() when frame was obtained
    pub time_ns: u64,
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
pub enum FrameUpdateEvent {
    UpdateViews(Views),
    UpdateFloorTransform(Option<RigidTransform3D<f32, Native, Floor>>),
}
