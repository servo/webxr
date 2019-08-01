use euclid::default::Size2D as UntypedSize2D;
use euclid::Point2D;
use euclid::Rect;
use euclid::RigidTransform3D;
use euclid::Rotation3D;
use euclid::Size2D;
use euclid::Transform3D;
use euclid::Vector3D;
use gleam::gl::{self, GLsync, GLuint, Gl};
use openxr::d3d::{Requirements, SessionCreateInfo, D3D11};
use openxr::sys::platform::ID3D11Device;
use openxr::{
    self, ApplicationInfo, CompositionLayerProjection, Entry, EnvironmentBlendMode, ExtensionSet,
    Extent2Di, FormFactor, Fovf, FrameState, FrameStream, FrameWaiter, Graphics, Instance, Posef,
    Quaternionf, ReferenceSpaceType, Session, Space, Swapchain, SwapchainCreateFlags,
    SwapchainCreateInfo, SwapchainUsageFlags, Vector3f, ViewConfigurationType,
};
use std::rc::Rc;
use std::{mem, ptr};
use webxr_api::Device;
use webxr_api::Discovery;
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
use webxr_api::Session as WebXrSession;
use webxr_api::SessionBuilder;
use webxr_api::SessionMode;
use webxr_api::View;
use webxr_api::Viewer;
use webxr_api::Views;
use winapi::shared::dxgi;
use winapi::shared::winerror::{DXGI_ERROR_NOT_FOUND, S_OK};
use winapi::um::d3d11;
use winapi::um::d3dcommon::*;
use winapi::Interface;
use wio::com::ComPtr;

const HEIGHT: f32 = 1.0;

pub struct OpenXrDiscovery {
    gl: Rc<dyn Gl>,
}

impl OpenXrDiscovery {
    pub fn new(gl: Rc<dyn Gl>) -> Self {
        Self { gl }
    }
}

fn create_instance() -> Result<Instance, String> {
    let entry = Entry::load().map_err(|e| format!("{:?}", e))?;

    let app_info = ApplicationInfo {
        application_name: "webvr",
        ..Default::default()
    };

    let exts = ExtensionSet {
        khr_d3d11_enable: true,
        ..Default::default()
    };

    entry
        .create_instance(&app_info, &exts)
        .map_err(|e| format!("{:?}", e))
}

impl Discovery for OpenXrDiscovery {
    fn request_session(
        &mut self,
        mode: SessionMode,
        xr: SessionBuilder,
    ) -> Result<WebXrSession, Error> {
        let instance = create_instance().map_err(|e| Error::BackendSpecific(e))?;

        if self.supports_session(mode) {
            let gl = self.gl.clone();
            xr.run_on_main_thread(move || OpenXrDevice::new(gl, instance))
        } else {
            Err(Error::NoMatchingDevice)
        }
    }

    fn supports_session(&self, mode: SessionMode) -> bool {
        mode == SessionMode::ImmersiveAR
    }
}

struct OpenXrDevice {
    #[allow(unused)]
    gl: Rc<dyn Gl>,
    #[allow(unused)]
    read_fbo: GLuint,
    events: EventBuffer,
    session: Session<D3D11>,
    frame_waiter: FrameWaiter,
    frame_stream: FrameStream<D3D11>,
    frame_state: FrameState,
    space: Space,
    openxr_views: Vec<openxr::View>,
    left_extent: Extent2Di,
    right_extent: Extent2Di,
    left_view: View<LeftEye>,
    right_view: View<RightEye>,
    left_swapchain: Swapchain<D3D11>,
    left_image: u32,
    right_swapchain: Swapchain<D3D11>,
    right_image: u32,
}

