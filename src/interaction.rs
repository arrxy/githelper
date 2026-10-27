use crate::{
    model::ClarificationTurn,
    plan::{Outcome, Plan, Proposal},
};
use std::io::{BufRead, Write};

type Error = Box<dyn std::error::Error>;

/// Own the dialogue in Rust; no incomplete plan is returned to the renderer.
pub fn clarify(
    mut translate: impl FnMut(&[ClarificationTurn]) -> Result<Proposal, Error>,
    input: &mut impl BufRead,
    prompts: &mut impl Write,
) -> Result<Option<Plan>, Error> {
    let mut history = Vec::new();
    let mut history_bytes = 0;
    loop {
        let plan = translate(&history)?.validate()?;
        match plan.outcome {
            Outcome::Ready => return Ok(Some(plan)),
            Outcome::Unsupported => {
                writeln!(
                    prompts,
                    "Unsupported request: {}",
                    plan.clarification.as_deref().unwrap_or_default()
                )?;
                return Ok(None);
            }
            Outcome::NeedsClarification => {
                let question = plan
                    .clarification
                    .as_deref()
                    .ok_or("missing clarification question")?;
                if history.len() == 3 {
                    return Err(format!("clarification limit reached: {question}. Rerun with a more specific request.").into());
                }
                writeln!(prompts, "{question}")?;
                write!(prompts, "Answer (blank to cancel): ")?;
                prompts.flush()?;
                let Some(answer) = read_answer(input)? else {
                    return Ok(None);
                };
                history_bytes += question.len() + answer.len();
                if history_bytes > 6_000 {
                    return Err("clarification history exceeds 6,000 bytes".into());
                }
                history.push(ClarificationTurn {
                    question: question.to_owned(),
                    answer,
                });
            }
        }
    }
}

fn read_answer(input: &mut impl BufRead) -> Result<Option<String>, Error> {
    let mut bytes = Vec::new();
    loop {
        let chunk = input.fill_buf()?;
        if chunk.is_empty() {
            break;
        }
        let count = chunk
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(chunk.len(), |index| index + 1);
        if bytes.len() + count > 1_002 {
            return Err(
                "answer must be at most 1,000 bytes and contain no control characters".into(),
            );
        }
        let complete = chunk[count - 1] == b'\n';
        bytes.extend_from_slice(&chunk[..count]);
        input.consume(count);
        if complete {
            break;
        }
    }
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    let answer = String::from_utf8(bytes)?;
    if answer.len() > 1_000 || answer.chars().any(char::is_control) {
        return Err("answer must be at most 1,000 bytes and contain no control characters".into());
    }
    if answer.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(answer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn proposal(outcome: Outcome) -> Proposal {
        serde_json::from_value(serde_json::json!({
            "outcome": outcome,
            "summary": "Inspect changes",
            "clarification": if matches!(outcome, Outcome::Ready) { "" } else { "Which changes?" },
            "steps": if matches!(outcome, Outcome::Ready) { serde_json::json!([{"action":"diff","value":"","paths":[]}]) } else { serde_json::json!([]) }
        })).unwrap()
    }

    #[test]
    fn preserves_answers_and_returns_only_ready_plan() {
        let mut calls = 0;
        let result = clarify(
            |history| {
                calls += 1;
                if calls == 1 {
                    assert!(history.is_empty());
                    Ok(proposal(Outcome::NeedsClarification))
                } else {
                    assert_eq!(history[0].answer, "unstaged");
                    assert_eq!(history[0].question, "Which changes?");
                    Ok(proposal(Outcome::Ready))
                }
            },
            &mut Cursor::new(b"unstaged\n"),
            &mut Vec::new(),
        )
        .unwrap();
        assert!(result.is_some());
        assert_eq!(calls, 2);
    }

    #[test]
    fn bounded_rounds_and_no_partial_plan() {
        let mut calls = 0;
        let result = clarify(
            |history| {
                calls += 1;
                assert_eq!(history.len(), calls - 1);
                Ok(proposal(Outcome::NeedsClarification))
            },
            &mut Cursor::new(b"one\ntwo\nthree\nfour\n"),
            &mut Vec::new(),
        );
        assert!(result.unwrap_err().to_string().contains("limit reached"));
        assert_eq!(calls, 4);
    }

    #[test]
    fn cancellation_and_unsupported_do_not_continue() {
        for input in [b"".as_slice(), b"\n", b"  \n"] {
            let mut calls = 0;
            assert!(
                clarify(
                    |_| {
                        calls += 1;
                        Ok(proposal(Outcome::NeedsClarification))
                    },
                    &mut Cursor::new(input),
                    &mut Vec::new()
                )
                .unwrap()
                .is_none()
            );
            assert_eq!(calls, 1);
        }
        let mut input = Cursor::new(b"unused\n");
        assert!(
            clarify(
                |_| Ok(proposal(Outcome::Unsupported)),
                &mut input,
                &mut Vec::new()
            )
            .unwrap()
            .is_none()
        );
        assert_eq!(input.position(), 0);
    }

    #[test]
    fn input_bounds_and_controls() {
        for input in [vec![b'a'; 100_000], b"bad\tanswer\n".to_vec(), vec![0xff]] {
            assert!(read_answer(&mut Cursor::new(input)).is_err());
        }
        assert_eq!(
            read_answer(&mut Cursor::new(b"message\r\n"))
                .unwrap()
                .unwrap(),
            "message"
        );
        assert_eq!(
            read_answer(&mut Cursor::new(vec![b'a'; 1000]))
                .unwrap()
                .unwrap()
                .len(),
            1000
        );
    }

    #[test]
    fn invalid_input_never_calls_model_again_and_late_failure_has_no_plan() {
        let mut calls = 0;
        let result = clarify(
            |_| {
                calls += 1;
                Ok(proposal(Outcome::NeedsClarification))
            },
            &mut Cursor::new(b"invalid\tanswer\n"),
            &mut Vec::new(),
        );
        assert!(result.is_err());
        assert_eq!(calls, 1);
        let mut calls = 0;
        let result = clarify(
            |_| {
                calls += 1;
                if calls == 1 {
                    Ok(proposal(Outcome::NeedsClarification))
                } else {
                    Err("late inference failure".into())
                }
            },
            &mut Cursor::new(b"unstaged\n"),
            &mut Vec::new(),
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("late inference failure")
        );
        assert_eq!(calls, 2);
    }

    #[test]
    fn invalid_proposal_and_model_error_stop() {
        assert!(
            clarify(
                |_| Err("inference failed".into()),
                &mut Cursor::new(b""),
                &mut Vec::new()
            )
            .is_err()
        );
        assert!(
            clarify(
                |_| {
                    let mut p = proposal(Outcome::Ready);
                    p.clarification = "question".into();
                    Ok(p)
                },
                &mut Cursor::new(b""),
                &mut Vec::new()
            )
            .is_err()
        );
    }
}
