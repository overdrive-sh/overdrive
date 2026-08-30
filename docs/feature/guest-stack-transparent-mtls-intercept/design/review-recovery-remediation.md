# DESIGN Recovery Remediation Review

## Review metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Review iteration | 1 |
| Role | Independent solution-architecture reviewer |
| Reviewed commit | `39e256890bfda97b1d2db368a9d462972387b74e` |
| Parent | `89604b407dab319d2c9e01bb34f3f20d3243f97e` |
| Review type | DESIGN recovery remediation |
| Scope | `feature-delta.md` and `design/wave-decisions.md` recovery amendment, checked against the accepted feature inputs, architecture SSOT/ADRs, the architecture-delta assessment, and the current production delta since approved step 02-04 |
| Excluded | Implementation changes, tests, roadmap remediation, mutation testing, and historical-review prescriptions as architecture authority |
| Verdict | **NEEDS_REVISION** |

## Executive assessment

The remediation correctly rejects the filesystem outbox, parallel lifecycle
truth, multi-process data-directory protocol, live-survivor reconstruction,
quarantine protocol, pre-start interception, generic public task primitives,
and retry-owner transfer. It also restores the accepted post-READY/pre-EXEC
ordering, makes awaited mTLS teardown explicit, constrains task ownership to the
allocation-local worker, pins the permitted public API, and gives every major
remediation-era mechanism an explicit disposition.

Two high-severity architecture defects remain. First, the selected owner
shutdown drains allocation guards while the accepted VM lifecycle deliberately
does not stop live VMs at server shutdown. That creates the exact live-guest,
guard-less interval which the boot ordering claims cannot occur. Second, the
terminal-cleanup design promises that VM Artifact Disposal converges every
remaining host residue even though its accepted executor owns only VM scope,
run-directory, and rootfs artifacts; it cannot converge mTLS worker or structural
network residue. Both defects put an implementation crafter at an unresolved
architecture choice, so the recovery is not yet executable as a closed contract.

## Scope and evidence

The review read the complete target diff and the complete amended sections, not
only the summary in `wave-decisions.md`. Evidence included:

- accepted feature issue GH #222, including its 2026-08-27 tap-in-netns scope
  amendment;
- `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md`;
- `docs/feature/guest-stack-transparent-mtls-intercept/design/wave-decisions.md`;
- `docs/feature/guest-stack-transparent-mtls-intercept/distill/test-scenarios.md`
  and `distill/red-classification.md`;
- `docs/feature/guest-stack-transparent-mtls-intercept/deliver/assessment-architecture-delta-since-02-04.md`;
- architecture `brief.md` and ADR-0069, ADR-0070, ADR-0081, ADR-0083,
  ADR-0088, and ADR-0089;
- the cumulative production delta from approved step-02-04 commit `408f5feb`
  through the reviewed commit, with focused source inspection of
  `ServerHandle::shutdown`, `MtlsInterceptWorker::{stop_alloc,shutdown_owner}`,
  `Driver::release_for_exit_emission`, VM reclamation executors, and action-shim
  start/terminal paths.

The reviewed commit changes exactly two design files: 452 insertions and 28
deletions. `git diff --check 89604b407dab319d2c9e01bb34f3f20d3243f97e
39e256890bfda97b1d2db368a9d462972387b74e` completed successfully.

## Findings

### GMR-ARCH-001 — Owner shutdown removes the guard from a VM that intentionally survives server shutdown

- **Severity:** High
- **Dimension:** Security ordering; lifecycle ownership; accepted-architecture compatibility
- **Locations:**
  - `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md:863-865`
  - `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md:996-1001`
  - `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md:1031-1054`
  - `docs/feature/guest-stack-transparent-mtls-intercept/design/wave-decisions.md:410-414`
  - `docs/product/architecture/adr-0083-driver-registry-and-per-driver-allocation-payload.md:1291-1294`
  - `crates/overdrive-worker/src/mtls_intercept_worker.rs:841-903`
  - `crates/overdrive-control-plane/src/lib.rs:1522-1538`

The recovery retains a one-shot `shutdown_owner` which “attempts all allocation
teardowns.” The exact retained `stop_alloc` contract closes admission, joins the
allocation task tree, drains enforced handles, and drops the outbound rule guard.
The production shutdown path invokes this owner shutdown, but it does not stop
driver-owned workloads or author lifecycle state. ADR-0083 explicitly chooses
no shutdown-time VM stop because boot reclamation is the sole survivor-removal
mechanism.

Those decisions do not compose. A graceful SIGINT can complete mTLS worker
teardown while the Cloud Hypervisor process and guest remain live. The recovery
then relies on a later boot to kill that guest, but its stated safety argument
only covers the order “kill old VMM, then stale-rule sweep.” The graceful
shutdown has already removed the rule before either event. The result is a live
guest outside its allocation interception boundary for an unbounded interval
(including the case in which no replacement process is started), contradicting
F3 and the claim that no live guest can cross a guard-less window.

