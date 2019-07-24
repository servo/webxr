use webxr_api::Discovery;
use webxr_api::Error;
use webxr_api::Session;
use webxr_api::SessionBuilder;
use webxr_api::SessionMode;

use super::device::GoogleVRDevice;

pub struct GoogleVRDiscovery {}

impl GoogleVRDiscovery {
    pub fn new() -> Self {
        GoogleVRDiscovery {}
    }
}
impl Discovery for GoogleVRDiscovery {
    fn request_session(&mut self, mode: SessionMode, xr: SessionBuilder) -> Result<Session, Error> {
        if self.supports_session(mode) {
            xr.spawn(move || GoogleVRDevice::new())
        } else {
            Err(Error::NoMatchingDevice)
        }
    }

    fn supports_session(&self, mode: SessionMode) -> bool {
        mode == SessionMode::ImmersiveVR
    }
}
