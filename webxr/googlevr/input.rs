/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use gvr_sys as gvr;
use gvr_sys::gvr_controller_api_status::*;
use gvr_sys::gvr_controller_handedness::*;

use euclid::RigidTransform3D;
use euclid::Rotation3D;
use std::ffi::CStr;
use std::mem;
use webxr_api::Handedness;
use webxr_api::Input;
use webxr_api::Native;

pub struct GoogleVRController {
    ctx: *mut gvr::gvr_context,
    controller_ctx: *mut gvr::gvr_controller_context,
    state: *mut gvr::gvr_controller_state,
}

impl GoogleVRController {
    pub unsafe fn new(
        ctx: *mut gvr::gvr_context,
        controller_ctx: *mut gvr::gvr_controller_context,
    ) -> Result<Self, String> {
        let gamepad = Self {
            ctx: ctx,
            controller_ctx: controller_ctx,
            state: gvr::gvr_controller_state_create(),
        };
        gvr::gvr_controller_state_update(controller_ctx, 0, gamepad.state);
        let api_status = gvr::gvr_controller_state_get_api_status(gamepad.state);
        if api_status != GVR_CONTROLLER_API_OK as i32 {
            let message = CStr::from_ptr(gvr::gvr_controller_api_status_to_string(api_status));
            return Err(message.to_string_lossy().into());
        }

        Ok(gamepad)
    }

    pub fn handedness(&self) -> Handedness {
        let handeness = unsafe {
            let prefs = gvr::gvr_get_user_prefs(self.ctx);
            gvr::gvr_user_prefs_get_controller_handedness(prefs)
        };
        if handeness == GVR_CONTROLLER_LEFT_HANDED as i32 {
            Handedness::Left
        } else {
            Handedness::Right
        }
    }

    pub fn state(&self) -> RigidTransform3D<f32, Input, Native> {
        unsafe {
            gvr::gvr_controller_state_update(self.controller_ctx, 0, self.state);
            let quat = gvr::gvr_controller_state_get_orientation(self.state);
            Rotation3D::unit_quaternion(quat.qx, quat.qy, quat.qz, quat.qw).into()
        }
    }
}

impl Drop for GoogleVRController {
    fn drop(&mut self) {
        unsafe {
            gvr::gvr_controller_state_destroy(mem::transmute(&self.state));
        }
    }
}
