#!/usr/bin/env python3
"""Run registered Fork Fleet validation with a bounded local toolchain environment."""

import os
import platform
import sys
import tomllib
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
CODEX_RS = REPO_ROOT / "codex-rs"
FORK_FLEET_JUST = (
    Path.home() / ".local/share/fork-fleet/toolchains/just-1.51.0/bin/just"
)


def rusty_v8_environment() -> dict[str, str]:
    with (CODEX_RS / "Cargo.lock").open("rb") as lock_file:
        packages = tomllib.load(lock_file)["package"]
    version = next(
        package["version"] for package in packages if package["name"] == "v8"
    )
    if sys.platform != "linux" or platform.machine() != "x86_64":
        raise SystemExit("Fork Fleet Codex validation supports Linux x86_64 only")
    triple = "x86_64-unknown-linux-gnu"
    cache = Path.home() / f".cache/codex-package/rusty-v8-{version}-{triple}"
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
    actions = {
        "format": (REPO_ROOT, [str(FORK_FLEET_JUST), "fmt-check"]),
        "locked-metadata": (
            CODEX_RS,
            ["cargo", "metadata", "--locked", "--format-version", "1", "--no-deps"],
        ),
        "workspace-tests": (REPO_ROOT, [str(FORK_FLEET_JUST), "test"]),
        "bazel-lock": (REPO_ROOT, [str(FORK_FLEET_JUST), "bazel-lock-check"]),
        "argument-comment-lint": (
            REPO_ROOT,
            [str(FORK_FLEET_JUST), "argument-comment-lint"],
        ),
        "release-build": (REPO_ROOT, [str(FORK_FLEET_JUST), "build-for-release"]),
    }
    if len(sys.argv) != 2 or sys.argv[1] not in actions:
        choices = ", ".join(sorted(actions))
        raise SystemExit(f"usage: {Path(sys.argv[0]).name} <{choices}>")

    cwd, command = actions[sys.argv[1]]
    tool_paths = [
        Path.home() / ".local/agent-shims",
        Path.home() / ".local/bin",
        Path.home() / ".asdf/shims",
        Path.home() / ".proto/bin",
        Path("/usr/local/sbin"),
        Path("/usr/local/bin"),
        Path("/usr/sbin"),
        Path("/usr/bin"),
        Path("/sbin"),
        Path("/bin"),
    ]
    environment = {
        "HOME": str(Path.home()),
        "PATH": os.pathsep.join(map(str, tool_paths)),
        "FORK_FLEET_NETWORK": os.environ.get("FORK_FLEET_NETWORK", "denied"),
        **rusty_v8_environment(),
    }
    os.chdir(cwd)
    os.execvpe(command[0], command, environment)


if __name__ == "__main__":
    main()
