use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=apple/ModelBridge.swift");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    assert_eq!(
        env::var("HOST").unwrap(),
        env::var("TARGET").unwrap(),
        "Build natively on your Mac; cross-compiling the Apple bridge is unsupported."
    );
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("githelper-model");
    let status = Command::new("xcrun")
        .args([
            "swiftc",
            "-parse-as-library",
            "-O",
            "apple/ModelBridge.swift",
            "-o",
        ])
        .arg(&output)
        .status()
        .expect("Install and select Xcode 26 or newer to compile the Apple model bridge.");
    assert!(
        status.success(),
        "Apple bridge compilation failed. Use Xcode 26+ with the macOS 26 SDK."
    );
}
