/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

#![allow(unused_imports)]

use crate::egl;
use crate::egl::types::EGLContext;

use euclid::default::Size2D as UntypedSize2D;
use euclid::Point2D;
use euclid::Rect;
use euclid::RigidTransform3D;
use euclid::Rotation3D;
use euclid::Size2D;
use euclid::Transform3D;
use euclid::Vector3D;

use gleam::gl;
use gleam::gl::GLint;
use gleam::gl::GLsizei;
use gleam::gl::GLuint;
use gleam::gl::Gl;

use log::debug;
use log::error;
use log::info;
use log::warn;

use magicleap_c_api::MLGraphicsBeginFrame;
use magicleap_c_api::MLGraphicsCreateClientGL;
use magicleap_c_api::MLGraphicsDestroyClient;
use magicleap_c_api::MLGraphicsEndFrame;
use magicleap_c_api::MLGraphicsFlags_MLGraphicsFlags_Default;
use magicleap_c_api::MLGraphicsFrameParams;
use magicleap_c_api::MLGraphicsGetRenderTargets;
use magicleap_c_api::MLGraphicsInitFrameParams;
use magicleap_c_api::MLGraphicsOptions;
use magicleap_c_api::MLGraphicsRenderBufferInfo;
use magicleap_c_api::MLGraphicsRenderTargetsInfo;
use magicleap_c_api::MLGraphicsSignalSyncObjectGL;
use magicleap_c_api::MLGraphicsVirtualCameraInfo;
use magicleap_c_api::MLGraphicsVirtualCameraInfoArray;
use magicleap_c_api::MLHandle;
use magicleap_c_api::MLHeadTrackingCreate;
use magicleap_c_api::MLHeadTrackingDestroy;
use magicleap_c_api::MLHeadTrackingGetStaticData;
use magicleap_c_api::MLHeadTrackingStaticData;
use magicleap_c_api::MLLifecycleSetReadyIndication;
use magicleap_c_api::MLPerceptionGetSnapshot;
use magicleap_c_api::MLResult;
use magicleap_c_api::MLSnapshotGetTransform;
use magicleap_c_api::MLSurfaceFormat_MLSurfaceFormat_D32Float;
use magicleap_c_api::MLSurfaceFormat_MLSurfaceFormat_RGBA8UNormSRGB;
use magicleap_c_api::MLTransform;

use std::mem;
use std::rc::Rc;
use std::thread;
use std::time::Duration;

use surfman::platform::generic::universal::context::Context as SurfmanContext;
use surfman::platform::generic::universal::device::Device as SurfmanDevice;
use surfman::platform::generic::universal::surface::Surface;

use webxr_api::Device;
use webxr_api::Discovery;
use webxr_api::Display;
use webxr_api::Error;
use webxr_api::Event;
use webxr_api::Floor;
use webxr_api::Frame;
use webxr_api::FrameUpdateEvent;
use webxr_api::InputSource;
use webxr_api::Native;
use webxr_api::Quitter;
use webxr_api::Receiver;
use webxr_api::Sender;
use webxr_api::Session;
use webxr_api::SessionBuilder;
use webxr_api::SessionMode;
use webxr_api::View;
use webxr_api::Viewer;
use webxr_api::Viewport;
use webxr_api::Views;

mod magicleap_c_api;

pub struct MagicLeapDiscovery {
    egl: EGLContext,
    gl: Rc<dyn Gl>,
}

pub struct MagicLeapDevice {
    surfman_device: SurfmanDevice,
    surfman_context: SurfmanContext,
    gl: Rc<dyn Gl>,
    read_fbo: GLuint,
    draw_fbo: GLuint,
    graphics_client: MLHandle,
    head_tracking_sdata: MLHeadTrackingStaticData,
    in_frame: bool,
    frame_handle: MLHandle,
    cameras: MLGraphicsVirtualCameraInfoArray,
    view_update_needed: bool,
}

