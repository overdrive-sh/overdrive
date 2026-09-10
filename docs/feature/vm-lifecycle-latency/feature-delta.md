# VM lifecycle latency — #283 / shared convergence path #260

**DESIGN status: APPROVED; independent DESIGN review APPROVED; user ratification APPROVED, 2026-09-10.**
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

The cooperative Service profile is a checked-in native integration-test guest
fixture (DISTILL strategy amendment below): foreground TCP responder on 18081, one
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
| Product outcome | Reuse existing E09 v2 for concurrent operator deployment/stop, truthful public results and external resource observations. Internal stage timing and guest-group guarantees are native in-process tests. | E09 runner invokes its existing checked-in example; no cargo test/nextest/Rust test binary or overdrive crate import/link, no inline recreated specs/workloads. |
| Existing product regressions | Rerun E09 v2 with the history-based independent-submission correction below: one persistent control plane, 20 logical healthy/failure pairs, two ten-pair cohorts, concurrency ten, every ledger row, existing startup truth/readiness/routing and final cleanup predicates. Also retain relevant E06/E08/E10/E11 boundaries unchanged. | Native built-binary expectations through authorized harness. All trials/failed attempts retained; a partial ledger or timing-only success is insufficient. |

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

## Wave: DISTILL / [REF] Strategy and history amendment

**Authoring: complete for independent review; roadmap validation pending.**
The user's 2026-09-10 refinement prefers Rust/seeded Sim; reuse existing
expectations where appropriate and add one only for a distinct necessary
operator outcome. DEVOPS is explicitly skipped. Existing Lima/native-metal
verification environments are inherited; no deployment, rollback, readiness,
new production API, or lifecycle mechanism is introduced by this amendment.
Density resolver: `lean / ask-intelligent / explicit_override`.

E09 v2 already expresses the affected concurrent deployment/stop journey, so
its example and expectation are updated in place. No new expectation ID or
latency example is needed. Native integration tests own the three internal
stage distributions and guest supervision. This supersedes only the DESIGN
handoff's new-example requirement and its instruction to retain E09 v2's
workaround verbatim. ADR-0102/0103, targets, ownership and historical DESIGN
review remain unchanged.

History was traced with `git log`, `git show`, and `git blame` before scenario
authoring, starting from service-kind-vm-workloads and following its active
E09-v2 runner to `examples/service-kind-vm-workloads-v2/`:

| Commit | Actual change and present path | DISTILL disposition |
|---|---|---|
| `cbcf9a6d2` | Introduced persistent-control-plane v2 example. A worker advanced from its own healthy cleanup to failure deployment; siblings could still be stopping. | Restore this independence, retaining subsequent honest cleanup/error evidence. |
| `b653e1ad1758d11be33be457849b284f78141333` | Replaced the 100-pair sample and low CPU-derived concurrency with exactly 20 pairs/two ten-worker cohorts. Set 600s remote owner + 60s cleanup, added runtime cleanup checks and terminal-state handling. | Retain the explicit 20-pair/concurrency-ten contract and cleanup predicates. This was a bounded sample correction, not the serialization workaround. |
| `f8dad8bccc4f880ce526da64897af53a42c1fae4` | `run-example.sh` added every-worker `healthy-cleanup-complete` join and `failure-submit-release`; 60s stop/disposal observation became 180s, with a comment explaining ten queued ~12s stops. E09 runner grew 600→1200s. Scheduler tests asserted the global cleanup gate. | Remove the global gate; each real worker submits failure after its own cleanup. Restore 60s observation and 600s owner windows. Replace gate/cancellation tests with real shell worker+coordinator independence and retained full failure ledger. |
| `9e41ffdb` | Aligned documentation/comments with the new 1200s allowance; did not change convergence. | Active example/expectation/index now document restored 600s + 60s cleanup. Historical captures are not relabeled. |
| `373b335c9ab623938eaa8e07992327fcc0e52d45` | `e09_v2_failure_stream_overlap_spike.rs` overrides streaming cap to 120s to accommodate nine controlled 12s serial stops, instead of the default 90s. | Preserve this historical manual-drain diagnostic. Removing only its override would test its deliberately serial harness, not ADR-0102's real owner. Seed 283001 and its extended production-server invariants are the scheduling gate. |

