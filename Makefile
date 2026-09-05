# Copyright 2026 Evgeniy Udodov
# SPDX-License-Identifier: GPL-3.0-only

.DEFAULT_GOAL := all
# Only the acceptance worker sub-make runs independent scenarios in parallel.
ifeq ($(UI_ACCEPTANCE_PARALLEL),)
.NOTPARALLEL:
endif

COMPOSE := docker compose
RUN := $(COMPOSE) run --rm toolchain
GIT := $(RUN) git -c safe.directory=/workspace
MACOS_BINARY ?=
MACOS_OUTPUT ?= /workspace/dist/Notrum.app
SOURCE_REVISION ?=
DEMO_WORKSPACE ?= /workspace/examples/demo-workspace
UI_JOBS ?= 2

UI_ACCEPTANCE_STANDARD := ui-click-localization ui-click-rss-cards ui-click-rss-keyboard ui-click-workspace ui-click-compatibility ui-click-categories ui-click-interaction ui-click-lifecycle ui-click-tags ui-click-editor ui-click-context-menu ui-click-selection ui-click-persistence ui-click-recovery ui-click-conflict ui-click-search ui-click-find ui-click-resize ui-click-visual
UI_ACCEPTANCE_SECURE := ui-click-password-change ui-click-secure ui-click-secure-recovery ui-click-secure-conflict ui-click-secure-integrity

.PHONY: all help check clean build build-macos build-linux build-linux-smoke build-container native-smoke native-external-smoke demo-data test-demo-data check-macos test test-release lint fmt fmt-check lock tree audit audit-source audit-dependencies audit-vulnerabilities \
	diff-check status log diff-stat diff image benchmark-generate \
	benchmark-ropey benchmark-lapce benchmark benchmark-editor test-editor \
	benchmark-viewport benchmark-search test-frontmatter test-storage test-core test-recovery test-search test-secure ui-check ui-smoke ui-autosave-smoke ui-recovery-smoke ui-conflict-smoke \
	ui-build ui-build-test-utils ui-operations-smoke ui-click-creation ui-click-workspace ui-click-compatibility ui-click-categories ui-click-interaction ui-click-lifecycle ui-click-tags ui-click-caret ui-click-editor ui-click-context-menu ui-click-selection ui-click-persistence ui-click-recovery ui-click-conflict ui-click-search ui-click-find ui-click-resize ui-click-visual ui-click-password ui-click-password-change ui-click-secure ui-click-secure-recovery ui-click-secure-conflict ui-click-secure-integrity ui-acceptance package-macos package-macos-smoke

all: check build native-external-smoke

help:
	$(RUN) sh -c 'printf "%s\n" \
		"Итоговая проверка: выберите ровно один самый широкий gate." \
		"  make ui-check  — все UI smokes и XTEST scenarios" \
		"  make check     — все проверки, включая ui-check, без native build" \
		"  make           — make check, затем native build" \
		"  make clean     — удалить Docker debug-артефакты Cargo" \
		"  make build-macos — macOS release в dist/Notrum.app (make build — алиас)" \
		"  make build-linux — Linux release в dist/linux/<архитектура>/notrum через Docker" \
		"  UI_JOBS=2      — число параллельных UI acceptance scenarios (1 для диагностики)" \
		"Диагностика после падения aggregate:" \
		"  make ui-click-<scenario> | make ui-acceptance | make test-<crate>" \
		"Не запускайте target, а затем включающий его aggregate на неизменном diff."'

check: fmt-check lint test test-release test-demo-data check-macos package-macos-smoke build-linux-smoke ui-check audit diff-check

clean:
	$(RUN) rm -rf -- /var/cache/notrum/target/debug

build: build-macos

build-macos:
	@revision="$$(docker compose run --rm toolchain sh -c 'revision=$$(git -c safe.directory=/workspace rev-parse HEAD); if test -n "$$(git -c safe.directory=/workspace status --porcelain)"; then printf "%s-dirty\n" "$$revision"; else printf "%s\n" "$$revision"; fi')"; \
		SOURCE_REVISION="$$revision" sh tools/build_macos.sh

build-linux:
	$(RUN) cargo build --locked --release -p notrum-app --bin notrum-app
	$(RUN) sh -eu -c 'output="/workspace/dist/linux/$$(uname -m)/notrum"; install -D -s -m 755 "$$CARGO_TARGET_DIR/release/notrum-app" "$$output"; printf "BUILT_APP path=%s\n" "$$output"'

