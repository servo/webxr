/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! The WebGL functionality needed by WebXR.

use gleam::gl::GLsync;
use gleam::gl::GLuint;

pub type WebGLContextId = usize;
pub type WebGLTextureId = GLuint;

/// A trait to get access a GL texture from a WebGL context.
// Note that this is not serializable, we run it in the same
// process as the XR sessions. This is important for safety,
// since we are sending GL sync objects. It does implement Send
// though, which is the main difference between this trait and
// the matching webrender trait.
pub trait WebGLExternalImageApi: Send {
    /// Lock the WebGL context, and get back a sync object for its current state.
    fn lock(&self, id: WebGLContextId) -> GLsync;

    /// Unlock the WebGL context.
    fn unlock(&self, id: WebGLContextId);

    /// Workaround for Clone not being object-safe
    fn clone_box(&self) -> Box<dyn WebGLExternalImageApi>;
}
