#!/usr/bin/env python3
# Copyright 2026 Evgeniy Udodov
# SPDX-License-Identifier: GPL-3.0-only
"""Native Linux checks for launch paths and the isolated fatal-error dialog."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

from generate_demo_data import generate_demo_workspace
from register_linux import register

BINARY = Path("/var/cache/notrum/target/debug/notrum-app")


def wait_for(check, seconds=8):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        result = check()
        if result:
            return result
        time.sleep(0.05)
    raise AssertionError("desktop condition timed out")


def windows(env, title):
    result = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", title], env=env, text=True, capture_output=True)
    return result.stdout.split()


def external(root, env):
    workspace = root / "workspace"
    generate_demo_workspace(workspace)
    first = root / "Заметка #1.MD"
    second = root / "two space.txt"
    for path in (first, second):
        path.write_text("External unchanged\n", encoding="utf-8")
    subprocess.run([str(BINARY), "--workspace", str(workspace), "--open", first.name, second.name,
                    "--smoke-exit-ms", "1400"], env=env, cwd=root, check=True, timeout=20)
    settings = workspace / ".notrum/settings.json"
    initial = json.loads(settings.read_text())
    assert [record["absolute_path"] for record in initial["external_files"]] == [str(first), str(second)]
    assert initial["selected_external"] == str(first)
    package = root / 'package $ % " 日本語'
    package.mkdir()
    shutil.copy2(BINARY, package / "notrum")
    shutil.copy2(Path(__file__).resolve().parent.parent / "app/notrum/assets/notrum-app-icon.svg", package / "notrum.svg")
    data = root / "data"
    register(package, data)
    desktop = data / "applications/org.notrum.Notrum.desktop"
    with (root / "desktop.log").open("w") as log:
        result = subprocess.run(["gio", "launch", str(desktop), str(second), str(first)],
                                env=env, stdout=log, stderr=log, timeout=15)
        if result.returncode:
            raise AssertionError((root / "desktop.log").read_text())
        window = wait_for(lambda: windows(env, "^Notrum$")[0:1])[0]
        try:
            wait_for(lambda: json.loads(settings.read_text())["selected_external"] == str(second))
        finally:
            subprocess.run(["xdotool", "windowclose", window], env=env, check=True)
            wait_for(lambda: not windows(env, "^Notrum$"))
    for path in (first, second):
        assert path.read_text() == "External unchanged\n"
    fresh_home = root / "first home"
    (fresh_home / "Downloads").mkdir(parents=True)
    fresh = dict(env, HOME=str(fresh_home))
    process = subprocess.Popen([str(BINARY), "--open", str(first), "--smoke-exit-ms", "4000"], env=fresh, cwd=root)
    try:
        wait_for(lambda: windows(fresh, "^Notrum$"))
        time.sleep(0.3)
        default = fresh_home / "Downloads/Notes"
        assert not default.exists()
        subprocess.run(["xdotool", "mousemove", "785", "495", "click", "1"], env=fresh, check=True)
        def pending_opened():
            try:
                return json.loads((default / ".notrum/settings.json").read_text()).get("selected_external") == str(first)
            except FileNotFoundError:
                return False
        wait_for(pending_opened)
        assert process.wait(timeout=8) == 0
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
    concurrent(root, env)


def concurrent(root, env):
    workspace = root / "concurrent workspace"
    generate_demo_workspace(workspace)
    path = root / "concurrent.md"
    path.write_text("Original external body\n")
    processes = []

    def xd(*args):
        return subprocess.run(["xdotool", *args], env=env, text=True, capture_output=True)

    def window(process):
        found = xd("search", "--all", "--onlyvisible", "--pid", str(process.pid), "--name", "^Notrum$").stdout.split()
        return found[0] if found else None

    def edit(window_id, text):
        xd("windowraise", window_id, "windowfocus", "--sync", window_id).check_returncode()
        time.sleep(0.08)
        xd("mousemove", "--window", window_id, "500", "160", "click", "1").check_returncode()
        time.sleep(0.08)
        xd("key", "--clearmodifiers", "ctrl+End").check_returncode()
        time.sleep(0.08)
        xd("type", "--clearmodifiers", "--delay", "1", text).check_returncode()

    try:
        for _ in range(2):
            process = subprocess.Popen([str(BINARY), "--workspace", str(workspace), "--open", str(path)],
                                       env=env, cwd=root, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            processes.append(process)
            wait_for(lambda: window(process))
            time.sleep(0.6)
        first, second = map(window, processes)
        assert first != second and all(process.poll() is None for process in processes)
        edit(first, "FIRST_PROCESS_SAVED")
        edit(second, "SECOND_PROCESS_UNSAVED")
        wait_for(lambda: "FIRST_PROCESS_SAVED" in path.read_text())
        committed = path.read_bytes()
        time.sleep(2)
        assert b"SECOND_PROCESS_UNSAVED" not in committed
        # A competing recovery write is rejected while the first window owns
        # the artifact. A subsequent edit can persist after its checked save.
        edit(second, "_RECOVERY_RETRY")
        recovery = workspace / ".notrum/recovery"
        records = wait_for(lambda: list(recovery.glob("*.nrrec")))
        time.sleep(1)
        assert path.read_bytes() == committed, "stale window overwrote the saved file"
        assert any(b"SECOND_PROCESS_UNSAVED_RECOVERY_RETRY" in record.read_bytes() for record in records)
    finally:
        # Simulate independent crashes; never let a close prompt discard work.
        for process in processes:
            if process.poll() is None:
                process.kill()
            process.wait()



def crash(root, env):
    sentinel = "synthetic protected body must never reach diagnostics"
    with (root / "crash.stderr").open("w+") as log:
        process = subprocess.Popen([str(BINARY), "--smoke-panic"], env=env, cwd=root, stdout=log, stderr=log)
        try:
            window = wait_for(lambda: windows(env, "Notrum")[0:1])[0]
            subprocess.run(["xdotool", "windowfocus", "--sync", window, "key", "Tab", "Return"], env=env, check=True)
            def clipboard():
                result = subprocess.run(["xclip", "-selection", "clipboard", "-o"], env=env, capture_output=True, text=True, timeout=2)
                return result.stdout if "Backtrace:" in result.stdout else None
            copied = wait_for(clipboard)
            assert sentinel not in copied
            subprocess.run(["xdotool", "windowclose", window], env=env, check=True)
            assert process.wait(timeout=8) == 1
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
        log.seek(0)
        assert sentinel not in log.read()
    report = (root / "error.log").read_text()
    assert "Backtrace:" in report and sentinel not in report
    headless = dict(env, DISPLAY=":9876", WAYLAND_DISPLAY="notrum-missing")
    result = subprocess.run([str(BINARY), "--smoke-panic"], cwd=root, env=headless, capture_output=True, text=True, timeout=8)
    assert result.returncode == 1 and sentinel not in result.stderr


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("scenario", choices=["external", "crash"])
    scenario = parser.parse_args().scenario
    with tempfile.TemporaryDirectory(prefix="notrum-desktop-") as temporary:
        root = Path(temporary)
        for name in ("home", "runtime"):
            (root / name).mkdir(mode=0o700)
        env = dict(os.environ, HOME=str(root / "home"), DISPLAY=":99", XDG_RUNTIME_DIR=str(root / "runtime"),
                   XDG_DATA_HOME=str(root / "data"), FLOEM_FORCE_TINY_SKIA="1", GDK_BACKEND="x11", GSK_RENDERER="cairo")
        server = subprocess.Popen(["Xvfb", ":99", "-screen", "0", "1240x800x24"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            wait_for(lambda: Path("/tmp/.X11-unix/X99").exists())
            (external if scenario == "external" else crash)(root, env)
            print(f"DESKTOP_SMOKE {scenario}=passed")
        finally:
            server.terminate()
            server.wait(timeout=5)


if __name__ == "__main__":
    main()
