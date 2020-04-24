use crate::Native;
use euclid::RigidTransform3D;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
pub struct HandSpace;

#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
pub struct Hand<J> {
    pub wrist: Option<J>,
    pub thumb_metacarpal: Option<J>,
    pub thumb_phalanx_proximal: Option<J>,
    pub thumb_phalanx_distal: Option<J>,
    pub thumb_phalanx_tip: Option<J>,
    pub index: Finger<J>,
    pub middle: Finger<J>,
    pub ring: Finger<J>,
    pub little: Finger<J>,
}

#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
pub struct Finger<J> {
    pub metacarpal: Option<J>,
    pub phalanx_proximal: Option<J>,
    pub phalanx_intermediate: Option<J>,
    pub phalanx_distal: Option<J>,
    pub phalanx_tip: Option<J>,
}

#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
pub struct Joint {
    pub pose: RigidTransform3D<f32, HandSpace, Native>,
    pub radius: f32,
}

impl Default for Joint {
    fn default() -> Self {
        Self {
            pose: RigidTransform3D::identity(),
            radius: 0.,
        }
    }
}

impl<J> Hand<J> {
    pub fn map<R>(&self, mut map: impl (FnMut(&Option<J>) -> Option<R>) + Copy) -> Hand<R> {
        Hand {
            wrist: map(&self.wrist),
            thumb_metacarpal: map(&self.thumb_metacarpal),
            thumb_phalanx_proximal: map(&self.thumb_phalanx_proximal),
            thumb_phalanx_distal: map(&self.thumb_phalanx_distal),
            thumb_phalanx_tip: map(&self.thumb_phalanx_tip),
            index: self.index.map(map),
            middle: self.middle.map(map),
            ring: self.ring.map(map),
            little: self.little.map(map),
        }
    }
}

impl<J> Finger<J> {
    pub fn map<R>(&self, mut map: impl (FnMut(&Option<J>) -> Option<R>) + Copy) -> Finger<R> {
        Finger {
            metacarpal: map(&self.metacarpal),
            phalanx_proximal: map(&self.phalanx_proximal),
            phalanx_intermediate: map(&self.phalanx_intermediate),
            phalanx_distal: map(&self.phalanx_distal),
            phalanx_tip: map(&self.phalanx_tip),
        }
    }
}
