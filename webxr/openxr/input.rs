use euclid::RigidTransform3D;
use openxr::d3d::D3D11;
use openxr::{
    self, Action, ActionSet, Binding, FrameState, Instance, Path, Posef, Quaternionf, Session,
    Space, SpaceLocationFlags, Vector3f,
};
use webxr_api::Handedness;
use webxr_api::Input;
use webxr_api::InputFrame;
use webxr_api::InputId;
use webxr_api::Native;
use webxr_api::SelectEvent;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum ClickState {
    Clicking,
    /// it's clicking, but it lost tracking during the click,
    /// so we'll only fire a selectend event
    ClickingLost,
    Done,
}

/// All the information on a single input frame
pub struct Frame {
    pub frame: InputFrame,
    pub select: Option<SelectEvent>,
    pub squeeze: Option<SelectEvent>,
    pub menu_selected: bool,
}

impl ClickState {
    fn update(
        &mut self,
        action: &Action<bool>,
        session: &Session<D3D11>,
    ) -> (/* is_active */ bool, Option<SelectEvent>) {
        let click = action.state(session, Path::NULL).unwrap();

        let select_event = if click.is_active {
            match (click.current_state, *self) {
                (true, ClickState::Done) => {
                    *self = ClickState::Clicking;
                    Some(SelectEvent::Start)
                }
                (false, ClickState::Clicking) => {
                    *self = ClickState::Done;
                    Some(SelectEvent::Select)
                }
                (false, ClickState::ClickingLost) => {
                    *self = ClickState::Done;
                    Some(SelectEvent::End)
                }
                _ => None,
            }
        } else if *self == ClickState::Clicking {
            *self = ClickState::ClickingLost;
            None
        } else {
            None
        };
        (click.is_active, select_event)
    }
}

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
pub struct OpenXRInput {
    id: InputId,
    action_aim_pose: Action<Posef>,
    action_aim_space: Space,
    action_grip_pose: Action<Posef>,
    action_grip_space: Space,
    action_click: Action<bool>,
    action_squeeze: Action<bool>,
    handedness: Handedness,
    click_state: ClickState,
    squeeze_state: ClickState,
    menu_gesture_sustain: u8,
}

fn hand_str(h: Handedness) -> &'static str {
    match h {
        Handedness::Right => "right",
        Handedness::Left => "left",
        _ => panic!("We don't support unknown handedness in openxr"),
    }
}

impl OpenXRInput {
    pub fn new(
        id: InputId,
        handedness: Handedness,
        action_set: &ActionSet,
        session: &Session<D3D11>,
    ) -> Self {
        let hand = hand_str(handedness);
        let action_aim_pose: Action<Posef> = action_set
            .create_action(
                &format!("{}_hand_aim", hand),
                &format!("{} hand aim", hand),
                &[],
            )
            .unwrap();
        let action_aim_space = action_aim_pose
            .create_space(session.clone(), Path::NULL, IDENTITY_POSE)
            .unwrap();
        let action_grip_pose: Action<Posef> = action_set
            .create_action(
                &format!("{}_hand_grip", hand),
                &format!("{} hand grip", hand),
                &[],
            )
            .unwrap();
        let action_grip_space = action_grip_pose
            .create_space(session.clone(), Path::NULL, IDENTITY_POSE)
            .unwrap();
        let action_click: Action<bool> = action_set
            .create_action(
                &format!("{}_hand_click", hand),
                &format!("{} hand click", hand),
                &[],
            )
            .unwrap();
        let action_squeeze: Action<bool> = action_set
            .create_action(
                &format!("{}_hand_squeeze", hand),
                &format!("{} hand squeeze", hand),
                &[],
            )
            .unwrap();
        Self {
            id,
            action_aim_pose,
            action_aim_space,
            action_grip_pose,
            action_grip_space,
            action_click,
            action_squeeze,
            handedness,
            click_state: ClickState::Done,
            squeeze_state: ClickState::Done,
            menu_gesture_sustain: 0,
        }
    }

    pub fn setup_inputs(instance: &Instance, session: &Session<D3D11>) -> (ActionSet, Self, Self) {
        let action_set = instance.create_action_set("hands", "Hands", 0).unwrap();
        let right_hand = OpenXRInput::new(InputId(0), Handedness::Right, &action_set, &session);
        let left_hand = OpenXRInput::new(InputId(1), Handedness::Left, &action_set, &session);

        let mut bindings =
            right_hand.get_bindings(instance, "trigger/value", Some("squeeze/click"));
        bindings.extend(
            left_hand
                .get_bindings(instance, "trigger/value", Some("squeeze/click"))
                .into_iter(),
        );
        let path_controller = instance
            .string_to_path("/interaction_profiles/microsoft/motion_controller")
            .unwrap();
        instance
            .suggest_interaction_profile_bindings(path_controller, &bindings)
            .unwrap();

        let mut bindings = right_hand.get_bindings(instance, "select/click", None);
        bindings.extend(
            left_hand
                .get_bindings(instance, "select/click", None)
                .into_iter(),
        );
        let path_controller = instance
            .string_to_path("/interaction_profiles/khr/simple_controller")
            .unwrap();
        instance
            .suggest_interaction_profile_bindings(path_controller, &bindings)
            .unwrap();
        session.attach_action_sets(&[&action_set]).unwrap();

        (action_set, right_hand, left_hand)
    }

