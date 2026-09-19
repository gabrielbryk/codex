#!/usr/bin/env python3
"""Render the generated PATCHES.md region from an immutable Fork Fleet plan."""

import argparse
import difflib
import importlib.util
from pathlib import Path
from typing import Any, Mapping

VALIDATION_SCRIPT = Path(__file__).with_name("fork_fleet_validation.py")
_SPEC = importlib.util.spec_from_file_location(
    "fork_fleet_validation", VALIDATION_SCRIPT
)
assert _SPEC is not None and _SPEC.loader is not None
_validation = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_validation)
load_immutable_plan = _validation.load_immutable_plan

BEGIN_MARKER = "<!-- forkctl:generated:begin -->"
END_MARKER = "<!-- forkctl:generated:end -->"

TABLE_HEADER = "| Patch | Class | Decision | Current upstream assessment |"
TABLE_SEPARATOR = "|---|---|---|---|"


def _cell(text: str) -> str:
    """Normalize a table cell value: escape pipes, collapse newlines, strip."""
    collapsed = text.replace("\r\n", " ").replace("\r", " ").replace("\n", " ")
    return collapsed.replace("|", "\\|").strip()


def render_target_paragraph(plan: Mapping[str, Any]) -> str:
    target_ref = plan.get("targetRef") or "unknown"
    target_sha = plan.get("targetSha")
    source_base_sha = plan.get("sourceBaseSha")
    return (
        f"Current upstream target: **`{target_ref}`** (`{target_sha}`). "
        f"Source base: `{source_base_sha}`."
    )


def render_table(patches: list[Mapping[str, Any]]) -> str:
    rows = sorted(patches, key=lambda patch: (patch.get("group") or "~", patch.get("patchId")))
    lines = [TABLE_HEADER, TABLE_SEPARATOR]
    for patch in rows:
        patch_id = patch.get("patchId")
        cls = patch.get("group") or patch.get("maintenanceClass") or "unclassified"

        decision = patch.get("disposition") or patch.get("recommendation")
        if decision == "upstream":
            decision = "drop"

        decision_basis = patch.get("decisionBasis")
        decision_reason = patch.get("decisionReason")
        assessment = decision_basis or decision_reason or patch.get("intent") or "no recorded basis"

        if decision_basis and decision_reason:
            assessment = f"{assessment} [{decision_reason}]"

        lines.append(
            f"| `{_cell(str(patch_id))}` | {_cell(str(cls))} | {_cell(str(decision))} | "
            f"{_cell(str(assessment))} |"
        )
    return "\n".join(lines)


def render_generated_region(plan: Mapping[str, Any]) -> str:
    target_paragraph = render_target_paragraph(plan)
    table = render_table(list(plan.get("patches") or []))
    return "\n".join(
        (
            target_paragraph,
            "",
            "## Maintained logical patches",
            "",
            table,
        )
    )


def splice(existing_text: str, region: str) -> str:
    if existing_text.count(BEGIN_MARKER) != 1 or existing_text.count(END_MARKER) != 1:
        raise SystemExit("fork patch manifest is missing the generated region markers")
    begin_index = existing_text.index(BEGIN_MARKER) + len(BEGIN_MARKER)
    end_index = existing_text.index(END_MARKER)
    if end_index < begin_index:
        raise SystemExit("fork patch manifest is missing the generated region markers")
    return existing_text[:begin_index] + f"\n{region}\n" + existing_text[end_index:]


def check(existing_text: str, region: str) -> int:
    spliced = splice(existing_text, region)
    if spliced == existing_text:
        return 0
    diff = difflib.unified_diff(
        existing_text.splitlines(keepends=True),
        spliced.splitlines(keepends=True),
        fromfile="PATCHES.md",
        tofile="PATCHES.md (rendered)",
    )
    print("".join(diff))
    return 1


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--plan", required=True)
    parser.add_argument("--manifest", required=True)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()

    plan = load_immutable_plan(Path(args.plan).resolve())
    manifest_path = Path(args.manifest)
    existing_text = manifest_path.read_text()
    region = render_generated_region(plan)

    if args.check:
        raise SystemExit(check(existing_text, region))

    spliced = splice(existing_text, region)
    if spliced != existing_text:
        manifest_path.write_text(spliced)
        print("updated PATCHES.md")
    else:
        print("PATCHES.md is current")


if __name__ == "__main__":
    main()
