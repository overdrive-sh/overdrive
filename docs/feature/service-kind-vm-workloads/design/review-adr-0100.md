# DESIGN review — ADR-0100: VM exit watchers may claim only their own accepted session

| Field | Value |
| --- | --- |
| Feature | `service-kind-vm-workloads` |
| ADR | Proposed ADR-0100 — VM exit watcher session ownership |
| Related ruling | `design/vm-restart-ending-authorship-ruling.md` |
| Roadmap boundary | Phase 02, step `02-03`; bounded design amendment before implementation |
| Review type | Fresh isolated `nw-solution-architect-reviewer` DESIGN review |
| Iteration | 1 |
| ADR-0099 disposition | Not reopened; this is an independent VM watcher review |
| Verdict | **APPROVED** |

## Scope and verdict basis

This review covers only the production-reachable same-allocation-ID VM watcher
authorship defect established by the native trace, the seeded Sim invariant,
and ADR-0100. It verifies the proposed reuse of the accepted session's
existing `BeaconWriter` identity, the exact private contract in ADR-0100 D1–D3,
the existing lifecycle owners, and compatibility with stop and reclamation.

The mechanism is necessary and appropriately bounded. The current
`ClaimGuard` decides from the allocation key alone, so an old watcher can
claim a replacement `Starting` or `Live` entry. Capturing a `Weak` to the
same `Arc<BeaconWriter>` that is already stored in that accepted session's
`LiveVm`, then applying that identity predicate in both the atomic claim and
failed-claim `Drop`, closes the demonstrated path without a public API,
persistence field, attempt/generation allocator, task supervisor, or new
owner.

No blocking, high, medium, or low design finding remains. Approval is for the
ADR's narrow authorship correction, not for implementation completion, E10
completion, or a change to the strict E10 helper. Step `02-03` still needs its
implementation review and independent black-box evidence. The existing
failed-instance-versus-operator-stop product decision remains explicit below;
it is not silently resolved by this ADR.

## Material reviewed

I read `AGENTS.md`, `CLAUDE.md`, all seven mandatory repository rule files,
the `nw-solution-architect-reviewer` definition, and its mandatory
`nw-sar-critique-dimensions` and `nw-roadmap-review-checks` skills. I also read:

- ADR-0100 and the bounded `vm-restart-ending-authorship-ruling.md`;
- the scoped feature-delta assumptions, architecture brief §105a.3, and the
  approved `02-03` roadmap entry;
- the current `VmDriver`, action-shim, exit-observer, WorkloadLifecycle, and
  VmReclamation owner paths;
- the full `e10-vm-early-exit-root-cause.md` diagnosis and the existing
  `crates/overdrive-sim/tests/e10_vm_early_exit_spike.rs`;
- the relevant accepted VM, lifecycle, restart-authority, and terminal
  reclamation contracts.

No implementation, test, expectation, SSOT, DES, or roadmap file was edited.
No commit or native-host execution was performed.

## Evidence and revalidated production path

The evidence distinguishes directly captured host effects from source-supported
private-state attribution. The native trace directly captures the second
beacon `bind(2)` returning `EADDRINUSE`, the allocation scope's `cgroup.kill`
write, and SIGKILL of the original VMM before the first ordinary 30-second
sweep (`docs/analysis/e10-vm-early-exit-root-cause.md:94–111`). It does not
instrument the private map claim. The fresh Sim run supplies that complementary
owner-path evidence.

The complete current path is:

1. Production `overdrive serve` registers the reconcilers. The convergence
   loop drains evaluations and awaits each `run_convergence_tick` serially
   (`crates/overdrive-control-plane/src/lib.rs:3332–3403`). The runtime then
   hydrates, validates, persists the reconciler view, and dispatches through
   the real action shim (`crates/overdrive-control-plane/src/reconciler_runtime.rs:1660–1783`).
2. The first VM reaches `Running`: `VmDriver::provision_vmm` claims
   `Starting` before creating the run directory and beacon listener
   (`crates/overdrive-worker/src/vm_driver.rs:1139–1213`), then installs
   `LiveVm` (`:1452–1466`). In the READY-winning arm, the existing code creates
   one `BeaconWriter`, stores its strong `Arc` in `LiveVm`, and spawns the one
   exit watcher (`:1520–1546`).
