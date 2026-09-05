#!/usr/bin/env python3
# Copyright 2026 Evgeniy Udodov
# SPDX-License-Identifier: GPL-3.0-only
"""Offline publication tests, executed only by the Docker test-publish target."""

import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import Mock, patch
import zipfile

import app_version
import publish

MANIFEST = '[package]\nname = "notrum-app"\nversion = "0.1.0"\n\n[dependencies]\nother = "0.1.0"\n'
LOCK = ('version = 4\n\n[[package]]\nname = "notrum-app"\nversion = "0.1.0"\n'
        '\n[[package]]\nname = "other"\nversion = "0.1.0"\n')
SHA = "a" * 40


def fixture_archive(path, *, target="linux", sha=SHA, corrupt=False):
    files = {"notrum": b"application", "SOURCE_REVISION.txt": (sha + "\n").encode()}
    if target == "macos":
        files["Notrum.app/Contents/Resources/release.json"] = json.dumps({
            "source_revision": sha, "version": "0.1.1"}).encode()
    records = [{"path": name, "sha256": hashlib.sha256(data).hexdigest()} for name, data in files.items()]
    files["build.json"] = json.dumps({"source_revision": sha, "platform": target,
                                      "architecture": "arm64", "files": records}).encode()
    if corrupt:
        files["notrum"] = b"changed after packaging"
    if path.suffix == ".zip":
        with zipfile.ZipFile(path, "w") as archive:
            for name, data in files.items():
                archive.writestr(name, data)
    else:
        with tarfile.open(path, "w:gz") as archive:
            for name, data in files.items():
                info = tarfile.TarInfo("notrum-linux-arm64/" + name)
                info.size = len(data)
                archive.addfile(info, io.BytesIO(data))


class VersionTests(unittest.TestCase):
    def test_increments_only_patch(self):
        for old, new in (("0.1.0", "0.1.1"), ("0.1.9", "0.1.10"), ("2.10.99", "2.10.100")):
            self.assertEqual(app_version.next_version(old), new)
        for invalid in ("0.1", "0.1.0-rc1", "v0.1.0", "01.1.0", "0.1.65535"):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                app_version.next_version(invalid)

    def test_changes_application_only(self):
        updated = app_version.replace_version(MANIFEST, "0.1.0", "0.1.1")
        self.assertEqual(app_version.read_version(updated), "0.1.1")
        self.assertIn('other = "0.1.0"', updated)
        lock = app_version.replace_version(LOCK, "0.1.0", "0.1.1", lock=True)
        self.assertIn('name = "notrum-app"\nversion = "0.1.1"', lock)
        self.assertIn('name = "other"\nversion = "0.1.0"', lock)
        with self.assertRaises(ValueError):
            app_version.replace_version(LOCK, "0.2.0", "0.2.1", lock=True)

    def test_repository_resolution(self):
        for url in ("git@github.com:notrum-ai/notrum.git", "https://github.com/notrum-ai/notrum",
                    "ssh://git@github.com/notrum-ai/notrum.git"):
            self.assertEqual(publish.repository_from_remote(url), "notrum-ai/notrum")
        for url in ("https://secret@github.com/org/repo", "https://example.com/org/repo", "../repo"):
            with self.assertRaises(ValueError):
                publish.repository_from_remote(url)

    def test_codex_uses_requested_model_and_structured_output(self):
        def execute(command, **kwargs):
            self.assertEqual(command[0], "/example/codex")
            self.assertEqual(command[command.index("--model") + 1], "gpt-5.6-luna")
            self.assertIn('model_reasoning_effort="medium"', command)
            self.assertEqual(command[command.index("--sandbox") + 1], "read-only")
            self.assertIn("--skip-git-repo-check", command)
            self.assertIn("evidence fixture", kwargs["input"])
            output = Path(command[command.index("--output-last-message") + 1])
            output.write_text('{"improvements":["Faster search."],"bug_fixes":[]}', encoding="utf-8")
        with patch.object(publish.shutil, "which", return_value="/example/codex"), \
                patch.object(publish, "run", side_effect=execute):
            self.assertEqual(publish.codex_notes("evidence fixture")["improvements"], ["Faster search."])
        with patch.object(publish.shutil, "which", return_value="/example/codex"), \
                patch.object(publish, "run", side_effect=subprocess.CalledProcessError(1, ["codex"])):
            with self.assertRaises(subprocess.CalledProcessError):
                publish.codex_notes("evidence fixture")
        for value in ({}, {"improvements": [""], "bug_fixes": []},
                      {"improvements": "bad", "bug_fixes": []}):
            with self.assertRaises(ValueError):
                publish.validate_notes(value)


class RepositoryTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "app/notrum").mkdir(parents=True)
        (self.root / app_version.MANIFEST).write_text(MANIFEST, encoding="utf-8")
        (self.root / "Cargo.lock").write_text(LOCK, encoding="utf-8")
        self.git("init", "-q")
        self.git("config", "user.name", "Fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        self.git("add", ".")
        self.git("commit", "-qm", "Initial version")
        self.initial = self.git("rev-parse", "HEAD").strip()
        self.state_path = self.root / ".git/publish.json"
        for name, value in (("ROOT", self.root), ("STATE", self.state_path), ("git", self.git)):
            patcher = patch.object(publish, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)

    def git(self, *args, input=None, env=None, stdout=subprocess.PIPE):
        return subprocess.run(["git", *args], cwd=self.root, input=input, text=True,
                              env={**publish.os.environ, **(env or {})}, stdout=stdout,
                              stderr=subprocess.PIPE, check=True).stdout or ""

    def feature(self):
        (self.root / "feature.txt").write_text("feature", encoding="utf-8")
        self.git("add", "feature.txt")
        self.git("commit", "-qm", "Add a feature")
        return self.git("rev-parse", "HEAD").strip()

    def state(self):
        with patch.object(publish, "release_notes", return_value="## Improvements\n\n- Feature.\n"):
            return publish.new_state("notrum-ai/notrum", "master", self.feature())

    def test_boundary_uses_version_value_not_any_manifest_edit(self):
        head = self.feature()
        self.assertEqual(publish.previous_version_commit(head), self.initial)
        manifest = self.root / app_version.MANIFEST
        manifest.write_text(MANIFEST.replace("[dependencies]", "# Formatting\n[dependencies]"), encoding="utf-8")
        self.git("add", app_version.MANIFEST)
        self.git("commit", "-qm", "Document dependencies")
        self.assertEqual(publish.previous_version_commit("HEAD"), self.initial)
        manifest.write_text(MANIFEST.replace('version = "0.1.0"', 'version = "0.1.1"'), encoding="utf-8")
        self.git("add", app_version.MANIFEST)
        self.git("commit", "-qm", "Release version")
        bumped = self.git("rev-parse", "HEAD").strip()
        self.assertEqual(publish.previous_version_commit("HEAD"), bumped)
        with self.assertRaisesRegex(ValueError, "no commits"):
            publish.new_state("notrum-ai/notrum", "master", bumped)

    def test_commit_and_crash_recovery_do_not_bump_twice(self):
        state = self.state()
        publish.save_state(state)
        publish.commit_version(state)
        release = state["sha"]
        paths = set(self.git("diff", "--name-only", state["head"], release).splitlines())
        self.assertEqual(paths, set(app_version.VERSION_FILES))
        self.assertEqual(self.git("log", "-1", "--format=%ae").strip(), publish.IDENTITY["GIT_AUTHOR_EMAIL"])
        state["sha"] = None  # Simulate a crash just after Git committed, before saving the SHA.
        publish.commit_version(state)
        self.assertEqual(state["sha"], release)
        self.assertEqual(app_version.read_version((self.root / app_version.MANIFEST).read_text()), "0.1.1")
        self.assertEqual(self.git("rev-list", "--count", f"{state['head']}..HEAD").strip(), "1")

    def test_unrelated_changes_are_preserved(self):
        state = self.state()
        path = self.root / "unrelated.txt"
        path.write_text("another agent", encoding="utf-8")
        self.git("add", "unrelated.txt")
        with self.assertRaisesRegex(ValueError, "unrelated"):
            publish.commit_version(state)
        self.assertEqual(path.read_text(), "another agent")
        self.assertEqual((self.root / app_version.MANIFEST).read_text(), MANIFEST)
        self.assertEqual(self.git("diff", "--cached", "--name-only").strip(), "unrelated.txt")

    def test_partial_version_write_can_be_resumed(self):
        state = self.state()
        (self.root / app_version.MANIFEST).write_text(state["updates"][app_version.MANIFEST], encoding="utf-8")
        publish.commit_version(state)
        self.assertEqual((self.root / "Cargo.lock").read_text(), state["updates"]["Cargo.lock"])

    def test_independent_version_edit_is_rejected(self):
        state = self.state()
        manifest = self.root / app_version.MANIFEST
        manifest.write_text(MANIFEST + "# another agent\n", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "independently"):
            publish.commit_version(state)
        self.assertTrue(manifest.read_text().endswith("# another agent\n"))


class ArchiveTests(unittest.TestCase):
    def test_provenance_and_every_file_checksum(self):
        with tempfile.TemporaryDirectory() as temporary:
            for extension in ("tar.gz", "zip"):
                path = Path(temporary) / ("fixture." + extension)
                fixture_archive(path)
                self.assertEqual(publish.validate_archive(path, SHA, "0.1.1", "linux"), "arm64")
                with self.assertRaisesRegex(ValueError, "different build"):
                    publish.validate_archive(path, "b" * 40, "0.1.1", "linux")
                fixture_archive(path, corrupt=True)
                with self.assertRaisesRegex(ValueError, "checksum"):
                    publish.validate_archive(path, SHA, "0.1.1", "linux")

    def test_macos_version_is_verified(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "fixture.tar.gz"
            fixture_archive(path, target="macos")
            with self.assertRaisesRegex(ValueError, "version"):
                publish.validate_archive(path, SHA, "0.1.2", "macos")

    def test_failed_build_does_not_mark_assets_ready(self):
        with tempfile.TemporaryDirectory() as temporary:
            state = {"version": "0.1.1", "sha": SHA, "assets": {}}
            with patch.object(publish, "WORK", Path(temporary)), patch.object(publish, "require_clean"), \
                    patch.object(publish, "run", side_effect=subprocess.CalledProcessError(2, ["make"])):
                with self.assertRaises(subprocess.CalledProcessError):
                    publish.build_assets(state)
            self.assertEqual(state["assets"], {})

    def test_ready_assets_are_not_rebuilt_and_changed_bytes_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "0.1.1").mkdir()
            path = root / "0.1.1/archive.zip"
            path.write_bytes(b"archive")
            state = {"version": "0.1.1", "sha": SHA, "assets": {path.name: publish.file_digest(path)}}
            with patch.object(publish, "WORK", root), patch.object(publish, "run") as execute:
                publish.build_assets(state)
                execute.assert_not_called()
                path.write_bytes(b"changed")
                with self.assertRaisesRegex(ValueError, "assets changed"):
                    publish.build_assets(state)


class UploadTests(unittest.TestCase):
    def test_draft_retry_skips_existing_assets_and_verifies_digests(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            asset = directory / "notrum.zip"
            asset.write_bytes(b"application")
            state = {"sha": SHA, "tag": "v0.1.1", "branch": "master", "notes": "Notes",
                     "assets": {asset.name: publish.file_digest(asset)}, "published": False}
            release = {"id": 42, "body": "Notes", "draft": True,
                       "html_url": "https://example.invalid/release"}
            github = Mock()
            github.url = "https://github.com/notrum-ai/notrum.git"
            github.remote_tag.return_value = SHA
            github.release.return_value = release
            remote_asset = {"name": asset.name, "state": "uploaded", "size": asset.stat().st_size,
                            "digest": "sha256:" + publish.file_digest(asset)}
            github.api.return_value = [[remote_asset]]
            github.publish_release.return_value = {**release, "draft": False}

            def git(*args, **kwargs):
                if args[0] == "tag":
                    return "v0.1.1\n"
                return SHA + "\n"

            with patch.object(publish, "git", side_effect=git), patch.object(publish, "require_clean"), \
                    patch.object(publish, "save_state"):
                publish.upload_release(state, github, directory)
            self.assertTrue(state["published"])
            github.upload_asset.assert_not_called()
            github.verify_asset.assert_called_once_with(remote_asset, asset)
            github.publish_release.assert_called_once_with(release)

    def test_missing_asset_is_uploaded_through_api(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            asset = directory / "notrum.zip"
            asset.write_bytes(b"application")
            state = {"sha": SHA, "tag": "v0.1.1", "branch": "master", "notes": "Notes",
                     "assets": {asset.name: publish.file_digest(asset)}, "published": False}
            release = {"id": 42, "body": "Notes", "draft": True,
                       "html_url": "https://example.invalid/release"}
            uploaded = {"name": asset.name, "state": "uploaded", "size": asset.stat().st_size,
                        "digest": "sha256:" + publish.file_digest(asset)}
            github = Mock()
            github.url = "https://github.com/notrum-ai/notrum.git"
            github.remote_tag.return_value = SHA
            github.release.return_value = release
            github.api.side_effect = [[[]], [[uploaded]]]
            github.upload_asset.return_value = uploaded
            github.publish_release.return_value = {**release, "draft": False}

            def git(*args, **kwargs):
                if args[0] == "tag":
                    return "v0.1.1\n"
                return SHA + "\n"

            with patch.object(publish, "git", side_effect=git), patch.object(publish, "require_clean"), \
                    patch.object(publish, "save_state"):
                publish.upload_release(state, github, directory)
            github.upload_asset.assert_called_once_with(release, asset)
            github.verify_asset.assert_called_once_with(uploaded, asset)

    def test_asset_digest_must_match_local_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            asset = Path(temporary) / "notrum.zip"
            asset.write_bytes(b"application")
            github = publish.GitHub("notrum-ai/notrum", "secret")
            with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                github.verify_asset({"state": "uploaded", "size": asset.stat().st_size,
                                     "digest": "sha256:" + "0" * 64}, asset)
            with self.assertRaisesRegex(ValueError, "did not return"):
                github.verify_asset({"state": "uploaded", "size": asset.stat().st_size}, asset)
            with self.assertRaisesRegex(ValueError, "unexpected size"):
                github.verify_asset({"state": "uploaded", "size": asset.stat().st_size + 1,
                                     "digest": "sha256:" + publish.file_digest(asset)}, asset)

    def test_conflicting_remote_tag_is_never_overwritten(self):
        github = Mock()
        github.remote_tag.return_value = "b" * 40
        with patch.object(publish, "require_clean"), self.assertRaisesRegex(ValueError, "different commit"):
            publish.upload_release({"sha": SHA, "tag": "v0.1.1"}, github, Path("/unused"))
        github.network_git.assert_not_called()


if __name__ == "__main__":
    unittest.main()
