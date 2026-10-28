//! Setup wording shared by the build script and `githelper --doctor`.
//!
//! This file stays free of third-party crates so `build.rs` can include it.
//! Pass/fail decisions for doctor live in `doctor`; the strings here are the
//! single source for the fixes both paths print.
//!
//! The build script and the binary each use a different subset of this module.
#![allow(dead_code)]

pub const MACOS_FIX: &str = "Upgrade to macOS 26 or newer on an Apple Intelligence-capable Mac.";

pub const XCODE_FIX: &str = "Install Xcode 26 or newer, then select it with `sudo xcode-select -s /Applications/Xcode.app/Contents/Developer` and rebuild.";

pub const SDK_FIX: &str = "Select an Xcode 26 or newer installation that includes the macOS 26 SDK: `sudo xcode-select -s /Applications/Xcode.app/Contents/Developer`.";

pub const LICENSE_FIX: &str =
    "Accept the Xcode license with `sudo xcodebuild -license`, then rebuild.";

pub const APPLE_INTELLIGENCE_FIX: &str = "Enable Apple Intelligence in System Settings and wait for the on-device model to finish downloading.";

pub const GIT_FIX: &str = "Install Git and make sure `git` is on your PATH.";

pub const REPO_FIX: &str =
    "Change to a Git repository or pass --repo PATH. This check is optional for --doctor.";

pub const DOCTOR_COMMAND_HINT: &str =
    "Run `githelper --doctor` to check macOS 26, Xcode 26, and Apple Intelligence.";

/// Model status used when doctor runs somewhere other than macOS.
pub const MODEL_UNSUPPORTED_PLATFORM: &str = "Requires macOS 26 or newer on an Apple Intelligence-capable Mac with Apple Intelligence enabled and its on-device model downloaded.";

/// Runtime error used when translation is attempted off macOS.
pub const MODEL_RUNTIME_UNSUPPORTED: &str = "Apple's on-device model requires macOS 26 or newer on an Apple Intelligence-capable Mac. There is no cloud fallback.";

pub fn with_doctor_hint(message: &str) -> String {
    let message = message.trim();
    if message.contains("githelper --doctor") {
        message.to_string()
    } else if message.is_empty() {
        DOCTOR_COMMAND_HINT.to_string()
    } else {
        format!("{message} {DOCTOR_COMMAND_HINT}")
    }
}

/// Leading numeric component of a version token (`26.1.2` → 26, `26A100` → 26).
pub fn major_version(version: &str) -> Option<u32> {
    let token = version.split_whitespace().next()?;
    let digits: String = token.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

pub fn xcode_version_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| line.starts_with("Xcode "))
        .map(str::to_owned)
}

pub fn xcode_major(text: &str) -> Option<u32> {
    let line = xcode_version_line(text)?;
    major_version(line.strip_prefix("Xcode ")?)
}