impl MagicLeapDiscovery {
    pub fn new(egl: EGLContext, gl: Rc<dyn Gl>) -> MagicLeapDiscovery {
        MagicLeapDiscovery { egl, gl }
    }
}

impl Discovery for MagicLeapDiscovery {
    fn supports_session(&self, mode: SessionMode) -> bool {
        mode == SessionMode::ImmersiveVR || mode == SessionMode::ImmersiveAR
    }

    fn request_session(&mut self, mode: SessionMode, xr: SessionBuilder) -> Result<Session, Error> {
        if !self.supports_session(mode) {
            return Err(Error::NoMatchingDevice);
        }
        let egl = self.egl;
        let gl = self.gl.clone();
        xr.run_on_main_thread(move || match MagicLeapDevice::new(egl, gl) {
            Ok(device) => Ok(device),
            Err(err) => {
                warn!("Failed to start Magic Leap XR ({})", String::from(err));
                Err(Error::NoMatchingDevice)
            }
        })
    }
}

impl MagicLeapDevice {
    fn new(egl: EGLContext, gl: Rc<dyn Gl>) -> Result<MagicLeapDevice, MLResult> {
        info!("Creating MagicLeapDevice");

        let options = MLGraphicsOptions {
            color_format: MLSurfaceFormat_MLSurfaceFormat_RGBA8UNormSRGB,
            depth_format: MLSurfaceFormat_MLSurfaceFormat_D32Float,
            graphics_flags: MLGraphicsFlags_MLGraphicsFlags_Default,
        };
        let mut graphics_client = MLHandle::default();
        let mut head_tracking = MLHandle::default();
        let mut head_tracking_sdata = MLHeadTrackingStaticData::default();
        unsafe {
            MLGraphicsCreateClientGL(&options, egl as MLHandle, &mut graphics_client).ok()?;
            MLLifecycleSetReadyIndication().ok()?;
            MLHeadTrackingCreate(&mut head_tracking).ok()?;
            MLHeadTrackingGetStaticData(head_tracking, &mut head_tracking_sdata).ok()?;
        }

        let framebuffers = gl.gen_framebuffers(2);
        let draw_fbo = framebuffers[0];
        let read_fbo = framebuffers[1];

        let in_frame = false;
        let frame_handle = MLHandle::default();
        let cameras = MLGraphicsVirtualCameraInfoArray::default();

        let (surfman_device, surfman_context) =
            unsafe { SurfmanDevice::from_current_hardware_context() }
                .or(Err(MLResult::UnspecifiedFailure))?;

        let mut device = MagicLeapDevice {
            surfman_device,
            surfman_context,
            gl,
            graphics_client,
            head_tracking_sdata,
            draw_fbo,
            read_fbo,
            in_frame,
            frame_handle,
            cameras,
            view_update_needed: false,
        };

        // Rather annoyingly, in order for the views to be available, we have to
        // start a frame at this point.
        device.start_frame()?;

        Ok(device)
    }
}

impl MagicLeapDevice {
    fn start_frame(&mut self) -> Result<(), MLResult> {
        if !self.in_frame {
            debug!("Starting frame");
            let mut params = MLGraphicsFrameParams::default();
            unsafe { MLGraphicsInitFrameParams(&mut params).ok()? };

            let mut result = unsafe {
                MLGraphicsBeginFrame(
                    self.graphics_client,
                    &params,
                    &mut self.frame_handle,
                    &mut self.cameras,
                )
            };
            if result == MLResult::Timeout {
                debug!("MLGraphicsBeginFrame timeout");
                let mut sleep = Duration::from_millis(1);
                let max_sleep = Duration::from_secs(5);
                // TODO: give up after a while
                while result == MLResult::Timeout {
                    sleep = (sleep * 2).min(max_sleep);
                    debug!(
                        "MLGraphicsBeginFrame exponential backoff {}ms",
                        sleep.as_millis()
                    );
                    thread::sleep(sleep);
                    result = unsafe {
                        MLGraphicsBeginFrame(
                            self.graphics_client,
                            &params,
                            &mut self.frame_handle,
                            &mut self.cameras,
                        )
                    };
                }
                debug!("MLGraphicsBeginFrame finished timeout");
            }
            result.ok()?;

            let mut snapshot = unsafe { mem::zeroed() };
            unsafe { MLPerceptionGetSnapshot(&mut snapshot).ok()? };

            let mut transform = MLTransform::default();
            unsafe {
                MLSnapshotGetTransform(
                    snapshot,
                    &self.head_tracking_sdata.coord_frame_head,
                    &mut transform,
                )
                .ok()?
            };

            debug!("Started frame");
            self.in_frame = true;
        }
        Ok(())
    }

