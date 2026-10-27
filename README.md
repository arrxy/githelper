# githelper

A Rust CLI that turns plain language into reviewable Git commands using Apple's
on-device language model. A small Swift bridge calls the Foundation Models
framework. No API key or cloud inference service is used.

```text
githelper "show my staged changes"

Show the changes currently staged for commit.

Shows changes currently staged for commit.
  git diff --staged --no-ext-diff --no-textconv --
```

The example illustrates the output format; model wording varies. **githelper
only prints commands. It never executes a generated plan.**

## Requirements

- An Apple Intelligence-capable Mac running macOS 26 or newer.
- Apple Intelligence enabled, with the on-device model fully downloaded and
  available for your device language and region.
- Xcode 26 or newer selected as the active developer directory, including its
  Swift compiler and macOS 26 SDK.
- A current stable Rust toolchain and Git.

## Install and use

From this repository on your Mac:

```sh
cargo install --path . --locked
githelper --doctor
cd ~/code/your-repository
githelper "show my changes"
githelper "create a branch called fix/login"
githelper "stage README.md and commit with message update docs"
githelper --repo ~/code/your-repository "show the last ten commits"
githelper --json "list my branches"
githelper --interactive "commit my changes"
githelper --draft-commit
githelper --draft-commit --json
```

`githelper --doctor` prints one pass or fail line per prerequisite. See
[Setup and troubleshooting](#setup-and-troubleshooting) when a check fails or
the build cannot find the Swift toolchain.

The build script compiles the Swift bridge and embeds it inside the Rust
executable. At inference time the helper is stored in a private directory under
the user cache, named by a hash of that exact binary. It is written atomically
with owner-only permissions and reused only when the cached file still matches.
The process exchanges JSON over stdin/stdout. You can move the built executable
without separately installing the bridge. Build natively for each Mac
architecture; cross-compilation is not supported.

## Setup and troubleshooting

`githelper --doctor` checks macOS, Xcode and Swift, Apple Intelligence, Git,
and whether `--repo` (default `.`) is inside a Git work tree. Each check prints
`[pass]` or `[fail]` plus a fix hint. The command exits 0 only when every
required check passes. The repository check is optional, so doctor can succeed
before you `cd` into a project; pass `--repo` when you want that path checked.
`githelper --doctor --json` prints the same result. `available` and `detail`
describe the on-device model (existing callers can keep reading those fields).
`ok` is true when every required check passed.

```text
$ githelper --doctor
[pass] macOS 26 or newer
       macOS 26.0
[pass] Xcode 26 and Swift
       Xcode 26.0, macOS SDK 26.0
[pass] Apple Intelligence
       Apple's on-device model is ready. Inference stays on this Mac.
[pass] Git
       git version 2.50.1
[pass] Git repository (optional)
       /Users/me/code/your-repository
All required checks passed.
```

On a machine that is not ready, the same command names the gap and exits
non-zero. From this repository on Linux, where Git is installed and the
working directory is a repository, doctor looks like this:

```text
$ githelper --doctor
[fail] macOS 26 or newer
       This system is linux, not macOS.
       Fix: Upgrade to macOS 26 or newer on an Apple Intelligence-capable Mac.
[fail] Xcode 26 and Swift
       The Swift toolchain is missing (xcrun: No such file or directory (os error 2)).
       Fix: Install Xcode 26 or newer, then select it with `sudo xcode-select -s /Applications/Xcode.app/Contents/Developer` and rebuild.
[fail] Apple Intelligence
       Requires macOS 26 or newer on an Apple Intelligence-capable Mac with Apple Intelligence enabled and its on-device model downloaded.
       Fix: Enable Apple Intelligence in System Settings and wait for the on-device model to finish downloading.
[pass] Git
       git version 2.43.0
[pass] Git repository (optional)
       /workspace
3 required checks failed.
```

Outside a work tree the repository line fails instead, with a hint to `cd`
into a repository or pass `--repo`. That line is optional, so a ready Mac
still exits 0. Stderr on failure is a one-line summary, for example
`githelper: setup checks failed: macOS 26 or newer; Xcode 26 and Swift; Apple Intelligence`.

A failed build prints one error and exits. It does not dump a panic backtrace.
The message names what is missing and how to fix it:

- Upgrade to macOS 26 or newer on an Apple Intelligence-capable Mac.
- Install Xcode 26 or newer, including the macOS 26 SDK.
- Select that Xcode: `sudo xcode-select -s /Applications/Xcode.app/Contents/Developer`
- Accept the license if prompted: `sudo xcodebuild -license`
- Enable Apple Intelligence and wait until the on-device model has downloaded.
- Install Git. Run githelper inside a repository, or pass `--repo`.

Confirm the selected tools with `xcodebuild -version` and
`xcrun --sdk macosx --show-sdk-version` before rebuilding. If a later run reports that the
model is unavailable or the Apple bridge failed, run `githelper --doctor`.
There is no cloud fallback.

## Supported requests

| Intent | Result |
| --- | --- |
| Status, unstaged/staged diff | Read-only Git inspection commands |
| Recent history | Last ten commits |
| List, create, switch branches | Local branch operations |
| Stage or unstage named files | Literal paths relative to the repository root |
| Stage everything | Includes new, modified, and deleted files |
| Commit with an explicit message | Commits the current index |
| Stash tracked changes | Leaves untracked files alone |
| Push | Uses the configured upstream and Git defaults |
| Pull | Fast-forward only |

Multiple operations can form a plan of up to six steps. Missing commit messages,
ambiguous requests, unsupported flags, and unsupported operations return a
clarification. This initial version does not support reset, clean, discard,
delete, merge, rebase, force push, or custom remotes/refspecs. Add those as explicit
typed operations with validation if you extend the tool.

## Clarification and commit subjects

Default translation, including `--json`, is one-shot. Missing information returns
`needsClarification`; unsupported intent returns `unsupported`. Both produce no
commands and remain successful one-shot results.

Use `--interactive` to answer clarification questions in a terminal. It preserves
the original request and previous answers, accepts at most three answers (four
model calls), and limits each answer to 1,000 bytes and the question/answer history
to 6,000 bytes. Prompts use stderr; only a completed plan uses stdout. Blank input
or EOF cancels successfully (exit 0), without a plan. Invalid input, exhausted
rounds, and model failures exit 1 without a partial plan. Unsupported requests
stop immediately. Interactive mode conflicts with JSON, doctor, and drafting.

`--draft-commit` proposes a single-line subject from the staged patch only. It
neither stages nor commits. Plain output is the subject; JSON is
`{"kind":"commit_message","subject":"..."}`. The staged patch is limited to
6,000 bytes without truncation; subjects must be nonempty, contain no control
characters, and fit within 200 bytes. Empty staging, unresolved conflicts,
invalid encoding, and oversized patches return errors. Binary changes provide
metadata only. Review the proposed subject before using it as an explicit
message in a normal translation request.

## Design

```text
Natural-language request + bounded repository metadata
                       ↓
Rust CLI → embedded Swift helper → Apple Foundation Models
                       ↓
             Structured operation plan
                       ↓
        Rust validation + fixed Git templates
                       ↓
            Quoted commands / JSON output
```

The model chooses typed operations rather than arbitrary shell strings. Rust
rejects malformed plans, unknown actions, unexpected arguments, invalid branch
names, and paths that escape the repository lexically. File operations use
`--literal-pathspecs` and `--` so filenames cannot become Git options or pathspec
expressions. Displayed commands use POSIX shell quoting, suitable for zsh/bash.
JSON output includes the executable, argument array, and deterministic explanation
for each command, plus an `outcome`: `ready`, `needsClarification`, or `unsupported`.
A destructive step also includes `destructive: true` and the warning text shown
ahead of that command. Explanations and warnings come from validated Rust
operations rather than model-generated prose.

For normal translation, local context contains the repository root, current
branch, up to 32 local branch names, and up to 4,000 characters of tracked-file status. It contains no
file contents, diffs, remote URLs, or untracked filenames. Context is collected
from the repository root even when invoked from a subdirectory. Instructions
tell the model to treat this metadata as untrusted data. Commit drafting instead
sends the bounded staged patch, including staged text content and binary-change
metadata, without ordinary translation context.

Inference stays on the Mac; there is no network inference fallback. The OS may
need to download its model beforehand. A model can still misinterpret intent,
so review every proposed command, including its scope, before running it. The
tool's validation constrains command syntax; it cannot prove intent matches.
Repository state can change between translation and execution.

## Development

```sh
cargo test --locked
cargo test --locked --features fake-model
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo clippy --all-targets --locked --features fake-model -- -D warnings
```

### Testing the plan path without Apple Intelligence

Linux cannot call the on-device model. The `fake-model` Cargo feature compiles
a test stand-in so the path from request to `Plan::validate` to printed or
`--json` output runs anywhere. It is off by default. A normal build ignores
`GITHELPER_FAKE_MODEL`, and a release binary does not contain the stand-in.
Do not ship a binary built with `--features fake-model`.

With the feature enabled, set `GITHELPER_FAKE_MODEL` to one model-response JSON
object, or to a path of a UTF-8 file containing one. Inline JSON is the value
when it starts with `{`; anything else is a file path. The object uses the
Swift bridge schema (`kind` of `plan`, `commit_message`, `status`, or `error`).
Unset, that build behaves like a build without the feature. The variable
applies to every model call in that process, including `--doctor` and
`--draft-commit`. The stand-in only parses the fixture. Plans still go through
`Plan::validate`, commit subjects still go through `CommitMessage::validate`,
and githelper still only prints commands. Every current action is benign, so
validated JSON omits `destructive` and `warning`. A fixture cannot invent those
fields or an action such as `reset`; validation still rejects them.

```sh
GITHELPER_FAKE_MODEL='{"kind":"plan","plan":{"outcome":"ready","summary":"Show status","clarification":"","steps":[{"action":"status","value":"","paths":[]}]}}' \
  cargo run --features fake-model -- --json "show status"
```

The Rust core and CLI checks run on Linux without the Apple framework.
Doctor pass/fail decisions are pure functions of probed facts, so Linux CI
covers the check wording without Xcode or Apple Intelligence. Real translation
is available only on a supported Mac. These features are implemented; actual model
inference and output quality remain unverified in this environment. macOS CI
compiles the real Swift bridge; an Apple Intelligence-capable Mac is required
to test actual inference, availability, refusal behavior, and translation
quality. After `githelper --doctor` succeeds, try the requests above and check
that ambiguous requests such as `githelper "commit my changes"` ask for
clarification.

Run the [on-device evaluation harness](evals/README.md) on a supported Mac to
check real model responses against reproducible Git scenarios. CI runs the
harness plumbing tests and `run.py --offline` for scenarios that fail before
inference. Model-dependent scenarios stay on a supported Mac.

MIT licensed.
