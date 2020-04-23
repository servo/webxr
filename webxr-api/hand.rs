use crate::Native;
use euclid::RigidTransform3D;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
pub struct HandSpace;

#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
pub struct Hand {
    pub wrist: Option<Joint>,
    pub thumb_metacarpal: Option<Joint>,
    pub thumb_phalanx_proximal: Option<Joint>,
    pub phalanx_distal: Option<Joint>,
    pub phalanx_tip: Option<Joint>,
    pub index: Finger,
    pub middle: Finger,
    pub ring: Finger,
    pub little: Finger,
}

#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
pub struct Finger {
    pub metacarpal: Option<Joint>,
    pub phalanx_proximal: Option<Joint>,
    pub phalanx_intermediate: Option<Joint>,
    pub phalanx_distal: Option<Joint>,
    pub phalanx_tip: Option<Joint>,
}

#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
pub struct Joint {
    pub pose: RigidTransform3D<f32, HandSpace, Native>,
    pub radius: f32,
}
