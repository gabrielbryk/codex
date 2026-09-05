#!/usr/bin/env python3
"""Run registered Fork Fleet validation with a bounded local toolchain environment."""

import os
import platform
import re
import selectors
import signal
import stat
import subprocess
import sys
import tempfile
import time
import tomllib
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, ContextManager, Iterable, Iterator


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
    (
        "validation.md",
        "Exact-target differential workspace gate",
        (
            "`INSTA_UPDATE=no`",
            "private per-side",
            "`CARGO_TARGET_DIR`",
            "disk-backed private validation",
            "hard output cap",
            "emitted excerpt is redacted",
            "Authorization",
            "quoted JSON credentials",
            "exactly one terminal summary",
            "expected nextest test-failure exit code",
            "fresh clone and mutable",
            "target HEAD, index tree, and untracked state",
            "does not prove semantic equivalence",
            "candidate-only failed test names",
            "exact-target failures",
            "disposable independent local clone",
            "every workspace-test command",
        ),
    ),
)
TARGET_SHA_PATTERN = re.compile(r"(?m)^- Target: `[^`]+` \(`(?P<sha>[0-9a-f]{40})`\)$")
FORMAT_FAILURE_PATTERN = re.compile(r"(?m)^Formatting failed: (?P<groups>.+)$")
REWORK_TRAILER_PATTERN = re.compile(r"(?m)^Fork-Fleet-Rework: \S+\s*$")
ANSI_ESCAPE_PATTERN = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")
NEXTEST_FAILURE_PATTERN = re.compile(
    r"^\s*(?:TRY\s+\d+\s+)?FAIL\s+\[[^]]+\]\s+\([^)]+\)\s+(?P<name>.+?)\s*$"
)
NEXTEST_SUMMARY_PATTERN = re.compile(
    r"^\s*Summary\s+\[[^]]+\]\s+\d+ tests? run:(?P<body>.+)$"
)
NEXTEST_FAILED_COUNT_PATTERN = re.compile(r"(?:^|, )(?P<count>\d+) failed(?:,|$)")
UNKNOWN_FAILURE_STATUS_PATTERN = re.compile(r"^\s*(?:TRY\s+\d+\s+)?\S*FAIL\S*\b")
SENSITIVE_ASSIGNMENT_PATTERN = re.compile(
    r"(?i)\b(token|secret|password|api[_-]?key|authorization|cookie)"
    r"(\s*[:=]\s*)([^\s,;]+)"
)
SENSITIVE_HEADER_PATTERN = re.compile(
    r"(?im)(?P<prefix>\b(?:proxy-)?authorization\s*:\s*"
    r"|\b(?:set-)?cookie\s*:\s*).*$"
)
SENSITIVE_JSON_STRING_PATTERN = re.compile(
    r'(?i)(?P<prefix>"[^"\n]*(?:token|secret|password|credential|cookie|authorization|api[_-]?key)'
    r'[^"\n]*"\s*:\s*)"(?:\\.|[^"\\])*"'
)
SENSITIVE_ENV_PATTERN = re.compile(
    r"(?i)(token|secret|password|credential|cookie|authorization|api.?key)"
)
COMMAND_TIMEOUT_SECONDS = 3_600
MAX_COMMAND_OUTPUT_BYTES = 64 * 1_024 * 1_024
NEXTEST_TEST_FAILURE_EXIT_CODE = 100
EXCERPT_HEAD_BYTES = 8 * 1_024
EXCERPT_TAIL_BYTES = 24 * 1_024


@dataclass(frozen=True)
class CommandResult:
    returncode: int
    failed_tests: frozenset[str]
    declared_failed_count: int | None
    summary_count: int
    excerpt: str
    parse_error: str | None = None
    timed_out: bool = False
    truncated: bool = False


@dataclass(frozen=True)
class WorkspaceCommand:
    cwd: Path
    argv: tuple[str, ...]


CommandRunner = Callable[[WorkspaceCommand, dict[str, str]], CommandResult]
TargetFactory = Callable[[str], ContextManager[Path]]


def validation_temp_parent() -> Path:
    parent = Path.home() / ".cache/fork-fleet/validation"
    parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    parent_stat = parent.stat()
    if parent.is_symlink() or parent_stat.st_uid != os.getuid():
        raise RuntimeError(f"validation temporary parent is not user-owned: {parent}")
    if stat.S_IMODE(parent_stat.st_mode) & 0o077:
        raise RuntimeError(f"validation temporary parent is not private: {parent}")
    return parent


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


