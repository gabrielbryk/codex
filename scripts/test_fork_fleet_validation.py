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


class UpgradeWorkflowAuditTest(unittest.TestCase):
    def workflow_plan(self) -> dict[str, object]:
        target_sha = "a" * 40
        patches = []
        for index in range(validation.EXPECTED_PATCH_COUNT):
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


if __name__ == "__main__":
    unittest.main()
