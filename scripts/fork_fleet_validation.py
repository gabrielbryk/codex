#!/usr/bin/env python3
"""Run registered Fork Fleet validation with a bounded local toolchain environment."""

import json
import os
import platform
import re
import stat
import subprocess
import sys
import tempfile
from collections.abc import Mapping
from pathlib import Path
from typing import Any

import tomllib

REPO_ROOT = Path(__file__).resolve().parents[1]
CODEX_RS = REPO_ROOT / "codex-rs"

CANONICAL_REFERENCES = (
    "planning-and-reconciliation.md",
    "release-history.md",
    "enforced-workflow.md",
    "validation.md",
    "shipping-and-cutover.md",
    "coordination-and-status.md",
    "improvement-and-reporting.md",
)
EXPECTED_PATCH_COUNT = 42


def load_immutable_plan(path: Path) -> Mapping[str, Any]:
    """Load a Fork Fleet immutable plan object or its versioned output envelope."""
    try:
        payload = json.loads(path.read_text())
    except OSError as error:
        raise SystemExit(f"workflow test plan cannot be read: {path}") from error
    except json.JSONDecodeError as error:
        raise SystemExit(f"workflow test plan is invalid JSON: {path}") from error
    if not isinstance(payload, Mapping):
        raise SystemExit("workflow test plan has invalid root")
    data = payload.get("data", payload)
    if not isinstance(data, Mapping):
        raise SystemExit("workflow test plan has invalid data envelope")
    return data


def validate_test_plan(manifest_text: str, test_plan: Mapping[str, Any]) -> None:
    """Validate a supplied immutable-plan fixture without reading or spawning external tools."""
    target_sha = test_plan.get("targetSha")
    patches = test_plan.get("patches")
    source_commit_order = test_plan.get("sourceCommitOrder")
    if not isinstance(target_sha, str) or not re.fullmatch(r"[0-9a-f]{40}", target_sha):
        raise SystemExit("workflow test plan has invalid target SHA")
    if not isinstance(patches, list) or len(patches) != EXPECTED_PATCH_COUNT:
        raise SystemExit(
            f"workflow test plan must contain exactly {EXPECTED_PATCH_COUNT} leaves"
        )
    if not isinstance(source_commit_order, list) or not source_commit_order:
        raise SystemExit("workflow test plan has no source commit order")
    if not all(
        isinstance(commit_sha, str) and re.fullmatch(r"[0-9a-f]{40}", commit_sha)
        for commit_sha in source_commit_order
    ):
        raise SystemExit("workflow test plan has invalid source commit order")
    if len(set(source_commit_order)) != len(source_commit_order):
        raise SystemExit("workflow test plan has duplicate source commit order")

    plan_ids: list[str] = []
    dependencies_by_patch: dict[str, list[str]] = {}
    owned_commits: set[str] = set()
    for patch in patches:
        if not isinstance(patch, Mapping):
            raise SystemExit("workflow test plan has invalid leaf")
        patch_id = patch.get("patchId")
        commit_shas = patch.get("commitShas")
        dependencies = patch.get("dependsOn", [])
        if not isinstance(patch_id, str) or not patch_id:
            raise SystemExit("workflow test plan has leaf without patchId")
        if not isinstance(commit_shas, list):
            raise SystemExit(f"workflow test plan leaf {patch_id} has invalid source commits")
        if not isinstance(dependencies, list) or not all(
            isinstance(dependency, str) for dependency in dependencies
        ):
            raise SystemExit(f"workflow test plan leaf {patch_id} has invalid dependencies")
        for commit_sha in commit_shas:
            if not isinstance(commit_sha, str) or not re.fullmatch(
                r"[0-9a-f]{40}", commit_sha
            ):
                raise SystemExit(
                    f"workflow test plan leaf {patch_id} has invalid source commit"
                )
            if commit_sha in owned_commits:
                raise SystemExit(
                    f"workflow test plan has duplicate source commit ownership: {commit_sha}"
                )
            owned_commits.add(commit_sha)
        plan_ids.append(patch_id)
        dependencies_by_patch[patch_id] = dependencies

    if len(set(plan_ids)) != len(plan_ids):
        raise SystemExit("workflow test plan has duplicate leaf ownership")
    if owned_commits != set(source_commit_order):
        raise SystemExit("workflow test plan source commits are not owned exactly once")
    for patch_id, dependencies in dependencies_by_patch.items():
        for dependency in dependencies:
            if dependency not in dependencies_by_patch:
                raise SystemExit(
                    f"workflow test plan leaf {patch_id} has unknown dependency: "
                    f"{dependency}"
                )

    visiting: set[str] = set()
    visited: set[str] = set()

    def visit(patch_id: str) -> None:
        if patch_id in visited:
            return
        if patch_id in visiting:
            raise SystemExit(f"workflow test plan has dependency cycle at: {patch_id}")
        visiting.add(patch_id)
        for dependency in dependencies_by_patch[patch_id]:
            visit(dependency)
        visiting.remove(patch_id)
        visited.add(patch_id)

    for patch_id in plan_ids:
        visit(patch_id)

    manifest_target = re.search(
        r"Current upstream target:.*?([0-9a-f]{40})", manifest_text, re.DOTALL
    )
    if manifest_target is None or manifest_target.group(1) != target_sha:
        raise SystemExit("fork patch manifest target does not match workflow test plan")
    manifest_ids = extract_manifest_patch_ids(manifest_text)
    if set(manifest_ids) != set(plan_ids):
        raise SystemExit("fork patch manifest leaves do not match workflow test plan")


