use crate::SessionBuilder;
use crate::SwapChains;

use euclid::Point2D;
use euclid::Rect;
use euclid::RigidTransform3D;
use euclid::Rotation3D;
use euclid::Size2D;
use euclid::Transform3D;
use euclid::Vector3D;
use log::{error, warn};
use openxr::d3d::{Requirements, SessionCreateInfo, D3D11};
use openxr::Graphics;
use openxr::{
    self, ActionSet, ActiveActionSet, ApplicationInfo, CompositionLayerFlags,
    CompositionLayerProjection, Entry, EnvironmentBlendMode, ExtensionSet, Extent2Di, FormFactor,
    Fovf, FrameState, FrameStream, FrameWaiter, Instance, Posef, Quaternionf, ReferenceSpaceType,
    SecondaryEndInfo, Session, Space, Swapchain, SwapchainCreateFlags, SwapchainCreateInfo,
    SwapchainUsageFlags, SystemId, Vector3f, ViewConfigurationType,
};
use std::ptr;
use std::sync::{Arc, Mutex};
use std::{cmp, mem};
use std::{thread, time::Duration};
use surfman::Adapter;
use surfman::Device as SurfmanDevice;
use surfman::Surface;
use surfman_chains::SurfaceProvider;
use webxr_api;
use webxr_api::util::{self, ClipPlanes};
use webxr_api::DeviceAPI;
use webxr_api::DiscoveryAPI;
use webxr_api::Error;
use webxr_api::Event;
use webxr_api::EventBuffer;
use webxr_api::Floor;
use webxr_api::Frame;
use webxr_api::InputId;
use webxr_api::InputSource;
use webxr_api::Native;
use webxr_api::Quitter;
use webxr_api::SelectKind;
use webxr_api::Sender;
use webxr_api::Session as WebXrSession;
use webxr_api::SessionId;
use webxr_api::SessionInit;
use webxr_api::SessionMode;
use webxr_api::View;
use webxr_api::ViewerPose;
use webxr_api::Viewports;
use webxr_api::Views;
use webxr_api::Visibility;
use winapi::shared::dxgi;
use winapi::shared::dxgiformat;
use winapi::shared::winerror::{DXGI_ERROR_NOT_FOUND, S_OK};
use winapi::Interface;
use wio::com::ComPtr;

mod input;
use input::OpenXRInput;

const HEIGHT: f32 = 1.4;

