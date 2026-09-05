import importlib.util
import shutil
import tempfile
import unittest
from pathlib import Path
from unittest import mock


SCRIPT = Path(__file__).with_name("fork_fleet_validation.py")
SPEC = importlib.util.spec_from_file_location("fork_fleet_validation", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
VALIDATION = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VALIDATION)


class UpgradeWorkflowAuditTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary_directory.cleanup)
        self.root = Path(self.temporary_directory.name)
        self.skill = self.root / "upgrade-codex-fork"
        shutil.copytree(VALIDATION.UPGRADE_SKILL, self.skill)

    def audit(self) -> None:
        VALIDATION.audit_upgrade_workflow(skill_root=self.skill)

    def assert_mutation_rejected(
        self, filename: str, marker: str, replacement: str
    ) -> None:
        path = self.skill / "references" / filename
        original = path.read_text(encoding="utf-8")
        mutated = original.replace(marker, replacement)
        self.assertNotEqual(original, mutated)
        path.write_text(mutated, encoding="utf-8")
        try:
            with self.assertRaises(SystemExit):
                self.audit()
        finally:
            path.write_text(original, encoding="utf-8")

    def test_accepts_canonical_repository_documents(self) -> None:
        self.audit()

    def test_rejects_reference_that_is_missing_and_unlinked(self) -> None:
        filename = "shipping-and-cutover.md"
        (self.skill / "references" / filename).unlink()
        skill_file = self.skill / "SKILL.md"
        skill_file.write_text(
            "\n".join(
                line
                for line in skill_file.read_text(encoding="utf-8").splitlines()
                if f"references/{filename}" not in line
            )
            + "\n",
            encoding="utf-8",
        )

        with self.assertRaisesRegex(SystemExit, "missing.*missing_files"):
            self.audit()

    def test_rejects_extra_reference_file(self) -> None:
        (self.skill / "references" / "extra.md").write_text(
            "# Unregistered reference\n", encoding="utf-8"
        )

        with self.assertRaisesRegex(SystemExit, "extra_files=.*extra.md"):
            self.audit()

    def test_rejects_contract_marker_spoofed_outside_required_section(self) -> None:
        planning = self.skill / "references" / "planning-and-reconciliation.md"
        planning.write_text(
            planning.read_text(encoding="utf-8").replace(
                "Generated-CLI freshness", "Generated CLI provenance"
            ),
            encoding="utf-8",
        )
        shipping = self.skill / "references" / "shipping-and-cutover.md"
        shipping.write_text(
            shipping.read_text(encoding="utf-8")
            + "\n<!-- spoof: Generated-CLI freshness -->\n",
            encoding="utf-8",
        )

        with self.assertRaisesRegex(SystemExit, "Generated-CLI freshness"):
            self.audit()

    def test_rejects_mutation_of_every_section_contract(self) -> None:
        checked = 0
        for filename, _heading, markers in VALIDATION.SECTION_CONTRACTS:
            for marker in markers:
                with self.subTest(filename=filename, marker=marker):
                    self.assert_mutation_rejected(filename, marker, "mutated contract")
                    checked += 1
        self.assertEqual(checked, 29)


class FormatBaselineTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary_directory.cleanup)
        self.root = Path(self.temporary_directory.name)
        (self.root / "PATCHES.md").write_text(
            "# Fork patch manifest\n\n"
            "- Target: `rust-v0.153.4` "
            "(`3d2ee51ca2d5db578f328aa75e20aa22c0197c9a`)\n",
            encoding="utf-8",
        )

    def test_reads_exact_target_sha_from_manifest(self) -> None:
        sha = "3d2ee51ca2d5db578f328aa75e20aa22c0197c9a"
        manifest = f"# Fork patch manifest\n\n- Target: `rust-v0.153.4` (`{sha}`)\n"

        self.assertEqual(VALIDATION.manifest_target_sha(manifest), sha)

    def test_rejects_manifest_without_exact_target_sha(self) -> None:
        with self.assertRaisesRegex(SystemExit, "exact target SHA"):
            VALIDATION.manifest_target_sha("- Target: `rust-v0.153.4`\n")

    def test_rejects_manifest_target_that_differs_from_history(self) -> None:
        manifest_sha = "3d2ee51ca2d5db578f328aa75e20aa22c0197c9a"
        history_sha = "90854393966b21e9ebfd21b122334eb09a20c93d"
        manifest = (
            f"# Fork patch manifest\n\n- Target: `rust-v0.153.4` (`{manifest_sha}`)\n"
        )

        with self.assertRaisesRegex(SystemExit, "history boundary"):
            VALIDATION.bound_manifest_target(manifest, history_sha, history_sha)

    def test_rejects_injected_target_that_differs_from_history(self) -> None:
        history_sha = "3d2ee51ca2d5db578f328aa75e20aa22c0197c9a"
        injected_sha = "90854393966b21e9ebfd21b122334eb09a20c93d"
        manifest = (
            f"# Fork patch manifest\n\n- Target: `rust-v0.153.4` (`{history_sha}`)\n"
        )

        with self.assertRaisesRegex(SystemExit, "Fork Fleet target SHA"):
            VALIDATION.bound_manifest_target(manifest, history_sha, injected_sha)

    def test_identifies_only_just_formatter_failure(self) -> None:
        output = "==> Just formatter failed\nFormatting failed: Just\n"

        self.assertEqual(VALIDATION.formatter_failures(output), {"Just"})

    def test_preserves_multiple_formatter_failures(self) -> None:
        output = "Formatting failed: Just, Rust, Python scripts\n"

        self.assertEqual(
            VALIDATION.formatter_failures(output),
            {"Just", "Rust", "Python scripts"},
        )

    def test_derives_target_from_first_non_fleet_non_rework_commit(self) -> None:
        fleet_commit = "1" * 40
        rework_commit = "2" * 40
        target_commit = "3" * 40
        check_output = mock.Mock(
            side_effect=[
                f"{fleet_commit}\n{rework_commit}\n{target_commit}\n",
                "fork-fleet@localhost\nprepare candidate\n",
                "gabe@example.com\nfeature\n\n"
                "Fork-Fleet-Rework: maintenance-target-bound-format-validation\n",
                "upstream@example.com\nupstream target\n",
            ]
        )

        with mock.patch.object(VALIDATION.subprocess, "check_output", check_output):
            self.assertEqual(VALIDATION.candidate_target_sha(), target_commit)

        self.assertEqual(check_output.call_count, 4)

    def test_format_validation_diffs_justfile_against_exact_target(self) -> None:
        target_sha = "3d2ee51ca2d5db578f328aa75e20aa22c0197c9a"
        completed = mock.Mock(returncode=7, stdout="Formatting failed: Rust\n")
        justfile_diff = mock.Mock(returncode=1)

        with (
            mock.patch.object(VALIDATION, "REPO_ROOT", self.root),
            mock.patch.object(
                VALIDATION, "candidate_target_sha", return_value=target_sha
            ),
            mock.patch.object(
                VALIDATION.subprocess,
                "run",
                side_effect=[completed, justfile_diff],
            ) as run,
        ):
            self.assertEqual(
                VALIDATION.run_format_validation(
                    Path("/toolchain/just"), {"PATH": "/bin"}, target_sha
                ),
                7,
            )

        self.assertEqual(
            run.call_args_list[1],
            mock.call(
                ["git", "diff", "--quiet", target_sha, "--", "justfile"],
                cwd=self.root,
                env={"PATH": "/bin"},
                check=False,
            ),
        )

    def test_format_validation_propagates_formatter_return_code(self) -> None:
        target_sha = "3d2ee51ca2d5db578f328aa75e20aa22c0197c9a"
        completed = mock.Mock(returncode=23, stdout="Formatting failed: Just, Rust\n")

        with (
            mock.patch.object(VALIDATION, "REPO_ROOT", self.root),
            mock.patch.object(
                VALIDATION, "candidate_target_sha", return_value=target_sha
            ),
            mock.patch.object(
                VALIDATION.subprocess,
                "run",
                side_effect=[completed, mock.Mock(returncode=0)],
            ),
        ):
            self.assertEqual(
                VALIDATION.run_format_validation(
                    Path("/toolchain/just"), {}, target_sha
                ),
                23,
            )

    def test_format_validation_accepts_only_just_on_exact_target_baseline(self) -> None:
        target_sha = "3d2ee51ca2d5db578f328aa75e20aa22c0197c9a"
        completed = mock.Mock(returncode=1, stdout="Formatting failed: Just\n")

        with (
            mock.patch.object(VALIDATION, "REPO_ROOT", self.root),
            mock.patch.object(
                VALIDATION, "candidate_target_sha", return_value=target_sha
            ),
            mock.patch.object(
                VALIDATION.subprocess,
                "run",
                side_effect=[completed, mock.Mock(returncode=0)],
            ),
        ):
            self.assertEqual(
                VALIDATION.run_format_validation(
                    Path("/toolchain/just"), {}, target_sha
                ),
                0,
            )

    def test_format_validation_rejects_just_when_justfile_differs_from_target(
        self,
    ) -> None:
        target_sha = "3d2ee51ca2d5db578f328aa75e20aa22c0197c9a"
        completed = mock.Mock(returncode=11, stdout="Formatting failed: Just\n")

        with (
            mock.patch.object(VALIDATION, "REPO_ROOT", self.root),
            mock.patch.object(
                VALIDATION, "candidate_target_sha", return_value=target_sha
            ),
            mock.patch.object(
                VALIDATION.subprocess,
                "run",
                side_effect=[completed, mock.Mock(returncode=1)],
            ),
        ):
            self.assertEqual(
                VALIDATION.run_format_validation(
                    Path("/toolchain/just"), {}, target_sha
                ),
                11,
            )


if __name__ == "__main__":
    unittest.main()