impl OpenXrDevice {
    fn new(gl: Rc<dyn Gl>, instance: Instance) -> Result<OpenXrDevice, Error> {
        let read_fbo = gl.gen_framebuffers(1)[0];
        debug_assert_eq!(gl.get_error(), gl::NO_ERROR);

        let system = instance
            .system(FormFactor::HEAD_MOUNTED_DISPLAY)
            .map_err(|e| Error::BackendSpecific(format!("{:?}", e)))?;

        let requirements = D3D11::requirements(&instance, system)
            .map_err(|e| Error::BackendSpecific(format!("{:?}", e)))?;
        let adapter = get_matching_adapter(&requirements).map_err(|e| Error::BackendSpecific(e))?;
        let feature_levels = select_feature_levels(&requirements);
        let (device, _device_context) = init_device_for_adapter(adapter, &feature_levels)
            .map_err(|e| Error::BackendSpecific(format!("{:?}", e)))?;

        let (session, mut frame_waiter, frame_stream) = unsafe {
            instance
                .create_session::<D3D11>(
                    system,
                    &SessionCreateInfo {
                        device: device.as_raw(),
                    },
                )
                .map_err(|e| Error::BackendSpecific(format!("{:?}", e)))?
        };

        session
            .begin(ViewConfigurationType::PRIMARY_STEREO)
            .map_err(|e| Error::BackendSpecific(format!("{:?}", e)))?;

        let ref_space_type = ReferenceSpaceType::LOCAL;
        let pose = Posef {
            orientation: Quaternionf {
                x: 0.,
                y: 0.,
                z: 0.,
                w: 1.,
            },
            position: Vector3f {
                x: 0.,
                y: 0.,
                z: 0.,
            },
        };
        let space = session
            .create_reference_space(ref_space_type, pose)
            .map_err(|e| Error::BackendSpecific(format!("{:?}", e)))?;

        let view_configuration_views = instance
            .enumerate_view_configuration_views(system, ViewConfigurationType::PRIMARY_STEREO)
            .map_err(|e| Error::BackendSpecific(format!("{:?}", e)))?;

        let left_view_configuration = view_configuration_views[0];
        let right_view_configuration = view_configuration_views[1];
        let left_extent = Extent2Di {
            width: left_view_configuration.recommended_image_rect_width as i32,
            height: left_view_configuration.recommended_image_rect_height as i32,
        };
        let right_extent = Extent2Di {
            width: right_view_configuration.recommended_image_rect_width as i32,
            height: right_view_configuration.recommended_image_rect_height as i32,
        };

        // https://github.com/servo/webxr/issues/32
        let near = 0.1;
        let far = 1000.;

        // Obtain view info
        let frame_state = frame_waiter.wait().expect("error waiting for frame");
        let (_view_flags, views) = session
            .locate_views(
                ViewConfigurationType::PRIMARY_STEREO,
                frame_state.predicted_display_time,
                &space,
            )
            .expect("error locating views");

        let lerped = lerp_transforms(&views[0].pose, &views[1].pose);
        let left_vp = Rect::new(
            Point2D::zero(),
            Size2D::new(
                left_view_configuration.recommended_image_rect_width as i32,
                left_view_configuration.recommended_image_rect_height as i32,
            ),
        );
        let right_vp = Rect::new(
            Point2D::new(
                left_view_configuration.recommended_image_rect_width as i32,
                0,
            ),
            Size2D::new(
                right_view_configuration.recommended_image_rect_width as i32,
                right_view_configuration.recommended_image_rect_height as i32,
            ),
        );
        let left_view = View {
            transform: transform(&views[0].pose).inverse().pre_transform(&lerped),
            projection: fov_to_projection_matrix(&views[0].fov, near, far),
            viewport: left_vp,
        };
        let right_view = View {
            transform: transform(&views[1].pose).inverse().pre_transform(&lerped),
            projection: fov_to_projection_matrix(&views[1].fov, near, far),
            viewport: right_vp,
        };

        // Create swapchains

        // XXXManishearth should we be doing this, or letting Servo set the format?
        let format = *session
            .enumerate_swapchain_formats()
            .map_err(|e| Error::BackendSpecific(format!("{:?}", e)))?
            .get(0)
            .ok_or(Error::BackendSpecific(
                "No available swapchain formats".into(),
            ))?;

        let swapchain_create_info = SwapchainCreateInfo {
            create_flags: SwapchainCreateFlags::EMPTY,
            usage_flags: SwapchainUsageFlags::COLOR_ATTACHMENT | SwapchainUsageFlags::SAMPLED,
            format,
            sample_count: 1,
            // XXXManishearth what if the recommended widths are different?
            width: left_view_configuration.recommended_image_rect_width,
            height: left_view_configuration.recommended_image_rect_height,
            face_count: 1,
            array_size: 1,
            mip_count: 1,
        };

        let left_swapchain = session
            .create_swapchain(&swapchain_create_info)
            .map_err(|e| Error::BackendSpecific(format!("{:?}", e)))?;
        let right_swapchain = session
            .create_swapchain(&swapchain_create_info)
            .map_err(|e| Error::BackendSpecific(format!("{:?}", e)))?;

        Ok(OpenXrDevice {
            events: Default::default(),
            gl,
            read_fbo,
            session,
            frame_stream,
            frame_waiter,
            frame_state,
            space,
            left_extent,
            right_extent,
            left_image: 0,
            right_image: 0,
            openxr_views: views,
            left_view,
            right_view,
            left_swapchain,
            right_swapchain,
        })
    }
}

