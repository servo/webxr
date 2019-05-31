/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! The WebGL functionality needed by WebXR.

use euclid::Size2D;
use gleam::gl::Gl;
use std::rc::Rc;

/// An identifier for a WebGL context.
// TODO: refactor the Servo webgl_traits crate to support sharing this type.
#[derive(Copy, Clone, Debug)]
pub struct WebGLContextId(pub usize);

/// A type to get access to a GL texture from a WebGL context.
// TODO: refactor the Servo canvas crate to support sharing this type.
#[derive(Clone)]
pub struct WebGLExternalImages(());

/// https://github.com/servo/servo/blob/123f58592c9bedae735a84fe5c93b0a20292ea86/components/canvas/webgl_thread.rs#L733-L742
impl WebGLExternalImages {
    // TODO: implement this
    pub fn lock(&mut self, _: WebGLContextId) -> (u32, Size2D<i32>) {
        unimplemented!();
    }

    // TODO: implement this
    pub fn unlock(&mut self, _: WebGLContextId) {
        unimplemented!()
    }
}

/// A factory for building GL objects.
// TODO refactor the Servo canvas crate to support sharing this type
#[derive(Clone)]
pub struct GLFactory(());

impl GLFactory {
    pub fn build(&mut self) -> Rc<Gl> {
        unimplemented!()
    }
}
