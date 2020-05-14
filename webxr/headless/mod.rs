/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::SessionBuilder;
use crate::SwapChains;

use webxr_api::util::{self, ClipPlanes, HitTestList};
use webxr_api::ApiSpace;
use webxr_api::BaseSpace;
use webxr_api::DeviceAPI;
use webxr_api::DiscoveryAPI;
use webxr_api::Error;
use webxr_api::Event;
use webxr_api::EventBuffer;
use webxr_api::Floor;
use webxr_api::Frame;
use webxr_api::FrameUpdateEvent;
use webxr_api::HitTestId;
use webxr_api::HitTestResult;
use webxr_api::HitTestSource;
use webxr_api::Input;
use webxr_api::InputFrame;
use webxr_api::InputId;
use webxr_api::InputSource;
use webxr_api::MockDeviceInit;
use webxr_api::MockDeviceMsg;
use webxr_api::MockDiscoveryAPI;
use webxr_api::MockInputMsg;
use webxr_api::MockViewInit;
use webxr_api::MockViewsInit;
use webxr_api::MockWorld;
use webxr_api::Native;
use webxr_api::Quitter;
use webxr_api::Ray;
use webxr_api::Receiver;
use webxr_api::SelectEvent;
use webxr_api::SelectKind;
use webxr_api::Sender;
use webxr_api::Session;
use webxr_api::SessionInit;
use webxr_api::SessionMode;
use webxr_api::Space;
use webxr_api::View;
use webxr_api::Viewer;
use webxr_api::ViewerPose;
use webxr_api::Viewports;
use webxr_api::Views;

use euclid::RigidTransform3D;

use std::sync::{Arc, Mutex};
use std::thread;

use surfman::Surface;

pub struct HeadlessMockDiscovery {}

struct HeadlessDiscovery {
    data: Arc<Mutex<HeadlessDeviceData>>,
    supports_vr: bool,
    supports_inline: bool,
    supports_ar: bool,
}

struct InputInfo {
    source: InputSource,
    active: bool,
    pointer: Option<RigidTransform3D<f32, Input, Native>>,
    grip: Option<RigidTransform3D<f32, Input, Native>>,
    clicking: bool,
}

struct HeadlessDevice {
    data: Arc<Mutex<HeadlessDeviceData>>,
    id: u32,
    hit_tests: HitTestList,
    granted_features: Vec<String>,
}

struct PerSessionData {
    id: u32,
    mode: SessionMode,
    clip_planes: ClipPlanes,
    quitter: Option<Quitter>,
    events: EventBuffer,
    needs_vp_update: bool,
}

struct HeadlessDeviceData {
    floor_transform: Option<RigidTransform3D<f32, Native, Floor>>,
    viewer_origin: Option<RigidTransform3D<f32, Viewer, Native>>,
    supported_features: Vec<String>,
    views: MockViewsInit,
    needs_floor_update: bool,
    inputs: Vec<InputInfo>,
    sessions: Vec<PerSessionData>,
    disconnected: bool,
    world: Option<MockWorld>,
    next_id: u32,
}

impl MockDiscoveryAPI<SwapChains> for HeadlessMockDiscovery {
    fn simulate_device_connection(
        &mut self,
        init: MockDeviceInit,
        receiver: Receiver<MockDeviceMsg>,
    ) -> Result<Box<dyn DiscoveryAPI<SwapChains>>, Error> {
        let viewer_origin = init.viewer_origin.clone();
        let floor_transform = init.floor_origin.map(|f| f.inverse());
        let views = init.views.clone();
        let data = HeadlessDeviceData {
            floor_transform,
            viewer_origin,
            supported_features: init.supported_features,
            views,
            needs_floor_update: false,
            inputs: vec![],
            sessions: vec![],
            disconnected: false,
            world: init.world,
            next_id: 0,
        };
        let data = Arc::new(Mutex::new(data));
        let data_ = data.clone();

        thread::spawn(move || {
            run_loop(receiver, data_);
        });
        Ok(Box::new(HeadlessDiscovery {
            data,
            supports_vr: init.supports_vr,
            supports_inline: init.supports_inline,
            supports_ar: init.supports_ar,
        }))
    }
}

