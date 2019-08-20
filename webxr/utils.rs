#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
pub struct ClipPlanes {
    pub near: f32,
    pub far: f32,
    /// Was there an update that needs propagation to the client?
    update: bool,
}

impl Default for ClipPlanes {
    fn default() -> Self {
        ClipPlanes {
            near: 0.1,
            far: 1000.,
            update: false,
        }
    }
}

impl ClipPlanes {
    pub fn update(&mut self, near: f32, far: f32) {
        self.near = near;
        self.far = far;
        self.update = true;
    }

    /// Checks for and clears the pending update flag
    pub fn recently_updated(&mut self) -> bool {
        if self.update {
            self.update = false;
            true
        } else {
            false
        }
    }
}
