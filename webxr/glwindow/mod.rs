/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use euclid::Size2D;
use euclid::TypedRigidTransform3D;
use euclid::TypedTransform3D;
use euclid::TypedVector3D;

use gleam::gl;
use gleam::gl::GLsizei;
use gleam::gl::GLsync;
use gleam::gl::Gl;

use glutin::dpi::PhysicalSize;
use glutin::EventsLoop;
use glutin::EventsLoopClosed;
use glutin::GlRequest;
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
use webxr_api::View;
use webxr_api::Views;

const HEIGHT: f32 = 1.0;
const EYE_DISTANCE: f32 = 0.25;
const NEAR: f32 = 0.1;
const FAR: f32 = 100.0;

pub struct GlWindowDiscovery {
    gl: Rc<Gl>,
    events_loop_factory: EventsLoopFactory,
    gl_version: GlRequest,
}

impl GlWindowDiscovery {
    pub fn new(
        gl: Rc<Gl>,
        events_loop_factory: EventsLoopFactory,
        gl_version: GlRequest,
    ) -> GlWindowDiscovery {
        GlWindowDiscovery {
            gl,
            events_loop_factory,
            gl_version,
        }
    }
}

impl Discovery for GlWindowDiscovery {
    fn request_session(&mut self, mode: SessionMode, xr: SessionBuilder) -> Result<Session, Error> {
        if self.supports_session(mode) {
            let gl = self.gl.clone();
            let gl_version = self.gl_version;
            let events_loop = (self.events_loop_factory)().or(Err(Error::NoMatchingDevice))?;
            xr.run_on_main_thread(move || GlWindowDevice::new(gl, gl_version, events_loop))
        } else {
            Err(Error::NoMatchingDevice)
        }
    }

    fn supports_session(&self, mode: SessionMode) -> bool {
        mode == SessionMode::ImmersiveVR
    }
}

pub struct GlWindowDevice {
    size: PhysicalSize,
    gl_context: WindowedContext<PossiblyCurrent>,
    gl: Rc<Gl>,
    // This will become used when we support keyboard bindings for the WebXR glwindow
    #[allow(dead_code)]
    events_loop: EventsLoop,
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

    fn render_animation_frame(&mut self, texture_id: u32, size: Size2D<i32>, sync: GLsync) {
        let width = size.width as GLsizei;
        let height = size.height as GLsizei;
        self.gl.clear_color(0.2, 0.3, 0.3, 1.0);
        self.gl.clear(gl::COLOR_BUFFER_BIT);
        self.gl.wait_sync(sync, 0, gl::TIMEOUT_IGNORED);
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
    fn new(
        gl: Rc<Gl>,
        gl_version: glutin::GlRequest,
        events_loop: glutin::EventsLoop,
    ) -> Result<GlWindowDevice, Error> {
        let window_builder = glutin::WindowBuilder::new()
            .with_title("Test XR device")
            .with_visibility(true)
            .with_multitouch();
        let gl_context = unsafe {
            glutin::ContextBuilder::new()
                .with_gl(gl_version)
                .with_vsync(false) // Assume the browser vsync is the same as the test VR window vsync
                .build_windowed(window_builder, &events_loop)
                .or(Err(Error::NoMatchingDevice))?
                .make_current()
                .or(Err(Error::NoMatchingDevice))?
        };
        let logical_size = gl_context
            .window()
            .get_inner_size()
            .ok_or(Error::NoMatchingDevice)?;
        let hidpi = gl_context.window().get_hidpi_factor();
        let size = logical_size.to_physical(hidpi);
        Ok(GlWindowDevice {
            gl_context,
            events_loop,
            gl,
            size,
        })
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

pub type EventsLoopFactory = Box<Fn() -> Result<EventsLoop, EventsLoopClosed>>;