def parse_nextest_lines(
    lines: Iterable[str],
) -> tuple[frozenset[str], int | None, int, str | None]:
    failed_tests = set()
    declared_failed_count = None
    summary_count = 0
    parse_error = None
    for raw_line in lines:
        line = ANSI_ESCAPE_PATTERN.sub("", raw_line)
        if summary_match := NEXTEST_SUMMARY_PATTERN.match(line):
            summary_count += 1
            failed_tests.clear()
            body = summary_match.group("body")
            failed_count_match = NEXTEST_FAILED_COUNT_PATTERN.search(body)
            declared_failed_count = (
                int(failed_count_match.group("count")) if failed_count_match else 0
            )
            continue
        if summary_count:
            if match := NEXTEST_FAILURE_PATTERN.match(line):
                failed_tests.add(" ".join(match.group("name").split()))
            elif UNKNOWN_FAILURE_STATUS_PATTERN.match(line):
                parse_error = f"unknown terminal nextest failure status: {line.strip()}"
    if summary_count != 1:
        parse_error = f"expected one terminal nextest summary, found {summary_count}"
    elif parse_error is None and declared_failed_count != len(failed_tests):
        parse_error = (
            "nextest summary declared "
            f"{declared_failed_count} failures but parsed {len(failed_tests)} unique names"
        )
    return frozenset(failed_tests), declared_failed_count, summary_count, parse_error


def parse_nextest_output(
    output: str,
) -> tuple[frozenset[str], int | None, int, str | None]:
    return parse_nextest_lines(output.splitlines())


def bounded_excerpt(output_file: Path) -> tuple[str, bool]:
    size = output_file.stat().st_size
    with output_file.open("rb") as output:
        head = output.read(EXCERPT_HEAD_BYTES)
        if size <= EXCERPT_HEAD_BYTES + EXCERPT_TAIL_BYTES:
            return head.decode("utf-8", errors="replace"), False
        output.seek(-EXCERPT_TAIL_BYTES, os.SEEK_END)
        tail = output.read(EXCERPT_TAIL_BYTES)
    omitted = size - EXCERPT_HEAD_BYTES - EXCERPT_TAIL_BYTES
    excerpt = (
        head.decode("utf-8", errors="replace")
        + f"\n... {omitted} output bytes omitted ...\n"
        + tail.decode("utf-8", errors="replace")
    )
    return excerpt, True


def terminate_process_group(process: subprocess.Popen[bytes]) -> None:
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        process.wait()
        return
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait()


def redact_output(output: str, environment: dict[str, str]) -> str:
    redacted = output
    for name, value in environment.items():
        if value and len(value) >= 4 and SENSITIVE_ENV_PATTERN.search(name):
            redacted = redacted.replace(value, "<REDACTED>")
    redacted = SENSITIVE_HEADER_PATTERN.sub(
        lambda match: f"{match.group('prefix')}<REDACTED>", redacted
    )
    redacted = SENSITIVE_JSON_STRING_PATTERN.sub(
        lambda match: f'{match.group("prefix")}"<REDACTED>"', redacted
    )
    return SENSITIVE_ASSIGNMENT_PATTERN.sub(
        lambda match: f"{match.group(1)}{match.group(2)}<REDACTED>", redacted
    )


def run_bounded_command(
    command: WorkspaceCommand,
    environment: dict[str, str],
) -> CommandResult:
    with tempfile.TemporaryDirectory(
        prefix="ffv-command-", dir=validation_temp_parent()
    ) as temporary_root:
        output_path = Path(temporary_root) / "output.log"
        with output_path.open("wb") as output:
            process = subprocess.Popen(
                command.argv,
                cwd=command.cwd,
                env=environment,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            assert process.stdout is not None
            selector = selectors.DefaultSelector()
            selector.register(process.stdout, selectors.EVENT_READ)
            deadline = time.monotonic() + COMMAND_TIMEOUT_SECONDS
            timed_out = False
            truncated = False
            written = 0
            try:
                while selector.get_map():
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        timed_out = True
                        terminate_process_group(process)
                        break
                    events = selector.select(timeout=min(remaining, 0.25))
                    for key, _mask in events:
                        chunk = os.read(key.fd, 64 * 1_024)
                        if not chunk:
                            selector.unregister(key.fileobj)
                            continue
                        available = MAX_COMMAND_OUTPUT_BYTES - written
                        if len(chunk) > available:
                            output.write(chunk[:available])
                            written += available
                            truncated = True
                            terminate_process_group(process)
                            selector.unregister(key.fileobj)
                            break
                        output.write(chunk)
                        written += len(chunk)
                    if truncated:
                        break
            finally:
                selector.close()
                process.stdout.close()
                if process.poll() is None:
                    if timed_out or truncated:
                        terminate_process_group(process)
                    else:
                        remaining = max(0.0, deadline - time.monotonic())
                        try:
                            process.wait(timeout=remaining)
                        except subprocess.TimeoutExpired:
                            timed_out = True
                            terminate_process_group(process)

        with output_path.open(encoding="utf-8", errors="replace") as output:
            (
                failed_tests,
                declared_failed_count,
                summary_count,
                parse_error,
            ) = parse_nextest_lines(output)
        excerpt, _ = bounded_excerpt(output_path)
        return CommandResult(
            returncode=process.returncode,
            failed_tests=failed_tests,
            declared_failed_count=declared_failed_count,
            summary_count=summary_count,
            excerpt=redact_output(excerpt, environment),
            parse_error=parse_error,
            timed_out=timed_out,
            truncated=truncated,
        )


def repository_snapshot(root: Path) -> tuple[str, str, str]:
    head = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=root, text=True
    ).strip()
    index_tree = subprocess.check_output(
        ["git", "write-tree"], cwd=root, text=True
    ).strip()
    status = subprocess.check_output(
        ["git", "status", "--porcelain=v1", "-uall"], cwd=root, text=True
    )
    return head, index_tree, status