The failure trajectory correction in `f8dad8bc` remains: a nonzero deployment
and failed startup probe can coexist with `Running`, positive restart count,
and the *same allocation's* prior Failed snapshot. Requiring the previous
special-case EADDRINUSE reason would discard valid public evidence. Append-only
command/stop diagnostics, failed attempts, full ledger, resource complements,
one-process identity and both active-cohort barriers remain. E09 v2 status is
reset to pending because its previous native audit exercised the global gate.

## Wave: DISTILL / [REF] Executable scenarios and evidence boundaries

The walking skeleton is S-VLL-01: an operator's independent workload reaches
Running while a different workload's real convergence effect is held. It
boots the production server in-process with real HTTPS, local intent storage
and the existing injected Driver/Clock ports. Native adapter coverage remains
an independent tier; Sim durations are never native latency measurements.

`LIVE RED` below means real assertions execute against current production and
fail on the named missing behavior, packaged as the repository-required
`#[should_panic(expected = "RED scaffold")]`. `SCAFFOLD` means a named explicit
pending panic: no behavior or coverage pass is claimed. GREEN must replace
these pending bodies and remove expected-panic attributes; a hook-compatible
pass alone never proves the feature. All source-local pure properties require
the exact `/// CONTRACT_SHAPE: pure-function.` line.