    // Approximate the viewer transform by linear interpolation of the two eyes
    fn lerp_transforms(&self) -> RigidTransform3D<f32, Viewer, Native> {
        let transform_0 = self.transform(0);
        let transform_1 = self.transform(1);

        let rotation = transform_0.rotation.lerp(&transform_1.rotation, 0.5);
        let translation = transform_0.translation.lerp(transform_1.translation, 0.5);

        RigidTransform3D::new(rotation, translation)
    }

    fn transform<Eye>(&self, index: usize) -> RigidTransform3D<f32, Eye, Native> {
        let quat = unsafe {
            self.cameras.virtual_cameras[index]
                .transform
                .rotation
                .__bindgen_anon_1
                .values
        };
        let rotation = Rotation3D::quaternion(quat[0], quat[1], quat[2], quat[3]);

        let pos = unsafe {
            self.cameras.virtual_cameras[index]
                .transform
                .position
                .__bindgen_anon_1
                .values
        };
        let translation = Vector3D::new(pos[0], pos[1], pos[2]);

        RigidTransform3D::new(rotation, translation)
    }

    fn projection<Eye>(&self, index: usize) -> Transform3D<f32, Eye, Display> {
        Transform3D::from_array(
            self.cameras.virtual_cameras[index]
                .projection
                .matrix_colmajor,
        )
    }

    fn viewport(&self, index: usize) -> Rect<i32, Viewport> {
        let width = self.cameras.viewport.w as i32;
        let height = self.cameras.viewport.h as i32;
        let origin = Point2D::new(0, (index as i32) * width);
        let size = Size2D::new(width, height);
        Rect::new(origin, size)
    }

    fn stop_frame(
        &mut self,
        texture_id: GLuint,
        size: UntypedSize2D<GLsizei>,
    ) -> Result<(), MLResult> {
        if self.in_frame {
            debug!("Stopping frame");

            let mut current_fbos = [0, 0];
            unsafe {
                self.gl
                    .get_integer_v(gl::DRAW_FRAMEBUFFER_BINDING, &mut current_fbos[0..])
            };
            unsafe {
                self.gl
                    .get_integer_v(gl::READ_FRAMEBUFFER_BINDING, &mut current_fbos[1..])
            };

            self.gl
                .bind_framebuffer(gl::READ_FRAMEBUFFER, self.read_fbo);
            self.gl
                .bind_framebuffer(gl::DRAW_FRAMEBUFFER, self.draw_fbo);
            self.gl.framebuffer_texture_2d(
                gl::READ_FRAMEBUFFER,
                gl::COLOR_ATTACHMENT0,
                gl::TEXTURE_2D,
                texture_id,
                0,
            );

            let color_id = self.cameras.color_id as GLuint;
            let depth_id = self.cameras.depth_id as GLuint;
            let draw = self.cameras.viewport;
            let (draw_x, draw_y, draw_w, draw_h) = (
                draw.x as GLint,
                draw.y as GLint,
                draw.w as GLint,
                draw.h as GLint,
            );

            for i in 0..self.cameras.num_virtual_cameras {
                let viewport = self.viewport(i as usize);
                let read_x = viewport.origin.x;
                let read_y = viewport.origin.y;
                let read_w = viewport.size.width;
                let read_h = viewport.size.height;
                let camera = &self.cameras.virtual_cameras[i as usize];
                let layer_id = camera.virtual_camera_name;
                self.gl.framebuffer_texture_layer(
                    gl::DRAW_FRAMEBUFFER,
                    gl::COLOR_ATTACHMENT0,
                    color_id,
                    0,
                    layer_id,
                );
                self.gl.framebuffer_texture_layer(
                    gl::DRAW_FRAMEBUFFER,
                    gl::DEPTH_ATTACHMENT,
                    depth_id,
                    0,
                    layer_id,
                );
                self.gl.viewport(draw_x, draw_y, draw_w, draw_h);
                if ((read_x + read_w) <= size.width) && ((read_y + read_h) <= size.height) {
                    self.gl.blit_framebuffer(
                        read_x,
                        read_y,
                        read_x + read_w,
                        read_y + read_h,
                        draw_x,
                        draw_y,
                        draw_x + draw_w,
                        draw_y + draw_h,
                        gl::COLOR_BUFFER_BIT,
                        gl::LINEAR,
                    );
                }
                unsafe {
                    MLGraphicsSignalSyncObjectGL(self.graphics_client, camera.sync_object).ok()?
                };
            }

            unsafe { MLGraphicsEndFrame(self.graphics_client, self.frame_handle).ok()? };

            self.gl
                .bind_framebuffer(gl::DRAW_FRAMEBUFFER, current_fbos[0] as GLuint);
            self.gl
                .bind_framebuffer(gl::READ_FRAMEBUFFER, current_fbos[1] as GLuint);

            debug!("Stopped frame");
            self.in_frame = false;
        }

        Ok(())
    }
}

