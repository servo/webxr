use crate::SessionBuilder;
use crate::SwapChains;

use euclid::Point2D;
use euclid::Rect;
use euclid::RigidTransform3D;
use euclid::Rotation3D;
use euclid::Size2D;
use euclid::Transform3D;
use euclid::Vector3D;
use gleam::gl::{self, GLuint, Gl};
use log::warn;
use openxr::d3d::{SessionCreateInfo, D3D11};
use openxr::Graphics;
use openxr::{
    self, ActionSet, ActiveActionSet, ApplicationInfo, CompositionLayerFlags,
    CompositionLayerProjection, Entry, EnvironmentBlendMode, ExtensionSet, Extent2Di, FormFactor,
    Fovf, FrameState, FrameStream, FrameWaiter, Instance, Posef, Quaternionf, ReferenceSpaceType,
    Session, Space, Swapchain, SwapchainCreateFlags, SwapchainCreateInfo, SwapchainUsageFlags,
    Vector3f, ViewConfigurationType,
};
use std::rc::Rc;
use surfman::platform::generic::universal::context::Context as SurfmanContext;
use surfman::platform::generic::universal::device::Device as SurfmanDevice;
use surfman::platform::generic::universal::surface::Surface;
use webxr_api;
use webxr_api::util::{self, ClipPlanes};
use webxr_api::DeviceAPI;
use webxr_api::DiscoveryAPI;
use webxr_api::Error;
use webxr_api::Event;
use webxr_api::EventBuffer;
use webxr_api::Floor;
use webxr_api::Frame;
use webxr_api::FrameUpdateEvent;
use webxr_api::Handedness;
use webxr_api::InputId;
use webxr_api::InputSource;
use webxr_api::Native;
use webxr_api::Quitter;
use webxr_api::SelectKind;
use webxr_api::Sender;
use webxr_api::Session as WebXrSession;
use webxr_api::SessionMode;
use webxr_api::TargetRayMode;
use webxr_api::View;
use webxr_api::Views;
use winapi::shared::dxgiformat;

mod input;
use input::OpenXRInput;

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
        application_name: "firefox.reality",
        application_version: 1,
        engine_name: "servo",
        engine_version: 1,
    };

    let exts = ExtensionSet {
        khr_d3d11_enable: true,
        ..Default::default()
    };

    entry
        .create_instance(&app_info, &exts)
        .map_err(|e| format!("{:?}", e))
}

fn pick_format(formats: &[dxgiformat::DXGI_FORMAT]) -> dxgiformat::DXGI_FORMAT {
    // TODO: extract the format from surfman's device and pick a matching
    // valid format based on that. For now, assume that eglChooseConfig will
    // gravitate to B8G8R8A8.
    warn!("Available formats: {:?}", formats);
    for format in formats {
        match *format {
            dxgiformat::DXGI_FORMAT_B8G8R8A8_UNORM => return *format,
            //dxgiformat::DXGI_FORMAT_R8G8B8A8_UNORM => return *format,
            f => {
                warn!("Backend requested unsupported format {:?}", f);
            }
        }
    }

    panic!("No formats supported amongst {:?}", formats);
}

impl DiscoveryAPI<SwapChains> for OpenXrDiscovery {
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
        mode == SessionMode::ImmersiveAR || mode == SessionMode::ImmersiveVR
    }
}

struct OpenXrDevice {
    instance: Instance,
    gl: Rc<dyn Gl>,
    read_fbo: GLuint,
    write_fbo: GLuint,
    events: EventBuffer,
    session: Session<D3D11>,
    frame_waiter: FrameWaiter,
    frame_stream: FrameStream<D3D11>,
    frame_state: FrameState,
    space: Space,
    viewer_space: Space,
    clip_planes: ClipPlanes,
    openxr_views: Vec<openxr::View>,
    view_configurations: Vec<openxr::ViewConfigurationView>,
    left_extent: Extent2Di,
    right_extent: Extent2Di,
    left_swapchain: Swapchain<D3D11>,
    left_image: u32,
    left_images: Vec<<D3D11 as Graphics>::SwapchainImage>,
    right_swapchain: Swapchain<D3D11>,
    right_image: u32,
    right_images: Vec<<D3D11 as Graphics>::SwapchainImage>,
    surfman: (SurfmanDevice, SurfmanContext),

