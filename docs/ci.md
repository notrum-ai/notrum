<!-- Copyright 2026 Evgeniy Udodov -->
<!-- SPDX-License-Identifier: GPL-3.0-only -->
# GitHub CI

[Development commands](development.md) · [Windows acceptance](windows.md)

`.github/workflows/ci.yml` runs for pull requests targeting `master`, pushes to
`master`, pushes to the moving `latest` tag, and manual dispatch. It does not change branch protection or PR merge
rules, publish Releases, or require a write-enabled repository token.
Every action is pinned to a complete commit SHA, checkout credentials are not
persisted, and repository contents permissions stay at `contents: read`.
Only the Linux job additionally has `id-token: write` for Codecov OIDC upload
authentication. A newer run cancels the older run for the same PR or branch.

| Job | Runner | Command and scope | Timeout |
| --- | --- | --- | --- |
| Linux | `ubuntu-24.04`, x64 | Build the Compose toolchain; `make COVERAGE=1 ci-linux` executes `make check-linux` (tests with coverage, UI, audits and Linux/macOS checks) and packages Linux | 120 min |
| Windows build | `ubuntu-24.04`, x64 | `make ci-windows-build` cross-compiles the Windows application and test kit, then packages both | 90 min |
| macOS | `macos-15`, Apple Silicon | `make NATIVE=1 ci-macos` builds with pinned Rust and executes the native launch and Finder smoke checks | 90 min |
| Windows | `windows-2025`, x64 | Verify and unpack the Windows build job's test package from this run; execute its existing PowerShell test runner and native smoke checks | 30 min |

Linux, macOS and the Windows cross-build start independently. Native Windows tests
wait only for the Windows build job; Linux UI failures do not block them.
The local `make check` still includes the full gate. Its Windows build targets
are grouped under `check-windows-build`, while `check-linux` runs the remaining
checks. CI runs both groups on separate runners without duplicating the Windows
compilation. `ci-package-linux` packages Linux only; `ci-package-windows` packages
the Windows application and test kit with the same source SHA.
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

## Code coverage

The Linux job measures Rust **line coverage** with pinned `cargo-llvm-cov 0.6.21`
and the LLVM tools supplied with Rust 1.88.0. `COVERAGE=1` replaces the usual
debug workspace test run with its instrumented equivalent, keeping
`--workspace --all-features`. Stable coverage tooling skips doctests, so those
still run separately with `cargo test --doc`. Release tests, desktop UI checks,
audits, and the other platform jobs retain their existing commands.

The report covers Rust workspace sources exercised by Linux unit and integration
tests, including the application crate. It does not measure Python scripts,
external UI acceptance scenarios, doctests, or macOS/Windows-specific code.
Dependencies and standalone test/example/benchmark source directories use
cargo-llvm-cov's default exclusions. Inline test modules and the workspace's
Rust probe tools are not specially excluded. No coverage-specific code paths
are enabled, and no minimum percentage is enforced before a baseline exists.

This avoids another CI runner and a duplicate debug test run. Instrumentation
still needs its own compilation; its artifacts stay in cargo-llvm-cov's separate
target directory inside the disposable Cargo target volume. Downloaded tools
are cached in the Docker image. Coverage does not instrument release packages.

To reproduce locally after rebuilding the toolchain image:

```sh
make image
make coverage
# Or select this as the final full gate:
make COVERAGE=1 check
```

`make coverage` writes `.ci/coverage/lcov.info` only after tests pass and a
nonempty report is generated. A new run removes the old report first, so failed
tests cannot publish a stale success. This path is already ignored by Git.
LCOV contains source paths and line execution counts, without source text or
per-function records. Raw profiles, test workspaces, and test logs are not
uploaded. The existing CI diagnostic filter still wraps the test command.

GitHub Actions retains the `coverage-linux` LCOV artifact for one day, including
when a later check fails. Codecov receives only this explicit file; file search,
source-based file fixes, additional collection plugins, and telemetry are
disabled. The README badges follow the moving `latest` release tag. Publishing
that tag starts an additional full CI run on the release commit. Only runs on
`refs/tags/latest` set Codecov's `override_branch` to `latest`; other runs retain
automatic branch detection. The coverage badge appears after Codecov processes
the first upload for `latest`. Its color reflects the measured result. Uploads explicitly use
`github.sha`, matching the tested checkout (including the merge SHA for a PR).

### Codecov setup