3. The real `ProbeRunner` observes the HTTP 302 startup failure. ServiceLifecycle
   owns that startup decision; `FinalizeFailed` writes the existing
   `Failed(35)` / `StartupProbeFailed` row, then calls
   `release_supervision` (`crates/overdrive-control-plane/src/action_shim/mod.rs:1759–1795`).
   `VmDriver::release_supervision` removes the map entry but does not await VMM
   death or unlink the already-present beacon pathname
   (`crates/overdrive-worker/src/vm_driver.rs:1868–1872`).
4. WorkloadLifecycle's existing same-ID restart policy emits
   `RestartAllocation` (`crates/overdrive-reconcilers/src/workload_lifecycle.rs:950–1051,1426–1465`).
   The action shim accepts `Driver::stop` returning `NotFound` and awaits the
   existing replacement path (`crates/overdrive-control-plane/src/action_shim/mod.rs:2272–2302,2409–2417`).
5. The replacement inserts `Starting` before binding the allocation-derived
   beacon (`vm_driver.rs:1139–1157,1196–1213`). The retained pathname makes
   the bind fail; existing `attempt_start_cleanup` calls `cgroup.kill` and
   removes the scope/run directory (`vm_driver.rs:1029–1053`). No replacement
   VMM is created in this branch.
6. The original watcher is the sole watcher that can report that VMM's death.
   After its existing guest-report drain and Running-confirmed gate, the
   current allocation-key-only `ClaimGuard::try_begin_ending` accepts either
   `Starting` or `Live` (`vm_driver.rs:929–944`), transitions the replacement
   slot to `EndingInFlight`, and emits another natural `ExitEvent` (`:2128–2145`).
   The exit observer accepts the event, writes the competing Failed row, and
   unconditionally releases supervision at the bottom of its loop
   (`crates/overdrive-control-plane/src/worker/exit_observer.rs:198–206,284–301`).
7. The checked-in spike drives these registered production owners, the real
   `VmDriver`, `ProbeRunner`, and observer without seeding an observation row
   or private claim. The final-source seed-257205 runs each recorded one failing
   authorship invariant and one passing no-restart reclamation control; the
   failing assertion is the second natural-crash occurrence after one VMM
   creation (`docs/analysis/e10-vm-early-exit-root-cause.md:229–257`).
8. The corrected seed-257204 control independently proves the terminal,
   unclaimed VmReclamation path: after `Failed(35)` authorship is released,
   registered reclamation kills/discards the surviving substrate while keeping
   the row and occurrence history unchanged
   (`design/vm-finalize-failed-ownership-ruling.md:7–31,101–129`).

This is a reachable production owner path, not a forced task abort or a
hand-wired map mutation. It proves the duplicate-ending safety failure. It
does not prove that every later same-ID retry fails to converge.

## ADR quality and architectural assessment

### D1 — Public, persisted, and ownership contract

**Approved.** ADR-0100 D1 retains the existing public `Driver` methods,
`AllocationHandle`, `ExitEvent`, `VmSupervision`, `LiveVm`, and `LiveMap` shapes
and explicitly forbids a new public method, type, enum variant, field,
parameter, action, event, configuration, dependency, or persisted shape
(`ADR-0100:33–45`). The private implementation delta is separately and exactly
specified in D3. Starting and Live remain the supervised Held variants;
EndingInFlight, status mapping, release points, observer retry/release,
operator stop transition 3b, boot cleanup, and reclamation leases remain
unchanged.

This matches the accepted brief's claim lifecycle: the map value is the
`VmSupervision` sum type and the claim is held until the ending is authored or
authorship is abandoned (`brief.md:9012–9058`). The proposal narrows only
which watcher may perform its existing transitions 3 and 4; it does not move
the release boundary or reinterpret a startup failure.

### D2 — Existing session identity, lifetime, and atomicity

**Approved.** The identity choice is necessary and minimal:

- Current production has exactly one `BeaconWriter` for one accepted READY
  session and installs it in that same `LiveVm` before spawning that session's
  watcher (`vm_driver.rs:1520–1546`). A replacement start creates a distinct
  writer; no reconnect/replacement writer exists within a `LiveVm`.
- `Arc::downgrade` retains only allocation identity. It does not keep the
  writer, socket, writer task, or VMM alive after `LiveVm` release. The
  `BeaconWriter` `Drop` still signals/aborts its writer task
  (`vm_driver.rs:739–759`). Rust's `Weak::ptr_eq` compares backing allocation
  identity, and a live `Weak` prevents that backing allocation from being
  reused for a different writer. No PID, pathname, raw pointer integer, value
  equality, counter, or second registry is introduced.
