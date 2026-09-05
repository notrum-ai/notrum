#!/usr/bin/env python3
# Copyright 2026 Evgeniy Udodov
# SPDX-License-Identifier: GPL-3.0-only

import contextlib
import io
import json
import os
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

import ci
from source_revision import validate_revision

SHA = "1234567890abcdef1234567890abcdef12345678"


class CITests(unittest.TestCase):
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
            for name in ("Notrum.exe", "LICENSE.txt", "Register.ps1", "tests/test.exe", "tests/Run-Tests.ps1"):
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
