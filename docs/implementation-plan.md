# Githelper feature implementation plan

Status: implemented. Rust validation and fixture tests cover the new modes.
Actual Apple model inference and translation quality remain unverified here;
macOS compilation and on-device inference are separate validation gates.

## Delivery and review

GPT-6 Astra orchestrates the work, defines shared interfaces, and validates the
combined implementation. GPT-6.1 Sol subagents implement scoped tasks and report
their changes and test evidence. Astra reviews correctness, integration, and
acceptance criteria before completion is reported.

Implement the features in order: explanations, interactive clarification, then
commit message drafting. Define shared Rust/Swift schemas before parallel edits
to prevent conflicting changes. Generated commands remain review-only.

## 1. Command explanations

Generate explanations in Rust from validated operations and display each above
its command. Include an `explanation` field alongside `program` and `args` in
JSON output. No additional model-generated explanation fields are needed.

Example:

```text
Stages all changes, including untracked files.
  git add --all
```

Explain scope precisely: commits use staged changes; stash excludes untracked
files; creating a branch also switches to it; pull allows fast-forwards only.
Describe intended effects without claiming that a command has executed.

Primary files: `src/plan.rs`, `src/main.rs`, tests, and README.

Acceptance:

- Every supported action has an accurate explanation.
- Command arguments, shell quoting, and mutation classification are preserved.
- Multistep plans retain command/explanation pairing and order.
- Clarification results contain no commands.
- Text and JSON rendering can be tested without Apple inference.

## 2. Interactive clarification

Add opt-in `--interactive` mode:

```text
githelper --interactive "commit my changes"
What commit message should Git use?
> update docs
```

Default invocation remains one-shot. Interactive mode requires terminal input
and conflicts with JSON, doctor, and drafting modes. Prompts go to stderr;
only completed plans go to stdout. Blank input or EOF cancels without commands
(exit 0). Invalid input, exhausted rounds, and model failures exit 1 without a
partial plan. Unsupported and unresolved one-shot results remain successful
outputs. Default JSON output is one-shot and never prompts.

Add matching Rust/Swift proposal outcomes:

- `ready`: empty clarification and one to six validated steps.
- `needsClarification`: a question and zero steps.
- `unsupported`: an explanation and zero steps; terminate immediately.

Allow at most three answered questions and four inference calls. Limit each
answer to 1,000 bytes and question/answer history to 6,000 bytes; reject invalid
input locally. Replay
the original request and typed question/answer history with each helper call.
Retain the existing single-request helper lifecycle. Treat repository metadata
and conversation content as data and enforce all result invariants in Rust.

Primary files: `src/main.rs`, `src/model.rs`, `src/plan.rs`,
`apple/ModelBridge.swift`, and a small interaction module if useful.

Acceptance:

- Missing information can be supplied within one invocation.
- Original intent and previous answers survive subsequent model calls.
- Unsupported requests stop immediately.
- Blank input, EOF, exhausted rounds, and model errors emit no partial plan.
- JSON and unattended invocations never prompt.
- Fixture-based tests verify round limits, malformed results, cancellation,
  bounded input, and final command validation without Apple inference.

## 3. Commit message drafting

Add a dedicated mode:

```sh
githelper --draft-commit
githelper -C ~/code/app --draft-commit --json
```

Text output is a proposed single-line subject. JSON has a distinct shape:
`{"kind":"commit_message","subject":"..."}`. This mode neither stages nor
commits. Normal translation continues requiring an explicit commit message.

Read only the staged patch using fixed read-only Git arguments. Disable
external diff, text conversion, color, and rename detection. Resolve the root
so initial commits and invocation from subdirectories work. Do not add the
ordinary context's unstaged status or unrelated branch names to this prompt.

Reject an empty index, unresolved conflicts, invalid UTF-8, and patches over
an initial 6,000-byte budget. Enforce limits while reading subprocess output,
bound stderr, and safely drain or terminate and reap the child on overflow.
Never silently truncate. Include binary-change metadata only, with instructions
to avoid inventing binary contents or motivation.

Use a dedicated Swift `CommitMessage` schema and bridge mode. Treat patch text
and paths as untrusted data. Independently validate the subject in Rust:
nonempty, one line, no control characters, and at most 200 bytes. Guide toward
roughly 72 characters without requiring Conventional Commits.

Primary files: `src/main.rs`, `src/repository.rs`, `src/model.rs`,
`apple/ModelBridge.swift`, and a small commit-message validation module.

Acceptance:

- Only staged content reaches the model, even when the same file has unstaged edits.
- Index and working-tree contents remain unchanged.
- Tests cover initial commits, subdirectories, binary and mode changes,
  deletions, empty staging, conflicts, encoding, and size limits.
- Configured external diff/textconv commands never execute.
- Subject validation and text/JSON output are tested using fixtures.
- Unavailable models and refusals produce actionable errors, not fabricated drafts.

## Validation gates

Run Rust formatting, tests, and Clippy for each integrated change. Update CLI
help and README with the new modes, output shapes, and limits. Use an injectable
model interface for deterministic Linux tests rather than a production bypass.

macOS CI must compile the real embedded Swift helper. Actual inference requires
an Apple Intelligence-capable Mac. Verify clarification quality, grounded
commit subjects, refusals, adversarial patch text, and context limits there.
The proposed byte budget is conservative but does not guarantee token fit;
adjust it using Mac validation evidence. Report local tests, macOS compilation,
and actual inference verification separately.