- `ClaimGuard::try_begin_ending` already holds the one `LiveMap` mutex across
  its read and `Live → EndingInFlight` mutation (`vm_driver.rs:935–944`). The
  proposed predicate adds only `Live(live_vm)`, `beacon: Some`, and
  `Weak::ptr_eq` to the watcher's captured witness. The check and mutation
  remain one synchronous locked operation, and the `ExitEvent` remains gated
  by the boolean result.
- The same predicate in `ClaimGuard::Drop` is load-bearing. If the old watcher
  refuses a replacement's `Starting` or different-session `Live`, its failed
  guard must not remove that replacement entry. The emitted guard remains a
  no-op, and `EndingInFlight`/absence remain no-ops. This closes both mutation
  sites of the demonstrated stale-owner path.

The proposed private signatures in ADR-0100 D3 (`ADR-0100:84–100`) are
feasible: `Weak<BeaconWriter>` is appended after the existing gate receiver,
`ClaimGuard::new` remains `const`, and no async work is added to the locked
section or to synchronous `release_supervision`.

### D3 — Stop, reclamation, and failure compatibility

**Approved.** The identity check does not change the existing stop/reclamation
ownership:

- If operator stop wins, `VmDriver::stop` extracts the writer and atomically
  changes Live to `EndingInFlight` before its existing SHUTDOWN, terminate, and
  cleanup sequence (`vm_driver.rs:1631–1680`). A watcher that later wakes sees
  EndingInFlight and emits nothing; status remains `NotFound` while the claim
  remains held.
- If the natural watcher wins first, it performs the existing atomic
  Live-to-EndingInFlight transition and emits one event. Stop then sees the
  existing non-Live state. No new stop race or event fence is invented.
- If ServiceLifecycle has already authored the startup terminal and called
  `release_supervision`, the old `LiveVm` and strong writer are dropped. Its
  weak witness cannot match a replacement's new writer, so the old watcher
  cannot claim or remove the replacement. VmReclamation remains the existing
  owner for terminal-unclaimed disposal.
- Reclamation still refuses every existing Starting, Live, and EndingInFlight
  entry through `ReclamationLease::try_acquire`; terminal disposal still
  performs host kill/discard without writing a new observation
  (`vm_reclamation.rs:163–190`; `action_shim/reclamation.rs:51–80,281–317`).

The design does not change guest report draining, classification, cgroup
accounting, channel send, observer retries, or the accepted residual in which
an observer task's death can leave EndingInFlight until process restart. Those
are existing boundaries and are correctly outside this reproduced defect.

### D4 — Lifecycle Gate Ownership

**Approved.** ADR-0100 contains the required existing ownership matrix and
separate Gate G-100 (`ADR-0100:102–146`). The declaration names the existing
owners precisely:

| Existing signal/state | Owner and promise preserved |
| --- | --- |
| VM natural `ExitEvent` | The VmDriver watcher for its own accepted session, after the existing Running-confirmed gate |
| Supervised allocation set | VmDriver; every `Starting`, `Live`, and `EndingInFlight` entry remains claimed |
| Startup failure / Stable | ServiceLifecycle; startup observations and Stable decision remain separate from driver start and cleanup |
| Same-ID restart | WorkloadLifecycle; existing row, intent, and budget decide another attempt |
| Terminal artifact disposal | VmReclamation; disposal does not rewrite Failed to Terminated |

Gate G-100 also supplies the required fields: concrete evidence and owner
path, exact promise, affected result, refusal projection, unaffected owners and
states, no-await ordering, counterexample, and evidence lanes. It explicitly
states that absence, `Starting`, pre-beacon `Live`, different-session `Live`,
and `EndingInFlight` produce false/no emission/no removal; no new typed refusal
error is needed.

The required boundary scenarios are present and executable:

| Boundary scenario | ADR-0100 obligation and review result |
| --- | --- |
| Gate available | Matching accepted-session `Live` plus writer identity advances to `EndingInFlight` and permits one existing event; source-local and natural-exit tests cover it. |
| Gate unavailable or timed out | There is no new awaited claim gate or timeout. Missing/absent state, pre-beacon state, or mismatched identity is a synchronous false/no-op; existing guest-report/Running-gate timeout and typed classification remain unchanged before this predicate. |
| Unrelated state | `Starting`, another session's `Live`, `EndingInFlight`, and absence cannot be transitioned or removed by this watcher; reclamation and stop owners retain their existing meanings. |
| Late success | A late original watcher cannot transition or remove a replacement entry. An already-emitted event remains governed by the existing observer/release path; ADR-0100 does not claim a new global event fence. |
| Disconnect/reconnect | Existing drain, cancellation, observer retry, and release behavior remains; there is no accepted-session reconnect path, and a closed transport does not grant authorship. |
| Feature disabled | No switch or new activation path exists; Exec behavior, VM pre-READY behavior, and non-applicable paths remain unchanged. |

