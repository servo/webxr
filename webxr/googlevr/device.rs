/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use webxr_api::Device;
use webxr_api::Error;
use webxr_api::Event;
use webxr_api::EventBuffer;
use webxr_api::Floor;
use webxr_api::Frame;
use webxr_api::FrameUpdateEvent;
use webxr_api::InputFrame;
use webxr_api::InputId;
use webxr_api::InputSource;
use webxr_api::Native;
use webxr_api::Quitter;
use webxr_api::Sender;
use webxr_api::TargetRayMode;
use webxr_api::View;
use webxr_api::Viewer;
use webxr_api::Views;

use crate::gles as gl;
use crate::utils::ClipPlanes;

use euclid::default::Size2D as DefaultSize2D;
use euclid::Point2D;
use euclid::Rect;
use euclid::RigidTransform3D;
use euclid::Rotation3D;
use euclid::Size2D;
use euclid::Transform3D;
use euclid::Vector3D;

use gvr_sys as gvr;
use gvr_sys::gvr_color_format_type::*;
use gvr_sys::gvr_depth_stencil_format_type::*;
use gvr_sys::gvr_feature::*;

use log::warn;

use std::{mem, ptr};

use super::discovery::SendPtr;
use super::input::GoogleVRController;

use surfman::platform::generic::universal::context::Context as SurfmanContext;
use surfman::platform::generic::universal::device::Device as SurfmanDevice;
use surfman::platform::generic::universal::surface::Surface;

#[cfg(target_os = "android")]
use crate::jni_utils::JNIScope;
#[cfg(target_os = "android")]
use android_injected_glue::ffi as ndk;

// 50ms is a good estimate recommended by the GVR Team.
// It takes in account the time between frame submission (without vsync) and
// when the rendered image is sent to the physical pixels on the display.
const PREDICTION_OFFSET_NANOS: i64 = 50000000; // 50ms

pub(crate) struct GoogleVRDevice {
    events: EventBuffer,
    multiview: bool,
    multisampling: bool,
    depth: bool,
    clip_planes: ClipPlanes,
    input: Option<GoogleVRController>,

    #[cfg(target_os = "android")]
    java_class: ndk::jclass,
    #[cfg(target_os = "android")]
    java_object: ndk::jobject,
    ctx: *mut gvr::gvr_context,
    controller_ctx: *mut gvr::gvr_controller_context,
    viewport_list: *mut gvr::gvr_buffer_viewport_list,
    left_eye_vp: *mut gvr::gvr_buffer_viewport,
    right_eye_vp: *mut gvr::gvr_buffer_viewport,
    render_size: gvr::gvr_sizei,
    swap_chain: *mut gvr::gvr_swap_chain,
    frame: *mut gvr::gvr_frame,
    synced_head_matrix: gvr::gvr_mat4f,
    fbo_id: u32,
    fbo_texture: u32,
    presenting: bool,
    frame_bound: bool,
    surfman: Option<(SurfmanDevice, SurfmanContext)>,
}

impl GoogleVRDevice {
    #[cfg(target_os = "android")]
    pub fn new(
        ctx: SendPtr<*mut gvr::gvr_context>,
        controller_ctx: SendPtr<*mut gvr::gvr_controller_context>,
        java_class: SendPtr<ndk::jclass>,
        java_object: SendPtr<ndk::jobject>,
    ) -> Result<Self, Error> {
        let mut device = GoogleVRDevice {
            events: Default::default(),
            multiview: false,
            multisampling: false,
            depth: false,
            clip_planes: Default::default(),
            input: None,

            ctx: ctx.get(),
            controller_ctx: controller_ctx.get(),
            java_class: java_class.get(),
            java_object: java_object.get(),
            viewport_list: ptr::null_mut(),
            left_eye_vp: ptr::null_mut(),
            right_eye_vp: ptr::null_mut(),
            render_size: gvr::gvr_sizei {
                width: 0,
                height: 0,
            },
            swap_chain: ptr::null_mut(),
            frame: ptr::null_mut(),
            synced_head_matrix: gvr_identity_matrix(),
            fbo_id: 0,
            fbo_texture: 0,
            presenting: false,
            frame_bound: false,
            surfman: None,
        };
        unsafe {
            device.init();
        }
        // XXXManishearth figure out how to block until presentation
        // starts
        device.start_present();
        Ok(device)
    }

