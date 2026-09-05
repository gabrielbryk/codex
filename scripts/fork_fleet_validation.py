#!/usr/bin/env python3
"""Run registered Fork Fleet validation with a bounded local toolchain environment."""

import os
import platform
import re
import stat
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
CODEX_RS = REPO_ROOT / "codex-rs"
UPGRADE_SKILL = REPO_ROOT / ".codex/skills/upgrade-codex-fork"
CANONICAL_REFERENCES = (
    "coordination-and-status.md",
    "improvement-and-reporting.md",
    "planning-and-reconciliation.md",
    "shipping-and-cutover.md",
    "validation.md",
)
SECTION_CONTRACTS = (
    (
        "planning-and-reconciliation.md",
        "Operator and runtime registry freshness",
        ("newly executed `forkctl plan`", "Generated-CLI freshness"),
    ),
    (
        "planning-and-reconciliation.md",
        "Operator and runtime registry freshness",
        ("sibling stable or prerelease tags", "artifact source marker"),
    ),
    (
        "planning-and-reconciliation.md",
        "Logical patch decisions",
        (
            "`adjudicatedTargetSha` exactly equals",
            "`sourceSurface`",
            "`replacementSurface`",
            "`sharedSurfaces`",
            "source-free active leaf",
            "source-backed, active, reviewed-drop quarantine leaves",
            "upstream owner, behavior, and tests",
            "implementation/test ledger",
            "exact production symbols",
            "focused integration test IDs and artifacts",
            "independent semantic reviewer",
        ),
    ),
    (
        "planning-and-reconciliation.md",
        "Candidate preparation",
        (
            "aggregate per-leaf diff",
            "800 changed lines",
            "complex logic below 500 changed lines",
            "Fork Fleet alone enforces final-phase semantic ordering",
        ),
    ),
    (
        "improvement-and-reporting.md",
        "Bounded run evidence",
        ("Never extrapolate a full commit SHA", "`git rev-parse HEAD`"),
    ),
    (
        "validation.md",
        "Preflight",
        (
            "documentation/repository coherence evidence only",
            "does not inspect or report candidate finalization state",
            "registered release evidence",
            "`upgrade-workflow-tests`",
            "Fork Fleet",
        ),
    ),
    (
        "validation.md",
        "Heavy-gate placement",
        ("`agent-workloads.slice`", "`agent-slice-exec`", "`forkctl validate`"),
    ),
)
TARGET_SHA_PATTERN = re.compile(r"(?m)^- Target: `[^`]+` \(`(?P<sha>[0-9a-f]{40})`\)$")
FORMAT_FAILURE_PATTERN = re.compile(r"(?m)^Formatting failed: (?P<groups>.+)$")
REWORK_TRAILER_PATTERN = re.compile(r"(?m)^Fork-Fleet-Rework: \S+\s*$")


def markdown_section(text: str, heading: str) -> str:
    match = re.search(rf"(?ms)^##+ {re.escape(heading)}\s*$\n(.*?)(?=^##+ |\Z)", text)
    if match is None:
        raise SystemExit(f"upgrade workflow is missing section: {heading}")
    return match.group(1)


def require_section_contracts(documents: dict[str, str]) -> int:
    checked = 0
    for filename, heading, patterns in SECTION_CONTRACTS:
        section = markdown_section(documents[filename], heading)
        for marker in patterns:
            if marker not in section:
                raise SystemExit(
                    f"upgrade workflow section {heading!r} in {filename} "
                    f"is missing contract: {marker}"
                )
            checked += 1
    return checked


def audit_upgrade_workflow(
    skill_root: Path = UPGRADE_SKILL,
) -> None:
    skill_file = skill_root / "SKILL.md"
    if not skill_file.is_file():
        raise SystemExit(f"upgrade workflow entrypoint is missing: {skill_file}")

    skill_text = skill_file.read_text(encoding="utf-8")
    if not skill_text.startswith("---\nname: upgrade-codex-fork\n"):
        raise SystemExit("upgrade workflow frontmatter is missing its canonical name")
    if "\ndescription:" not in skill_text.split("---", 2)[1]:
        raise SystemExit("upgrade workflow frontmatter is missing its description")

    linked_references = [
        match.group(1)
        for match in re.finditer(r"\]\((references/[^)]+\.md)\)", skill_text)
    ]
    expected_references = [f"references/{name}" for name in CANONICAL_REFERENCES]
    actual_reference_files = sorted(
        path.name for path in (skill_root / "references").glob("*.md")
    )
    references_match = len(linked_references) == len(expected_references) and set(
        linked_references
    ) == set(expected_references)
    files_match = actual_reference_files == sorted(CANONICAL_REFERENCES)
    if not references_match or not files_match:
        missing = sorted(set(expected_references) - set(linked_references))
        unknown = sorted(set(linked_references) - set(expected_references))
        missing_files = sorted(set(CANONICAL_REFERENCES) - set(actual_reference_files))
        extra_files = sorted(set(actual_reference_files) - set(CANONICAL_REFERENCES))
        raise SystemExit(
            "upgrade workflow reference mismatch: "
            f"missing={missing}, unknown={unknown}, missing_files={missing_files}, "
            f"extra_files={extra_files}"
        )

    documents = {
        name: (skill_root / "references" / name).read_text(encoding="utf-8")
        for name in CANONICAL_REFERENCES
    }
    contract_count = require_section_contracts(documents)
    workflow_text = "\n".join([skill_text, *documents.values()])

    stale_targets = sorted(set(re.findall(r"\b[0-9a-f]{40}\b", workflow_text)))
    stale_versions = sorted(
        set(re.findall(r"\brust-v\d+(?:\.\d+){2}\b", workflow_text))
    )
    if stale_targets or stale_versions:
        raise SystemExit(
            "upgrade workflow contains stale static target decisions: "
            f"shas={stale_targets}, versions={stale_versions}"
        )

    print(
        "upgrade workflow documentation/repository coherence audit passed: "
        f"{len(expected_references)} references, {contract_count} section contracts; "
        "runtime registry, plan, target, surface, and finalization enforcement remains "
        "Fork Fleet"
    )