Enable `notrum-ai/notrum` in [Codecov](https://app.codecov.io/gh/notrum-ai/notrum)
with the repository owner's GitHub account if it is not connected yet. The
workflow uses [GitHub OIDC](https://github.com/codecov/codecov-action#using-oidc),
so no `CODECOV_TOKEN` secret is required. Repository/organization policy must
allow the pinned Codecov action and the Linux job's `id-token: write` permission.
No repository-content write permission is granted.

Fork and Dependabot pull requests still measure coverage and retain the artifact,
but skip the OIDC upload because their token permissions may be restricted.
Pushes, manual runs, and other same-repository PRs upload automatically. Upload
errors fail the Linux job visibly; the artifact remains available even if Codecov
is unavailable or initial account setup is incomplete. `codecov.yml` disables
PR comments, inline annotations, and percentage-based commit statuses.

## Caches and runner storage

Both Ubuntu jobs set `COMPOSE_FILE=compose.yaml:compose.ci.yaml`. Buildx Bake
reads those same Compose files, loads `notrum-toolchain:ci`, and restores/exports
Docker layers through the GitHub Actions cache backend. No image is pushed to a
registry. The local Compose configuration and its named volumes are unchanged.

Actions caches only downloaded Cargo registry and Git sources on Linux and
macOS. Keys include OS, architecture and lockfile/toolchain inputs. Cargo target,
incremental artifacts, native binaries, test workspaces, settings and credentials
are never cached across runs. Linux target remains a named volume on the
disposable runner; macOS target remains on that runner's local filesystem.
Before cache archiving, Linux returns ownership of `.ci/cargo` to the runner.
Some published crates contain owner-only source files; extraction by Docker's
root user otherwise makes those files unreadable to the host cache action.
The Windows build job performs the same ownership cleanup and uses a separate
Cargo cache key, with Linux sources as a fallback. The jobs share the Docker
toolchain layer cache but have separate working directories and target volumes.

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
- `reports-linux`, `reports-macos`, `reports-windows-build`, `reports-windows`: status and cleaned diagnostics,
  uploaded also when checks fail or a run is cancelled after checkout.
- `coverage-linux`: the completed Rust LCOV report, retained also if later checks fail.

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

Native test kits also emit fixed-vocabulary `NATIVE_IO`, `NATIVE_SAVE`,
`NATIVE_OPERATION`, `NATIVE_CLEANUP`,
`NATIVE_TEMP`, `NATIVE_RESULT`, `NATIVE_ASSERT` and `NATIVE_PATH` records.
I/O failures distinguish lock creation, validation, opening and acquisition;
metadata opening, inspection, permission capture, hashing and cursor restoration;
atomic replacement; and temporary cleanup. Records contain only known operation,
stage and error-kind labels, numeric OS codes (zero when unavailable), cleanup
outcomes, temporary counts and boolean path-comparison results. They never contain
error messages, path strings or file contents. Malformed records are rejected
before the legacy diagnostic extractors, and filtering the records again is safe.
The same records survive in `checks.log` and `windows-results.json`.

Rust captures these diagnostics with each test, so successful tests remain quiet
and failed tests retain their diagnostic context. The platform instrumentation is
enabled by `test-utils` (included in the native kit's `--all-features` build);
ordinary release builds do not emit it. A failing first-use lock test identifies
creation/acquisition failures separately from save conflicts. A Windows rerun is
still required to establish which remaining native failures are resolved.

UI acceptance failures additionally emit `UI_ACCEPTANCE_DIAGNOSTIC` lines to
the Actions console and `checks.log`. They retain the scenario, a fixed stage,
an allowlisted exception type (or `Exception` for an unknown type), and known
test-tool source paths with line numbers. `password_change` distinguishes setup,
protection, settings validation, clipboard checks, rotation, backup verification,
restart, old-password rejection and new-password unlock. These lines are flushed
before screenshots or cleanup; failures in those operations are reported separately
and do not replace the original failure. No exception messages, source-line text,
command arguments or local variables are included, including in local UI failure
output. Existing `UI_ACCEPTANCE_PASS`/`UI_ACCEPTANCE_FAIL` markers and exit codes
remain unchanged. This diagnostic addition does not establish the cause of an
earlier failure whose report omitted those details.

## Verify the first GitHub runs

After pushing the workflow commit, open **Actions → CI**. A first successful run
must show cache misses/build steps, all four jobs passing, and the nine named
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
