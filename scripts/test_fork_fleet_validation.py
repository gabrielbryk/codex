#!/usr/bin/env python3
"""Focused tests for the no-side-effect Fork Fleet workflow contract audit."""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
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
            commit_sha
            for patch in patches
            for commit_sha in patch["commitShas"]
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
            (root / ".codex/skills/upgrade-codex-fork/references/validation.md").unlink()
            with self.assertRaisesRegex(SystemExit, "canonical references: validation.md"):
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
                (root / "PATCHES.md").read_text().replace(
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
            with self.assertRaisesRegex(SystemExit, "duplicate patch ownership: leaf-00"):
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
            with self.assertRaisesRegex(SystemExit, "duplicate source commit ownership"):
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
            line.split("|")[1].strip().strip("`"): line for line in table.splitlines()
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
        duplicated = f"{render.BEGIN_MARKER}\n{render.BEGIN_MARKER}\n{render.END_MARKER}"
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

            with patch.object(validation, "CODEX_RS", codex_rs), patch.dict(
                validation.os.environ, clear=False
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


if __name__ == "__main__":
    unittest.main()
