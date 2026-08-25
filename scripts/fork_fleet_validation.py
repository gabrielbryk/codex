#!/usr/bin/env python3
"""Run registered Fork Fleet validation with a bounded local toolchain environment."""

import os
import platform
import stat
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
CODEX_RS = REPO_ROOT / "codex-rs"


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
    if not fork_fleet_just.is_file():
        raise SystemExit(f"Fork Fleet just executable is missing: {fork_fleet_just}")

    actions = {
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
