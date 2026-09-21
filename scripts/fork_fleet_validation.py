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
import xml.etree.ElementTree as ET
from collections.abc import Mapping
from pathlib import Path
from typing import Any

import tomllib

REPO_ROOT = Path(__file__).resolve().parents[1]
CODEX_RS = REPO_ROOT / "codex-rs"
NEXTEST_FORK_FLEET_PROFILE = "fork-fleet"

# Shared with both the `workspace-tests` and `workspace-tests-compile` actions.
WORKSPACE_TEST_PACKAGE_ARGS = [
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
]

CANONICAL_REFERENCES = (
    "planning-and-reconciliation.md",
    "release-history.md",
    "enforced-workflow.md",
    "validation.md",
    "shipping-and-cutover.md",
    "coordination-and-status.md",
    "improvement-and-reporting.md",
)
NORMALIZATION_ALLOWLIST = ("PATCHES.md", "codex-rs/Cargo.lock")


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
    if not isinstance(patches, list) or not patches:
        raise SystemExit("workflow test plan has no leaves")
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
            raise SystemExit(
                f"workflow test plan leaf {patch_id} has invalid source commits"
            )
        if not isinstance(dependencies, list) or not all(
            isinstance(dependency, str) for dependency in dependencies
        ):
            raise SystemExit(
                f"workflow test plan leaf {patch_id} has invalid dependencies"
            )
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
            raise SystemExit(
                f"fork patch manifest has incomplete patch evidence: {cells[1]}"
            )
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
    canonical_links = re.findall(r"\[[^]]+\]\(references/([^)]+)\)", skill_text)
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
    duplicate_ids = sorted(
        {patch_id for patch_id in patch_ids if patch_ids.count(patch_id) > 1}
    )
    if duplicate_ids:
        raise SystemExit(
            "fork patch manifest has duplicate patch ownership: "
            + ", ".join(duplicate_ids)
        )
    if test_plan is not None:
        validate_test_plan(manifest_text, test_plan)


def validate_attribution(
    repo_root: Path,
    plan: Mapping[str, Any],
    changed_paths: list[str] | None = None,
    env: Mapping[str, str] | None = None,
) -> None:
    """Reject candidate files that are not attributable to a declared patch surface."""
    env = env if env is not None else os.environ
    env_target = env.get("FORK_FLEET_TARGET_SHA")
    plan_target = plan.get("targetSha")
    if env_target and plan_target and env_target != plan_target:
        raise SystemExit("attribution target disagrees with the immutable plan")

    if changed_paths is None:
        target = env_target or plan_target
        completed = subprocess.run(
            ["git", "diff", "--name-only", f"{target}..HEAD"],
            cwd=repo_root,
            capture_output=True,
            text=True,
            check=False,
        )
        if completed.returncode:
            raise SystemExit(
                "git diff failed while computing attribution changed paths"
            )
        changed_paths = [line for line in completed.stdout.splitlines() if line]

    prefixes: set[str] = set()
    for patch in plan.get("patches") or []:
        for key in (
            "ownedSurfaces",
            "sourceSurfaces",
            "replacementSurfaces",
            "sharedSurfaces",
        ):
            for surface in patch.get(key) or []:
                if surface in ("", "."):
                    continue
                prefixes.add(surface)

    def matches(path: str, prefix: str) -> bool:
        return path == prefix or path.startswith(prefix.rstrip("/") + "/")

    matched_any = {prefix: False for prefix in prefixes}
    unattributed = []
    for path in changed_paths:
        if path in NORMALIZATION_ALLOWLIST:
            continue
        hit = False
        for prefix in prefixes:
            if matches(path, prefix):
                hit = True
                matched_any[prefix] = True
        if not hit:
            unattributed.append(path)

    if unattributed:
        raise SystemExit(
            "unattributed candidate files: " + ", ".join(sorted(unattributed))
        )

    for prefix in sorted(matched_any):
        if not matched_any[prefix]:
            print(f"warning: surface matches no candidate file: {prefix}")