impl Device for OpenXrDevice {
    fn floor_transform(&self) -> RigidTransform3D<f32, Native, Floor> {
        let translation = Vector3D::new(-HEIGHT, 0.0, 0.0);
        RigidTransform3D::from_translation(translation)
    }

    fn views(&self) -> Views {
        Views::Stereo(self.left_view.clone(), self.right_view.clone())
    }

    fn wait_for_animation_frame(&mut self) -> Frame {
        self.frame_state = self.frame_waiter.wait().expect("error waiting for frame");
        // XXXManishearth should we check frame_state.should_render?
        let (_view_flags, views) = self
            .session
            .locate_views(
                ViewConfigurationType::PRIMARY_STEREO,
                self.frame_state.predicted_display_time,
                &self.space,
            )
            .expect("error locating views");
        self.openxr_views = views;
        let view = self.openxr_views[0];
        let rotation = Rotation3D::unit_quaternion(
            view.pose.orientation.x,
            view.pose.orientation.y,
            view.pose.orientation.z,
            view.pose.orientation.w,
        );
        let translation = Vector3D::new(
            view.pose.position.x,
            view.pose.position.y,
            view.pose.position.z,
        );
        let transform = RigidTransform3D::new(rotation, translation);

        self.frame_stream
            .begin()
            .expect("failed to start frame stream");

        self.left_image = self.left_swapchain.acquire_image().unwrap();
        self.left_swapchain
            .wait_image(openxr::Duration::INFINITE)
            .unwrap();
        self.right_image = self.right_swapchain.acquire_image().unwrap();
        self.right_swapchain
            .wait_image(openxr::Duration::INFINITE)
            .unwrap();
        Frame {
            transform,
            inputs: vec![],
        }
    }

