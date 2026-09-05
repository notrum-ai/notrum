# Development

[Back to README](../README.md) · [Contributing](../CONTRIBUTING.md)

## Toolchain and commands

Use the root Makefile for project commands. Rust checks, tests, linters, audits,
benchmarks, and Git inspection run in the Docker Compose `toolchain` service.
Docker with Compose and a running engine are required. The Dockerfile pins
Rust 1.88.0 and the audit tools. Named Docker volumes retain build and download
caches between runs.

Native Apple Silicon operations run through `make build`, `make native-smoke`,
and `make native-external-smoke`. They require macOS, Xcode Command Line Tools,
and `python3`. The build installs pinned Rust under `.host-build/`, uses the
system Xcode SDK, and leaves global Rust directories and shell profiles alone.
`native-external-smoke` requires an already built bundle.

Start by inspecting the current checkout:

```sh
make status
make diff
```

`make log` is available when recent history is relevant and the task permits
inspecting it. Do not overwrite unrelated working changes.

| Command | Purpose |
|---|---|
| `make help` | Reminder of aggregate check selection |
| `make image` | Build the pinned Docker toolchain image |
| `make test` / `make test-release` | Workspace Rust tests in debug / release mode |
| `make test-<crate>` | Focused tests for crates with a Makefile target |
| `make lint` / `make fmt-check` | Clippy / formatting check |
| `make fmt` | Apply Rust formatting |
| `make ui-click-<scenario>` | Diagnose a particular UI acceptance scenario |
| `make ui-check` | UI smoke tests and click-driven acceptance scenarios |
| `make audit` | Source boundary, dependency license/source, and RustSec checks |
| `make check` | Full gate, including container builds and UI checks, without a native macOS build |
| `make` | Full gate, native Apple Silicon build, then native external-file smoke |
| `make build` | Build `dist/Notrum.app` on an Apple Silicon Mac |
| `make native-smoke` | Generate demo data, build, and launch the bundle in a temporary workspace |
| `make native-external-smoke` | Test Finder/Launch Services delivery to an existing bundle |
| `make clean` | Remove Docker debug Cargo artifacts |

Choose exactly one final aggregate appropriate to the change: `make ui-check`,
`make check`, or `make`. Do not run each narrower target followed by every
aggregate that includes it on an unchanged diff. If an aggregate fails,
diagnose the failing target, fix the cause, and repeat the original aggregate
once. Do not run `make ui-build` separately before an aggregate that includes it.

`make clean` retains release and macOS cross-target artifacts, Cargo registry
and Git caches, benchmark data, and the native `.host-build/` directory.

## UI acceptance

UI acceptance drives an ordinary Floem window through XTEST in disposable
workspaces under Xvfb. It covers editing, autosave, recovery, conflicts, search,
settings, protected notes, and RSS. Visual checks examine transitions, text
visibility, and hover/tooltip surfaces without stored pixel-exact PNG baselines.

Up to two independent scenarios run in separate containers at once. Standard
and `test-utils` scenarios run in successive groups, with a build before each
group so the binary is not replaced during a test. The timing-sensitive caret
test runs separately before parallel groups. Other aggregate stages remain
sequential. Set `UI_JOBS=1` for sequential diagnosis or `UI_JOBS=2` for the default.

## Dependencies and native file opening

