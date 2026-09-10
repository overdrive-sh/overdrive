# VM lifecycle latency — #283 / shared convergence path #260

**Status: APPROVED; independent DESIGN review APPROVED; user ratification APPROVED, 2026-09-10.**
[Review iteration 2](design/review.md#iteration-2--remediation-re-review) closed
F-01. The user subsequently approved the complete policy package on 2026-09-10.
Application/component DESIGN, propose mode, existing OOP paradigm.
This is the approved architecture contract; implementation and measured
performance remain unverified. Density: `lean`, `ask-intelligent` (installed resolver,
explicit global override). No roadmap is produced in DESIGN.

## Wave: DESIGN / [REF] Inputs and verified premise

Consultation: ✓ architecture brief, relevant ADRs 0013, 0035/0036, 0082/0083,
0084/0086, 0089/0099, 0100/0101 and VM journey; ✓ [research](../../research/orchestration/vm-lifecycle-latency-283-comprehensive-research.md),
✓ [RCA](../../analysis/root-cause-analysis-vm-lifecycle-latency-283.md),
✓ [final APPROVED RCA review](../../analysis/review-vm-lifecycle-latency-283.md),
✓ issues [283](https://github.com/overdrive-sh/overdrive/issues/283) and
[260](https://github.com/overdrive-sh/overdrive/issues/260), including comments
(both empty when fetched). The RCA's pending-review wording is stale.

⊘ `discuss/wave-decisions.md`, `discuss/user-stories.md`, `discuss/story-map.md`,
`discuss/outcome-kpis.md` and `spike/findings.md` under this feature: all absent.
The user authorized the issue, approved investigation and research as inputs;
these absences do not manufacture DISCUSS decisions or block this design.
The VM journey has historical unimplemented/cut statements; accepted amendments
and current source, rather than those historical sentences, ground the design.

Production files under `crates/*/src/*` have no changes between research baseline
`d880ce983276e7d4107450b264f6ceab64c70eff` and current
`44cfb4ee3cfdc66caecc820254c36ccbd7200a8f`. The owner paths below were read again.
No expensive probe was repeated against an unchanged premise.

| Proven boundary | Current owner path and evidence | Consequence |
|---|---|---|
| Shared serialization | HTTPS submit/stop → `handlers.rs:58–78` → broker → `lib.rs:3320–3393` → `run_convergence_tick` → sequential `action_shim/mod.rs:900–953` → Driver | The complete evaluation must be independently admitted; parallelizing an inner driver call alone cannot pass seed 283001. |
| Actual scheduling identity | Submit and accepted AllocStatus/ProbeResult routing produce `workload/<workload_id>`; runtime Views are per `(reconciler, TargetResource)`; WorkloadLifecycle hydrates the workload's allocation set (`workload_lifecycle.rs:446–479`) | Allocation IDs are not Evaluation keys. Preserve the workload aggregate while isolating independent workloads. |
| Healthy Service stop delay | `vm_driver.rs:1651–1757`; native 12,020.339 ms = request window 2,002.164 + VMM wait 10,017.545 ending SIGKILL + cleanup calls 0.630 | Remove the fixed wait and make the guest responsive. These individual observations are not quantiles. |
| Guest response gap | `overdrive-init/main.rs:153–205,1025–1080`: receive EXEC → synchronous child wait → read SHUTDOWN | PID 1 must read control while the command runs. Writer completion at 0.119 ms proves no guest receipt. |
| Existing result ownership | `reconciler_runtime.rs:1497–1608`; stop shim `mod.rs:2811–2963`; independent `worker/exit_observer.rs:84–102,185–225` | Retain View persistence/re-enqueue, cleanup and terminal write, session authorship and LWW arbitration. |

Seed `283001` in `crates/overdrive-sim/tests/vm_lifecycle_latency_283_spike.rs`
already fails both held-start and held-stop liveness through the real server,
HTTPS handlers, broker and convergence owner; healthy/released controls pass.
The existing injected Driver is sufficient. No new Sim seam is authorized.

Priorities, in order: independent progress; healthy stop responsiveness; honest
guest/host completion; stage-separated measurements. The two observed waits
account for approximately 99.99% of this Service stop; kernel tuning and generic
hydration/recovery work have no demonstrated necessity here.

## Wave: DESIGN / [REF] DDD list

Here DDD denotes numbered design decisions, not a new domain-modeling wave.

| ID | Verdict | Decision and reason |
|---|---|---|
| DDD-1 | APPROVED | Extend the existing broker and convergence owner with eight owned concurrent evaluations and one active lease per `TargetResource`; current Views require workload-wide serialization. |
| DDD-2 | APPROVED | Oldest eligible pending evaluation first; replacement at the same broker key retains age. Starts/stops share capacity. Completion releases a lease only after runtime result consumption. |
| DDD-3 | APPROVED | Keep `TickContext.deadline` advisory for pure reconciliation; existing driver deadlines own effects. Close admission and drain admitted evaluations at server shutdown. |
| DDD-4 | APPROVED | Start SHUTDOWN submission and the existing VMM-exit wait together; two seconds caps writer work, ten seconds caps VMM grace, neither is a minimum wait. |
| DDD-5 | APPROVED | Preserve the generic stop result and existing best-effort cleanup policy. A healthy-stop performance verdict separately requires observed normal VMM exit and verified driver-artifact absence. Do not silently strengthen `Ok(())`. |
| DDD-6 | APPROVED | One PID 1 supervisor; one child-led process group; SIGTERM, five-second grace, SIGKILL if needed, reaping, real direct-child exit code, then poweroff. Keep the existing wire vocabulary. |
| DDD-7 | APPROVED | Validate named-profile READY, finite-Job completion and cooperative-Service stop targets independently; health/Stable and cleanup remain distinct boundaries. |

Authoritative interface and ownership contracts are in accepted
[ADR-0102](../../product/architecture/adr-0102-bounded-convergence-evaluation-ownership.md)
and [ADR-0103](../../product/architecture/adr-0103-responsive-vm-stop-and-guest-supervision.md).

## Wave: DESIGN / [REF] Component decomposition

| Component / production path | Change | Responsibility |
|---|---|---|
| `overdrive-core/src/eval_broker.rs` | Extend existing pending policy and existing method signatures | Coalesce pending work, retain enqueue age, admit only eligible keys up to capacity. |
| `overdrive-control-plane/src/lib.rs` convergence owner | Extend | Own full evaluation futures, target leases, cadence, result consumption and drain. No independent lifecycle daemon. |
| `overdrive-control-plane/src/reconciler_runtime.rs` | Preserve lifecycle; timestamp submission fallout | Hydrate current state, persist View before actions, execute actions, re-enqueue unresolved work. |
| `handlers.rs`, interest-router constructors, `action_shim/enqueue_evaluation.rs`, reclamation fan-out | Bounded signature fallout | Supply the existing injected clock at submission; no new routing authority. |
| `overdrive-worker/src/vm_driver.rs` | Extend existing stop owner | Overlap bounded writer work and VMM termination wait; await driver cleanup calls before returning. |
| `overdrive-init/src/main.rs` | Replace blocking workload wait with one supervisor | Own control consumption, command group, signaling, reaping and poweroff. |
| `overdrive-host/src/vmm.rs`, SimVmm, exit observer, reclamation executors | Reuse behavior; bounded reaper event | Reaper, existing substrate contracts, independent LWW ending and atomic reclamation claims. |

## Wave: DESIGN / [REF] Driving ports

| Surface | Contract |
|---|---|
| Existing `overdrive deploy`, `overdrive job stop`, workload describe/watch and corresponding HTTPS handlers | Same public commands, routes, payloads and state vocabulary. Accepted intent is not effect completion. |
| Accepted observation events and existing resync schedules | Submit their existing targets into the same broker. No new event projection, reconnect, hydration or replay mechanism. |
| Existing `ServerHandle::shutdown(self, drain_deadline: Duration) -> Result<(), ServerShutdownError>` | Same signature; admission closes, admitted evaluations drain, then existing shutdown order continues. `drain_deadline` remains the HTTP budget. |
| Existing guest `EXEC` / `SHUTDOWN` frames | Same shared codec and transport; the sole PID 1 owner handles control during execution. No receipt-acknowledgement frame. |

## Wave: DESIGN / [REF] Driven ports and adapters

| Port | Reused adapters / guarantee and empirical evidence |
|---|---|
| `Clock` | SystemClock / SimClock; all scheduler timestamps use the injected monotonic clock. No time read in broker policy. |
| Reconciler hydration read ports, ViewStore, IntentStore, ObservationStore | Existing real/Sim adapters and accepted probe contracts; preserve fsync → hot View → dispatch order and LWW results. No widened read capability. |
| `Driver::{start,stop,status}` | VmDriver, ExecDriver, SimDriver. Existing typed results and Running/terminal authorities; see ADR-0103 for the explicitly limited stop postcondition. |
| `Vmm::{create,terminate,probe}` | CloudHypervisorVmm / SimVmm. Native reaper/process evidence and existing two-adapter equivalence tests. `Killed` also means already gone; it is not proof of an issued SIGKILL. |
| CgroupFs/CgroupManager and VmHostState | Existing probe-before-use composition and native cleanup observations; preserve existing start/reclamation contracts. New generic cleanup/recovery gates are out of scope. |
| Guest Linux control/process syscalls | Existing standalone init's File/std/nix boundary. Real guest tests must prove group signaling, reaping and poweroff; simulation cannot establish kernel effects. No new adapter trait or universal startup-probe framework. |

Retain existing compile-time probe methods, composition checks and substrate
fault tests for reused host adapters. Guest supervision is verified through the
real guest entry point and native process effects. Creating new production
probe interfaces just to satisfy a generic methodology checklist would expand
this bounded defect; none is proposed. External contract tests are required for
the existing Cloud Hypervisor/vsock/Linux boundary, using the pinned artifact
versions. Rust trait checks, crate-boundary/`dst-lint` gates and the existing
adapter-equivalence tests enforce the retained dependency direction.

## Wave: DESIGN / [REF] Technology choices

Keep Rust 1.95.0 (declared MSRV 1.88), Tokio 1.52.1, futures 0.3.32, tracing
0.1.44 and nix 0.30.1 from the current lockfile; no upgrades. Rust and futures:
MIT/Apache-2.0; Tokio/tracing: MIT; nix: MIT. The repository retains its
FSL-1.1-ALv2 license. Add only nix `poll`, `process`, `signal` features to
overdrive-init's existing dependency as needed by the pinned interface.
Keep the musl guest binary, current guest image and Cloud Hypervisor v53.0
benchmark baseline (Apache-2.0); no Tokio runtime in PID 1, actor framework,
new service, image factory, dependency or kernel tuning.

## Wave: DESIGN / [REF] Reuse Analysis

| Existing component | Overlap | Decision / justification | Contract Shape, bounded universe and assertion |
|---|---|---|---|
| EvaluationBroker | Coalescing, pending work, counters | EXTEND; avoid a second queue/registry | bounded-change: submitted/admitted keys, pending/cancelable records, ordering/age metadata and existing counters; whole broker delta against a value model. |
| Convergence owner + runtime | Dispatch, View ownership and re-enqueue | EXTEND owner, REUSE runtime; the proven obstruction is here | bounded-change: admitted target set, matching Views, emitted actions and outcomes, existing broker; seeded production-owner safety/liveness and complete affected-row/View complements. |
| Reconciler pure transitions | Workload and Service policy | REUSE unchanged; no allocation-key View repartition | pure-function: actions/next View only; existing properties and unaffected health/restart cases. |
| Action shim + exit observer | Cleanup and terminal publication | REUSE; independent authorities remain | bounded-change: current allocation, its declared cleanup resources and permitted rows/events; LWW winner, no late resurrection, full row/history/resource complements. |
| BeaconWriter + Vmm reaper | Request ownership and completion | EXTEND stop composition, REUSE ports | bounded-change: one accepted session and its writer/VMM/resources; writer is joined, reaper observed, native absence checks. |
| Init + shared beacon codec | Workload execution and command parsing | EXTEND existing owner; no reader thread or second codec | bounded-change: one guest control stream, direct child and its process group, adopted-child reaping, poweroff; real guest direct/descendant/escaped-group cases. |
| Existing native harness, examples, E09 v2 | Operator journey and real substrate | REUSE; a small later checked-in latency example supplies the cooperative profile | bounded-change: example-owned identities and artifacts; external black-box observation, independent of Rust integration tests. |

No new overlapping component is justified or created. No unbounded-preservation
operation is introduced. Existing broker and driver state is process-local;
intent/observations/Views retain their existing persistence owners.

## Wave: DESIGN / [REF] Lifecycle Gate Ownership

| Existing signal/state | Owner | Promise / permitted gate | Must not gate |
|---|---|---|---|
| Broker admission | Convergence owner | Capacity available and target has no active evaluation | Intent acceptance, independent exit observation |
| Persisted View | Reconciler runtime | Typed reconciliation memory fsynced before action dispatch | Claim that an emitted action succeeded |
| READY | Guest init | Existing bootstrap/network/control prerequisites completed | Service health/Stable; workload has not necessarily executed |
| Driver start / allocation Running | VmDriver + action shim | Driver start succeeds on READY; the shim commits Running, then installs interception and releases EXEC (ADRs 0089/0099) | EXEC receipt/execution, Service startup/readiness, cleanup |
| Service Stable / eligibility | ServiceLifecycle and established consumers | Existing startup verdict and separate readiness projection (ADR-0101) | Driver start or VMM exit timing |
| Driver stop `Ok` / `status == NotFound` | Driver | Existing relinquished active status; stop calls have completed | Normal guest exit or verified artifact absence |
| Terminal allocation row | Action shim / independent exit observer | Existing typed reason and LWW arbitration | Universal physical cleanup or Service identity deletion |
| Supervised allocation set | VmDriver | Starting, Live and EndingInFlight remain claimed | Raw process liveness; permission for a stale watcher |
| Healthy-stop benchmark pass | Native measurement owner | Normal VMM exit plus specified driver artifacts verified absent | Production terminal classification or generic health |

### G-1 — bounded, target-exclusive evaluation admission

- **Evidence/owner:** submit/stop handlers → broker → convergence owner → runtime
  → shim → driver, cited above; seed 283001. ADR-0035's View key is retained.
- **Promise/affected result:** at most eight admitted evaluation futures and
  one per target, from before hydration through consumed `ConvergenceError` or
  success after dispatch/re-enqueue. Only scheduling eligibility changes.
- **Failure projection:** full capacity/active target leaves work pending;
  duplicate pending key coalesces. Unknown registered-name/invalid hydration
  follows existing runtime results. No synthetic deadline error or cancellation
  of active effects. Admission-close leaves nonadmitted work pending until
  ordinary process teardown. `pending_at_exit` reports only the coalesced
  pending-entry count at the convergence owner's locked broker snapshot after
  all admitted results are consumed. Later producer submissions remain pending
  and unexecuted outside that count; there is no final server-backlog report.
  Neither population is reported as completed (ADR-0102).
- **Ordering/budget:** latest state hydrates only after lease admission; View
  fsync → hot View → awaited shim → existing re-enqueue → consumed result →
  release. Same injected cadence discovers new work; completion can refill
  capacity immediately. No extra driver deadline is created.
- **Unaffected/counterexample:** a held VM start for workload A cannot hold B's
  evaluation. WorkloadLifecycle and ServiceLifecycle for A share one target
  lease: separate reconciler-name locks would allow competing stop/restart
  chains and overlapping aggregate Views. Reclamation uses its existing atomic
  allocation claim, not a new node-wide exclusion over all workloads.
- **Disconnect/late success/disabled:** subscription recovery stays unchanged;
  late driver results complete their original lease and existing terminal
  arbitration, never a replacement lease. No switch; Exec gets the same bounded
  scheduling, VM-specific gates remain inactive without a VM driver.
- **Evidence lane:** seeded Sim through production serve plus in-process
  whole-View/row/completion complements; native concurrency demonstrates actual
  overlap rather than claiming Sim wall time as a latency measurement.

### G-2 — completion-driven host stop wait

- **Evidence/owner:** public stop intent → WorkloadLifecycle → stop shim →
  VmDriver → BeaconWriter/Vmm reaper → driver cleanup → shim cleanup/LWW write.
  The native held Service trace proves both waits and missing guest response.
- **Promise/affected result:** stop no longer waits for the two-second writer
  bound after request completion. It still awaits VMM termination and existing
  driver cleanup calls. Only stop's timing/order changes; the narrow generic
  `Driver::stop` postcondition and error policy are preserved, explicitly.
- **Failure projection:** writer error/EOF/no writer/timeout cannot mean receipt;
  continue the same VMM wait. Force at the single ten-second host grace if
  needed. Existing `NotFound`, best-effort termination/cleanup handling and
  shim typed errors remain as specified in ADR-0103. A failed cleanup observation
  fails the healthy benchmark, even if public stop or Driver returned success.
- **Ordering/budget:** Live → EndingInFlight before request; writer two-second
  cap overlaps VMM ten-second grace from stop entry; join writer; await existing
  cleanup calls, then shim cleanup/LWW. Never add 2 + 10 seconds or restart ten
  seconds after the writer. A forced termination is not a healthy latency pass.
- **Unaffected/counterexample:** host writer completion with a guest that never
  reads cannot authorize cleanup of a live VMM. READY, Running and Stable gain
  no receipt/cleanup dependency; stopped Service identity remains present.
- **Disconnect/late success/disabled:** no reconnect protocol; a closed accepted
  connection leaves VMM escalation available. Duplicate stop retains existing
  NotFound behavior. Natural-exit races use existing ADR-0100/LWW ownership;
  late completion cannot resurrect a terminal allocation. Exec stop is unchanged.
- **Evidence lane:** existing SimVmm/Driver contracts and seeded owner tests;
  native in-process reaper/writer/cleanup tests and built-binary public stop.

### G-3 — responsive guest workload termination

- **Evidence/owner:** VmDriver's accepted session sends EXEC after interception
  succeeds; failed post-start setup may instead send SHUTDOWN before EXEC.
  `overdrive-init::run` alone owns the guest lifecycle. Native Service evidence
  proves the synchronous wait prevents control consumption.
- **Promise/affected result:** after valid EXEC, control remains serviced while
  the workload lives; graceful termination targets the child-led process group;
  poweroff follows group completion/reaping. Exact order/errors are ADR-0103.
- **Failure projection:** pre-EXEC EOF/malformed/unexpected message retains
  `NoExecReceived`/`BeaconParse`/`UnexpectedBeaconMessage`, with no child start.
  Duplicate SHUTDOWN does not reset grace; malformed/EOF during execution
  triggers bounded group termination before the existing fatal poweroff path.
  Signal/wait errors use the pinned private InitError variants. Grace expiry
  SIGKILLs remaining group members; the host's ten-second fallback remains.
- **Ordering/budget:** establish the group before command execution; first
  shutdown receipt starts one five-second guest grace. Reap all available
  direct/adopted children, preserve direct-child status, emit EXIT at most once,
  power off without waiting for a post-EXIT SHUTDOWN. Guest/host clocks are not
  subtracted from each other.
- **Unaffected/counterexample:** a shell may exit while its child remains in
  the group; direct-child wait alone cannot pass the poweroff gate. A descendant
  deliberately leaving the group is outside graceful-group coverage and dies
  at guest poweroff. This is not a general process-tree supervision platform.
  READY is still before EXEC, and Service health remains workload-owned.
- **Late success/disabled:** a late SHUTDOWN after natural completion cannot
  change the recorded direct-child exit status. No subsequent EXEC is accepted.
  Existing non-VM execution is unaffected; a rootfs with the old init still
  needs the host escalation and cannot pass the cooperative healthy benchmark.
- **Evidence lane:** native in-process production VM composition with real
  guest process/group effects; pure exit mapping complements; black-box example
  checks only public result and actual owned-resource disappearance.

## Wave: DESIGN / [REF] Stage-separated latency contract

All numbers below are **approved validation targets awaiting measurement**,
never observed SLOs.
Normal boot uses fresh VMs/clones, no snapshot/restore. Benchmark artifacts are
pinned by binary/kernel/rootfs hash; record native host CPU/RAM/kernel, CH v53.0,
default-feature product, guest init hash, `cpu_milli=500` (one VCPU), 128 MiB,
rootfs exact measured size, and cache state. Use the existing E08 resources;
do not claim equivalence if image size changes.

| Boundary, host monotonic time unless noted | Approved target | Required separation |
|---|---|---|
| `Vmm::create` entry → accepted existing READY | P95 ≤2,000 ms; P99 ≤3,000 ms | Report Driver start preparation and admission queue separately; do not substitute VMM spawn or Service Stable. |
| Controlled finite Job's successful host EXEC-release event → normal reaper-observed VMM exit | P95 ≤500 ms; P99 ≤1,000 ms | Existing `vm.beacon.exec.released` boundary, after writer acknowledgement; not guest receipt. Workload immediately exits 0, no children or Service listener; report cleanup separately. |
| Cooperative Service's `VmDriver::stop` entry → observed normal VMM exit and all required driver artifacts absent | P95 ≤1,000 ms; P99 ≤1,500 ms | Queue/intent acceptance, shim terminal publication and shim-owned cleanup reported separately. No generic Service or uncooperative-workload SLO. |
| Guest SHUTDOWN receipt → group reaped; reaper exit → driver cleanup; stop-intent commit → admission/terminal row | Distributions required, no additional SLO specified | Guest durations use its own monotonic clock; no guest-ack protocol is implied. |

The cooperative Service profile is a checked-in guest program under a later
`examples/vm-lifecycle-latency/` bundle: foreground TCP responder on 18081, one
child in its process group, both explicitly handle SIGTERM and exit 0 within
100 ms; no daemonization, active requests, durable flush obligation or remote
dependency during stop. Establish existing guest TCP startup success and one
successful operator-visible request before stop. Include a separate ignored-
SIGTERM profile for grace/escalation correctness; never pool it into healthy
quantiles. This creates an empirical Service-stop contract that finite Jobs
cannot supply.

Required cleanup set: the allocation's VMM process, cgroup scope, VM run
directory/sockets, per-launch rootfs clone and clone-index link. Clone absence
must precede index removal. Also record existing shim-owned netns/interception
cleanup separately. Service/Job intent, identity and terminal history are
durable records and are not leaks. No logical deletion or dataplane teardown
redesign is authorized.

For each of the three profiles collect 200 scheduled trials sequentially and
200 at ten concurrent operator workers through one persistent serve process
(twenty cohorts of ten; scheduler capacity remains eight). Fresh allocation
identity per trial. Pre-read immutable inputs for the declared warm-host-cache
lane; VM boots remain cold. Record a separate cold-host-cache exploratory lane
only if cache conditions are controlled and recorded; the approved thresholds
apply to the declared warm-host-cache lane and do not claim cold disk latency.
Use nearest-rank empirical quantiles (`ceil(p*n)`, 1-based), report n, min,
median, P95/P99/max and full ledger. Timeout, forced kill, missing measurement
or incomplete cleanup is a failed trial, never dropped or retried into success;
any such trial fails the healthy-profile gate. A 200-trial P99 is a small
empirical sample, not a reliability claim.

Keep primary latency runs untraced except bounded existing/new stage events;
calibrate their overhead against an uninstrumented control. Instrumented
diagnostic runs explain stages but do not replace the primary distribution.
No histogram backend is added: the harness computes distributions from retained
events. ADR-0102 specifies queue/active/outcome/drain events; ADR-0103 specifies
VM stage events. Diagnostic failure evidence is retained unchanged.

## Wave: DESIGN / [REF] Executable obligations and handoff

| Boundary | Required obligation | Owner of evidence |
|---|---|---|
| Seeded Sim, real server owner | Keep seed 283001, held-start and held-stop schedules and healthy/released controls; B progresses before A is released. For ≤7 held distinct targets, an eligible healthy target eventually completes; at eight active operations the ninth stays pending. | DISTILL authors production-owner invariants using the existing composition seam. A legacy hand-driven serial Sim drain is not substitute evidence. |
| Seeded scheduling safety | Conflicting WorkloadLifecycle/ServiceLifecycle evaluations for one workload never overlap hydration→completion; duplicate pending work coalesces, latest intent wins, FIFO age survives replacement, freed capacity is usable and no target starves behind a repeatedly enqueued target. | Seeded schedules through real submit/stop/observation routes; no fabricated private allocation or terminal state. |
| Runtime/ending integration | View fsync failure emits no effect; driver error preserves established re-enqueue; late/natural exits retain LWW winner and ADR-0100 session claim; same-ID replacement is not touched by an old watcher. Existing reclamation claims prevent a node sweep from killing admitted supervised work. | In-process production crates; full bounded View/row/history/resource deltas, not a single happy-path count. Preserve seed 257205 and applicable existing gates. |
| Shutdown ownership | Cancel server admission with held active work: shutdown remains pending until its real effect/result completes. Assert `pending_at_exit` against coalesced pending entries at the convergence-owner exit snapshot, not the later server backlog. Submissions linearized after that snapshot stay unexecuted and are excluded from the unchanged report; consumed exit-observer events finish their existing retry loop before observer join. | Seeded production-server test with existing release controls; no test-only task abort promoted into a production defect or new reporting/barrier mechanism. |
| Real guest/control | SHUTDOWN while direct child lives; cooperative and ignored signals; child exits before descendant; natural exit concurrent with shutdown; duplicate/partial/coalesced frames; EOF; real direct-child signal/exit mapping; deliberate group escape characterized honestly. | Native in-process integration using production host/worker/init, real CH/guest fixtures; no spawning the built Overdrive binary, no expectation evidence. |
| Native stop composition | Early normal exit has no two-second floor; blocked writer bound overlaps ten-second grace; writer task consumed; forced case distinguished; exact driver cleanup complement verified. Preserve existing non-VM, boot failure, Starting/EndingInFlight/reclamation and Service startup/readiness behavior. | Native in-process integration, plus existing VMM equivalence suite. These are obligations, not claims that every failure is a newly reproduced defect. |
| Product outcome | New checked-in latency example drives built default-feature Overdrive via public CLI/HTTPS and checks public result, elapsed operator journey and external process/resource observations only. | Black-box expectation runner invokes the example; no cargo test/nextest/Rust test binary or overdrive crate import/link, no inline recreated specs/workloads. |
| Existing product regressions | Rerun E09 v2 exactly: one persistent control plane, 20 logical healthy/failure pairs, two ten-pair cohorts, concurrency ten, every ledger row, existing startup truth/readiness/routing and final cleanup predicates. Also retain relevant E06/E08/E10/E11 boundaries unchanged. | Native built-binary expectations through authorized harness. All trials/failed attempts retained; a partial ledger or timing-only success is insufficient. |

Every new/transitioned test needs its per-test Contract Shape declaration;
source-local pure Rust properties use exactly
`/// CONTRACT_SHAPE: pure-function.`. Test design belongs to DISTILL. Scope is
VM responsiveness and the shared convergence execution path; a new suspected
timing defect first requires a failing seeded production-owner invariant.
Missing testability cannot justify a new seam without user approval.

## Wave: DESIGN / [REF] Decisions table

Independent review and user ratification are approved. DDD-1 through DDD-7 are locked.

| ID | Status | Contract |
|---|---|---|
| DDD-1 | Approved | Eight evaluations; one active full chain per actual target |
| DDD-2 | Approved | FIFO among eligible broker keys; pending replacement retains age; shared capacity |
| DDD-3 | Approved | Advisory tick deadline; active effects drain; pending work remains unexecuted on close |
| DDD-4 | Approved | Concurrent request/VMM wait; 2 s writer cap within 10 s host grace |
| DDD-5 | Approved | Generic stop postcondition unchanged; healthy benchmark requires independent verified completion |
| DDD-6 | Approved | Child-led process group; SIGTERM; 5 s guest grace; SIGKILL/reap; no post-EXIT wait |
| DDD-7 | Approved | Named warm-host-cache profiles and approved 2/3 s READY, 0.5/1 s Job, 1/1.5 s Service targets |

## Wave: DESIGN / [REF] Open questions and approval boundary

The approved package above answers all seven research DESIGN questions.
On 2026-09-10 the user ratified **eight shared slots, nonadmitted shutdown
disposition, five-second guest grace/group boundary, retained narrow stop API,
and the benchmark targets**. Implementation and native validation remain
outstanding; approval does not establish measured performance.

A stronger `Driver::stop Ok` contract guaranteeing verified cleanup would need
a separately approved error/retry ownership amendment: today cleanup errors
are discarded and EndingInFlight retains no retry payload. It is not smuggled
into this latency fix. Guest receipt acknowledgements, allocation-key View
repartition, separate stop capacity, and an actor framework are rejected for
this bounded design; reopen only on evidence that the selected design cannot
satisfy the original defect. Exact workload image hashes/native host inventory
are validation run inputs; neither may be replaced with invented measurements.

Independent DESIGN review and user ratification are complete. The design is
ready for DISTILL; downstream acceptance design and implementation must use the
ADRs' exact interfaces. This commit records design approval only; production,
test and expectation implementation remain outstanding.

## Wave: DESIGN / [REF] Author validation

- Initial pre-review feature layout: installed validator's pure entry point
  checked this one feature and returned zero offenders. Its CLI treats immediate child
  directories as features, so pointing it directly at this feature initially
  checked zero; that empty check was not counted as validation.
- Current full-feature layout: the validator flags only the required separate
  independent-review artifact `design/review.md` as a legacy layout. That
  artifact and its full review history are preserved unchanged; the initial
  pre-review pass is not presented as a clean result for the current tree.
- Feature-delta heading validator: passed (all typed DESIGN sections).
- Outcome collision CLI `check-delta`: exit 0, **zero outcomes checked**, because
  it scanned registered OUT IDs while this design was pending and did not
  register new promises. Supplementary read-only `outcomes check` calls for bounded broker
  admission, VM-stop composition and guest supervision each returned
  `NO COLLISIONS`. The registry was not changed; the empty aggregate scan is
  not represented as a semantic collision proof.
- Local relative links in the feature delta/ADRs resolve; tracked diff and all
  new-file whitespace checks are clean. Source-baseline comparison found no
  production changes since the research commit's baseline.
- No executable tests, native benchmarks or Mermaid rendering were run in
  this documentation-only DESIGN. Independent review is APPROVED with F-01
  closed; user ratification was recorded on 2026-09-10. The approved validation
  distributions remain unmeasured. Installed density telemetry helper is absent;
  no synthetic telemetry was written.
