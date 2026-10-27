"""Linux-compatible plumbing tests; these do not evaluate Apple inference."""
import contextlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import run as harness
import scenarios


class HarnessTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.binary = self.root / "githelper-under-test"

    def case(self, setup=None, check=None):
        def fixture(repo, git):
            (repo / "tracked.txt").write_text("staged content\n")
            git(["add", "tracked.txt"])
            git(["commit", "-m", "Initial fixture"])
        return SimpleNamespace(name="plumbing", description="mocked inference boundary",
                               args=("show status",), setup=setup or fixture,
                               check=check or (lambda value: [] if value == {"ok": True} else ["wrong result"]),
                               expected_exit=0, expected_stderr="")

    def execute(self, case=None, output='{"ok":true}', code=0, keep=False, mutate=False):
        real_process = harness.run_process

        def boundary(command, cwd, env, timeout, stdout_path, stderr_path):
            if command[0] != str(self.binary):
                return real_process(command, cwd, env, timeout, stdout_path, stderr_path)
            # Setup really initialized and committed a temporary Git repository.
            self.assertTrue((cwd / ".git" / "HEAD").is_file())
            self.assertEqual(subprocess.check_output(["git", "rev-list", "--count", "HEAD"],
                                                     cwd=cwd, env=env, text=True).strip(), "1")
            if mutate:
                (cwd / "tracked.txt").write_text("unexpected mutation\n")
            stdout_path.write_text(output)
            stderr_path.write_text("model unavailable" if code else "")
            return {"returncode": code, "timed_out": False, "duration": 0.001}

        with patch.object(harness, "run_process", side_effect=boundary):
            result = harness.run_case(case or self.case(), self.binary, self.root / "artifacts", 5, keep)
        self.assertEqual(json.loads((self.root / "artifacts/result.json").read_text()), result)
        return result

    def assert_removed(self, result):
        self.assertFalse(Path(result["fixture_path"]).parent.exists())
        self.assertFalse(result["fixture_preserved"])

    def test_success_cleans_real_git_fixture(self):
        result = self.execute(keep=True)
        self.assertTrue(result["passed"], result["reasons"])
        self.assert_removed(result)
        self.assertTrue((self.root / "artifacts/before.json").exists())

    def test_setup_failure_cleans_fixture(self):
        def broken(repo, git):
            raise RuntimeError("setup failed deliberately")
        result = self.execute(self.case(setup=broken))
        self.assertFalse(result["passed"])
        self.assertIn("setup failed deliberately", " ".join(result["reasons"]))
        self.assert_removed(result)

    def test_model_failure_cleans_fixture(self):
        result = self.execute(code=1, output="")
        self.assertFalse(result["passed"])
        self.assertIn("exit 1", " ".join(result["reasons"]))
        self.assert_removed(result)

    def test_invalid_json_cleans_fixture(self):
        result = self.execute(output="not JSON")
        self.assertFalse(result["passed"])
        self.assertIn("JSONDecodeError", " ".join(result["reasons"]))
        self.assert_removed(result)

    def test_failed_semantic_assertion_cleans_fixture(self):
        result = self.execute(self.case(check=lambda value: ["wrong command scope"]))
        self.assertFalse(result["passed"])
        self.assertIn("wrong command scope", result["reasons"])
        self.assert_removed(result)

    def test_assertion_exception_cleans_fixture(self):
        def broken(value):
            raise AssertionError("malformed response")
        result = self.execute(self.case(check=broken))
        self.assertFalse(result["passed"])
        self.assertIn("AssertionError", " ".join(result["reasons"]))
        self.assert_removed(result)

    def test_keep_failures_preserves_fixture_and_test_removes_it(self):
        result = self.execute(output="invalid", keep=True)
        workspace = Path(result["fixture_path"]).parent
        self.addCleanup(shutil.rmtree, workspace, ignore_errors=True)
        self.assertTrue(result["fixture_preserved"])
        self.assertTrue((workspace / "repo/.git/HEAD").exists())

    def test_mutation_fails_even_when_response_passes(self):
        result = self.execute(mutate=True)
        self.assertFalse(result["passed"])
        self.assertIn("repository mutated: tracked.txt", result["reasons"])
        self.assert_removed(result)

    def test_snapshot_covers_content_modes_symlinks_and_git(self):
        repo = self.root / "snapshot"
        (repo / ".git").mkdir(parents=True)
        tracked = repo / "file"
        tracked.write_text("original")
        (repo / ".git/index").write_bytes(b"index")
        (repo / "link").symlink_to("file")
        first = harness.snapshot_tree(repo)
        tracked.write_text("modified")
        tracked.chmod(0o755)
        (repo / "link").unlink()
        (repo / "link").symlink_to("other")
        (repo / ".git/index").write_bytes(b"changed index")
        second = harness.snapshot_tree(repo)
        self.assertNotEqual(first["file"]["sha256"], second["file"]["sha256"])
        self.assertNotEqual(first["file"]["mode"], second["file"]["mode"])
        self.assertEqual(second["link"]["target"], "other")
        self.assertNotEqual(first[".git/index"], second[".git/index"])

    def test_environment_removes_inherited_git_overrides(self):
        hostile = {"GIT_DIR": "/wrong", "GIT_WORK_TREE": "/wrong", "GIT_INDEX_FILE": "/wrong",
                   "GIT_CONFIG_COUNT": "1", "GIT_CONFIG_KEY_0": "core.hooksPath",
                   "GIT_CONFIG_VALUE_0": "/wrong", "GIT_SSH_COMMAND": "unexpected"}
        with patch.dict(os.environ, hostile):
            env = harness.isolated_environment(self.root / "home")
        for key in hostile:
            self.assertNotIn(key, env)
        self.assertEqual(env.get("HOME"), os.environ.get("HOME"))
        self.assertEqual(env["XDG_CONFIG_HOME"], str(self.root / "home/config"))
        self.assertEqual(env["GIT_CONFIG_NOSYSTEM"], "1")
        self.assertEqual(env["GIT_CONFIG_GLOBAL"], os.devnull)

    def test_inherited_global_config_and_hooks_do_not_execute(self):
        marker = self.root / "hook-ran"
        hooks = self.root / "hostile-hooks"
        hooks.mkdir()
        hook = hooks / "pre-commit"
        hook.write_text("#!/bin/sh\ntouch '" + str(marker) + "'\nexit 1\n")
        hook.chmod(0o755)
        config = self.root / "hostile.gitconfig"
        config.write_text("[core]\n hooksPath = " + str(hooks) + "\n[commit]\n gpgSign = true\n")
        with patch.dict(os.environ, {"GIT_CONFIG_GLOBAL": str(config), "GIT_CONFIG_SYSTEM": str(config)}):
            result = self.execute()
        self.assertTrue(result["passed"], result["reasons"])
        self.assertFalse(marker.exists())
        self.assert_removed(result)

    def test_interrupt_cleans_fixture_before_propagating(self):
        real_process = harness.run_process
        fixture = []

        def boundary(command, cwd, env, timeout, stdout_path, stderr_path):
            if command[0] == str(self.binary):
                fixture.append(cwd)
                raise KeyboardInterrupt()
            return real_process(command, cwd, env, timeout, stdout_path, stderr_path)

        with patch.object(harness, "run_process", side_effect=boundary):
            with self.assertRaises(KeyboardInterrupt):
                harness.run_case(self.case(), self.binary, self.root / "interrupted", 5, False)
        self.assertEqual(len(fixture), 1)
        self.assertFalse(fixture[0].parent.exists())
        result = json.loads((self.root / "interrupted/result.json").read_text())
        self.assertFalse(result["passed"])
        self.assertTrue(result["reasons"])

    def test_expected_failure_rejects_unexpected_stdout(self):
        case = self.case()
        case.expected_exit = 1
        case.expected_stderr = "model unavailable"
        result = self.execute(case, code=1, output='{"unexpected":"partial output"}')
        self.assertFalse(result["passed"], result["reasons"])
        self.assertTrue(any("stdout" in reason for reason in result["reasons"]))
        self.assert_removed(result)

    def test_failed_doctor_cannot_be_reported_as_evaluation_success(self):
        self.binary.write_bytes(b"dummy binary for metadata hashing only")
        artifacts = self.root / "preflight-artifacts"
        fixtures = []

        def doctor(command, cwd, env, timeout, stdout_path, stderr_path):
            self.assertEqual(command, [str(self.binary), "--doctor", "--json"])
            fixtures.append(cwd)
            stdout_path.write_text(json.dumps({"available": False, "detail": "Model unavailable"}))
            stderr_path.write_text("")
            return {"returncode": 1, "timed_out": False, "duration": 0.001}

        with patch.object(harness, "run_process", side_effect=doctor), \
                patch.object(harness, "run_case") as run_case, \
                contextlib.redirect_stdout(io.StringIO()):
            exit_code = harness.main(["--binary", str(self.binary), "--artifacts", str(artifacts)])
        self.assertEqual(exit_code, 1)
        run_case.assert_not_called()
        summary = json.loads((artifacts / "summary.json").read_text())
        self.assertFalse(summary["passed"])
        self.assertIs(summary["doctor_status"]["available"], False)
        self.assertEqual(summary["cases"], [])
        self.assertTrue(summary["error"])
        self.assertEqual(len(fixtures), 1)
        self.assertFalse(fixtures[0].exists())

    def test_semantic_checkers_reject_wrong_commands_and_malformed_output(self):
        case = next(case for case in scenarios.CASES if case.name == "status-mixed")
        valid = {"outcome": "ready", "clarification": None, "summary": "Show status",
                 "changes_repository": False, "commands": [{"program": "git",
                 "args": ["status", "--short", "--branch"], "explanation": "Shows status"}]}
        self.assertEqual(case.check(valid), [])
        valid["commands"][0]["args"] = ["add", "--all"]
        self.assertTrue(case.check(valid))
        for malformed in (None, [], {"commands": "bad"}, {"commands": [None]}):
            self.assertTrue(case.check(malformed))
        self.assertTrue(scenarios.draft_check({"kind": "commit_message", "subject": "Fix ratio\u2028in denominator"}))

    def test_process_nonzero_exit_preserves_diagnostics(self):
        result = harness.run_process(
            [sys.executable, "-c", "import sys; print('partial'); print('failure detail', file=sys.stderr); sys.exit(7)"],
            self.root, dict(os.environ), 5, self.root / "out", self.root / "err")
        self.assertEqual(result["returncode"], 7)
        self.assertFalse(result["timed_out"])
        self.assertEqual((self.root / "out").read_text(), "partial\n")
        self.assertEqual((self.root / "err").read_text(), "failure detail\n")

    @unittest.skipUnless(os.name == "posix", "requires POSIX process groups")
    def test_timeout_kills_descendant_before_it_can_write(self):
        marker = self.root / "escaped"
        child_code = "import pathlib,time; time.sleep(0.6); pathlib.Path(%r).write_text('escaped')" % str(marker)
        parent_code = "import subprocess,sys,time; subprocess.Popen([sys.executable,'-c',%r]); time.sleep(10)" % child_code
        result = harness.run_process([sys.executable, "-c", parent_code], self.root,
                                     dict(os.environ), 0.15, self.root / "out", self.root / "err")
        self.assertTrue(result["timed_out"])
        self.assertNotEqual(result["returncode"], 0)
        time.sleep(0.7)
        self.assertFalse(marker.exists(), "descendant survived timeout")


if __name__ == "__main__":
    unittest.main()
