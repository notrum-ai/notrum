# Agent instructions for Notrum

## Getting started

1. Read [README.md](README.md) in full.
2. Run `git status --short --branch` and `git diff` on the host to inspect the
   checkout and user changes. Use `git log` only when history is relevant and
   the user's task permits it.
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
  network exceptions are the restricted `ureq` HTTP/HTTPS client in `notrum-rss`
  (any host or port) and HTTPS client in `notrum-ai` (fixed OpenAI/Anthropic
  model catalog endpoints only); HTTP/HTTPS article opening is allowed only
  through the RSS crate's dedicated hardened opener. The test-only RSS HTTP
  integration target may use a loopback server to exercise the client.
- AI settings are global. API keys belong only in the OS credential store via
  `notrum-platform`; config files contain opaque references and model aliases.
  Never send notes while checking a key or listing models. Connecting creates
  the provider-specific `default` model alias.
  It can be edited but not deleted or renamed; missing aliases resolve through
  `default`. Existing unavailable selections must fail explicitly.
- `.notrum/` contains settings and potentially unsaved recovery work as well
  as derived caches. Never treat the whole directory as disposable. Preserve
  `.notrum_security/` and `.notrum_backups/`; see [storage documentation](docs/storage.md).

## Development and checks

- The root Makefile is the primary command interface. Rust toolchain commands,
  tests, linters, audits, and benchmarks run through the Docker Compose
  `toolchain` service. Run all Git commands directly on the host, never in
  Docker or through Makefile targets that run Git in Docker.
- On the host, use `git`, `make`, `docker`/`docker compose`, and file editing. Native
  Apple Silicon operations are exposed by `make build`, `make native-smoke`,
  and `make native-external-smoke`. Builds use pinned Rust in ignored
  `.host-build/` and the system Xcode SDK without changing global Rust or shell
  profiles. The external-file smoke uses host Python and an existing bundle.
- `make publish` runs its Python orchestrator and locally authenticated Codex
  on the host; it calls GitHub's REST API directly and runs Rust in Docker.
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
2. Review `git diff` on the host and confirm that original user changes are preserved.
3. Report what changed, which checks passed, and any remaining failures or risks.