This is a narrow correction to an existing authorship gate, not a new
readiness or lifecycle gate. The proposal preserves the distinction between
READY, Running, Stable, backend health, and terminal disposal.

### D5 — Testability and evidence separation

**Approved.** The seeded `overdrive-sim` failure is the correct evidence lane
for the ordering/ownership defect: it uses registered WorkloadLifecycle,
ServiceLifecycle, and VmReclamation, the production convergence entry point,
the real VmDriver/ProbeRunner/observer, and only existing driven-port
substitutions for simulated host resources. It does not spawn the production
binary, seed an observation row, set a private claim, or assert native kernel
effects. The independent native trace supplies those host effects; the native
E10 run remains a separate built-default-feature black-box gate.

ADR-0100's verification obligations correctly require both necessity and fix
evidence: the unchanged seed-257205 schedule must lose its second natural
crash, the no-restart reclamation control must preserve row/history and remove
artifacts, and source-local complements must exercise matching and
non-matching map states with complete map deltas (`ADR-0100:148–172`).

One bounded test fallout is explicit and acceptable, not a design finding:
`vm_driver.rs:2729–2812` currently invokes private `run_exit_watcher` without
the new witness and uses a synthetic `Starting` entry so the watcher can emit.
The implementation must append the specified `Weak` argument and transition
that test to the new contract: `Starting` must be a refusal, while a positive
gate-release/emission case must use a matching accepted-session `Live` entry
and writer witness (or an equally direct source-local complement). The changed
test must carry `/// CONTRACT_SHAPE: bounded-change.`. It must not pass an
empty-weak fallback or retain the old Starting-emits assertion, because either
would contradict ADR-0100 D2 and mask the defect.

## E10 stop-oracle boundary and separate product decision

The ruling's caution is confirmed by current source. The operator-stop branch
in WorkloadLifecycle selects only rows whose observed state is `Running`
(`crates/overdrive-reconcilers/src/workload_lifecycle.rs:619–661`). A `Failed`
row is restartable under the separate restart predicate, but it is not a
candidate for that stop action (`:1468–1475`). The checked-in example helper
`wait_for_workload_stop` is Terminated-only (`examples/service-kind-vm-workloads/run-example.sh:216–227`).

Therefore the known sequence has two distinct meanings:

- a Running allocation reconciled for operator stop follows the existing
  StopAllocation/Terminated path (the HTTP 204 positive control);
- an already-authored startup-failed allocation, or a replacement that never
  reached Running, may remain Failed while VmReclamation disposes its
  resources without reauthoring the ending.

The native HTTP 302 trace proves the failed same-ID bind/cgroup-kill sequence;
it records stop intent, not a successful later Running replacement. The Sim
failure stops after one rejected replacement. Neither is evidence of a
separate eventual-restart liveness failure. No requirement to move disposal
before start, change the cgroup cleanup, change retry budget, delay startup
finalization, or rewrite Failed to Terminated is established.

ADR-0100 and its ruling are honest about this boundary: they do not promise
that the authorship fix makes all E10 cells pass, and they expressly preserve
the strict helper without changing it to `Terminated|Failed` (`ADR-0100:167–172`;
`vm-restart-ending-authorship-ruling.md:101–131`). This review does not
recommend a helper relaxation, removal of the assertion, or manufactured stop
transition.

The precise unresolved product decision is only this: if step `02-03` must
finish with the current Terminated-only cleanup helper green for startup-failed
cells, the user must separately choose whether the product contract should
change those Failed-instance stop semantics or whether the E10 scenario needs
a separately approved disposal-specific oracle. That decision is outside
ADR-0100 and no action is authorized here. Preserving the existing lifecycle
semantics is the current ruling recommendation.

## Architectural-bias and roadmap checks