    #[cfg(not(target_os = "android"))]
    pub fn new(
        ctx: SendPtr<*mut gvr::gvr_context>,
        controller_ctx: SendPtr<*mut gvr::gvr_controller_context>,
    ) -> Result<Self, Error> {
        let mut device = GoogleVRDevice {
            events: Default::default(),
            multiview: false,
            multisampling: false,
            depth: false,
            clip_planes: Default::default(),
            input: None,

            ctx: ctx.get(),
            controller_ctx: controller_ctx.get(),
            viewport_list: ptr::null_mut(),
            left_eye_vp: ptr::null_mut(),
            right_eye_vp: ptr::null_mut(),
            render_size: gvr::gvr_sizei {
                width: 0,
                height: 0,
            },
            swap_chain: ptr::null_mut(),
            frame: ptr::null_mut(),
            synced_head_matrix: gvr_identity_matrix(),
            fbo_id: 0,
            fbo_texture: 0,
            presenting: false,
            frame_bound: false,
            surfman: None,
        };
        unsafe {
            device.init();
        }
        // XXXManishearth figure out how to block until presentation
        // starts
        device.start_present();
        Ok(device)
    }

    unsafe fn init(&mut self) {
        let list = gvr::gvr_buffer_viewport_list_create(self.ctx);

        // gvr_refresh_viewer_profile must be called before getting recommended bufer viewports.
        gvr::gvr_refresh_viewer_profile(self.ctx);

        // Gets the recommended buffer viewport configuration, populating a previously
        // allocated gvr_buffer_viewport_list object. The updated values include the
        // per-eye recommended viewport and field of view for the target.
        gvr::gvr_get_recommended_buffer_viewports(self.ctx, list);

        // Create viewport buffers for both eyes.
        self.left_eye_vp = gvr::gvr_buffer_viewport_create(self.ctx);
        gvr::gvr_buffer_viewport_list_get_item(
            list,
            gvr::gvr_eye::GVR_LEFT_EYE as usize,
            self.left_eye_vp,
        );
        self.right_eye_vp = gvr::gvr_buffer_viewport_create(self.ctx);
        gvr::gvr_buffer_viewport_list_get_item(
            list,
            gvr::gvr_eye::GVR_RIGHT_EYE as usize,
            self.right_eye_vp,
        );

        if let Ok(input) = GoogleVRController::new(self.ctx, self.controller_ctx) {
            self.input = Some(input);
        }
    }