    fn render_animation_frame(
        &mut self,
        _texture_id: u32,
        _size: UntypedSize2D<i32>,
        _sync: Option<GLsync>,
    ) {
        let _left_image = self.left_swapchain.enumerate_images().unwrap()[self.left_image as usize];
        let _right_image =
            self.right_swapchain.enumerate_images().unwrap()[self.right_image as usize];
        // TODO blit to left and right swapchain image
        self.left_swapchain.release_image().unwrap();
        self.right_swapchain.release_image().unwrap();

        self.frame_stream
            .end(
                self.frame_state.predicted_display_time,
                EnvironmentBlendMode::ADDITIVE,
                &[&CompositionLayerProjection::new()
                    .space(&self.space)
                    .views(&[
                        openxr::CompositionLayerProjectionView::new()
                            .pose(self.openxr_views[0].pose)
                            .fov(self.openxr_views[0].fov)
                            .sub_image(
                                // XXXManishearth is this correct?
                                openxr::SwapchainSubImage::new()
                                    .swapchain(&self.left_swapchain)
                                    .image_array_index(0)
                                    .image_rect(openxr::Rect2Di {
                                        offset: openxr::Offset2Di { x: 0, y: 0 },
                                        extent: self.left_extent,
                                    }),
                            ),
                        openxr::CompositionLayerProjectionView::new()
                            .pose(self.openxr_views[0].pose)
                            .fov(self.openxr_views[0].fov)
                            .sub_image(
                                openxr::SwapchainSubImage::new()
                                    .swapchain(&self.right_swapchain)
                                    .image_array_index(0)
                                    .image_rect(openxr::Rect2Di {
                                        offset: openxr::Offset2Di { x: 0, y: 0 },
                                        extent: self.right_extent,
                                    }),
                            ),
                    ])],
            )
            .unwrap();

        // let width = size.width as GLsizei;
        // let height = size.height as GLsizei;

        // self.gl.clear_color(0.2, 0.3, 0.3, 1.0);
        // self.gl.clear(gl::COLOR_BUFFER_BIT);
        // debug_assert_eq!(self.gl.get_error(), gl::NO_ERROR);

        // if let Some(sync) = sync {
        //     self.gl.wait_sync(sync, 0, gl::TIMEOUT_IGNORED);
        //     debug_assert_eq!(self.gl.get_error(), gl::NO_ERROR);
        // }

        // self.gl
        //     .bind_framebuffer(gl::READ_FRAMEBUFFER, self.read_fbo);
        // debug_assert_eq!(self.gl.get_error(), gl::NO_ERROR);

        // self.gl.framebuffer_texture_2d(
        //     gl::READ_FRAMEBUFFER,
        //     gl::COLOR_ATTACHMENT0,
        //     gl::TEXTURE_2D,
        //     texture_id,
        //     0,
        // );
        // debug_assert_eq!(self.gl.get_error(), gl::NO_ERROR);

        // self.gl.viewport(0, 0, width, height);
        // self.gl.blit_framebuffer(
        //     0,
        //     0,
        //     width,
        //     height,
        //     0,
        //     0,
        //     inner_size.width,
        //     inner_size.height,
        //     gl::COLOR_BUFFER_BIT,
        //     gl::NEAREST,
        // );
        // debug_assert_eq!(self.gl.get_error(), gl::NO_ERROR);
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
}

fn get_matching_adapter(
    requirements: &Requirements,
) -> Result<ComPtr<dxgi::IDXGIAdapter1>, String> {
    unsafe {
        let mut factory_ptr: *mut dxgi::IDXGIFactory1 = ptr::null_mut();
        let result = dxgi::CreateDXGIFactory1(
            &dxgi::IDXGIFactory1::uuidof(),
            &mut factory_ptr as *mut _ as *mut _,
        );
        assert_eq!(result, S_OK);
        let factory = ComPtr::from_raw(factory_ptr);

        let index = 0;
        loop {
            let mut adapter_ptr = ptr::null_mut();
            let result = factory.EnumAdapters1(index, &mut adapter_ptr);
            if result == DXGI_ERROR_NOT_FOUND {
                return Err("No matching adapter".to_owned());
            }
            assert_eq!(result, S_OK);
            let adapter = ComPtr::from_raw(adapter_ptr);
            let mut adapter_desc = mem::zeroed();
            let result = adapter.GetDesc1(&mut adapter_desc);
            assert_eq!(result, S_OK);
            let adapter_luid = &adapter_desc.AdapterLuid;
            if adapter_luid.LowPart == requirements.adapter_luid.LowPart
                && adapter_luid.HighPart == requirements.adapter_luid.HighPart
            {
                return Ok(adapter);
            }
        }
    }
}

fn select_feature_levels(requirements: &Requirements) -> Vec<D3D_FEATURE_LEVEL> {
    let levels = [
        D3D_FEATURE_LEVEL_12_1,
        D3D_FEATURE_LEVEL_12_0,
        D3D_FEATURE_LEVEL_11_1,
        D3D_FEATURE_LEVEL_11_0,
        D3D_FEATURE_LEVEL_10_1,
        D3D_FEATURE_LEVEL_10_0,
    ];
    levels
        .into_iter()
        .filter(|&&level| level >= requirements.min_feature_level)
        .map(|&level| level)
        .collect()
}

fn init_device_for_adapter(
    adapter: ComPtr<dxgi::IDXGIAdapter1>,
    feature_levels: &[D3D_FEATURE_LEVEL],
) -> Result<(ComPtr<ID3D11Device>, ComPtr<d3d11::ID3D11DeviceContext>), String> {
    let adapter = adapter.up::<dxgi::IDXGIAdapter>();
    unsafe {
        let mut device_ptr = ptr::null_mut();
        let mut device_context_ptr = ptr::null_mut();
        let hr = d3d11::D3D11CreateDevice(
            adapter.as_raw(),
            D3D_DRIVER_TYPE_UNKNOWN,
            ptr::null_mut(),
            d3d11::D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            feature_levels.as_ptr(),
            feature_levels.len() as u32,
            d3d11::D3D11_SDK_VERSION,
            &mut device_ptr,
            ptr::null_mut(),
            &mut device_context_ptr,
        );
        assert_eq!(hr, S_OK);
        let device = ComPtr::from_raw(device_ptr);
        let device_context = ComPtr::from_raw(device_context_ptr);
        Ok((device, device_context))
    }
}

fn transform<Eye>(pose: &Posef) -> RigidTransform3D<f32, Eye, Native> {
    let rotation = Rotation3D::quaternion(
        pose.orientation.x,
        pose.orientation.y,
        pose.orientation.z,
        pose.orientation.w,
    );

    let translation = Vector3D::new(pose.position.x, pose.position.y, pose.position.z);

    RigidTransform3D::new(rotation, translation)
}

// Approximate the viewer transform by linear interpolation of the two eyes
fn lerp_transforms(left: &Posef, right: &Posef) -> RigidTransform3D<f32, Viewer, Native> {
    let left = transform(left);
    let right = transform(right);

    let rotation = left.rotation.lerp(&right.rotation, 0.5);
    let translation = left.translation.lerp(right.translation, 0.5);

    RigidTransform3D::new(rotation, translation)
}

#[inline]
fn fov_to_projection_matrix<T, U>(fov: &Fovf, near: f32, far: f32) -> Transform3D<f32, T, U> {
    let left = -fov.angle_left.tan() * near;
    let right = fov.angle_right.tan() * near;
    let top = fov.angle_up.tan() * near;
    let bottom = -fov.angle_down.tan() * near;
    Transform3D::ortho(left, right, bottom, top, near, far)
}
