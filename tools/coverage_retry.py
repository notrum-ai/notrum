#!/usr/bin/env python3
# Copyright 2026 Evgeniy Udodov
# SPDX-License-Identifier: GPL-3.0-only
"""Resolve an existing trusted CI report without relabelling it as a new commit."""

import json
import os
from pathlib import Path
import re
import urllib.request


def coverage_revision(run, repository):
    if (run.get("path") != ".github/workflows/ci.yml"
            or run.get("status") != "completed"
            or run.get("event") not in ("push", "workflow_dispatch")
            or run.get("repository", {}).get("full_name") != repository
            or run.get("head_repository", {}).get("full_name") != repository):
        raise ValueError("coverage retry requires a completed same-repository CI push/manual run")
    commit = run.get("head_sha", "")
    branch = run.get("head_branch", "")
    if not re.fullmatch(r"[0-9a-f]{40}", commit) or branch not in ("master", "latest"):
        raise ValueError("coverage retry requires a full source SHA on master or latest")
    # A later packaging, acceptance, or upload failure does not invalidate LCOV.
    return commit, branch


def main():
    run_id = os.environ["COVERAGE_RUN_ID"]
    repository = os.environ["GITHUB_REPOSITORY"]
    if not re.fullmatch(r"[1-9][0-9]*", run_id):
        raise ValueError("CI run ID must be a positive decimal integer")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise ValueError("invalid GitHub repository")
    request = urllib.request.Request(
        f"https://api.github.com/repos/{repository}/actions/runs/{run_id}",
        headers={"Accept": "application/vnd.github+json",
                 "Authorization": f"Bearer {os.environ['GITHUB_TOKEN']}",
                 "X-GitHub-Api-Version": "2022-11-28"},
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        run = json.load(response)
    commit, branch = coverage_revision(run, repository)
    with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as output:
        output.write(f"commit={commit}\nbranch={branch}\n")
    print(f"Retrying coverage from CI run {run_id}: {branch} at {commit}")


if __name__ == "__main__":
    main()
