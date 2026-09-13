# Evolution — VM recreation allocation ID reuse (GH #284)

**Finalized:** 2026-09-13. **Feature slug:**
`vm-recreation-allocation-id-reuse`. **Waves:** DESIGN → DISTILL → DELIVER →
FINALIZE. **Completion record:** [roadmap](../feature/vm-recreation-allocation-id-reuse/deliver/roadmap.json),
[execution log](../feature/vm-recreation-allocation-id-reuse/deliver/execution-log.json),
and the three step reviews under
[`docs/feature/vm-recreation-allocation-id-reuse/deliver/`](../feature/vm-recreation-allocation-id-reuse/deliver/).

## Outcome and accepted decision

Every physical VM execution now receives a fresh `AllocationId`, while the
existing `WorkloadId` and `WorkloadLifecycle` remain the stable logical owner
of desired state, replacement policy, and current-allocation selection. This
prevents predecessor and replacement executions from sharing the run
directory, beacon socket, cgroup scope, rootfs clone/index, or VMM process
identity that produced the reproduced `EADDRINUSE` and late-cleanup `ENOENT`
failures.

The implementation follows the accepted [feature delta](../feature/vm-recreation-allocation-id-reuse/feature-delta.md)
and [ADR-0104](../product/architecture/adr-0104-vm-recreation-fresh-allocation-identity.md):
VM initial placement, generation replacement, Workload Failure replacement,
and Platform Reclamation reserve an issued ID in the existing
`WorkloadLifecycleView` before dispatch and use the existing
`StartAllocation` action with matching action, allocation-spec, and SVID
identity. Accepted rows alone determine the numeric-current allocation, and
predecessor rows retain their own terminal history. Exec recovery keeps the
existing same-ID `RestartAllocation` behavior.

DELIVER added no public method, type, field, action variant, trait, parameter,
dependency, persistence store, current pointer, cleanup subsystem, retry,
sleep, global lock, or compatibility alias. It implemented the approved
allocation-identity decision through existing actions, View fields, row
publication, and execution-scoped cleanup capabilities; it made no further
architecture or persistence decision.

## Delivery record

| Step | Commit | Activated scenarios | Delivered outcome |
|---|---|---:|---|
| 01-01 — Reserve fresh VM execution identities | `a0f19ebe8008f85136e8f619cb909de1243672f6` | 14 | Fresh VM ID selection and pre-dispatch reservation across initial, generation, Workload Failure, and Platform Reclamation paths; candidate-keyed policy carry; numeric-current/history isolation; checked attempt exhaustion; unchanged Exec same-ID recovery. |
| 01-02 — Prove production-owner restart isolation | `e7fd61c74e227be7ea0e9e12eff18ce9b1b40b48` | 1 | Seed 257205 crosses rejected Running publication, awaited cleanup, redb runtime close/reopen, restored reservation, higher-ID recovery, delayed predecessor disposal, repeated reclamation, and final cleanup through registered production owners. |
| 01-03 — Prove native artifact ownership | `d4db67f6f34b52244ae3a5f215bfc08ef03f4ecc` | 6 | Qualified-metal regressions cover retained predecessor history, zero/None replacement history, clean rootfs clone, SVID and mesh re-enrolment, typed fail-closed re-enrolment, disjoint beacon/artifact/process ownership, and exact final cleanup. |

All 21 DISTILL scenarios are active. Step 01-01 contains the only production
change; steps 01-02 and 01-03 activate the independent production-owner and
native host-effect evidence lanes.

## Reviewed fixture-only corrections

The step reviews approved two bounded corrections to pre-authored fixtures:

- Step 01-02 snapshots network provision and teardown vectors before comparing
  them. This avoids a Rust temporary-guard lifetime deadlock from locking the
  same mutex twice inside one assertion while preserving the counts, values,
  order, seed, and bounded-change oracle.
- Step 01-03 changes the test helper's logical-clock settling delay from 25 ms
  to one second so the production `ProbeRunner` receives its existing
  one-second interval, and observes predecessor artifact presence at the valid
  Running boundary before the terminal transition removes its vsock. No
  expected artifact, ordering, row, strace, replacement-survival, or final
  cleanup assertion was weakened.

These are test-fixture corrections only; neither adds or changes production
behavior.

## Verification and review disposition

- Step 01-01 passed its 14/14 focused scenarios, 19/19 full reconciler
  acceptance tests, 557/557 full core acceptance tests, and all three retained
  proptest replays with novel generation disabled.
- Step 01-02 passed its 2/2 focused seeded owner-path selection and 13/13
  affected control-plane preservation tests.
- Lima-routed workspace or owning-crate compile, clippy with `-D warnings`,
  formatting, and `dst-lint` checks passed as recorded by the step reviews.
- The exact six-test step 01-03 selection passed **6/6** on a qualified native,
  non-virtualized x86_64 KVM host through `cargo xtask metal run --`. Lima was
  used only for compile, clippy, and active-test selection for that lane.
- The retained raw syscall capture is
  `/var/tmp/overdrive-test-evidence/vm-allocation-ownership/1789290158143227525-1773360/strace.raw`.
  It records successful distinct beacon binds, rejects replacement-path
  `EADDRINUSE`, and establishes replacement creation before predecessor
  `unlink`/`rmdir` cleanup.
- `des-verify-integrity` passed for
  `docs/feature/vm-recreation-allocation-id-reuse/deliver/`; all three steps
  have complete RED → GREEN → COMMIT traces, including recorded failed GREEN
  attempts before their passing retries where applicable.

The independent native-Markdown reviews are all final `APPROVED`:
[01-01](../feature/vm-recreation-allocation-id-reuse/deliver/review-01-01.md),
[01-02](../feature/vm-recreation-allocation-id-reuse/deliver/review-01-02.md),
and [01-03](../feature/vm-recreation-allocation-id-reuse/deliver/review-01-03.md).
The first 01-01 iteration's proposed reviewer-only outcome-anchor finding was
rejected on re-review as out of scope and unsupported by repository policy;
no test or production remediation was authorized.

Mutation testing was **skipped by explicit user direction** at the final
DELIVER gate. No mutant ran, no kill rate was measured, and this disposition
is a user-directed skip rather than a passing mutation result.

## Finalization disposition

The accepted feature/design records already live at their permanent repository
homes, so no architecture, ADR, scenario, UX, example, or expectation artifact
was copied or moved. The complete feature workspace remains under
[`docs/feature/vm-recreation-allocation-id-reuse/`](../feature/vm-recreation-allocation-id-reuse/)
as historical wave evidence.
