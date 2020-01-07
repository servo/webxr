/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use gl_generator::{Api, Fallbacks, Profile, Registry};
use std::env;
use std::fs;
use std::fs::File;
use std::path::Path;
use std::path::PathBuf;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();

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

    // EGL bindings
    if cfg!(feature = "egl") {
        let mut file = File::create(&Path::new(&out_dir).join("egl_bindings.rs")).unwrap();
        Registry::new(Api::Egl, (1, 5), Profile::Core, Fallbacks::All, [])
            .write_bindings(gl_generator::StructGenerator, &mut file)
            .unwrap();
        if cfg!(target_os = "windows") {
            println!("cargo:rustc-link-lib=libEGL");
        } else {
            println!("cargo:rustc-link-lib=EGL");
        }
    }

    // Magicleap C API
    if cfg!(feature = "magicleap") {
        let mut builder = bindgen::Builder::default()
            .header("magicleap/magicleap_c_api.h")
            .blacklist_type("MLResult")
            .size_t_is_usize(true)
            .derive_default(true)
            .rustfmt_bindings(true);

        if let Ok(mlsdk) = env::var("MAGICLEAP_SDK") {
            builder = builder.clang_args(&[
                format!("--no-standard-includes"),
                format!("--sysroot={}", mlsdk),
                format!("-I{}/include", mlsdk),
                format!("-I{}/lumin/usr/include", mlsdk),
                format!("-I{}/tools/toolchains/lib64/clang/3.8/include", mlsdk),
            ]);
        }

        if let Ok(flags) = env::var("CFLAGS") {
            for flag in flags.split_whitespace() {
                builder = builder.clang_arg(flag);
            }
        }

        if let Ok(flags) = env::var("CLANGFLAGS") {
            for flag in flags.split_whitespace() {
                builder = builder.clang_arg(flag);
            }
        }

        let bindings = builder.generate().expect("Unable to generate bindings");
        let out_path = PathBuf::from(&out_dir);
        bindings
            .write_to_file(out_path.join("magicleap_c_api.rs"))
            .expect("Couldn't write bindings!");
    }
}
