/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! This crate defines the Rust implementation of WebXR for various devices.

#[cfg(feature = "glwindow")]
pub mod glwindow;

#[cfg(feature = "headless")]
pub mod headless;

#[cfg(feature = "googlevr")]
pub mod googlevr;

#[cfg(feature = "gles")]
mod gles;

#[cfg(all(feature = "googlevr", target_os = "android"))]
pub(crate) mod jni_utils;

#[cfg(feature = "magicleap")]
pub mod magicleap;

#[cfg(feature = "egl")]
mod egl;

#[cfg(feature = "openxr-api")]
pub mod openxr;

pub(crate) mod utils;

pub struct Surface(pub surfman::platform::generic::universal::surface::Surface);

impl webxr_api::Surface for Surface {}

impl std::ops::Deref for Surface {
    type Target = surfman::platform::generic::universal::surface::Surface;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct SwapChain(surfman_chains::SwapChain);

impl webxr_api::SwapChain for SwapChain {
    type Surface = Surface;

    fn take_surface(&self) -> Option<Self::Surface> {
        self.0.take_surface().map(Surface)
    }
    fn recycle_surface(&self, surface: Self::Surface) {
        self.0.recycle_surface(surface.0)
    }
}

#[derive(Clone)]
pub struct SwapChains(pub surfman_chains::SwapChains<webxr_api::SwapChainId>);

impl webxr_api::SwapChains for SwapChains {
    type SwapChain = SwapChain;
    type Surface = Surface;

    fn get(&self, id: webxr_api::SwapChainId) -> Option<Self::SwapChain> {
        self.0.get(id).map(SwapChain)
    }
}