def extract_manifest_patch_ids(manifest_text: str) -> list[str]:
    """Extract logical-patch IDs from the manifest's decision table."""
    patch_ids = []
    for line in manifest_text.splitlines():
        if not line.startswith("| `"):
            continue
        cells = [cell.strip() for cell in line.split("|")]
        if len(cells) < 5 or not cells[1].startswith("`") or not cells[1].endswith("`"):
            continue
        patch_ids.append(cells[1][1:-1])
        if cells[3] not in {"apply", "rework", "drop"} or not cells[4]:
            raise SystemExit(f"fork patch manifest has incomplete patch evidence: {cells[1]}")
    return patch_ids


def validate_upgrade_workflow_contract(
    repo_root: Path, test_plan: Mapping[str, Any] | None = None
) -> None:
    """Reject a malformed repository-owned upgrade workflow before expensive gates."""
    skill = repo_root / ".codex/skills/upgrade-codex-fork/SKILL.md"
    manifest = repo_root / "PATCHES.md"
    missing = [str(path) for path in (skill, manifest) if not path.is_file()]
    if missing:
        raise SystemExit(f"upgrade workflow contract is missing: {', '.join(missing)}")

    skill_text = skill.read_text()
    markers = (
        "name: upgrade-codex-fork",
        "Fleet owns registry intent and candidates.",
        "Never edit installed caches or rebase the maintained checkout.",
        "## One run, one next action",
    )
    missing_markers = [marker for marker in markers if marker not in skill_text]
    if missing_markers:
        raise SystemExit(
            "upgrade workflow skill is missing required markers: "
            + ", ".join(missing_markers)
        )

    references = skill.parent / "references"
    missing_references = [
        name for name in CANONICAL_REFERENCES if not (references / name).is_file()
    ]
    if missing_references:
        raise SystemExit(
            "upgrade workflow is missing canonical references: "
            + ", ".join(missing_references)
        )
    canonical_links = re.findall(
        r"\[[^]]+\]\(references/([^)]+)\)", skill_text
    )
    if canonical_links != list(CANONICAL_REFERENCES):
        raise SystemExit(
            "upgrade workflow canonical reference links do not match required order"
        )

    manifest_text = manifest.read_text()
    manifest_markers = (
        "# Fork patch manifest",
        "Fork Fleet owns the logical patch mapping;",
        "Current upstream target:",
        "## Maintained logical patches",
        "## Per-upgrade verification",
        "scripts/fork_fleet_validation.py",
    )
    missing_manifest_markers = [
        marker for marker in manifest_markers if marker not in manifest_text
    ]
    if missing_manifest_markers:
        raise SystemExit(
            "fork patch manifest is missing required evidence: "
            + ", ".join(missing_manifest_markers)
        )

    patch_ids = extract_manifest_patch_ids(manifest_text)
    if not patch_ids:
        raise SystemExit("fork patch manifest has no maintained patches")
    duplicate_ids = sorted({patch_id for patch_id in patch_ids if patch_ids.count(patch_id) > 1})
    if duplicate_ids:
        raise SystemExit(
            "fork patch manifest has duplicate patch ownership: " + ", ".join(duplicate_ids)
        )
    if test_plan is not None:
        validate_test_plan(manifest_text, test_plan)


