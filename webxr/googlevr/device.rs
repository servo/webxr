use gleam::gl::GLsync;

use webxr_api::Device;
use webxr_api::Error;
use webxr_api::Event;
use webxr_api::EventBuffer;
use webxr_api::Floor;
use webxr_api::Frame;
use webxr_api::InputSource;
use webxr_api::LeftEye;
use webxr_api::Native;
use webxr_api::Quitter;
use webxr_api::RightEye;
use webxr_api::Sender;
use webxr_api::View;
use webxr_api::Viewer;
use webxr_api::Views;

use crate::gl;

use euclid::default::Size2D as DefaultSize2D;
use euclid::Point2D;
use euclid::Rect;
use euclid::RigidTransform3D;
use euclid::Size2D;
use euclid::Transform3D;
use euclid::Vector3D;

use gvr_sys as gvr;
use gvr_sys::gvr_color_format_type::*;
use gvr_sys::gvr_depth_stencil_format_type::*;
use gvr_sys::gvr_feature::*;
use std::{mem, ptr};

use super::discovery::SendPtr;

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
    left_view: View<LeftEye>,
    right_view: View<RightEye>,
    near: f32,
    far: f32,

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
    presenting: bool,
    frame_bound: bool,
}

fn empty_view<T>() -> View<T> {
    View {
        transform: RigidTransform3D::identity(),
        projection: Transform3D::identity(),
        viewport: Default::default(),
    }
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
            left_view: empty_view(),
            right_view: empty_view(),
            // https://github.com/servo/webxr/issues/32
            near: 0.1,
            far: 1000.0,

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
            presenting: false,
            frame_bound: false,
        };
        unsafe {
            device.init();
        }
        // XXXManishearth figure out how to block until presentation
        // starts
        device.start_present();
        device.initialize_views();
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
            left_view: empty_view(),
            right_view: empty_view(),
            // https://github.com/servo/webxr/issues/32
            near: 0.1,
            far: 1000.0,

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
            presenting: false,
            frame_bound: false,
        };
        unsafe {
            device.init();
        }
        // XXXManishearth figure out how to block until presentation
        // starts
        device.start_present();
        device.initialize_views();
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
    }

    unsafe fn initialize_gl(&mut self) {
        // Note: In some scenarios gvr_initialize_gl crashes if gvr_refresh_viewer_profile call isn't called before.
        gvr::gvr_refresh_viewer_profile(self.ctx);
        // Initializes gvr necessary GL-related objects.
        gvr::gvr_initialize_gl(self.ctx);

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

    fn initialize_views(&mut self) {
        unsafe {
            self.left_view = self.fetch_eye(gvr::gvr_eye::GVR_LEFT_EYE, self.left_eye_vp);
            self.right_view = self.fetch_eye(gvr::gvr_eye::GVR_RIGHT_EYE, self.right_eye_vp);
        }
    }

    unsafe fn fetch_eye<T>(&self, eye: gvr::gvr_eye, vp: *mut gvr::gvr_buffer_viewport) -> View<T> {
        let eye_fov = gvr::gvr_buffer_viewport_get_source_fov(vp);
        let projection = fov_to_projection_matrix(&eye_fov, self.near, self.far);

        // this matrix converts from head space to eye space,
        // i.e. it's the inverse of the offset
        let eye_mat = gvr::gvr_get_eye_from_head_matrix(self.ctx, eye as i32);
        // XXXManishearth we should decompose the matrix properly instead of assuming it's
        // only translation
        let transform = Vector3D::new(-eye_mat.m[0][3], -eye_mat.m[1][3], -eye_mat.m[2][3]).into();

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
        unimplemented!("need to decompose matrix")
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
}

impl Device for GoogleVRDevice {
    fn floor_transform(&self) -> RigidTransform3D<f32, Native, Floor> {
        // GoogleVR doesn't know about the floor
        // XXXManishearth perhaps we should report a guesstimate value here
        RigidTransform3D::identity()
    }

    fn views(&self) -> Views {
        Views::Stereo(self.left_view.clone(), self.right_view.clone())
    }

    fn wait_for_animation_frame(&mut self) -> Frame {
        unsafe {
            self.acquire_frame();
        }
        // Predict head matrix
        Frame {
            transform: self.fetch_head_matrix(),
            inputs: vec![],
        }
    }

    fn render_animation_frame(
        &mut self,
        _texture_id: u32,
        _size: DefaultSize2D<i32>,
        _sync: GLsync,
    ) {
        unimplemented!()
    }

    fn initial_inputs(&self) -> Vec<InputSource> {
        vec![]
    }

    fn set_event_dest(&mut self, _dest: Sender<Event>) {
        unimplemented!()
    }

    fn quit(&mut self) {
        self.stop_present();
        self.events.callback(Event::SessionEnd);
    }

    fn set_quitter(&mut self, _: Quitter) {
        // do nothing for now until we need the quitter
    }
}

#[inline]
fn fov_to_projection_matrix<T, U>(
    fov: &gvr::gvr_rectf,
    near: f32,
    far: f32,
) -> Transform3D<f32, T, U> {
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
