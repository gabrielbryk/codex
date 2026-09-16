#!/usr/bin/env python3
"""Focused tests for the no-side-effect Fork Fleet workflow contract audit."""

import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).with_name("fork_fleet_validation.py")
SPEC = importlib.util.spec_from_file_location("fork_fleet_validation", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
validation = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(validation)


class UpgradeWorkflowAuditTest(unittest.TestCase):
    def write_repo(self, root: Path) -> None:
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
        for reference in validation.CANONICAL_REFERENCES:
            (references / reference).write_text("source-owned workflow reference\n")
        (root / "PATCHES.md").write_text(
            "\n".join(
                (
                    "# Fork patch manifest",
                    "Fork Fleet owns the logical patch mapping;",
                    "Current upstream target: `deadbeef`",
                    "## Maintained logical patches",
                    "| `retained-leaf` | release chore | rework | evidence |",
                    "## Per-upgrade verification",
                    "scripts/fork_fleet_validation.py",
                )
            )
        )

    def test_accepts_complete_source_owned_contract(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            self.write_repo(root)
            with patch.object(validation.subprocess, "run") as run:
                validation.validate_upgrade_workflow_contract(root)
            run.assert_not_called()

    def test_rejects_missing_canonical_reference_before_other_gates(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            self.write_repo(root)
            (root / ".codex/skills/upgrade-codex-fork/references/validation.md").unlink()
            with self.assertRaisesRegex(SystemExit, "canonical references: validation.md"):
                validation.validate_upgrade_workflow_contract(root)

    def test_rejects_retained_leaf_without_decision_or_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            self.write_repo(root)
            (root / "PATCHES.md").write_text(
                (root / "PATCHES.md").read_text().replace(
                    "| `retained-leaf` | release chore | rework | evidence |",
                    "| `retained-leaf` | release chore | unknown | |",
                )
            )
            with self.assertRaisesRegex(SystemExit, "incomplete patch evidence"):
                validation.validate_upgrade_workflow_contract(root)

    def test_rejects_duplicate_patch_ownership(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            self.write_repo(root)
            manifest = root / "PATCHES.md"
            manifest.write_text(
                manifest.read_text().replace(
                    "## Per-upgrade verification",
                    "| `retained-leaf` | release chore | apply | another evidence |\n"
                    "## Per-upgrade verification",
                )
            )
            with self.assertRaisesRegex(SystemExit, "duplicate patch ownership: retained-leaf"):
                validation.validate_upgrade_workflow_contract(root)


if __name__ == "__main__":
    unittest.main()
