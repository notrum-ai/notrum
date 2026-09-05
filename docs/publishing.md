# Publishing a release

Run `make publish` on an Apple Silicon Mac from the repository's default branch
(currently `master`). The command publishes to the GitHub repository configured
as `origin`. Commit your source changes first: both the working tree and index
must be clean, including untracked files that are not ignored. Local commits
ahead of GitHub are included; a behind or diverged branch is rejected.

## Requirements

- The normal macOS build requirements: Docker Compose with a running engine,
  Xcode Command Line Tools, and host Python 3.9 or newer.
- A working local `codex` CLI in `PATH`, already signed in and able to use
  `gpt-5.6-luna`. The command explicitly selects `medium` reasoning effort.
- GitHub CLI (`gh`) in `PATH`, authenticated with `gh auth login`, with permission
  to push commits/tags and create Releases in the repository. Repository rules
  still apply; publication does not bypass branch protection.

Python orchestrates publication on the host and invokes the host's Codex and
GitHub CLIs. Git and Rust commands continue to run in the Docker toolchain.
GitHub authentication is passed to individual Docker Git commands in environment
variables; tokens are not written to the checkout or Git configuration. The
existing SSH or HTTPS `origin` URL is preserved.

## What the command does

1. Validates prerequisites, the checkout and the remote default branch.
2. Reads `app/notrum/Cargo.toml` and increases only its patch component:
   `0.1.0` becomes `0.1.1`, and `0.1.9` becomes `0.1.10`. Updates only the
   corresponding application entry in `Cargo.lock`; dependency versions stay
   unchanged. macOS and Windows package versions come from the app manifest.
3. Finds the last first-parent commit that changed the application's version
   value. For the first release, this is the initial `0.1.0` commit. Feeds the
   subsequent committed history, patches and net diff to local `codex exec` in
   bounded portions, then combines the summaries. No notes workspace or ignored
   files are included. Codex writes English `Improvements` and `Bug fixes`
   sections; an empty category is `None.`. Its output is generated text and can
   still contain interpretation errors.
4. Creates `Release vX.Y.Z` with only the two version files, using
   `Evgeniy Udodov <1926460+flrnull@users.noreply.github.com>` as author and
   committer. The description is saved locally and used as the annotated tag's
   message; it is not added as a tracked changelog file.
5. Runs the full `make` aggregate on the release commit. Packages the resulting
   macOS arm64 application, Windows x64 application and Linux executable for the
   Docker architecture (normally aarch64 on Apple Silicon). Linux x86_64 is not
   implicitly cross-built. Builds remain unsigned and macOS is not notarized;
   the existing platform validation limitations still apply.
6. Checks archive manifests, source SHA and file hashes. Prepares three
   versioned archives and `SHA256SUMS`, retaining licenses and platform metadata.
   The Windows test kit is not a release asset.
7. Pushes the default branch and annotated `vX.Y.Z` tag atomically, without a
   force push. Creates a draft Release, uploads the archives and checksums,
   verifies their downloaded bytes, then publishes it as Latest. The command
   prints the Release URL. No confirmation prompt is required.

No new commits since the previous version change means no new release. A Codex
failure, unsupported version or failed build stops the process; the command does
not substitute another model or silently skip checks.

## Retrying

`make publish` takes an exclusive local lock and saves its state in
`.host-build/publish/state.json`. After a failure, rerun the same command: it
continues the pending version instead of increasing it again. A failed build
leaves the local version commit in place, without pushing it. Until all archives
are prepared, a retry runs the full aggregate again. Once prepared, archives are
reused only if their saved hashes still match.

Keep the pending checkout and `.host-build/publish/` intact until publication
finishes. Independent changes to HEAD, the version files, prepared assets,
existing release notes or remote tags cause an error rather than being
overwritten. Restore the pending checkout before retrying; do not discard the
saved state just to bypass an error. Existing uploaded assets are verified and
never replaced automatically. A successfully published release is not bumped
again until additional source commits exist.

Publication tests are part of `make check` through `make test-publish`. They use
temporary repositories and mocked external services; they never create a real
GitHub Release or invoke a real Codex model.
