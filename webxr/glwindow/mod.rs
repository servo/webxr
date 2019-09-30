/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::utils::ClipPlanes;
use euclid::default::Size2D as UntypedSize2D;
use euclid::Angle;
use euclid::Point2D;
use euclid::Rect;
use euclid::RigidTransform3D;
use euclid::Rotation3D;
use euclid::Size2D;
use euclid::Transform3D;
use euclid::Trig;
use euclid::UnknownUnit;
use euclid::Vector3D;

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
use webxr_api::Event;
use webxr_api::EventBuffer;
use webxr_api::Floor;
use webxr_api::Frame;
use webxr_api::FrameUpdateEvent;
use webxr_api::InputSource;
use webxr_api::Native;
use webxr_api::Quitter;
use webxr_api::Sender;
use webxr_api::Session;
use webxr_api::SessionBuilder;
use webxr_api::SessionMode;
use webxr_api::View;
use webxr_api::Views;

const HEIGHT: f32 = 1.0;
const EYE_DISTANCE: f32 = 0.25;

pub trait GlWindow {
    fn make_current(&self);
    fn swap_buffers(&self);
    fn size(&self) -> UntypedSize2D<GLsizei>;
    fn new_window(&self) -> Result<Rc<dyn GlWindow>, ()>;
    fn get_rotation(&self) -> Rotation3D<f32, UnknownUnit, UnknownUnit>;
}

pub struct GlWindowDiscovery {
    gl: Rc<dyn Gl>,
    factory: Box<dyn Fn() -> Result<Rc<dyn GlWindow>, ()>>,
}

impl GlWindowDiscovery {
    pub fn new(
        gl: Rc<dyn Gl>,
        factory: Box<dyn Fn() -> Result<Rc<dyn GlWindow>, ()>>,
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
    window: Rc<dyn GlWindow>,
    read_fbo: GLuint,
    events: EventBuffer,
    clip_planes: ClipPlanes,
}

impl Device for GlWindowDevice {
    fn floor_transform(&self) -> RigidTransform3D<f32, Native, Floor> {
        let translation = Vector3D::new(-HEIGHT, 0.0, 0.0);
        RigidTransform3D::from_translation(translation)
    }

    fn views(&self) -> Views {
        let left = self.view(false);
        let right = self.view(true);
        Views::Stereo(left, right)
    }

    fn wait_for_animation_frame(&mut self) -> Option<Frame> {
        self.window.swap_buffers();
        let translation = Vector3D::new(0.0, 0.0, -5.0);
        let translation: RigidTransform3D<_, _, Native> =
            RigidTransform3D::from_translation(translation);
        let rotation = Rotation3D::from_untyped(&self.window.get_rotation());
        let rotation = RigidTransform3D::from_rotation(rotation);
        let transform = translation.post_transform(&rotation);
        let events = if self.clip_planes.recently_updated() {
            vec![FrameUpdateEvent::UpdateViews(self.views())]
        } else {
            vec![]
        };
        Some(Frame {
            transform,
            inputs: vec![],
            events,
        })
    }

    fn render_animation_frame(
        &mut self,
        texture_id: u32,
        size: UntypedSize2D<i32>,
        sync: Option<GLsync>,
    ) {
        // Perform the sync while the old window's GL context is active,
        // or it won't actually do anything - the XR window's GL context
        // doesn't recognize the sync object and triggers GL_INVALID_OPERATION.
        if let Some(sync) = sync {
            self.gl.wait_sync(sync, 0, gl::TIMEOUT_IGNORED);
            debug_assert_eq!(self.gl.get_error(), gl::NO_ERROR);
        }

        self.window.make_current();

        let width = size.width as GLsizei;
        let height = size.height as GLsizei;
        let inner_size = self.window.size();

        self.gl.clear_color(0.2, 0.3, 0.3, 1.0);
        self.gl.clear(gl::COLOR_BUFFER_BIT);
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

    fn set_event_dest(&mut self, dest: Sender<Event>) {
        self.events.upgrade(dest)
    }

    fn quit(&mut self) {
        self.events.callback(Event::SessionEnd);
    }

    fn set_quitter(&mut self, _: Quitter) {
        // Glwindow currently doesn't have any way to end its own session
        // XXXManishearth add something for this that listens for the window
        // being closed
    }

    fn update_clip_planes(&mut self, near: f32, far: f32) {
        self.clip_planes.update(near, far)
    }
}

impl GlWindowDevice {
    fn new(gl: Rc<dyn Gl>, window: Rc<dyn GlWindow>) -> Result<GlWindowDevice, Error> {
        window.make_current();
        let read_fbo = gl.gen_framebuffers(1)[0];
        debug_assert_eq!(gl.get_error(), gl::NO_ERROR);

        Ok(GlWindowDevice {
            gl,
            window,
            read_fbo,
            events: Default::default(),
            clip_planes: Default::default(),
        })
    }

    fn view<Eye>(&self, is_right: bool) -> View<Eye> {
        let window_size = self.window.size();
        let viewport_size = Size2D::new(window_size.width / 2, window_size.height);
        let viewport_x_origin = if is_right { viewport_size.width } else { 0 };
        let viewport_origin = Point2D::new(viewport_x_origin, 0);
        let viewport = Rect::new(viewport_origin, viewport_size);
        let projection = self.perspective();
        let eye_distance = if is_right {
            EYE_DISTANCE
        } else {
            -EYE_DISTANCE
        };
        let translation = Vector3D::new(eye_distance, 0.0, 0.0);
        let transform = RigidTransform3D::from_translation(translation);
        View {
            transform,
            projection,
            viewport,
        }
    }

    fn perspective<Eye>(&self) -> Transform3D<f32, Eye, Display> {
        let near = self.clip_planes.near;
        let far = self.clip_planes.far;
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
            return Transform3D::row_major(
                f / aspect, 0.0, 0.0,                   0.0,
                0.0,        f,   0.0,                   0.0,
                0.0,        0.0, (far + near) * nf,     -1.0,
                0.0,        0.0, 2.0 * far * near * nf, 0.0,
            );
        }
    }
}

pub type EventsLoopFactory = Box<dyn Fn() -> Result<EventsLoop, EventsLoopClosed>>;