def candidate_snapshot() -> tuple[str, str, str]:
    return repository_snapshot(REPO_ROOT)


@contextmanager
def materialize_target_clone(target_sha: str) -> Iterator[Path]:
    with tempfile.TemporaryDirectory(
        prefix="ffv-target-", dir=validation_temp_parent()
    ) as temporary_root:
        target_root = Path(temporary_root) / "codex"
        subprocess.run(
            [
                "git",
                "clone",
                "--no-hardlinks",
                "--no-checkout",
                "--quiet",
                str(REPO_ROOT),
                str(target_root),
            ],
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=300,
        )
        subprocess.run(
            ["git", "checkout", "--detach", "--quiet", target_sha],
            cwd=target_root,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=300,
        )
        checked_out_sha = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=target_root, text=True, timeout=30
        ).strip()
        if checked_out_sha != target_sha:
            raise RuntimeError(
                "disposable target clone resolved the wrong SHA: "
                f"{checked_out_sha} != {target_sha}"
            )
        yield target_root


@contextmanager
def isolated_command_environment(
    environment: dict[str, str], side: str
) -> Iterator[dict[str, str]]:
    with tempfile.TemporaryDirectory(
        prefix=f"ffv-{side}-", dir=validation_temp_parent()
    ) as temporary_root:
        private_root = Path(temporary_root)
        private_home = private_root / "home"
        private_codex_home = private_home / ".codex"
        private_tmp = private_root / "tmp"
        private_cache = private_root / "cache"
        cargo_target = private_root / "cargo-target"
        for directory in (
            private_home,
            private_codex_home,
            private_tmp,
            private_cache,
            cargo_target,
        ):
            directory.mkdir(mode=0o700)
        yield {
            **environment,
            "HOME": str(private_home),
            "CODEX_HOME": str(private_codex_home),
            "TMPDIR": str(private_tmp),
            "XDG_CACHE_HOME": str(private_cache),
            "CARGO_TARGET_DIR": str(cargo_target),
            "INSTA_UPDATE": "no",
        }


def print_command_result(
    label: str,
    command: WorkspaceCommand,
    result: CommandResult,
    environment: dict[str, str],
) -> None:
    rendered_command = redact_output(" ".join(command.argv), environment)
    print(
        f"workspace differential {label}: returncode={result.returncode} "
        f"failed={len(result.failed_tests)} command={rendered_command}"
    )
    if result.failed_tests:
        print(f"workspace differential {label} failures:")
        for test_name in sorted(result.failed_tests):
            print(f"- {redact_output(test_name, environment)}")
    if result.returncode:
        print(f"--- {label} bounded excerpt ---", file=sys.stderr)
        print(result.excerpt, file=sys.stderr)
        print(f"--- end {label} bounded excerpt ---", file=sys.stderr)


def nextest_result_error(result: CommandResult) -> str | None:
    if result.timed_out:
        return "command timed out"
    if result.truncated:
        return "command output exceeded the hard cap"
    if result.parse_error:
        return result.parse_error
    if result.returncode == 0 and result.declared_failed_count != 0:
        return "successful nextest command declared failures"
    if result.returncode == NEXTEST_TEST_FAILURE_EXIT_CODE:
        if not result.declared_failed_count:
            return "nextest test-failure exit did not declare failed tests"
        return None
    if result.returncode != 0:
        return (
            f"unexpected nextest exit code {result.returncode}; expected "
            f"0 or {NEXTEST_TEST_FAILURE_EXIT_CODE}"
        )
    return None


