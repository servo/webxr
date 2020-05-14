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
use euclid::UnknownUnit;
use euclid::Vector3D;

use gleam::gl;
use gleam::gl::GLuint;
use gleam::gl::Gl;

use std::ffi::c_void;
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
use webxr_api::InputSource;
use webxr_api::Native;
use webxr_api::Quitter;
use webxr_api::Sender;
use webxr_api::Session;
use webxr_api::SessionInit;
use webxr_api::SessionMode;
use webxr_api::View;
use webxr_api::Viewport;
use webxr_api::Viewports;
use webxr_api::Views;

// How far off the ground are the viewer's eyes?
const HEIGHT: f32 = 1.0;

// What is half the vertical field of view?
const FOV_UP: f32 = 45.0;

// Some guesstimated numbers, hopefully it doesn't matter if these are off by a bit.

// What the distance between the viewer's eyes?
const INTER_PUPILLARY_DISTANCE: f32 = 0.06;

// What is the size of a pixel?
const PIXELS_PER_METRE: f32 = 6000.0;

pub trait GlWindow {
    fn get_native_widget(&self, device: &SurfmanDevice) -> NativeWidget;
    fn get_rotation(&self) -> Rotation3D<f32, UnknownUnit, UnknownUnit>;
    fn get_translation(&self) -> Vector3D<f32, UnknownUnit>;