def run_manifest_sync(
    repo_root: Path, plan: Mapping[str, Any], plan_path: Path
) -> None:
    completed = subprocess.run(
        [
            sys.executable,
            str(repo_root / "scripts/render_patches_manifest.py"),
            "--plan",
            str(plan_path),
            "--manifest",
            str(repo_root / "PATCHES.md"),
            "--check",
        ],
        cwd=repo_root,
        check=False,
    )
    if completed.returncode:
        raise SystemExit(completed.returncode)


def assert_clean_schema_fixtures(repo_root: Path) -> None:
    schema_dir = "codex-rs/app-server-protocol/schema"
    diff = subprocess.run(
        ["git", "diff", "--name-only", "--", schema_dir],
        cwd=repo_root,
        capture_output=True,
        text=True,
        check=False,
    )
    if diff.returncode:
        raise SystemExit("git diff failed while checking app-server schema fixtures")
    status = subprocess.run(
        ["git", "status", "--porcelain", "--", schema_dir],
        cwd=repo_root,
        capture_output=True,
        text=True,
        check=False,
    )
    if status.returncode:
        raise SystemExit("git status failed while checking app-server schema fixtures")

    paths = {line for line in diff.stdout.splitlines() if line}
    for line in status.stdout.splitlines():
        if line.startswith("??"):
            paths.add(line[3:].strip())
    if paths:
        raise SystemExit(
            "app-server schema fixtures drifted: " + ", ".join(sorted(paths))
        )


PLAN_ACTIONS = {
    "upgrade-workflow-audit": lambda repo_root, plan, plan_path: (
        validate_upgrade_workflow_contract(repo_root, plan)
    ),
    "attribution": lambda repo_root, plan, plan_path: validate_attribution(
        repo_root, plan
    ),
    "manifest-sync": run_manifest_sync,
}

POST_CHECKS = {
    "schema-fixtures": assert_clean_schema_fixtures,
}


def run_upgrade_workflow_tests() -> None:
    completed = subprocess.run(
        [sys.executable, str(REPO_ROOT / "scripts/test_fork_fleet_validation.py")],
        cwd=REPO_ROOT,
        check=False,
    )
    if completed.returncode:
        raise SystemExit(completed.returncode)


def rust_toolchain_environment(
    real_home: Path, env: Mapping[str, str] | None = None
) -> dict[str, str]:
    """Prefix PATH with the pinned rustup toolchain's bin dir (plus ~/.cargo/bin)
    so `just`/`cargo` invocations resolve `cargo`/`rustc` even when spawned
    with an otherwise-empty inherited environment. Mirrors the composition
    style of rusty_v8_environment: derive everything from real_home with
    fallbacks, never trust an already-sanitized `env` for anything but
    existing overrides.
    """
    base_env: dict[str, str] = dict(env) if env is not None else {}
    toolchain_text = (CODEX_RS / "rust-toolchain.toml").read_text(encoding="utf-8")
    match = re.search(r'channel\s*=\s*"([^"]+)"', toolchain_text)
    if not match:
        raise SystemExit(
            f"unable to resolve pinned channel from {CODEX_RS / 'rust-toolchain.toml'}"
        )
    channel = match.group(1)
    if sys.platform != "linux" or platform.machine() != "x86_64":
        raise SystemExit("Fork Fleet Codex validation supports Linux x86_64 only")
    triple = "x86_64-unknown-linux-gnu"

    home = Path(base_env.get("HOME") or os.environ.get("HOME") or real_home)
    rustup_home = Path(
        base_env.get("RUSTUP_HOME")
        or os.environ.get("RUSTUP_HOME")
        or (real_home / ".rustup")
    )
    cargo_home = Path(
        base_env.get("CARGO_HOME")
        or os.environ.get("CARGO_HOME")
        or (real_home / ".cargo")
    )
    rustup_toolchain = f"{channel}-{triple}"
    toolchain_bin = rustup_home / "toolchains" / rustup_toolchain / "bin"
    if not toolchain_bin.is_dir():
        raise SystemExit(
            f"pinned rustup toolchain bin directory is missing: {toolchain_bin}"
        )
    cargo_bin = cargo_home / "bin"

    # uv (installed to ~/.local/bin) and the pinned fork-fleet `just` toolchain
    # are not on the sanitized PATH validation stages inherit; add them here so
    # `format`/other actions that shell out to `just`/`uv` resolve the pinned
    # binaries rather than an asdf shim or nothing at all. JUST_BIN, if set,
    # names the directory containing the `just` binary and takes precedence
    # over the fork-fleet toolchain default.
    just_bin_override = base_env.get("JUST_BIN") or os.environ.get("JUST_BIN")
    just_bin = (
        Path(just_bin_override)
        if just_bin_override
        else real_home / ".local/share/fork-fleet/toolchains/just-1.51.0/bin"
    )
    local_bin = real_home / ".local/bin"

    path_entries = [str(toolchain_bin)]
    if cargo_bin.is_dir():
        path_entries.append(str(cargo_bin))
    if just_bin.is_dir():
        path_entries.append(str(just_bin))
    if local_bin.is_dir():
        path_entries.append(str(local_bin))
    path_entries.extend(["/usr/local/bin", "/usr/bin", "/bin"])
    existing_path = base_env.get("PATH")
    if existing_path:
        path_entries.append(existing_path)

    result = dict(base_env)
    result["PATH"] = os.pathsep.join(path_entries)
    result.setdefault("HOME", str(home))
    result.setdefault("CARGO_HOME", str(cargo_home))
    result.setdefault("RUSTUP_HOME", str(rustup_home))
    result.setdefault("RUSTUP_TOOLCHAIN", rustup_toolchain)
    result.setdefault("UV_PYTHON", "3.13")
    return result


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


