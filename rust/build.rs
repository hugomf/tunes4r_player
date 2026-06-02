fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();

    if target.contains("android") {
        eprintln!("[build] Configuring for Android target: {}", target);
        println!("cargo:rustc-link-lib=c++_shared");
        println!("cargo:rustc-link-arg=-fexceptions");
        println!("cargo:rustc-link-arg=-frtti");
        println!("cargo:rustc-link-arg=-Wl,-z,max-page-size=16384");
        println!("cargo:rustc-link-arg=-Wl,-z,common-page-size=16384");
    }

    if target.contains("ios") {
        eprintln!("[build] Configuring for iOS target: {}", target);
        println!("cargo:rustc-link-lib=c++");
        println!("cargo:rustc-link-arg=-framework");
        println!("cargo:rustc-link-arg=Security");
        println!("cargo:rustc-link-arg=-framework");
        println!("cargo:rustc-link-arg=CoreFoundation");
        println!("cargo:rustc-link-arg=-framework");
        println!("cargo:rustc-link-arg=AVFAudio");
        println!("cargo:rustc-link-arg=-framework");
        println!("cargo:rustc-link-arg=AudioToolbox");
        println!("cargo:rustc-link-arg=-framework");
        println!("cargo:rustc-link-arg=CoreAudio");
        println!("cargo:rustc-link-arg=-framework");
        println!("cargo:rustc-link-arg=Foundation");

        println!("cargo:rustc-link-search=native=/tmp/opus-build/install-ios/lib");
        println!("cargo:rustc-link-lib=opus");

        println!("cargo:rustc-link-search=native=/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/clang/17/lib/darwin");
        println!("cargo:rustc-link-lib=clang_rt.ios");

        println!("cargo:rerun-if-env-changed=TARGET");
    }

    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=src/models.rs");
    println!("cargo:rerun-if-changed=src/playback.rs");
    println!("cargo:rerun-if-changed=src/dsp.rs");
    println!("cargo:rerun-if-changed=src/ffi.rs");

    #[cfg(feature = "rustls-platform-verifier")]
    {
        let crate_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let cbindgen_config = std::path::Path::new(&crate_dir).join("cbindgen.toml");

        if cbindgen_config.exists() {
            println!("cargo:rerun-if-changed={}", cbindgen_config.display());

            let output = std::process::Command::new("cbindgen")
                .args([
                    "--config",
                    cbindgen_config.to_str().unwrap(),
                    "--output",
                    "include/tunes4r.h",
                ])
                .current_dir(&crate_dir)
                .output();

            match output {
                Ok(result) => {
                    if result.status.success() {
                        println!("[build] cbindgen: header generated successfully");
                    } else {
                        let stderr = String::from_utf8_lossy(&result.stderr);
                        eprintln!("[build] cbindgen warning: {}", stderr);
                    }
                }
                Err(e) => {
                    eprintln!(
                        "[build] cbindgen not found: {}. Install with: cargo install cbindgen",
                        e
                    );
                }
            }
        }
    }
}
