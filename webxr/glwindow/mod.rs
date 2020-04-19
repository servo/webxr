/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::SessionBuilder;
use crate::SwapChains;

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
use gleam::gl::GLuint;
use gleam::gl::Gl;

use std::rc::Rc;

use surfman::Adapter;
use surfman::Connection;
use surfman::Context;
use surfman::ContextAttributes;
use surfman::Device as SurfmanDevice;
use surfman::GLApi;
use surfman::NativeWidget;
use surfman::Surface;
use surfman::SurfaceAccess;
use surfman::SurfaceType;

use webxr_api::util::ClipPlanes;
use webxr_api::DeviceAPI;
use webxr_api::DiscoveryAPI;
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
use webxr_api::SessionInit;
use webxr_api::SessionMode;
use webxr_api::View;
use webxr_api::Views;

const HEIGHT: f32 = 1.0;
const EYE_DISTANCE: f32 = 0.25;

pub trait GlWindow {
    fn get_native_widget(&self, device: &SurfmanDevice) -> NativeWidget;
    fn get_rotation(&self) -> Rotation3D<f32, UnknownUnit, UnknownUnit>;
    fn get_translation(&self) -> Vector3D<f32, UnknownUnit>;
}

pub struct GlWindowDiscovery {
    connection: Connection,
    adapter: Adapter,
    context_attributes: ContextAttributes,
    factory: Box<dyn Fn() -> Result<Box<dyn GlWindow>, ()>>,
}

impl GlWindowDiscovery {
    pub fn new(
        connection: Connection,
        adapter: Adapter,
        context_attributes: ContextAttributes,
        factory: Box<dyn Fn() -> Result<Box<dyn GlWindow>, ()>>,
    ) -> GlWindowDiscovery {
        GlWindowDiscovery {
            connection,
            adapter,
            context_attributes,
            factory,
        }
    }
}

impl DiscoveryAPI<SwapChains> for GlWindowDiscovery {
    fn request_session(
        &mut self,
        mode: SessionMode,
        init: &SessionInit,
        xr: SessionBuilder,
    ) -> Result<Session, Error> {
        if self.supports_session(mode) {
            let granted_features = init.validate(mode, &["local-floor".into()])?;
            let connection = self.connection.clone();
            let adapter = self.adapter.clone();
            let context_attributes = self.context_attributes.clone();
            let window = (self.factory)().or(Err(Error::NoMatchingDevice))?;
            xr.run_on_main_thread(move || {
                GlWindowDevice::new(
                    connection,
                    adapter,
                    context_attributes,
                    window,
                    granted_features,
                )
            })
        } else {
            Err(Error::NoMatchingDevice)
        }
    }

    fn supports_session(&self, mode: SessionMode) -> bool {
        mode == SessionMode::ImmersiveVR || mode == SessionMode::ImmersiveAR
    }
}

pub struct GlWindowDevice {
    device: SurfmanDevice,
    context: Context,
    gl: Rc<dyn Gl>,
    window: Box<dyn GlWindow>,
    read_fbo: GLuint,
    events: EventBuffer,
    clip_planes: ClipPlanes,
    granted_features: Vec<String>,
}

impl DeviceAPI<Surface> for GlWindowDevice {
    fn floor_transform(&self) -> Option<RigidTransform3D<f32, Native, Floor>> {
        let translation = Vector3D::new(HEIGHT, 0.0, 0.0);
        Some(RigidTransform3D::from_translation(translation))
    }

    fn views(&self) -> Views {
        let left = self.view(false);
        let right = self.view(true);
        Views::Stereo(left, right)
    }

    fn wait_for_animation_frame(&mut self) -> Option<Frame> {
        let mut surface = self
            .device
            .unbind_surface_from_context(&mut self.context)
            .unwrap()
            .unwrap();
        self.device
            .present_surface(&self.context, &mut surface)
            .unwrap();
        self.device
            .bind_surface_to_context(&mut self.context, surface)
            .unwrap();
        let time_ns = time::precise_time_ns();
        let translation = Vector3D::from_untyped(self.window.get_translation());
        let translation: RigidTransform3D<_, _, Native> =
            RigidTransform3D::from_translation(translation);
        let rotation = Rotation3D::from_untyped(&self.window.get_rotation());
        let rotation = RigidTransform3D::from_rotation(rotation);
        let transform = Some(translation.post_transform(&rotation));
        let events = if self.clip_planes.recently_updated() {
            vec![FrameUpdateEvent::UpdateViews(self.views())]
        } else {
            vec![]
        };
        Some(Frame {
            transform,
            inputs: vec![],
            events,
            time_ns,
            sent_time: 0,
            hit_test_results: vec![],
        })
    }

