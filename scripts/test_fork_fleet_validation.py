import importlib.util
import os
import shutil
import sys
import tempfile
import unittest
from contextlib import contextmanager
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
        self.assertEqual(checked, 60)


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


class DifferentialWorkspaceTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary_directory.cleanup)
        self.root = Path(self.temporary_directory.name) / "candidate"
        self.root.mkdir()
        self.target = Path(self.temporary_directory.name) / "target"
        self.target.mkdir()
        self.command = VALIDATION.WorkspaceCommand(
            cwd=self.root / "codex-rs",
            argv=("just", "test", "--workspace", "--locked"),
        )
        self.snapshot = ("candidate-head", "candidate-tree", "")
        self.target_snapshot = ("a" * 40, "target-tree", "")
        self.target_factory_calls: list[str] = []
        self.target_preparer = mock.Mock(return_value=self.target_snapshot)

    @contextmanager
    def target_factory(self, target_sha: str):
        self.target_factory_calls.append(target_sha)
        yield self.target

    @staticmethod
    def result(
        returncode: int,
        *failed_tests: str,
        parseable: bool = True,
        truncated: bool = False,
        timed_out_tests: tuple[str, ...] = (),
    ) -> object:
        return VALIDATION.CommandResult(
            returncode=returncode,
            failed_tests=frozenset(failed_tests),
            declared_failed_count=len(failed_tests) if parseable else None,
            timed_out_tests=frozenset(timed_out_tests),
            declared_timed_out_count=(len(timed_out_tests) if parseable else None),
            summary_count=1 if parseable else 0,
            excerpt="bounded output",
            parse_error=(
                None if parseable else "expected one terminal nextest summary, found 0"
            ),
            truncated=truncated,
        )

    def execute_commands(
        self,
        commands: tuple[object, ...],
        runner: mock.Mock,
        *,
        target_factory=None,
        candidate_snapshots=None,
        target_snapshots=None,
    ) -> int:
        with (
            mock.patch.object(VALIDATION, "REPO_ROOT", self.root),
            mock.patch.object(
                VALIDATION,
                "candidate_snapshot",
                side_effect=candidate_snapshots or [self.snapshot, self.snapshot],
            ),
            mock.patch.object(
                VALIDATION,
                "repository_snapshot",
                side_effect=target_snapshots,
                return_value=self.target_snapshot,
            ),
            mock.patch.object(
                VALIDATION, "bounded_target_delta", return_value="bounded delta"
            ),
        ):
            return VALIDATION.run_differential_workspace_tests(
                commands,
                {"PATH": "/bin", "INSTA_UPDATE": "always"},
                "a" * 40,
                runner=runner,
                target_factory=target_factory or self.target_factory,
                target_preparer=self.target_preparer,
            )

    def execute_gate(self, runner: mock.Mock) -> int:
        return self.execute_commands((self.command,), runner)

    def test_candidate_pass_skips_exact_target(self) -> None:
        runner = mock.Mock(return_value=self.result(0))

        self.assertEqual(self.execute_gate(runner), 0)

        self.assertEqual(self.target_factory_calls, [])
        self.assertEqual(runner.call_count, 1)
        self.assertEqual(runner.call_args.args[1]["INSTA_UPDATE"], "no")
        self.assertIn("CARGO_TARGET_DIR", runner.call_args.args[1])

    def test_parser_uses_only_terminal_failures_after_summary(self) -> None:
        output = """\
  TRY 1 FAIL [ 0.100s] (1/2) crate test::eventual_flake
────────────
     Summary [ 1.000s] 2 tests run: 1 passed (1 flaky), 1 failed
   FLAKY 2/2 [ 0.100s] (1/2) crate test::eventual_flake
  TRY 2 FAIL [ 0.200s] (2/2) crate test::terminal_failure
"""

        self.assertEqual(
            VALIDATION.parse_nextest_output(output),
            (
                frozenset({"crate test::terminal_failure"}),
                1,
                frozenset(),
                0,
                1,
                None,
            ),
        )

    def test_parser_rejects_ambiguous_terminal_formats(self) -> None:
        count_mismatch = """\
     Summary [ 1.000s] 2 tests run: 0 passed, 2 failed
  TRY 2 FAIL [ 0.200s] (2/2) crate test::only_parsed_failure
"""
        multiple_summaries = """\
     Summary [ 1.000s] 1 test run: 1 passed
     Summary [ 1.000s] 1 test run: 0 passed, 1 failed
  TRY 2 FAIL [ 0.200s] (1/1) crate test::failure
"""
        unknown_status = """\
     Summary [ 1.000s] 1 test run: 0 passed, 1 failed
  XFAIL [ 0.200s] (1/1) crate test::failure
"""
        cases = (
            (count_mismatch, "declared 2 failures"),
            (multiple_summaries, "expected one terminal nextest summary"),
            (unknown_status, "unknown terminal nextest failure status"),
        )
        for output, expected_error in cases:
            with self.subTest(expected_error=expected_error):
                self.assertIn(
                    expected_error, VALIDATION.parse_nextest_output(output)[5]
                )

    def test_parser_accepts_mixed_fail_and_timeout_terminal_statuses(self) -> None:
        ordinary_failures = "\n".join(
            f"        FAIL [ 0.100s] ({index}/63) crate test::failure_{index}"
            for index in range(1, 63)
        )
        output = f"""\
     Summary [ 1.000s] 64 tests run: 0 passed, 63 failed, 1 timed out
{ordinary_failures}
 TRY 2 FL+LK [ 0.200s] (63/63) crate test::observed_fail_and_leak
   TRY 2 TMT [ 60.000s] (64/64) crate test::observed_timeout
"""

        (
            failed_tests,
            declared_failed_count,
            timed_out_tests,
            declared_timed_out_count,
            summary_count,
            parse_error,
        ) = VALIDATION.parse_nextest_output(output)
        self.assertEqual(declared_failed_count, 63)
        self.assertEqual(declared_timed_out_count, 1)
        self.assertEqual(summary_count, 1)
        self.assertEqual(len(failed_tests), 63)
        self.assertIn("crate test::observed_fail_and_leak", failed_tests)
        self.assertEqual(timed_out_tests, {"crate test::observed_timeout"})
        self.assertIsNone(parse_error)

    def test_parser_rejects_timed_out_count_mismatch(self) -> None:
        output = """\
     Summary [ 1.000s] 2 tests run: 0 passed, 2 timed out
   TRY 2 TMT [ 60.000s] (1/2) crate test::only_timeout
"""

        self.assertIn(
            "declared 2 timed out",
            VALIDATION.parse_nextest_output(output)[5],
        )

    def test_accepts_candidate_failures_that_are_subset_of_target(self) -> None:
        runner = mock.Mock(
            side_effect=[
                self.result(100, "crate test::shared_failure"),
                self.result(
                    100,
                    "crate test::shared_failure",
                    "crate test::target_only_failure",
                ),
            ]
        )

        self.assertEqual(self.execute_gate(runner), 0)

        self.assertEqual(self.target_factory_calls, ["a" * 40])
        self.assertEqual(runner.call_args_list[1].args[0].cwd, self.target / "codex-rs")
        self.assertEqual(
            runner.call_args_list[0].args[0].argv,
            runner.call_args_list[1].args[0].argv,
        )
        candidate_environment = runner.call_args_list[0].args[1]
        target_environment = runner.call_args_list[1].args[1]
        for variable in ("HOME", "TMPDIR", "CARGO_TARGET_DIR"):
            self.assertNotEqual(
                candidate_environment[variable], target_environment[variable]
            )

    def test_accepts_candidate_timeout_subset_of_target_timeouts(self) -> None:
        runner = mock.Mock(
            side_effect=[
                self.result(
                    100,
                    timed_out_tests=("crate test::shared_timeout",),
                ),
                self.result(
                    100,
                    timed_out_tests=(
                        "crate test::shared_timeout",
                        "crate test::target_only_timeout",
                    ),
                ),
            ]
        )

        self.assertEqual(self.execute_gate(runner), 0)

    def test_rejects_candidate_only_timeout(self) -> None:
        runner = mock.Mock(
            side_effect=[
                self.result(
                    100,
                    timed_out_tests=("crate test::candidate_timeout",),
                ),
                self.result(
                    100,
                    timed_out_tests=("crate test::upstream_timeout",),
                ),
            ]
        )

        self.assertEqual(self.execute_gate(runner), 1)

    def test_rejects_timeout_covered_only_by_target_failure(self) -> None:
        test_name = "crate test::category_mismatch"
        runner = mock.Mock(
            side_effect=[
                self.result(100, timed_out_tests=(test_name,)),
                self.result(100, test_name),
            ]
        )

        self.assertEqual(self.execute_gate(runner), 1)

    def test_accepts_only_local_workspace_version_lock_restamps(self) -> None:
        before_lock = """\
version = 4

[[package]]
name = "local-package"
version = "0.0.0"
dependencies = ["dep"]
"""
        after_lock = before_lock.replace('version = "0.0.0"', 'version = "0.153.4"')
        initial = ("a" * 40, "tree", "")
        prepared = ("a" * 40, "tree", " M codex-rs/Cargo.lock\n")

        self.assertEqual(
            VALIDATION.validate_target_lock_normalization(
                initial,
                prepared,
                before_lock,
                after_lock,
                "0.153.4",
                frozenset({"local-package"}),
            ),
            1,
        )

    def test_rejects_unexpected_lock_or_file_normalization(self) -> None:
        before_lock = """\
version = 4

[[package]]
name = "local-package"
version = "0.0.0"
dependencies = ["dep"]
"""
        restamped = before_lock.replace('version = "0.0.0"', 'version = "0.153.4"')
        initial = ("a" * 40, "tree", "")
        cases = (
            (
                ("a" * 40, "tree", " M codex-rs/Cargo.lock\n"),
                restamped.replace('dependencies = ["dep"]', 'dependencies = ["other"]'),
                frozenset({"local-package"}),
            ),
            (
                ("a" * 40, "tree", " M README.md\n"),
                restamped,
                frozenset({"local-package"}),
            ),
            (
                ("a" * 40, "tree", " M codex-rs/Cargo.lock\n"),
                restamped,
                frozenset(),
            ),
        )
        for prepared, after_lock, workspace_packages in cases:
            with self.subTest(status=prepared[2]):
                with self.assertRaises(RuntimeError):
                    VALIDATION.validate_target_lock_normalization(
                        initial,
                        prepared,
                        before_lock,
                        after_lock,
                        "0.153.4",
                        workspace_packages,
                    )

    def test_rejects_non_version_textual_lock_changes(self) -> None:
        before_lock = """\
version = 4

[[package]]
name = "local-package"
version = "0.0.0"
dependencies = ["dep"]
"""
        restamped = before_lock.replace('version = "0.0.0"', 'version = "0.153.4"')
        initial = ("a" * 40, "tree", "")
        prepared = ("a" * 40, "tree", " M codex-rs/Cargo.lock\n")
        cases = (
            restamped + "# unexpected comment\n",
            restamped.replace(
                'name = "local-package"\nversion = "0.153.4"',
                'version = "0.153.4"\nname = "local-package"',
            ),
            restamped.replace('dependencies = ["dep"]', 'dependencies  = ["dep"]'),
        )

        for after_lock in cases:
            with self.subTest(after_lock=after_lock):
                with self.assertRaisesRegex(RuntimeError, "exact expected textual"):
                    VALIDATION.validate_target_lock_normalization(
                        initial,
                        prepared,
                        before_lock,
                        after_lock,
                        "0.153.4",
                        frozenset({"local-package"}),
                    )

    def test_rejects_candidate_only_failure(self) -> None:
        runner = mock.Mock(
            side_effect=[
                self.result(100, "crate test::candidate_regression"),
                self.result(100, "crate test::upstream_failure"),
            ]
        )

        self.assertEqual(self.execute_gate(runner), 1)

    def test_rejects_unparseable_nonzero_target(self) -> None:
        runner = mock.Mock(
            side_effect=[
                self.result(100, "crate test::shared_failure"),
                self.result(101, parseable=False),
            ]
        )

        self.assertEqual(self.execute_gate(runner), 1)

    def test_rejects_non_nextest_failure_exit_code(self) -> None:
        runner = mock.Mock(return_value=self.result(101, "crate test::failure"))

        self.assertEqual(self.execute_gate(runner), 1)

    def test_redacts_sensitive_environment_and_assignments(self) -> None:
        output = "API_KEY=visible token:also-visible ordinary=safe"

        self.assertEqual(
            VALIDATION.redact_output(output, {"API_KEY": "visible"}),
            "API_KEY=<REDACTED> token:<REDACTED> ordinary=safe",
        )

    def test_redacts_bearer_authorization_header_through_end_of_line(self) -> None:
        output = "Authorization: Bearer abc123 trailing-material\nordinary=safe"

        self.assertEqual(
            VALIDATION.redact_output(output, {}),
            "Authorization: <REDACTED>\nordinary=safe",
        )

    def test_redacts_cookie_header_through_end_of_line(self) -> None:
        output = "Cookie: session=abc123; csrf=def456\nordinary=safe"

        self.assertEqual(
            VALIDATION.redact_output(output, {}),
            "Cookie: <REDACTED>\nordinary=safe",
        )

    def test_redacts_quoted_json_credentials(self) -> None:
        output = (
            '{"access_token":"generated-secret","authorization":"Bearer read-secret",'
            '"ordinary":"safe"}'
        )

        self.assertEqual(
            VALIDATION.redact_output(output, {}),
            '{"access_token":"<REDACTED>","authorization":"<REDACTED>",'
            '"ordinary":"safe"}',
        )

    def test_command_output_cap_marks_result_truncated(self) -> None:
        command = VALIDATION.WorkspaceCommand(
            cwd=self.root,
            argv=(sys.executable, "-c", "print('x' * 4096)"),
        )
        with (
            mock.patch.object(VALIDATION, "MAX_COMMAND_OUTPUT_BYTES", 64),
            mock.patch.object(
                VALIDATION, "validation_temp_parent", return_value=self.root
            ),
        ):
            result = VALIDATION.run_bounded_command(command, dict(os.environ))

        self.assertTrue(result.truncated)
        self.assertLessEqual(len(result.excerpt.encode()), 64)

    def test_capped_capture_limits_combined_stdout_and_stderr(self) -> None:
        with mock.patch.object(
            VALIDATION, "validation_temp_parent", return_value=self.root
        ):
            result = VALIDATION.run_capped_process(
                (
                    sys.executable,
                    "-c",
                    "import sys; print('x' * 4096); print('y' * 4096, file=sys.stderr)",
                ),
                self.root,
                dict(os.environ),
                timeout_seconds=30,
                max_output_bytes=64,
            )

        self.assertTrue(result.truncated)
        self.assertLessEqual(len((result.stdout + result.stderr).encode()), 64)

    def test_capped_capture_terminates_on_timeout(self) -> None:
        with mock.patch.object(
            VALIDATION, "validation_temp_parent", return_value=self.root
        ):
            result = VALIDATION.run_capped_process(
                (sys.executable, "-c", "import time; time.sleep(60)"),
                self.root,
                dict(os.environ),
                timeout_seconds=0.01,
            )

        self.assertTrue(result.timed_out)

    def test_production_workspace_commands_are_locked(self) -> None:
        with (
            mock.patch.object(VALIDATION, "REPO_ROOT", self.root),
            mock.patch.object(VALIDATION, "CODEX_RS", self.root / "codex-rs"),
        ):
            commands = VALIDATION.workspace_test_commands(Path("/toolchain/just"))

        self.assertEqual(len(commands), 2)
        self.assertTrue(all("--locked" in command.argv for command in commands))
        failure = self.result(100, "crate test::shared_failure")
        runner = mock.Mock(side_effect=[failure, failure, failure, failure])

        self.assertEqual(self.execute_commands(commands, runner), 0)
        for candidate_call, target_call in (
            (runner.call_args_list[0], runner.call_args_list[1]),
            (runner.call_args_list[2], runner.call_args_list[3]),
        ):
            self.assertEqual(
                candidate_call.args[0].argv,
                target_call.args[0].argv,
            )

    def test_rejects_target_timeout(self) -> None:
        target_timeout = VALIDATION.CommandResult(
            returncode=-15,
            failed_tests=frozenset(),
            declared_failed_count=None,
            timed_out_tests=frozenset(),
            declared_timed_out_count=None,
            summary_count=0,
            excerpt="timed out",
            timed_out=True,
        )
        runner = mock.Mock(
            side_effect=[
                self.result(100, "crate test::shared_failure"),
                target_timeout,
            ]
        )

        self.assertEqual(self.execute_gate(runner), 1)

    def test_rejects_dirty_candidate_before_running_commands(self) -> None:
        runner = mock.Mock()
        dirty_snapshot = ("candidate-head", "candidate-tree", "?? generated.snap.new\n")
        with (
            mock.patch.object(VALIDATION, "REPO_ROOT", self.root),
            mock.patch.object(
                VALIDATION, "candidate_snapshot", return_value=dirty_snapshot
            ),
        ):
            result = VALIDATION.run_differential_workspace_tests(
                (self.command,),
                {"PATH": "/bin"},
                "a" * 40,
                runner=runner,
                target_factory=self.target_factory,
            )

        self.assertEqual(result, 1)
        runner.assert_not_called()

    def test_rejects_candidate_mutation_during_gate(self) -> None:
        runner = mock.Mock(return_value=self.result(0))
        changed_snapshot = ("changed-head", "changed-tree", "")
        result = self.execute_commands(
            (self.command,),
            runner,
            candidate_snapshots=[self.snapshot, changed_snapshot],
        )

        self.assertEqual(result, 1)

    def test_rejects_target_setup_failure(self) -> None:
        @contextmanager
        def failing_target_factory(_target_sha: str):
            raise RuntimeError("clone failed")
            yield self.target

        second_command = VALIDATION.WorkspaceCommand(
            cwd=self.root,
            argv=("just", "test", "-p", "codex-v8-poc"),
        )
        runner = mock.Mock(
            side_effect=[
                self.result(100, "crate test::shared_failure"),
                self.result(0),
            ]
        )
        result = self.execute_commands(
            (self.command, second_command),
            runner,
            target_factory=failing_target_factory,
        )

        self.assertEqual(result, 1)
        self.assertEqual(runner.call_count, 2)

    def test_rejects_target_repository_mutation(self) -> None:
        changed_target = ("a" * 40, "changed-tree", " M Cargo.lock\n")
        runner = mock.Mock(
            side_effect=[
                self.result(100, "crate test::shared_failure"),
                self.result(100, "crate test::shared_failure"),
            ]
        )
        result = self.execute_commands(
            (self.command,),
            runner,
            target_snapshots=[changed_target],
        )

        self.assertEqual(result, 1)

    def test_evaluates_every_workspace_command(self) -> None:
        second_command = VALIDATION.WorkspaceCommand(
            cwd=self.root,
            argv=("just", "test", "-p", "codex-v8-poc"),
        )
        runner = mock.Mock(
            side_effect=[
                self.result(0),
                self.result(100, "crate test::shared_failure"),
                self.result(100, "crate test::shared_failure"),
            ]
        )
        result = self.execute_commands((self.command, second_command), runner)

        self.assertEqual(result, 0)
        self.assertEqual(runner.call_count, 3)

    def test_uses_fresh_target_for_each_failed_command(self) -> None:
        second_command = VALIDATION.WorkspaceCommand(
            cwd=self.root,
            argv=("just", "test", "-p", "codex-v8-poc"),
        )
        failure = self.result(100, "crate test::shared_failure")
        runner = mock.Mock(side_effect=[failure, failure, failure, failure])
        result = self.execute_commands((self.command, second_command), runner)

        self.assertEqual(result, 0)
        self.assertEqual(self.target_factory_calls, ["a" * 40, "a" * 40])
        self.assertEqual(self.target_preparer.call_count, 2)


if __name__ == "__main__":
    unittest.main()