    unsafe fn initialize_gl(&mut self) {
        // Note: In some scenarios gvr_initialize_gl crashes if gvr_refresh_viewer_profile call isn't called before.
        gvr::gvr_refresh_viewer_profile(self.ctx);
        // Initializes gvr necessary GL-related objects.
        gvr::gvr_initialize_gl(self.ctx);

        self.surfman = SurfmanDevice::from_current_hardware_context().ok();

        // GVR_FEATURE_MULTIVIEW must be checked after gvr_initialize_gl is called or the function will crash.
        if self.multiview && !gvr::gvr_is_feature_supported(self.ctx, GVR_FEATURE_MULTIVIEW as i32)
        {
            self.multiview = false;
            warn!("Multiview not supported. Fallback to standar framebuffer.")
        }

        // Create a framebuffer required to attach and
        // blit the external texture into the main gvr pixel buffer.
        gl::GenFramebuffers(1, &mut self.fbo_id);

        // Initialize gvr swap chain
        let spec = gvr::gvr_buffer_spec_create(self.ctx);
        self.render_size = self.recommended_render_size();

        if self.multiview {
            // Multiview requires half size because the buffer is a texture array with 2 half width layers.
            gvr::gvr_buffer_spec_set_multiview_layers(spec, 2);
            gvr::gvr_buffer_spec_set_size(
                spec,
                gvr::gvr_sizei {
                    width: self.render_size.width / 2,
                    height: self.render_size.height,
                },
            );
        } else {
            gvr::gvr_buffer_spec_set_size(spec, self.render_size);
        }

        if self.multisampling {
            gvr::gvr_buffer_spec_set_samples(spec, 2);
        } else {
            gvr::gvr_buffer_spec_set_samples(spec, 0);
        }
        gvr::gvr_buffer_spec_set_color_format(spec, GVR_COLOR_FORMAT_RGBA_8888 as i32);

        if self.depth {
            gvr::gvr_buffer_spec_set_depth_stencil_format(
                spec,
                GVR_DEPTH_STENCIL_FORMAT_DEPTH_16 as i32,
            );
        } else {
            gvr::gvr_buffer_spec_set_depth_stencil_format(
                spec,
                GVR_DEPTH_STENCIL_FORMAT_NONE as i32,
            );
        }

        self.swap_chain = gvr::gvr_swap_chain_create(self.ctx, mem::transmute(&spec), 1);
        gvr::gvr_buffer_spec_destroy(mem::transmute(&spec));
    }

    fn recommended_render_size(&self) -> gvr::gvr_sizei {
        // GVR SDK states that thee maximum effective render target size can be very large.
        // Most applications need to scale down to compensate.
        // Half pixel sizes are used by scaling each dimension by sqrt(2)/2 ~= 7/10ths.
        let render_target_size =
            unsafe { gvr::gvr_get_maximum_effective_render_target_size(self.ctx) };
        gvr::gvr_sizei {
            width: (7 * render_target_size.width) / 10,
            height: (7 * render_target_size.height) / 10,
        }
    }

    #[cfg(target_os = "android")]
    fn start_present(&mut self) {
        if self.presenting {
            return;
        }
        self.presenting = true;
        unsafe {
            if let Ok(jni_scope) = JNIScope::attach() {
                let jni = jni_scope.jni();
                let env = jni_scope.env;
                let method = jni_scope.get_method(self.java_class, "startPresent", "()V", false);
                (jni.CallVoidMethod)(env, self.java_object, method);
            }
        }

        if self.swap_chain.is_null() {
            unsafe {
                self.initialize_gl();
                debug_assert!(!self.swap_chain.is_null());
            }
        }
    }

    #[cfg(not(target_os = "android"))]
    fn start_present(&mut self) {
        if self.presenting {
            return;
        }
        self.presenting = true;
        if self.swap_chain.is_null() {
            unsafe {
                self.initialize_gl();
                debug_assert!(!self.swap_chain.is_null());
            }
        }
    }

    // Hint to indicate that we are going to stop sending frames to the device
    #[cfg(target_os = "android")]
    fn stop_present(&mut self) {
        if !self.presenting {
            return;
        }
        self.presenting = false;
        unsafe {
            if let Ok(jni_scope) = JNIScope::attach() {
                let jni = jni_scope.jni();
                let env = jni_scope.env;
                let method = jni_scope.get_method(self.java_class, "stopPresent", "()V", false);
                (jni.CallVoidMethod)(env, self.java_object, method);
            }
        }
    }

    #[cfg(not(target_os = "android"))]
    fn stop_present(&mut self) {
        self.presenting = false;
    }

    unsafe fn fetch_eye<T>(&self, eye: gvr::gvr_eye, vp: *mut gvr::gvr_buffer_viewport) -> View<T> {
        let eye_fov = gvr::gvr_buffer_viewport_get_source_fov(vp);
        let projection = fov_to_projection_matrix(&eye_fov, self.clip_planes);

        // this matrix converts from head space to eye space,
        // i.e. it's the inverse of the offset
        let eye_mat = gvr::gvr_get_eye_from_head_matrix(self.ctx, eye as i32);
        // XXXManishearth we should decompose the matrix properly instead of assuming it's
        // only translation
        let transform = decompose_rigid(&eye_mat).inverse();

        let size = Size2D::new(self.render_size.width / 2, self.render_size.height);
        let origin = if eye == gvr::gvr_eye::GVR_LEFT_EYE {
            Point2D::origin()
        } else {
            Point2D::new(self.render_size.width / 2, 0)
        };
        let viewport = Rect::new(origin, size);

        View {
            projection,
            transform,
            viewport,
        }
    }

