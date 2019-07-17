/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use euclid::Angle;
use euclid::Size2D;
use euclid::Trig;
use euclid::TypedPoint2D;
use euclid::TypedRect;
use euclid::TypedRigidTransform3D;
use euclid::TypedSize2D;
use euclid::TypedTransform3D;
use euclid::TypedVector3D;

use gleam::gl;
use gleam::gl::GLsizei;
use gleam::gl::GLsync;
use gleam::gl::GLuint;
use gleam::gl::Gl;

use glutin::EventsLoop;
use glutin::EventsLoopClosed;

use std::rc::Rc;

use webxr_api::Device;
use webxr_api::Discovery;
use webxr_api::Display;
use webxr_api::Error;
use webxr_api::Floor;
use webxr_api::Frame;
use webxr_api::InputSource;
use webxr_api::Native;
use webxr_api::Session;
use webxr_api::SessionBuilder;
use webxr_api::SessionMode;
use webxr_api::View;
use webxr_api::Views;

const HEIGHT: f32 = 1.0;
const EYE_DISTANCE: f32 = 0.25;
const NEAR: f32 = 0.1;
const FAR: f32 = 100.0;

pub trait GlWindow {
    fn make_current(&mut self);
    fn swap_buffers(&mut self);
    fn size(&self) -> Size2D<GLsizei>;
    fn new_window(&self) -> Result<Box<dyn GlWindow>, ()>;
}

pub struct GlWindowDiscovery {
    gl: Rc<dyn Gl>,
    factory: Box<dyn Fn() -> Result<Box<dyn GlWindow>, ()>>,
}

impl GlWindowDiscovery {
    pub fn new(
        gl: Rc<dyn Gl>,
        factory: Box<dyn Fn() -> Result<Box<dyn GlWindow>, ()>>,
    ) -> GlWindowDiscovery {
        GlWindowDiscovery { gl, factory }
    }
}

impl Discovery for GlWindowDiscovery {
    fn request_session(&mut self, mode: SessionMode, xr: SessionBuilder) -> Result<Session, Error> {
        if self.supports_session(mode) {
            let gl = self.gl.clone();
            let window = (self.factory)().or(Err(Error::NoMatchingDevice))?;
            xr.run_on_main_thread(move || GlWindowDevice::new(gl, window))
        } else {
            Err(Error::NoMatchingDevice)
        }
    }

    fn supports_session(&self, mode: SessionMode) -> bool {
        mode == SessionMode::ImmersiveVR
    }
}

pub struct GlWindowDevice {
    gl: Rc<dyn Gl>,
    window: Box<dyn GlWindow>,
    read_fbo: GLuint,
}

impl Device for GlWindowDevice {
    fn floor_transform(&self) -> TypedRigidTransform3D<f32, Native, Floor> {
        let translation = TypedVector3D::new(-HEIGHT, 0.0, 0.0);
        TypedRigidTransform3D::from_translation(translation)
    }

    fn views(&self) -> Views {
        let left = self.view(false);
        let right = self.view(true);
        Views::Stereo(left, right)
    }

    fn wait_for_animation_frame(&mut self) -> Frame {
        self.window.swap_buffers();
        let translation = TypedVector3D::new(0.0, 0.0, -5.0);
        let transform = TypedRigidTransform3D::from_translation(translation);
        Frame {
            transform,
            inputs: vec![],
        }
    }

    fn render_animation_frame(&mut self, texture_id: u32, size: Size2D<i32>, sync: GLsync) {
        self.window.make_current();

        let width = size.width as GLsizei;
        let height = size.height as GLsizei;
        let inner_size = self.window.size();

        self.gl.clear_color(0.2, 0.3, 0.3, 1.0);
        self.gl.clear(gl::COLOR_BUFFER_BIT);
        self.gl.wait_sync(sync, 0, gl::TIMEOUT_IGNORED);
        debug_assert_eq!(self.gl.get_error(), gl::NO_ERROR);

        self.gl
            .bind_framebuffer(gl::READ_FRAMEBUFFER, self.read_fbo);
        debug_assert_eq!(self.gl.get_error(), gl::NO_ERROR);

        self.gl.framebuffer_texture_2d(
            gl::READ_FRAMEBUFFER,
            gl::COLOR_ATTACHMENT0,
            gl::TEXTURE_2D,
            texture_id,
            0,
        );
        debug_assert_eq!(self.gl.get_error(), gl::NO_ERROR);

        self.gl.viewport(0, 0, width, height);
        self.gl.blit_framebuffer(
            0,
            0,
            width,
            height,
            0,
            0,
            inner_size.width,
            inner_size.height,
            gl::COLOR_BUFFER_BIT,
            gl::NEAREST,
        );
        debug_assert_eq!(self.gl.get_error(), gl::NO_ERROR);
    }

    fn initial_inputs(&self) -> Vec<InputSource> {
        vec![]
    }
}

impl GlWindowDevice {
    fn new(gl: Rc<dyn Gl>, mut window: Box<dyn GlWindow>) -> Result<GlWindowDevice, Error> {
        window.make_current();
        let read_fbo = gl.gen_framebuffers(1)[0];
        debug_assert_eq!(gl.get_error(), gl::NO_ERROR);

        Ok(GlWindowDevice {
            gl,
            window,
            read_fbo,
        })
    }

    fn view<Eye>(&self, is_right: bool) -> View<Eye> {
        let window_size = self.window.size();
        let viewport_size = TypedSize2D::new(window_size.width / 2, window_size.height);
        let viewport_x_origin = if is_right { viewport_size.width } else { 0 };
        let viewport_origin = TypedPoint2D::new(viewport_x_origin, 0);
        let viewport = TypedRect::new(viewport_origin, viewport_size);
        let projection = self.perspective(NEAR, FAR);
        let eye_distance = if is_right {
            EYE_DISTANCE
        } else {
            -EYE_DISTANCE
        };
        let translation = TypedVector3D::new(eye_distance, 0.0, 0.0);
        let transform = TypedRigidTransform3D::from_translation(translation);
        View {
            transform,
            projection,
            viewport,
        }
    }

    fn perspective<Eye>(&self, near: f32, far: f32) -> TypedTransform3D<f32, Eye, Display> {
        // https://github.com/toji/gl-matrix/blob/bd3307196563fbb331b40fc6ebecbbfcc2a4722c/src/mat4.js#L1271
        let size = self.window.size();
        let width = size.width as f32;
        let height = size.height as f32;
        let fov_up = Angle::radians(f32::fast_atan2(2.0 * height, width));
        let f = 1.0 / fov_up.radians.tan();
        let nf = 1.0 / (near - far);
        let aspect = (width / 2.0) / height;

        // Dear rustfmt, This is a 4x4 matrix, please leave it alone. Best, ajeffrey.
        {
            #[rustfmt::skip]
            // Sigh, row-major vs column-major
            return TypedTransform3D::row_major(
                f / aspect, 0.0, 0.0,                   0.0,
                0.0,        f,   0.0,                   0.0,
                0.0,        0.0, (far + near) * nf,     -1.0,
                0.0,        0.0, 2.0 * far * near * nf, 0.0,
            );
        }
    }
}

pub type EventsLoopFactory = Box<dyn Fn() -> Result<EventsLoop, EventsLoopClosed>>;
