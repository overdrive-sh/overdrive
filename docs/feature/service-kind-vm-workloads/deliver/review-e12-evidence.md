# E12 Evidence Audit — Different-Fox Review

## Metadata

| Field | Value |
|---|---|
| Feature | `service-kind-vm-workloads` |
| Roadmap step | `03-02` — E12 liveness restart |
| Expectation | `E12-vm-service-liveness-restart-describe` |
| Auditor | Independent evidence-only auditor (Codex) |
| Audit date | 2026-09-09T15:58:49Z |
| Declared substrate | `native-metal` |
| Final capture date | 2026-09-09T15:37:13Z |
| Final capture SHA | `d850c94dc69d3bef7d6cb962210672efb67b0686` |
| Verdict | **SATISFIED** |

The implementation review in `deliver/review-03-02.md` records an independent
iteration-2 **APPROVED** verdict. That review is used only to establish the
separate implementation gate; it is not treated as proof of any E12 evidence
claim.

## Audit-only boundary

This audit examined the approved 03-02 contract, the E12 README and runner as
the black-box assertion anchor, the verification catalogue conventions, the
final captured receipt, and every retained E12 attempt. It did not inspect the
production or test implementation that generated the capture, import or link
an Overdrive crate, run the expectation again, run Rust tests, or run mutation
testing. The retained dirty-patch files were identity/provenance artifacts; no
implementation diff content was read.

Only this Markdown artifact is written by the audit. No README, INDEX,
evidence file, DES log, staging area, or commit was changed.

## Contract and anchors

The approved roadmap entry for step 03-02 requires:

1. the liveness threshold to produce only the existing liveness stop;
2. `WorkloadLifecycle` to own the existing-budget restart/final-failure
   decision; and
3. E12 to show liveness restart, ordinary replacement startup, no
   readiness-owned restart, no dead revival, and zero leaks.

The same entry requires the E12 harness evidence and independent audit. The
expectation README anchors the capture to `S-SVM-28`, `US-SVM-3`, and the
accepted lifecycle gate ownership. The anchors are present in the final
receipt (`evidence/verification.yaml:13-14`) and predate this audit.

The runner is the executable black-box predicate. It requires exactly three
ledger rows, validates the before/terminal/after states and liveness
observations, requires one allocation identity across the rows, compares the
pre- and post-restart startup/`Since` values, requires the observed threshold
count to be at least the fixture threshold, rejects readiness failure
attribution, requires the prior terminal observation in public describe, and
requires the complete zero-delta cleanup line (`runner.sh:58-110`).

## Evidence examined

The following material was read:

- `deliver/roadmap.json` — the approved `03-02` E12 entry.
- `verification/README.md` and `verification/expectations/INDEX.md` — anchor,
  execution, status, and retention conventions.
- `verification/expectations/E12-vm-service-liveness-restart-describe/README.md`
  and `runner.sh`.
- `execution-substrate` (`native-metal`).
- Final evidence: `verification.yaml`, `product-run.meta`, complete
  `product-run.out`, complete `run.log`, `liveness-restart.tsv`, and
  `dirty-status.txt`. The final `dirty-diff.patch` was retained and hashed for
  provenance only; its contents were not inspected.
- Retained `attempt-1` through `attempt-4` manifests, metadata, complete
  product output, complete runner logs, ledgers, and dirty-status records.
  Attempt dirty patches were retained but not inspected for implementation
  content.

## Provenance and actuality

The final manifest records `runner_invoked: true`, `execution_status:
"succeeded"`, `execution_substrate: "native-metal"`, `executed_in_lima:
false`, runner exit `0`, seed `1`, and matching product/harness SHA
`d850c94d...` (`evidence/verification.yaml:1-14`). This is the expected
native-metal lane, not a Lima substitution.

The final invocation metadata records the metal runner, the checked-in
`liveness-restart` example, a 1200-second journey budget plus 90-second
cleanup grace, and exit `0` (`evidence/product-run.meta:1-9`). The complete
captured output shows the metal lease/preflight, runtime preparation, the
built `target/debug/overdrive` deploy, and three public `workload describe`
snapshots (`evidence/product-run.out:1-32`). It ends with the product's E12
PASS line and teardown deltas of zero for VM, probes, network, cgroup,
run-directory, mount, loop, and preparation (`product-run.out:91-101`).

