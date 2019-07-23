use gl_generator::{Api, Fallbacks, Profile, Registry};
use std::env;
use std::fs::{self, File};
use std::path::Path;

fn main() {
    // Copy AARs
    if let Ok(aar_out_dir) = env::var("AAR_OUT_DIR") {
        fs::copy(
            &Path::new("googlevr/aar/GVRService.aar"),
            &Path::new(&aar_out_dir).join("GVRService.aar"),
        )
        .unwrap();
    }

    if !cfg!(feature = "googlevr") {
        return;
    }

    let out_dir = env::var("OUT_DIR").unwrap();

    // GLES 2.0 bindings
    let mut file = File::create(&Path::new(&out_dir).join("gles_bindings.rs")).unwrap();
    let gles_reg = Registry::new(Api::Gles2, (3, 0), Profile::Core, Fallbacks::All, []);
    gles_reg
        .write_bindings(gl_generator::StaticGenerator, &mut file)
        .unwrap();
}