    // input
    action_set: ActionSet,
    right_hand: OpenXRInput,
    left_hand: OpenXRInput,
}

struct AutoDestroyContext {
    surfman: Option<(SurfmanDevice, SurfmanContext)>,
}

impl AutoDestroyContext {
    fn new(surfman: (SurfmanDevice, SurfmanContext)) -> AutoDestroyContext {
        AutoDestroyContext {
            surfman: Some(surfman),
        }
    }

    fn extract(mut self) -> (SurfmanDevice, SurfmanContext) {
        self.surfman.take().unwrap()
    }
}

impl Drop for AutoDestroyContext {
    fn drop(&mut self) {
        if let Some((device, mut context)) = self.surfman.take() {
            let _ = device.destroy_context(&mut context);
        }
    }
}

impl OpenXrDevice {
    fn new(gl: Rc<dyn Gl>, instance: Instance) -> Result<OpenXrDevice, Error> {
        let read_fbo = gl.gen_framebuffers(1)[0];
        debug_assert_eq!(gl.get_error(), gl::NO_ERROR);

        let write_fbo = gl.gen_framebuffers(1)[0];
        debug_assert_eq!(gl.get_error(), gl::NO_ERROR);

        let system = instance
            .system(FormFactor::HEAD_MOUNTED_DISPLAY)
            .map_err(|e| Error::BackendSpecific(format!("{:?}", e)))?;

        // FIXME: we should be using these graphics requirements to drive the actual
        //        d3d device creation, rather than assuming the device that surfman
        //        already created is appropriate. OpenXR returns a validation error
        //        unless we call this method, so we call it and ignore the results
        //        in the short term.
        let _requirements = D3D11::requirements(&instance, system)
            .map_err(|e| Error::BackendSpecific(format!("{:?}", e)))?;

        // Get the current surfman device and extract it's D3D device. This will ensure
        // that the OpenXR runtime's texture will be shareable with surfman's surfaces.
        let surfman = unsafe {
            SurfmanDevice::from_current_context().expect("Failed to create graphics context!")
        };
        let device = surfman.0.d3d11_device();
        let surfman = AutoDestroyContext::new(surfman);

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

        // XXXPaul initialisation should happen on SessionStateChanged(Ready)?

        session
            .begin(ViewConfigurationType::PRIMARY_STEREO)
            .map_err(|e| Error::BackendSpecific(format!("{:?}", e)))?;

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
            .create_reference_space(ReferenceSpaceType::LOCAL, pose)
            .map_err(|e| Error::BackendSpecific(format!("{:?}", e)))?;

        let viewer_space = session
            .create_reference_space(ReferenceSpaceType::VIEW, pose)
            .map_err(|e| Error::BackendSpecific(format!("{:?}", e)))?;

        let view_configurations = instance
            .enumerate_view_configuration_views(system, ViewConfigurationType::PRIMARY_STEREO)
            .map_err(|e| Error::BackendSpecific(format!("{:?}", e)))?;

        let left_view_configuration = view_configurations[0];
        let right_view_configuration = view_configurations[1];
        assert_eq!(
            left_view_configuration.recommended_image_rect_width,
            right_view_configuration.recommended_image_rect_width,
        );
        assert_eq!(
            left_view_configuration.recommended_image_rect_height,
            right_view_configuration.recommended_image_rect_height,
        );
        assert_eq!(
            left_view_configuration.recommended_swapchain_sample_count,
            right_view_configuration.recommended_swapchain_sample_count,
        );

        let left_extent = Extent2Di {
            width: left_view_configuration.recommended_image_rect_width as i32,
            height: left_view_configuration.recommended_image_rect_height as i32,
        };
        let right_extent = Extent2Di {
            width: right_view_configuration.recommended_image_rect_width as i32,
            height: right_view_configuration.recommended_image_rect_height as i32,
        };

        // Obtain view info
        let frame_state = frame_waiter.wait().expect("error waiting for frame");

        // Create swapchains

        // XXXManishearth should we be doing this, or letting Servo set the format?
        let formats = session
            .enumerate_swapchain_formats()
            .map_err(|e| Error::BackendSpecific(format!("{:?}", e)))?;
        let format = pick_format(&formats);
        let swapchain_create_info = SwapchainCreateInfo {
            create_flags: SwapchainCreateFlags::EMPTY,
            usage_flags: SwapchainUsageFlags::COLOR_ATTACHMENT | SwapchainUsageFlags::SAMPLED,
            format,
            sample_count: left_view_configuration.recommended_swapchain_sample_count,
            width: left_view_configuration.recommended_image_rect_width,
            height: left_view_configuration.recommended_image_rect_height,
            face_count: 1,
            array_size: 1,
            mip_count: 1,
        };

        let left_swapchain = session
            .create_swapchain(&swapchain_create_info)
            .map_err(|e| Error::BackendSpecific(format!("{:?}", e)))?;
        let left_images = left_swapchain
            .enumerate_images()
            .map_err(|e| Error::BackendSpecific(format!("{:?}", e)))?;
        let right_swapchain = session
            .create_swapchain(&swapchain_create_info)
            .map_err(|e| Error::BackendSpecific(format!("{:?}", e)))?;
        let right_images = right_swapchain
            .enumerate_images()
            .map_err(|e| Error::BackendSpecific(format!("{:?}", e)))?;

        // input

        let action_set = instance.create_action_set("hands", "Hands", 0).unwrap();
        let right_hand = OpenXRInput::new(InputId(0), Handedness::Right, &action_set, &session);
        let left_hand = OpenXRInput::new(InputId(1), Handedness::Left, &action_set, &session);
        let mut bindings = right_hand.get_bindings(&instance);
        bindings.extend(left_hand.get_bindings(&instance).into_iter());
        let path_controller = instance
            .string_to_path("/interaction_profiles/khr/simple_controller")
            .unwrap();
        instance
            .suggest_interaction_profile_bindings(path_controller, &bindings)
            .unwrap();
        session.attach_action_sets(&[&action_set]).unwrap();

        Ok(OpenXrDevice {
            instance,
            events: Default::default(),
            gl,
            read_fbo,
            write_fbo,
            session,
            frame_stream,
            frame_waiter,
            frame_state,
            space,
            viewer_space,
            clip_planes: Default::default(),
            left_extent,
            right_extent,
            openxr_views: vec![],
            view_configurations,
            left_swapchain,
            right_swapchain,
            left_images,
            right_images,
            left_image: 0,
            right_image: 0,
            surfman: surfman.extract(),

            action_set,
            right_hand,
            left_hand,
        })
    }

