# VM same-ID restart — ending-authorship ruling

Date: 2026-09-07. Scope: `service-kind-vm-workloads`, phase 02, step `02-03`.
Status: **Accepted**, following [independent DESIGN review, iteration 1 — APPROVED](review-adr-0100.md).
No implementation, test, expectation, DES event, or commit is part of this
ruling. The separate E10 expectation-contract disposition was subsequently
approved by the user on 2026-09-07, as recorded below; implementation review
remains pending.

## Decision

Correct the proven defect at the VM exit watcher's existing atomic claim gate:
the watcher may transition or abandon **its own accepted guest session's Live
entry**, never whichever entry currently occupies the allocation ID. Reuse the
existing `Arc<BeaconWriter>` as session identity, held by the watcher only as a
`Weak<BeaconWriter>`. No new attempt identifier, claim map, persistence, task
supervisor, or public API is needed. The exact private contract is
[Accepted ADR-0100](../../../product/architecture/adr-0100-vm-exit-watcher-session-ownership.md).

This is sufficient for seed 257205's second-ending failure. It deliberately
does **not** make an already-authored Failed ending into an operator Terminated
ending, guarantee a successful replacement start, or claim E10 passes.

## Evidence and complete owner path

The [full diagnosis](../../../analysis/e10-vm-early-exit-root-cause.md) and
[`e10_vm_early_exit_spike.rs`](../../../../crates/overdrive-sim/tests/e10_vm_early_exit_spike.rs)
were read in full. The two final-source seed-257205 runs recorded there
(`b2ed992a-95fb-4f9b-ac98-3b89e55b1d69`,
`01fbba8b-69e1-4851-93ae-eec9551ee2f4`) each fail the authorship invariant and
pass the reclamation control: nextest 100, outer xtask 1. This design did not
rerun or rewrite them. Inspected driver/shim hashes still match the diagnosis:
`e1526b750076e96bb0b540e5f5b44dc4c3886706db00aa2804d93e4e154ed0d0` and
`5a99b5b6eb2f13715bd2e08e3582cfe3c4d738fb09369a85b971ce16a1d0df56`.

| Boundary | Current production fact, before this proposal |
| --- | --- |
| Production owner | `overdrive serve` registers the reconcilers; `spawn_convergence_loop` awaits dispatch serially and checks shutdown afterward (`crates/overdrive-control-plane/src/lib.rs:3377,3400`). The registered convergence entry point drives hydration, decisions, persisted view, validation, and dispatch (`src/reconciler_runtime.rs:1703`). |
| First ending | Real ProbeRunner's HTTP failure is consumed by ServiceLifecycle; FinalizeFailed publishes Failed(35) / StartupProbeFailed, then releases supervision (`src/action_shim/mod.rs:1778,1789,1791`). Networking teardown observes the original VMM still alive. |
| What release means | `VmDriver::release_supervision` removes the map entry (`crates/overdrive-worker/src/vm_driver.rs:1870`). It does not await VMM death. This is the accepted ending-authorship release, not a missing-stop defect. |
| Intervening attempt | WorkloadLifecycle emits its same-ID RestartAllocation under existing policy (`crates/overdrive-reconcilers/src/workload_lifecycle.rs:951,1052,1429`). The shim accepts VM stop's NotFound and awaits replacement start (`action_shim/mod.rs:2280,2411`). |
| Actual failed start | VmDriver inserts Starting before binding the allocation-derived beacon (`vm_driver.rs:1139,1156,1199`). Existing pathname causes EADDRINUSE. Its existing failed-start unwind kills that allocation scope (`:1029,1047`), containing the original VMM; no replacement VMM is created. |
| Authorship violation | The original watcher is the only spawned watcher. Its allocation-keyed guard accepts Starting or Live (`:935`), takes the replacement's claim, and emits a natural crash (`:2128`). The observer fresh-stamps and accepts Failed(36), clearing the startup terminal and appending a natural-crash occurrence (`worker/exit_observer.rs:458–476`). |
| Current cancellation/retry | No owner is cancelled in the witness. Convergence awaits the action. The exit observer checks cancellation before receive, then awaits its consumed event's retry procedure (`worker/exit_observer.rs:198,206,342`) and releases at its end (`:300`). Failed replacement publication does not enter ADR-0099's Running-only repair (`action_shim/mod.rs:2655`). |
| Control | Without intervening restart, registered VmReclamation acquires EndingInFlight, kills/discards, and preserves the **entire** Failed(35) row and occurrence history (`action_shim/reclamation.rs:63,299`). |

The native trace independently proves EADDRINUSE, the original scope's
cgroup.kill write, and original VMM SIGKILL before the first 30-second sweep.
Private historical claim transitions were not instrumented; their attribution
is source-supported inference. The fresh Sim run observes the second natural
ending through the sole production watcher/observer emission path. Sim signal
9 and native absent signal are not asserted to be identical poll outcomes.

## Necessity and reuse

The allocation ID persists across restart, but the watcher belongs to exactly
one accepted guest session. Production creates its BeaconWriter once in the
READY-winning arm and installs it in that same LiveVm before spawning that
session's watcher (`vm_driver.rs:1523–1545`). There is no watcher for Starting,
nor any beacon reconnect/replacement path within a LiveVm. This existing
identity is sufficient; a second token or a durable generation is unnecessary.

