use openxr::d3d::D3D11;
use openxr::{
    self, Action, ActionSet, ActionState, Binding, FrameState, Instance, Path, Posef, Quaternionf,
    Session, Space, SpaceLocationFlags, Vector3f,
};
use webxr_api::Handedness;
use webxr_api::InputFrame;
use webxr_api::InputId;

pub struct OpenXRInput {
    id: InputId,
    action_pose: Action<Posef>,
    action_click: Action<bool>,
}

impl OpenXRInput {
    pub fn new(id: InputId, hand: Handedness, instance: &Instance, action_set: &ActionSet) -> Self {
        let hand = match hand {
            Handedness::Right => "right",
            Handedness::Left => "left",
            _ => panic!("We don't support unknown handedness in openxr"),
        };
        let action_pose: Action<Posef> = action_set
            .create_action(&format!("{}_hand", hand), &format!("{} hand", hand), &[])
            .unwrap();
        let action_click: Action<bool> = action_set
            .create_action(
                &format!("{}_hand_click", hand),
                &format!("{} hand click", hand),
                &[],
            )
            .unwrap();
        let path_pose = instance
            .string_to_path(&format!("/user/hand/{}/input/aim/pose", hand))
            .unwrap();
        let binding_pose = Binding::new(&action_pose, path_pose);
        let path_click = instance
            .string_to_path(&format!("/user/hand/{}/input/select/click", hand))
            .unwrap();
        let binding_click = Binding::new(&action_click, path_click);
        let path_controller = instance
            .string_to_path("/interaction_profiles/khr/simple_controller")
            .unwrap();
        instance
            .suggest_interaction_profile_bindings(path_controller, &[binding_pose, binding_click])
            .unwrap();
        Self {
            id,
            action_pose,
            action_click,
        }
    }

    pub fn frame(
        &self,
        session: &Session<D3D11>,
        frame_state: &FrameState,
        base_space: &Space,
    ) -> (InputFrame, ActionState<bool>) {
        let identity_pose = Posef {
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
        let hand_space = self
            .action_pose
            .create_space(session.clone(), Path::NULL, identity_pose)
            .unwrap();
        let location = hand_space
            .locate(base_space, frame_state.predicted_display_time)
            .unwrap();

        let pose_valid = location
            .location_flags
            .intersects(SpaceLocationFlags::POSITION_VALID | SpaceLocationFlags::ORIENTATION_VALID);
        let target_ray_origin = if pose_valid {
            Some(super::transform(&location.pose))
        } else {
            None
        };

        let click = self.action_click.state(session, Path::NULL).unwrap();

        let input_frame = InputFrame {
            target_ray_origin,
            id: self.id,
            pressed: click.is_active && click.current_state,
            grip_origin: None,
        };

        (input_frame, click)
    }
}
