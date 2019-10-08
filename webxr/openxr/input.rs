use euclid::RigidTransform3D;
use openxr::d3d::D3D11;
use openxr::{
    self, Action, ActionSet, ActionState, Binding, FrameState, Instance, Path, Posef, Quaternionf,
    Session, Space, SpaceLocationFlags, Vector3f,
};
use webxr_api::Handedness;
use webxr_api::Input;
use webxr_api::InputFrame;
use webxr_api::InputId;
use webxr_api::Native;

pub struct OpenXRInput {
    id: InputId,
    action_aim_pose: Action<Posef>,
    action_grip_pose: Action<Posef>,
    action_click: Action<bool>,
    hand: &'static str,
}

impl OpenXRInput {
    pub fn new(id: InputId, hand: Handedness, action_set: &ActionSet) -> Self {
        let hand = match hand {
            Handedness::Right => "right",
            Handedness::Left => "left",
            _ => panic!("We don't support unknown handedness in openxr"),
        };
        let action_aim_pose: Action<Posef> = action_set
            .create_action(
                &format!("{}_hand_aim", hand),
                &format!("{} hand aim", hand),
                &[],
            )
            .unwrap();
        let action_grip_pose: Action<Posef> = action_set
            .create_action(
                &format!("{}_hand_grip", hand),
                &format!("{} hand grip", hand),
                &[],
            )
            .unwrap();
        let action_click: Action<bool> = action_set
            .create_action(
                &format!("{}_hand_click", hand),
                &format!("{} hand click", hand),
                &[],
            )
            .unwrap();
        Self {
            id,
            action_aim_pose,
            action_grip_pose,
            action_click,
            hand,
        }
    }

    pub fn get_bindings(&self, instance: &Instance) -> Vec<Binding> {
        let path_aim_pose = instance
            .string_to_path(&format!("/user/hand/{}/input/aim/pose", self.hand))
            .unwrap();
        let binding_aim_pose = Binding::new(&self.action_aim_pose, path_aim_pose);
        let path_grip_pose = instance
            .string_to_path(&format!("/user/hand/{}/input/grip/pose", self.hand))
            .unwrap();
        let binding_grip_pose = Binding::new(&self.action_grip_pose, path_grip_pose);
        let path_click = instance
            .string_to_path(&format!("/user/hand/{}/input/select/click", self.hand))
            .unwrap();
        let binding_click = Binding::new(&self.action_click, path_click);

        vec![binding_aim_pose, binding_grip_pose, binding_click]
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
        let target_ray_origin = pose_for(
            &self.action_aim_pose,
            session,
            frame_state,
            base_space,
            identity_pose,
        );

        let grip_origin = pose_for(
            &self.action_grip_pose,
            session,
            frame_state,
            base_space,
            identity_pose,
        );

        let click = self.action_click.state(session, Path::NULL).unwrap();

        let input_frame = InputFrame {
            target_ray_origin,
            id: self.id,
            pressed: click.is_active && click.current_state,
            grip_origin,
        };

        (input_frame, click)
    }
}

fn pose_for(
    action: &Action<Posef>,
    session: &Session<D3D11>,
    frame_state: &FrameState,
    base_space: &Space,
    identity_pose: Posef,
) -> Option<RigidTransform3D<f32, Input, Native>> {
    let action_space = action
        .create_space(session.clone(), Path::NULL, identity_pose)
        .unwrap();
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
