#!/usr/bin/env python3
# Copyright 2026 Evgeniy Udodov
# SPDX-License-Identifier: GPL-3.0-only

import contextlib
import io
import json
import os
from pathlib import Path
import sys
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import Mock, patch
import zipfile

import ci
from ci_diagnostics import UI_SCENARIOS, rust_test_report
from source_revision import validate_revision
import ui_acceptance

SHA = "1234567890abcdef1234567890abcdef12345678"


class CITests(unittest.TestCase):
    def test_ui_diagnostics_keep_only_context_and_known_traceback_locations(self):
        self.assertEqual(UI_SCENARIOS, set(ui_acceptance.SCENARIOS))
        try:
            ui_acceptance.wait_until("SYNTHETIC_SECRET", lambda: False, timeout=0)
        except ui_acceptance.AcceptanceFailure as error:
            error.add_note("SYNTHETIC_SECRET")
            lines = ui_acceptance.failure_diagnostics("password_change", "rotate", error)
        self.assertEqual(lines[0], "UI_ACCEPTANCE_DIAGNOSTIC scenario=password_change "
                         "stage=rotate exception=AcceptanceFailure")
        self.assertEqual(len(lines), 2)
        self.assertRegex(lines[1], r" location=tools/ui_acceptance\.py:[1-9][0-9]*$")
        for line in lines:
            self.assertEqual(ci.safe_line(line), line)
            self.assertNotIn("SYNTHETIC_SECRET", line)
            self.assertNotIn(str(Path(__file__).parent), line)
            self.assertNotIn("test_ci.py", line)

    def test_ui_diagnostics_do_not_render_unknown_types_or_subprocess_payloads(self):
        class PrivateError(Exception):
            def __str__(self):
                raise AssertionError("exception payload must not be rendered")

        PrivateError.__name__ = "SYNTHETIC_SECRET"
        # A custom class cannot impersonate an allowlisted exception by name.
        disguised = type("ValueError", (Exception,), {})
        for error, kind in (
            (PrivateError(), "Exception"),
            (disguised("SYNTHETIC_SECRET"), "Exception"),
            (subprocess.CalledProcessError(1, ["SYNTHETIC_SECRET"],
                                          output="SYNTHETIC_SECRET"), "CalledProcessError"),
            (subprocess.TimeoutExpired(["SYNTHETIC_SECRET"], 1,
                                       stderr="SYNTHETIC_SECRET"), "TimeoutExpired"),
        ):
            with self.subTest(kind=kind):
                lines = ui_acceptance.failure_diagnostics("password_change", "rotate", error)
                self.assertEqual(lines, ["UI_ACCEPTANCE_DIAGNOSTIC scenario=password_change "
                                         f"stage=rotate exception={kind}"])
                self.assertEqual(ci.safe_line(lines[0]), lines[0])

    def test_ui_diagnostic_filter_rejects_unknown_fields_and_context(self):
        valid = ("UI_ACCEPTANCE_DIAGNOSTIC scenario=password_change stage=rotate "
                 "exception=AcceptanceFailure location=tools/ui_acceptance.py:123")
        for line in (
            valid.replace("password_change", "private_scenario"),
            valid.replace("stage=rotate", "stage=private_stage"),
            valid.replace("password_change", "ai"),
            valid.replace("AcceptanceFailure", "PrivateError"),
            valid.replace("tools/ui_acceptance.py", "tools/private.py"),
            valid.replace("tools/ui_acceptance.py", "/tmp/private/tools/ui_acceptance.py"),
            valid.replace(":123", ":0"),
            valid.replace(":123", ":-1"),
            valid + " secret=SYNTHETIC_SECRET",
            valid + " crates/private/src/lib.rs:12:3",
            valid + " (os error 5)",
            valid + "\nSYNTHETIC_SECRET",
        ):
            with self.subTest(line=line):
                self.assertIsNone(ci.safe_line(line))
        self.assertEqual(ci.safe_line(valid), valid)

    def run_ui_with_fake_driver(self, *, scenario_fails=True, secondary_failures=False,
                                cleanup_fails=False):
        driver = Mock(spec=ui_acceptance.WindowDriver)
        driver.scenario = "password_change"
        driver.stage = "startup"
        # Preserve the real stage validator while replacing construction below.
        set_stage = ui_acceptance.WindowDriver.set_stage
        driver.set_stage.side_effect = lambda stage: set_stage(driver, stage)
        output = io.StringIO()

        def scenario(current, workspace):
            current.set_stage("rotate")
            if scenario_fails:
                ui_acceptance.wait_until("SYNTHETIC_SECRET", lambda: False, timeout=0)

        def capture():
            self.assertIn("stage=rotate exception=AcceptanceFailure", output.getvalue())
            self.assertIn("location=tools/ui_acceptance.py:", output.getvalue())
            if secondary_failures:
                raise ValueError("SYNTHETIC_SECRET")
            return None

        driver.capture_failure.side_effect = capture
        if secondary_failures or cleanup_fails:
            driver.cleanup.side_effect = OSError("SYNTHETIC_SECRET")
        if secondary_failures:
            driver.sanitize_failure_logs.side_effect = PermissionError("SYNTHETIC_SECRET")
        with patch.object(ui_acceptance, "APP_BINARY", Path(__file__)), \
                patch.object(sys, "argv", ["ui_acceptance.py", "password_change"]), \
                patch.object(ui_acceptance, "WindowDriver", return_value=driver), \
                patch.object(ui_acceptance, "copy_demo", return_value=Path("unused")), \
                patch.dict(ui_acceptance.SCENARIOS, password_change=scenario), \
                contextlib.redirect_stderr(output), contextlib.redirect_stdout(output):
            code = ui_acceptance.main()
        self.assertNotIn("SYNTHETIC_SECRET", output.getvalue())
        return code, output.getvalue(), driver

    def test_ui_failure_survives_capture_cleanup_and_artifact_failures(self):
        code, output, driver = self.run_ui_with_fake_driver(secondary_failures=True)
        self.assertEqual(code, 1)
        self.assertEqual(output.count("UI_ACCEPTANCE_FAIL scenario=password_change"), 1)
        self.assertNotIn("UI_ACCEPTANCE_PASS", output)
        stages = ["stage=rotate", "stage=capture", "stage=cleanup", "stage=artifacts"]
        positions = [output.index(stage) for stage in stages]
        self.assertEqual(positions, sorted(positions))
        driver.cleanup.assert_called_once()
        driver.sanitize_failure_logs.assert_called_once()
        driver.remove_success_artifacts.assert_not_called()

    def test_ui_success_and_cleanup_failure_keep_exit_status(self):
        code, output, driver = self.run_ui_with_fake_driver(scenario_fails=False)
        self.assertEqual(code, 0)
        self.assertEqual(output, "UI_ACCEPTANCE_PASS scenario=password_change\n")
        driver.remove_success_artifacts.assert_called_once()
        driver.capture_failure.assert_not_called()
        code, output, driver = self.run_ui_with_fake_driver(scenario_fails=False,
                                                         cleanup_fails=True)
        self.assertEqual(code, 1)
        self.assertIn("stage=cleanup exception=OSError", output)
        self.assertNotIn("UI_ACCEPTANCE_PASS", output)
        driver.sanitize_failure_logs.assert_called_once()

    def test_ui_failure_reaches_ci_console_and_report_without_secrets(self):
        code, diagnostics, _ = self.run_ui_with_fake_driver()
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, SOURCE_REVISION=SHA):
            root = Path(temporary)
            console = io.StringIO()
            with patch.object(ci, "ROOT", root), patch.object(ci, "REPORTS", root / "reports"), \
                    contextlib.redirect_stdout(console):
                result = ci.run("linux", [sys.executable, "-c",
                                         f"import sys; sys.stderr.write({diagnostics!r}); "
                                         f"print('SYNTHETIC_SECRET'); sys.exit({code})"])
            self.assertEqual(result, 1)
            report = json.loads((root / "reports/linux/status.json").read_text())
            self.assertEqual(report["status"], "failed")
            self.assertEqual(report["exit_code"], 1)
            log = (root / "reports/linux/checks.log").read_text()
            self.assertEqual(console.getvalue(), log)
            self.assertIn("stage=rotate exception=AcceptanceFailure", log)
            self.assertIn("location=tools/ui_acceptance.py:", log)
            for path in (root / "reports").rglob("*"):
                if path.is_file():
                    self.assertNotIn("SYNTHETIC_SECRET", path.read_text())
                    self.assertNotIn(str(root), path.read_text())
            self.assertNotIn("diagnostic directory:", log)

    def test_revision_requires_exact_checkout_sha(self):
        self.assertEqual(validate_revision(SHA, SHA), SHA)
        for value in ("", "master", SHA[:7], SHA + "-dirty", "f" * 40):
            with self.subTest(value=value), self.assertRaises(ValueError):
                validate_revision(value, SHA)

    def test_diagnostics_exclude_payloads_thread_names_and_document_data(self):
        secret = "synthetic password and protected editor body"
        for line in (secret, f"thread '{secret}' panicked", f"left: {secret}", f"error: {secret}"):
            self.assertNotIn(secret, ci.safe_line(line) or "")
        self.assertEqual(ci.safe_line(f"thread '{secret}' panicked at crates/core/src/lib.rs:12:3:"),
                         "Rust diagnostic location: crates/core/src/lib.rs:12:3")
        self.assertEqual(ci.safe_line(f"UI_ACCEPTANCE_FAIL scenario=secure: {secret}"),
                         "UI_ACCEPTANCE_FAIL scenario=secure")
        self.assertEqual(ci.safe_line("FAIL: test_icon (__main__.PackageMacosTests.test_icon)"),
                         "Python test FAIL: __main__.PackageMacosTests.test_icon")
        self.assertEqual(ci.safe_line('File "/workspace/tools/test_package_macos.py", line 111, in test_icon'),
                         "Python diagnostic location: tools/test_package_macos.py:111")
        self.assertEqual(ci.safe_line(f"AssertionError: {secret}"),
                         "Python exception: AssertionError (details omitted)")
        self.assertEqual(ci.safe_line("AssertionError"),
                         "Python exception: AssertionError (details omitted)")
        self.assertEqual(ci.safe_line(f"subprocess.CalledProcessError: {secret}"),
                         "Python exception: CalledProcessError (details omitted)")
        self.assertIsNone(ci.safe_line(f"FAIL: test_icon (__main__.PackageMacosTests.test_icon) {secret}"))
        self.assertEqual(ci.safe_line(f'PreCommit {{ stage: Replace, message: "{secret} (os error 5)" }}'),
                         "Rust failure: stage=Replace os_error=5")
        self.assertEqual(ci.safe_line(f'Os {{ code: 32, kind: PermissionDenied, message: "{secret}" }}'),
                         "Rust failure: os_error=32")
        self.assertEqual(ci.safe_line("Rust failure: stage=Replace os_error=5"),
                         "Rust failure: stage=Replace os_error=5")

    def test_failed_command_retains_only_safe_report(self):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, SOURCE_REVISION=SHA):
            root = Path(temporary)
            console = io.StringIO()
            with patch.object(ci, "ROOT", root), patch.object(ci, "REPORTS", root / "reports"), \
                    contextlib.redirect_stdout(console):
                code = ci.run("linux", [sys.executable, "-c",
                                       "print('SYNTHETIC_SECRET'); print('test tests::example ... FAILED'); exit(7)"])
            self.assertEqual(code, 7)
            self.assertNotIn("SYNTHETIC_SECRET", console.getvalue())
            report = json.loads((root / "reports/linux/status.json").read_text())
            self.assertEqual(report["status"], "failed")
            self.assertEqual(report["source_revision"], SHA)
            for path in (root / "reports").rglob("*"):
                if path.is_file():
                    self.assertNotIn("SYNTHETIC_SECRET", path.read_text())

    def test_windows_rust_report_keeps_failures_and_locations_without_payloads(self):
        raw = "\n".join([
            "test tests::first ... ok",
            "test tests::second ... FAILED",
            "thread 'SYNTHETIC_SECRET' panicked at app/notrum/src/main.rs:123:4:",
            "assertion failed: SYNTHETIC_SECRET",
            "left: C:\\Users\\SYNTHETIC_SECRET\\note.md",
            "test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s",
        ])
        report = rust_test_report(raw.splitlines())
        self.assertEqual(report["failedTests"], ["tests::second"])
        self.assertIn("Rust diagnostic location: app/notrum/src/main.rs:123:4", report["diagnostics"])
        self.assertIn("test tests::first ... ok", report["diagnostics"])
        self.assertNotIn("SYNTHETIC_SECRET", json.dumps(report))
        self.assertEqual(rust_test_report(["SYNTHETIC_SECRET"]),
                         {"failedTests": [], "diagnostics": []})
        with tempfile.TemporaryDirectory() as temporary:
            log = Path(temporary) / "Windows 日本語 test.log"
            log.write_text(raw, encoding="utf-8-sig")
            result = subprocess.run([sys.executable, str(Path(ci.__file__).with_name("ci_diagnostics.py")),
                                     str(log)], check=True, text=True, capture_output=True)
            self.assertEqual(json.loads(result.stdout), report)
            self.assertNotIn("SYNTHETIC_SECRET", result.stdout + result.stderr)

    def test_linux_archive_preserves_mode_and_excludes_unrelated_files(self):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, SOURCE_REVISION=SHA):
            root = Path(temporary)
            directory = root / "dist/linux/x86_64"
            directory.mkdir(parents=True)
            for name in ("notrum", "notrum.svg", "Register.py", "org.notrum.Notrum.desktop", "LICENSE.txt"):
                (directory / name).write_text("fixture")
            (directory / "notrum").chmod(0o755)
            (directory / "personal.md").write_text("must not be archived")
            with patch.object(ci, "ROOT", root), patch.object(ci, "ARTIFACTS", root / "artifacts"), \
                    patch("ci.platform.machine", return_value="x86_64"):
                ci.package("linux")
            with tarfile.open(root / "artifacts/linux/notrum-linux-x86_64.tar.gz") as archive:
                prefix = "notrum-linux-x86_64/"
                self.assertEqual(archive.getmember(prefix + "notrum").mode & 0o777, 0o755)
                self.assertEqual(archive.extractfile(prefix + "SOURCE_REVISION.txt").read().strip().decode(), SHA)
                self.assertFalse(any(name.endswith("personal.md") for name in archive.getnames()))

    def test_windows_transfer_verifies_revision_checksum_and_paths(self):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, SOURCE_REVISION=SHA):
            root = Path(temporary)
            directory = root / "dist/windows/x86_64"
            (directory / "tests").mkdir(parents=True)
            for name in ("Notrum.exe", "LICENSE.txt", "Register.ps1", "tests/test.exe", "tests/Run-Tests.ps1", "tests/ci_diagnostics.py"):
                (directory / name).write_text("fixture")
            for folder in (directory, directory / "tests"):
                (folder / "dependencies.json").write_text("{}")
            (directory / "tests/tests.json").write_text('["test.exe"]')
            with patch.object(ci, "ROOT", root), patch.object(ci, "ARTIFACTS", root / "artifacts"):
                ci.package("windows-tests")
            archive = root / "artifacts/windows-tests/notrum-windows-tests-x86_64.zip"
            with self.assertRaises(ValueError):
                ci.extract_windows(archive, root / "wrong", "f" * 40)
            ci.extract_windows(archive, root / "valid", SHA)
            self.assertTrue((root / "valid/tests/test.exe").is_file())
            tampered = root / "tampered.zip"
            with zipfile.ZipFile(archive) as original, zipfile.ZipFile(tampered, "w") as output:
                for name in original.namelist():
                    output.writestr(name, b"modified" if name == "Notrum.exe" else original.read(name))
            with self.assertRaises(ValueError):
                ci.extract_windows(tampered, root / "tampered", SHA)
            with zipfile.ZipFile(root / "unsafe.zip", "w") as output:
                output.writestr("../outside", "must not escape")
            with self.assertRaises(ValueError):
                ci.extract_windows(root / "unsafe.zip", root / "unsafe", SHA)
            self.assertFalse((root / "outside").exists())


if __name__ == "__main__":
    unittest.main()