    fn render_animation_frame(&mut self, surface: Surface) -> Surface {
        self.device.make_context_current(&self.context).unwrap();
        debug_assert_eq!(self.gl.get_error(), gl::NO_ERROR);

        let size = self.device.surface_info(&surface).size;
        let surface_texture = self
            .device
            .create_surface_texture(&mut self.context, surface)
            .unwrap();
        let texture_id = self.device.surface_texture_object(&surface_texture);

        let width = size.width as GLsizei;
        let height = size.height as GLsizei;
        let inner_size = self
            .device
            .context_surface_info(&self.context)
            .unwrap()
            .unwrap()
            .size;

        self.gl.clear_color(0.2, 0.3, 0.3, 1.0);
        self.gl.clear(gl::COLOR_BUFFER_BIT);
        debug_assert_eq!(self.gl.get_error(), gl::NO_ERROR);

        self.gl
            .bind_framebuffer(gl::READ_FRAMEBUFFER, self.read_fbo);
        debug_assert_eq!(self.gl.get_error(), gl::NO_ERROR);

        self.gl.framebuffer_texture_2d(
            gl::READ_FRAMEBUFFER,
            gl::COLOR_ATTACHMENT0,
            self.device.surface_gl_texture_target(),
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

        self.device
            .destroy_surface_texture(&mut self.context, surface_texture)
            .unwrap()
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

    fn granted_features(&self) -> &[String] {
        &self.granted_features
    }
}

impl Drop for GlWindowDevice {
    fn drop(&mut self) {
        let _ = self.device.destroy_context(&mut self.context);
    }
}

impl GlWindowDevice {
    fn new(
        connection: Connection,
        adapter: Adapter,
        context_attributes: ContextAttributes,
        window: Box<dyn GlWindow>,
        granted_features: Vec<String>,
    ) -> Result<GlWindowDevice, Error> {
        let mut device = connection.create_device(&adapter).unwrap();
        let context_descriptor = device
            .create_context_descriptor(&context_attributes)
            .unwrap();
        let mut context = device.create_context(&context_descriptor).unwrap();
        let native_widget = window.get_native_widget(&device);
        let surface_type = SurfaceType::Widget { native_widget };
        let surface = device
            .create_surface(&context, SurfaceAccess::GPUOnly, surface_type)
            .unwrap();
        device.make_context_current(&context).unwrap();
        device
            .bind_surface_to_context(&mut context, surface)
            .unwrap();

        let gl = match device.gl_api() {
            GLApi::GL => unsafe { gl::GlFns::load_with(|s| device.get_proc_address(&context, s)) },
            GLApi::GLES => unsafe {
                gl::GlesFns::load_with(|s| device.get_proc_address(&context, s))
            },
        };
        let read_fbo = gl.gen_framebuffers(1)[0];
        debug_assert_eq!(gl.get_error(), gl::NO_ERROR);

        let framebuffer_object = device
            .context_surface_info(&context)
            .unwrap()
            .map(|info| info.framebuffer_object)
            .unwrap_or(0);
        gl.bind_framebuffer(gl::FRAMEBUFFER, framebuffer_object);
        debug_assert_eq!(
            (
                gl.get_error(),
                gl.check_frame_buffer_status(gl::FRAMEBUFFER)
            ),
            (gl::NO_ERROR, gl::FRAMEBUFFER_COMPLETE)
        );

        Ok(GlWindowDevice {
            gl,
            window,
            device,
            context,
            read_fbo,
            events: Default::default(),
            clip_planes: Default::default(),
            granted_features,
        })
    }

    fn view<Eye>(&self, is_right: bool) -> View<Eye> {
        let window_size = self
            .device
            .context_surface_info(&self.context)
            .unwrap()
            .unwrap()
            .size;
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
        let size = self
            .device
            .context_surface_info(&self.context)
            .unwrap()
            .unwrap()
            .size;
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
