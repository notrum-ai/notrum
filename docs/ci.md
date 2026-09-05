<!-- Copyright 2026 Evgeniy Udodov -->
<!-- SPDX-License-Identifier: GPL-3.0-only -->
# GitHub CI

[Development commands](development.md) · [Windows acceptance](windows.md)

`.github/workflows/ci.yml` runs for pull requests targeting `master`, pushes to
`master`, and manual dispatch. It does not change branch protection or PR merge
rules, publish Releases, or require a write-enabled repository token.
Every action is pinned to a complete commit SHA, checkout credentials are not
persisted, and the workflow has only `contents: read`. A newer run cancels the
older run for the same PR or branch.

| Job | Runner | Command and scope | Timeout |
| --- | --- | --- | --- |
| Linux | `ubuntu-24.04`, x64 | Build the Compose toolchain; `make ci-linux` executes the complete `make check` and packages Linux/Windows binaries | 120 min |
| macOS | `macos-15`, Apple Silicon | `make NATIVE=1 ci-macos` builds with pinned Rust and executes the native launch and Finder smoke checks | 90 min |
| Windows | `windows-2025`, x64 | Verify and unpack the Linux job's test package from this run; execute its existing PowerShell test runner and native smoke checks | 30 min |

Linux and macOS start independently. Windows waits for a successful Linux job.
Tests remain in Makefile and the shared test scripts, including desktop file
opening, multiple processes, localization, crash reporting, clipboard, recovery
and password-change acceptance. YAML contains orchestration rather than copies
of those tests. Windows uses the same compiled test list and native runner as
local Windows acceptance; manual GPU, IME and accessibility checks remain manual.

## Local validation and native commands

```sh
make ci-validate
make check
make diff
```

`ci-validate` runs actionlint 1.7.12 from its digest-pinned Docker image and checks
the merged `compose.yaml` + `compose.ci.yaml`, including cache mounts and profile
settings. It is included in `make check`. Artifact privacy, permissions, transfer
integrity and source-revision validation have tests in `tools/test_ci.py`.

On an Apple Silicon Mac with Xcode Command Line Tools and Python, Docker is not
needed for the following commands. Supply the full SHA of the actual checkout:

```sh
make NATIVE=1 SOURCE_REVISION=<40-character-HEAD-SHA> native-check
make NATIVE=1 SOURCE_REVISION=<40-character-HEAD-SHA> ci-macos
```

`native-check` reuses `native-smoke` and `native-external-smoke`; the former reuses
`build-macos` and `demo-data`. `ci-macos` additionally writes sanitized reports
and a distributable archive. Empty, shortened, symbolic or mismatching revision
values fail before native compilation. In Actions, `SOURCE_REVISION` is
`github.sha`, checked against `git rev-parse HEAD`; for a pull request this is the
actual tested merge checkout rather than the PR head branch SHA.

Without `NATIVE=1`, existing commands retain their Docker-backed behavior and
local builds continue to record the automatic revision, including `-dirty`.
Native Rust and downloads remain inside ignored `.host-build/` with the existing
pinned Rust/rustup and system Xcode SDK. Global Rust and shell profiles are unchanged.
The macOS CI wrapper explicitly starts native Make through `arch -arm64`, so an
Intel Python installation running under Rosetta cannot change the build architecture.

## Caches and runner storage

The Linux workflow sets `COMPOSE_FILE=compose.yaml:compose.ci.yaml`. Buildx Bake
reads those same Compose files, loads `notrum-toolchain:ci`, and restores/exports
Docker layers through the GitHub Actions cache backend. No image is pushed to a
registry. The local Compose configuration and its named volumes are unchanged.

Actions caches only downloaded Cargo registry and Git sources on Linux and
macOS. Keys include OS, architecture and lockfile/toolchain inputs. Cargo target,
incremental artifacts, native binaries, test workspaces, settings and credentials
are never cached across runs. Linux target remains a named volume on the
disposable runner; macOS target remains on that runner's local filesystem.

Only CI disables incremental compilation and debug symbols for dev/test via
environment variables. Debug assertions, overflow checks, test features, release
tests, linters and audits retain their normal behavior. No Cargo profile in the
repository is weakened. Linux removes only the unused Android and .NET SDKs on
the disposable GitHub-hosted runner, reporting free space before cleanup, before
the gate and at job completion (including failure).

## Artifacts and diagnostics

Artifacts are retained for **one day**, the minimum supported retention period:

- `notrum-linux` and `notrum-macos`: `.tar.gz` archives preserving executable modes.
- `notrum-windows`: portable application ZIP, without the test executables.
- `windows-test-package`: separate ZIP containing that Windows application,
  runtime DLLs and the compiled test kit for the dependent job in the same run.
- `reports-linux`, `reports-macos`, `reports-windows`: status and cleaned diagnostics,
  uploaded also when checks fail or a run is cancelled after checkout.

Build archives include the project license, existing runtime notices where
applicable, `SOURCE_REVISION.txt` and a `build.json` file with per-file SHA-256
checksums. The Windows job rejects a different revision, changed bytes, unexpected
archive members, links and escaping paths before running any supplied executable.

Reports record the source SHA, platform, exit status, known check names and Rust
diagnostic source locations. Python failures retain test names, tool source
locations and exception types. The diagnostic filter excludes arbitrary test output,
panic payloads, thread names, editor text and temporary paths. Neither whole
workspaces nor screenshots, raw application logs, recovery files or caches are
uploaded. Native Windows CI reports omit temporary workspace/log paths and
exception payloads; the default local PowerShell runner still retains its usual
diagnostic workspace. The Windows kit runs every Rust test executable before
reporting failure; `windows-results.json` records failed test names, sanitized
diagnostics and the current test phase. It uses the same diagnostic filter as
the CI console, and does not upload raw test logs. CI logs are deliberately
reduced: reproduce a failing named check locally to inspect unrestricted fixture
diagnostics.

## Verify the first GitHub runs

After pushing the workflow commit, open **Actions → CI**. A first successful run
must show cache misses/build steps, all three jobs passing, and the seven named
artifacts above. Download the archives before they expire and inspect their
source SHA, license and checksums; the Unix executable bits are inside the tar
archives. Use **Run workflow** on the same branch to create a second run, verify
Cargo cache hits and cached Docker layers, then verify all jobs and artifacts
again. A cache hit alone is not proof that checks executed.

Record links and conclusions for both runs when they exist. Local checks and a
Windows cross-build do not establish a passing GitHub-hosted Windows run.
If repository or organization policy disables Actions or blocks the pinned
actions, the repository owner must enable the CI workflow/allow those actions.
No change to branch protection or workflow write permissions is needed.