    fn get_bindings(
        &self,
        instance: &Instance,
        select_name: &str,
        squeeze_name: Option<&str>,
    ) -> Vec<Binding> {
        let hand = hand_str(self.handedness);
        let path_aim_pose = instance
            .string_to_path(&format!("/user/hand/{}/input/aim/pose", hand))
            .unwrap();
        let binding_aim_pose = Binding::new(&self.action_aim_pose, path_aim_pose);
        let path_grip_pose = instance
            .string_to_path(&format!("/user/hand/{}/input/grip/pose", hand))
            .unwrap();
        let binding_grip_pose = Binding::new(&self.action_grip_pose, path_grip_pose);
        let path_click = instance
            .string_to_path(&format!("/user/hand/{}/input/{}", hand, select_name))
            .unwrap();
        let binding_click = Binding::new(&self.action_click, path_click);

        let mut ret = vec![binding_aim_pose, binding_grip_pose, binding_click];
        if let Some(squeeze_name) = squeeze_name {
            let path_squeeze = instance
                .string_to_path(&format!("/user/hand/{}/input/{}", hand, squeeze_name))
                .unwrap();
            let binding_squeeze = Binding::new(&self.action_squeeze, path_squeeze);
            ret.push(binding_squeeze);
        }
        ret
    }

    pub fn frame(
        &mut self,
        session: &Session<D3D11>,
        frame_state: &FrameState,
        base_space: &Space,
    ) -> Frame {
        use euclid::Vector3D;
        let target_ray_origin = pose_for(&self.action_aim_space, frame_state, base_space);

        let grip_origin = pose_for(&self.action_grip_space, frame_state, base_space);

        let mut menu_selected = false;
        // Check if the palm is facing up. This is our "menu" gesture.
        if let Some(grip_origin) = grip_origin {
            // The X axis of the grip is perpendicular to the palm, however its
            // direction is the opposite for each hand
            //
            // We obtain a unit vector poking out of the palm
            let x_dir = if let Handedness::Left = self.handedness {
                1.0
            } else {
                -1.0
            };

            // Rotate it by the grip to obtain the desired vector
            let grip_x = grip_origin
                .rotation
                .transform_vector3d(Vector3D::new(x_dir, 0.0, 0.0));

            // Dot product it with the "up" vector to see if it's pointing up
            let angle = grip_x.dot(Vector3D::new(0.0, 1.0, 0.0));

            // If the angle is close enough to 0, its cosine will be
            // close to 1
            if angle > 0.9 {
                self.menu_gesture_sustain += 1;
                if self.menu_gesture_sustain > 60 {
                    menu_selected = true;
                    self.menu_gesture_sustain = 0;
                }
            } else {
                self.menu_gesture_sustain = 0;
            }
        } else {
            self.menu_gesture_sustain = 0;
        }

        let click = self.action_click.state(session, Path::NULL).unwrap();
        let squeeze = self.action_squeeze.state(session, Path::NULL).unwrap();

        let (click_is_active, click_event) = self.click_state.update(&self.action_click, session);
        let (squeeze_is_active, squeeze_event) =
            self.squeeze_state.update(&self.action_squeeze, session);

        let input_frame = InputFrame {
            target_ray_origin,
            id: self.id,
            pressed: click_is_active && click.current_state,
            squeezed: squeeze_is_active && squeeze.current_state,
            grip_origin,
        };

        Frame {
            frame: input_frame,
            select: click_event,
            squeeze: squeeze_event,
            menu_selected,
        }
    }
}

fn pose_for(
    action_space: &Space,
    frame_state: &FrameState,
    base_space: &Space,
) -> Option<RigidTransform3D<f32, Input, Native>> {
    let location = action_space
        .locate(base_space, frame_state.predicted_display_time)
        .unwrap();
    let pose_valid = location
        .location_flags
        .intersects(SpaceLocationFlags::POSITION_VALID | SpaceLocationFlags::ORIENTATION_VALID);
    if pose_valid {
        Some(super::transform(&location.pose))
    } else {
        None
    }
}
