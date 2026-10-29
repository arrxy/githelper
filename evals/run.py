#!/usr/bin/env python3
"""Real, on-device githelper evaluations. Never executes a generated plan.

``--offline`` runs only scenarios that fail before inference and skips the
Apple model preflight. Those scenarios are not model-quality results.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
from pathlib import Path
import shutil
import signal
import stat
import subprocess
import tempfile
import time
import uuid


def isolated_environment(home: Path) -> dict[str, str]:
    home.mkdir(parents=True, exist_ok=True)
    config = home / "config"
    config.mkdir(exist_ok=True)
    temporary = home / "tmp"
    temporary.mkdir(exist_ok=True)
    env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    env.update(XDG_CONFIG_HOME=str(config), TMPDIR=str(temporary), TMP=str(temporary), TEMP=str(temporary), GIT_CONFIG_NOSYSTEM="1",
               GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_SYSTEM=os.devnull, GIT_TERMINAL_PROMPT="0", GIT_PAGER="cat",
               GIT_EDITOR="true", GIT_SEQUENCE_EDITOR="true", LC_ALL="C")
    return env


def snapshot_tree(root: Path) -> dict[str, dict]:
    """Capture contents, permissions and symlink targets, including Git internals."""
    result = {}
    for directory, dirs, files in os.walk(root, followlinks=False):
        for name in sorted(dirs + files):
            path = Path(directory) / name
            info = path.lstat()
            entry = {"mode": stat.S_IMODE(info.st_mode), "type": stat.S_IFMT(info.st_mode)}
            if path.is_symlink():
                entry["target"] = os.readlink(path)
            elif path.is_file():
                digest = hashlib.sha256()
                with path.open("rb") as stream:
                    for chunk in iter(lambda: stream.read(65536), b""):
                        digest.update(chunk)
                entry["sha256"] = digest.hexdigest()
            result[path.relative_to(root).as_posix()] = entry
    return result


def run_process(command: list[str], cwd: Path, env: dict, timeout: float,
                stdout_path: Path, stderr_path: Path) -> dict:
    """Capture to files and always kill/reap the process group on exit or interruption."""
    started = time.monotonic()
    timed_out = False
    stdout_path.parent.mkdir(parents=True, exist_ok=True)
    stderr_path.parent.mkdir(parents=True, exist_ok=True)
    with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
        child = subprocess.Popen(command, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                 stdout=stdout, stderr=stderr, start_new_session=True)
        try:
            try:
                child.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                timed_out = True
        finally:
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            finally:
                child.wait()
    return {"returncode": child.returncode, "timed_out": timed_out,
            "duration": round(time.monotonic() - started, 3)}


def read_output(path: Path, limit: int = 2_000_000) -> str:
    with path.open("rb") as stream:
        data = stream.read(limit + 1)
    if len(data) > limit:
        raise ValueError(f"output exceeds {limit} bytes: {path}")
    return data.decode("utf-8")


def run_case(case, binary: Path, artifact_dir: Path, timeout: float,
             keep_failures: bool) -> dict:
    artifact_dir.mkdir(parents=True, exist_ok=True)
    workspace = Path(tempfile.mkdtemp(prefix="githelper-eval-"))
    repo = workspace / "repo"
    result = {"name": case.name, "description": case.description, "passed": False,
              "reasons": [], "args": list(case.args), "fixture_path": str(repo),
              "workspace_path": str(workspace), "fixture_preserved": False}
    started = time.monotonic()
    try:
        repo.mkdir()
        env = isolated_environment(workspace / "config-home")
        # Eval/debug only: on plan validation failure the binary writes the raw
        # model proposal JSON here. Success leaves the file absent. Never shown
        # in normal CLI output.
        env["GITHELPER_DUMP_PROPOSAL"] = str(artifact_dir / "raw-model-proposal.json")
        setup_count = 0

        def git(arguments: list[str]) -> str:
            nonlocal setup_count
            setup_count += 1
            output = artifact_dir / f"setup-{setup_count}.stdout"
            errors = artifact_dir / f"setup-{setup_count}.stderr"
            process = run_process(["git", *arguments], repo, env, timeout, output, errors)
            if process["timed_out"] or process["returncode"] != 0:
                raise RuntimeError(f"fixture Git failed: {arguments!r}: {read_output(errors)}")
            return read_output(output)

        git(["-c", "init.templateDir=", "init", "--initial-branch=main"])
        hooks = workspace / "empty-hooks"
        hooks.mkdir()
        for key, value in [("user.name", "Githelper Eval"), ("user.email", "eval@example.invalid"),
                           ("core.hooksPath", str(hooks)), ("commit.gpgSign", "false"),
                           ("tag.gpgSign", "false"), ("core.autocrlf", "false"),
                           # Keep Git from starting background maintenance that writes
                           # .git/objects/maintenance.lock while the repo is snapshotted.
                           ("maintenance.auto", "false"), ("gc.auto", "0"),
                           ("gc.autoDetach", "false")]:
            git(["config", "--local", key, value])
        case.setup(repo, git)
        before = snapshot_tree(repo)
        (artifact_dir / "before.json").write_text(json.dumps(before, indent=2) + "\n")
        process = run_process([str(binary), "--repo", str(repo), "--json", *case.args],
                              repo, env, timeout, artifact_dir / "stdout.txt", artifact_dir / "stderr.txt")
        result.update(process)
        after = snapshot_tree(repo)
        (artifact_dir / "after.json").write_text(json.dumps(after, indent=2) + "\n")
        if before != after:
            changed = sorted(key for key in before.keys() | after.keys() if before.get(key) != after.get(key))
            result["reasons"].append("repository mutated: " + ", ".join(changed))
        if process["timed_out"]:
            result["reasons"].append(f"timed out after {timeout:g}s")
        stdout = read_output(artifact_dir / "stdout.txt")
        stderr = read_output(artifact_dir / "stderr.txt")
        if process["timed_out"]:
            pass
        elif (
            getattr(case, "accept_validation_error", False)
            and process["returncode"] != 0
            and "invalid model proposal" in stderr
        ):
            # Safe refusal: model proposed something validation rejected.
            if stdout.strip():
                result["reasons"].append("unexpected stdout for validation refusal")
        elif process["returncode"] != case.expected_exit:
            result["reasons"].append(f"exit {process['returncode']}, expected {case.expected_exit}")
        elif case.expected_exit == 0 and process["returncode"] == 0:
            result["reasons"].extend(case.check(json.loads(stdout)))
        elif case.expected_exit != 0:
            if case.expected_stderr not in stderr:
                result["reasons"].append(f"stderr missing {case.expected_stderr!r}")
            if stdout.strip():
                result["reasons"].append("unexpected stdout for failed request")
        result["passed"] = not result["reasons"]
    except KeyboardInterrupt:
        result["reasons"].append("interrupted")
        raise
    except Exception as error:
        result["reasons"].append(f"{type(error).__name__}: {error}")
    finally:
        result["duration"] = round(time.monotonic() - started, 3)
        if keep_failures and not result["passed"]:
            result["fixture_preserved"] = True
        else:
            shutil.rmtree(workspace)
        (artifact_dir / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


def main(argv: list[str] | None = None) -> int:
    from scenarios import CASES

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/githelper"))
    parser.add_argument("--case", action="append", default=[], help="exact case name; repeatable")
    parser.add_argument("--list", action="store_true", help="list cases without running the model")
    parser.add_argument(
        "--offline",
        action="store_true",
        help="run only scenarios that do not call the model, and skip Apple preflight",
    )
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--artifacts", type=Path)
    parser.add_argument("--keep-failures", action="store_true")
    args = parser.parse_args(argv)
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("--timeout must be positive")
    names = {case.name for case in CASES}
    if set(args.case) - names:
        parser.error("unknown cases: " + ", ".join(sorted(set(args.case) - names)))
    cases = [case for case in CASES if not args.case or case.name in args.case]
    if args.offline:
        selected_model = [case.name for case in cases if case.requires_model]
        if args.case and selected_model:
            parser.error(
                "model-dependent scenarios stay on a supported Mac; "
                "--offline cannot run: " + ", ".join(selected_model)
            )
        cases = [case for case in cases if not case.requires_model]
        if not cases:
            parser.error("no offline scenarios selected")
    if args.list:
        for case in cases:
            suffix = "" if case.requires_model else " [offline]"
            print(f"{case.name}: {case.description}{suffix}")
        return 0
    binary = args.binary.resolve()
    artifacts = (args.artifacts or Path("eval-results") / (time.strftime("%Y%m%dT%H%M%SZ", time.gmtime()) + "-" + uuid.uuid4().hex[:8])).resolve()
    artifacts.mkdir(parents=True, exist_ok=False)
    summary = {"binary": str(binary), "artifacts": str(artifacts), "cases": [], "passed": False,
               "offline": args.offline, "platform": platform.platform(),
               "python_version": platform.python_version(),
               "started_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}
    exit_code = 1
    def interrupted(_signal, _frame):
        raise KeyboardInterrupt
    previous_sigterm = signal.signal(signal.SIGTERM, interrupted)
    try:
        binary_digest = hashlib.sha256()
        binary_size = 0
        with binary.open("rb") as stream:
            for chunk in iter(lambda: stream.read(65536), b""):
                binary_digest.update(chunk)
                binary_size += len(chunk)
        summary["binary_sha256"] = binary_digest.hexdigest()
        summary["binary_size_bytes"] = binary_size
        if args.offline:
            summary["preflight"] = {"skipped": True}
        else:
            with tempfile.TemporaryDirectory(prefix="githelper-preflight-") as temporary:
                root = Path(temporary)
                preflight = run_process([str(binary), "--doctor", "--json"], root,
                                        isolated_environment(root / "config-home"), args.timeout,
                                        artifacts / "doctor.stdout.txt", artifacts / "doctor.stderr.txt")
                summary["preflight"] = preflight
                try:
                    status = json.loads(read_output(artifacts / "doctor.stdout.txt"))
                except (ValueError, UnicodeError):
                    status = {}
                if not isinstance(status, dict):
                    status = {}
                summary["doctor_status"] = {key: status.get(key) for key in ("available", "detail")}
                if preflight["timed_out"] or preflight["returncode"] != 0:
                    raise RuntimeError("Apple model preflight failed; see doctor stdout/stderr artifacts")
                if status.get("available") is not True:
                    raise RuntimeError("Apple model preflight did not report available=true")
        for index, case in enumerate(cases, 1):
            result = run_case(case, binary, artifacts / f"{index:02d}-{case.name}", args.timeout, args.keep_failures)
            summary["cases"].append(result)
            print(f"{'PASS' if result['passed'] else 'FAIL'} {case.name}" + (": " + "; ".join(result["reasons"]) if result["reasons"] else ""), flush=True)
        summary["passed"] = all(result["passed"] for result in summary["cases"])
        exit_code = 0 if summary["passed"] else 1
    except KeyboardInterrupt:
        summary["error"] = "interrupted"
        exit_code = 130
    except Exception as error:
        summary["error"] = f"{type(error).__name__}: {error}"
        print(f"FAIL {summary['error']}", flush=True)
    finally:
        signal.signal(signal.SIGTERM, previous_sigterm)
        (artifacts / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        print(f"Artifacts: {artifacts}", flush=True)
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