This is not solved by the test-only abrupt owner seam: that seam intentionally
models process loss and cannot define production graceful-shutdown semantics.
Nor may DELIVER invent shutdown-time lifecycle finalization, survivor adoption,
quarantine, or a second public operation to fill the gap.

**Required remediation:** Pin a production owner-shutdown disposition which
preserves the accepted no-shutdown-time-VM-stop decision and keeps every
surviving VM fail-closed until the next boot's kill-before-sweep boundary. The
answer must use the already accepted lifecycle/kernel mechanisms, must state
the exact ownership/drop behavior of allocation rule guards and listeners, and
must remain compatible with the exact one-shot public signatures. If no such
shape is intended, explicitly revise the affected accepted decision through
DESIGN; do not leave the choice to the 02-06 crafter.

### GMR-ARCH-002 — VM Artifact Disposal is assigned residue it cannot observe or remove

- **Severity:** High
- **Dimension:** Recovery authority; mechanism completeness; state-layer hygiene
- **Locations:**
  - `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md:893-898`
  - `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md:1003-1020`
  - `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md:1125-1138`
  - `docs/product/architecture/adr-0083-driver-registry-and-per-driver-allocation-payload.md:1055-1064`
  - `crates/overdrive-control-plane/src/action_shim/reclamation.rs:281-317`

R4 applies to all terminal cleanup owners—awaited mTLS stop, structural
netns/slot teardown, and the driver hook—and says that after any cleanup failure
the lifecycle row may commit and “existing VM Artifact Disposal converges
remaining host residue.” The accepted Artifact Disposal executor has a narrower
authority: it kills the VM cgroup scope and discards VM run-directory/rootfs
artifacts while authoring no row. It has no mTLS worker, network provisioner,
slot allocator, listener, enforced-handle, or allocation-rule capability.

The terminal-row fence then makes a duplicate finalization a zero-effect no-op,
so a failed terminal `stop_alloc` or structural network teardown is not retried
by action replay after the row commits. A private later `stop_alloc` or owner
shutdown can retry retained mTLS handles, and a future boot's ordinary network
GC/rule sweep may remove some other residue, but neither fact makes VM Artifact
Disposal the general convergence owner asserted by R4. The current wording
therefore both overstates accepted recovery and invites DELIVER to widen the VM
reclaimer into another cross-layer cleanup subsystem—the architecture failure
this remediation is meant to remove.

**Required remediation:** Limit each recovery promise to the resources its
accepted mechanism actually owns. State separately what happens after a failed
mTLS stop, a failed structural network teardown, and failed VM artifact cleanup;
identify only already-reachable retry/boot paths, and preserve the typed error
when no accepted bounded convergence promise exists. Do not add a durable
cleanup token, parallel state machine, wider reclamation port, or hidden
terminal-row meaning to make the prose true.

## Fixed-decision compliance

| Fixed decision | Result | Evidence and assessment |
|---|---|---|
| ObservationStore terminal row is the sole durable lifecycle truth | PASS | F1 and R1 remove `terminal-effects/`, receipts, replay, `effect_key`, and lifecycle-event ports; broadcast is explicitly ephemeral and snapshot/relist recovers truth. |
| No outbox or parallel system of record | PASS | Removal is unconditional in the recovery table and exact surface disposition. No ObservationStore row is repurposed as an intent or delivery queue. |
| One process owns one data directory | PASS | F2 rejects shared-directory multi-process coordination and the reclamation lease is explicitly process-local and non-durable. |
| Exact capture-ready through awaited EXEC-release order | PASS | F3 and R6 restore C3/capture, awaited driver start/READY, Running commit, awaited intercept install and D7 baseline, then awaited release. Pre-start interception is removed. |
| Await effects by evolving the existing operation | PASS | `stop_alloc`, worker shutdown, server shutdown, CLI shutdown, and EXEC release are awaited; runtime lookup, detached cleanup, and sibling public methods are forbidden. |
| Roadmap lists are guidance | PASS | F5 permits compiler/criterion fallout while leaving the roadmap outside this DESIGN edit. |
| Review prescriptions are provenance, not architecture | PASS | The recovery expressly supersedes journal/quarantine/survivor/pre-start prescriptions and treats old review artifacts as history. |
| Cleanup/recovery promises stay within accepted mechanisms | **FAIL** | GMR-ARCH-002 assigns mTLS/network residue to the VM Artifact Disposal executor, whose accepted authority does not include it. |
| No hidden state machine or multi-instance protocol | PASS | Pending cleanup tokens, special Pending meaning, route/event records, retry state machines, and cross-process leases are removed. |
| No pre-start intercept or hidden survivor reconstruction | PASS | Both mechanisms and their tests/call sites are explicitly removed. |
| Private allocation-local task ownership only | PASS | R3 removes the public generic core module and confines registration/stop completion to one mTLS allocation; resolver/server/driver/reclamation reuse is forbidden. |
| Resume fresh 02-05, then fresh 02-06, preserving 02-04 | PASS | The recovery boundary is explicit and treats `408f5feb` as comparison-only, not reset authority. |
| Exact API with no invented surface | PASS | The four async signatures and retained/removed cross-crate surfaces are exact; private helper freedom is bounded. |
| Live VM remains protected through replacement recovery | **FAIL** | GMR-ARCH-001 leaves graceful owner shutdown free to delete the guard before the accepted boot reclaimer kills the surviving VM. |

