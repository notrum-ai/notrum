#!/usr/bin/env python3
# Copyright 2026 Evgeniy Udodov
# SPDX-License-Identifier: GPL-3.0-only
"""Shared safe CI diagnostics, also shipped with the native Windows test kit."""

import json
from pathlib import Path
import re
import sys


def safe_line(line):
    """Never copy arbitrary test output, panic payloads, app logs or document paths."""
    line = re.sub(r"\x1b\[[0-9;]*[A-Za-z]", "", line).strip()
    if re.fullmatch(r"test [a-zA-Z0-9_:]+ \.\.\. (ok|FAILED|ignored)", line):
        return line
    if re.fullmatch(r"test result: (ok|FAILED)\. [0-9a-z ;.]+", line):
        return line
    if re.fullmatch(r"Rust failure: (?:stage=[A-Za-z]+(?: os_error=[0-9]+)?|os_error=[0-9]+)", line):
        return line
    stage = re.search(r"PreCommit \{ stage: (OpenTarget|Scan|CreateTemp|Write|FileSync|ConflictCheck|Replace|SourceRemove|ParentSync)\b", line)
    os_error = re.search(r"(?:\(os error |Os \{ code: )([0-9]{1,5})(?:\)|,)", line)
    if stage or os_error:
        parts = ([f"stage={stage[1]}"] if stage else []) + ([f"os_error={os_error[1]}"] if os_error else [])
        return "Rust failure: " + " ".join(parts)
    match = re.fullmatch(r"(FAIL|ERROR): (test_[a-zA-Z0-9_]+) \(([a-zA-Z0-9_.]+)\)", line)
    if match:
        return f"Python test {match[1]}: {match[3]}"
    match = re.fullmatch(r'File "(?:[^"\n]*/)?(tools/[a-zA-Z0-9_]+\.py)", line ([0-9]+), in [a-zA-Z0-9_<>]+', line)
    if match:
        return f"Python diagnostic location: {match[1]}:{match[2]}"
    match = re.match(r"(?:subprocess\.)?(AssertionError|ValueError|OSError|FileNotFoundError|PermissionError|CalledProcessError|TimeoutExpired)(?::|$)", line)
    if match:
        return f"Python exception: {match[1]} (details omitted)"
    match = re.match(r"UI_ACCEPTANCE_(PASS|FAIL) scenario=([a-z_]+)(?:\s|:|$)", line)
    if match:
        return f"UI_ACCEPTANCE_{match[1]} scenario={match[2]}"
    match = re.match(r"(DESKTOP_SMOKE) (external|crash)=passed$", line)
    if match:
        return match[0]
    match = re.match(r"NATIVE_EXTERNAL_SMOKE_OK cold_start=(True|False) ordered_files=([0-9]+)", line)
    if match:
        return match[0]
    if re.fullmatch(r"SOURCE_AUDIT [a-z0-9_= ]+", line):
        return line
    if line in ("bans ok, licenses ok, sources ok", "bans ok", "licenses ok", "sources ok"):
        return line
    if line in (
        "build-macos: make build-macos requires an Apple Silicon Mac",
        "build-macos: Xcode Command Line Tools are required",
        "build-macos: curl and shasum are required",
        "build-macos: system python3 is required for bundle assembly",
        "build-macos: SOURCE_REVISION is required",
        "build-macos: rustup-init checksum mismatch",
    ):
        return line
    match = re.match(r"warning: ([0-9]+) allowed warnings found", line)
    if match:
        return match[0]
    match = re.match(r"error(?:\[(E[0-9]+)\])?:", line)
    if match:
        return f"Compiler/tool error: {match[1] or 'unspecified'} (details omitted)"
    match = re.search(r"((?:app|crates|tools)/[a-zA-Z0-9_./-]+\.rs):([0-9]+):([0-9]+)", line)
    if match:
        return f"Rust diagnostic location: {match[0]}"
    match = re.match(r"(?:Compiling|Checking) ([a-zA-Z0-9_-]+) (v[0-9][a-zA-Z0-9.+-]*)", line)
    if match:
        return match[0]
    match = re.match(r"make(?:\[[0-9]+\])?: \*\*\* \[([a-zA-Z0-9_./: -]+)\] Error ([0-9]+)", line)
    if match:
        return match[0]
    return None


def rust_test_report(lines):
    failed = []
    diagnostics = []
    for line in lines:
        cleaned = safe_line(line)
        if cleaned is None:
            continue
        match = re.fullmatch(r"test ([a-zA-Z0-9_:]+) \.\.\. FAILED", cleaned)
        if match and match[1] not in failed:
            failed.append(match[1])
        diagnostics.append(cleaned)
    return {"failedTests": failed, "diagnostics": diagnostics}


if __name__ == "__main__":
    with Path(sys.argv[1]).open(encoding="utf-8-sig", errors="replace") as source:
        print(json.dumps(rust_test_report(source)))