def nextest_fork_fleet_junit_path() -> Path:
    return CODEX_RS / "target" / "nextest" / NEXTEST_FORK_FLEET_PROFILE / "junit.xml"


def workspace_tests_failures_path() -> Path:
    """Where the workspace-tests failure-set sidecar is written: the daemon's
    per-run artifact directory when it sets FORK_FLEET_ARTIFACT_DIR, else next
    to the nextest fork-fleet profile's own output directory.
    """
    artifact_dir = os.environ.get("FORK_FLEET_ARTIFACT_DIR")
    if artifact_dir:
        return Path(artifact_dir) / "workspace-tests-failures.json"
    return (
        CODEX_RS
        / "target"
        / "nextest"
        / NEXTEST_FORK_FLEET_PROFILE
        / ("workspace-tests-failures.json")
    )


def parse_nextest_junit_failures(junit_path: Path) -> set[str]:
    """Parse a nextest JUnit report into a set of "<binary-id>::<test name>"
    failure/error ids. Returns an empty set if the report does not exist (a
    build step ran instead of a test invocation, or nextest never produced a
    report).
    """
    if not junit_path.is_file():
        return set()
    tree = ET.parse(junit_path)
    failures: set[str] = set()
    for testsuite in tree.getroot().iter("testsuite"):
        binary_id = testsuite.get("name", "")
        for testcase in testsuite.iter("testcase"):
            if testcase.find("failure") is None and testcase.find("error") is None:
                continue
            test_name = testcase.get("name", "")
            failures.add(f"{binary_id}::{test_name}")
    return failures


def write_workspace_tests_failures(failures: set[str]) -> Path:
    output_path = workspace_tests_failures_path()
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(sorted(failures), indent=2) + "\n")
    return output_path


# `--only` selectors are "<binary-id>::<test-name>" round-tripping
# `parse_nextest_junit_failures`'s output ids exactly. The character class
# whitelist also doubles as shell-metacharacter rejection.
ONLY_SELECTOR_PATTERN = re.compile(r"^[A-Za-z0-9_:./-]+::.+$")

# Packages `--only` is allowed to target: the ones workspace-tests itself
# exercises (the shared package list, plus codex-v8-poc, which workspace-tests
# runs as a separate sandbox-featured invocation).
ONLY_SELECTOR_PACKAGES = [*WORKSPACE_TEST_PACKAGE_ARGS[1::2], "codex-v8-poc"]