## Mechanism-disposition coverage

| Remediation-era mechanism | Disposition coverage | Review result |
|---|---|---|
| Failed-start cleanup ownership/retry protocol | `RESHAPE` to one awaited `VmDriver::start` owner; remove pending owner/token/public cleanup carrier | Covered and coherent for VM-owned start resources |
| Pure same-attempt terminal fence | `RETAIN` | Covered; exact duplicate finalization is zero-effect and Platform Reclamation is the only same-id reopen |
| `OwnedTaskSet` / `CompletionFence` | `RESHAPE` to private allocation owner/completion | Covered; no generic cross-layer reuse survives |
| Allocation stop | `RETAIN / RESHAPE` as awaited, fallible, cancellation-safe per-allocation stop | Covered, subject to GMR-ARCH-001/002 at outer shutdown and post-terminal failure boundaries |
| Server shutdown/retry owner | `RESHAPE` to async, fallible, one-shot diagnostics only | Public retry transfer is correctly removed; production survivor safety is incomplete per GMR-ARCH-001 |
| Execution-time reclamation arbitration | `RETAIN, NARROW` process-local claim over the VM supervision map | Covered; no persistence or cross-process authority is implied |
| Live-survivor reconstruction | `REMOVE` | Covered, including plan/apply/recover call sites |
| Recovery quarantine | `REMOVE` | Covered, including kernel type, userdata, batches, APIs, and tests |
| Pre-start intercept and rollback | `REMOVE` | Covered; fresh and same-id restart return to the same post-READY gate |
| Terminal outbox/event protocol | `REMOVE` | Covered comprehensively across persistence, ports, receipts, replay, event key, and terminal hook |
| Public cleanup/error additions | `REMOVE / NARROW` | Covered; the retained duplicate-start cause and reclamation claim are bounded |
| Abrupt test-owner seam | `RETAIN` behind integration gates only | Covered; it authors no lifecycle truth and is not accepted as a substitute for boot-reclamation evidence |
| Tests and compiler fallout | `RESHAPE` | Covered by behavior/protocol classification and explicit no-mutation boundary |
| VM Artifact Disposal as a universal residue owner | No valid disposition | **Not covered honestly; GMR-ARCH-002** |

## Architecture quality summary

| Dimension | Assessment |
|---|---|
| Requirements alignment | Strong except for the two recovery/shutdown contradictions above |
| System-of-record discipline | Strong; one durable lifecycle truth and no queue semantics in ObservationStore |
| Component boundaries | Strong for task ownership and public surface; incomplete at owner shutdown and residue ownership |
| Concurrency and ordering | Strong for start, terminal-row, task-registration, and reclamation races; unsafe graceful-shutdown ordering remains |
| Failure semantics | Typed and awaited at the named APIs, but terminal cleanup overpromises convergence beyond the available capability set |
| Evolvability | Good removal of speculative protocols; exact surface constraints prevent another review-driven API accretion cycle |
| Testability | Mechanism-specific test retention/removal is explicit and preserves independent Rust/expectation boundaries |

## Independent verification

- `git diff --check` for the reviewed parent/target range: **PASS**.
- Target file scope: **PASS**; only the two authorized DESIGN artifacts changed.
- Fixed-decision trace: **12 PASS, 2 FAIL**.
- Remediation-mechanism classification: comprehensive except for the invalid
  universal Artifact Disposal ownership claim identified in GMR-ARCH-002.
- Public API audit: no unpinned recovery API is sanctioned by the amendment.
- Design/code executability audit: **FAIL** at graceful owner shutdown and
  post-terminal cleanup failure, where the prose requires behavior not supplied
  by the selected accepted mechanisms.
- No code, tests, roadmap, assessment, or mutation configuration was changed or
  executed by this review.

## Remediation gate

The original designer must remediate both high-severity findings in the DESIGN
artifacts and return the resulting commit for a fresh review iteration. The
remediation must not reintroduce an outbox, parallel lifecycle truth, durable
cleanup token, live-survivor join, quarantine protocol, pre-start intercept,
generic task primitive, public retry owner, shared-directory protocol, or new
public API. It must make graceful owner shutdown and each residue class
executable using only the selected accepted mechanisms.

## Verdict

**NEEDS_REVISION**

Open findings: **0 Critical, 2 High, 0 Medium, 0 Low**.

DELIVER must not resume at 02-05 RED until this recovery DESIGN has been
remediated and independently approved.
