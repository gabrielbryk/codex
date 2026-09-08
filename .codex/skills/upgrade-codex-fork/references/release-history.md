# Release-history dossier

Generate once per pinned target; save bounded artifacts in the upgrade run directory. Git narrows
review; it does not replace behavior tests. These commands are read-only after fetching and pinning
refs in the planning preflight. Resolve each placeholder to a verified commit first:

```bash
git rev-parse 'refs/tags/<old-stable>^{commit}'
git rev-parse 'refs/tags/<target-stable>^{commit}'
git rev-list --left-right --count <old-base>...<new-base>
git log --left-right --cherry-mark --oneline <old-base>...<new-base>
git diff --find-renames --name-status <old-base> <new-base>
git diff --stat <old-base> <new-base>
git diff --name-only <old-base> <old-fork>
git log --oneline <old-base>..<new-base> -- <patch-family-paths>
git range-diff <old-base>..<old-fork> <new-base>..<candidate>
```

`old-base` must be the registry's explicit source base, not a merge base with the target. The old
release tag is a separate provenance cross-check. `old-fork` is the pinned source tip, and the final
range-diff waits until the candidate exists. Never run these with unresolved angle-bracket values.

## Interpret the graph correctly

- Two-endpoint `git diff old new` describes shipped tree changes. Triple-dot diff and GitHub's
  comparison UI start at the merge base and can omit old-release hotfix differences.
- `git log old..new` describes ancestry additions, not all behavior differences.
- Stable release lines contain backports and need not be ancestors of the next stable release.
  GitHub release `target_commitish: main` does not establish tag ancestry.
- `range-diff` compares the declared old fork series with the new fork series. Inspect missing,
  rewritten, and new patches; do not include the whole upstream upgrade in the old patch range.
- `git cherry` and stable patch IDs propose textual equivalence only. Bundled CI edits can produce
  false negatives; path-scoped patch IDs can help investigate them. A match does not prove the
  change survived later reverts or that failure handling and tests are equivalent.

Intersect upstream changed paths with the old fork's net changed paths, preserving both sides of
renames. Review that overlap first, plus mandatory protocol/schema, daemon/TUI lifecycle, execution,
MCP, dependency/toolchain, and packaging surfaces. Nonoverlap is not proof of independence.
Give each reviewer one patch family and a bounded dossier, not the full upstream diff/transcript.
For every family record old intent, upstream replacement and tests, apply/rework/drop decision,
remaining invariants, host consumers, and required narrow gates. Retargeting invalidates only
evidence affected by the endpoint delta; preserve the earlier dossier instead of recollecting it.

## Verified September example

The 0.150.1 base is `90854393966b21e9ebfd21b122334eb09a20c93d`; 0.153.4 is
`3d2ee51ca2d5db578f328aa75e20aa22c0197c9a`. They diverge: two old-side and 353 new-side commits.
The endpoint diff changes 1,603 files; the old fork at `1798ad4dd7028dc37fb264b648a084420a0251bf`
changes 235 files relative to its base. Direct intersection: 95 files, before integration expansion.

Path history exposes native replacements before porting old recovery code:
`a7913390f7` preserves drafts, `907c34e867` adds automatic reconnect, and `746798b2f7` restores
navigation. Review their behavior against writer-lease contention; their existence alone does not
prove the fork's full recovery contract is upstreamed.

Backport `7a48857579` bundles CI changes with main's `528fd7ace5` image-budget behavior. Whole-commit
patch IDs differ, but the feature/test-path patch IDs match. This is a concrete reason not to use
`git cherry` as an automatic retain/drop classifier.

## Upstream release mechanism

Verified at upstream `3d2ee51ca2d5db578f328aa75e20aa22c0197c9a`:
`.github/workflows/rust-release.yml` runs on `rust-v*.*.*` tags, requires tag/Cargo version agreement,
and classifies stable versus prerelease assets/npm publication. Its moving `latest-alpha-cli` ref
is not a stable-target authority. `.github/workflows/rust-release-prepare.yml` refreshes model
metadata; despite its name, it does not cut a release. Release-line PR #42805 targets `release/0.153`
and backports main with adaptation. Reinspect these pinned workflow files on the next upgrade;
release policy can change. Use release metadata plus peeled tags and actual ancestry, not names.