build-linux-smoke: build-linux
	$(RUN) sh tools/smoke_linux.sh

build-container:
	$(RUN) cargo build --workspace --all-targets --all-features

native-smoke: demo-data build
	@workspace="$$(mktemp -d /tmp/notrum-native-smoke.XXXXXX)"; trap 'rm -rf "$$workspace"' EXIT; cp -R examples/demo-workspace/. "$$workspace"; ./dist/Notrum.app/Contents/MacOS/Notrum "$$workspace" --smoke-exit-ms 1800

native-external-smoke: demo-data
	python3 tools/native_external_smoke.py

demo-data:
	$(RUN) python3 -B tools/generate_demo_data.py "$(DEMO_WORKSPACE)"

test-demo-data:
	$(RUN) python3 -B tools/test_generate_demo_data.py

check-macos:
	$(RUN) env CC_aarch64_apple_darwin=clang CFLAGS_aarch64_apple_darwin='-ffreestanding -nostdinc -isystem /usr/lib/llvm-14/lib/clang/14.0.6/include -I/workspace/docker/rust/macos-cross-headers' cargo check -p notrum-app --target aarch64-apple-darwin

package-macos:
	$(RUN) python3 tools/package_macos.py --binary "$(MACOS_BINARY)" --output "$(MACOS_OUTPUT)" --source-revision "$(SOURCE_REVISION)"

package-macos-smoke:
	$(RUN) python3 -B tools/test_package_macos.py

test:
	$(RUN) cargo test --workspace --all-features

test-release:
	$(RUN) cargo test --workspace --all-features --release

test-frontmatter:
	$(RUN) cargo test -p notrum-frontmatter

test-storage:
	$(RUN) cargo test -p notrum-storage

test-editor:
	$(RUN) cargo test -p notrum-editor

test-core:
	$(RUN) cargo test -p notrum-core

test-recovery:
	$(RUN) cargo test -p notrum-recovery

test-search:
	$(RUN) cargo test -p notrum-search

test-secure:
	$(RUN) cargo test -p notrum-secure

lint:
	$(RUN) cargo clippy --workspace --all-targets --all-features -- -D warnings

fmt:
	$(RUN) cargo fmt --all

fmt-check:
	$(RUN) cargo fmt --all --check

lock:
	$(RUN) cargo generate-lockfile

tree:
	$(RUN) cargo tree --workspace --edges normal,build,dev

audit: audit-source audit-dependencies audit-vulnerabilities

audit-source:
	$(RUN) python3 tools/audit_source.py

audit-dependencies:
	$(RUN) cargo deny --locked -L error check bans licenses sources

audit-vulnerabilities:
	$(RUN) cargo audit

diff-check:
	$(GIT) diff --check

status:
	$(GIT) status --short --branch

log:
	$(GIT) log --oneline --decorate -5

diff-stat:
	$(GIT) diff --stat

diff:
	$(GIT) diff

image:
	$(COMPOSE) build toolchain

benchmark-generate:
	$(RUN) cargo run --release -p notrum-buffer-probe -- generate /var/cache/notrum/benchmark-data

benchmark-ropey: benchmark-generate
	$(RUN) sh -c 'set -eu; for size in 10000000 100000000 1000000000; do runs=5; if [ "$$size" -eq 1000000000 ]; then runs=3; fi; cargo run --release -q -p notrum-buffer-probe -- probe ropey /var/cache/notrum/benchmark-data/notrum-$$size.md warmup; run=1; while [ "$$run" -le "$$runs" ]; do cargo run --release -q -p notrum-buffer-probe -- probe ropey /var/cache/notrum/benchmark-data/notrum-$$size.md "$$run"; run=$$((run + 1)); done; done'

benchmark-lapce: benchmark-generate
	$(RUN) sh -c 'set -eu; for size in 10000000 100000000 1000000000; do runs=5; if [ "$$size" -eq 1000000000 ]; then runs=3; fi; cargo run --release -q -p notrum-buffer-probe -- probe lapce /var/cache/notrum/benchmark-data/notrum-$$size.md warmup; run=1; while [ "$$run" -le "$$runs" ]; do cargo run --release -q -p notrum-buffer-probe -- probe lapce /var/cache/notrum/benchmark-data/notrum-$$size.md "$$run"; run=$$((run + 1)); done; done'

