# September 2026 upgrade audit

Scope: 0.150.1 preparation/shipment, 0.153.2 preparation, 0.153.3/.4 reconciliation and cutover,
and the subsequent reconnect/host-lifecycle repair. This is historical evidence, not current runtime
status. Source-owned process changes belong in this skill; generic runtime mechanisms belong in
`claude-process-guard`, and Fork Fleet coordinator/gate defects belong in Worklens.

## Measured context cost, not an invoice

The last native `event_msg/token_count` cumulative counters in the three root transcripts were:

| Root session | Input | Cached input (subset) | Output | Spawn calls |
| --- | ---: | ---: | ---: | ---: |
| `01a041ad-815d-7793-b8c2-07b4706e4017` | 100,967,045 | 99,108,352 | 197,095 | 29 |
| `01a069c2-a9e3-7483-bfb1-e6d87f44aad6` | 14,858,142 | 14,309,888 | 34,297 | 11 |
| `01a06e0b-d712-79e3-90dc-bb017f436832` | 543,665,936 | 536,712,320 | 818,535 | 91 |

These are reported cumulative counters, not unique text, billed cost, or a measured waste fraction.
They include repeated cached context and later work within each root; child-session accounting is
not independently reconciled here. Do not add cached input to input, sum every cumulative event,
add reasoning output again, or label the sum a complete fleet cost. Spawn count is not proof that
delegation was wasteful. The actionable waste below is grounded in abandoned/repeated work.

## Evidence and prevention

Transcript paths are under `~/.codex/sessions/`; line numbers below are JSONL record locators.

| Evidence | Avoidable work / failure | Required prevention |
| --- | --- | --- |
| Sep 3 root `01a069c2`, records 337–402 | Direct Git continuation caused coordinator drift and a replacement candidate | Stage only; continue through Fork Fleet; preserve mismatched candidate |
| Same root, records 818–853 | Many `rework` decisions became sequential historical replay; large handover family remained unsafe/conflicted | Target-native design and behavior tests before replay; bounded patch families |
| Sep 4 root `01a06e0b`, records 24734–25145 | zlib/BLAKE3 fixes and Windows build investigation were discarded after discovering the local gate reproduced upstream's cross-platform release matrix | Expand gate commands and host prerequisites before scheduling; use canonical Linux packaging |
| Same root, records 26183–26393 | Marker check passed, but dropped generation APIs made the installed router incompatible | Probe actual candidate RPC/env contract before activation; track host consumers when dropping patches |
| Same root, records 26493–26581 | Hard cutover waited on a five-minute graceful router timeout | Explicit migration authority, bounded lifecycle plan, rollback, independent execution, full post-check |
| Follow-up reconnect repair: production `e40820b4475a`, tests `8cb304b21b` | Native reconnect existed but stopped retrying writer contention too early | Test contention beyond early attempts, both conversation/overview, draft retention and no uncertain-input replay |
| Follow-up host repair `2ec36f8` in claude-process-guard | New installed version coexisted with an old server in another profile | Generic home-scoped service ownership; verify every scoped daemon and old TUI executable separately |

Exact audited root files:

- `2026/08/27/rollout-2026-08-27T00-24-45-01a041ad-815d-7793-b8c2-07b4706e4017.jsonl`
- `2026/09/03/rollout-2026-09-03T19-12-40-01a069c2-a9e3-7483-bfb1-e6d87f44aad6.jsonl`
- `2026/09/04/rollout-2026-09-04T15-11-04-01a06e0b-d712-79e3-90dc-bb017f436832.jsonl`

Many efficiency rules already existed: serialized gates, frozen source, narrow failure classification,
one broad canary, cached builds, compact output, and checkpoint reuse. Their existence did not
prevent the failures. The coordinator must record the required evidence before advancing, rather
than merely rereading the rules after a failure. The initial documentation pass added no automated
enforcement. The subsequent implementation adds dossier/template generation, phase eligibility,
source-bound receipts/reuse, attempt limits, and regression scenarios through the
[enforced helper workflow](enforced-workflow.md). Semantic review and external observations still
require truthful evidence; the helper does not grant authority. An unresolved registry/publication
blocker must be named early with the exact missing
choice; repeated “proceed” should not trigger the same expensive inventory or build again.

## Next-run acceptance contract

1. One pinned target, declared source base, destination/authority, runtime contract, and bounded
   [Git dossier](release-history.md). Inspect native replacements before porting fork mechanisms.
2. One decision table with retained invariants, tests, host consumers, and target-native rework plans.
3. One coordinator-owned candidate; no raw Git continuation; independent review only at coherent
   boundaries, with bounded evidence and disjoint ownership.
4. Preflight actual gate expansion; focused debug/test iteration, serialized gates, stable fixtures,
   one approved broad canary at most, one final optimized package.
5. Behavioral candidate/host compatibility before promotion. No cache-string marker substitutes.
6. Four separate completion receipts: source publication, selected artifact, live servers across
   scoped homes, existing clients/reconnect. Preserve drafts and active work; migration authority
   is not inferred from version drift.
7. Save exact SHA/tree, invocation fingerprint, gate outcome/duration, and artifacts in the run ledger.
   On retarget/resume, reuse valid evidence and rerun only invalidated checks. Report token counters
   with cache/lineage caveats and log abandoned gates so the next audit can measure improvement.
