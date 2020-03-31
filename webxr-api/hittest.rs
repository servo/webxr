use crate::ApiSpace;
use crate::Space;
use euclid::Vector3D;
use std::iter::FromIterator;

#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
/// https://immersive-web.github.io/hit-test/#xrray
pub struct Ray {
    pub origin: Vector3D<f32, ApiSpace>,
    pub direction: Vector3D<f32, ApiSpace>,
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
/// https://immersive-web.github.io/hit-test/#enumdef-xrhittesttrackabletype
pub enum EntityType {
    Point,
    Plane,
    Mesh,
}

#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
/// https://immersive-web.github.io/hit-test/#dictdef-xrhittestoptionsinit
pub struct HitTestSource {
    pub space: Space,
    pub ray: Ray,
    pub types: EntityTypes,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
pub struct HitTestId(pub u32);

#[derive(Copy, Clone, Debug, Default)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
/// Vec<EntityType>, but better
pub struct EntityTypes {
    pub point: bool,
    pub plane: bool,
    pub mesh: bool,
}

impl EntityTypes {
    pub fn is_type(self, ty: EntityType) -> bool {
        match ty {
            EntityType::Point => self.point,
            EntityType::Plane => self.plane,
            EntityType::Mesh => self.mesh,
        }
    }

    pub fn add_type(&mut self, ty: EntityType) {
        match ty {
            EntityType::Point => self.point = true,
            EntityType::Plane => self.plane = true,
            EntityType::Mesh => self.mesh = true,
        }
    }
}

impl FromIterator<EntityType> for EntityTypes {
    fn from_iter<T>(iter: T) -> Self
    where
        T: IntoIterator<Item = EntityType>,
    {
        iter.into_iter().fold(Default::default(), |mut acc, e| {
            acc.add_type(e);
            acc
        })
    }
}