    fn bind_framebuffer(&mut self) {
        // No op
        if self.frame.is_null() {
            warn!("null frame with context");
            return;
        }

        unsafe {
            if self.frame_bound {
                // Required to avoid some warnings from the GVR SDK.
                // It doesn't like binding the same framebuffer multiple times.
                gvr::gvr_frame_unbind(self.frame);
            }
            // gvr_frame_bind_buffer may make the current active texture unit dirty
            let mut active_unit = 0;
            gl::GetIntegerv(gl::ACTIVE_TEXTURE, &mut active_unit);

            // Bind daydream FBO
            gvr::gvr_frame_bind_buffer(self.frame, 0);
            self.frame_bound = true;

            // Restore texture unit
            gl::ActiveTexture(active_unit as u32);
        }
    }

    fn update_recommended_buffer_viewports(&self) {
        unsafe {
            gvr::gvr_get_recommended_buffer_viewports(self.ctx, self.viewport_list);
            if self.multiview {
                // gvr_get_recommended_buffer_viewports function assumes that the client is not
                // using multiview to render to multiple layers simultaneously.
                // The uv and source layers need to be updated for multiview.
                let fullscreen_uv = gvr_texture_bounds(&[0.0, 0.0, 1.0, 1.0]);
                // Left eye
                gvr::gvr_buffer_viewport_set_source_uv(self.left_eye_vp, fullscreen_uv);
                gvr::gvr_buffer_viewport_set_source_layer(self.left_eye_vp, 0);
                // Right eye
                gvr::gvr_buffer_viewport_set_source_uv(self.right_eye_vp, fullscreen_uv);
                gvr::gvr_buffer_viewport_set_source_layer(self.right_eye_vp, 1);
                // Update viewport list
                gvr::gvr_buffer_viewport_list_set_item(self.viewport_list, 0, self.left_eye_vp);
                gvr::gvr_buffer_viewport_list_set_item(self.viewport_list, 1, self.right_eye_vp);
            }
        }
    }

    fn fetch_head_matrix(&mut self) -> RigidTransform3D<f32, Viewer, Native> {
        let mut next_vsync = unsafe { gvr::gvr_get_time_point_now() };
        next_vsync.monotonic_system_time_nanos += PREDICTION_OFFSET_NANOS;
        unsafe {
            let m = gvr::gvr_get_head_space_from_start_space_rotation(self.ctx, next_vsync);
            self.synced_head_matrix = gvr::gvr_apply_neck_model(self.ctx, m, 1.0);
        };
        decompose_rigid(&self.synced_head_matrix)
    }

    unsafe fn acquire_frame(&mut self) {
        if !self.frame.is_null() {
            warn!("frame not submitted");
            // Release acquired frame if the user has not called submit_Frame()
            gvr::gvr_frame_submit(
                mem::transmute(&self.frame),
                self.viewport_list,
                self.synced_head_matrix,
            );
        }

        self.update_recommended_buffer_viewports();
        // Handle resize
        let size = self.recommended_render_size();
        if size.width != self.render_size.width || size.height != self.render_size.height {
            gvr::gvr_swap_chain_resize_buffer(self.swap_chain, 0, size);
            self.render_size = size;
        }

        self.frame = gvr::gvr_swap_chain_acquire_frame(self.swap_chain);
    }