| ID / contract | Given → When → Then | Executable / evidence state |
|---|---|---|
| S-VLL-01, bounded-change | Given healthy/released controls and one held start or stop for A; when B is submitted through HTTPS; then B reaches Running before A is released, queries remain live, and cooperative shutdown joins. | `overdrive-sim/tests/vm_lifecycle_latency_283_spike.rs`: existing two `slow_*_does_not_block_independent_convergence` LIVE RED; `healthy_driver_control_progresses` real green. Seed 283001 retained. |
| S-VLL-02, bounded-change | Given seven distinct workload targets held in start or stop; when an independent target is submitted; then all seven are admitted and the independent workload progresses in the remaining slot. | Same file: `seven_held_{starts,stops}_leave_one_progress_slot`, LIVE RED. Shared fixture, same seed and production owner. |
| S-VLL-03, bounded-change | Given eight distinct held targets; when a ninth arrives and one effect completes; then ninth stays pending before completion and progresses after that one slot is freed, with the other seven still held. | Same file: `eight_held_{starts,stops}_bound_and_refill_admission`, LIVE RED. |
| S-VLL-04, bounded-change | Given an actual Service workload with held start; when accepted observations and public stop produce WorkloadLifecycle and ServiceLifecycle turns for that workload; then hydration-through-result-consumption never overlaps across those names, duplicate pending work coalesces, and the later turn sees latest stop intent. | Same file: `same_workload_reconcilers_share_the_complete_evaluation_lease`, SCAFFOLD. Full target Views/rows/driver-membership delta and independent-progress complement required. |
| S-VLL-05a/b, pure-function | Given generated submit/replace/drain/reap traces over workload/service/node targets and reconciler names; when bounded admission runs with zero/one/7/8/9 limits and none/some/all blocked targets; then returned order, residence durations, pending/cancelled/dispatched counters and reap results match an independent reference model; hot requeue cannot starve older eligible work. | `overdrive-core/src/eval_broker.rs::bounded_admission_contract`, two SCAFFOLD property specifications; migrate existing broker properties to ADR-0102's exact signatures in the same step. Include clock before/equal/after enqueue and saturating durations. |
| S-VLL-06a, bounded-change | Given eight active starts/stops and a pending ninth; when server shutdown closes admission; then shutdown waits for real effects/results, all active owners drain, and ninth never executes. | Sim `admission_close_drains_owned_{starts,stops}_without_admitting_ninth`: sequential permit release checks each entered Driver call's return and shim-authored Running/Terminated row, checks shutdown before each remaining hold, and rejects any ninth Driver entry/allocation row. LIVE RED remains the eight-way admission precondition. Private evaluation-result consumption and ninth evaluation non-admission remain acceptance-designer-owned construction; see Remaining S06 owner oracle. Exit-report fields belong to S06b. |
| S-VLL-06b, bounded-change | Given consumed active results and coalesced pending work; when the convergence owner snapshots broker state and exits; then exactly one report equals that locked snapshot, later submissions remain unexecuted outside its unchanged count, and consumed observer events finish existing retries before join. | Sim `convergence_exit_report_is_the_owner_snapshot_not_final_server_backlog`, SCAFFOLD; source-local owner access may locate the final assertions beside the existing private owner rather than introduce a public test seam. No claim to drain the unread observer queue. |
| S-VLL-07, bounded-change | Given accepted session and immediately terminating SimVmm; when stop completes its request; then stop can finish without advancing the two-second writer deadline, VMM is no longer live, run directory is gone, and EndingInFlight remains supervised. | `overdrive-worker/tests/acceptance/vm_driver_stop_totality.rs::completed_shutdown_write_has_no_two_second_floor`, LIVE RED. This is an adapter-ordering proof, not normal native guest exit. |
| S-VLL-08, bounded-change | Given writer success/error/EOF/absence/backpressure and early/deadline VMM completion; when stop runs; then writer and the single ten-second VMM grace overlap, writer is consumed at two seconds or earlier exit, cleanup calls finish, and forced/missing-cleanup results cannot pass a healthy measurement. | Same file: `writer_bound_overlaps_the_single_vmm_grace_and_every_writer_is_consumed`, SCAFFOLD; reuse existing real BeaconWriter and held-Vmm fixture. Existing backpressured EXEC-release, stop-totality and clone-index tests remain complements. |
| S-VLL-09, pure-function | Given successful pre-READY stages and a completed command; when the production guest lifecycle finishes; then the exact trace is root/modules/connect/network/READY/EXEC/operator/EXIT/poweroff with no post-EXIT SHUTDOWN read. | `overdrive-init/src/main.rs::tests::completed_command_powers_off_without_waiting_for_shutdown`, LIVE RED. Existing READY-failure and exit-status properties retained. |
| S-VLL-10a/b, bounded-change | Given production init with live child/group or incomplete/invalid control frames; when natural exit, SHUTDOWN, repeated SHUTDOWN, EOF, malformed or duplicate EXEC occurs; then group teardown/reaping is bounded by one five-second grace, direct status is retained, EXIT occurs at most once on success, and original typed errors survive failed streams. | Existing native module `overdrive-cli/tests/integration/vm_stop_restart_and_vmm_death.rs`: two `guest_*` SCAFFOLD matrices. Include child-before-descendant, TERM cooperation/ignore, split/coalesced frames, real signal/status, ESRCH/EINTR/ECHILD and deliberate group escape; distinguish pre-EXEC no-child errors from during-execution teardown. Private init tests may cover its exact approved File-based entrypoint; no new adapter/API. |
| S-VLL-11, bounded-change | Given the approved fresh-VM images/resources/warm input cache and named READY/finite-Job/cooperative-Service profiles; when each runs 200 sequential and 200 with ten operator workers through one persistent in-process server; then every scheduled trial remains in the ledger, all healthy trials have normal VMM exit and required cleanup, and nearest-rank stage distributions meet the existing targets. | Same native module: `native_lifecycle_profiles_meet_stage_targets_without_dropping_trials`, SCAFFOLD. This is the only 1200-trial owner. Native profile fixture and stage collector remain to be completed; no measured quantiles exist. |
| S-VLL-12, bounded-change | Given one default-feature built product and existing checked-in E09 v2 bundle; when two ten-pair cohorts independently advance from each worker's cleanup to failure submission; then all 20 truthful public healthy/failure/peer results and cleanup complements hold within restored windows, with one serve identity and no discarded pair. | Updated E09 v2 example/runner/contract; host-safe real shell worker+coordinator independence regression green. Native amended expectation pending. |
| S-VLL-13, preservation | Given fsync failure, unchanged View, driver error/re-enqueue, late/natural exit, same-ID replacement, Starting/EndingInFlight and reclamation; when the existing production boundaries run under the new owner; then no effect precedes durable View, old session cannot touch replacement, LWW and reclamation claims retain current results. | Composed View-failure oracle: `overdrive-sim/tests/vm_lifecycle_latency_283_spike.rs::view_fsync_failure_prevents_dispatch_and_recovers`. The existing `reconciler_runtime_view_store::{runtime_writes_through_before_in_memory_update,runtime_skips_write_through_when_next_view_equals_in_memory}` remain narrower persistence-helper/Eq-elision complements; neither dispatches an evaluation. Retain `runtime_convergence_loop::{stop_after_failed_alloc_drains_broker,view_below_ceiling_with_seen_at_re_enqueues}`, seed 257205 in `e10_vm_early_exit_spike.rs`, `vm_reclamation_claim_lifecycle`, stop-totality and clone-index suites. Run these as regressions; no new defect is asserted. |

