/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::Native;
use crate::Viewer;

use euclid::TypedRigidTransform3D;

/// The per-frame data that is provided by the device.
/// https://www.w3.org/TR/webxr/#xrframe
// TODO: other fields?
pub struct Frame {
    /// The transform from the viewer to native coordinates
    pub transform: TypedRigidTransform3D<f32, Viewer, Native>,
}
