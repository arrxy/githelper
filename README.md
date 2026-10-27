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

If compilation cannot find Foundation Models, check `xcodebuild -version` and
`xcrun --show-sdk-version`. Select your Xcode 26+ installation as the active
developer directory before rebuilding.

The build script compiles the Swift bridge and embeds it inside the Rust
executable. At inference time the helper is stored in a private directory under
the user cache, named by a hash of that exact binary. It is written atomically
with owner-only permissions and reused only when the cached file still matches.
The process exchanges JSON over stdin/stdout. You can move the built executable
without separately installing the bridge. Build natively for each Mac
architecture; cross-compilation is not supported.

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
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
```

The Rust core and CLI checks run on Linux without the Apple framework.
Translation is available only on a supported Mac. These features are implemented;
actual model inference and output quality remain unverified in this environment.
macOS CI compiles the real Swift bridge; an Apple Intelligence-capable Mac is required to test actual
inference, availability, refusal behavior, and translation quality. After
`githelper --doctor` succeeds, try the requests above and check that ambiguous
requests such as `githelper "commit my changes"` ask for clarification.

Run the [on-device evaluation harness](evals/README.md) on a supported Mac to
check real model responses against reproducible Git scenarios. Its separate
Python plumbing tests run without inference.

MIT licensed.
