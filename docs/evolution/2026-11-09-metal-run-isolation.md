# Evolution — Concurrent Metal verification from independent worktrees

**Feature:** `metal-run-isolation`. **Finalized:** 2026-11-09.
**Scope:** developer verification tooling, one release slice and two implementation steps.
The [feature contract and wave history](../feature/metal-run-isolation/feature-delta.md),
[approved roadmap](../feature/metal-run-isolation/deliver/roadmap.json) and
[unchanged execution log](../feature/metal-run-isolation/deliver/execution-log.json)
remain the canonical delivery records.

## Delivered outcome and context

Ordinary verification from independent prepared Git worktrees can overlap on
the shared Metal box. Each checkout owns its source, build artifacts and
evidence. Each durable invocation owns its guest-launch resources and cleanup.
Same-checkout operations remain exclusive, and physical-host maintenance still
requires exclusive admission. The motivating job is
[`J-PLAT-006`](../product/jobs.yaml): assess independent changes without waiting
for an unrelated ordinary execution lease or trusting mixed results.

Previously, sanitised branch names could alias remote state, and a box-wide
execution lease serialized ordinary commands. Removing that lease alone would
leave singleton ownership, guest CID/listener collisions, direct-command kernel
effects and completion ownership unresolved. The delivered slice addresses
those boundaries together under the user-approved P1–P5 contracts. It changes
developer tooling rather than Overdrive product-runtime semantics.

## Decisions and discoveries