def run_upgrade_workflow_tests() -> None:
    completed = subprocess.run(
        [sys.executable, str(REPO_ROOT / "scripts/test_fork_fleet_validation.py")],
        cwd=REPO_ROOT,
        check=False,
    )
    if completed.returncode:
        raise SystemExit(completed.returncode)


def rusty_v8_environment(real_home: Path) -> dict[str, str]:
    with (CODEX_RS / "Cargo.lock").open("rb") as lock_file:
        packages = tomllib.load(lock_file)["package"]
    version = next(
        package["version"] for package in packages if package["name"] == "v8"
    )
    if sys.platform != "linux" or platform.machine() != "x86_64":
        raise SystemExit("Fork Fleet Codex validation supports Linux x86_64 only")
    triple = "x86_64-unknown-linux-gnu"
    cache = real_home / f".cache/codex-package/rusty-v8-{version}-{triple}"
    archive = cache / f"librusty_v8_ptrcomp_sandbox_release_{triple}.a.gz"
    binding = cache / f"src_binding_ptrcomp_sandbox_release_{triple}.rs"
    for artifact in (archive, binding):
        if not artifact.is_file():
            raise SystemExit(f"required verified V8 artifact is missing: {artifact}")
    return {
        "RUSTY_V8_ARCHIVE": str(archive),
        "RUSTY_V8_SRC_BINDING_PATH": str(binding),
    }