    fn render_layer(
        &mut self,
        texture_id: u32,
        texture_size: DefaultSize2D<i32>,
        texture_target: u32,
    ) {
        if self.frame.is_null() {
            warn!("null frame when calling render_layer");
            return;
        }
        debug_assert!(self.fbo_id > 0);

        unsafe {
            // Save current fbo to restore it when the frame is submitted.
            let mut current_fbo = 0;
            gl::GetIntegerv(gl::FRAMEBUFFER_BINDING, &mut current_fbo);

            if self.fbo_texture != texture_id {
                // Attach external texture to the used later in BlitFramebuffer.
                gl::BindFramebuffer(gl::FRAMEBUFFER, self.fbo_id);
                gl::FramebufferTexture2D(
                    gl::FRAMEBUFFER,
                    gl::COLOR_ATTACHMENT0,
                    texture_target,
                    texture_id,
                    0,
                );
                self.fbo_texture = texture_id;
            }

            // BlitFramebuffer: external texture to gvr pixel buffer
            self.bind_framebuffer();
            gl::BindFramebuffer(gl::READ_FRAMEBUFFER, self.fbo_id);
            gl::BlitFramebuffer(
                0,
                0,
                texture_size.width,
                texture_size.height,
                0,
                0,
                self.render_size.width,
                self.render_size.height,
                gl::COLOR_BUFFER_BIT,
                gl::LINEAR,
            );
            gvr::gvr_frame_unbind(self.frame);
            self.frame_bound = false;
            // Restore bound fbo
            gl::BindFramebuffer(gl::FRAMEBUFFER, current_fbo as u32);

            // set up uvs
            // XXXManishearth do we need to negotiate size here?
            // gvr::gvr_buffer_viewport_set_source_uv(self.left_eye_vp, gvr_texture_bounds(&layer.left_bounds));
            // gvr::gvr_buffer_viewport_set_source_uv(self.right_eye_vp, gvr_texture_bounds(&layer.right_bounds));
        }
    }

    fn submit_frame(&mut self) {
        if self.frame.is_null() {
            warn!("null frame with context");
            return;
        }

        unsafe {
            if self.frame_bound {
                gvr::gvr_frame_unbind(self.frame);
                self.frame_bound = false;
            }
            // submit frame
            gvr::gvr_frame_submit(
                mem::transmute(&self.frame),
                self.viewport_list,
                self.synced_head_matrix,
            );
        }
    }

    fn input_state(&self) -> Vec<InputFrame> {
        if let Some(ref i) = self.input {
            vec![InputFrame {
                target_ray_origin: Some(i.state()),
                id: InputId(0),
                grip_origin: None,
                pressed: false,
            }]
        } else {
            vec![]
        }
    }
}

impl Device for GoogleVRDevice {
    fn floor_transform(&self) -> RigidTransform3D<f32, Native, Floor> {
        // GoogleVR doesn't know about the floor
        // XXXManishearth perhaps we should report a guesstimate value here
        RigidTransform3D::identity()
    }

    fn views(&self) -> Views {
        unsafe {
            let left_view = self.fetch_eye(gvr::gvr_eye::GVR_LEFT_EYE, self.left_eye_vp);
            let right_view = self.fetch_eye(gvr::gvr_eye::GVR_RIGHT_EYE, self.right_eye_vp);
            Views::Stereo(left_view, right_view)
        }
    }

    fn wait_for_animation_frame(&mut self) -> Option<Frame> {
        unsafe {
            self.acquire_frame();
        }
        let events = if self.clip_planes.recently_updated() {
            vec![FrameUpdateEvent::UpdateViews(self.views())]
        } else {
            vec![]
        };
        // Predict head matrix
        Some(Frame {
            transform: self.fetch_head_matrix(),
            inputs: self.input_state(),
            events,
        })
    }

    fn render_animation_frame(&mut self, surface: Surface) -> Surface {
        let (device, mut context) = self.surfman.take().unwrap();
        let texture_size = surface.size();
        let surface_texture = device
            .create_surface_texture(&mut context, surface)
            .unwrap();
        let texture_id = surface_texture.gl_texture();
        let texture_target = device.surface_gl_texture_target();
        self.render_layer(texture_id, texture_size, texture_target);
        self.submit_frame();
        let surface = device
            .destroy_surface_texture(&mut context, surface_texture)
            .unwrap();
        self.surfman = Some((device, context));
        surface
    }