impl Device for MagicLeapDevice {
    fn wait_for_animation_frame(&mut self) -> Option<Frame> {
        if let Err(err) = self.start_frame() {
            error!("Failed to start frame ({:?}).", err);
        }

        let transform = self.lerp_transforms();
        let inputs = Vec::new();
        let events = if self.view_update_needed {
            vec![FrameUpdateEvent::UpdateViews(self.views())]
        } else {
            vec![]
        };
        Some(Frame {
            transform,
            inputs,
            events,
        })
    }

    fn render_animation_frame(&mut self, surface: Surface) -> Surface {
        self.surfman_device
            .make_context_current(&self.surfman_context);
        debug_assert_eq!(self.gl.get_error(), gl::NO_ERROR);

        let size = surface.size();
        let surface_texture = self
            .surfman_device
            .create_surface_texture(&mut self.surfman_context, surface)
            .unwrap();
        let texture_id = surface_texture.gl_texture();

        if let Err(err) = self.stop_frame(texture_id, size) {
            error!("Failed to stop frame ({:?}).", err);
        }

        self.surfman_device
            .destroy_surface_texture(&mut self.surfman_context, surface_texture)
            .unwrap()
    }

    fn views(&self) -> Views {
        let lerped = self.lerp_transforms();
        let left = View {
            transform: self.transform(0).inverse().pre_transform(&lerped),
            projection: self.projection(0),
            viewport: self.viewport(0),
        };
        let right = View {
            transform: self.transform(1).inverse().pre_transform(&lerped),
            projection: self.projection(1),
            viewport: self.viewport(1),
        };
        Views::Stereo(left, right)
    }

    fn floor_transform(&self) -> RigidTransform3D<f32, Native, Floor> {
        // TODO: get this from the device
        RigidTransform3D::from_translation(Vector3D::new(0.0, -1.0, 0.0))
    }

    fn initial_inputs(&self) -> Vec<InputSource> {
        Vec::new()
    }

    fn set_event_dest(&mut self, _dest: Sender<Event>) {
        // TODO: handle events
    }

    fn quit(&mut self) {
        // TODO: handle quit
    }

    fn set_quitter(&mut self, _quitter: Quitter) {
        // TODO: handle quit
    }

    fn update_clip_planes(&mut self, _near: f32, _far: f32) {
        self.view_update_needed = true;
        // XXXManishearth tell the device about the new clip planes
    }
}