def manifest_target_sha(manifest_text: str) -> str:
    match = TARGET_SHA_PATTERN.search(manifest_text)
    if match is None:
        raise SystemExit("PATCHES.md is missing the exact target SHA")
    return match.group("sha")


def candidate_target_sha() -> str:
    commits = subprocess.check_output(
        ["git", "rev-list", "--first-parent", "HEAD"],
        cwd=REPO_ROOT,
        text=True,
    ).splitlines()
    for commit in commits:
        metadata = subprocess.check_output(
            ["git", "show", "-s", "--format=%ce%n%B", commit],
            cwd=REPO_ROOT,
            text=True,
        )
        committer_email, _, message = metadata.partition("\n")
        if committer_email == "fork-fleet@localhost":
            continue
        if REWORK_TRAILER_PATTERN.search(message):
            continue
        return commit
    raise SystemExit("could not derive the upstream target from candidate history")


def bound_manifest_target(
    manifest_text: str,
    derived_target_sha: str,
    injected_target_sha: str | None,
) -> str:
    target_sha = manifest_target_sha(manifest_text)
    if injected_target_sha is not None and injected_target_sha != derived_target_sha:
        raise SystemExit(
            "Fork Fleet target SHA does not match the candidate history boundary: "
            f"{injected_target_sha} != {derived_target_sha}"
        )
    if target_sha != derived_target_sha:
        raise SystemExit(
            "PATCHES.md target SHA does not match the candidate history boundary: "
            f"{target_sha} != {derived_target_sha}"
        )
    return target_sha


def formatter_failures(output: str) -> frozenset[str]:
    matches = list(FORMAT_FAILURE_PATTERN.finditer(output))
    if len(matches) != 1:
        return frozenset()
    return frozenset(group.strip() for group in matches[0].group("groups").split(","))


def run_format_validation(
    just: Path,
    environment: dict[str, str],
    injected_target_sha: str | None,
) -> int:
    target_sha = bound_manifest_target(
        (REPO_ROOT / "PATCHES.md").read_text(encoding="utf-8"),
        candidate_target_sha(),
        injected_target_sha,
    )
    completed = subprocess.run(
        [str(just), "fmt-check"],
        cwd=REPO_ROOT,
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        check=False,
    )
    if completed.returncode == 0:
        return 0

    justfile_changed = subprocess.run(
        ["git", "diff", "--quiet", target_sha, "--", "justfile"],
        cwd=REPO_ROOT,
        env=environment,
        check=False,
    ).returncode
    failures = formatter_failures(completed.stdout)
    if justfile_changed == 0 and failures == {"Just"}:
        print(
            "format validation accepted the exact upstream justfile baseline; "
            "all changed-language formatter groups passed"
        )
        return 0

    print(completed.stdout, end="", file=sys.stderr)
    return completed.returncode


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
    real_home = Path.home()
    fork_fleet_just = (
        real_home / ".local/share/fork-fleet/toolchains/just-1.51.0/bin/just"
    )
    actions = {
        "upgrade-workflow-audit": [],
        "upgrade-workflow-tests": [],
        "format": [],
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
        "workspace-check": [
            (
                CODEX_RS,
                [
                    "cargo",
                    "check",
                    "--workspace",
                    "--all-targets",
                    "--locked",
                ],
            )
        ],
        "workspace-tests": [
            (
                REPO_ROOT,
                [
                    str(fork_fleet_just),
                    "test",
                    "--workspace",
                    "--exclude",
                    "codex-v8-poc",
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

    if sys.argv[1] == "upgrade-workflow-audit":
        audit_upgrade_workflow()
        return
    if sys.argv[1] == "upgrade-workflow-tests":
        raise SystemExit(
            subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "unittest",
                    "scripts/test_fork_fleet_validation.py",
                    "-v",
                ],
                cwd=REPO_ROOT,
                check=False,
            ).returncode
        )
    if not fork_fleet_just.is_file():
        raise SystemExit(f"Fork Fleet just executable is missing: {fork_fleet_just}")

    commands = actions[sys.argv[1]]
    tool_paths = [
        real_home / ".local/bin",
        real_home / ".cargo/bin",
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
        if sys.argv[1] == "format":
            raise SystemExit(
                run_format_validation(
                    fork_fleet_just,
                    environment,
                    os.environ.get("FORK_FLEET_TARGET_SHA"),
                )
            )
        for cwd, command in commands:
            completed = subprocess.run(command, cwd=cwd, env=environment, check=False)
            if completed.returncode:
                raise SystemExit(completed.returncode)


if __name__ == "__main__":
    main()