The user's **“Approve P1–P5”** approval and independent DESIGN approval are
recorded in the [feature contract](../feature/metal-run-isolation/feature-delta.md#wave-design--ref-user-approval--2026-11-08).
Earlier proposed/pending labels and the construction
[wave decision](../feature/metal-run-isolation/spike/wave-decisions.md) remain
historical, not current acceptance states.

| Decision | Delivered ownership and tradeoff | Permanent authority |
|---|---|---|
| P1 — checkout identity | Canonical Git-worktree identity, approved ordinary-clone fallback and full remote binding prevent branch-name aliasing. Legacy caches remain intact; new identities start cold. | [ADR-0277](../product/architecture/adr-0277-metal-checkout-identity-is-owned-by-the-local-git-worktree.md) |
| P2 — admission | Existing durable services hold shared maintenance admission for ordinary work and exclusive checkout admission. Explicit maintenance remains exclusive. | [ADR-0278](../product/architecture/adr-0278-metal-ordinary-runs-share-the-maintenance-barrier.md) |
| P3 — guest-launch resources | Anonymous local-mode vsock network namespaces isolate existing CID/listener identities. Invocation roots bound staging, VMM runtime and command temporary files. | [ADR-0279](../product/architecture/adr-0279-metal-guest-resources-belong-to-the-durable-invocation.md) |
| P4 — selected test execution | A closed placement inventory and nextest wrapper preserve the selected ELF, arguments, helpers, environment, retry and result owner. Direct selected guest attempts incur fresh-group boot cost. Arbitrary maintenance uses explicit `--exclusive`. | [ADR-0280](../product/architecture/adr-0280-metal-nextest-execution-uses-explicit-test-placement.md) |
| P5 — completion | The existing systemd owner drains descendants and removes only its three roots before completion becomes collectable. Collection is read-only; command results and cleanup results remain distinct. | [ADR-0281](../product/architecture/adr-0281-metal-run-completion-follows-owned-resource-cleanup.md) |

The [resource analysis](../feature/metal-run-isolation/design/resource-alternatives.md)
and [construction findings](../feature/metal-run-isolation/spike/findings.md)
record why the initial 224-window CID allocator was superseded. Reading the
pinned Linux 7.0 source exposed an existing namespace boundary for both CIDs and
listeners. Native construction established its actual VMM use. A numeric hash
alone could not provide collision-free ownership. A whole-run outer guest, a
persistent execution broker and a batch-result interpreter were left unselected
because they changed the approved boundary or added execution machinery. Their
rejection is not a claim that every alternative failed an experiment.

Construction also distinguished six real-socket test identities from pure Sim
execution. A native fixture's API-socket EACCES reproduced in both namespace
contexts and was traced to its supplied parent traversal permissions. Supported
nextest timeout bypassed Rust Drop, establishing the need for owner-stop
finalization. These findings shaped placement and cleanup without weakening the
original native or zero-delta requirements.

Later actual-owner conformance found a queued-controller-EOF defect. The bounded
correction `8b1dd117` cancels and reaps that invocation's queued lock waiter.
The [retained proof](../feature/metal-run-isolation/deliver/native-evidence/owner-queued-eof-verification-2026-11-09-090213Z/README.md)
records the exact failing seed, both controller-loss controls and a fresh
randomized seed passing after correction. The final slice review also closed
the external archive-marker oracle error and reproduced login-mode preparation
permissions/argument mapping. Those bounded corrections are distinct from the
unrelated baseline failure below.

## Implementation and independent approvals

| Boundary | Completed record |
|---|---|
| Step 01-01 — bind checkout source and retained artifacts | Implementation `ee0bbd658`; seven assigned Lima identity/transfer cases. [Step review](../feature/metal-run-isolation/deliver/review-01-01.md) **APPROVED**. |
| Step 01-02 — concurrent ordinary execution and owned cleanup | Implementation `2ba223220` and bounded corrections through final runtime source `060359bc1cf88c78252e9d789396821413b41d7e`. Reported step evidence includes 32 supported acceptance cases, 24 fixture-library tests and two qualification transitions. [Review history](../feature/metal-run-isolation/deliver/review-01-02.md) retains the actual findings and remediation. |
| Final independent full-slice review | [review-slice.md](../feature/metal-run-isolation/deliver/review-slice.md), committed in `df561587e`, **APPROVED** for implementation and consolidated feature-specific native evidence. FSL-1 and FSL-2 are closed; no open feature finding remains. |
| Final documentation review | [review-documentation.md, iteration 3](../feature/metal-run-isolation/deliver/review-documentation.md#iteration-3--final-native-evidence-reconciliation), **APPROVED** for content commit `33654e76715810c6fe172cc5a60f4e5833c94279`, with no open documentation finding. D1 remains closed. |

Finalization mechanically verified roadmap `validation.status = approved` and
the schema-3 CLI-authored RED/GREEN/COMMIT traces. With
`PYTHONPATH=/Users/marcus/.claude/lib/python`, `des-verify-integrity` returned 0:
**All 2 steps have complete DES traces**. Schema 3 records phase events rather
than a per-step `DONE` field. The verifier and independent approvals establish
completion without inventing a status field or rewriting phase events.

## Verified outcomes and precise evidence limits

The [consolidated native receipt](../feature/metal-run-isolation/deliver/native-evidence/placement-final-complement-2026-11-09-114229Z/README.md)
is the entry point for the original raw records and their source attribution.
Final acceptance combines explicitly identified captures. It does not represent
an uninterrupted full-driver pass at one SHA.

- The [paired worktree and admission capture](../feature/metal-run-isolation/deliver/native-evidence/feature-native-closeout-2026-11-09-104319Z/README.md)
  returned both driver results 0 at root `8914e5ee` and peer `06f25ce2`, with
  matching recorded feature hashes. Actual shared maintenance locks, distinct
  checkout locks and simultaneous same-CID VMMs in distinct namespaces prove
  ordinary overlap. Controller-loss/repeat collection, unchanged survivor
  state, exact raw physical complement and completed-owner process/root absence
  satisfy K1–K3 at those recorded sources.
- Admission separates the actual public guest-timeout case from E4's approved
  controlled-prior-owner witness on its fixture-private barrier. Maintenance
  progressed only after that prior owner's finalizer completed; the registered
  ordinary waiter remained unadmitted. The witness retains that boundary.
- Placement retains its original selected-attempt prefix at `8914e5ee`, the
  corrected archive typed-marker assertions and invalid-mode result at
  `4bea1ebe`, and the [four existing execution modes](../feature/metal-run-isolation/deliver/native-evidence/login-mode-cache-argument-correction-2026-11-09-113245Z/README.md)
  at frozen runtime source `060359bc`. Privileged, privileged/no-sync,
  login/no-sudo and login/no-sync/no-sudo each returned 0. The corrected archive
  refusal remains `artifact_scope_mismatch`/64. Intended nextest 100 and command
  255 results remain their actual failures/statuses.
- The unchanged final observer returned 0 at evidence-only source `f8c0f02d`,
  retained in `d4d011b4`. It reports zero forbidden physical deltas, zero owned
  processes and all three exact owned roots absent. Only the two paired finite
  RA expiry countdowns differ in raw physical snapshots, under the existing
  approved projection. No raw observation or zero-state invariant was changed.
- Native owner evidence spans fourteen initially passing identities and the
  later queued-EOF correction proof. The latter contains four executions across
  three identities, not a fresh all-fifteen campaign. Direct-native launcher
  evidence and specifically named nested-kernel evidence keep their distinct
  claims.
- The [permanent Quint model and evidence](../../specs/quint/metal-run-isolation/README.md)
  retain 18 expected outcomes: 16 holds and two intentional hazard violations.
  They model bounded admission/completion under declared fairness assumptions;
  they do not prove the kernel or promise infinite-command termination. No new
  formal or native campaign ran during finalization.

Mutation testing was disabled by the user's explicit **“NO mutation testing”**
instruction recorded in DEVOPS. No mutation ran and no score is claimed.
There is no measured speedup, throughput result, unlimited capacity promise,
reboot recovery or automatic pruning.

### Single unrelated owner-burst exclusion

`guest_flow::real_kernel::a_burst_of_host_aborts_resets_every_application_connection`
remains a failing preimplementation body. The
[clean baseline receipt](../feature/metal-run-isolation/deliver/native-evidence/owner-burst-preimplementation-baseline-2026-11-09/README.md)
at `0e8ae265c` reproduced the same one-second/1,000-flow assertion with 78 ends
consumed after successful preparation. The body and fixture blobs are identical
to current source, and the [current original-fixture comparison](../feature/metal-run-isolation/deliver/native-evidence/owner-burst-current-branch-comparison-2026-11-09/README.md)
also fails independently of the new dispatcher.

The user's **“stop focusing on failures that are not related to this feature !!!!!”**
instruction excludes investigation/remediation of that baseline body. Only that
body was omitted through the documented transient driver selection. Its original
test, oracle and checked-in selection remain intact. A full current-source
owner-cell GREEN remains unproven. This exclusion does not waive P1–P5, physical
kernel isolation, exact command selection/results, stopped-owner process/root
absence or survivor preservation. Earlier failed and invalidated captures remain
retained with their original results.

## Operator documentation, blog and validation

The checked-in [operator guide](../../website/content/docs/how-to/run-verification-on-metal.mdx)
describes preparation, ordinary/exclusive commands, collection, typed outcomes
and current limits. The [engineering post](../../website/content/blog/sharing-a-metal-box-without-sharing-test-state.mdx)
explains the original constraint, serious alternatives, kernel discovery,
ownership choices, later defects and measured outcomes. Both final versions and
the [validation record](../feature/metal-run-isolation/deliver/documentation-validation-final.md)
are committed in `33654e767` and independently approved by documentation review
iteration 3.

Final scoped validation reports passing typecheck, lint, all 12 authored links,
both heading anchors, navigation and regenerated docs/blog collection discovery.
The earlier Next/OpenNext build, one-index and focused local Worker observations
retain their original attribution; they were not rerun for final prose or this
closeout. The earlier author retained no standalone command logs. No claim is
made that every website acceptance suite passed.

The existing `deterministic simulation` search-ranking assertion failed in
`test:blog`, `test:mcp` and `test:mcp:analytics`, and reproduced on an untouched
archived website baseline. Later analytics assertions remain unexecuted. This
unrelated ranking failure was recorded and was not rerun or changed during final
reconciliation/finalization. Website publication and deployment remain separate;
neither occurred.

## Artifact preservation and closeout

The exact FINALIZE migration map has no matching source artifact: no standard
`design/{architecture-design,component-boundaries,technology-stack,data-models}.md`,
`design/adrs/ADR-*.md`, `distill/walking-skeleton.md` or mapped `discuss/journey-*`
file exists for this feature. No lasting artifact needed copying or moving.
The five accepted ADRs already reside in `docs/product/architecture/`; formal
specifications/evidence already reside in `specs/quint/metal-run-isolation/`.
The feature contract, construction findings, placement inventories, native
receipts and verification catalog retain their existing canonical locations.

All wave history, reviews, roadmap, DES events and raw evidence are preserved.
No feature directory, session marker or temporary artifact was deleted.
Finalization changes only this record, the non-normative DELIVER/FINALIZE
completion references and the completed reviewer's final documentation artifact.
The normative feature-contract prefix is byte-identical to its pre-finalization
version. The seven unrelated dirty paths, including the stopped service
feature's work, remain outside this commit and unchanged.

The lasting lesson is to establish the exact kernel/owner boundary before
inventing allocation or recovery machinery, then keep model, source, native and
documentation evidence independent and precisely attributed. A composite proof
can close unchanged feature outcomes without erasing failed attempts or
misrepresenting an unrelated baseline failure as success.
