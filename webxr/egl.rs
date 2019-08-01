/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

#![allow(non_camel_case_types, non_snake_case)]

use std::os::raw::{c_long, c_void};
pub type khronos_utime_nanoseconds_t = khronos_uint64_t;
pub type khronos_uint64_t = u64;
pub type khronos_ssize_t = c_long;
pub type EGLint = i32;
pub type EGLNativeDisplayType = *const c_void;
pub type EGLNativePixmapType = *const c_void;
pub type EGLNativeWindowType = *const c_void;
pub type NativeDisplayType = EGLNativeDisplayType;
pub type NativePixmapType = EGLNativePixmapType;
pub type NativeWindowType = EGLNativeWindowType;

include!(concat!(env!("OUT_DIR"), "/egl_bindings.rs"));
