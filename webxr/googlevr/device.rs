use gleam::gl::GLsync;

use webxr_api::Device;
use webxr_api::Error;
use webxr_api::Event;
use webxr_api::EventBuffer;
use webxr_api::Floor;
use webxr_api::Frame;
use webxr_api::InputSource;
use webxr_api::Native;
use webxr_api::Sender;
use webxr_api::Quitter;
use webxr_api::Views;

use euclid::default::Size2D;
use euclid::RigidTransform3D;

use gvr_sys as gvr;
use std::ptr;

use super::discovery::SendPtr;

#[cfg(target_os = "android")]
use android_injected_glue::ffi as ndk;

pub(crate) struct GoogleVRDevice {
    events: EventBuffer,

    #[cfg(target_os = "android")]
    java_class: ndk::jclass,
    ctx: *mut gvr::gvr_context,
    controller_ctx: *mut gvr::gvr_controller_context,
    viewport_list: *mut gvr::gvr_buffer_viewport_list,
    left_eye_vp: *mut gvr::gvr_buffer_viewport,
    right_eye_vp: *mut gvr::gvr_buffer_viewport,
    render_size: gvr::gvr_sizei,
    swap_chain: *mut gvr::gvr_swap_chain,
    frame: *mut gvr::gvr_frame,

}

impl GoogleVRDevice {
    #[cfg(target_os = "android")]
    pub fn new(ctx: SendPtr<*mut gvr::gvr_context>, controller_ctx: SendPtr<*mut gvr::gvr_controller_context>, java_class: SendPtr<ndk::jclass>) -> Result<Self, Error> {
        let mut device = GoogleVRDevice {
            events: Default::default(),
            ctx: ctx.get(), controller_ctx: controller_ctx.get(), java_class: java_class.get(),
            viewport_list: ptr::null_mut(),
            left_eye_vp: ptr::null_mut(),
            right_eye_vp: ptr::null_mut(),
            render_size: gvr::gvr_sizei {
                width: 0,
                height: 0,
            },
            swap_chain: ptr::null_mut(),
            frame: ptr::null_mut(),
        };
        device.init();
        Ok(device)
    }

    #[cfg(not(target_os = "android"))]
    pub fn new(ctx: SendPtr<*mut gvr::gvr_context>, controller_ctx: SendPtr<*mut gvr::gvr_controller_context>) -> Result<Self, Error> {
        let mut device = GoogleVRDevice {
            events: Default::default(),
            ctx: ctx.get(), controller_ctx: controller_ctx.get(), 
            viewport_list: ptr::null_mut(),
            left_eye_vp: ptr::null_mut(),
            right_eye_vp: ptr::null_mut(),
            render_size: gvr::gvr_sizei {
                width: 0,
                height: 0,
            },
            swap_chain: ptr::null_mut(),
            frame: ptr::null_mut(),
        };
        device.init();
        Ok(device)
    }


    fn init(&mut self) {
        unsafe {
            let list = gvr::gvr_buffer_viewport_list_create(self.ctx);

            // gvr_refresh_viewer_profile must be called before getting recommended bufer viewports.
            gvr::gvr_refresh_viewer_profile(self.ctx);

            // Gets the recommended buffer viewport configuration, populating a previously
            // allocated gvr_buffer_viewport_list object. The updated values include the
            // per-eye recommended viewport and field of view for the target.
            gvr::gvr_get_recommended_buffer_viewports(self.ctx, list);

            // Create viewport buffers for both eyes.
            self.left_eye_vp = gvr::gvr_buffer_viewport_create(self.ctx);
            gvr::gvr_buffer_viewport_list_get_item(list, gvr::gvr_eye::GVR_LEFT_EYE as usize, self.left_eye_vp);
            self.right_eye_vp = gvr::gvr_buffer_viewport_create(self.ctx);
            gvr::gvr_buffer_viewport_list_get_item(list, gvr::gvr_eye::GVR_RIGHT_EYE as usize, self.right_eye_vp);

            // The following fields are expected to be null at initialization
            // self.frame = ptr::null_mut();
            // self.swap_chain = ptr::null_mut();
        }
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
        self.events.callback(Event::SessionEnd);
    }

    fn set_quitter(&mut self, _: Quitter) {
        // do nothing for now until we need the quitter
    }
}