def run_differential_workspace_tests(
    commands: tuple[WorkspaceCommand, ...],
    environment: dict[str, str],
    target_sha: str,
    *,
    runner: CommandRunner = run_bounded_command,
    target_factory: TargetFactory = materialize_target_clone,
) -> int:
    before = candidate_snapshot()
    if before[2]:
        print("workspace differential requires a clean candidate", file=sys.stderr)
        return 1

    failed = False
    try:
        for command in commands:
            with isolated_command_environment(
                environment, "candidate"
            ) as candidate_environment:
                candidate_result = runner(command, candidate_environment)
            print_command_result(
                "candidate", command, candidate_result, candidate_environment
            )
            if candidate_error := nextest_result_error(candidate_result):
                print(
                    "candidate workspace result rejected: "
                    + redact_output(candidate_error, candidate_environment),
                    file=sys.stderr,
                )
                failed = True
                continue
            if candidate_result.returncode == 0:
                continue
            try:
                relative_cwd = command.cwd.relative_to(REPO_ROOT)
            except ValueError:
                print(
                    f"workspace command cwd is outside candidate: {command.cwd}",
                    file=sys.stderr,
                )
                failed = True
                continue
            try:
                with target_factory(target_sha) as target_root:
                    target_before = repository_snapshot(target_root)
                    if target_before[0] != target_sha:
                        raise RuntimeError(
                            "exact-target clone HEAD does not match the bound target SHA"
                        )
                    if target_before[2]:
                        raise RuntimeError(
                            "exact-target clone was dirty before testing"
                        )
                    target_command = WorkspaceCommand(
                        cwd=target_root / relative_cwd,
                        argv=command.argv,
                    )
                    with isolated_command_environment(
                        environment, "target"
                    ) as target_environment:
                        target_result = runner(target_command, target_environment)
                    target_after = repository_snapshot(target_root)
                    if target_after != target_before:
                        raise RuntimeError(
                            "exact-target HEAD, index, or worktree changed during testing"
                        )
            except (OSError, RuntimeError, subprocess.SubprocessError) as error:
                print(
                    f"exact-target setup or execution failed: {error}", file=sys.stderr
                )
                failed = True
                continue
            print_command_result(
                "exact-target", target_command, target_result, target_environment
            )
            if target_error := nextest_result_error(target_result):
                print(
                    "exact-target workspace result rejected: "
                    + redact_output(target_error, target_environment),
                    file=sys.stderr,
                )
                failed = True
                continue
            if target_result.returncode == 0:
                candidate_only = candidate_result.failed_tests
            else:
                candidate_only = (
                    candidate_result.failed_tests - target_result.failed_tests
                )

            if candidate_only:
                print(
                    "candidate-only failed test names: "
                    + redact_output(", ".join(sorted(candidate_only)), environment),
                    file=sys.stderr,
                )
                failed = True
            else:
                print(
                    "workspace differential accepted candidate failures as a "
                    "subset of exact-target failures"
                )
    finally:
        after = candidate_snapshot()
        if after != before:
            print(
                "candidate HEAD, index, or worktree changed during workspace validation",
                file=sys.stderr,
            )
            failed = True
    return int(failed)


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
        "workspace-tests": [],
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
        if sys.argv[1] == "workspace-tests":
            target_sha = bound_manifest_target(
                (REPO_ROOT / "PATCHES.md").read_text(encoding="utf-8"),
                candidate_target_sha(),
                os.environ.get("FORK_FLEET_TARGET_SHA"),
            )
            workspace_commands = (
                WorkspaceCommand(
                    cwd=REPO_ROOT,
                    argv=(
                        str(fork_fleet_just),
                        "test",
                        "--workspace",
                        "--exclude",
                        "codex-v8-poc",
                    ),
                ),
                WorkspaceCommand(
                    cwd=CODEX_RS,
                    argv=(
                        str(fork_fleet_just),
                        "test",
                        "-p",
                        "codex-v8-poc",
                        "--features",
                        "sandbox",
                    ),
                ),
            )
            raise SystemExit(
                run_differential_workspace_tests(
                    workspace_commands,
                    environment,
                    target_sha,
                )
            )
        for cwd, command in commands:
            completed = subprocess.run(command, cwd=cwd, env=environment, check=False)
            if completed.returncode:
                raise SystemExit(completed.returncode)


if __name__ == "__main__":
    main()