    fn handle_openxr_events(&mut self) -> bool {
        use openxr::Event::*;
        loop {
            let mut buffer = openxr::EventDataBuffer::new();
            let event = self.instance.poll_event(&mut buffer).unwrap();
            match event {
                Some(SessionStateChanged(session_change)) => match session_change.state() {
                    openxr::SessionState::EXITING | openxr::SessionState::LOSS_PENDING => {
                        break;
                    }
                    _ => {
                        // FIXME: Handle other states
                    }
                },
                Some(InstanceLossPending(_)) => {
                    break;
                }
                Some(_) => {
                    // FIXME: Handle other events
                }
                None => {
                    // No more events to process
                    break;
                }
            }
        }
        true
    }
}

impl DeviceAPI<Surface> for OpenXrDevice {
    fn floor_transform(&self) -> Option<RigidTransform3D<f32, Native, Floor>> {
        let translation = Vector3D::new(HEIGHT, 0.0, 0.0);
        Some(RigidTransform3D::from_translation(translation))
    }

    fn views(&self) -> Views {
        let left_view_configuration = &self.view_configurations[0];
        let right_view_configuration = &self.view_configurations[1];
        let (_view_flags, views) = self
            .session
            .locate_views(
                ViewConfigurationType::PRIMARY_STEREO,
                self.frame_state.predicted_display_time,
                &self.viewer_space,
            )
            .expect("error locating views");
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
            transform: transform(&views[0].pose).inverse(),
            projection: fov_to_projection_matrix(&views[0].fov, self.clip_planes),
            viewport: left_vp,
        };
        let right_view = View {
            transform: transform(&views[1].pose).inverse(),
            projection: fov_to_projection_matrix(&views[1].fov, self.clip_planes),
            viewport: right_vp,
        };