## Wave: DISTILL / [REF] Coverage self-audit and handoff

Upstream inputs: DESIGN/ADRs supply state, error, concurrency and mode contracts;
research/RCA supply reproduced schedules; VM journey steps 2/3/4/6 and existing
US-SVM-1/K1 supply operator outcomes. There are no new CLI/config flags. The
user explicitly supplies the environment decision by inheriting verification
and skipping DEVOPS; missing DEVOPS documents are not a specification gap.

| Category | Population / review result |
|---|---|
| C1 equivalence/boundary | S01–03 and S05: 0/1/7/8/9, blocked/all-eligible, clock saturation. Live owner boundary RED; broker property bodies pending. |
| C2 state/transition | Pending→admitted→consumed, Live→EndingInFlight; before EXEC/running/group teardown/EXIT/poweroff. S04/06/08/10 include illegal/repeated events; native bodies pending. |
| C3 cardinality | Zero/one/many pending keys, targets, direct/adopted children; eight active plus ninth pending. S05/10 pending for complete generated domains. |
| C4 lifecycle/idempotency | Pending replacement, requeue/reap, repeated stop/SHUTDOWN, late exit, same-ID replacement; S04/05/08/10/13. Existing stop-without-live owner remains the inverse-without-prerequisite proof. |
| C5 decision table | Shared Exec/VM scheduling; host writer × termination; pre/post EXEC × frame × child/group state. No new mode flag, so flag orthogonality is inapplicable. |
| C6 negative/robustness | Full/blocked admission; View fsync failure; writer and signal/wait failures; malformed/EOF/duplicate frames; forced/missing cleanup cannot pass. S03–06/08/10–13. Closed error sets are ADR-0103's existing/private errors, not new public failures. |
| C7 environment/interruption | Seeded held production effects and cooperative shutdown; native Linux KVM/real child groups; inherited external-resource preconditions fail closed. No production-readiness or deployment scenario. |

Mechanical checklist (15 items): C1a/b specified (S05/S02–03); C2a/b specified
(state inventory above/S04/10); C3 specified (S05/10); C4a/b specified
(S05/08/10/existing double-stop); C5a specified (matrix above), C5b N/A (no new
flags); C6a/b/c specified (S08/10 and retained typed-error tests); C7a/b/c
specified (fsync failure/S06/eight-way owner). **Scenario specification is
present; executable completion remains partial:** eight named scaffolds still
need real bodies, and the ten LIVE RED tests still need GREEN transitions.
S06a additionally requires acceptance-designer construction of the complete
owner-result/non-admission oracle below. The composed S13 regression covers
View failure at dispatch; its old persistence helper is only a complement.
This is a DISTILL handoff, not approval to call those obligations satisfied.
The independent reviewer must judge scaffold adequacy and roadmap readiness.

Fixture reuse: the shared production-server capacity fixture serves six live
parameter cases, the previous seed fixture three, and E09's actual worker /
coordinator fixture all ten concurrent workers; >4× reuse is met where that
shape is useful. Generated state-machine/reference-model tests are reserved
for pure broker policy; fixed owner/native call sequences remain examples.
No cosmetic PBT wrapper or duplicated generic test-policy infrastructure was
added to inflate density. The two broker property specifications remain
explicit RED scaffolds until the exact approved signatures exist.

