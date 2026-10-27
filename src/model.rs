use crate::{plan::Proposal, repository::Context};
use serde::{Deserialize, Serialize};

pub struct AppleModel;

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelStatus {
    pub available: bool,
    pub detail: String,
}

#[derive(Serialize)]
struct Request<'a> {
    mode: &'a str,
    request: Option<&'a str>,
    context: Option<&'a Context>,
}

#[derive(Deserialize)]
struct Response {
    kind: String,
    plan: Option<Proposal>,
    available: Option<bool>,
    detail: Option<String>,
    error: Option<String>,
}

impl AppleModel {
    pub fn status(&self) -> Result<ModelStatus, Box<dyn std::error::Error>> {
        let response = invoke(&Request {
            mode: "doctor",
            request: None,
            context: None,
        })?;
        if response.kind != "status" {
            return Err("unexpected model status response".into());
        }
        Ok(ModelStatus {
            available: response
                .available
                .ok_or("model status missing availability")?,
            detail: response.detail.ok_or("model status missing detail")?,
        })
    }

    pub fn translate(
        &self,
        request: &str,
        context: &Context,
    ) -> Result<Proposal, Box<dyn std::error::Error>> {
        let response = invoke(&Request {
            mode: "translate",
            request: Some(request),
            context: Some(context),
        })?;
        if response.kind != "plan" {
            return Err("unexpected model translation response".into());
        }
        response.plan.ok_or_else(|| "model returned no plan".into())
    }
}

fn invoke(request: &Request<'_>) -> Result<Response, Box<dyn std::error::Error>> {
    let response = platform_invoke(request)?;
    if let Some(error) = &response.error {
        return Err(error.clone().into());
    }
    Ok(response)
}

fn platform_invoke(request: &Request<'_>) -> Result<Response, Box<dyn std::error::Error>> {
    #[cfg(not(target_os = "macos"))]
    {
        if request.mode == "doctor" {
            return Ok(Response {
                kind: "status".into(), plan: None, available: Some(false), error: None,
                detail: Some("Requires macOS 26+ on an Apple Intelligence-capable Mac with Apple Intelligence enabled and its model downloaded.".into()),
            });
        }
        Err("Apple's on-device model requires macOS 26+; run githelper on a supported Mac. There is no cloud fallback.".into())
    }
    #[cfg(target_os = "macos")]
    {
        use std::{
            io::Write,
            os::unix::fs::PermissionsExt,
            process::{Command, Stdio},
        };
        const HELPER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/githelper-model"));
        // Embed the compiled Swift helper so the installed Rust executable is self-contained.
        let directory = tempfile::tempdir()?;
        let helper = directory.path().join("githelper-model");
        std::fs::write(&helper, HELPER)?;
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700))?;
        let mut child = Command::new(helper)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut input = child.stdin.take().ok_or("could not open model input")?;
        input.write_all(&serde_json::to_vec(request)?)?;
        drop(input);
        let output = child.wait_with_output()?;
        let response: Response = serde_json::from_slice(&output.stdout).map_err(|e| {
            format!(
                "could not read Apple model response ({e}); helper: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )
        })?;
        if !output.status.success() {
            return Err("Apple model helper failed".into());
        }
        Ok(response)
    }
}
