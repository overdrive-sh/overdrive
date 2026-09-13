# Evolution — Driver-neutral allocation replacement (GH #284)

**Corrective finalization:** 2026-09-13. **Feature slug:**
`vm-recreation-allocation-id-reuse`. **Waves:** corrective DESIGN →
re-DISTILL → DELIVER → FINALIZE. **Current completion record:**
[driver-neutral roadmap](../feature/vm-recreation-allocation-id-reuse/deliver/roadmap.json),
[execution log](../feature/vm-recreation-allocation-id-reuse/deliver/execution-log.json),
[step 01-01 review](../feature/vm-recreation-allocation-id-reuse/deliver/review-01-01.md),
[step 01-02 review](../feature/vm-recreation-allocation-id-reuse/deliver/review-01-02.md),
and [qualified-metal evidence](../feature/vm-recreation-allocation-id-reuse/deliver/evidence-01-02-metal.md).

## Superseded VM-only delivery did not ship

An earlier delivery implemented the narrower ADR-0104 model: VM replacement
used fresh identities through `StartAllocation`, while legacy Exec retained
same-ID `RestartAllocation`. That three-step delivery and the first version of
this evolution record were subsequently rejected because allocation identity
is application policy shared by every execution driver, not VM-specific
policy.

The rejected implementation was reverted in commit `4fb267f3` before the
corrective delivery began. It is not the shipped or current implementation.
Its roadmap, execution log, and three reviews remain unchanged as historical
evidence under
[`deliver/superseded-vm-only/`](../feature/vm-recreation-allocation-id-reuse/deliver/superseded-vm-only/).
Those artifacts explain the rejected branch but do not establish behavior for
the completed driver-neutral implementation and must not be cited as current
DELIVER evidence.

ADR-0104 remains a historical record of the reproduced GH #284 VM host-artifact
alias and rejected VM-only remedy. It is superseded for implementation by the
corrective design recorded in the
[feature delta](../feature/vm-recreation-allocation-id-reuse/feature-delta.md)
and the active decisions in ADR-0105, ADR-0106, ADR-0108, and ADR-0109.

## Completed driver-neutral outcome

Every eligible physical allocation replacement now receives a fresh successor
`AllocationId`, independent of whether the current adapter is Exec or VM.
`WorkloadId` remains the stable logical owner of desired generation,
replacement policy, Workload Failure budget, and current-allocation selection.

The existing public action shape is unchanged. `RestartAllocation.alloc_id`
names the accepted terminal predecessor and `RestartAllocation.spec.alloc`
names its distinct fresh successor. Driver matching only projects the already
selected successor into the existing driver payload; it does not choose
identity or action policy.

The completed contract also establishes that:

- the checked allocator advances above both accepted allocation rows and all
  durably issued `WorkloadLifecycleView.restart_counts` keys, preserves gaps,
  ignores malformed suffixes, and does not wrap at `u32::MAX`;
- only the numeric-current accepted `Failed` or `Terminated` allocation can
  hand off replacement ownership; `Draining`, historical terminal rows, and
  View-only reservations cannot become the predecessor;
- the convergence runtime fsyncs the returned View before dispatch, so the
  successor identity is consumed even if no successor row is later accepted;
- the action shim completes the successor outcome before one ordered exact-old
  predecessor driver → mTLS → structural-network cleanup attempt;
- successor failure remains primary when both successor work and predecessor
  cleanup fail, while a cleanup-only failure retains its existing typed error;
  and
- successor lifecycle is published at the fresh allocation key, preserving
  predecessor rows and occurrence history while starting successor
  per-allocation history at zero/`None`.

No public action, method, type, field, trait, parameter, port, store, schema,
driver-policy method, retry owner, cleanup subsystem, lock, or network
mechanism was added. Legacy Exec follows the same driver-neutral physical
identity rule until its separately authorized removal under GH #293.

## Corrective delivery record

The corrective re-DISTILL produced a fresh approved two-step roadmap. It did
not continue or amend the superseded three-step VM-only roadmap.

| Step | Commits | Delivered outcome | Final review |
|---|---|---|---|
| 01-01 — Reserve driver-neutral successor identities | implementation `1c01a1d3`; bounded acceptance reconciliation `4467df40`; DES remediation evidence `8b25d5cb` | Driver-neutral predecessor/successor action construction, checked row-plus-View allocation, terminal handoff, candidate-keyed policy carry, initial reservation, and SystemGc `StartAllocation` preservation. | **APPROVED**, iteration 2; F-01 closed by reconciling stale active acceptance specifications without a production compatibility branch. |
| 01-02 — Complete successor-first exact-ID ownership | implementation `c830b554`; qualified-metal fixture `69957033`; bounded acceptance reconciliation `72cb4fea`; native evidence and live-comment remediation `b295d973` | Successor-first action-shim sequencing, fresh-key publication/history, exact-old cleanup and result precedence, durable fsync/reopen behavior, Service/exit/stream complements, and current qualified-metal host evidence. | **APPROVED**, iteration 2; F-01 through F-03 closed without new API or architecture. |

The current root [execution log](../feature/vm-recreation-allocation-id-reuse/deliver/execution-log.json)
retains every actually executed RED, GREEN, COMMIT, and remediation phase.
With `PYTHONPATH=/Users/marcus/.claude/lib/python`,
`des-verify-integrity` reports: `All 2 steps have complete DES traces`.

## Verification and evidence

- Step 01-01's final affected acceptance run completed with 575 passed and no
  skipped tests across the two affected acceptance binaries. Its focused
  identity, reclamation, payload-projection, checked-allocation, formatting,
  compilation, clippy, and `dst-lint` gates are recorded in the review.
- Step 01-02's focused driver-neutral Sim selection passed 5/5; the mapped
  Service selection passed 3/3; the exit-observer and stream selections passed
  2/2 each; and the active Service terminal-preservation control passed 1/1.
  The affected full Sim, reconciler, core, and control-plane suites and the
  workspace check, clippy, formatting, and `dst-lint` gates are recorded in
  the final review.
- The current qualified-metal receipt is the exact six-test selection in
  [`evidence-01-02-metal.md`](../feature/vm-recreation-allocation-id-reuse/deliver/evidence-01-02-metal.md).
  It ran through `cargo xtask metal run --` on a native,
  non-virtualized x86_64 KVM host and completed **6/6 with exit status 0**.
  The predecessor-artifact case cites the retained raw syscall capture at
  `/var/tmp/overdrive-test-evidence/vm-allocation-ownership/1789326561586357993-1787813/strace.raw`.
  This receipt covers the corrective driver-neutral source and does not rely
  on the older VM-only native evidence.

Mutation testing was **skipped by explicit user direction** for this corrective
DELIVER wave. No mutant ran, no kill rate was measured, and the skip is not
represented as a passing mutation result.

## Finalization disposition

The active feature delta, fresh roadmap, current execution log, two approved
reviews, and current qualified-metal receipt remain in their canonical
repository locations. The superseded VM-only evidence remains preserved in
its archive and was not deleted, moved, or rewritten during corrective
finalization. No verification expectation, example, architecture record, or
product/test artifact was copied or moved.