fn run_loop(receiver: Receiver<MockDeviceMsg>, data: Arc<Mutex<HeadlessDeviceData>>) {
    while let Ok(msg) = receiver.recv() {
        if !data.lock().expect("Mutex poisoned").handle_msg(msg) {
            break;
        }
    }
}

impl DiscoveryAPI<SwapChains> for HeadlessDiscovery {
    fn request_session(
        &mut self,
        mode: SessionMode,
        init: &SessionInit,
        xr: SessionBuilder,
    ) -> Result<Session, Error> {
        if !self.supports_session(mode) {
            return Err(Error::NoMatchingDevice);
        }
        let data = self.data.clone();
        let mut d = data.lock().unwrap();
        let id = d.next_id;
        d.next_id += 1;
        let per_session = PerSessionData {
            id,
            mode,
            clip_planes: Default::default(),
            quitter: Default::default(),
            events: Default::default(),
            needs_vp_update: false,
        };
        d.sessions.push(per_session);

        let granted_features = init.validate(mode, &d.supported_features)?;
        drop(d);
        xr.spawn(move || {
            Ok(HeadlessDevice {
                data,
                id,
                granted_features,
                hit_tests: HitTestList::default(),
            })
        })
    }

    fn supports_session(&self, mode: SessionMode) -> bool {
        if self.data.lock().unwrap().disconnected {
            return false;
        }
        match mode {
            SessionMode::Inline => self.supports_inline,
            SessionMode::ImmersiveVR => self.supports_vr,
            SessionMode::ImmersiveAR => self.supports_ar,
        }
    }
}

fn view<Eye>(
    init: MockViewInit<Eye>,
    viewer: RigidTransform3D<f32, Viewer, Native>,
    clip_planes: ClipPlanes,
) -> View<Eye> {
    let projection = if let Some((l, r, t, b)) = init.fov {
        util::fov_to_projection_matrix(l, r, t, b, clip_planes)
    } else {
        init.projection
    };

    View {
        transform: viewer.pre_transform(&init.transform.inverse()),
        projection,
    }
}

impl HeadlessDevice {
    fn with_per_session<R>(&self, f: impl FnOnce(&mut PerSessionData) -> R) -> R {
        f(self
            .data
            .lock()
            .unwrap()
            .sessions
            .iter_mut()
            .find(|s| s.id == self.id)
            .unwrap())
    }
}

impl DeviceAPI<Surface> for HeadlessDevice {
    fn floor_transform(&self) -> Option<RigidTransform3D<f32, Native, Floor>> {
        self.data.lock().unwrap().floor_transform.clone()
    }

    fn viewports(&self) -> Viewports {
        let d = self.data.lock().unwrap();
        let per_session = d.sessions.iter().find(|s| s.id == self.id).unwrap();
        d.viewports(per_session.mode)
    }

    fn wait_for_animation_frame(&mut self) -> Option<Frame> {
        thread::sleep(std::time::Duration::from_millis(20));
        let mut data = self.data.lock().unwrap();
        let mut frame = data.get_frame(&data.sessions.iter().find(|s| s.id == self.id).unwrap());
        let per_session = data.sessions.iter_mut().find(|s| s.id == self.id).unwrap();
        if per_session.needs_vp_update {
            per_session.needs_vp_update = false;
            let mode = per_session.mode;
            let vp = data.viewports(mode);
            frame.events.push(FrameUpdateEvent::UpdateViewports(vp));
        }
        let events = self.hit_tests.commit_tests();
        frame.events = events;

        if let Some(ref world) = data.world {
            for source in self.hit_tests.tests() {
                let ray = data.native_ray(source.ray, source.space);
                let ray = if let Some(ray) = ray { ray } else { break };
                let hits = world
                    .regions
                    .iter()
                    .filter(|region| source.types.is_type(region.ty))
                    .flat_map(|region| &region.faces)
                    .filter_map(|triangle| triangle.intersect(ray))
                    .map(|space| HitTestResult {
                        space,
                        id: source.id,
                    });
                frame.hit_test_results.extend(hits);
            }
        }

        if data.needs_floor_update {
            frame.events.push(FrameUpdateEvent::UpdateFloorTransform(
                data.floor_transform.clone(),
            ));
            data.needs_floor_update = false;
        }
        Some(frame)
    }

