/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use gl_generator::{Api, Fallbacks, Profile, Registry};
use std::env;
use std::fs;
use std::fs::File;
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

    // GLES 2.0 bindings
    if cfg!(feature = "gles") {
        let mut file = File::create(&Path::new(&out_dir).join("gles_bindings.rs")).unwrap();
        let gles_reg = Registry::new(Api::Gles2, (3, 0), Profile::Core, Fallbacks::All, []);
        gles_reg
            .write_bindings(gl_generator::StaticGenerator, &mut file)
            .unwrap();
    }

    let out_dir = env::var("OUT_DIR").unwrap();

    // GLES 2.0 bindings
    let mut file = File::create(&Path::new(&out_dir).join("gles_bindings.rs")).unwrap();
    let gles_reg = Registry::new(Api::Gles2, (3, 0), Profile::Core, Fallbacks::All, []);
    gles_reg
        .write_bindings(gl_generator::StaticGenerator, &mut file)
        .unwrap();
}
