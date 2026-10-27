# On-device model evaluations

These evaluations invoke the real githelper binary and Apple's on-device model.
Run them on macOS 26+ with Apple Intelligence enabled, a supported Mac, and its
model downloaded. They never execute generated Git commands. Each scenario uses
a temporary Git repository with fixed setup commands, isolated Git configuration,
and disabled hooks. The runner preserves the user's HOME for Apple inference.

Python 3.10 or newer is required. Check `python3 --version`; if the Mac's
system Python is older, install a current Python (for example, `brew install
python`) and use that interpreter.

From the repository root:

```sh
cargo build --locked
python3 evals/run.py --binary target/debug/githelper
```

The runner automatically checks `--doctor --json` before running scenarios. An
unavailable model, timeout, malformed output, unexpected exit, failed semantic
assertion, or repository mutation fails the run. Exit 0 means every selected
scenario passed; exit 1 means failure (130 for interruption).

```sh
python3 evals/run.py --list
python3 evals/run.py --case diff-staged --timeout 180
python3 evals/run.py --artifacts eval-results/manual-run --keep-failures
```

Use the exact case names printed by `--list`; repeat `--case` to select several.
`--timeout` sets the per-process timeout in seconds (default 120), including Git
fixture setup and model preflight. `--artifacts` must name a new directory;
default reports go under `eval-results/` in a uniquely named run directory.
`--list` does not invoke the model. Timed-out processes and their descendants
are terminated.

The runner prints PASS/FAIL per case and the artifact directory. `summary.json`
contains overall results, platform/Python information, binary hash and size,
and preflight status. Each case directory includes
`result.json`, captured stdout/stderr, fixture setup logs, and before/after
repository snapshots covering file hashes, modes, symlink targets, and Git
internals. Inspect failure reasons and raw model output together. Reports and
retained fixtures can contain the generated fixture patches and model output.

Temporary repositories are removed after each case, including failures and
setup errors. `--keep-failures` preserves failed fixtures only; their locations
appear as `fixture_path` in result files, with `fixture_preserved: true`.
The `workspace_path` field names the entire retained temporary workspace; remove
that directory when finished.
The runner scopes `TMPDIR`, `TMP`, and `TEMP` to the workspace so extracted
Swift helper files are also removed with it. Artifacts remain until explicitly
deleted. Ctrl-C and SIGTERM trigger cleanup; SIGKILL, power loss, or machine
shutdown cannot reliably run cleanup handlers.

Assertions check structured outcomes, command scope, schema, and selected
properties of drafted subjects. Recovery scenarios require reflog inspection
for a lost commit, an undone reset, or a deleted branch, and require a named
SHA to become a `recovered/<name>` branch or a clarification. Those plans must
not emit `git reset`. Passing them does not establish general model
quality, consistent behavior across repeated runs, resistance to all prompt
injection, or the quality of a subject's wording. Review commit subjects for
accuracy and clarity, and examine refusals and ambiguous requests on the Mac.
Actual results depend on the OS/model version and local model availability.

The automated inference suite checks ambiguity through one-shot JSON results;
it does not exercise terminal clarification followups. Rust dialogue unit tests
cover interaction control flow without inference. To review real followups on
a Mac, run this manually in a terminal, supply a commit message when prompted,
and inspect the final proposed command:

```sh
target/debug/githelper --interactive "commit my changes"
```

## Harness tests without inference

```sh
python3 -m unittest discover -s evals -p 'test_*.py'
```

These standard-library tests use real temporary Git fixtures and mock only the
githelper process boundary. They test cleanup, error reporting, mutation checks,
Git configuration and hook isolation, interruption cleanup, and POSIX timeout
cleanup. Run these tests locally; the evaluation harness and its preflight
checks are excluded from CI.
Their passing results are not model evaluation results. Normal
`cargo test --locked` also does not run model evaluations.