    fn render_animation_frame(&mut self, surface: Surface) -> Surface {
        surface
    }

    fn initial_inputs(&self) -> Vec<InputSource> {
        vec![]
    }

    fn set_event_dest(&mut self, dest: Sender<Event>) {
        self.with_per_session(|s| s.events.upgrade(dest))
    }

    fn quit(&mut self) {
        self.with_per_session(|s| s.events.callback(Event::SessionEnd))
    }

    fn set_quitter(&mut self, quitter: Quitter) {
        self.with_per_session(|s| s.quitter = Some(quitter))
    }

    fn update_clip_planes(&mut self, near: f32, far: f32) {
        self.with_per_session(|s| s.clip_planes.update(near, far));
    }

    fn granted_features(&self) -> &[String] {
        &self.granted_features
    }

    fn request_hit_test(&mut self, source: HitTestSource) {
        self.hit_tests.request_hit_test(source)
    }

    fn cancel_hit_test(&mut self, id: HitTestId) {
        self.hit_tests.cancel_hit_test(id)
    }
}

impl HeadlessMockDiscovery {
    pub fn new() -> HeadlessMockDiscovery {
        HeadlessMockDiscovery {}
    }
}

macro_rules! with_all_sessions {
    ($self:ident, |$s:ident| $e:expr) => {
        for $s in &mut $self.sessions {
            $e;
        }
    };
}

impl HeadlessDeviceData {
    fn get_frame(&self, s: &PerSessionData) -> Frame {
        let time_ns = time::precise_time_ns();
        let views = self.views.clone();

        let pose = self.viewer_origin.map(|transform| {
            let views = if s.mode == SessionMode::Inline {
                Views::Inline
            } else {
                match views {
                    MockViewsInit::Mono(one) => Views::Mono(view(one, transform, s.clip_planes)),
                    MockViewsInit::Stereo(one, two) => Views::Stereo(
                        view(one, transform, s.clip_planes),
                        view(two, transform, s.clip_planes),
                    ),
                }
            };

            ViewerPose { transform, views }
        });
        let inputs = self
            .inputs
            .iter()
            .filter(|i| i.active)
            .map(|i| InputFrame {
                id: i.source.id,
                target_ray_origin: i.pointer,
                grip_origin: i.grip,
                pressed: false,
                squeezed: false,
                hand: None,
            })
            .collect();

        Frame {
            pose,
            inputs,
            events: vec![],
            time_ns,
            sent_time: 0,
            hit_test_results: vec![],
        }
    }

    fn viewports(&self, mode: SessionMode) -> Viewports {
        let vec = if mode == SessionMode::Inline {
            vec![]
        } else {
            match &self.views {
                MockViewsInit::Mono(one) => vec![one.viewport],
                MockViewsInit::Stereo(one, two) => vec![one.viewport, two.viewport],
            }
        };
        Viewports { viewports: vec }
    }

    fn trigger_select(&mut self, id: InputId, kind: SelectKind, event: SelectEvent) {
        for i in 0..self.sessions.len() {
            let frame = self.get_frame(&self.sessions[i]);
            self.sessions[i]
                .events
                .callback(Event::Select(id, kind, event, frame));
        }
    }