# Extra cargo args a package's nextest invocation always needs, beyond
# `-p <package>` and the resolved target selector.
ONLY_SELECTOR_EXTRA_PACKAGE_ARGS: dict[str, list[str]] = {
    "codex-v8-poc": ["--features", "sandbox"],
}


def validate_only_selector(selector: str) -> None:
    if not ONLY_SELECTOR_PATTERN.fullmatch(selector):
        raise SystemExit(f"invalid --only selector: {selector}")


def parse_cargo_package_name(manifest_text: str) -> str | None:
    """Extract the `name` field of a Cargo.toml's `[package]` table only
    (never a dependency's `name = "..."` rename)."""
    table = re.search(r"(?ms)^\[package\]\s*$(.*?)(?=^\[|\Z)", manifest_text)
    if table is None:
        return None
    name = re.search(r'(?m)^name\s*=\s*"([^"]+)"', table.group(1))
    return name.group(1) if name else None


def parse_cargo_bin_names(manifest_text: str) -> list[str]:
    """Extract every `[[bin]]` table's `name` field, in file order."""
    names = []
    for table in re.finditer(r"(?ms)^\[\[bin\]\]\s*$(.*?)(?=^\[|\Z)", manifest_text):
        name = re.search(r'(?m)^name\s*=\s*"([^"]+)"', table.group(1))
        if name:
            names.append(name.group(1))
    return names


def find_package_crate_dir(codex_rs: Path, package: str) -> Path | None:
    for manifest in sorted(codex_rs.rglob("Cargo.toml")):
        if "target" in manifest.parts:
            continue
        if parse_cargo_package_name(manifest.read_text(encoding="utf-8")) == package:
            return manifest.parent
    return None


# binary-id -> (package, kind, target). kind is "lib", "bin", or "test";
# target is the bin/test-target name, or None for "lib".
OnlyBinaryId = tuple[str, str, str | None]


def discover_only_binary_ids(
    codex_rs: Path, packages: list[str]
) -> dict[str, OnlyBinaryId]:
    """Map every statically-discoverable nextest binary-id for `packages` to
    its (package, kind, target), mirroring nextest's own binary-id
    convention: the bare package name for the lib target, "<package>::bin/
    <name>" for a [[bin]] target, and "<package>::<file-stem>" for each
    tests/*.rs integration-test target. Static (Cargo.toml/tests-dir
    discovery) rather than a `cargo nextest list` round trip, so resolving
    `--only` needs no build.
    """
    binary_ids: dict[str, OnlyBinaryId] = {}
    for package in packages:
        crate_dir = find_package_crate_dir(codex_rs, package)
        if crate_dir is None:
            continue
        manifest_text = (crate_dir / "Cargo.toml").read_text(encoding="utf-8")
        if (crate_dir / "src" / "lib.rs").is_file():
            binary_ids[package] = (package, "lib", None)
        for bin_name in parse_cargo_bin_names(manifest_text):
            binary_ids[f"{package}::bin/{bin_name}"] = (package, "bin", bin_name)
        tests_dir = crate_dir / "tests"
        if tests_dir.is_dir():
            for test_file in sorted(tests_dir.glob("*.rs")):
                binary_ids[f"{package}::{test_file.stem}"] = (
                    package,
                    "test",
                    test_file.stem,
                )
    return binary_ids


def resolve_only_selector(
    selector: str, binary_ids: Mapping[str, OnlyBinaryId]
) -> tuple[str, str, str | None, str]:
    """Resolve a validated "<binary-id>::<test-name>" selector against known
    binary ids, returning (package, kind, target, test_name). Picks the
    longest matching binary-id prefix so e.g. "codex-core::all::mod::case"
    resolves to the "codex-core::all" test target (test name "mod::case"),
    not the "codex-core" lib target (test name "all::mod::case")."""
    candidates = [
        binary_id for binary_id in binary_ids if selector.startswith(binary_id + "::")
    ]
    if not candidates:
        raise SystemExit(
            f"--only selector does not match a known test binary: {selector}"
        )
    binary_id = max(candidates, key=len)
    package, kind, target = binary_ids[binary_id]
    test_name = selector[len(binary_id) + 2 :]
    if not test_name:
        raise SystemExit(f"--only selector is missing a test name: {selector}")
    return package, kind, target, test_name