def main() -> None:
    if len(sys.argv) == 3 and sys.argv[1] == "upgrade-workflow-audit":
        validate_upgrade_workflow_contract(
            REPO_ROOT, load_immutable_plan(Path(sys.argv[2]).resolve())
        )
        return
    if len(sys.argv) == 2 and sys.argv[1] == "upgrade-workflow-tests":
        run_upgrade_workflow_tests()
        return

    real_home = Path.home()
    fork_fleet_just = (
        real_home / ".local/share/fork-fleet/toolchains/just-1.51.0/bin/just"
    )
    if not fork_fleet_just.is_file():
        raise SystemExit(f"Fork Fleet just executable is missing: {fork_fleet_just}")

    actions = {
        "upgrade-workflow-audit <immutable-plan-path>": [],
        "upgrade-workflow-tests": [],
        "format": [(REPO_ROOT, [str(fork_fleet_just), "fmt-check"])],
        "locked-metadata": [
            (
                CODEX_RS,
                [
                    "cargo",
                    "metadata",
                    "--locked",
                    "--format-version",
                    "1",
                    "--no-deps",
                ],
            )
        ],
        "workspace-tests": [
            (
                REPO_ROOT,
                [
                    str(fork_fleet_just),
                    "test",
                    "--test-threads=4",
                    "-p",
                    "codex-app-server",
                    "-p",
                    "codex-app-server-client",
                    "-p",
                    "codex-app-server-daemon",
                    "-p",
                    "codex-app-server-protocol",
                    "-p",
                    "codex-app-server-transport",
                    "-p",
                    "codex-cli",
                    "-p",
                    "codex-config",
                    "-p",
                    "codex-core",
                    "-p",
                    "codex-external-agent-migration",
                    "-p",
                    "codex-goal-extension",
                    "-p",
                    "codex-mcp",
                    "-p",
                    "codex-otel",
                    "-p",
                    "codex-protocol",
                    "-p",
                    "codex-rmcp-client",
                    "-p",
                    "codex-thread-manager-sample",
                    "-p",
                    "codex-tui",
                    "-p",
                    "codex-uds",
                    "-p",
                    "codex-utils-pty",
                ],
            ),
            (
                CODEX_RS,
                [
                    str(fork_fleet_just),
                    "test",
                    "-p",
                    "codex-v8-poc",
                    "--features",
                    "sandbox",
                ],
            ),
        ],
        "v8-poc-tests": [
            (
                CODEX_RS,
                [
                    str(fork_fleet_just),
                    "test",
                    "-p",
                    "codex-v8-poc",
                    "--features",
                    "sandbox",
                ],
            )
        ],
        "bazel-lock": [(REPO_ROOT, [str(fork_fleet_just), "bazel-lock-check"])],
        "argument-comment-lint": [
            (REPO_ROOT, [str(fork_fleet_just), "argument-comment-lint"])
        ],
        "release-build": [(REPO_ROOT, [str(fork_fleet_just), "build-for-release"])],
    }
    if len(sys.argv) != 2 or sys.argv[1] not in actions:
        choices = ", ".join(sorted(actions))
        raise SystemExit(f"usage: {Path(sys.argv[0]).name} <{choices}>")

    commands = actions[sys.argv[1]]
    tool_paths = [
        real_home / ".local/bin",
        real_home / ".asdf/shims",
        real_home / ".proto/bin",
        Path("/usr/local/sbin"),
        Path("/usr/local/bin"),
        Path("/usr/sbin"),
        Path("/usr/bin"),
        Path("/sbin"),
        Path("/bin"),
    ]
    temporary_parent = None
    runtime_dir = Path(f"/run/user/{os.getuid()}")
    for candidate in (Path("/dev/shm"), runtime_dir):
        try:
            candidate_stat = candidate.stat()
        except OSError:
            continue
        mode = stat.S_IMODE(candidate_stat.st_mode)
        secure_shared_tmp = (
            candidate == Path("/dev/shm")
            and candidate_stat.st_uid == 0
            and mode == 0o1777
        )
        secure_runtime_dir = (
            candidate == runtime_dir
            and candidate_stat.st_uid == os.getuid()
            and not mode & 0o077
        )
        if secure_shared_tmp or secure_runtime_dir:
            temporary_parent = candidate
            break
    if temporary_parent is None:
        raise SystemExit("no secure temporary parent is available")

    with tempfile.TemporaryDirectory(
        prefix="ffv-", dir=temporary_parent
    ) as temporary_root:
        private_root = Path(temporary_root)
        private_home = private_root / "home"
        private_tmp = private_root / "tmp"
        private_codex_home = private_home / ".codex"
        private_cache = private_root / "cache"
        private_home.mkdir(mode=0o700)
        private_tmp.mkdir(mode=0o700)
        private_codex_home.mkdir(mode=0o700)
        private_cache.mkdir(mode=0o700)
        environment = {
            "HOME": str(private_home),
            "CODEX_HOME": str(private_codex_home),
            "TMPDIR": str(private_tmp),
            "PATH": os.pathsep.join(map(str, tool_paths)),
            "ASDF_DATA_DIR": str(real_home / ".asdf"),
            "PROTO_HOME": str(real_home / ".proto"),
            "CARGO_HOME": str(real_home / ".cargo"),
            "RUSTUP_HOME": str(real_home / ".rustup"),
            "XDG_CACHE_HOME": str(private_cache),
            "FORK_FLEET_NETWORK": os.environ.get("FORK_FLEET_NETWORK", "denied"),
            **rusty_v8_environment(real_home),
        }
        for cwd, command in commands:
            completed = subprocess.run(command, cwd=cwd, env=environment, check=False)
            if completed.returncode:
                raise SystemExit(completed.returncode)


if __name__ == "__main__":
    main()
