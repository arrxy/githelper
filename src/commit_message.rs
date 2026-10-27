use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommitMessage {
    pub subject: String,
}

impl CommitMessage {
    pub fn validate(mut self) -> Result<Self, Box<dyn std::error::Error>> {
        if self.subject.len() > 200
            || self
                .subject
                .chars()
                .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
        {
            return Err("commit subject must be one line, contain no control characters, and be at most 200 bytes".into());
        }
        self.subject = self.subject.trim().to_owned();
        if self.subject.is_empty() {
            return Err("commit subject must not be empty".into());
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_subjects() {
        for subject in [
            "",
            "   ",
            "fix\n",
            "\tfix",
            "fix\rbug",
            "fix\u{1b}bug",
            "fix\u{2028}bug",
            "fix\u{2029}bug",
            "fix\u{2028}",
            "\u{2029}fix",
            &"é".repeat(101),
        ] {
            assert!(
                CommitMessage {
                    subject: subject.into()
                }
                .validate()
                .is_err()
            );
        }
        assert_eq!(
            CommitMessage {
                subject: "  Fix café  ".into()
            }
            .validate()
            .unwrap()
            .subject,
            "Fix café"
        );
        assert!(
            CommitMessage {
                subject: "x".repeat(200)
            }
            .validate()
            .is_ok()
        );
        assert!(
            serde_json::from_str::<CommitMessage>(r#"{"subject":"Fix","command":"git commit"}"#)
                .is_err()
        );
    }
}