    fn get_mode(&self) -> GlWindowMode {
        GlWindowMode::Blit
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum GlWindowMode {
    Blit,
    StereoLeftRight,
    StereoRedCyan,
}

impl GlWindowMode {
    fn is_anaglyph(&self) -> bool {
        match self {
            GlWindowMode::Blit | GlWindowMode::StereoLeftRight => false,
            GlWindowMode::StereoRedCyan => true,
        }
    }
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
    shader: Option<GlWindowShader>,
}

impl DeviceAPI<Surface> for GlWindowDevice {
    fn floor_transform(&self) -> Option<RigidTransform3D<f32, Native, Floor>> {
        let translation = Vector3D::new(0.0, HEIGHT, 0.0);
        Some(RigidTransform3D::from_translation(translation))
    }

    fn viewports(&self) -> Viewports {
        let size = self.viewport_size();
        Viewports {
            viewports: vec![
                Rect::new(Point2D::default(), size),
                Rect::new(Point2D::new(size.width, 0), size),
            ],
        }
    }

    fn wait_for_animation_frame(&mut self) -> Option<Frame> {
        debug_assert_eq!(
            (
                self.gl.get_error(),
                self.gl.check_frame_buffer_status(gl::FRAMEBUFFER)
            ),
            (gl::NO_ERROR, gl::FRAMEBUFFER_COMPLETE)
        );
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
        let framebuffer_object = self
            .device
            .context_surface_info(&self.context)
            .unwrap()
            .map(|info| info.framebuffer_object)
            .unwrap_or(0);
        self.gl
            .bind_framebuffer(gl::FRAMEBUFFER, framebuffer_object);
        debug_assert_eq!(
            (
                self.gl.get_error(),
                self.gl.check_frame_buffer_status(gl::FRAMEBUFFER)
            ),
            (gl::NO_ERROR, gl::FRAMEBUFFER_COMPLETE)
        );
        let time_ns = time::precise_time_ns();
        let translation = Vector3D::from_untyped(self.window.get_translation());
        let translation: RigidTransform3D<_, _, Native> =
            RigidTransform3D::from_translation(translation);
        let rotation = Rotation3D::from_untyped(&self.window.get_rotation());
        let rotation = RigidTransform3D::from_rotation(rotation);
        let transform = Some(translation.post_transform(&rotation));
        Some(Frame {
            transform,
            inputs: vec![],
            events: vec![],
            views: self.views(),
            time_ns,
            sent_time: 0,
            hit_test_results: vec![],
        })
    }

    fn render_animation_frame(&mut self, surface: Surface) -> Surface {
        self.device.make_context_current(&self.context).unwrap();
        debug_assert_eq!(self.gl.get_error(), gl::NO_ERROR);

        let viewport_size = self.viewport_size();
        let texture_size = self.device.surface_info(&surface).size;
        let surface_texture = self
            .device
            .create_surface_texture(&mut self.context, surface)
            .unwrap();
        let texture_id = self.device.surface_texture_object(&surface_texture);
        let texture_target = self.device.surface_gl_texture_target();

        self.gl.clear_color(0.2, 0.3, 0.3, 1.0);
        self.gl.clear(gl::COLOR_BUFFER_BIT);
        debug_assert_eq!(self.gl.get_error(), gl::NO_ERROR);

        if let Some(ref shader) = self.shader {
            shader.draw_texture(texture_id, texture_target, texture_size, viewport_size);
        } else {
            self.blit_texture(texture_id, texture_target, texture_size, viewport_size);
        }
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
        self.gl.delete_framebuffers(&[self.read_fbo]);
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

        let shader = GlWindowShader::new(gl.clone(), window.get_mode());
        debug_assert_eq!(gl.get_error(), gl::NO_ERROR);

        Ok(GlWindowDevice {
            gl,
            window,
            device,
            context,
            read_fbo,
            events: Default::default(),
            clip_planes: Default::default(),
            granted_features,
            shader,
        })
    }

    fn blit_texture(
        &self,
        texture_id: GLuint,
        texture_target: GLuint,
        texture_size: Size2D<i32, UnknownUnit>,
        viewport_size: Size2D<i32, Viewport>,
    ) {
        self.gl
            .bind_framebuffer(gl::READ_FRAMEBUFFER, self.read_fbo);
        self.gl.framebuffer_texture_2d(
            gl::READ_FRAMEBUFFER,
            gl::COLOR_ATTACHMENT0,
            texture_target,
            texture_id,
            0,
        );
        self.gl.blit_framebuffer(
            0,
            0,
            texture_size.width,
            texture_size.height,
            0,
            0,
            viewport_size.width * 2,
            viewport_size.height,
            gl::COLOR_BUFFER_BIT,
            gl::NEAREST,
        );
    }

    fn viewport_size(&self) -> Size2D<i32, Viewport> {
        let window_size = self
            .device
            .context_surface_info(&self.context)
            .unwrap()
            .unwrap()
            .size
            .to_i32();
        if self.window.get_mode().is_anaglyph() {
            // This device has a slightly odd characteristic, which is that anaglyphic stereo
            // renders both eyes to the same surface. If we want the two eyes to be parallel,
            // and to agree at distance infinity, this means gettng the XR content to render some
            // wasted pixels, which are stripped off when we render to the target surface.
            // (The wasted pixels are on the right of the left eye and vice versa.)
            let wasted_pixels = (INTER_PUPILLARY_DISTANCE / PIXELS_PER_METRE) as i32;
            Size2D::new(window_size.width + wasted_pixels, window_size.height)
        } else {
            Size2D::new(window_size.width / 2, window_size.height)
        }
    }

    fn views(&self) -> Views {
        let left = self.view(false);
        let right = self.view(true);
        Views::Stereo(left, right)
    }

    fn view<Eye>(&self, is_right: bool) -> View<Eye> {
        let projection = self.perspective();
        let translation = if is_right {
            Vector3D::new(-INTER_PUPILLARY_DISTANCE / 2.0, 0.0, 0.0)
        } else {
            Vector3D::new(INTER_PUPILLARY_DISTANCE / 2.0, 0.0, 0.0)
        };
        let transform = RigidTransform3D::from_translation(translation);
        View {
            transform,
            projection,
        }
    }

    fn perspective<Eye>(&self) -> Transform3D<f32, Eye, Display> {
        let near = self.clip_planes.near;
        let far = self.clip_planes.far;
        // https://gith<ub.com/toji/gl-matrix/blob/bd3307196563fbb331b40fc6ebecbbfcc2a4722c/src/mat4.js#L1271
        let size = self.viewport_size();
        let fov_up = Angle::degrees(FOV_UP);
        let f = 1.0 / fov_up.radians.tan();
        let nf = 1.0 / (near - far);
        let aspect = size.width as f32 / size.height as f32;

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

struct GlWindowShader {
    gl: Rc<dyn Gl>,
    buffer: GLuint,
    vao: GLuint,
    program: GLuint,
    mode: GlWindowMode,
}

const VERTEX_ATTRIBUTE: GLuint = 0;
const VERTICES: &[[f32; 2]; 4] = &[[-1.0, -1.0], [-1.0, 1.0], [1.0, -1.0], [1.0, 1.0]];

const PASSTHROUGH_VERTEX_SHADER: &[u8] = b"
  #version 330 core
  layout(location=0) in vec2 coord;
  out vec2 vTexCoord;
  void main(void) {
    gl_Position = vec4(coord, 0.0, 1.0);
    vTexCoord = coord * 0.5 + 0.5;
  }
";

const PASSTHROUGH_FRAGMENT_SHADER: &[u8] = b"
  #version 330 core
  layout(location=0) out vec4 color;
  uniform sampler2D image;
  in vec2 vTexCoord;
  void main() {
    color = texture(image, vTexCoord);
  }
";

const ANAGLYPH_VERTEX_SHADER: &[u8] = b"
  #version 330 core
  layout(location=0) in vec2 coord;
  uniform float wasted; // What fraction of the image is wasted?
  out vec2 left_coord;
  out vec2 right_coord;
  void main(void) {
    gl_Position = vec4(coord, 0.0, 1.0);
    vec2 coordn = coord * 0.5 + 0.5;
    left_coord = vec2(mix(wasted/2, 0.5, coordn.x), coordn.y);
    right_coord = vec2(mix(0.5, 1-wasted/2, coordn.x), coordn.y);
  }
";

const ANAGLYPH_RED_CYAN_FRAGMENT_SHADER: &[u8] = b"
  #version 330 core
  layout(location=0) out vec4 color;
  uniform sampler2D image;
  in vec2 left_coord;
  in vec2 right_coord;
  void main() {
    vec4 left_color = texture(image, left_coord);
    vec4 right_color = texture(image, right_coord);
    float red = left_color.x;
    float green = right_color.y;
    float blue = right_color.z;
    color = vec4(red, green, blue, 1.0);
  }
";

impl GlWindowShader {
    fn new(gl: Rc<dyn Gl>, mode: GlWindowMode) -> Option<GlWindowShader> {
        // The shader source
        let (vertex_source, fragment_source) = match mode {
            GlWindowMode::Blit => {
                return None;
            }
            GlWindowMode::StereoLeftRight => {
                (PASSTHROUGH_VERTEX_SHADER, PASSTHROUGH_FRAGMENT_SHADER)
            }
            GlWindowMode::StereoRedCyan => {
                (ANAGLYPH_VERTEX_SHADER, ANAGLYPH_RED_CYAN_FRAGMENT_SHADER)
            }
        };

        // TODO: work out why shaders don't work on macos
        if cfg!(target_os = "macos") {
            log::warn!("XR shaders may not render on MacOS.");
        }

        // The four corners of the window in a VAO, set to attribute 0
        let buffer = gl.gen_buffers(1)[0];
        let vao = gl.gen_vertex_arrays(1)[0];
        gl.bind_buffer(gl::ARRAY_BUFFER, buffer);
        gl.buffer_data_untyped(
            gl::ARRAY_BUFFER,
            std::mem::size_of_val(VERTICES) as isize,
            VERTICES as *const _ as *const c_void,
            gl::STATIC_DRAW,
        );
        gl.bind_vertex_array(vao);
        gl.vertex_attrib_pointer(
            VERTEX_ATTRIBUTE,
            VERTICES[0].len() as i32,
            gl::FLOAT,
            false,
            0,
            0,
        );
        gl.enable_vertex_attrib_array(VERTEX_ATTRIBUTE);
        debug_assert_eq!(gl.get_error(), gl::NO_ERROR);

        // The shader program
        let program = gl.create_program();
        let vertex_shader = gl.create_shader(gl::VERTEX_SHADER);
        let fragment_shader = gl.create_shader(gl::FRAGMENT_SHADER);
        gl.shader_source(vertex_shader, &[vertex_source]);
        gl.compile_shader(vertex_shader);
        gl.attach_shader(program, vertex_shader);
        gl.shader_source(fragment_shader, &[fragment_source]);
        gl.compile_shader(fragment_shader);
        gl.attach_shader(program, fragment_shader);
        gl.link_program(program);
        debug_assert_eq!(gl.get_error(), gl::NO_ERROR);

        // Check for errors
        // TODO: something other than panic?
        let mut status = [0];
        unsafe { gl.get_shader_iv(vertex_shader, gl::COMPILE_STATUS, &mut status) };
        assert_eq!(
            status[0],
            gl::TRUE as i32,
            "Failed to compile vertex shader: {}",
            gl.get_shader_info_log(vertex_shader)
        );
        unsafe { gl.get_shader_iv(fragment_shader, gl::COMPILE_STATUS, &mut status) };
        assert_eq!(
            status[0],
            gl::TRUE as i32,
            "Failed to compile fragment shader: {}",
            gl.get_shader_info_log(fragment_shader)
        );
        unsafe { gl.get_program_iv(program, gl::LINK_STATUS, &mut status) };
        assert_eq!(
            status[0],
            gl::TRUE as i32,
            "Failed to link: {}",
            gl.get_program_info_log(program)
        );

        // Clean up
        gl.delete_shader(vertex_shader);
        debug_assert_eq!(gl.get_error(), gl::NO_ERROR);
        gl.delete_shader(fragment_shader);
        debug_assert_eq!(gl.get_error(), gl::NO_ERROR);

        // And we're done
        Some(GlWindowShader {
            gl,
            buffer,
            vao,
            program,
            mode,
        })
    }

    fn draw_texture(
        &self,
        texture_id: GLuint,
        texture_target: GLuint,
        texture_size: Size2D<i32, UnknownUnit>,
        viewport_size: Size2D<i32, Viewport>,
    ) {
        self.gl.use_program(self.program);
        self.gl.bind_vertex_array(self.vao);
        self.gl.active_texture(gl::TEXTURE0);
        self.gl.bind_texture(texture_target, texture_id);

        if self.mode.is_anaglyph() {
            let wasted = 1.0
                - (texture_size.width as f32 / viewport_size.width as f32)
                    .max(0.0)
                    .min(1.0);
            let wasted_location = self.gl.get_uniform_location(self.program, "wasted");
            self.gl.uniform_1f(wasted_location, wasted);
        }

        self.gl
            .draw_arrays(gl::TRIANGLE_STRIP, 0, VERTICES.len() as i32);
        debug_assert_eq!(self.gl.get_error(), gl::NO_ERROR);
    }
}

impl Drop for GlWindowShader {
    fn drop(&mut self) {
        self.gl.delete_buffers(&[self.buffer]);
        self.gl.delete_vertex_arrays(&[self.vao]);
        self.gl.delete_program(self.program);
    }
}
