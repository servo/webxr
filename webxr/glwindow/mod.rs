/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use euclid::Size2D;
use euclid::TypedRigidTransform3D;
use euclid::TypedTransform3D;
use euclid::TypedVector3D;

use gleam::gl;
use gleam::gl::GLsizei;
use gleam::gl::Gl;

use glutin::dpi::PhysicalSize;
use glutin::PossiblyCurrent;
use glutin::WindowedContext;

use std::rc::Rc;

use webxr_api::Device;
use webxr_api::Discovery;
use webxr_api::Error;
use webxr_api::Floor;
use webxr_api::Frame;
use webxr_api::Native;
use webxr_api::Session;
use webxr_api::SessionBuilder;
use webxr_api::SessionMode;
use webxr_api::SessionThread;
use webxr_api::View;
use webxr_api::Views;

const HEIGHT: f32 = 1.0;
const EYE_DISTANCE: f32 = 0.25;
const NEAR: f32 = 0.1;
const FAR: f32 = 100.0;

pub struct GlWindowDiscovery {
    thread: Option<SessionThread<GlWindowDevice>>,
}

impl Discovery for GlWindowDiscovery {
    fn request_session(
        &mut self,
        mode: SessionMode,
        mut xr: SessionBuilder,
    ) -> Result<Session, Error> {
        if !self.supports_session(mode) {
            return Err(Error::NoMatchingDevice);
        }
        let gl = xr.gl();
        let device = GlWindowDevice::new(gl);
        let mut thread = xr.new_thread(device);
        let session = thread.new_session();
        self.thread = Some(thread);
        Ok(session)
    }

    fn supports_session(&self, mode: SessionMode) -> bool {
        mode == SessionMode::ImmersiveVR && self.thread.is_none()
    }
}

pub struct GlWindowDevice {
    size: PhysicalSize,
    gl_context: WindowedContext<PossiblyCurrent>,
    gl: Rc<Gl>,
}

impl Device for GlWindowDevice {
    fn floor_transform(&self) -> TypedRigidTransform3D<f32, Native, Floor> {
        let translation = TypedVector3D::new(-HEIGHT, 0.0, 0.0);
        TypedRigidTransform3D::from_translation(translation)
    }

    fn views(&self) -> Views {
        let left = self.view(-EYE_DISTANCE);
        let right = self.view(EYE_DISTANCE);
        Views::Stereo(left, right)
    }

    fn wait_for_animation_frame(&mut self) -> Frame {
        let _ = self.gl_context.swap_buffers();
        let transform = TypedRigidTransform3D::identity();
        Frame { transform }
    }

    fn render_animation_frame(&mut self, texture_id: u32, size: Size2D<i32>) {
        let width = size.width as GLsizei;
        let height = size.height as GLsizei;
        self.gl.clear_color(0.2, 0.3, 0.3, 1.0);
        self.gl.clear(gl::COLOR_BUFFER_BIT);
        self.gl.framebuffer_texture_2d(
            gl::READ_FRAMEBUFFER,
            gl::COLOR_ATTACHMENT0,
            gl::TEXTURE_2D,
            texture_id,
            0,
        );
        self.gl.viewport(0, 0, width, height);
        self.gl.blit_framebuffer(
            0,
            0,
            width,
            height,
            0,
            0,
            width,
            height,
            gl::COLOR_BUFFER_BIT,
            gl::NEAREST,
        );
    }
}

impl GlWindowDevice {
    fn new(_gl: Rc<Gl>) -> GlWindowDevice {
        unimplemented!()
    }

    fn view<Eye>(&self, offset: f32) -> View<Eye> {
        let width = self.size.width as f32;
        let height = self.size.height as f32;
        let projection = TypedTransform3D::ortho(0.0, width, 0.0, height, NEAR, FAR);
        let translation = TypedVector3D::new(offset, 0.0, 0.0);
        let transform = TypedRigidTransform3D::from_translation(translation);
        View {
            transform,
            projection,
        }
    }
}
