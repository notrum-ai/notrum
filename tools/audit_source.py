#!/usr/bin/env python3
# Copyright 2026 Evgeniy Udodov
# SPDX-License-Identifier: GPL-3.0-only
"""Deterministic audit for project-owned Rust and direct manifests."""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
OWNED_ROOTS = (ROOT / "app", ROOT / "crates", ROOT / "tools")
FORBIDDEN_RUST = {
    "runtime network API": re.compile(
        r"\b(?:std::net|TcpStream|TcpListener|UdpSocket|reqwest|surf|hyper)::?"
    ),
    "process spawning": re.compile(r"\b(?:std::process::Command|Command::new)\b"),
    "database API": re.compile(r"\b(?:rusqlite|sqlx|sqlite|diesel)::?"),
    "browser or JavaScript runtime": re.compile(
        r"\b(?:webview|webkit|javascript|quick_js|deno_core|boa_engine|v8)::?",
        re.IGNORECASE,
    ),
}
FORBIDDEN_DEPENDENCIES = re.compile(
    r"^\s*(?:boa_engine|curl|deno_core|diesel|hyper|quick-js|reqwest|rusqlite|sqlx|sqlite|surf|v8|webkit2gtk|wry)\s*=",
    re.MULTILINE,
)


def fail(message: str) -> None:
    print(f"SOURCE_AUDIT_ERROR {message}", file=sys.stderr)


def main() -> int:
    rust_files = sorted(
        path
        for root in OWNED_ROOTS
        for path in root.rglob("*.rs")
        if path.is_file()
    )
    manifests = sorted(
        path
        for root in OWNED_ROOTS
        for path in root.rglob("Cargo.toml")
        if path.is_file()
    )
    errors = 0
    for path in rust_files:
        text = path.read_text(encoding="utf-8")
        relative = path.relative_to(ROOT)
        if "#![forbid(unsafe_code)]" not in "\n".join(text.splitlines()[:5]):
            fail(f"{relative}: missing crate-level #![forbid(unsafe_code)]")
            errors += 1
        for line_number, line in enumerate(text.splitlines(), start=1):
            if re.search(r"\bunsafe\b", line):
                fail(f"{relative}:{line_number}: project-owned unsafe token")
                errors += 1
        for label, pattern in FORBIDDEN_RUST.items():
            match = pattern.search(text)
            if match:
                line_number = text.count("\n", 0, match.start()) + 1
                fail(f"{relative}:{line_number}: forbidden {label}")
                errors += 1
        if re.search(r"\bureq::", text) and relative.parts[:2] not in (
            ("crates", "notrum-rss"), ("crates", "notrum-ai")
        ):
            fail(f"{relative}: ureq is restricted to RSS and AI transport crates")
            errors += 1
        if re.search(r"\bwebbrowser::", text) and relative.parts[:2] != ("crates", "notrum-rss"):
            fail(f"{relative}: browser handoff is restricted to crates/notrum-rss")
            errors += 1

    for path in manifests:
        text = path.read_text(encoding="utf-8")
        relative = path.relative_to(ROOT)
        match = FORBIDDEN_DEPENDENCIES.search(text)
        if match:
            line_number = text.count("\n", 0, match.start()) + 1
            fail(f"{relative}:{line_number}: forbidden direct dependency")
            errors += 1
        if re.search(r"^\s*ureq\s*=", text, re.MULTILINE) and relative not in (
            Path("crates/notrum-rss/Cargo.toml"), Path("crates/notrum-ai/Cargo.toml")
        ):
            fail(f"{relative}: HTTP dependency crosses the RSS/AI transport boundary")
            errors += 1
        if re.search(r"^\s*webbrowser\s*=", text, re.MULTILINE) and relative != Path("crates/notrum-rss/Cargo.toml"):
            fail(f"{relative}: browser handoff crosses the RSS boundary")
            errors += 1
        if re.search(r"^\s*keyring\s*=", text, re.MULTILINE) and relative != Path("crates/notrum-platform/Cargo.toml"):
            fail(f"{relative}: credential dependency crosses the platform boundary")
            errors += 1
        if relative.parts[0] == "crates" and re.search(r"^\s*floem\s*=", text, re.MULTILINE):
            fail(f"{relative}: UI dependency crosses a core crate boundary")
            errors += 1

    if errors:
        return 1
    print(
        "SOURCE_AUDIT "
        f"rust_files={len(rust_files)} manifests={len(manifests)} "
        "project_unsafe=0 rss_https_boundary=1 ai_https_boundary=1 process_spawn=0 database=0 web_js=0"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