The delivery roadmap contains three dependent steps: full convergence owner
and timestamp fallout; responsive host/guest termination; native stage
measurement and amended E09 validation. Every step requires a fresh isolated
crafter and reviewer, on-disk Markdown review, original-step remediation until
APPROVED, and exact ADR API verification. Scaffold transition must retain these
scenario oracles; acceptance-designer assistance owns any additional test
construction, and no private testability need authorizes a public API. Mutation
is one final wave gate after all step reviews. Roadmap lists are guidance for
necessary compiler/test fallout only. No DES events, commit or push occurred
in DISTILL, and the pre-existing AGENTS.md edit is excluded from this work.

### Delivery execution gates

The scenario oracles above and the exact ADR signatures remain mandatory;
the concise roadmap criteria do not replace them. Its `scenario_locators`
mapping expands every abbreviated test name into an existing file and exact
function, or the existing shell command/runner. The primary `test_file` and
`scenario_name` identify each step's entry test only. **S-VLL-13 is a shared
regression gate for all three steps**, using the mapped existing functions and
the roadmap's control-plane, early-exit, stop-totality and clone-index suite
commands. Its early-exit seed remains 257205. Pending-body locators identify
executable scaffolds, not completed assertions or a passing guarantee.

For 01-01, timestamp/signature changes include every compiler-required broker,
router and enqueue caller; `InterestRouterBroker` lives in the listed
`overdrive-control-plane/src/lib.rs`. For 01-02, retain the existing native
exit-classification/equivalence fixtures and transition obsolete fixed 2+10s
and post-EXIT-read assertions explicitly. Test-private native fixtures and
nextest serialization configuration may change where the approved scenarios
require them. A pinned nix feature change in `Cargo.toml` is permitted only
as compiler-required fallout. Approved bounded VM/reaper stage events and
their measurement overhead remain required; no public test seam is implied.

Initialize execution with `des-init-log` and
`PYTHONPATH=/Users/marcus/.claude/lib/python` only after independent roadmap
approval. Follow AGENTS.md as the source of truth for agent selection, fresh
crafter/reviewer isolation, RED → GREEN → COMMIT, attribution and on-disk review
requirements. Remove a
RED expected-panic marker only when its complete scenario assertions exist.
The final mutation command retained in the roadmap runs once after all step
reviews are APPROVED and required native evidence exists. Its existing wrapper
must pass: at least 80% kill rate, baseline drift within the configured bound,
and investigated timeout/unviable outcomes; do not change exclusions. Retain
`target/xtask/mutants-summary.json`. DEVOPS remains skipped.

### Remaining S06 owner oracle

Acceptance-designer assistance owns this construction in step **01-01**, before
removing either S06a RED marker. Current sequential-release checks observe the
Driver-return boundary and subsequent action-shim row, not consumption of the
complete `run_convergence_tick` result by the convergence owner. An absent
Driver call or allocation row likewise does not prove that an evaluation was
never admitted/hydrated.

Using the existing seeded production-server composition and tracing boundary,
complete the oracle against ADR-0102's already approved
`convergence.evaluation.admitted`, `convergence.evaluation.completed` and
`convergence.drain.completed` events:

- Match all eight distinct target/tick admissions to the held Driver ledger
  before closing admission. Retain both start and stop schedules, seed 283001.
- Release seven effects one at a time. After each release, match exactly that
  target's real Driver result, shim-authored row and owner-consumed completion;
  the remaining held result(s) keep shutdown pending and no drain-completed
  event exists. Release the eighth last and require every matching completion
  before the sole drain event and cooperative owner join.
- Require `admitted_at_close = 8` and `completed_during_drain = 8`; the ninth
  target has no admission or completion event, Driver entry or allocation row.
  S06b separately supplies the exact locked `pending_at_exit` snapshot and
  later-submission exclusion oracle; retain its consumed-observer retry check.

These events and eight active evaluations are absent from current production.
`ServerHandle` does not expose its private convergence task/result collection.
The strengthened test can therefore exercise only the current one-held-Driver
control before its real concurrent-entry RED; it cannot yet execute this complete
owner oracle. Extend the tests against the approved event implementation during
01-01, without adding a public accessor, cancellation seam or new lifecycle
mechanism. If that existing tracing boundary cannot establish the stipulated
owner facts, surface the precise remaining gap before any API change. The
crafter may not treat port completion or an expected panic as a substitute.

## Wave: DISTILL / [REF] Author validation and limitations