        Views::Stereo(left_view, right_view)
    }

    fn wait_for_animation_frame(&mut self) -> Option<Frame> {
        if !self.handle_openxr_events() {
            // Session is not running anymore.
            return None;
        }
        self.frame_state = self.frame_waiter.wait().expect("error waiting for frame");
        let time_ns = time::precise_time_ns();
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
        let pose = self
            .viewer_space
            .locate(&self.space, self.frame_state.predicted_display_time)
            .unwrap();
        let transform = Some(transform(&pose.pose));
        let events = if self.clip_planes.recently_updated() {
            vec![FrameUpdateEvent::UpdateViews(self.views())]
        } else {
            vec![]
        };

        let active_action_set = ActiveActionSet::new(&self.action_set);

        self.session.sync_actions(&[active_action_set]).unwrap();

        let (right_input_frame, right_select) =
            self.right_hand
                .frame(&self.session, &self.frame_state, &self.space);
        let (left_input_frame, left_select) =
            self.left_hand
                .frame(&self.session, &self.frame_state, &self.space);

        let frame = Frame {
            transform,
            inputs: vec![right_input_frame, left_input_frame],
            events,
            time_ns,
        };

        if let Some(right_select) = right_select {
            self.events.callback(Event::Select(
                InputId(0),
                SelectKind::Select,
                right_select,
                frame.clone(),
            ));
        }
        if let Some(left_select) = left_select {
            self.events.callback(Event::Select(
                InputId(1),
                SelectKind::Select,
                left_select,
                frame.clone(),
            ));
        }

        // todo use pose in input
        Some(frame)
    }

    fn render_animation_frame(&mut self, surface: Surface) -> Surface {
        let device = &mut self.surfman.0;
        let context = &mut self.surfman.1;
        let size = device.surface_info(&surface).size;
        let surface_texture = device.create_surface_texture(context, surface).unwrap();
        let texture_id = surface_texture.gl_texture();

        let mut value = [0];
        unsafe {
            self.gl.get_integer_v(gl::FRAMEBUFFER_BINDING, &mut value);
        }
        let old_framebuffer = value[0] as gl::GLuint;

        // Bind the completed WebXR frame to the read framebuffer.
        self.gl
            .bind_framebuffer(gl::READ_FRAMEBUFFER, self.read_fbo);
        self.gl.framebuffer_texture_2d(
            gl::READ_FRAMEBUFFER,
            gl::COLOR_ATTACHMENT0,
            device.surface_gl_texture_target(),
            texture_id,
            0,
        );

        // XXXManishearth this code should perhaps be in wait_for_animation_frame,
        // but we then get errors that wait_image was called without a release_image()
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

        let left_image = self.left_images[self.left_image as usize];
        let right_image = self.right_images[self.right_image as usize];

        let left_surface = unsafe {
            device
                .create_surface_from_texture(
                    &context,
                    &Size2D::new(size.width / 2, size.height),
                    left_image,
                )
                .expect("couldn't create left surface")
        };
        let left_surface_texture = device
            .create_surface_texture(context, left_surface)
            .expect("couldn't create left surface texture");
        let left_texture_id = left_surface_texture.gl_texture();

        let right_surface = unsafe {
            device
                .create_surface_from_texture(
                    &context,
                    &Size2D::new(size.width / 2, size.height),
                    right_image,
                )
                .expect("couldn't create right surface")
        };
        let right_surface_texture = device
            .create_surface_texture(context, right_surface)
            .expect("couldn't create right surface texture");
        let right_texture_id = right_surface_texture.gl_texture();

        self.gl
            .bind_framebuffer(gl::DRAW_FRAMEBUFFER, self.write_fbo);

        // Bind the left eye's texture to the draw framebuffer.
        self.gl.framebuffer_texture_2d(
            gl::DRAW_FRAMEBUFFER,
            gl::COLOR_ATTACHMENT0,
            device.surface_gl_texture_target(),
            left_texture_id,
            0,
        );

        // Blit the appropriate rectangle from the WebXR texture to the d3d texture,
        // flipping the y axis in the process to account for OpenGL->D3D.
        self.gl.blit_framebuffer(
            0,
            0,
            size.width / 2,
            size.height,
            0,
            size.height,
            size.width / 2,
            0,
            gl::COLOR_BUFFER_BIT,
            gl::NEAREST,
        );
        debug_assert_eq!(self.gl.get_error(), gl::NO_ERROR);

        // Bind the right eye's texture to the draw framebuffer.
        self.gl.framebuffer_texture_2d(
            gl::DRAW_FRAMEBUFFER,
            gl::COLOR_ATTACHMENT0,
            device.surface_gl_texture_target(),
            right_texture_id,
            0,
        );

        // Blit the appropriate rectangle from the WebXR texture to the d3d texture.
        self.gl.blit_framebuffer(
            size.width / 2,
            0,
            size.width,
            size.height,
            0,
            size.height,
            size.width / 2,
            0,
            gl::COLOR_BUFFER_BIT,
            gl::NEAREST,
        );
        debug_assert_eq!(self.gl.get_error(), gl::NO_ERROR);

        self.gl.flush();

        // Restore old GL bindings.
        self.gl.bind_framebuffer(gl::FRAMEBUFFER, old_framebuffer);

        self.left_swapchain.release_image().unwrap();
        self.right_swapchain.release_image().unwrap();
        self.frame_stream
            .end(
                self.frame_state.predicted_display_time,
                EnvironmentBlendMode::ADDITIVE,
                &[&CompositionLayerProjection::new()
                    .space(&self.space)
                    .layer_flags(CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA)
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
                            .pose(self.openxr_views[1].pose)
                            .fov(self.openxr_views[1].fov)
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

        let surface = device
            .destroy_surface_texture(context, surface_texture)
            .unwrap();
        let left_surface = device
            .destroy_surface_texture(context, left_surface_texture)
            .unwrap();
        device.destroy_surface(context, left_surface).unwrap();

        let right_surface = device
            .destroy_surface_texture(context, right_surface_texture)
            .unwrap();
        device.destroy_surface(context, right_surface).unwrap();

        surface
    }

    fn initial_inputs(&self) -> Vec<InputSource> {
        vec![
            InputSource {
                handedness: Handedness::Right,
                id: InputId(0),
                target_ray_mode: TargetRayMode::TrackedPointer,
                supports_grip: true,
            },
            InputSource {
                handedness: Handedness::Left,
                id: InputId(1),
                target_ray_mode: TargetRayMode::TrackedPointer,
                supports_grip: true,
            },
        ]
    }

    fn set_event_dest(&mut self, dest: Sender<Event>) {
        self.events.upgrade(dest)
    }

    fn quit(&mut self) {
        self.session.request_exit().unwrap();
    }

    fn set_quitter(&mut self, _: Quitter) {
        // Glwindow currently doesn't have any way to end its own session
        // XXXManishearth add something for this that listens for the window
        // being closed
    }

    fn update_clip_planes(&mut self, near: f32, far: f32) {
        self.clip_planes.update(near, far);
    }

    fn environment_blend_mode(&self) -> webxr_api::EnvironmentBlendMode {
        webxr_api::EnvironmentBlendMode::Additive
    }
}

impl Drop for OpenXrDevice {
    fn drop(&mut self) {
        let _ = self.surfman.0.destroy_context(&mut self.surfman.1);
    }
}

fn transform<Src, Dst>(pose: &Posef) -> RigidTransform3D<f32, Src, Dst> {
    let rotation = Rotation3D::quaternion(
        pose.orientation.x,
        pose.orientation.y,
        pose.orientation.z,
        pose.orientation.w,
    );

    let translation = Vector3D::new(pose.position.x, pose.position.y, pose.position.z);

    RigidTransform3D::new(rotation, translation)
}

#[inline]
fn fov_to_projection_matrix<T, U>(fov: &Fovf, clip_planes: ClipPlanes) -> Transform3D<f32, T, U> {
    util::fov_to_projection_matrix(
        fov.angle_left,
        fov.angle_right,
        fov.angle_up,
        fov.angle_down,
        clip_planes,
    )
}