A Weak reference neither retains the writer/socket owner nor delays its Drop;
it retains only backing allocation identity. Rust documents both that lifetime
distinction and identity comparison in [Weak](https://doc.rust-lang.org/std/sync/struct.Weak.html#method.ptr_eq).
The map comparison and mutation remain one synchronous locked operation.
Both claim acquisition **and the failed-claim Drop** must use the same ownership
predicate, or suppressing the event would still let the old guard erase the
replacement's entry. This is one correction with two mutation sites, not a
generalized attempt-fencing subsystem.

The [withdrawn finalization proposal](vm-finalize-failed-ownership-ruling.md)
stays withdrawn. VmReclamation remains the terminal-unclaimed disposal owner.
[ADR-0099](../../../product/architecture/adr-0099-restart-running-write-acknowledgement.md)
remains the distinct successful Exec restart acknowledgement correction;
ADR-0098's existing network lookup recovery is preserved, neither superseded
nor made a dependency here.

## Same-ID start failure is distinct from a second ending

The beacon collision and its unwind are **observed**, not hypothetical. However,
the seeded invariant does not require every authorized start to succeed: it
expressly permits a rejected replacement to record its own start failure. With
the stale watcher refused, the existing shim may accept Failed(36) with its
typed driver-start classification and `terminal: None`; this is the replacement
attempt's rejection, not a second natural ending of the original VM. No
replacement Running occurrence or restart-count increment is justified.

The supplied evidence does not establish a separate failure of eventual
same-ID restart convergence: the test stops after one rejected replacement,
and the native example records stop intent rather than leaving run intent
standing through the next policy-selected attempt. Neither is proof that the
subsequent permitted retry cannot converge. No requirement to move disposal
before start, change cgroup cleanup, change the retry budget, or delay startup
finalization is established by this invariant. Such a remedy is **not** an
unavoidable dependency of preserving the original instance's ending.

Do not call the observed bind collision harmless or fixed. It remains an
observed startup rejection; any claimed safety/liveness defect beyond the
second ending needs its own seeded failing invariant against the existing
production owner path before becoming another implementation requirement.
This ruling neither authorizes nor specifies such an expansion.

## Expected operator trajectory and E10 boundary

| Actual ordering | Expected meaning under the existing owners |
| --- | --- |
| Stop is reconciled while the allocation is Running | WorkloadLifecycle selects StopAllocation; the stop path authors Terminated / Stopped(Operator), and owns its existing cleanup. HTTP204 is the native positive control. |
| Startup failure is authored; stop intent is observed before another start | Failed remains the authored ending. Stop suppresses further starts; terminal-unclaimed VmReclamation disposes the surviving VMM/artifacts without reauthoring that ending. |
| The witnessed replacement fails before stop is reconciled | Its own driver-start rejection may become the current Failed row. The old VM must add no natural-crash occurrence. Stop intent does not retroactively make this never-started replacement Running or Terminated. |
| An authorized replacement actually reaches accepted Running before stop is reconciled | That replacement is selected for the ordinary stop path. Its Terminated ending does not imply that the earlier startup-failed instance was rewritten as operator-stopped. |

These distinctions follow the current stop branch, which selects **only
Running** (`workload_lifecycle.rs:637–655`), and the no-write terminal disposal
executor, not the cleanup ledger. A stop arriving during restart does not cancel
that awaited restart; it affects a subsequent evaluation. No deterministic
Terminated-only outcome follows for E10's startup-failed cells merely by fixing
the stale watcher, or even by removing the first bind collision.

At independent DESIGN review, `wait_for_workload_stop` was Terminated-only
(`examples/service-kind-vm-workloads/run-example.sh:215–227`). S-SVM-26 and
E10's README anchor HTTP status agreement, non-disclosure and owned cleanup,
not reclassification of every failed allocation. The authorship correction alone
cannot promise E10 completion with that helper applied to every failed cell.
The independent review therefore left the expectation-contract disposition
unresolved and authorized no helper change.

**Subsequent user disposition, 2026-09-07:** after the orchestrator explained
that an allocation Running when stopped reaches Terminated, whereas an
allocation already failed startup before stop may remain Failed with cleanup
verified without erasing its failure, the user replied: **"yes, correct it."**
This approves correcting the E10 example/expectation contract, not changing
product lifecycle semantics. Distinguish the witnessed startup-failure disposal
from stopping a Running allocation or replacement. Do not accept arbitrary
crashes, substitute a blanket Terminated-or-Failed assertion, remove lifecycle
assertions, or manufacture a production stop transition. The separate seeded
authorship regression continues to detect the duplicate natural-crash ending.

This later user authorization resolves the prior choice; it does not amend the
historical independent review or claim that review approved the changed test.
The narrow example/expectation correction still requires implementation review
and full verification. All eight-cell HTTP obligations and owned-resource
cleanup checks remain required. No implementation or expectation file is
changed by this documentation update.

## Handoff

Authoritative implementation contract: Accepted ADR-0100 D1–D3, independently
approved in [iteration 1](review-adr-0100.md). Required evidence: unchanged
seed-257205 invariant and no-restart
control, source-local ownership predicate complements, existing natural-exit,
Running-gate, stop, and reclamation tests. E10 remains an independent native
built-product gate; passing the Sim regression is not permission to mark it
satisfied. Each changed/live test needs its per-test Contract Shape declaration.

Intended production edit boundary is `overdrive-worker/src/vm_driver.rs` only,
with tightly related test fallout. No driver trait, store, action shim,
reconciler, host cleanup, broker, timestamp, schema, or expectation edit is
needed for this authorship correction. Existing C4 L1/L2 Service-health diagrams
in [c4-diagrams.md](../../../product/architecture/c4-diagrams.md)
are adopted unchanged; all boundaries and deployments are unchanged.