The verified local evidence root is `.context/vm-lifecycle-latency-distill/`.
These author records retain their original exit codes and limitations.

- History-directed shell regression: current test with unchanged HEAD example
  timed out (exit 124) after five seconds at the global cleanup gate; the
  amended real `run_trial` + `run_cohort` completed with exit 0. The fixture
  explicitly holds worker 10's healthy stop until worker 1 begins failure
  deployment; it preserves all joins and cleanup records. Full scheduler suite
  passes. Records:
  `.context/vm-lifecycle-latency-distill/shell-eqtv8rcs/baseline.txt`,
  `.context/vm-lifecycle-latency-distill/shell-eqtv8rcs/scheduler.txt`, and
  `.context/vm-lifecycle-latency-distill/shell-eqtv8rcs/runner.txt`.
- Initial unwrapped seed regression run (`cargo xtask lima run -- cargo nextest
  run -p overdrive-sim --features integration-tests --test
  vm_lifecycle_latency_283_spike --no-fail-fast`) reproduced both existing
  failures: independent Running false while held, true after release; healthy
  control true; shutdown joined. New seven/eight-slot cases admitted exactly
  one target on current production, reproducing the same owner serialization.
- Final focused Rust command selected the Sim binary, source-local broker
  contracts, completed-command/stop regressions and writer-overlap contract
  across core/init/worker/Sim with `--features integration-tests
  --no-fail-fast --success-output immediate`. Result: 16 hook-compatible
  passes = ten live expected behavioral failures, five explicit pending-body
  panics, and one real healthy control. Full command/stdout are retained in
  `.context/vm-lifecycle-latency-distill/rust-r4vu5xqj/command.txt`,
  `.context/vm-lifecycle-latency-distill/rust-r4vu5xqj/output.txt`, and
  `.context/vm-lifecycle-latency-distill/rust-r4vu5xqj/exit.txt`. These are not sixteen
  completed acceptance guarantees. Three additional native scaffolds compile
  but were not executed as acceptance evidence.
- `cargo xtask lima run -- cargo check -p overdrive-cli --features
  integration-tests,kvm-tests --test integration`: passed. `cargo xtask lima
  run -- cargo clippy -p overdrive-core -p overdrive-init -p overdrive-worker
  -p overdrive-sim -p overdrive-cli --all-targets --features
  integration-tests,kvm-tests -- -D warnings`: passed after replacing one new
  test's single-pattern match with `if let`. No production branch changed.
- Shell syntax/ShellCheck and `cargo fmt --all --check` pass. The runner's
  transcript-cardinality/identity cases pass, and its complete host-safe suite
  passed on macOS; its timeout fixture is intermittent on macOS and fails in
  Lima with a surviving TERM-resistant fixture descendant. A bounded run of
  the **unchanged HEAD runner plus unchanged HEAD harness** reproduced the
  same Lima failure, exit 1; record
  `.context/vm-lifecycle-latency-distill/runner-baseline-jypkq8wq/0.txt`. No fix to
  that pre-existing fixture process-lifetime behavior is included, and no
  clean full Lima runner result is claimed.
- Feature-delta validator passes all 17 typed wave sections; roadmap validator
  reports valid one phase/three steps. Its status was pending at authoring;
  independent approval is recorded below. Before the DISTILL review artifacts
  were added, feature-layout validation checked this actual feature (not zero
  features) and reported only the mandatory historical `design/review.md` as
  legacy layout; that artifact remains unchanged.
  Whitespace checks pass. No mutation test was run in DISTILL.

One temporary timeout-diagnostic copy lacked executable permission. Its PATH
substitution fell through to real Cargo and unintentionally started E09 on the
native host at 16:27:39 UTC. The local diagnostic was interrupted; the native
example owner exited, but its serve process remained. The exact serve PID and
start ticks were matched to its recorded identity before stopping it; its
owned materialization was removed with the existing token-checking preparer.
The exact content-matched temporary remote diagnostic file was removed too.
There were no E09-named run/cgroup artifacts or VMMs at cleanup inspection.
Recovered measurements, all 20 partial/failed ledger rows, case diagnostics and
serve log are retained in
`.context/vm-lifecycle-latency-distill/native-diagnostic-20260910T162739.tar.gz`.
This accidental, incomplete attempt is **not** an expectation pass or a latency
measurement. Approved native profile distributions and a deliberate complete
amended E09 capture remain outstanding; no native success is claimed.