    fn initial_inputs(&self) -> Vec<InputSource> {
        if let Some(ref i) = self.input {
            vec![InputSource {
                handedness: i.handedness(),
                id: InputId(0),
                target_ray_mode: TargetRayMode::TrackedPointer,
                supports_grip: false,
            }]
        } else {
            vec![]
        }
    }

    fn set_event_dest(&mut self, dest: Sender<Event>) {
        self.events.upgrade(dest);
    }

    fn quit(&mut self) {
        self.stop_present();
        self.events.callback(Event::SessionEnd);
    }

    fn set_quitter(&mut self, _: Quitter) {
        // do nothing for now until we need the quitter
    }

    fn update_clip_planes(&mut self, near: f32, far: f32) {
        self.clip_planes.update(near, far)
    }
}

impl Drop for GoogleVRDevice {
    fn drop(&mut self) {
        if let Some((ref device, ref mut context)) = self.surfman {
            let _ = device.destroy_context(context);
        }
    }
}

#[inline]
fn fov_to_projection_matrix<T, U>(
    fov: &gvr::gvr_rectf,
    clip_planes: ClipPlanes,
) -> Transform3D<f32, T, U> {
    let near = clip_planes.near;
    let far = clip_planes.far;
    let left = -fov.left.to_radians().tan() * near;
    let right = fov.right.to_radians().tan() * near;
    let top = fov.top.to_radians().tan() * near;
    let bottom = -fov.bottom.to_radians().tan() * near;
    Transform3D::ortho(left, right, bottom, top, near, far)
}

#[inline]
fn gvr_texture_bounds(array: &[f32; 4]) -> gvr::gvr_rectf {
    gvr::gvr_rectf {
        left: array[0],
        right: array[0] + array[2],
        bottom: array[1],
        top: array[1] + array[3],
    }
}

#[inline]
fn gvr_identity_matrix() -> gvr::gvr_mat4f {
    gvr::gvr_mat4f {
        m: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    }
}

fn decompose_rotation<T, U>(mat: &gvr::gvr_mat4f) -> Rotation3D<f32, T, U> {
    // https://math.stackexchange.com/a/3183435/24293
    let m = &mat.m;
    if m[2][2] < 0. {
        if m[0][0] > m[1][1] {
            let t = 1. + m[0][0] - m[1][1] - m[2][2];
            Rotation3D::unit_quaternion(t, m[0][1] + m[1][0], m[2][0] + m[0][2], m[1][2] - m[2][1])
        } else {
            let t = 1. - m[0][0] + m[1][1] - m[2][2];
            Rotation3D::unit_quaternion(m[0][1] + m[1][0], t, m[1][2] + m[2][1], m[2][0] - m[0][2])
        }
    } else {
        if m[0][0] < -m[1][1] {
            let t = 1. - m[0][0] - m[1][1] + m[2][2];
            Rotation3D::unit_quaternion(m[2][0] + m[0][2], m[1][2] + m[2][1], t, m[0][1] - m[1][0])
        } else {
            let t = 1. + m[0][0] + m[1][1] + m[2][2];
            Rotation3D::unit_quaternion(m[1][2] - m[2][1], m[2][0] - m[0][2], m[0][1] - m[1][0], t)
        }
    }
}

fn decompose_translation<T>(mat: &gvr::gvr_mat4f) -> Vector3D<f32, T> {
    Vector3D::new(mat.m[0][3], mat.m[1][3], mat.m[2][3])
}

fn decompose_rigid<T, U>(mat: &gvr::gvr_mat4f) -> RigidTransform3D<f32, T, U> {
    // Rigid transform matrices formed by applying a rotation first and then a translation
    // decompose cleanly based on their rotation and translation components.
    RigidTransform3D::new(decompose_rotation(mat), decompose_translation(mat))
}
