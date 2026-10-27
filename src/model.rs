use crate::{commit_message::CommitMessage, plan::Proposal, repository::Context};
use serde::{Deserialize, Serialize};

pub struct AppleModel;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClarificationTurn {
    pub question: String,
    pub answer: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelStatus {
    pub available: bool,
    pub detail: String,
}

#[derive(Serialize)]
struct Request<'a> {
    mode: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    request: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<&'a Context>,
    #[serde(skip_serializing_if = "Option::is_none")]
    history: Option<&'a [ClarificationTurn]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    staged_diff: Option<&'a str>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Response {
    #[serde(rename = "plan")]
    Plan { plan: Proposal },
    #[serde(rename = "commit_message")]
    CommitMessage { commit_message: CommitMessage },
    #[serde(rename = "status")]
    Status { available: bool, detail: String },
    #[serde(rename = "error")]
    Error { error: String },
}

impl AppleModel {
    pub fn status(&self) -> Result<ModelStatus, Box<dyn std::error::Error>> {
        match invoke(&Request {
            mode: "doctor",
            request: None,
            context: None,
            history: None,
            staged_diff: None,
        })? {
            Response::Status { available, detail } => Ok(ModelStatus { available, detail }),
            _ => Err("unexpected model status response".into()),
        }
    }

    pub fn translate(
        &self,
        request: &str,
        context: &Context,
        history: &[ClarificationTurn],
    ) -> Result<Proposal, Box<dyn std::error::Error>> {
        match invoke(&Request {
            mode: "translate",
            request: Some(request),
            context: Some(context),
            history: Some(history),
            staged_diff: None,
        })? {
            Response::Plan { plan } => Ok(plan),
            _ => Err("unexpected model translation response".into()),
        }
    }

    pub fn draft_commit(
        &self,
        staged_diff: &str,
    ) -> Result<CommitMessage, Box<dyn std::error::Error>> {
        match invoke(&Request {
            mode: "draftCommit",
            request: None,
            context: None,
            history: None,
            staged_diff: Some(staged_diff),
        })? {
            Response::CommitMessage { commit_message } => commit_message.validate(),
            _ => Err("unexpected model commit message response".into()),
        }
    }
}

fn invoke(request: &Request<'_>) -> Result<Response, Box<dyn std::error::Error>> {
    match platform_invoke(request)? {
        Response::Error { error } => Err(error.into()),
        response => Ok(response),
    }
}

fn platform_invoke(request: &Request<'_>) -> Result<Response, Box<dyn std::error::Error>> {
    #[cfg(not(target_os = "macos"))]
    {
        if request.mode == "doctor" {
            return Ok(Response::Status {
                available: false,
                detail: "Requires macOS 26+ on an Apple Intelligence-capable Mac with Apple Intelligence enabled and its model downloaded.".into(),
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
        // Each invocation gets a fresh helper; typed history replays prior clarification.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_request_contains_only_staged_patch() {
        let request = Request {
            mode: "draftCommit",
            request: None,
            context: None,
            history: None,
            staged_diff: Some("diff --git a/a b/a\n+hello"),
        };
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({
                "mode": "draftCommit", "staged_diff": "diff --git a/a b/a\n+hello"
            })
        );
    }

    #[test]
    fn translation_replays_typed_history_and_original_request() {
        let history = [ClarificationTurn {
            question: "Message?".into(),
            answer: "Fix docs".into(),
        }];
        let request = Request {
            mode: "translate",
            request: Some("commit my changes"),
            context: None,
            history: Some(&history),
            staged_diff: None,
        };
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["request"], "commit my changes");
        assert_eq!(
            value["history"][0],
            serde_json::json!({"question":"Message?","answer":"Fix docs"})
        );
        assert!(value.get("staged_diff").is_none());
    }

    #[test]
    fn response_schema_rejects_missing_mixed_and_unknown_payloads() {
        for fixture in [
            r#"{"kind":"commit_message"}"#,
            r#"{"kind":"commit_message","commit_message":{"subject":"Fix docs"},"plan":{}}"#,
            r#"{"kind":"unknown"}"#,
            r#"{"kind":"status","available":true}"#,
        ] {
            assert!(
                serde_json::from_str::<Response>(fixture).is_err(),
                "{fixture}"
            );
        }
        let response: Response = serde_json::from_str(
            r#"{"kind":"commit_message","commit_message":{"subject":"Fix docs"}}"#,
        )
        .unwrap();
        match response {
            Response::CommitMessage { commit_message } => {
                assert_eq!(commit_message.validate().unwrap().subject, "Fix docs")
            }
            _ => panic!("wrong response"),
        }
    }
}
