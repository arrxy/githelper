use std::{env, path::PathBuf, process::Command};

#[path = "src/setup_text.rs"]
mod setup_text;

fn main() {
    println!("cargo:rerun-if-changed=apple/ModelBridge.swift");
    println!("cargo:rerun-if-changed=src/setup_text.rs");
    println!("cargo:rerun-if-env-changed=DEVELOPER_DIR");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    if let Err(message) = compile_bridge() {
        let warning = message.replace(['\r', '\n'], " ");
        println!("cargo:warning={warning}");
        eprintln!("error: {message}");
        std::process::exit(1);
    }
}

fn compile_bridge() -> Result<(), String> {
    let host = env::var("HOST").map_err(|_| "Cargo did not set HOST.".to_string())?;
    let target = env::var("TARGET").map_err(|_| "Cargo did not set TARGET.".to_string())?;
    setup_text::ensure_native_build(&host, &target)?;

    // `xcrun --find` only proves swiftc exists. Compile through
    // `xcrun --sdk macosx swiftc` so the macOS SDK is selected explicitly.
    let swiftc = capture("xcrun", &["--find", "swiftc"]).map_err(missing_toolchain)?;
    if swiftc.trim().is_empty() {
        return Err(missing_toolchain(
            "xcrun --find swiftc returned an empty path",
        ));
    }

    let xcodebuild = capture("xcodebuild", &["-version"]).map_err(command_issue)?;
    let sdk =
        capture("xcrun", &["--sdk", "macosx", "--show-sdk-version"]).map_err(command_issue)?;
    if let Err(issue) = setup_text::validate_toolchain(&xcodebuild, &sdk) {
        return Err(issue.build_error());
    }

    let out_dir = env::var_os("OUT_DIR").ok_or_else(|| "Cargo did not set OUT_DIR.".to_string())?;
    let output = PathBuf::from(out_dir).join("githelper-model");
    let compiled = Command::new("xcrun")
        .args([
            "--sdk",
            "macosx",
            "swiftc",
            "-parse-as-library",
            "-O",
            "apple/ModelBridge.swift",
            "-o",
        ])
        .arg(&output)
        .output()
        .map_err(|error| missing_toolchain(error.to_string()))?;
    if !compiled.status.success() {
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&compiled.stdout),
            String::from_utf8_lossy(&compiled.stderr)
        );
        return Err(setup_text::bridge_compile_failed(&combined));
    }
    Ok(())
}

fn missing_toolchain(detail: impl AsRef<str>) -> String {
    setup_text::ToolchainIssue::Missing {
        detail: detail.as_ref().to_string(),
    }
    .build_error()
}

fn command_issue(detail: String) -> String {
    if setup_text::mentions_license(&detail) {
        setup_text::ToolchainIssue::License.build_error()
    } else {
        missing_toolchain(detail)
    }
}

fn capture(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("{program}: {error}"))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.success() {
        Ok(text)
    } else if text.trim().is_empty() {
        Err(format!("{program} exited with {}", output.status))
    } else {
        Err(format!(
            "{program} exited with {}: {}",
            output.status,
            text.trim()
        ))
    }
}