Floem comes from crates.io. Finder file opening uses `floem-winit 0.29.5` from
the public [Notrum fork](https://github.com/notrum-ai/winit/tree/macos/open/files).
The `macos/open/files` branch is based on upstream revision
`69fa86042d44e58c04e0fee71f171470ade45792`. It adds an `application:openURLs:`
handler and an ordered `take_opened_files()` queue.

The root Cargo manifest pins the fork's complete Git revision, and Cargo.lock
records the resolved dependency. Cargo downloads its source into its own cache;
there is no vendored copy in this repository. Updating the patch requires an
explicit revision and lockfile update. The default `make` gate includes native
tests for delivery of multiple files at startup and to a running application.

Keep Cargo.lock committed and retain `publish = false` for workspace packages.
The root workspace defines `GPL-3.0-only`, inherited by all project packages.
The license audit includes these packages even though they are not published
to crates.io. Dependencies retain their own licenses.

## Known dependency warnings

On **2026-09-05**, `make audit` completed successfully but `cargo audit` reported
**14 allowed warnings** for the locked dependency graph. A successful exit is
not a claim that these issues are resolved. Repeated versions count separately.

| Kind | Crates and locked versions | Advisory |
|---|---|---|
| Unmaintained | `bitmaps 2.1.0` | [RUSTSEC-2026-0247](https://rustsec.org/advisories/RUSTSEC-2026-0247) |
| Unmaintained | `im 15.1.0` | [RUSTSEC-2026-0248](https://rustsec.org/advisories/RUSTSEC-2026-0248) |
| Unmaintained | `im-rc 15.1.0` | [RUSTSEC-2026-0250](https://rustsec.org/advisories/RUSTSEC-2026-0250) |
| Unmaintained | `paste 1.0.15` | [RUSTSEC-2024-0436](https://rustsec.org/advisories/RUSTSEC-2024-0436) |
| Unmaintained | `rustybuzz 0.14.1`, `0.18.0` | [RUSTSEC-2026-0206](https://rustsec.org/advisories/RUSTSEC-2026-0206) |
| Unmaintained | `sized-chunks 0.6.5` | [RUSTSEC-2026-0251](https://rustsec.org/advisories/RUSTSEC-2026-0251) |
| Unmaintained | `ttf-parser 0.20.0`, `0.21.1`, `0.24.1`, `0.25.1` | [RUSTSEC-2026-0192](https://rustsec.org/advisories/RUSTSEC-2026-0192) |
| Unsound | `im 15.1.0`: aliasing violation in `OrdSet` insertion | [RUSTSEC-2023-0126](https://rustsec.org/advisories/RUSTSEC-2023-0126) |
| Unsound | `lru 0.16.4`: potential use-after-free in `LruCache::pop()` | [RUSTSEC-2026-0253](https://rustsec.org/advisories/RUSTSEC-2026-0253) |
| Unsound | `sized-chunks 0.6.5`: panic-safety issues, use-after-free / double-free | [RUSTSEC-2026-0255](https://rustsec.org/advisories/RUSTSEC-2026-0255) |

Dependency upgrades and an assessment of affected call paths are separate
follow-up work. Do not suppress these warnings to make a release appear ready.

## Benchmarks and generated assets

`make benchmark` compares Ropey and Lapce buffers; `make benchmark-editor`,
`make benchmark-viewport`, and `make benchmark-search` exercise the respective
project layers. These targets generate 10 MB, 100 MB, and 1 GB datasets in a
Docker volume and can take substantial time and disk space. They are separate
from the final quality gate.

`make demo-data` recreates the generated notes in `examples/demo-workspace`.
The ignored demo folder is for disposable data, not personal notes.
The demo generator, probe tools, UI test helpers, Cargo.lock, and both icon
assets are maintained project inputs and should stay in the repository.

The vector source is `app/notrum/assets/notrum-app-icon.svg`; the bundle uses
`Notrum.icns` from the same directory. To regenerate the icon after editing it:

```sh
docker compose run --rm toolchain python3 -B tools/generate_app_icon.py \
  app/notrum/assets/notrum-app-icon.svg app/notrum/assets/Notrum.icns
```

## Packaging and release limitations

You can package an existing arm64 binary from a controlled macOS runner inside
Docker. Set `SOURCE_REVISION` to the actual source revision of that binary
(the value below is only a placeholder):

```sh
make package-macos \
  MACOS_BINARY=/workspace/artifacts/notrum-app \
  SOURCE_REVISION=0123456789abcdef
```

The packager checks for a Mach-O arm64 slice, refuses to overwrite an unknown
output, and records SHA-256, version, and source revision in
`Contents/Resources/release.json`.

Local bundles are not Developer ID signed or notarized. Before distributing a
binary release, complete native macOS functional, APFS filename, Unicode/IME,
accessibility, large-file UI, and protected-note UX checks; address the open
dependency warnings; prepare the license notices and corresponding source for
the distribution; and complete signing, notarization, and a Gatekeeper smoke.
Docker checks and a native launch smoke alone do not establish release readiness.