const IDENTITY_POSE: Posef = Posef {
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

const VIEW_INIT: openxr::View = openxr::View {
    pose: IDENTITY_POSE,
    fov: Fovf {
        angle_left: 0.,
        angle_right: 0.,
        angle_up: 0.,
        angle_down: 0.,
    },
};

// Whether or not the secondary view is enabled
// Disabled by default to reduce texture sizes
// XXXManishearth we can make this into a pref
const SECONDARY_VIEW_ENABLED: bool = false;
// How much to downscale the view capture by.
const SECONDARY_VIEW_DOWNSCALE: u32 = 2;

pub trait GlThread: Send {
    fn execute(&self, runnable: Box<dyn FnOnce(&SurfmanDevice) + Send>);
    fn clone(&self) -> Box<dyn GlThread>;
}

pub trait SurfaceProviderRegistration: Send {
    fn register(&self, id: SessionId, provider: Box<dyn SurfaceProvider<SurfmanDevice> + Send>);
    fn clone(&self) -> Box<dyn SurfaceProviderRegistration>;
}

/// Provides a way to spawn and interact with context menus
pub trait ContextMenuProvider: Send {
    /// Open a context menu, return a way to poll for the result
    fn open_context_menu(&self) -> Box<dyn ContextMenuFuture>;
    /// Clone self as a trait object
    fn clone_object(&self) -> Box<dyn ContextMenuProvider>;
}

/// A way to poll for the result of the context menu request
pub trait ContextMenuFuture {
    fn poll(&self) -> ContextMenuResult;
}

/// The result of polling on a context menu request
pub enum ContextMenuResult {
    /// Session should exit
    ExitSession,
    /// Dialog was dismissed
    Dismissed,
    /// User has not acted on dialog
    Pending,
}

struct ViewInfo {
    view: openxr::View,
    extent: Extent2Di,
}

impl Drop for OpenXrDevice {
    fn drop(&mut self) {
        // This should be happening automatically in the destructors,
        // but it isn't, presumably because there's an extra handle floating
        // around somewhere
        // XXXManishearth find out where that extra handle is
        unsafe {
            (self.instance.fp().destroy_session)(self.session.as_raw());
            (self.instance.fp().destroy_instance)(self.instance.as_raw());
        }
    }
}

pub struct OpenXrDiscovery {
    gl_thread: Box<dyn GlThread>,
    provider_registration: Box<dyn SurfaceProviderRegistration>,
    context_menu_provider: Box<dyn ContextMenuProvider>,
}

impl OpenXrDiscovery {
    pub fn new(
        gl_thread: Box<dyn GlThread>,
        provider_registration: Box<dyn SurfaceProviderRegistration>,
        context_menu_provider: Box<dyn ContextMenuProvider>,
    ) -> Self {
        Self {
            gl_thread,
            provider_registration,
            context_menu_provider,
        }
    }
}

pub struct CreatedInstance {
    instance: Instance,
    supports_hands: bool,
    supports_secondary: bool,
    system: SystemId,
}

pub fn create_instance(needs_hands: bool) -> Result<CreatedInstance, String> {
    let entry = Entry::load().map_err(|e| format!("Entry::load {:?}", e))?;
    let supported = entry
        .enumerate_extensions()
        .map_err(|e| format!("Entry::enumerate_extensions {:?}", e))?;
    warn!("Available extensions:\n{:?}", supported);
    let mut supports_hands = needs_hands && supported.msft_hand_tracking_preview;
    let supports_secondary = SECONDARY_VIEW_ENABLED
        && supported.msft_secondary_view_configuration_preview
        && supported.msft_first_person_observer_preview;
    let app_info = ApplicationInfo {
        application_name: "firefox.reality",
        application_version: 1,
        engine_name: "servo",
        engine_version: 1,
    };

    let mut exts = ExtensionSet::default();
    exts.khr_d3d11_enable = true;
    if supports_hands {
        exts.msft_hand_tracking_preview = true;
    }

    if supports_secondary {
        exts.msft_secondary_view_configuration_preview = true;
        exts.msft_first_person_observer_preview = true;
    }

    let instance = entry
        .create_instance(&app_info, &exts)
        .map_err(|e| format!("Entry::create_instance {:?}", e))?;
    let system = instance
        .system(FormFactor::HEAD_MOUNTED_DISPLAY)
        .map_err(|e| format!("Instance::system {:?}", e))?;

    if supports_hands {
        supports_hands |= instance
            .supports_hand_tracking(system)
            .map_err(|e| format!("Instance::supports_hand_tracking {:?}", e))?;
    }

    Ok(CreatedInstance {
        instance,
        supports_hands,
        supports_secondary,
        system,
    })
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

pub fn create_surfman_adapter() -> Option<Adapter> {
    let instance = create_instance(false).ok()?;
    let system = instance
        .instance
        .system(FormFactor::HEAD_MOUNTED_DISPLAY)
        .ok()?;

    let requirements = D3D11::requirements(&instance.instance, system).ok()?;
    let adapter = get_matching_adapter(&requirements).ok()?;
    Some(Adapter::from_dxgi_adapter(adapter.up()))
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
        init: &SessionInit,
        xr: SessionBuilder,
    ) -> Result<WebXrSession, Error> {
        if self.supports_session(mode) {
            let gl_thread = self.gl_thread.clone();
            let provider_registration = self.provider_registration.clone();
            let needs_hands = init.feature_requested("hand-tracking");
            let instance = create_instance(needs_hands).map_err(|e| Error::BackendSpecific(e))?;

            let mut supported_features = vec!["local-floor".into()];
            if instance.supports_hands {
                supported_features.push("hand-tracking".into());
            }

            let granted_features = init.validate(mode, &supported_features)?;
            let id = xr.id();
            let context_menu_provider = self.context_menu_provider.clone_object();
            xr.spawn(move || {
                OpenXrDevice::new(
                    gl_thread,
                    provider_registration,
                    instance,
                    granted_features,
                    id,
                    context_menu_provider,
                )
            })
        } else {
            Err(Error::NoMatchingDevice)
        }
    }

    fn supports_session(&self, mode: SessionMode) -> bool {
        mode == SessionMode::ImmersiveAR || mode == SessionMode::ImmersiveVR
    }
}

struct OpenXrDevice {
    session: Session<D3D11>,
    instance: Instance,
    events: EventBuffer,
    frame_waiter: FrameWaiter,
    shared_data: Arc<Mutex<SharedData>>,
    viewer_space: Space,
    blend_mode: EnvironmentBlendMode,
    clip_planes: ClipPlanes,
    view_configurations: Vec<openxr::ViewConfigurationView>,
    secondary_configuration: Option<openxr::ViewConfigurationView>,

    // input
    action_set: ActionSet,
    right_hand: OpenXRInput,
    left_hand: OpenXRInput,
    granted_features: Vec<String>,
    context_menu_provider: Box<dyn ContextMenuProvider>,
    context_menu_future: Option<Box<dyn ContextMenuFuture>>,
}

/// Data that is shared between the openxr thread and the
/// surface provider that runs in the webgl thread.
struct SharedData {
    left: ViewInfo,
    right: ViewInfo,
    secondary: Option<ViewInfo>,
    frame_state: Option<FrameState>,
    frame_stream: FrameStream<D3D11>,
    space: Space,
}

struct OpenXrProvider {
    images: Box<[<D3D11 as Graphics>::SwapchainImage]>,
    image_queue: Vec<usize>,
    surfaces: Box<[Option<Surface>]>,
    swapchain: Swapchain<D3D11>,
    shared_data: Arc<Mutex<SharedData>>,
    fake_surface: Option<Surface>,
    blend_mode: EnvironmentBlendMode,
    secondary_blend_mode: Option<EnvironmentBlendMode>,
}

// This is required due to the presence of the swapchain image
// pointers in the struct. D3D11 resources like textures are
// safe to send between threads.
unsafe impl Send for OpenXrProvider {}

impl SurfaceProvider<SurfmanDevice> for OpenXrProvider {
    fn recycle_front_buffer(&mut self, _device: &mut surfman::Device) {
        // At this point the frame contents have been rendered, so we can release access to the texture
        // in preparation for displaying it.
        let mut data = self.shared_data.lock().unwrap();
        let data = &mut *data;
        if let Err(e) = self.swapchain.release_image() {
            error!("Error releasing swapchain image: {:?}", e);
        }

        // Invert the up/down angles so that openxr flips the texture in the y axis.
        let mut l_fov = data.left.view.fov;
        let mut r_fov = data.right.view.fov;
        std::mem::swap(&mut l_fov.angle_up, &mut l_fov.angle_down);
        std::mem::swap(&mut r_fov.angle_up, &mut r_fov.angle_down);

        let views = [
            openxr::CompositionLayerProjectionView::new()
                .pose(data.left.view.pose)
                .fov(l_fov)
                .sub_image(
                    openxr::SwapchainSubImage::new()
                        .swapchain(&self.swapchain)
                        .image_array_index(0)
                        .image_rect(openxr::Rect2Di {
                            offset: openxr::Offset2Di { x: 0, y: 0 },
                            extent: data.left.extent,
                        }),
                ),
            openxr::CompositionLayerProjectionView::new()
                .pose(data.right.view.pose)
                .fov(r_fov)
                .sub_image(
                    openxr::SwapchainSubImage::new()
                        .swapchain(&self.swapchain)
                        .image_array_index(0)
                        .image_rect(openxr::Rect2Di {
                            offset: openxr::Offset2Di {
                                x: data.left.extent.width,
                                y: 0,
                            },
                            extent: data.right.extent,
                        }),
                ),
        ];

        let layers = [&*CompositionLayerProjection::new()
            .space(&data.space)
            .layer_flags(CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA)
            .views(&views[..])];

        if let Some(ref secondary) = data.secondary {
            let mut s_fov = secondary.view.fov;
            let secondary_blend_mode = self
                .secondary_blend_mode
                .expect("secondary blend mode must be set if secondary views are enabled");
            std::mem::swap(&mut s_fov.angle_up, &mut s_fov.angle_down);
            let views = [openxr::CompositionLayerProjectionView::new()
                .pose(secondary.view.pose)
                .fov(s_fov)
                .sub_image(
                    openxr::SwapchainSubImage::new()
                        .swapchain(&self.swapchain)
                        .image_array_index(0)
                        .image_rect(openxr::Rect2Di {
                            offset: openxr::Offset2Di {
                                x: data.left.extent.width + data.right.extent.width,
                                y: 0,
                            },
                            extent: secondary.extent,
                        }),
                )];

            let secondary_layers = [&*CompositionLayerProjection::new()
                .space(&data.space)
                .layer_flags(CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA)
                .views(&views[..])];
            if let Err(e) = data.frame_stream.end_secondary(
                data.frame_state.as_ref().unwrap().predicted_display_time,
                self.blend_mode,
                &layers[..],
                SecondaryEndInfo {
                    ty: ViewConfigurationType::SECONDARY_MONO_FIRST_PERSON_OBSERVER_MSFT,
                    // XXXManishearth should we use the secondary layer's blend mode here, given
                    // that the content will be using the primary blend mode?
                    environment_blend_mode: secondary_blend_mode,
                    layers: &secondary_layers,
                },
            ) {
                error!("Error ending frame: {:?}", e);
            }
        } else {
            if let Err(e) = data.frame_stream.end(
                data.frame_state.as_ref().unwrap().predicted_display_time,
                self.blend_mode,
                &layers[..],
            ) {
                error!("Error ending frame: {:?}", e);
            }
        }
    }

    fn recycle_surface(&mut self, surface: Surface) {
        assert!(self.fake_surface.is_none());
        self.fake_surface = Some(surface);
    }

    fn provide_surface(
        &mut self,
        device: &mut surfman::Device,
        context: &mut surfman::Context,
        size: euclid::default::Size2D<i32>,
    ) -> Result<Surface, surfman::Error> {
        let image = self.swapchain.acquire_image().map_err(|e| {
            error!("Error acquiring swapchain image: {:?}", e);
            surfman::Error::Failed
        })?;
        self.swapchain
            .wait_image(openxr::Duration::INFINITE)
            .map_err(|e| {
                error!("Error waiting on swapchain image: {:?}", e);
                surfman::Error::Failed
            })?;

        // Store the current image index that was acquired in the queue of
        // surfaces that have been handed out.
        self.image_queue.push(image as usize);

        // If we already have a surface, we can return it immediately.
        // Otherwise we need to create a new surface that wraps the
        // OpenXR texture.
        let surface = self.surfaces[image as usize]
            .take()
            .ok_or(surfman::Error::SurfaceDataInaccessible)
            .or_else(|_| unsafe {
                device.create_surface_from_texture(
                    context,
                    &Size2D::new(size.width, size.height),
                    self.images[image as usize],
                )
            });
        surface
    }

    fn take_front_buffer(&mut self) -> Option<Surface> {
        self.fake_surface.take()
    }

    fn set_front_buffer(
        &mut self,
        device: &mut surfman::Device,
        context: &mut surfman::Context,
        new_front_buffer: Surface,
    ) -> Result<(), surfman::Error> {
        // At this point the front buffer's contents are already present in the underlying openxr texture.
        // We only need to store the surface because the webxr crate's API assumes that Surface objects
        // must be passed to the rendering method.

        // Return the complete surface to the surface cache in the position corresponding
        // to the front of the outstanding surface queue.
        let pending_idx = self.image_queue[0];
        assert!(self.surfaces[pending_idx].is_none());
        self.surfaces[pending_idx] = Some(new_front_buffer);
        // Remove the first element of the queue of outstanding surfaces.
        self.image_queue.remove(0);

        // We will be handing out a threadsafe surface in the future, so we need
        // to create it if it doesn't already exist.
        if self.fake_surface.is_none() {
            self.fake_surface = Some(device.create_surface(
                context,
                surfman::SurfaceAccess::GPUOnly,
                surfman::SurfaceType::Generic {
                    size: Size2D::new(1, 1),
                },
            )?);
        }

        Ok(())
    }

    fn create_sized_surface(
        &mut self,
        _device: &mut surfman::Device,
        _context: &mut surfman::Context,
        _size: euclid::default::Size2D<i32>,
    ) -> Result<Surface, surfman::Error> {
        // All OpenXR-based surfaces are created once during session initialization; we cannot create new ones.
        // This is only used when resizing, however, and OpenXR-based systems don't resize.
        Err(surfman::Error::UnsupportedOnThisPlatform)
    }

    fn destroy_all_surfaces(
        &mut self,
        device: &mut surfman::Device,
        context: &mut surfman::Context,
    ) -> Result<(), surfman::Error> {
        // Destroy any cached surfaces that wrap OpenXR textures.
        for surface in self.surfaces.iter_mut().map(Option::take) {
            if let Some(mut surface) = surface {
                device.destroy_surface(context, &mut surface)?;
            }
        }
        if let Some(mut fake) = self.fake_surface.take() {
            device.destroy_surface(context, &mut fake)?;
        }
        Ok(())
    }
}

impl OpenXrDevice {
    fn new(
        gl_thread: Box<dyn GlThread>,
        provider_registration: Box<dyn SurfaceProviderRegistration>,
        instance: CreatedInstance,
        granted_features: Vec<String>,
        id: SessionId,
        context_menu_provider: Box<dyn ContextMenuProvider>,
    ) -> Result<OpenXrDevice, Error> {
        let CreatedInstance {
            instance,
            supports_hands,
            supports_secondary,
            system,
        } = instance;

        let (device_tx, device_rx) = crossbeam_channel::unbounded();
        let (provider_tx, provider_rx) = crossbeam_channel::unbounded();
        let _ = gl_thread.execute(Box::new(move |device| {
            // Get the current surfman device and extract it's D3D device. This will ensure
            // that the OpenXR runtime's texture will be shareable with surfman's surfaces.
            let native_device = device.native_device();
            let d3d_device = native_device.d3d11_device;
            // Smuggle the pointer out as a usize value; D3D11 devices are threadsafe
            // so it's safe to use it from another thread.
            let _ = device_tx.send(d3d_device as usize);
            let _ = provider_rx.recv();
        }));
        // Get the D3D11 device pointer from the webgl thread.
        let device = device_rx.recv().unwrap();

        // OpenXR returns a validation error unless we call this method, so we call it
        // and ignore the results. Users of this backend are expected to have already called
        // create_surfman_adapter, which uses the graphics requirements to create a matching
        // surfman adapter. The previous code that obtains a D3D11 device is expected to
        // have been created via this matching adapter.
        let _requirements = D3D11::requirements(&instance, system)
            .map_err(|e| Error::BackendSpecific(format!("D3D11::requirements {:?}", e)))?;

        let (session, frame_waiter, frame_stream) = unsafe {
            instance
                .create_session::<D3D11>(
                    system,
                    &SessionCreateInfo {
                        device: device as *mut _,
                    },
                )
                .map_err(|e| Error::BackendSpecific(format!("Instance::create_session {:?}", e)))?
        };

        // XXXPaul initialisation should happen on SessionStateChanged(Ready)?

        if supports_secondary {
            session
                .begin_with_secondary(
                    ViewConfigurationType::PRIMARY_STEREO,
                    &[ViewConfigurationType::SECONDARY_MONO_FIRST_PERSON_OBSERVER_MSFT],
                )
                .map_err(|e| {
                    Error::BackendSpecific(format!("Session::begin_with_secondary {:?}", e))
                })?;
        } else {
            session
                .begin(ViewConfigurationType::PRIMARY_STEREO)
                .map_err(|e| Error::BackendSpecific(format!("Session::begin {:?}", e)))?;
        }

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
            .map_err(|e| {
                Error::BackendSpecific(format!("Session::create_reference_space {:?}", e))
            })?;

        let viewer_space = session
            .create_reference_space(ReferenceSpaceType::VIEW, pose)
            .map_err(|e| {
                Error::BackendSpecific(format!("Session::create_reference_space {:?}", e))
            })?;

        let view_configuration_type = ViewConfigurationType::PRIMARY_STEREO;
        let view_configurations = instance
            .enumerate_view_configuration_views(system, view_configuration_type)
            .map_err(|e| {
                Error::BackendSpecific(format!(
                    "Session::enumerate_view_configuration_views {:?}",
                    e
                ))
            })?;

        let blend_mode = instance
            .enumerate_environment_blend_modes(system, view_configuration_type)
            .map_err(|e| {
                Error::BackendSpecific(format!(
                    "Instance::enumerate_environment_blend_modes {:?}",
                    e
                ))
            })?[0];

        let left_view_configuration = view_configurations[0];
        let right_view_configuration = view_configurations[1];
        let left_extent = Extent2Di {
            width: left_view_configuration.recommended_image_rect_width as i32,
            height: left_view_configuration.recommended_image_rect_height as i32,
        };
        let right_extent = Extent2Di {
            width: right_view_configuration.recommended_image_rect_width as i32,
            height: right_view_configuration.recommended_image_rect_height as i32,
        };

        assert_eq!(
            left_view_configuration.recommended_image_rect_height,
            right_view_configuration.recommended_image_rect_height,
        );
        let mut sw_width = left_view_configuration.recommended_image_rect_width
            + right_view_configuration.recommended_image_rect_width;
        let mut sw_height = left_view_configuration.recommended_image_rect_height;
        let (secondary_configuration, secondary_blend_mode) = if supports_secondary {
            let view_configuration = *instance
                .enumerate_view_configuration_views(
                    system,
                    ViewConfigurationType::SECONDARY_MONO_FIRST_PERSON_OBSERVER_MSFT,
                )
                .map_err(|e| {
                    Error::BackendSpecific(format!(
                        "Session::enumerate_view_configuration_views {:?}",
                        e
                    ))
                })?
                .get(0)
                .expect(
                    "Session::enumerate_view_configuration_views() returned no secondary views",
                );

            sw_width += view_configuration.recommended_image_rect_width / SECONDARY_VIEW_DOWNSCALE;
            sw_height = cmp::max(
                sw_height,
                view_configuration.recommended_image_rect_height / SECONDARY_VIEW_DOWNSCALE,
            );
            let secondary_blend_mode = instance
                .enumerate_environment_blend_modes(
                    system,
                    ViewConfigurationType::SECONDARY_MONO_FIRST_PERSON_OBSERVER_MSFT,
                )
                .map_err(|e| {
                    Error::BackendSpecific(format!(
                        "Instance::enumerate_environment_blend_modes {:?}",
                        e
                    ))
                })?[0];
            (Some(view_configuration), Some(secondary_blend_mode))
        } else {
            (None, None)
        };

        // Create swapchains

        // XXXManishearth should we be doing this, or letting Servo set the format?
        let formats = session.enumerate_swapchain_formats().map_err(|e| {
            Error::BackendSpecific(format!("Session::enumerate_swapchain_formats {:?}", e))
        })?;
        let format = pick_format(&formats);

        let swapchain_create_info = SwapchainCreateInfo {
            create_flags: SwapchainCreateFlags::EMPTY,
            usage_flags: SwapchainUsageFlags::COLOR_ATTACHMENT | SwapchainUsageFlags::SAMPLED,
            format,
            sample_count: 1,
            width: sw_width,
            height: sw_height,
            face_count: 1,
            array_size: 1,
            mip_count: 1,
        };

        let swapchain = session
            .create_swapchain(&swapchain_create_info)
            .map_err(|e| Error::BackendSpecific(format!("Session::create_swapchain {:?}", e)))?;
        let images = swapchain
            .enumerate_images()
            .map_err(|e| Error::BackendSpecific(format!("Session::enumerate_images {:?}", e)))?;

        let mut surfaces = Vec::with_capacity(images.len());
        for _ in 0..images.len() {
            surfaces.push(None);
        }

        let left = ViewInfo {
            view: VIEW_INIT,
            extent: left_extent,
        };
        let right = ViewInfo {
            view: VIEW_INIT,
            extent: right_extent,
        };
        let shared_data = Arc::new(Mutex::new(SharedData {
            frame_stream,
            frame_state: None,
            space,
            left,
            right,
            secondary: None,
        }));

        let provider = Box::new(OpenXrProvider {
            swapchain,
            image_queue: Vec::with_capacity(images.len()),
            images: images.into_boxed_slice(),
            surfaces: surfaces.into_boxed_slice(),
            fake_surface: None,
            shared_data: shared_data.clone(),
            blend_mode,
            secondary_blend_mode,
        });
        provider_registration.register(id, provider);
        // Ensure the webgl thread is blocked until we're done initializing
        // the surface provider.
        let _ = provider_tx.send(());

        // input

        let (action_set, right_hand, left_hand) =
            OpenXRInput::setup_inputs(&instance, &session, supports_hands);

        Ok(OpenXrDevice {
            instance,
            events: Default::default(),
            session,
            frame_waiter,
            viewer_space,
            clip_planes: Default::default(),
            blend_mode,
            view_configurations,
            secondary_configuration,
            shared_data,

            action_set,
            right_hand,
            left_hand,
            granted_features,
            context_menu_provider,
            context_menu_future: None,
        })
    }

    fn handle_openxr_events(&mut self) -> bool {
        use openxr::Event::*;
        let mut stopped = false;
        loop {
            let mut buffer = openxr::EventDataBuffer::new();
            let event = match self.instance.poll_event(&mut buffer) {
                Ok(event) => event,
                Err(e) => {
                    error!("Error polling events: {:?}", e);
                    return false;
                }
            };
            match event {
                Some(SessionStateChanged(session_change)) => match session_change.state() {
                    openxr::SessionState::EXITING | openxr::SessionState::LOSS_PENDING => {
                        self.events.callback(Event::SessionEnd);
                        return false;
                    }
                    openxr::SessionState::STOPPING => {
                        self.events
                            .callback(Event::VisibilityChange(Visibility::Hidden));
                        if let Err(e) = self.session.end() {
                            error!("Session failed to end on STOPPING: {:?}", e);
                        }
                        stopped = true;
                    }
                    openxr::SessionState::READY if stopped => {
                        self.events
                            .callback(Event::VisibilityChange(Visibility::Visible));
                        if let Err(e) = self.session.begin(ViewConfigurationType::PRIMARY_STEREO) {
                            error!("Session failed to begin on READY: {:?}", e);
                        }
                        stopped = false;
                    }
                    openxr::SessionState::FOCUSED => {
                        self.events
                            .callback(Event::VisibilityChange(Visibility::Visible));
                    }
                    openxr::SessionState::VISIBLE => {
                        self.events
                            .callback(Event::VisibilityChange(Visibility::VisibleBlurred));
                    }
                    _ => {
                        // FIXME: Handle other states
                    }
                },
                Some(InstanceLossPending(_)) => {
                    self.events.callback(Event::SessionEnd);
                    return false;
                }
                Some(_) => {
                    // FIXME: Handle other events
                }
                None if stopped => {
                    // XXXManishearth be able to handle exits during this time
                    thread::sleep(Duration::from_millis(200));
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

impl OpenXrDevice {
    fn views(&self) -> Views {
        let default_views = Views::Stereo(
            View {
                ..Default::default()
            },
            View {
                ..Default::default()
            },
        );

        let data = self.shared_data.lock().unwrap();
        let frame_state = if let Some(ref fs) = data.frame_state {
            fs
        } else {
            // This data isn't accessed till the first frame, so it
            // doesn't really matter what it is right now
            return default_views;
        };

        let (_view_flags, views) = match self.session.locate_views(
            ViewConfigurationType::PRIMARY_STEREO,
            frame_state.predicted_display_time,
            &self.viewer_space,
        ) {
            Ok(data) => data,
            Err(e) => {
                error!("Error locating views: {:?}", e);
                return default_views;
            }
        };
        let left_view = View {
            transform: transform(&views[0].pose).inverse(),
            projection: fov_to_projection_matrix(&views[0].fov, self.clip_planes),
        };
        let right_view = View {
            transform: transform(&views[1].pose).inverse(),
            projection: fov_to_projection_matrix(&views[1].fov, self.clip_planes),
        };
        if self.secondary_configuration.is_some() {
            if data.secondary.is_some() {
                let (_view_flags, views) = match self.session.locate_views(
                    ViewConfigurationType::SECONDARY_MONO_FIRST_PERSON_OBSERVER_MSFT,
                    frame_state.predicted_display_time,
                    &self.viewer_space,
                ) {
                    Ok(data) => data,
                    Err(e) => {
                        error!("Error locating views: {:?}", e);
                        return default_views;
                    }
                };

                let third_eye = View {
                    transform: transform(&views[0].pose).inverse(),
                    projection: fov_to_projection_matrix(&views[0].fov, self.clip_planes),
                };
                return Views::StereoCapture(left_view, right_view, third_eye);
            }
        }
        Views::Stereo(left_view, right_view)
    }
}

impl DeviceAPI<Surface> for OpenXrDevice {
    fn floor_transform(&self) -> Option<RigidTransform3D<f32, Native, Floor>> {
        let translation = Vector3D::new(0.0, HEIGHT, 0.0);
        Some(RigidTransform3D::from_translation(translation))
    }

    fn viewports(&self) -> Viewports {
        let left_view_configuration = &self.view_configurations[0];
        let right_view_configuration = &self.view_configurations[1];
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
        let mut viewports = vec![left_vp, right_vp];
        if let Some(config) = self.secondary_configuration {
            let secondary_vp = Rect::new(
                Point2D::new(
                    left_view_configuration.recommended_image_rect_width as i32
                        + right_view_configuration.recommended_image_rect_width as i32,
                    0,
                ),
                Size2D::new(
                    (config.recommended_image_rect_width / SECONDARY_VIEW_DOWNSCALE) as i32,
                    (config.recommended_image_rect_height / SECONDARY_VIEW_DOWNSCALE) as i32,
                ),
            );
            viewports.push(secondary_vp)
        }
        Viewports { viewports }
    }

    fn wait_for_animation_frame(&mut self) -> Option<Frame> {
        if !self.handle_openxr_events() {
            warn!("no frame, session isn't running");
            // Session is not running anymore.
            return None;
        }
        if let Some(ref context_menu_future) = self.context_menu_future {
            match context_menu_future.poll() {
                ContextMenuResult::ExitSession => {
                    self.quit();
                    return None;
                }
                ContextMenuResult::Dismissed => self.context_menu_future = None,
                ContextMenuResult::Pending => (),
            }
        }

        let mut data = self.shared_data.lock().unwrap();
        let frame_state = if let Some(secondary_configuration) = self.secondary_configuration {
            let (frame_state, secondary_state) = match self
                .frame_waiter
                .wait_secondary(ViewConfigurationType::SECONDARY_MONO_FIRST_PERSON_OBSERVER_MSFT)
            {
                Ok(frame_state) => frame_state,
                Err(e) => {
                    error!("Error waiting on frame: {:?}", e);
                    return None;
                }
            };

            assert_eq!(
                secondary_state.ty,
                ViewConfigurationType::SECONDARY_MONO_FIRST_PERSON_OBSERVER_MSFT
            );

            if data.secondary.is_some() != secondary_state.active {
                let extent = Extent2Di {
                    width: (secondary_configuration.recommended_image_rect_width
                        / SECONDARY_VIEW_DOWNSCALE) as i32,
                    height: (secondary_configuration.recommended_image_rect_height
                        / SECONDARY_VIEW_DOWNSCALE) as i32,
                };
                data.secondary = if secondary_state.active {
                    Some(ViewInfo {
                        view: VIEW_INIT,
                        extent,
                    })
                } else {
                    None
                };

                println!(
                    "Secondary view configuration state changed to {}",
                    secondary_state.active
                );
            }

            frame_state
        } else {
            match self.frame_waiter.wait() {
                Ok(frame_state) => frame_state,
                Err(e) => {
                    error!("Error waiting on frame: {:?}", e);
                    return None;
                }
            }
        };

        let time_ns = time::precise_time_ns();

        if let Err(e) = data.frame_stream.begin() {
            error!("Error beginning frame stream: {:?}", e);
            return None;
        }

        // XXXManishearth should we check frame_state.should_render?
        let (_view_flags, views) = match self.session.locate_views(
            ViewConfigurationType::PRIMARY_STEREO,
            frame_state.predicted_display_time,
            &data.space,
        ) {
            Ok(data) => data,
            Err(e) => {
                error!("Error locating views: {:?}", e);
                return None;
            }
        };
        data.left.view = views[0];
        data.right.view = views[1];
        let pose = match self
            .viewer_space
            .locate(&data.space, frame_state.predicted_display_time)
        {
            Ok(pose) => pose,
            Err(e) => {
                error!("Error locating viewer space: {:?}", e);
                return None;
            }
        };
        let transform = transform(&pose.pose);

        // the following code does a split borrow, which only works on actual references
        let data_ = &mut *data;
        if let Some(ref mut info) = data_.secondary {
            let view = match self.session.locate_views(
                ViewConfigurationType::SECONDARY_MONO_FIRST_PERSON_OBSERVER_MSFT,
                frame_state.predicted_display_time,
                &data_.space,
            ) {
                Ok(v) => v.1[0],
                Err(e) => {
                    error!("Error locating views: {:?}", e);
                    return None;
                }
            };
            info.view = view;
        }

        let active_action_set = ActiveActionSet::new(&self.action_set);

        if let Err(e) = self.session.sync_actions(&[active_action_set]) {
            error!("Error syncing actions: {:?}", e);
            return None;
        }

        let mut right = self
            .right_hand
            .frame(&self.session, &frame_state, &data.space, &transform);
        let mut left = self
            .left_hand
            .frame(&self.session, &frame_state, &data.space, &transform);

        data.frame_state = Some(frame_state);
        // views() needs to reacquire the lock.
        drop(data);
        let views = self.views();

        if (left.menu_selected || right.menu_selected) && self.context_menu_future.is_none() {
            self.context_menu_future = Some(self.context_menu_provider.open_context_menu());
        } else if self.context_menu_future.is_some() {
            // Do not surface input info whilst the context menu is open
            // We don't do this for the first frame after the context menu is opened
            // so that the appropriate select cancel events may fire
            right.frame.target_ray_origin = None;
            right.frame.grip_origin = None;
            left.frame.target_ray_origin = None;
            left.frame.grip_origin = None;
            right.select = None;
            right.squeeze = None;
            left.select = None;
            left.squeeze = None;
        }

        let frame = Frame {
            pose: Some(ViewerPose { transform, views }),
            inputs: vec![right.frame, left.frame],
            events: vec![],
            time_ns,
            sent_time: 0,
            hit_test_results: vec![],
        };

        if let Some(right_select) = right.select {
            self.events.callback(Event::Select(
                InputId(0),
                SelectKind::Select,
                right_select,
                frame.clone(),
            ));
        }
        if let Some(right_squeeze) = right.squeeze {
            self.events.callback(Event::Select(
                InputId(0),
                SelectKind::Squeeze,
                right_squeeze,
                frame.clone(),
            ));
        }
        if let Some(left_select) = left.select {
            self.events.callback(Event::Select(
                InputId(1),
                SelectKind::Select,
                left_select,
                frame.clone(),
            ));
        }
        if let Some(left_squeeze) = left.squeeze {
            self.events.callback(Event::Select(
                InputId(1),
                SelectKind::Squeeze,
                left_squeeze,
                frame.clone(),
            ));
        }
        Some(frame)
    }

    fn render_animation_frame(&mut self, surface: Surface) -> Surface {
        // We have already told OpenXR to display the frame as part of `recycle_front_buffer`.
        // Due to threading issues we can't call D3D11 APIs on the openxr thread as the
        // WebGL thread might be using the device simultaneously, so this method is a no-op.
        surface
    }

    fn initial_inputs(&self) -> Vec<InputSource> {
        vec![
            self.right_hand.input_source(),
            self.left_hand.input_source(),
        ]
    }

    fn set_event_dest(&mut self, dest: Sender<Event>) {
        self.events.upgrade(dest)
    }

    fn quit(&mut self) {
        self.session.request_exit().unwrap();
        loop {
            let mut buffer = openxr::EventDataBuffer::new();
            let event = match self.instance.poll_event(&mut buffer) {
                Ok(e) => e,
                Err(e) => {
                    error!("Error polling for event while quitting: {:?}", e);
                    break;
                }
            };
            match event {
                Some(openxr::Event::SessionStateChanged(session_change)) => {
                    match session_change.state() {
                        openxr::SessionState::EXITING => {
                            break;
                        }
                        openxr::SessionState::STOPPING => {
                            if let Err(e) = self.session.end() {
                                error!("Session failed to end while STOPPING: {:?}", e);
                            }
                        }
                        _ => (),
                    }
                }
                _ => (),
            }
            thread::sleep(Duration::from_millis(30));
        }
        self.events.callback(Event::SessionEnd);
    }

    fn set_quitter(&mut self, _: Quitter) {
        // the quitter is only needed if we have anything from outside the render
        // thread that can signal a quit. We don't.
    }

    fn update_clip_planes(&mut self, near: f32, far: f32) {
        self.clip_planes.update(near, far);
    }

    fn environment_blend_mode(&self) -> webxr_api::EnvironmentBlendMode {
        match self.blend_mode {
            EnvironmentBlendMode::OPAQUE => webxr_api::EnvironmentBlendMode::Opaque,
            EnvironmentBlendMode::ALPHA_BLEND => webxr_api::EnvironmentBlendMode::AlphaBlend,
            EnvironmentBlendMode::ADDITIVE => webxr_api::EnvironmentBlendMode::Additive,
            v => unimplemented!("unsupported blend mode: {:?}", v),
        }
    }

    fn granted_features(&self) -> &[String] {
        &self.granted_features
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