def only_nextest_commands(
    selectors: list[str],
    binary_ids: Mapping[str, OnlyBinaryId],
    retries: int,
) -> list[tuple[Path, list[str]]]:
    """Build one `cargo nextest run` invocation per (package, kind, target)
    group among the resolved selectors, filtering to exactly the selected
    test names via `-E 'test(=name) | test(=name2) | ...'`."""
    groups: dict[tuple[str, str, str | None], list[str]] = {}
    order: list[tuple[str, str, str | None]] = []
    for selector in selectors:
        package, kind, target, test_name = resolve_only_selector(selector, binary_ids)
        key = (package, kind, target)
        if key not in groups:
            groups[key] = []
            order.append(key)
        groups[key].append(test_name)

    target_flags = {
        "lib": ["--lib"],
        "bin": lambda name: ["--bin", name],
        "test": lambda name: ["--test", name],
    }
    commands: list[tuple[Path, list[str]]] = []
    for package, kind, target in order:
        target_args = (
            target_flags["lib"] if kind == "lib" else target_flags[kind](target)
        )
        filter_expr = " | ".join(
            f"test(={test_name})" for test_name in groups[(package, kind, target)]
        )
        commands.append(
            (
                CODEX_RS,
                [
                    "cargo",
                    "nextest",
                    "run",
                    "--profile",
                    NEXTEST_FORK_FLEET_PROFILE,
                    "-p",
                    package,
                    *target_args,
                    *ONLY_SELECTOR_EXTRA_PACKAGE_ARGS.get(package, []),
                    "-E",
                    filter_expr,
                    "--retries",
                    str(retries),
                ],
            )
        )
    return commands


def parse_workspace_tests_args(argv: list[str]) -> tuple[list[str], int | None]:
    """Parse `workspace-tests`/`workspace-tests-compile` trailing args:
    repeatable `--only <selector>` and an optional `--retries <N>`."""
    only: list[str] = []
    retries: int | None = None
    index = 0
    while index < len(argv):
        arg = argv[index]
        if arg == "--only":
            if index + 1 >= len(argv):
                raise SystemExit("--only requires a value")
            only.append(argv[index + 1])
            index += 2
        elif arg == "--retries":
            if index + 1 >= len(argv):
                raise SystemExit("--retries requires a value")
            try:
                retries = int(argv[index + 1])
            except ValueError as error:
                raise SystemExit(
                    f"--retries must be an integer: {argv[index + 1]}"
                ) from error
            index += 2
        else:
            raise SystemExit(f"unrecognized argument: {arg}")
    for selector in only:
        validate_only_selector(selector)
    return only, retries


def is_nextest_invocation(command: list[str]) -> bool:
    return "nextest" in command or (len(command) >= 2 and command[1] == "test")


def run_workspace_tests(
    commands: list[tuple[Path, list[str]]], environment: Mapping[str, str]
) -> None:
    """Run the workspace-tests command sequence (code-mode-host build, then the
    nextest invocations). Build failures raise immediately (nothing to parse).
    A nextest invocation's non-zero exit is deferred until every invocation has
    run and its JUnit report has been folded into the failure-set sidecar, so
    the sidecar reflects every failure even when an earlier package fails
    first; the process still exits with nextest's own (first non-zero) code.
    """
    final_returncode = 0
    failures: set[str] = set()
    for cwd, command in commands:
        completed = subprocess.run(command, cwd=cwd, env=environment, check=False)
        if not is_nextest_invocation(command):
            if completed.returncode:
                raise SystemExit(completed.returncode)
            continue
        failures |= parse_nextest_junit_failures(nextest_fork_fleet_junit_path())
        if completed.returncode and not final_returncode:
            final_returncode = completed.returncode
    write_workspace_tests_failures(failures)
    if final_returncode:
        raise SystemExit(final_returncode)