### Cross-wave remediation evidence

F-01 now has a composed preservation regression at
`crates/overdrive-sim/tests/vm_lifecycle_latency_283_spike.rs::view_fsync_failure_prevents_dispatch_and_recovers`.
It drives the unchanged `run_convergence_tick` through real hydration,
WorkloadLifecycle reconciliation, View persistence and awaited action-shim
Driver/observation publication. Valid desired-generation inputs enter through
the existing IntentStore; no next View or allocation consequence is seeded.
A healthy target first reaches Running. Injected SimViewStore fsync failure on
another target yields `ConvergenceError::ViewPersist`, unchanged complete hot
and stored View maps, unchanged allocation rows/Driver starts and no lifecycle
publication. Clearing the fault lets the same inputs place exactly once and
persist their generated View. No production/testability API was added. The
old persistence-helper test retains only its original narrower guarantee.

F-02's strengthened start and stop cases each observed **one Driver entry
before close, one sequential release stage, and eight Driver returns with
matching shim-authored rows by cooperative join**. The additional seven ran
from the current owner's pre-drained serial batch during cleanup. Those eight
serial returns are not eight concurrent active evaluations. Each case also
observed zero ninth-workload Driver entries and zero allocation rows. Only
then did its expected RED report the concurrent-entry precondition `1 != 8`.
The later eight-way entry-set assertion is not reached, and complete private
owner consumption/non-admission remains the explicit acceptance-construction
obligation above. No shutdown-loss or new production defect is claimed.

Focused command: `cargo xtask lima run -- cargo nextest run -p overdrive-sim
--features integration-tests --test vm_lifecycle_latency_283_spike
--no-fail-fast --success-output immediate`. Exit 0: 12 harness passes comprise
eight live behavioral REDs, two explicit pending bodies, and two genuine
non-panic passes (healthy control and the new composed S13 regression).
Records: `.context/vm-lifecycle-astra-remediation/focused-3-{command,output,exit}.txt`.
The earlier retained attempts record a missing test-local trait import and
fixture-oracle corrections; they are not additional production findings.
Scoped `cargo xtask lima run -- cargo clippy -p overdrive-sim --features
integration-tests --test vm_lifecycle_latency_283_spike -- -D warnings`,
formatting and whitespace checks pass. The feature-delta validator checks 18
sections; the roadmap validator accepts one phase/three steps, with 500 words
across JSON string values and status pending at authoring. Command records and preservation
checks are retained in `.context/vm-lifecycle-astra-remediation/`.
Across the feature, the ten behavioral REDs and eight pending bodies remain;
S06a has the additional pending owner oracle described above. Native profiles,
amended E09 and the excluded accidental archive retain their prior status.

## Wave: DISTILL / [REF] Independent review and handoff

DISTILL and the [three-step delivery roadmap](deliver/roadmap.json) are
**APPROVED for implementation handoff**, 2026-09-10.
[Cross-wave re-review, iteration 2](review-all-waves-astra.md#iteration-2--bounded-remediation-re-review)
closed F-01, F-02 and N-01 after the bounded corrections.
The following earlier independent approvals remain historical records:

- [Acceptance design](distill/review-acceptance.md): APPROVED, iteration 1.
- [Architecture alignment and roadmap](distill/review-design-alignment.md):
  APPROVED, iteration 2; R-01, R-02 and R-03 closed.
- [User scope and Git history](distill/review-scope-history.md): APPROVED,
  iteration 2; F-01 closed.

The roadmap records the completed cross-wave re-review. This approval does
not assert implementation GREEN, native latency distributions, or a completed
amended E09 capture. Ten behavioral RED tests and eight explicit pending-body
scaffolds remain the implementation starting point; a scaffold panic is not
proof of its required behavior. The remaining S06 owner-consumption and
non-admission assertions are acceptance-designer-owned prerequisites to
removing its RED marker. DELIVER has not started, and no DES phase events or
implementation commits were created in DISTILL.