    fn handle_msg(&mut self, msg: MockDeviceMsg) -> bool {
        match msg {
            MockDeviceMsg::SetWorld(w) => self.world = Some(w),
            MockDeviceMsg::ClearWorld => self.world = None,
            MockDeviceMsg::SetViewerOrigin(viewer_origin) => {
                self.viewer_origin = viewer_origin;
            }
            MockDeviceMsg::SetFloorOrigin(floor_origin) => {
                self.floor_transform = floor_origin.map(|f| f.inverse());
                self.needs_floor_update = true;
            }
            MockDeviceMsg::SetViews(views) => {
                self.views = views;
                with_all_sessions!(self, |s| {
                    s.needs_vp_update = true;
                })
            }
            MockDeviceMsg::VisibilityChange(v) => {
                with_all_sessions!(self, |s| s.events.callback(Event::VisibilityChange(v)))
            }
            MockDeviceMsg::AddInputSource(init) => {
                self.inputs.push(InputInfo {
                    source: init.source.clone(),
                    pointer: init.pointer_origin,
                    grip: init.grip_origin,
                    active: true,
                    clicking: false,
                });
                with_all_sessions!(self, |s| s
                    .events
                    .callback(Event::AddInput(init.source.clone())))
            }
            MockDeviceMsg::MessageInputSource(id, msg) => {
                if let Some(ref mut input) = self.inputs.iter_mut().find(|i| i.source.id == id) {
                    match msg {
                        MockInputMsg::SetHandedness(h) => {
                            input.source.handedness = h;
                            with_all_sessions!(self, |s| {
                                s.events
                                    .callback(Event::UpdateInput(id, input.source.clone()))
                            });
                        }
                        MockInputMsg::SetProfiles(p) => {
                            input.source.profiles = p;
                            with_all_sessions!(self, |s| {
                                s.events
                                    .callback(Event::UpdateInput(id, input.source.clone()))
                            });
                        }
                        MockInputMsg::SetTargetRayMode(t) => {
                            input.source.target_ray_mode = t;
                            with_all_sessions!(self, |s| {
                                s.events
                                    .callback(Event::UpdateInput(id, input.source.clone()))
                            });
                        }
                        MockInputMsg::SetPointerOrigin(p) => input.pointer = p,
                        MockInputMsg::SetGripOrigin(p) => input.grip = p,
                        MockInputMsg::TriggerSelect(kind, event) => {
                            if !input.active {
                                return true;
                            }
                            let clicking = input.clicking;
                            input.clicking = event == SelectEvent::Start;
                            match event {
                                SelectEvent::Start => {
                                    self.trigger_select(id, kind, event);
                                }
                                SelectEvent::End => {
                                    if clicking {
                                        self.trigger_select(id, kind, SelectEvent::Select);
                                    } else {
                                        self.trigger_select(id, kind, SelectEvent::End);
                                    }
                                }
                                SelectEvent::Select => {
                                    self.trigger_select(id, kind, SelectEvent::Start);
                                    self.trigger_select(id, kind, SelectEvent::Select);
                                }
                            }
                        }
                        MockInputMsg::Disconnect => {
                            if input.active {
                                with_all_sessions!(self, |s| s
                                    .events
                                    .callback(Event::RemoveInput(input.source.id)));
                                input.active = false;
                                input.clicking = false;
                            }
                        }
                        MockInputMsg::Reconnect => {
                            if !input.active {
                                with_all_sessions!(self, |s| s
                                    .events
                                    .callback(Event::AddInput(input.source.clone())));
                                input.active = true;
                            }
                        }
                    }
                }
            }
            MockDeviceMsg::Disconnect(s) => {
                self.disconnected = true;
                with_all_sessions!(self, |s| s.quitter.as_ref().map(|q| q.quit()));
                // notify the client that we're done disconnecting
                let _ = s.send(());
                return false;
            }
        }
        true
    }

    fn native_ray(&self, ray: Ray<ApiSpace>, space: Space) -> Option<Ray<Native>> {
        let origin: RigidTransform3D<f32, ApiSpace, Native> = match space.base {
            BaseSpace::Local => RigidTransform3D::identity(),
            BaseSpace::Floor => self.floor_transform?.inverse().cast_unit(),
            BaseSpace::Viewer => self.viewer_origin?.cast_unit(),
            BaseSpace::TargetRay(id) => self
                .inputs
                .iter()
                .find(|i| i.source.id == id)?
                .pointer?
                .cast_unit(),
            BaseSpace::Grip(id) => self
                .inputs
                .iter()
                .find(|i| i.source.id == id)?
                .grip?
                .cast_unit(),
            BaseSpace::Joint(..) => panic!("Cannot request mocking backend with hands"),
        };
        let space_origin = origin.pre_transform(&space.offset);

        let origin_rigid: RigidTransform3D<f32, ApiSpace, ApiSpace> = ray.origin.into();
        Some(Ray {
            origin: origin_rigid.post_transform(&space_origin).translation,
            direction: space_origin.rotation.transform_vector3d(ray.direction),
        })
    }
}
