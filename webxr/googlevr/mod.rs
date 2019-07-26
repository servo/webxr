/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

pub use discovery::GoogleVRDiscovery;

pub(crate) mod device;
pub(crate) mod discovery;
pub(crate) mod input;

// Export functions called from Java
#[cfg(target_os = "android")]
pub mod jni {
    pub use super::discovery::Java_com_rust_webvr_GVRService_nativeOnPause;
    pub use super::discovery::Java_com_rust_webvr_GVRService_nativeOnResume;
}