The capture explicitly declares a dirty source tree and retains both the
dirty status and dirty patch. The status identifies the example, runner,
expectation, evidence, DES, and pre-existing review bookkeeping, rather than
silently presenting a dirty capture as a clean commit
(`evidence/dirty-status.txt:1-16`). The SHA in the manifest is therefore the
historical capture identity; the later checkout/commit state was not
substituted for it. The retained final dirty patch has the read-only SHA-256
digest `a8cfd0db88fa947b4f9ec9fa521f994222f4e775598a052b446311da3c73d491`.

## Claim-by-claim audit

### 1. Liveness threshold produces a terminal allocation before restart

**Result: SATISFIED.**

The final ledger has one healthy `before` row, then a `terminal` row with the
same allocation ID, state `Terminated`, restart count `0`, liveness `fail`,
`HTTP 503`, liveness attribution, and observed failure count `13`, followed by
the `after` row (`evidence/liveness-restart.tsv:1-4`). The corresponding public
describe sequence shows `Replicas 1/1`, then `1/0` with the allocation
`Terminated` at `(c=213,w=local)`, before the replacement's `1/1` snapshot at
`(c=214,w=local)` (`evidence/product-run.out:33-75`). This is the required
terminal-before-replacement ordering, not a narration inferred from the final
state.

The runner's terminal predicate independently requires a terminal state,
zero prior restarts, a failed liveness probe, `HTTP 503`, liveness attribution,
and a threshold count at or above the fixture threshold
(`runner.sh:63-79`).

### 2. Replacement is an ordinary fresh VM Running/startup path

**Result: SATISFIED.**

The final `after` row is `Running`, has restart count `1`, liveness `pass`,
`HTTP 204`, and the same allocation ID. Its startup observation changes from
`1788968249596` before the failure to `1788968284160` after it, and its
allocation `Since` value changes from `(c=15,w=local)` to `(c=214,w=local)`
(`evidence/liveness-restart.tsv:2-4`). The public after-replacement snapshot
shows the ordinary `driver started` reason, startup `last=pass`, readiness
`last=pass`, liveness `last=pass`, and the new certificate serial
(`evidence/product-run.out:68-90`). No VM-specific lifecycle state appears in
that public projection.

The runner checks the exact ordinary replacement fields and rejects unchanged
startup observation or `Since` values (`runner.sh:80-97`). The changed
observations and changed `Since` value therefore establish a fresh replacement
attempt rather than a stale terminal row being relabelled as Running.

### 3. Readiness does not own the restart; the dead allocation is not revived

**Result: SATISFIED.**

At the terminal snapshot, readiness is still `last=pass` while liveness is the
failed `HTTP 503` observation (`evidence/product-run.out:64-67`). After
replacement, readiness is again `last=pass` and the replacement is Running
(`product-run.out:87-90`). The runner rejects any public readiness failure
attribution (`runner.sh:102-105`).

The terminal row is observed before the changed `Since`/startup values and the
after row has restart count `1` (`evidence/liveness-restart.tsv:2-4`); the
runner also requires one identity across all three rows and the changed
post-restart observations (`runner.sh:93-97`). No dead-revival contradiction is
present in the captured public sequence.

### 4. Liveness attribution is observed while public attribution remains generic

**Result: SATISFIED.**

The executed ledger records liveness role, failed status, `HTTP 503`, the
`liveness-probe` terminal attribution, and the observed count `13` at the
terminal row (`evidence/product-run.out:91-95`). In the public describe
transcript, the terminal allocation says only `reason: stopped`, and the
replacement's `last terminated` line also renders `stopped`
(`product-run.out:54-75`). This matches the anchored requirement that the
internal observed transition is liveness-derived while the public prior
terminal reason stays generic.

The runner obtains the expected threshold from the checked-in fixture and
checks the captured numeric witness rather than accepting a missing or
non-numeric field (`runner.sh:17-24`, `runner.sh:68-79`). No evidence
contradiction indicates that the final liveness attribution was fabricated or
unobserved.