| Check | Result and evidence |
| --- | --- |
| Technology preference / resume-driven complexity | **Pass.** No new technology or dependency is selected; standard private `Arc`/`Weak` identity uses the existing Rust/Tokio ownership model. |
| Context and alternatives | **Pass.** ADR-0100 explains the native/Sim defect, constraints, owner path, and rejects at least four broader or lifetime-changing alternatives (`ADR-0100:182–197`). |
| Quality attributes | **Pass.** Reliability is improved by rejecting duplicate authorship; the normal path adds only pointer identity under an existing lock; no I/O, deadline, persistence, security, or wire surface changes; testability and observability lanes are explicit. |
| External validity | **Pass.** Roadmap `02-03` invokes checked-in E09/E10/E13 examples through the built product and harness; ADR-0100 preserves those boundaries. |
| Acceptance coupling / implementation code | **Pass.** The roadmap criteria remain stakeholder-visible matrices; ADR-0100 D3 is an intentional private design contract, not an invented public acceptance surface or roadmap algorithm. |
| Unit/integration boundary | **Pass with bounded fallout noted above.** Private ownership complements stay source-local; the seeded invariant stays in-process; expectations remain black-box and must not import or run an Overdrive test binary. |
| Priority validation Q1 | **Yes.** The largest bottleneck for this amendment is the repeated seed-257205 duplicate-ending safety failure, corroborated by native syscall chronology. |
| Priority validation Q2 | **Adequate.** A Live-only check, strong writer, per-attempt/generation fencing, serialized publishers, stop-before-release, and pre-start reclamation are evaluated and rejected as incomplete or broader than the evidence. |
| Priority validation Q3 | **Correct.** The identity is attached at the narrow accepted-session boundary; no global persistence or lifecycle mechanism is introduced for a per-session ownership defect. |
| Priority validation Q4 | **Justified.** Two final-source one-fail/one-pass runs, the corrected reclamation control, direct native `EADDRINUSE`/cgroup/SIGKILL evidence, and the one-lock source path support the choice. |

## Findings and dispositions

| ID | Severity | Status | Disposition |
| --- | --- | --- | --- |
| None | — | — | No design finding. ADR-0100 is sufficiently precise and bounded for implementation. |

The existing direct `run_exit_watcher` test's Starting-based positive fixture
is recorded as required bounded test fallout, not an ADR defect: ADR-0100
already states Starting is an exact no-op and explicitly requires both
Starting refusal and matching-session success coverage. The E10 helper issue is
recorded as a separate product-contract decision, not a remediation finding;
no helper or lifecycle semantics change is authorized by this review.

## Verification record

| Check | Result |
| --- | --- |
| Mandatory project/rule and reviewer-skill reads | Complete. |
| Production owner reachability | Confirmed through `serve` → registered convergence → ServiceLifecycle finalization → WorkloadLifecycle same-ID restart → VmDriver failed-start cleanup → original watcher/observer. |
| Necessity evidence | Confirmed from the retained seed-257205 one-fail/one-pass runs and the preserved failing assertion (`e10-vm-early-exit-root-cause.md:237–257`). |
| Native host evidence | Inspected from the retained diagnosis only; no native execution during this review. |
| Reclamation compatibility | Confirmed from the corrected seed-257204 ruling and current lease/executor paths. |
| Weak lifetime/identity | Confirmed against the existing strong `LiveVm.beacon`, writer `Drop`, and `Weak::ptr_eq` contract. |
| Atomicity | Confirmed: read/predicate and `Live → EndingInFlight` mutation remain under the same existing `LiveMap` lock; Drop uses the same predicate. |
| Stop/E10 boundary | Confirmed from the Running-only stop branch and Terminated-only helper; no relaxation recommended. |
| Artifact scope | Only this native Markdown review artifact was added; no unrelated dirty work was changed. |

## Iteration 1 disposition and verdict

| Area | Disposition |
| --- | --- |
| Seeded defect and production reachability | Confirmed. |
| Exact public/persisted API boundary | Approved. |
| Weak session identity and pointer semantics | Approved. |
| Atomic claim and failed-claim Drop predicate | Approved. |
| Stop and VmReclamation compatibility | Approved. |
| Lifecycle Gate Ownership / boundary scenarios | Approved. |
| Necessity/fix evidence and test boundary | Approved, with the stated private-test fallout. |
| E10 strict helper and Failed-instance semantics | Preserved as a separate user/product decision; no ADR finding. |

**APPROVED.** ADR-0100 is a necessary, feasible, and minimal correction for
the proven stale VM exit-watcher authorship path. It reuses the accepted
session's existing identity, keeps both map mutations atomic, preserves stop
and reclamation ownership, and does not invent public or persistent surface.
This approval does not claim that implementation or the E09/E10/E13 evidence
gate is complete.
