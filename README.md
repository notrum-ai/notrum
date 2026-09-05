# Notrum

Notrum is a local, native Markdown editor and RSS/Atom reader built with
Rust and Floem. Notes live in ordinary files in a Notable-style workspace.
There is no WebView, JavaScript runtime, cloud account, or database holding
the authoritative copy of your notes.

Use it to keep project notes and reading lists in folders you control, edit
existing text files in place, and read subscribed feeds in the same application.
Your notes remain Markdown files that you can back up or open in another editor.

![Notrum showing the generated demo workspace, with tagged notes in the sidebar and Markdown in the editor](docs/images/demo-workspace.png)

*Demo workspace in the native interface, captured in the Linux test environment.*

## Features

- Markdown editing with autosave, crash recovery, and external change detection.
- Categories from YAML tags, favorites, pinning, manual ordering, and sorting.
- Local search and soft deletion of notes.
- Password-protected note bodies using authenticated age encryption.
- External `.md`, `.markdown`, and `.txt` files opened in place, including from Finder.
- HTTPS RSS and Atom subscriptions with a native reader and `J`/`K` navigation.

## Project status

Notrum is in early development. The interface defaults to English and offers
17 language variants in Settings → General → Language.
Native builds target **Apple Silicon Macs**. Linux release binaries can be
built through Docker for the container's architecture. Portable Windows x64
builds target Windows 10/11 on local NTFS. Cross-compilation and actual Windows
acceptance are separate checks; see the [Windows guide](docs/windows.md).

There is no signed, notarized public release. Local builds are unsigned.
Known dependency warnings and the remaining work before distributing binaries
are documented in the [development guide](docs/development.md).

Your workspace's `notes/` directory holds the authoritative note files.
Opening a workspace does not migrate or rewrite existing notes. Encryption
protects the note body; filenames and YAML metadata, including titles and tags,
remain readable. Read the [storage and security guide](docs/storage.md) before
managing backups or removing local state. Recovery files can contain unsaved
work and must not be treated as disposable cache.

## Build and run

### macOS

Requirements: an Apple Silicon Mac, Docker with Docker Compose and a running
Docker engine, Xcode Command Line Tools, and `python3`. A preinstalled Rust
toolchain is not required. Clone the repository, then build the application:

```sh
git clone https://github.com/notrum-ai/notrum.git
cd notrum
make build-macos
open dist/Notrum.app
```

On first launch, choose a workspace or create one through the application.
To open an existing workspace explicitly:

```sh
open dist/Notrum.app --args /absolute/path/to/workspace
```

`make build-macos` downloads pinned Rust 1.88.0 into the ignored `.host-build/`
directory and builds with the system macOS SDK. It does not change your global
Rust installation or shell profile. It builds the application in release mode
and packages it as `dist/Notrum.app`. `make build` remains an alias for this
command.

To try generated demo notes:

```sh
make demo-data
open dist/Notrum.app --args "$PWD/examples/demo-workspace"
```

The demo command resets the generated demo notes. Use a separate workspace
for your own writing.

### Linux

With Docker Compose and a running Docker engine, build an optimized Linux
executable using the pinned toolchain (no host Rust installation required):

```sh
make build-linux
```

The stripped release binary is written to `dist/linux/<architecture>/notrum`,
where the architecture matches the Docker container: `aarch64` on Apple
Silicon by default, or `x86_64` on an Intel/AMD host. Build on the matching
architecture for the destination Linux machine. The command also works from
macOS, but the resulting executable runs only on Linux.

On Linux, launch it with an optional workspace path:

```sh
./dist/linux/$(uname -m)/notrum /absolute/path/to/workspace
```

This is a dynamically linked binary built on Debian 12 (glibc 2.36), not a
self-contained application bundle. A compatible Linux desktop with X11 or
Wayland, the corresponding system libraries, fonts, and an XDG desktop portal
for file dialogs is required. See the [development guide](docs/development.md)
for build verification and release limitations.

### Windows 10/11 x64

Build on macOS or Linux with Docker Compose:

```sh
make build-windows
make test-windows-build
```

Copy `dist/windows/x86_64/` to a Windows computer and run `Notrum.exe`.
Any required non-system runtime DLLs are included beside the executable.
The application is portable, unsigned, and runs without a console window.
The existing `make build` command still builds the macOS application.

Settings use `%USERPROFILE%\.notrum.cfg`, falling back to
`%HOMEDRIVE%%HOMEPATH%\.notrum.cfg`. English remains the default language;
all 17 language variants and saved workspace selection are available.

Run the supplied test kit and complete the native UI checklist before treating
that Windows build as validated. Instructions, supported filesystem boundaries,
and known validation limits are in [docs/windows.md](docs/windows.md).

## Documentation and contributions

- [User guide](docs/user-guide.md): workspaces, notes, external files, and feeds.
- [Storage and security](docs/storage.md): file layout, recovery, encryption, and backups.
- [Development guide](docs/development.md): toolchain, checks, benchmarks, and packaging.
- [Contributing](CONTRIBUTING.md): reporting bugs and submitting changes.
- [Security policy](SECURITY.md): reporting vulnerabilities privately.

## License

Copyright 2026 Evgeniy Udodov.

Notrum's own code, scripts, documentation, and application artwork are licensed
under the **GNU General Public License version 3 only** (`GPL-3.0-only`).
See [LICENSE](LICENSE) for the full terms. Notrum comes without warranty.
Third-party dependencies retain their respective licenses; the project's
license does not replace them.
