#!/usr/bin/env python3
"""Focused tests for the no-side-effect Fork Fleet workflow contract audit."""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("fork_fleet_validation.py")
SPEC = importlib.util.spec_from_file_location("fork_fleet_validation", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
validation = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(validation)

RENDER_SCRIPT = Path(__file__).with_name("render_patches_manifest.py")
RENDER_SPEC = importlib.util.spec_from_file_location(
    "render_patches_manifest", RENDER_SCRIPT
)
assert RENDER_SPEC is not None and RENDER_SPEC.loader is not None
render = importlib.util.module_from_spec(RENDER_SPEC)
RENDER_SPEC.loader.exec_module(render)

# Not load-bearing; only used to build a plausibly-sized fixture plan below.
REPRESENTATIVE_PATCH_COUNT = 45


class UpgradeWorkflowAuditTest(unittest.TestCase):
    def workflow_plan(self) -> dict[str, object]:
        target_sha = "a" * 40
        patches = []
        for index in range(REPRESENTATIVE_PATCH_COUNT):
            patch_id = f"leaf-{index:02d}"
            patches.append(
                {
                    "patchId": patch_id,
                    "commitShas": [f"{index:040x}"],
                    "dependsOn": [f"leaf-{index - 1:02d}"] if index else [],
                }
            )
        return {
            "targetSha": target_sha,
            "sourceCommitOrder": [patch["commitShas"][0] for patch in patches],
            "patches": patches,
        }

    def representative_plan(self) -> dict[str, object]:
        plan = self.workflow_plan()
        patches = plan["patches"]
        for leaf in patches:
            leaf["dependsOn"] = []
        patches[0]["patchId"] = "oauth-token-store-transaction"
        patches[0]["commitShas"] = []
        patches[1]["patchId"] = "oauth-reactive-authorization-retry"
        patches[1]["commitShas"] = []
        patches[1]["dependsOn"] = ["oauth-token-store-transaction"]
        patches[2]["dependsOn"] = ["oauth-reactive-authorization-retry"]
        patches[32]["patchId"] = "canonical-package"
        patches[32]["dependsOn"] = ["workflow-contract"]
        patches[37]["patchId"] = "workflow-contract"
        patches[37]["dependsOn"] = ["repo-workflow"]
        patches[41]["patchId"] = "repo-workflow"
        patches[41]["commitShas"] = []
        plan["sourceCommitOrder"] = [
            commit_sha for patch in patches for commit_sha in patch["commitShas"]
        ]
        return plan

    def write_repo(self, root: Path, test_plan: dict[str, object]) -> None:
        skill = root / ".codex/skills/upgrade-codex-fork"
        references = skill / "references"
        references.mkdir(parents=True)
        (skill / "SKILL.md").write_text(
            "\n".join(
                (
                    "---",
                    "name: upgrade-codex-fork",
                    "---",
                    "Fleet owns registry intent and candidates.",
                    "Never edit installed caches or rebase the maintained checkout.",
                    "## One run, one next action",
                )
            )
        )
        links = []
        for reference in validation.CANONICAL_REFERENCES:
            (references / reference).write_text("source-owned workflow reference\n")
            links.append(f"[reference](references/{reference})")
        (root / "PATCHES.md").write_text(
            "\n".join(
                (
                    "# Fork patch manifest",
                    "Fork Fleet owns the logical patch mapping;",
                    f"Current upstream target: `{test_plan['targetSha']}`",
                    "## Maintained logical patches",
                    *(
                        f"| `{patch['patchId']}` | release chore | rework | evidence |"
                        for patch in test_plan["patches"]
                    ),
                    "## Per-upgrade verification",
                    "scripts/fork_fleet_validation.py",
                )
            )
        )
        (skill / "SKILL.md").write_text(
            (skill / "SKILL.md").read_text() + "\n" + "\n".join(links)
        )

    def test_accepts_complete_source_owned_contract(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            test_plan = self.workflow_plan()
            self.write_repo(root, test_plan)
            with patch.object(validation.subprocess, "run") as run:
                validation.validate_upgrade_workflow_contract(root, test_plan)
            run.assert_not_called()

    def test_rejects_missing_canonical_reference_before_other_gates(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            test_plan = self.workflow_plan()
            self.write_repo(root, test_plan)
            (
                root / ".codex/skills/upgrade-codex-fork/references/validation.md"
            ).unlink()
            with self.assertRaisesRegex(
                SystemExit, "canonical references: validation.md"
            ):
                validation.validate_upgrade_workflow_contract(root, test_plan)

    def test_rejects_changed_canonical_reference_link(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            test_plan = self.workflow_plan()
            self.write_repo(root, test_plan)
            skill = root / ".codex/skills/upgrade-codex-fork/SKILL.md"
            skill.write_text(
                skill.read_text().replace(
                    "references/validation.md", "references/other-validation.md"
                )
            )
            with self.assertRaisesRegex(SystemExit, "canonical reference links"):
                validation.validate_upgrade_workflow_contract(root, test_plan)

    def test_rejects_retained_leaf_without_decision_or_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            test_plan = self.workflow_plan()
            self.write_repo(root, test_plan)
            (root / "PATCHES.md").write_text(
                (root / "PATCHES.md")
                .read_text()
                .replace(
                    "| `leaf-00` | release chore | rework | evidence |",
                    "| `leaf-00` | release chore | unknown | |",
                )
            )
            with self.assertRaisesRegex(SystemExit, "incomplete patch evidence"):
                validation.validate_upgrade_workflow_contract(root, test_plan)

    def test_rejects_duplicate_patch_ownership(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            test_plan = self.workflow_plan()
            self.write_repo(root, test_plan)
            manifest = root / "PATCHES.md"
            manifest.write_text(
                manifest.read_text().replace(
                    "## Per-upgrade verification",
                    "| `leaf-00` | release chore | apply | another evidence |\n"
                    "## Per-upgrade verification",
                )
            )
            with self.assertRaisesRegex(
                SystemExit, "duplicate patch ownership: leaf-00"
            ):
                validation.validate_upgrade_workflow_contract(root, test_plan)

    def test_rejects_stale_manifest_target_and_leaf_set(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            test_plan = self.workflow_plan()
            self.write_repo(root, test_plan)
            manifest = root / "PATCHES.md"
            manifest.write_text(
                manifest.read_text()
                .replace(str(test_plan["targetSha"]), "b" * 40)
                .replace("| `leaf-41` | release chore | rework | evidence |\n", "")
            )
            with self.assertRaisesRegex(SystemExit, "target does not match"):
                validation.validate_upgrade_workflow_contract(root, test_plan)

    def test_rejects_manifest_leaf_set_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            test_plan = self.workflow_plan()
            self.write_repo(root, test_plan)
            manifest = root / "PATCHES.md"
            manifest.write_text(
                manifest.read_text().replace(
                    "| `leaf-41` | release chore | rework | evidence |\n", ""
                )
            )
            with self.assertRaisesRegex(SystemExit, "leaves do not match"):
                validation.validate_upgrade_workflow_contract(root, test_plan)

    def test_rejects_duplicate_source_commit_ownership(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            test_plan = self.workflow_plan()
            self.write_repo(root, test_plan)
            patches = test_plan["patches"]
            patches[1]["commitShas"] = patches[0]["commitShas"]
            with self.assertRaisesRegex(
                SystemExit, "duplicate source commit ownership"
            ):
                validation.validate_upgrade_workflow_contract(root, test_plan)

    def test_rejects_unowned_source_commit(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            test_plan = self.workflow_plan()
            self.write_repo(root, test_plan)
            test_plan["sourceCommitOrder"].append("f" * 40)
            with self.assertRaisesRegex(SystemExit, "not owned exactly once"):
                validation.validate_upgrade_workflow_contract(root, test_plan)

    def test_rejects_dependency_out_of_order(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            test_plan = self.representative_plan()
            self.write_repo(root, test_plan)
            test_plan["patches"][41]["dependsOn"] = ["canonical-package"]
            with self.assertRaisesRegex(SystemExit, "dependency cycle"):
                validation.validate_upgrade_workflow_contract(root, test_plan)

    def test_accepts_source_free_leaves_and_non_topological_order(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            test_plan = self.representative_plan()
            self.write_repo(root, test_plan)
            validation.validate_upgrade_workflow_contract(root, test_plan)

    def test_audit_cli_reads_explicit_immutable_plan_without_subprocess(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            test_plan = self.representative_plan()
            self.write_repo(root, test_plan)
            plan_path = root / "fleet-plan.json"
            plan_path.write_text(json.dumps({"data": test_plan}))
            with (
                patch.object(validation, "REPO_ROOT", root),
                patch.object(
                    validation.sys,
                    "argv",
                    [str(SCRIPT), "upgrade-workflow-audit", str(plan_path)],
                ),
                patch.object(validation.subprocess, "run") as run,
            ):
                validation.main()
            run.assert_not_called()


class RenderPatchesManifestTest(unittest.TestCase):
    def fake_plan(self) -> dict[str, object]:
        return {
            "targetRef": "rust-v9.9.9",
            "targetSha": "a" * 40,
            "sourceSha": "b" * 40,
            "sourceBaseSha": "c" * 40,
            "patches": [
                {
                    "patchId": "zeta-patch",
                    "group": "fork feature",
                    "recommendation": "apply",
                    "decisionBasis": "bounded local behavior",
                },
                {
                    "patchId": "alpha-patch",
                    "group": "fork feature",
                    "recommendation": "apply",
                    "decisionBasis": "bounded local behavior",
                    "decisionReason": "no upstream equivalent",
                },
                {
                    "patchId": "beta-patch",
                    "maintenanceClass": "defensive",
                    "recommendation": "rework",
                    "decisionReason": "partial upstream coverage",
                    "adjudicationStatus": "confirmed",
                },
                {
                    "patchId": "gamma-patch",
                    "disposition": "upstream",
                    "recommendation": "apply",
                    "intent": "superseded by upstream retry logic",
                },
                {
                    "patchId": "delta-patch",
                    "recommendation": "drop",
                },
            ],
        }

    def test_rows_sorted_by_group_then_id(self) -> None:
        table = render.render_table(self.fake_plan()["patches"])
        rows = [line for line in table.splitlines() if line.startswith("| `")]
        patch_ids = [row.split("|")[1].strip().strip("`") for row in rows]
        self.assertEqual(
            patch_ids,
            ["alpha-patch", "zeta-patch", "beta-patch", "delta-patch", "gamma-patch"],
        )

    def test_fallback_chain_and_upstream_mapping(self) -> None:
        table = render.render_table(self.fake_plan()["patches"])
        rows = {
            line.split("|")[1].strip().strip("`"): line
            for line in table.splitlines()
            if line.startswith("| `")
        }
        self.assertIn("bounded local behavior", rows["zeta-patch"])
        self.assertIn(
            "bounded local behavior [no upstream equivalent]", rows["alpha-patch"]
        )
        self.assertIn("partial upstream coverage", rows["beta-patch"])
        self.assertNotIn("adjudication", rows["beta-patch"])
        self.assertIn("| drop |", rows["gamma-patch"])
        self.assertIn("no recorded basis", rows["delta-patch"])

    def test_ignores_volatile_plan_fields(self) -> None:
        plan_a = self.fake_plan()
        plan_b = self.fake_plan()
        plan_b["sourceSha"] = "d" * 40
        plan_b["patches"][2]["adjudicationStatus"] = "reopened"
        self.assertEqual(
            render.render_generated_region(plan_a),
            render.render_generated_region(plan_b),
        )

    def test_rendered_region_satisfies_manifest_and_test_plan_validation(self) -> None:
        plan = self.fake_plan()
        region = render.render_generated_region(plan)
        manifest_text = "\n".join(
            (
                "# Fork patch manifest",
                render.BEGIN_MARKER,
                region,
                render.END_MARKER,
            )
        )
        manifest_ids = validation.extract_manifest_patch_ids(manifest_text)
        self.assertEqual(
            set(manifest_ids),
            {"alpha-patch", "beta-patch", "gamma-patch", "delta-patch", "zeta-patch"},
        )

        patches = [
            {"patchId": patch_id, "commitShas": [f"{index:040x}"], "dependsOn": []}
            for index, patch_id in enumerate(sorted(manifest_ids))
        ]
        test_plan = {
            "targetSha": plan["targetSha"],
            "sourceCommitOrder": [patch["commitShas"][0] for patch in patches],
            "patches": patches,
        }
        validation.validate_test_plan(manifest_text, test_plan)

    def test_check_reports_diff_for_stale_region_and_zero_when_current(self) -> None:
        plan = self.fake_plan()
        region = render.render_generated_region(plan)
        base_text = "\n".join(
            ("# Fork patch manifest", render.BEGIN_MARKER, render.END_MARKER, "")
        )
        current_text = render.splice(base_text, region)
        self.assertEqual(render.check(current_text, region), 0)

        stale_text = current_text.replace("alpha-patch", "omega-patch")
        self.assertEqual(render.check(stale_text, region), 1)

    def test_splice_raises_on_missing_or_duplicate_markers(self) -> None:
        with self.assertRaises(SystemExit):
            render.splice("no markers here", "region")
        duplicated = (
            f"{render.BEGIN_MARKER}\n{render.BEGIN_MARKER}\n{render.END_MARKER}"
        )
        with self.assertRaises(SystemExit):
            render.splice(duplicated, "region")


class AttributionTest(unittest.TestCase):
    def plan_with_surfaces(self) -> dict[str, object]:
        return {
            "targetSha": "a" * 40,
            "patches": [
                {
                    "patchId": "owned-patch",
                    "ownedSurfaces": ["codex-rs/core/src/route_claim.rs"],
                },
                {
                    "patchId": "source-patch",
                    "sourceSurfaces": ["codex-rs/mcp/src"],
                },
                {
                    "patchId": "replacement-patch",
                    "replacementSurfaces": ["codex-rs/tui/src/recovery.rs"],
                },
                {
                    "patchId": "shared-patch",
                    "sharedSurfaces": ["."],
                },
                {
                    "patchId": "unused-patch",
                    "ownedSurfaces": ["codex-rs/never/touched"],
                },
            ],
        }

    def test_files_covered_by_each_surface_kind_pass(self) -> None:
        changed = [
            "codex-rs/core/src/route_claim.rs",
            "codex-rs/mcp/src/handler.rs",
            "codex-rs/tui/src/recovery.rs",
        ]
        validation.validate_attribution(
            Path("/repo"), self.plan_with_surfaces(), changed_paths=changed, env={}
        )

    def test_allowlist_paths_pass(self) -> None:
        validation.validate_attribution(
            Path("/repo"),
            self.plan_with_surfaces(),
            changed_paths=["PATCHES.md", "codex-rs/Cargo.lock"],
            env={},
        )

    def test_unattributed_path_fails_named(self) -> None:
        with self.assertRaisesRegex(
            SystemExit, "unattributed candidate files: codex-rs/unrelated.rs"
        ):
            validation.validate_attribution(
                Path("/repo"),
                self.plan_with_surfaces(),
                changed_paths=["codex-rs/unrelated.rs"],
                env={},
            )

    def test_surface_matching_nothing_only_warns(self) -> None:
        with patch("builtins.print") as printed:
            validation.validate_attribution(
                Path("/repo"),
                self.plan_with_surfaces(),
                changed_paths=["codex-rs/core/src/route_claim.rs"],
                env={},
            )
        warnings = [call.args[0] for call in printed.call_args_list]
        self.assertTrue(
            any("codex-rs/never/touched" in warning for warning in warnings)
        )

    def test_env_target_disagreeing_with_plan_fails_before_diff(self) -> None:
        with patch.object(validation.subprocess, "run") as run:
            with self.assertRaisesRegex(
                SystemExit, "attribution target disagrees with the immutable plan"
            ):
                validation.validate_attribution(
                    Path("/repo"),
                    self.plan_with_surfaces(),
                    env={"FORK_FLEET_TARGET_SHA": "f" * 40},
                )
        run.assert_not_called()


class ManifestCountTest(unittest.TestCase):
    def test_validate_test_plan_accepts_arbitrary_leaf_count(self) -> None:
        target_sha = "d" * 40
        patch_ids = ["only-leaf-one", "only-leaf-two", "only-leaf-three"]
        manifest_text = "\n".join(
            (
                "# Fork patch manifest",
                f"Current upstream target: `{target_sha}`",
                *(
                    f"| `{patch_id}` | release chore | apply | evidence |"
                    for patch_id in patch_ids
                ),
            )
        )
        patches = [
            {"patchId": patch_id, "commitShas": [f"{index:040x}"], "dependsOn": []}
            for index, patch_id in enumerate(patch_ids)
        ]
        test_plan = {
            "targetSha": target_sha,
            "sourceCommitOrder": [patch["commitShas"][0] for patch in patches],
            "patches": patches,
        }
        validation.validate_test_plan(manifest_text, test_plan)


class RustToolchainEnvironmentTest(unittest.TestCase):
    def _fixture(self, tmp: str, channel: str = "1.95.0") -> tuple[Path, Path, Path]:
        root = Path(tmp)
        codex_rs = root / "codex-rs"
        codex_rs.mkdir()
        (codex_rs / "rust-toolchain.toml").write_text(
            f'[toolchain]\nchannel = "{channel}"\ncomponents = ["clippy"]\n'
        )
        real_home = root / "home"
        real_home.mkdir()
        return root, codex_rs, real_home

    def test_path_is_prefixed_even_with_empty_inherited_environment(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            _, codex_rs, real_home = self._fixture(tmp)
            toolchain_bin = (
                real_home / ".rustup/toolchains/1.95.0-x86_64-unknown-linux-gnu/bin"
            )
            toolchain_bin.mkdir(parents=True)
            cargo_bin = real_home / ".cargo/bin"
            cargo_bin.mkdir(parents=True)

            with (
                patch.object(validation, "CODEX_RS", codex_rs),
                patch.dict(validation.os.environ, clear=False),
            ):
                for key in ("HOME", "RUSTUP_HOME", "CARGO_HOME"):
                    validation.os.environ.pop(key, None)
                env = validation.rust_toolchain_environment(real_home, env={})

            path_entries = env["PATH"].split(":")
            self.assertEqual(path_entries[0], str(toolchain_bin))
            self.assertIn(str(cargo_bin), path_entries)
            self.assertIn("/usr/local/bin", path_entries)
            self.assertIn("/usr/bin", path_entries)
            self.assertIn("/bin", path_entries)
            self.assertEqual(env["RUSTUP_TOOLCHAIN"], "1.95.0-x86_64-unknown-linux-gnu")
            self.assertEqual(env["RUSTUP_HOME"], str(real_home / ".rustup"))
            self.assertEqual(env["CARGO_HOME"], str(real_home / ".cargo"))
            self.assertEqual(env["HOME"], str(real_home))

    def test_existing_env_values_are_preserved_not_overwritten(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            _, codex_rs, real_home = self._fixture(tmp)
            toolchain_bin = (
                real_home / ".rustup/toolchains/1.95.0-x86_64-unknown-linux-gnu/bin"
            )
            toolchain_bin.mkdir(parents=True)
            sanitized_home = real_home / "sanitized"
            sanitized_home.mkdir()

            with patch.object(validation, "CODEX_RS", codex_rs):
                env = validation.rust_toolchain_environment(
                    real_home,
                    env={"HOME": str(sanitized_home), "PATH": "/existing/tool/dir"},
                )

            self.assertEqual(env["HOME"], str(sanitized_home))
            path_entries = env["PATH"].split(":")
            self.assertEqual(path_entries[0], str(toolchain_bin))
            self.assertEqual(path_entries[-1], "/existing/tool/dir")

    def test_missing_toolchain_bin_raises(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            _, codex_rs, real_home = self._fixture(tmp, channel="9.9.9")

            with patch.object(validation, "CODEX_RS", codex_rs):
                with self.assertRaisesRegex(
                    SystemExit, "pinned rustup toolchain bin directory is missing"
                ):
                    validation.rust_toolchain_environment(real_home, env={})

    def test_path_includes_uv_and_toolchain_just_bins(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            _, codex_rs, real_home = self._fixture(tmp)
            (
                real_home / ".rustup/toolchains/1.95.0-x86_64-unknown-linux-gnu/bin"
            ).mkdir(parents=True)
            local_bin = real_home / ".local/bin"
            local_bin.mkdir(parents=True)
            toolchain_just_bin = (
                real_home / ".local/share/fork-fleet/toolchains/just-1.51.0/bin"
            )
            toolchain_just_bin.mkdir(parents=True)

            with patch.object(validation, "CODEX_RS", codex_rs):
                env = validation.rust_toolchain_environment(real_home, env={})

            path_entries = env["PATH"].split(":")
            self.assertIn(str(local_bin), path_entries)
            self.assertIn(str(toolchain_just_bin), path_entries)
            self.assertEqual(env["UV_PYTHON"], "3.13")

    def test_just_bin_override_takes_precedence_over_toolchain_default(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            _, codex_rs, real_home = self._fixture(tmp)
            (
                real_home / ".rustup/toolchains/1.95.0-x86_64-unknown-linux-gnu/bin"
            ).mkdir(parents=True)
            default_just_bin = (
                real_home / ".local/share/fork-fleet/toolchains/just-1.51.0/bin"
            )
            default_just_bin.mkdir(parents=True)
            override_bin = Path(tmp) / "custom-just-bin"
            override_bin.mkdir()

            with patch.object(validation, "CODEX_RS", codex_rs):
                env = validation.rust_toolchain_environment(
                    real_home, env={"JUST_BIN": str(override_bin)}
                )

            path_entries = env["PATH"].split(":")
            self.assertIn(str(override_bin), path_entries)
            self.assertNotIn(str(default_just_bin), path_entries)

    def test_uv_python_not_overwritten_when_already_set(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            _, codex_rs, real_home = self._fixture(tmp)
            (
                real_home / ".rustup/toolchains/1.95.0-x86_64-unknown-linux-gnu/bin"
            ).mkdir(parents=True)

            with patch.object(validation, "CODEX_RS", codex_rs):
                env = validation.rust_toolchain_environment(
                    real_home, env={"UV_PYTHON": "3.11"}
                )

            self.assertEqual(env["UV_PYTHON"], "3.11")


class WorkspaceTestPackageArgsTest(unittest.TestCase):
    def test_package_args_are_dash_p_flag_pairs(self) -> None:
        args = validation.WORKSPACE_TEST_PACKAGE_ARGS
        self.assertEqual(len(args) % 2, 0)
        for flag in args[0::2]:
            self.assertEqual(flag, "-p")
        self.assertIn("codex-tui", args)
        self.assertIn("codex-core", args)
        self.assertIn("codex-app-server", args)


class NextestJunitFailureParsingTest(unittest.TestCase):
    def test_parses_failures_and_errors_across_testsuites(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            junit_path = Path(tmp) / "junit.xml"
            junit_path.write_text(
                """<?xml version="1.0"?>
<testsuites>
  <testsuite name="codex-core">
    <testcase name="test_pass"/>
    <testcase name="test_fail"><failure message="boom"/></testcase>
  </testsuite>
  <testsuite name="codex-tui">
    <testcase name="test_error"><error message="panic"/></testcase>
  </testsuite>
</testsuites>
"""
            )
            failures = validation.parse_nextest_junit_failures(junit_path)
            self.assertEqual(
                failures, {"codex-core::test_fail", "codex-tui::test_error"}
            )

    def test_missing_junit_returns_empty_set(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            failures = validation.parse_nextest_junit_failures(
                Path(tmp) / "missing.xml"
            )
            self.assertEqual(failures, set())


class WorkspaceTestsFailuresSidecarTest(unittest.TestCase):
    def test_default_path_is_under_the_fork_fleet_nextest_profile_dir(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            codex_rs = Path(tmp) / "codex-rs"
            with (
                patch.object(validation, "CODEX_RS", codex_rs),
                patch.dict(validation.os.environ, clear=False),
            ):
                validation.os.environ.pop("FORK_FLEET_ARTIFACT_DIR", None)
                path = validation.workspace_tests_failures_path()
            self.assertEqual(
                path,
                codex_rs / "target/nextest/fork-fleet/workspace-tests-failures.json",
            )

    def test_artifact_dir_env_overrides_default_path(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            with patch.dict(
                validation.os.environ,
                {"FORK_FLEET_ARTIFACT_DIR": tmp},
                clear=False,
            ):
                path = validation.workspace_tests_failures_path()
            self.assertEqual(path, Path(tmp) / "workspace-tests-failures.json")

    def test_write_sorts_failures_and_creates_parent_dirs(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            with patch.dict(
                validation.os.environ,
                {"FORK_FLEET_ARTIFACT_DIR": tmp},
                clear=False,
            ):
                output_path = validation.write_workspace_tests_failures(
                    {"codex-tui::z", "codex-core::a"}
                )
            self.assertEqual(
                json.loads(output_path.read_text()),
                ["codex-core::a", "codex-tui::z"],
            )


class RunWorkspaceTestsTest(unittest.TestCase):
    def test_build_failure_raises_immediately_without_writing_a_sidecar(self) -> None:
        commands = [
            (Path("/repo/codex-rs"), ["cargo", "build", "-p", "codex-code-mode-host"]),
            (Path("/repo"), ["just", "test", "--profile", "fork-fleet"]),
        ]
        with tempfile.TemporaryDirectory() as tmp:
            with (
                patch.object(validation.subprocess, "run") as run,
                patch.dict(
                    validation.os.environ, {"FORK_FLEET_ARTIFACT_DIR": tmp}, clear=False
                ),
            ):
                run.return_value = SimpleNamespace(returncode=1)
                with self.assertRaises(SystemExit) as ctx:
                    validation.run_workspace_tests(commands, {})
            self.assertEqual(ctx.exception.code, 1)
            run.assert_called_once()
            self.assertFalse((Path(tmp) / "workspace-tests-failures.json").exists())

    def test_aggregates_failures_across_invocations_before_one_overwrites_junit(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            codex_rs = Path(tmp) / "codex-rs"
            junit_path = codex_rs / "target/nextest/fork-fleet/junit.xml"
            commands = [
                (Path(tmp), ["cargo", "build", "-p", "codex-code-mode-host"]),
                (
                    Path(tmp),
                    ["just", "test", "--profile", "fork-fleet", "-p", "codex-core"],
                ),
                (
                    Path(tmp),
                    ["just", "test", "--profile", "fork-fleet", "-p", "codex-v8-poc"],
                ),
            ]
            call_count = 0

            def fake_run(command, cwd, env, check):
                nonlocal call_count
                call_count += 1
                if command[0] == "cargo":
                    return SimpleNamespace(returncode=0)
                junit_path.parent.mkdir(parents=True, exist_ok=True)
                if "codex-core" in command:
                    junit_path.write_text(
                        '<testsuites><testsuite name="codex-core">'
                        '<testcase name="test_a"><failure/></testcase>'
                        "</testsuite></testsuites>"
                    )
                    return SimpleNamespace(returncode=100)
                junit_path.write_text(
                    '<testsuites><testsuite name="codex-v8-poc">'
                    '<testcase name="test_b"/>'
                    "</testsuite></testsuites>"
                )
                return SimpleNamespace(returncode=0)

            with (
                patch.object(validation, "CODEX_RS", codex_rs),
                patch.object(validation.subprocess, "run", side_effect=fake_run),
                patch.dict(
                    validation.os.environ, {"FORK_FLEET_ARTIFACT_DIR": tmp}, clear=False
                ),
            ):
                with self.assertRaises(SystemExit) as ctx:
                    validation.run_workspace_tests(commands, {})

            self.assertEqual(ctx.exception.code, 100)
            self.assertEqual(call_count, 3)
            sidecar = Path(tmp) / "workspace-tests-failures.json"
            self.assertEqual(json.loads(sidecar.read_text()), ["codex-core::test_a"])


class OnlySelectorValidationTest(unittest.TestCase):
    def test_accepts_well_formed_selector(self) -> None:
        validation.validate_only_selector("codex-core::route_claim::tests::case_a")

    def test_rejects_selector_without_test_name(self) -> None:
        with self.assertRaisesRegex(SystemExit, "invalid --only selector"):
            validation.validate_only_selector("codex-core")

    def test_rejects_shell_metacharacters_in_the_binary_id_segment(self) -> None:
        # The contract's whitelist (`^[A-Za-z0-9_:./-]+::.+$`) only restricts
        # the binary-id segment before `::`; commands are exec'd as argv
        # lists (never a shell), so the free-form test-name segment after
        # `::` is not itself a shell-injection surface.
        for selector in (
            "codex$(whoami)::case",
            "codex-core; rm -rf /::case",
            "codex-core`id`::case",
            "codex-core && echo hi::case",
        ):
            with self.assertRaisesRegex(SystemExit, "invalid --only selector"):
                validation.validate_only_selector(selector)


class DiscoverOnlyBinaryIdsTest(unittest.TestCase):
    def write_crate(
        self,
        root: Path,
        name: str,
        *,
        bins: list[str] | None = None,
        has_lib: bool = True,
        test_files: list[str] | None = None,
    ) -> None:
        crate_dir = root / name
        (crate_dir / "src").mkdir(parents=True)
        lines = [
            "[package]",
            f'name = "{name}"',
            'version = "0.1.0"',
            "",
        ]
        for bin_name in bins or []:
            lines += ["[[bin]]", f'name = "{bin_name}"', 'path = "src/bin/x.rs"', ""]
        (crate_dir / "Cargo.toml").write_text("\n".join(lines))
        if has_lib:
            (crate_dir / "src" / "lib.rs").write_text("")
        if test_files:
            tests_dir = crate_dir / "tests"
            tests_dir.mkdir()
            for test_file in test_files:
                (tests_dir / f"{test_file}.rs").write_text("")

    def test_maps_lib_bin_and_integration_test_binary_ids(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            codex_rs = Path(tmp)
            self.write_crate(
                codex_rs,
                "codex-app-server",
                bins=["codex-app-server", "exec-server"],
                test_files=["all"],
            )
            self.write_crate(codex_rs, "codex-core", test_files=["all", "extra"])
            self.write_crate(codex_rs, "codex-v8-poc", has_lib=True)

            binary_ids = validation.discover_only_binary_ids(
                codex_rs, ["codex-app-server", "codex-core", "codex-v8-poc"]
            )

            self.assertEqual(
                binary_ids["codex-app-server"], ("codex-app-server", "lib", None)
            )
            self.assertEqual(
                binary_ids["codex-app-server::bin/exec-server"],
                ("codex-app-server", "bin", "exec-server"),
            )
            self.assertEqual(
                binary_ids["codex-app-server::all"],
                ("codex-app-server", "test", "all"),
            )
            self.assertEqual(binary_ids["codex-core"], ("codex-core", "lib", None))
            self.assertEqual(
                binary_ids["codex-core::all"], ("codex-core", "test", "all")
            )
            self.assertEqual(
                binary_ids["codex-core::extra"], ("codex-core", "test", "extra")
            )
            self.assertEqual(binary_ids["codex-v8-poc"], ("codex-v8-poc", "lib", None))

    def test_missing_package_is_skipped(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            codex_rs = Path(tmp)
            binary_ids = validation.discover_only_binary_ids(
                codex_rs, ["codex-never-registered"]
            )
            self.assertEqual(binary_ids, {})


class ResolveOnlySelectorTest(unittest.TestCase):
    def binary_ids(self) -> dict[str, tuple[str, str, str | None]]:
        return {
            "codex-core": ("codex-core", "lib", None),
            "codex-core::all": ("codex-core", "test", "all"),
            "codex-app-server::bin/exec-server": (
                "codex-app-server",
                "bin",
                "exec-server",
            ),
        }

    def test_prefers_longest_matching_binary_id(self) -> None:
        result = validation.resolve_only_selector(
            "codex-core::all::mod::case", self.binary_ids()
        )
        self.assertEqual(result, ("codex-core", "test", "all", "mod::case"))

    def test_falls_back_to_lib_binary_id(self) -> None:
        result = validation.resolve_only_selector(
            "codex-core::route_claim::tests::case_a", self.binary_ids()
        )
        self.assertEqual(
            result, ("codex-core", "lib", None, "route_claim::tests::case_a")
        )

    def test_resolves_bin_binary_id(self) -> None:
        result = validation.resolve_only_selector(
            "codex-app-server::bin/exec-server::smoke_test", self.binary_ids()
        )
        self.assertEqual(
            result,
            ("codex-app-server", "bin", "exec-server", "smoke_test"),
        )

    def test_rejects_unknown_binary_id(self) -> None:
        with self.assertRaisesRegex(SystemExit, "does not match a known test binary"):
            validation.resolve_only_selector("codex-unknown::case", self.binary_ids())


class OnlyNextestCommandsTest(unittest.TestCase):
    def test_builds_expected_command_for_single_selector(self) -> None:
        binary_ids = {"codex-core": ("codex-core", "lib", None)}
        commands = validation.only_nextest_commands(
            ["codex-core::route_claim::tests::case_a"], binary_ids, retries=0
        )
        self.assertEqual(len(commands), 1)
        cwd, command = commands[0]
        self.assertEqual(cwd, validation.CODEX_RS)
        self.assertEqual(
            command,
            [
                "cargo",
                "nextest",
                "run",
                "--profile",
                "fork-fleet",
                "-p",
                "codex-core",
                "--lib",
                "-E",
                "test(=route_claim::tests::case_a)",
                "--retries",
                "0",
            ],
        )

    def test_groups_multiple_selectors_in_same_binary_into_one_filter(self) -> None:
        binary_ids = {"codex-core::all": ("codex-core", "test", "all")}
        commands = validation.only_nextest_commands(
            [
                "codex-core::all::case_a",
                "codex-core::all::case_b",
            ],
            binary_ids,
            retries=2,
        )
        self.assertEqual(len(commands), 1)
        _, command = commands[0]
        self.assertEqual(
            command,
            [
                "cargo",
                "nextest",
                "run",
                "--profile",
                "fork-fleet",
                "-p",
                "codex-core",
                "--test",
                "all",
                "-E",
                "test(=case_a) | test(=case_b)",
                "--retries",
                "2",
            ],
        )

    def test_v8_poc_gets_sandbox_feature_flag(self) -> None:
        binary_ids = {"codex-v8-poc": ("codex-v8-poc", "lib", None)}
        commands = validation.only_nextest_commands(
            ["codex-v8-poc::case_a"], binary_ids, retries=0
        )
        _, command = commands[0]
        self.assertEqual(
            command,
            [
                "cargo",
                "nextest",
                "run",
                "--profile",
                "fork-fleet",
                "-p",
                "codex-v8-poc",
                "--lib",
                "--features",
                "sandbox",
                "-E",
                "test(=case_a)",
                "--retries",
                "0",
            ],
        )

    def test_separate_binaries_produce_separate_commands_in_order(self) -> None:
        binary_ids = {
            "codex-core": ("codex-core", "lib", None),
            "codex-tui": ("codex-tui", "lib", None),
        }
        commands = validation.only_nextest_commands(
            ["codex-tui::case_a", "codex-core::case_b"], binary_ids, retries=0
        )
        packages = [command[6] for _, command in commands]
        self.assertEqual(packages, ["codex-tui", "codex-core"])


class ParseWorkspaceTestsArgsTest(unittest.TestCase):
    def test_no_args_returns_empty_defaults(self) -> None:
        self.assertEqual(validation.parse_workspace_tests_args([]), ([], None))

    def test_collects_repeated_only_and_parses_retries(self) -> None:
        only, retries = validation.parse_workspace_tests_args(
            [
                "--only",
                "codex-core::case_a",
                "--only",
                "codex-tui::case_b",
                "--retries",
                "3",
            ]
        )
        self.assertEqual(only, ["codex-core::case_a", "codex-tui::case_b"])
        self.assertEqual(retries, 3)

    def test_rejects_invalid_only_selector(self) -> None:
        with self.assertRaisesRegex(SystemExit, "invalid --only selector"):
            validation.parse_workspace_tests_args(["--only", "no-separator"])

    def test_rejects_non_integer_retries(self) -> None:
        with self.assertRaisesRegex(SystemExit, "--retries must be an integer"):
            validation.parse_workspace_tests_args(["--retries", "abc"])

    def test_rejects_dangling_flag_without_value(self) -> None:
        with self.assertRaisesRegex(SystemExit, "--only requires a value"):
            validation.parse_workspace_tests_args(["--only"])

    def test_rejects_unrecognized_argument(self) -> None:
        with self.assertRaisesRegex(SystemExit, "unrecognized argument"):
            validation.parse_workspace_tests_args(["--bogus"])


class RunWorkspaceTestsOnlySidecarTest(unittest.TestCase):
    def test_only_run_sidecar_contains_only_the_selected_failure(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            codex_rs = Path(tmp) / "codex-rs"
            junit_path = codex_rs / "target/nextest/fork-fleet/junit.xml"
            commands = [
                (Path(tmp), ["cargo", "build", "-p", "codex-code-mode-host"]),
                (
                    codex_rs,
                    [
                        "cargo",
                        "nextest",
                        "run",
                        "--profile",
                        "fork-fleet",
                        "-p",
                        "codex-core",
                        "--lib",
                        "-E",
                        "test(=route_claim::tests::case_a)",
                        "--retries",
                        "0",
                    ],
                ),
            ]

            def fake_run(command, cwd, env, check):
                if command[0] == "cargo" and "nextest" not in command:
                    return SimpleNamespace(returncode=0)
                junit_path.parent.mkdir(parents=True, exist_ok=True)
                junit_path.write_text(
                    '<testsuites><testsuite name="codex-core">'
                    '<testcase name="route_claim::tests::case_a"><failure/></testcase>'
                    "</testsuite></testsuites>"
                )
                return SimpleNamespace(returncode=100)

            with (
                patch.object(validation, "CODEX_RS", codex_rs),
                patch.object(validation.subprocess, "run", side_effect=fake_run),
                patch.dict(
                    validation.os.environ, {"FORK_FLEET_ARTIFACT_DIR": tmp}, clear=False
                ),
            ):
                with self.assertRaises(SystemExit) as ctx:
                    validation.run_workspace_tests(commands, {})

            self.assertEqual(ctx.exception.code, 100)
            sidecar = Path(tmp) / "workspace-tests-failures.json"
            self.assertEqual(
                json.loads(sidecar.read_text()),
                ["codex-core::route_claim::tests::case_a"],
            )


if __name__ == "__main__":
    unittest.main()