benchmark: benchmark-ropey benchmark-lapce

benchmark-editor: benchmark-generate
	$(RUN) sh -c 'set -eu; for size in 10000000 100000000 1000000000; do runs=5; if [ "$$size" -eq 1000000000 ]; then runs=3; fi; timeout 600s cargo run --release -q -p notrum-editor-probe -- /var/cache/notrum/benchmark-data/notrum-$$size.md warmup; run=1; while [ "$$run" -le "$$runs" ]; do timeout 600s cargo run --release -q -p notrum-editor-probe -- /var/cache/notrum/benchmark-data/notrum-$$size.md "$$run"; run=$$((run + 1)); done; done'

benchmark-viewport: benchmark-generate
	$(RUN) sh -c 'set -eu; for size in 10000000 100000000 1000000000; do runs=5; if [ "$$size" -eq 1000000000 ]; then runs=3; fi; timeout 600s cargo run --release -q -p notrum-ui-probe -- /var/cache/notrum/benchmark-data/notrum-$$size.md warmup; run=1; while [ "$$run" -le "$$runs" ]; do timeout 600s cargo run --release -q -p notrum-ui-probe -- /var/cache/notrum/benchmark-data/notrum-$$size.md "$$run"; run=$$((run + 1)); done; done'

benchmark-search: benchmark-generate
	$(RUN) sh -c 'set -eu; for size in 10000000 100000000 1000000000; do workspace=/var/cache/notrum/search-benchmark/workspace-$$size; dataset=/var/cache/notrum/benchmark-data/notrum-$$size.md; timeout 600s cargo run --release -q -p notrum-search-probe -- prepare "$$workspace" "$$dataset"; timeout 600s cargo run --release -q -p notrum-search-probe -- probe "$$workspace" "$$size"; done'

ui-build:
	$(RUN) cargo build -q -p notrum-app

ui-build-test-utils:
	$(RUN) cargo build -q -p notrum-app --features test-utils

ui-smoke: ui-build
	$(RUN) sh -c 'set -eu; workspace=$$(mktemp -d); python3 -B tools/generate_demo_data.py "$$workspace"; runtime=/tmp/notrum-xdg; mkdir -p "$$runtime"; chmod 700 "$$runtime"; Xvfb :99 -screen 0 1240x800x24 >/tmp/notrum-xvfb.log 2>&1 & server=$$!; trap "kill $$server 2>/dev/null || true" EXIT; attempt=0; until [ -S /tmp/.X11-unix/X99 ]; do attempt=$$((attempt + 1)); if ! kill -0 "$$server" 2>/dev/null || [ "$$attempt" -ge 50 ]; then cat /tmp/notrum-xvfb.log; exit 1; fi; sleep 0.1; done; export DISPLAY=:99 XDG_RUNTIME_DIR="$$runtime" FLOEM_FORCE_TINY_SKIA=1; /var/cache/notrum/target/debug/notrum-app "$$workspace" --smoke-exit-ms 1800'

ui-autosave-smoke: ui-build
	$(RUN) sh -c 'set -eu; workspace=$$(mktemp -d); python3 -B tools/generate_demo_data.py "$$workspace"; runtime=/tmp/notrum-xdg; mkdir -p "$$runtime"; chmod 700 "$$runtime"; Xvfb :99 -screen 0 1240x800x24 >/tmp/notrum-xvfb.log 2>&1 & server=$$!; trap "kill $$server 2>/dev/null || true" EXIT; attempt=0; until [ -S /tmp/.X11-unix/X99 ]; do attempt=$$((attempt + 1)); if ! kill -0 "$$server" 2>/dev/null || [ "$$attempt" -ge 50 ]; then cat /tmp/notrum-xvfb.log; exit 1; fi; sleep 0.1; done; export DISPLAY=:99 XDG_RUNTIME_DIR="$$runtime" FLOEM_FORCE_TINY_SKIA=1; /var/cache/notrum/target/debug/notrum-app "$$workspace" --smoke-autosave --smoke-exit-ms 2500; saved="$$workspace/notes/[notrum autosave smoke].md"; test ! -e "$$workspace/notes/Project Alpha.md"; grep -F "[notrum autosave smoke]" "$$saved"'

