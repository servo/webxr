/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! This crate uses `euclid`'s typed units, and exposes different coordinate spaces.

use euclid::TypedRigidTransform3D;
use euclid::TypedTransform3D;

#[cfg(feature = "ipc")]
use serde::{Deserialize, Serialize};

/// The coordinate space of the viewer
/// https://immersive-web.github.io/webxr/#dom-xrreferencespacetype-viewer
#[derive(Clone, Debug)]
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
pub enum Viewer {}

/// The coordinate space of the floor
/// https://immersive-web.github.io/webxr/#dom-xrreferencespacetype-local-floor
#[derive(Clone, Debug)]
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
pub enum Floor {}

/// The coordinate space of the left eye
/// https://immersive-web.github.io/webxr/#dom-xreye-left
#[derive(Clone, Debug)]
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
pub enum LeftEye {}

/// The coordinate space of the right eye
/// https://immersive-web.github.io/webxr/#dom-xreye-right
#[derive(Clone, Debug)]
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
pub enum RightEye {}

/// The native 3D coordinate space of the device
/// This is not part of the webvr specification.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
pub enum Native {}

/// The 2D coordinate space of a device display
/// This is not part of the webvr specification.
// TODO: are we OK assuming that we can use the same coordinate system for all displays?
#[derive(Clone, Debug)]
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
pub enum Display {}

/// For each eye, the transform from the viewer to that eye,
/// and its projection onto its display.
/// For stereo displays, we have a `View<LeftEye>` and a `View<RightEye>`.
/// For mono displays, we hagve a `View<Viewer>` (where the transform is the identity).
/// https://immersive-web.github.io/webxr/#xrview
#[derive(Clone, Debug)]
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
pub struct View<Eye> {
    pub transform: TypedRigidTransform3D<f32, Viewer, Eye>,
    pub projection: TypedTransform3D<f32, Eye, Display>,
}

/// Whether a device is mono or stereo, and the views it supports.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "ipc", derive(Serialize, Deserialize))]
pub enum Views {
    Mono(View<Viewer>),
    Stereo(View<LeftEye>, View<RightEye>),
}