pub fn sdk_version_token(text: &str) -> Option<String> {
    text.lines().map(str::trim).find_map(|line| {
        let token = line.split_whitespace().next()?;
        major_version(token)?;
        Some(token.to_owned())
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toolchain {
    pub xcode_line: String,
    pub sdk: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolchainIssue {
    Missing { detail: String },
    TooOld { found: String },
    SdkTooOld { xcode_line: String, sdk: String },
    License,
    UnreadableXcode { detail: String },
    UnreadableSdk { detail: String },
}

impl ToolchainIssue {
    pub fn message(&self) -> String {
        match self {
            Self::Missing { detail } => {
                let detail = excerpt(detail, 8);
                if detail.is_empty() {
                    "The Swift toolchain is missing.".to_string()
                } else {
                    format!("The Swift toolchain is missing ({detail}).")
                }
            }
            Self::TooOld { found } => format!("Xcode 26 or newer is required (found {found})."),
            Self::SdkTooOld { xcode_line, sdk } => {
                format!("The macOS 26 SDK is required (found SDK {sdk} from {xcode_line}).")
            }
            Self::License => {
                "Xcode is installed but its license has not been accepted.".to_string()
            }
            Self::UnreadableXcode { detail } => {
                let detail = excerpt(detail, 8);
                if detail.is_empty() {
                    "Could not read an Xcode version from `xcodebuild -version`.".to_string()
                } else {
                    format!("Could not read an Xcode version from `xcodebuild -version`. {detail}")
                }
            }
            Self::UnreadableSdk { detail } => {
                let detail = excerpt(detail, 6);
                if detail.is_empty() {
                    "Could not read the macOS SDK version from `xcrun --sdk macosx --show-sdk-version`."
                        .to_string()
                } else {
                    format!(
                        "Could not read the macOS SDK version from `xcrun --sdk macosx --show-sdk-version`. {detail}"
                    )
                }
            }
        }
    }

    /// Primary fix for this failure. Doctor prints it as the check hint.
    pub fn hint(&self) -> &'static str {
        match self {
            Self::License => LICENSE_FIX,
            Self::SdkTooOld { .. } | Self::UnreadableSdk { .. } => SDK_FIX,
            _ => XCODE_FIX,
        }
    }

    /// Single build-script error. Names macOS 26, Xcode 26, xcode-select, and Apple Intelligence.
    pub fn build_error(&self) -> String {
        let mut parts = vec![self.message(), self.hint().to_string()];
        for fix in [MACOS_FIX, XCODE_FIX, APPLE_INTELLIGENCE_FIX] {
            if !parts.iter().any(|part| part.contains(fix)) {
                parts.push(fix.to_string());
            }
        }
        parts.join(" ")
    }
}

pub fn validate_toolchain(
    xcodebuild_text: &str,
    sdk_text: &str,
) -> Result<Toolchain, ToolchainIssue> {
    let combined = format!("{xcodebuild_text}\n{sdk_text}");
    if mentions_license(&combined) {
        return Err(ToolchainIssue::License);
    }
    let Some(xcode_line) = xcode_version_line(xcodebuild_text) else {
        return Err(ToolchainIssue::UnreadableXcode {
            detail: xcodebuild_text.to_string(),
        });
    };
    let Some(major) = xcode_major(xcodebuild_text) else {
        return Err(ToolchainIssue::UnreadableXcode {
            detail: xcodebuild_text.to_string(),
        });
    };
    if major < 26 {
        return Err(ToolchainIssue::TooOld { found: xcode_line });
    }
    let Some(sdk) = sdk_version_token(sdk_text) else {
        return Err(ToolchainIssue::UnreadableSdk {
            detail: sdk_text.to_string(),
        });
    };
    let Some(sdk_major) = major_version(&sdk) else {
        return Err(ToolchainIssue::UnreadableSdk {
            detail: sdk_text.to_string(),
        });
    };
    if sdk_major < 26 {
        return Err(ToolchainIssue::SdkTooOld { xcode_line, sdk });
    }
    Ok(Toolchain { xcode_line, sdk })
}

pub fn mentions_license(text: &str) -> bool {
    text.to_ascii_lowercase().contains("license")
}

pub fn ensure_native_build(host: &str, target: &str) -> Result<(), String> {
    if host == target {
        Ok(())
    } else {
        Err(format!(
            "Build natively on your Mac (host {host}, target {target}); cross-compiling the Apple bridge is unsupported. {MACOS_FIX} {XCODE_FIX} {APPLE_INTELLIGENCE_FIX}"
        ))
    }
}

pub fn bridge_compile_failed(output: &str) -> String {
    let detail = excerpt(output, 20);
    let body = if detail.is_empty() {
        "swiftc produced no output.".to_string()
    } else {
        detail
    };
    format!(
        "Apple bridge compilation failed. Use Xcode 26 or newer with the macOS 26 SDK.\n{body}\n{XCODE_FIX} {MACOS_FIX} {APPLE_INTELLIGENCE_FIX}"
    )
}

pub fn excerpt(text: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect();
    if max_lines == 0 {
        return String::new();
    }
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn major_version_reads_the_leading_number() {
        assert_eq!(major_version("26.0.1"), Some(26));
        assert_eq!(major_version("26"), Some(26));
        assert_eq!(major_version("  15.6 "), Some(15));
        assert_eq!(major_version("26A5234a"), Some(26));
        assert_eq!(major_version("Xcode"), None);
        assert_eq!(major_version(""), None);
    }

    #[test]
    fn toolchain_accepts_xcode_26_and_rejects_older_or_unreadable_tools() {
        let ready = validate_toolchain("Xcode 26.0\nBuild version 17A400\n", "26.0\n").unwrap();
        assert_eq!(ready.xcode_line, "Xcode 26.0");
        assert_eq!(ready.sdk, "26.0");

        let beta = validate_toolchain("Xcode 26.1 Beta\n", "26.1").unwrap();
        assert_eq!(beta.sdk, "26.1");

        assert_eq!(
            validate_toolchain("Xcode 16.4\nBuild version 16F6\n", "15.5"),
            Err(ToolchainIssue::TooOld {
                found: "Xcode 16.4".into()
            })
        );
        assert_eq!(
            validate_toolchain("Xcode 26.0\n", "15.5"),
            Err(ToolchainIssue::SdkTooOld {
                xcode_line: "Xcode 26.0".into(),
                sdk: "15.5".into()
            })
        );
        assert_eq!(
            validate_toolchain("not xcode\n", "26.0"),
            Err(ToolchainIssue::UnreadableXcode {
                detail: "not xcode\n".into()
            })
        );
        assert!(matches!(
            validate_toolchain("Xcode 26.0\n", "macosx"),
            Err(ToolchainIssue::UnreadableSdk { .. })
        ));
        assert_eq!(
            validate_toolchain("You have not agreed to the Xcode license agreements.\n", ""),
            Err(ToolchainIssue::License)
        );
    }

    #[test]
    fn build_errors_name_macos_xcode_selection_and_apple_intelligence() {
        let issues = [
            ToolchainIssue::Missing {
                detail: "xcrun: No such file or directory (os error 2)".into(),
            },
            ToolchainIssue::TooOld {
                found: "Xcode 16.4".into(),
            },
            ToolchainIssue::SdkTooOld {
                xcode_line: "Xcode 26.0".into(),
                sdk: "15.5".into(),
            },
            ToolchainIssue::License,
            ToolchainIssue::UnreadableXcode {
                detail: String::new(),
            },
            ToolchainIssue::UnreadableSdk {
                detail: "no sdk".into(),
            },
        ];
        for issue in issues {
            let error = issue.build_error();
            assert!(error.contains("macOS 26"), "{error}");
            assert!(error.contains("Xcode 26"), "{error}");
            assert!(error.contains("xcode-select"), "{error}");
            assert!(error.contains("Apple Intelligence"), "{error}");
            assert!(!error.contains("panicked"), "{error}");
        }
        let license = ToolchainIssue::License.build_error();
        assert!(license.contains("sudo xcodebuild -license"));
        let cross = ensure_native_build("aarch64-apple-darwin", "x86_64-apple-darwin").unwrap_err();
        assert!(cross.contains("cross-compiling"));
        assert!(cross.contains("xcode-select"));
        assert!(ensure_native_build("aarch64-apple-darwin", "aarch64-apple-darwin").is_ok());
        let compile = bridge_compile_failed(
            "error: unable to resolve module dependency: 'FoundationModels'\n",
        );
        assert!(compile.contains("FoundationModels"));
        assert!(compile.contains("xcode-select"));
        assert!(compile.contains("Apple Intelligence"));
        assert!(compile.contains("macOS 26"));
    }

    #[test]
    fn doctor_hint_is_appended_once() {
        let hinted = with_doctor_hint("Apple model unavailable.");
        assert!(hinted.contains("githelper --doctor"));
        assert_eq!(with_doctor_hint(&hinted), hinted);
        assert_eq!(with_doctor_hint("  "), DOCTOR_COMMAND_HINT);
    }
}
