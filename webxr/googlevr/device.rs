use gleam::gl::GLsync;

use webxr_api::Device;
use webxr_api::Error;
use webxr_api::Event;
use webxr_api::EventBuffer;
use webxr_api::Floor;
use webxr_api::Frame;
use webxr_api::InputSource;
use webxr_api::Native;
use webxr_api::Quitter;
use webxr_api::Sender;
use webxr_api::Views;

use crate::gl;

use euclid::default::Size2D;
use euclid::RigidTransform3D;

use gvr_sys as gvr;
use gvr_sys::gvr_color_format_type::*;
use gvr_sys::gvr_depth_stencil_format_type::*;
use gvr_sys::gvr_feature::*;
use std::{mem, ptr};

use super::discovery::SendPtr;

#[cfg(target_os = "android")]
use android_injected_glue::ffi as ndk;
#[cfg(target_os = "android")]
use crate::jni_utils::JNIScope;

pub(crate) struct GoogleVRDevice {
    events: EventBuffer,
    multiview: bool,
    multisampling: bool,
    depth: bool,

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
    fbo_id: u32,
    presenting: bool,
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
            fbo_id: 0,
            presenting: false,
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
            fbo_id: 0,
            presenting: false,
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
}

impl Device for GoogleVRDevice {
    fn floor_transform(&self) -> RigidTransform3D<f32, Native, Floor> {
        unimplemented!()
    }

    fn views(&self) -> Views {
        unimplemented!()
    }

    fn wait_for_animation_frame(&mut self) -> Frame {
        unimplemented!()
    }

    fn render_animation_frame(&mut self, _texture_id: u32, _size: Size2D<i32>, _sync: GLsync) {
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