### 5. Cleanup is demonstrably zero

**Result: SATISFIED.**

The complete product output contains the exact zero cleanup delta for every
declared resource category (`evidence/product-run.out:97-101`). The runner
requires that exact line before printing the E12 PASS marker
(`runner.sh:106-110`).

## Attempt-history handling

Failed and obsolete captures were not concealed or silently replaced:

| Capture | Receipt | Evidence disposition |
|---|---|---|
| Attempt 1 | `14:47:04Z`, exit `1` (`attempt-1/verification.yaml:1-16`, `attempt-1/product-run.meta:7-9`) | Retained and excluded. The full output shows repeated restarts without a qualifying terminal-before-replacement ledger and the runner reports that no ordinary replacement was produced. |
| Attempt 2 | `14:51:57Z`, exit `1` (`attempt-2/verification.yaml:1-16`, `attempt-2/product-run.meta:7-9`) | Retained and excluded. The full output records the runner's ordinary-liveness-pass failure. |
| Attempt 3 | `14:54:34Z`, nominal runner exit `0` (`attempt-3/verification.yaml:1-16`) | Retained but excluded as obsolete evidence: its ledger lacks the current startup/`Since`/threshold columns and shows the terminal and after projections as `Running` with liveness still failed. It cannot support the approved E12 contract. |
| Attempt 4 | `15:32:53Z` manifest, runner exit `1` (`attempt-4/verification.yaml:1-16`) | Retained and excluded. Although its remote product output reached a nominal PASS, the complete runner log records duplicated ledger rows and the exact lifecycle assertion failure (`attempt-4/run.log:1-14`); the failed harness result is authoritative for that attempt. |
| Final | `15:37:13Z`, succeeded, exit `0` (`evidence/verification.yaml:1-16`, `product-run.meta:7-9`) | Qualifying capture. It has one clean three-row ledger, complete public describe output, and all runner predicates pass. |

The final evidence is not being rescued by selecting a convenient attempt: the
earlier failures remain visible, and the only accepted receipt is the later
fresh capture with the current 11-column ledger and clean runner result.

## Non-findings and boundary decisions

- No traffic/client assertion was required. The approved E12 anchor is the
  describe/lifecycle journey; adding a peer-traffic or eligibility claim would
  expand the contract beyond the roadmap.
- No implementation-source or public-API conclusion is made here. Those
  questions belong to the separately approved implementation review.
- No Rust test, test binary, crate import/link, or expectation rerun was used
  as evidence. The capture is the black-box example through the built
  default-feature binary.
- Mutation testing was not run, as explicitly directed by the user and as
  required to remain outside this evidence audit.
- The expectation README and INDEX still say `pending`; that is documentation
  bookkeeping, not a contradiction in the captured receipt. They remain
  unchanged by this audit.

## Read-only commands used

The audit used only read-only inspection: targeted `rg --files`/`rg` lookups,
`sed`/`nl` reads of the approved anchor, runner, catalogue, review verdict,
manifests, outputs, ledgers, and statuses; `find`/`wc` for evidence inventory;
`git rev-parse` and `git status` for checkout identity; and read-only SHA/status
checks for retained dirty-patch provenance. No cargo command, expectation
harness invocation, source implementation read, mutation command, staging
operation, or commit was performed.

## Verdict and next documentation action

**SATISFIED.** The final native-metal receipt is executed, anchored, pinned to
its product/harness SHA and declared dirty state, preserves all failed
attempts, and directly satisfies every approved E12 black-box predicate:
terminal liveness stop before replacement, fresh ordinary startup, no
readiness-owned restart, no dead revival, observed liveness attribution with
generic public `stopped` rendering, and zero cleanup delta.

After this artifact is accepted by the orchestrator, update only the catalogue
documentation: change E12's README status from `pending` to `satisfied` and
link this audit artifact, then change the E12 row in
`verification/expectations/INDEX.md` (and its nearby feature-coverage note)
from pending to satisfied with the same audit link. Do not alter the captured
receipt, retained attempts, runner, DES log, or implementation review as part
of that documentation update.