def main() -> None:
    if len(sys.argv) == 3 and sys.argv[1] in PLAN_ACTIONS:
        plan_path = Path(sys.argv[2]).resolve()
        PLAN_ACTIONS[sys.argv[1]](REPO_ROOT, load_immutable_plan(plan_path), plan_path)
        return
    if len(sys.argv) == 2 and sys.argv[1] == "upgrade-workflow-tests":
        run_upgrade_workflow_tests()
        return

    real_home = Path.home()
    just_bin_override = os.environ.get("JUST_BIN")
    fork_fleet_just = (
        Path(just_bin_override) / "just"
        if just_bin_override
        else real_home / ".local/share/fork-fleet/toolchains/just-1.51.0/bin/just"
    )
    if not fork_fleet_just.is_file():
        raise SystemExit(f"Fork Fleet just executable is missing: {fork_fleet_just}")

    actions = {
        "upgrade-workflow-audit <immutable-plan-path>": [],
        "attribution <immutable-plan-path>": [],
        "manifest-sync <immutable-plan-path>": [],
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
                CODEX_RS,
                ["cargo", "build", "-p", "codex-code-mode-host"],
            ),
            (
                REPO_ROOT,
                [
                    str(fork_fleet_just),
                    "test",
                    "--test-threads=4",
                    "--profile",
                    NEXTEST_FORK_FLEET_PROFILE,
                    *WORKSPACE_TEST_PACKAGE_ARGS,
                ],
            ),
            (
                CODEX_RS,
                [
                    str(fork_fleet_just),
                    "test",
                    "--profile",
                    NEXTEST_FORK_FLEET_PROFILE,
                    "-p",
                    "codex-v8-poc",
                    "--features",
                    "sandbox",
                ],
            ),
        ],
        "workspace-tests-compile": [
            (
                REPO_ROOT,
                [
                    str(fork_fleet_just),
                    "test",
                    "--no-run",
                    *WORKSPACE_TEST_PACKAGE_ARGS,
                ],
            ),
            (
                CODEX_RS,
                [
                    str(fork_fleet_just),
                    "test",
                    "--no-run",
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
        "schema-fixtures": [
            (REPO_ROOT, [str(fork_fleet_just), "write-app-server-schema"]),
            (
                REPO_ROOT,
                [str(fork_fleet_just), "write-app-server-schema", "--experimental"],
            ),
        ],
    }
    workspace_tests_style = sys.argv[1:2] in (
        ["workspace-tests"],
        ["workspace-tests-compile"],
    )
    if workspace_tests_style:
        action = sys.argv[1]
        only_selectors, retries = parse_workspace_tests_args(sys.argv[2:])
    elif len(sys.argv) != 2 or sys.argv[1] not in actions:
        choices = ", ".join(sorted(actions))
        raise SystemExit(f"usage: {Path(sys.argv[0]).name} <{choices}>")
    else:
        action = sys.argv[1]
        only_selectors, retries = [], None

    commands = actions[action]
    if action == "workspace-tests-compile":
        # `--only`/`--retries` are accepted but not meaningful for a
        # compile-only (`--no-run`) invocation: it always compiles every
        # workspace-tests package.
        pass
    elif action == "workspace-tests" and only_selectors:
        binary_ids = discover_only_binary_ids(CODEX_RS, ONLY_SELECTOR_PACKAGES)
        commands = [commands[0]] + only_nextest_commands(
            only_selectors, binary_ids, retries if retries is not None else 0
        )
    elif action == "workspace-tests" and retries is not None:
        retries_args = ["--retries", str(retries)]
        commands = [
            (
                cwd,
                [*command, *retries_args]
                if is_nextest_invocation(command)
                else command,
            )
            for cwd, command in commands
        ]
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
        environment = rust_toolchain_environment(real_home, environment)
        if action == "workspace-tests":
            run_workspace_tests(commands, environment)
        else:
            for cwd, command in commands:
                completed = subprocess.run(
                    command, cwd=cwd, env=environment, check=False
                )
                if completed.returncode:
                    raise SystemExit(completed.returncode)

    post_check = POST_CHECKS.get(action)
    if post_check is not None:
        post_check(REPO_ROOT)


if __name__ == "__main__":
    main()