ui-recovery-smoke: ui-build
	$(RUN) sh -c 'set -eu; workspace=$$(mktemp -d); python3 -B tools/generate_demo_data.py "$$workspace"; runtime=/tmp/notrum-xdg; mkdir -p "$$runtime"; chmod 700 "$$runtime"; Xvfb :99 -screen 0 1240x800x24 >/tmp/notrum-xvfb.log 2>&1 & server=$$!; trap "kill $$server 2>/dev/null || true" EXIT; attempt=0; until [ -S /tmp/.X11-unix/X99 ]; do attempt=$$((attempt + 1)); if ! kill -0 "$$server" 2>/dev/null || [ "$$attempt" -ge 50 ]; then cat /tmp/notrum-xvfb.log; exit 1; fi; sleep 0.1; done; export DISPLAY=:99 XDG_RUNTIME_DIR="$$runtime" FLOEM_FORCE_TINY_SKIA=1; /var/cache/notrum/target/debug/notrum-app "$$workspace" --smoke-autosave --smoke-exit-ms 500; if grep -F "[notrum autosave smoke]" "$$workspace/notes/Project Alpha.md"; then exit 1; fi; test -n "$$(find "$$workspace/.notrum/recovery" -name "*.nrrec" -type f -print -quit)"; /var/cache/notrum/target/debug/notrum-app "$$workspace" --smoke-restore --smoke-exit-ms 2500; grep -F "[notrum autosave smoke]" "$$workspace/notes/Project Alpha.md"; test -z "$$(find "$$workspace/.notrum/recovery" -name "*.nrrec" -type f -print -quit)"'

ui-conflict-smoke: ui-build
	$(RUN) sh -c 'set -eu; workspace=$$(mktemp -d); python3 -B tools/generate_demo_data.py "$$workspace"; cp "$$workspace/notes/Reading List.md" "$$workspace/external.md"; runtime=/tmp/notrum-xdg; mkdir -p "$$runtime"; chmod 700 "$$runtime"; Xvfb :99 -screen 0 1240x800x24 >/tmp/notrum-xvfb.log 2>&1 & server=$$!; trap "kill $$server 2>/dev/null || true" EXIT; attempt=0; until [ -S /tmp/.X11-unix/X99 ]; do attempt=$$((attempt + 1)); if ! kill -0 "$$server" 2>/dev/null || [ "$$attempt" -ge 50 ]; then cat /tmp/notrum-xvfb.log; exit 1; fi; sleep 0.1; done; export DISPLAY=:99 XDG_RUNTIME_DIR="$$runtime" FLOEM_FORCE_TINY_SKIA=1; /var/cache/notrum/target/debug/notrum-app "$$workspace" --smoke-autosave --smoke-exit-ms 1800 & app=$$!; sleep 0.5; cp "$$workspace/external.md" "$$workspace/notes/Project Alpha.md"; wait $$app; cmp -s "$$workspace/external.md" "$$workspace/notes/Project Alpha.md"; test -n "$$(find "$$workspace/.notrum/recovery" -name "*.nrrec" -type f -print -quit)"'

ui-operations-smoke: ui-build
	$(RUN) sh -c 'set -eu; workspace=$$(mktemp -d); python3 -B tools/generate_demo_data.py "$$workspace"; cp "$$workspace/notes/Project Alpha.md" "$$workspace/project-before.md"; runtime=/tmp/notrum-xdg; mkdir -p "$$runtime"; chmod 700 "$$runtime"; Xvfb :99 -screen 0 1240x800x24 >/tmp/notrum-xvfb.log 2>&1 & server=$$!; trap "kill $$server 2>/dev/null || true" EXIT; attempt=0; until [ -S /tmp/.X11-unix/X99 ]; do attempt=$$((attempt + 1)); if ! kill -0 "$$server" 2>/dev/null || [ "$$attempt" -ge 50 ]; then cat /tmp/notrum-xvfb.log; exit 1; fi; sleep 0.1; done; export DISPLAY=:99 XDG_RUNTIME_DIR="$$runtime" FLOEM_FORCE_TINY_SKIA=1; /var/cache/notrum/target/debug/notrum-app "$$workspace" --smoke-operations --smoke-exit-ms 1800; cmp -s "$$workspace/project-before.md" "$$workspace/notes/Project Alpha.md"; test ! -e "$$workspace/notes/Smoke Note.md"; deleted="$$workspace/notes/Smoke Renamed.md"; test -f "$$deleted"; grep -F "title: '\''Smoke Renamed'\''" "$$deleted"; grep -F "  - '\''Smoke'\''" "$$deleted"; grep -F "pinned: true" "$$deleted"; grep -F "favorited: true" "$$deleted"; grep -F "deleted: true" "$$deleted"; test ! -e "$$workspace/.notrum/trash"; test -z "$$(find "$$workspace/notes" -name "*.notrum-tmp-*" -print -quit)"'

