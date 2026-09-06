#!/usr/bin/env python3
# Copyright 2026 Evgeniy Udodov
# SPDX-License-Identifier: GPL-3.0-only
"""Exercise Make's coverage publication and test dispatch without compiling Rust."""

import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent
FAKE_RUNNER = '''import json
import os
from pathlib import Path
import subprocess
import sys

args = sys.argv[1:]
if args[0] != "cargo":
    raise SystemExit(subprocess.call(args))
with Path("cargo_calls.jsonl").open("a") as output:
    output.write(json.dumps(args) + "\\n")
if args[1] == "llvm-cov":
    report = Path(args[args.index("--output-path") + 1])
    report.write_text("" if os.environ["COVERAGE_TEST_EMPTY"] == "1" else
                      "SF:crates/notrum-core/src/lib.rs\\nDA:1,1\\nend_of_record\\n")
    raise SystemExit(int(os.environ["COVERAGE_TEST_EXIT"]))
'''


class CoverageTests(unittest.TestCase):
    def run_make(self, target="coverage", *, coverage="0", exit_code=0, empty=False):
        temporary = tempfile.TemporaryDirectory(prefix="notrum-coverage-test-")
        self.addCleanup(temporary.cleanup)
        directory = Path(temporary.name)
        runner = directory / "runner.py"
        runner.write_text(FAKE_RUNNER)
        report = directory / ".ci/coverage/lcov.info"
        report.parent.mkdir(parents=True)
        report.write_text("stale report")
        environment = dict(os.environ, COVERAGE_TEST_EXIT=str(exit_code),
                           COVERAGE_TEST_EMPTY="1" if empty else "0")
        # Do not inherit the outer aggregate's jobserver or variable overrides.
        for name in ("MAKEFLAGS", "MFLAGS", "MAKELEVEL", "MAKEOVERRIDES"):
            environment.pop(name, None)
        result = subprocess.run(
            ["make", "--no-print-directory", "-f", str(ROOT / "Makefile"),
             f"RUN={shlex.quote(sys.executable)} {shlex.quote(str(runner))}",
             f"COVERAGE={coverage}", target],
            cwd=directory, env=environment, capture_output=True, text=True, check=False,
        )
        calls = [json.loads(line) for line in
                 (directory / "cargo_calls.jsonl").read_text().splitlines()]
        return result, report, calls

    def test_success_replaces_stale_report_only_after_tests(self):
        result, report, calls = self.run_make()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue(report.read_text().startswith("SF:crates/"))
        self.assertFalse(report.with_suffix(".tmp").exists())
        self.assertEqual(len(calls), 1)

    def test_failed_tests_never_publish_partial_or_stale_coverage(self):
        result, report, _ = self.run_make(exit_code=42)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(report.exists())
        self.assertTrue(report.with_suffix(".tmp").exists())

    def test_empty_report_is_an_error(self):
        result, report, _ = self.run_make(empty=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(report.exists())

    def test_coverage_mode_runs_workspace_tests_once_and_keeps_doctests(self):
        result, _, calls = self.run_make("test", coverage="1")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual([call[1] for call in calls], ["llvm-cov", "test"])
        for call in calls:
            self.assertIn("--workspace", call)
            self.assertIn("--all-features", call)
        self.assertIn("--doc", calls[1])
        self.assertNotIn("--ignore-run-fail", calls[0])

    def test_default_test_target_remains_uninstrumented(self):
        result, _, calls = self.run_make("test")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(calls, [["cargo", "test", "--workspace", "--all-features"]])


if __name__ == "__main__":
    unittest.main()
