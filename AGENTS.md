# Agent instructions for Notrum

## Getting started

1. Read [README.md](README.md) in full.
2. Run `make status` and `make diff` to inspect the checkout and user changes.
   Use `make log` only when history is relevant and the user's task permits it.
3. Work directly on the requested task without separate planning files or
   progress journals.
4. Do not overwrite unrelated user changes or include them in your change set.

## Product boundaries

- Notrum is a local native Rust/Floem application without a WebView, JavaScript,
  network services, or a database as the authoritative store.
- The only authoritative source of workspace notes is UTF-8 Markdown with YAML
  front matter in `<workspace>/notes/`. Preserve unknown fields, files, and
  directories.
- Opening or scanning a workspace must not rewrite notes.
- Saves and metadata operations must preserve atomic/no-overwrite,
  conflict/recovery, and bounded-memory guarantees.
- Protected notes leave YAML and filenames readable while storing the Markdown
  body as an authenticated age envelope. Body plaintext must not reach a
  persistent index, recovery record, cache, diagnostics, or temporary files.
- Project-owned Rust must remain safe-only. Do not add `unsafe`, SQLite,
  WebView/browser runtimes, JavaScript runtimes, or process execution. The only
  network exception is the restricted `ureq` HTTPS client in `notrum-rss`;
  HTTPS article opening is allowed only through that crate's dedicated
  hardened opener.
- `.notrum/` contains settings and potentially unsaved recovery work as well
  as derived caches. Never treat the whole directory as disposable. Preserve
  `.notrum_security/` and `.notrum_backups/`; see [storage documentation](docs/storage.md).

## Development and checks

- The root Makefile is the primary command interface. Rust toolchain commands,
  tests, linters, audits, benchmarks, and Git commands run through the Docker
  Compose `toolchain` service.
- On the host, use `make`, `docker`/`docker compose`, and file editing. Native
  Apple Silicon operations are exposed by `make build`, `make native-smoke`,
  and `make native-external-smoke`. Builds use pinned Rust in ignored
  `.host-build/` and the system Xcode SDK without changing global Rust or shell
  profiles. The external-file smoke uses host Python and an existing bundle.
- Write or update tests with behavior changes. Do not weaken tests to hide a
  defect.
- Use scoped UI styles. Do not apply global theme/style overrides for local
  changes.
- Select exactly one widest applicable final aggregate: `make ui-check` for UI,
  `make check` for the full gate without a native build, or `make` for the full
  gate, native build, and external-file smoke. Use narrow targets only during
  development or failure diagnosis; after fixing a failure, repeat the original
  aggregate once.
- Do not run `make ui-build` separately before a final aggregate that includes it.
- Keep project license metadata and SPDX notices consistent with `GPL-3.0-only`.
  Preserve dependency license notices and do not bypass audits.
- Do not create Git commits unless the user explicitly requests them.

## Finishing a task

1. Run the applicable final check.
2. Review `make diff` and confirm that original user changes are preserved.
3. Report what changed, which checks passed, and any remaining failures or risks.