ui-click-creation: ui-build
	$(RUN) python3 -B tools/ui_acceptance.py creation

.PHONY: ui-click-rss-keyboard
ui-click-rss-keyboard: ui-build
	$(RUN) python3 -B tools/ui_acceptance.py rss_keyboard

.PHONY: ui-click-rss-cards
ui-click-rss-cards: ui-build
	$(RUN) python3 -B tools/ui_acceptance.py rss_cards

ui-click-lifecycle: ui-build
	$(RUN) python3 -B tools/ui_acceptance.py lifecycle

ui-click-tags: ui-build
	$(RUN) python3 -B tools/ui_acceptance.py tags

.PHONY: ui-click-localization
ui-click-localization: ui-build
	$(RUN) python3 -B tools/ui_acceptance.py localization

ui-click-workspace: ui-build
	$(RUN) python3 -B tools/ui_acceptance.py workspace

ui-click-compatibility: ui-build
	$(RUN) python3 -B tools/ui_acceptance.py compatibility

ui-click-categories: ui-build
	$(RUN) python3 -B tools/ui_acceptance.py categories

ui-click-interaction: ui-build
	$(RUN) python3 -B tools/ui_acceptance.py interaction

ui-click-caret: ui-build
	$(RUN) python3 -B tools/ui_acceptance.py caret

ui-click-editor: ui-build
	$(RUN) python3 -B tools/ui_acceptance.py editor

ui-click-context-menu: ui-build
	$(RUN) python3 -B tools/ui_acceptance.py context_menu

ui-click-selection: ui-build
	$(RUN) python3 -B tools/ui_acceptance.py selection

ui-click-persistence: ui-build
	$(RUN) python3 -B tools/ui_acceptance.py persistence

ui-click-recovery: ui-build
	$(RUN) python3 -B tools/ui_acceptance.py recovery

ui-click-conflict: ui-build
	$(RUN) python3 -B tools/ui_acceptance.py conflict

ui-click-search: ui-build
	$(RUN) python3 -B tools/ui_acceptance.py search

ui-click-find: ui-build
	$(RUN) python3 -B tools/ui_acceptance.py find

ui-click-resize: ui-build
	$(RUN) python3 -B tools/ui_acceptance.py resize

ui-click-visual: ui-build
	$(RUN) python3 -B tools/ui_acceptance.py visual

ui-click-password: ui-build-test-utils
	$(RUN) python3 -B tools/ui_acceptance.py password_dialog

ui-click-password-change: ui-build-test-utils
	$(RUN) python3 -B tools/ui_acceptance.py password_change

ui-click-secure: ui-build-test-utils
	$(RUN) python3 -B tools/ui_acceptance.py secure

ui-click-secure-recovery: ui-build-test-utils
	$(RUN) python3 -B tools/ui_acceptance.py secure_recovery

ui-click-secure-conflict: ui-build-test-utils
	$(RUN) python3 -B tools/ui_acceptance.py secure_conflict

ui-click-secure-integrity: ui-build-test-utils
	$(RUN) python3 -B tools/ui_acceptance.py secure_integrity

# Aggregates are nested deliberately. For final verification invoke only the
# widest target required by the change: ui-check, check, or all (default make).
# Finish each batch before rebuilding the shared binary with different features.
# -o suppresses build prerequisites in workers; every scenario gets its own container.
# Caret blink snapshots are timing-sensitive, so run them before parallel workers.
ui-acceptance: ui-build ui-click-caret
	$(MAKE) -j$(UI_JOBS) UI_ACCEPTANCE_PARALLEL=1 -o ui-build $(UI_ACCEPTANCE_STANDARD)
	$(MAKE) ui-build-test-utils
	$(MAKE) -j$(UI_JOBS) UI_ACCEPTANCE_PARALLEL=1 -o ui-build-test-utils $(UI_ACCEPTANCE_SECURE)

ui-check: ui-smoke ui-autosave-smoke ui-recovery-smoke ui-conflict-smoke ui-operations-smoke ui-acceptance
