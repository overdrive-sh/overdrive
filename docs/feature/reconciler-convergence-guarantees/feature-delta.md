<!-- markdownlint-disable MD024 -->
# Feature Delta — `reconciler-convergence-guarantees`

Make reconciler convergence a runtime and test guarantee rather than a property each
author must get right. Source: GH #351 as corrected by the grounded verification at
`.context/convergence-verification.md` (HEAD `fb71b400f`); the verification supersedes
the issue wherever they differ. GH #344 (CALM/monotonicity checklist for LWW reads) stays
a separate issue and is cross-referenced, not absorbed.

## Wave: DISCUSS / [REF] Input checklist

- ✓ `.context/issue-351.md`, `.context/issue-344.md`
- ✓ `.context/convergence-verification.md` (supersedes the issue)
- ✓ `docs/product/jobs.yaml` (J-PLAT-004, J-OPS-003 read in full; both extended, none minted)
- ✓ `docs/product/personas/` (ana-platform-engineer, devon-platform-engineer; marcus persona read, not used)
- ⊘ `docs/product/vision.md`, `docs/product/architecture/brief.md`, ADR-0035/0036/0079/0084/0086/0087: not re-read in full; consulted through `.claude/rules/reconcilers.md`, `development.md` § "Reconciler I/O" and the verification doc, which cite them. DESIGN reads the ADRs.
- ✓ Precedent layout: `docs/feature/reconciler-framework-improvements/feature-delta.md`
- ✓ Live code spot-checked: `reconciler_runtime.rs:1585-1751` (hydrate `?` at 1589-1590 and view persist `?` at 1638-1642 both return before the requeue at 1739; dispatch errors do requeue), `service_map_hydrator.rs:456-488` (fingerprint stored on emit), `workload_lifecycle.rs` (`released_for_deletion` stamped at :247, gate :1531).
- ⊘ No journey artifacts (lightweight UX; no new operator surface). No DIVERGE artifacts: recorded as a risk in `wave-decisions` below.

## Wave: DISCUSS / [REF] Persona

- **Ana Moreno** (`ana-platform-engineer`) — operator who deploys Service workloads and trusts the platform to converge to what she declared. Beneficiary of the runtime and latch stories (US-01 to US-04).
- **Devon Reyes** (`devon-platform-engineer`) — authors reconcilers against the DST harness. Beneficiary of the property test and the rule (US-05, US-06).

No persona invented or modified.

## Wave: DISCUSS / [REF] JTBD

One-liner: **When a reconciler's action is dropped or fails, or its hydration or View write errors, I want the platform to converge again on its own from what is actually observed, so a deploy never stays wedged until I re-submit it, and so a reconciler author gets a mechanical check instead of a review hunch.**

- Operator stories trace to **J-OPS-003** (platform converges the declared intent; `workload describe` is honest).
- Author stories trace to **J-PLAT-004** (reconciler correctness mechanically checked before it is relied on).
- Both jobs are extended in `docs/product/jobs.yaml` (changelog entry only). No new job: the outcome statements already say "converges" and "mechanically checked".

## Wave: DISCUSS / [REF] Locked decisions

Only decisions grounded in the verification or ruled by the user in the dispatch.

| ID | Decision | Source |
|---|---|---|
| D1 | Scope is P1 (periodic re-evaluation), P2 (remove record-on-emit latches), P3 (shared multi-tick convergence property), and the `reconcilers.md` rule change. | User |
| D2 | `workload-lifecycle.observed_generation` stays in #325. Not in this feature. | User |
| D3 | Boot-only host-infra convergence (#197 veth, #198 cgroup, #199 XDP/cgroup-sock attach, #234 TPROXY routing) is out of scope. | User |
| D4 | P1 enumerates targets from intent AND observation, not `alloc_status` rows alone. | User |
| D5 | The service-lifecycle `stable_announced`/`terminal_announced` latch claim is refuted (`stable_announced` deleted; `terminal_announced` gates no action). It is not in scope as written. `startup_last_acted_run` and `liveness_last_acted_run` (act-once markers stamped on `StopAllocation` emit) are assessed, not assumed defective. | Verification finding 3 |
| D6 | `workflow-lifecycle` being un-woken is NOT a production defect: `serve` builds an empty `WorkflowRegistry` and nothing writes `workflows/` intent in production. Not scope. | Verification finding 5, `development.md` § "Ground the premise" |
| D7 | The hydrator's local-backend effect (`RegisterLocalBackend`/`DeregisterLocalBackend`) was unobservable. **Superseded 2026-11-15 (D-9): the local-backend path has no production producer and is deleted, latch included; nothing is made observable.** | Verification finding 1 (premise refuted by DESIGN); user ruling D-9 |
| D8 | `released_for_deletion` is replaced by a decision from allocator state, read through the existing `service_vip_view` read-port. **Moved to GH #211 (user ruling D-5, 2026-11-13); not delivered by this feature.** | User |
| D9 | The View may decide WHEN to act (backoff, budgets, rate limits). It never decides WHETHER the effect is in place; only `actual` decides that. | Issue rule text; user chose the rule change |
| D10 | Every new `actual` read introduced by P2 carries a DESIGN note naming the guard that makes it safe under LWW convergence (cross-reference #344). | User |
| D11 | Each slice is production-drivable through `overdrive serve` + `overdrive deploy` with a fault a real deploy can reach. A test that hand-installs what production does not is not acceptance. | CLAUDE.md § "Build vertical slices" |
| D13 | `ServiceLifecycleView::last_emitted_backend_fingerprint` is named as the single exception in the new View rule, citing ADR-0079 § D4/D9; ownership of the shared backend row is a separate decision outside this feature. | User, 2026-11-13 (closes O7) |
| D14 | Slice 03 carries one seeded reproduction of the possible VIP leak (same spec re-submitted, then deleted again), proving the leak and that removing the latch closes it. **Moved to GH #211 (user ruling D-5, 2026-11-13); not delivered by this feature.** | User, 2026-11-13 (closes O4) |
| D15 | The `development.md` "log-and-skip" text is corrected and `next_evaluation_at` is documented in the rules, in the same commit as slice 01's behaviour change. | User, 2026-11-13 (closes O8) |
| D12 | One feature with four slices. Periodic re-evaluation (slice 01) is not split into its own feature; the slices ship independently in order 01 to 04. | User, 2026-11-13 (closes O1) |

PROPOSED (not locked; need user or DESIGN ruling): see "Open decisions" O2, O3, O5, O6 (O1, O4, O7, O8 closed by D12 to D15).

## Wave: DISCUSS / [REF] Ground the premise (per production path)

| Defect | Reachable in a real deploy? | Production path |
|---|---|---|
| Hydrator local-backend latch | **No (DESIGN finding, 2026-11-15); deleted by D-9.** DISCUSS originally said Yes: | The hydrator emits `RegisterLocalBackend` only for a backend whose address equals `host_ipv4` (`service_map_hydrator.rs:356-370`). Since ADR-0276 every Service backend address is the Running allocation's `workload_addr`, taken from the guest network plan (`action_shim/mod.rs:1991`, `:2458`) and allocated from `GUEST_NODE_PREFIX` 100.95.0.0/16 (`guest_network.rs:561`, `guest_transport/mod.rs:594`), never `host_ipv4`. Non-VM provisioning was deleted (`fc3bc7d04`). The local set is always empty in a real deploy. |
| Hydrator un-woken after `Failed` result | Yes | `DataplaneUpdateService` `Err` writes a `Failed` row; next tick inside backoff yields `Noop` with no `next_evaluation_at`; a steady service gets no further backend-row write to wake it. |
| `released_for_deletion` latch | **Not reachable through production today (DESIGN finding); moved to GH #211.** DISCUSS originally said Yes: | Service deletion; `ReleaseServiceVip` executor returns `AllocatorRelease` on a redb `IntentStore::delete` error; View keeps the digest. |
| Hydrate error skips requeue | Yes, rare | `RunningAllocationWithoutWorkloadAddr` raised by `service-lifecycle` hydrate (data-dependent), or store read errors; early `?` at `rt:1589-1590`. |
| View write error skips dispatch and requeue | Yes, rare | `persist_view` redb write failure, early `?` at `rt:1638-1642`. |
| Fresh deploy with no `alloc_status` row never relisted | Yes, rare | First `workload-lifecycle` hydrate fails; `handlers.rs:521` submitted once; relist routes only `alloc_status` rows; no boot seeding of existing intents. |
| `workflow-lifecycle` un-woken | No | Empty registry in `serve`. |

## Wave: DISCUSS / [REF] Scope Assessment

Result: **OVERSIZED SIGNALS PRESENT; split considered and rejected by the user (D12). One feature, four independently shippable slices.**

- Stories: 6 (4 operator/author value stories, 1 assessment, 1 rule change). Under the 10-story signal.
- Bounded contexts touched: 4 (control-plane runtime, reconcilers crate, dataplane/allocator adapters, sim + core test surface). Over the 3-context signal. (signal 1)
- Independent outcomes that could ship separately: yes, P1 is independent of P2/P3. (signal 2)
- Effort: about 11 days across 4 slices, under the 2-week signal.
- Two signals hit. The feature stays as written until the user rules. Slice 01 is isolated so it can move out without rework.

## Wave: DISCUSS / [REF] Walking skeleton strategy

Brownfield. The skeleton is the existing `overdrive serve` + `overdrive deploy` convergence path; this feature adds no new end-to-end path. Each slice is a thin vertical cut through that path: a fault is injected at an existing driven port (store read, view write, dataplane insert, allocator release), production authors the resulting observation, and the oracle is observable output (`overdrive workload describe`, service map readback, allocator state). No separate walking skeleton is built.

## Wave: DISCUSS / [REF] Driving ports and pre-requisites

Driving ports: `overdrive serve` (`run_server` composition root, convergence loop); `overdrive deploy <SPEC>`; `overdrive workload describe`; `overdrive job stop`. Workload deletion (#211) is not a driving port of this feature: the VIP-release story moved to #211 (D-5). Fault injection goes through existing driven ports only (`IntentStore`, `ObservationStore`, ViewStore, `Dataplane`, VIP allocator), as `Sim*` adapters at Tier 1 and a named real-kernel stimulus at Tier 3 (DISTILL names it for slice 01).

Pre-requisites: none blocking. #325 (`observed_generation`) is adjacent and independent. #271 (migrate `EnqueueEvaluation` wakes to `interests()`) and #270 (warm reflector cache) overlap with P1 target enumeration and are cited, not consumed.

## Wave: DISCUSS / [REF] User stories

### US-01: A deploy recovers on its own after a transient hydrate or View-write failure

`job_id: J-OPS-003` · Persona: Ana · @driving_port: `overdrive serve` + `overdrive deploy`

#### Problem
Ana Moreno deploys `payments` (Service, VIP 10.96.0.14). The convergence tick for it hits a store read error. The runtime returns early and never requeues, the relist only routes targets that already have an `alloc_status` row, and nothing re-submits. `payments` stays Pending until she re-POSTs the spec.

#### Elevator Pitch
- Before: after a transient hydrate or View-write error, Ana runs `overdrive deploy payments.toml`, then `overdrive workload describe payments` shows no allocation or a stale one indefinitely, until she re-submits.
- After: run `overdrive serve` + `overdrive deploy payments.toml` with the same transient fault, then `overdrive workload describe payments` → sees `Running` within one re-evaluation period, with no second deploy.
- Decision enabled: Ana can trust a Pending workload to resolve by itself and stop re-submitting as a reflex.

#### Domain examples
1. Hydrate error: `payments`' first `workload-lifecycle` hydrate returns `ObservationRead`; the store recovers 5 s later; `payments` reaches Running with no second POST.
2. View-write error: `persist_view` fails for `workload-lifecycle/payments`; the next pass recomputes from fresh state and dispatches `StartAllocation`.
3. Un-woken hydrator: `DataplaneUpdateService` fails for `checkout` (VIP 10.96.0.21), the row is `Failed`, the next tick is inside backoff and emits `Noop`; after backoff expires the service map is programmed without any new backend-row write.

#### UAT scenarios
- S1 A workload whose first hydrate failed reaches Running after the store recovers.
- S2 A workload whose View write failed is dispatched on a later pass.
- S3 A service whose dataplane update failed is retried after its backoff without an unrelated wake.
- S4 A deployed workload with intent but no `alloc_status` row is still re-evaluated.
- S5 A converged target emits no non-`Noop` action on repeated passes (stability).
- S6 A target that has left both desired and actual is no longer re-evaluated (bounded cost).

#### Acceptance criteria
- [ ] AC1: Fault = hydrate error (`ObservationRead`, `IntentRead`, `RunningAllocationWithoutWorkloadAddr`); owner path = `run_server` convergence loop + `workload-lifecycle` and `service-lifecycle`; oracle = `workload describe` shows Running and the service map is programmed within one re-evaluation period after the fault clears, with no second `deploy`.
- [ ] AC2: Fault = `persist_view` error; oracle = same, and no action is dispatched on the failing tick (View is not claimed).
- [ ] AC3: Fault = `Failed` dataplane update result inside backoff; oracle = retried once backoff elapses with no backend-row write.
- [ ] AC4: Every registered reconciler is re-evaluated for every target in desired ∪ actual, enumerated from intent and observation. A target with intent and no `alloc_status` row is included.
- [ ] AC5: At a converged state, re-evaluation emits zero non-`Noop` actions, and evaluation cost per period is bounded by target count (guardrail: at most 3% duty cycle at 1000 targets, from the #266 estimate).
- [ ] AC6: Tier-1 seeded `overdrive-sim` invariant, seed printed on failure, covers the three fault stimuli and the recovery bound.

#### Technical notes
- DESIGN must reconcile with `reconcilers.md` § "The wakeup model" (a reconciler declares row-backed OR host-backed, "never both"; a redundant cadence is called double-triggering) and ADR-0084. A runtime-wide periodic pass changes that discipline: this needs an ADR-0084 amendment and user approval per `design.md` § "Surface material design decisions".
- The no-busy-loop rule holds: re-evaluation writes no observation row.
- Period value is PROPOSED (O5). Enumeration mechanism is DESIGN's; candidates (options only): per-reconciler scan of its own intent prefix plus observation row families; a runtime-owned union of the managed-target sets. Interacts with #270 and #271.

### US-02: A failed local-backend registration is retried until it is observably in place

**REMOVED BY THE DELETION (user ruling D-9, 2026-11-15; ADR-0283).** The story's premise is unreachable: no production path produces a local backend (a Service backend address is always the allocation's guest-prefix `workload_addr`, never `host_ipv4`; see "Ground the premise"). The path, and with it the story's latch, is deleted by slice 02; there is nothing to converge and no AC to meet. The text below is the original DISCUSS record, kept as history.

`job_id: J-OPS-003` · Persona: Ana · @driving_port: `overdrive serve` + `overdrive deploy`

#### Problem
Ana deploys `checkout` with a same-host backend. `RegisterLocalBackend` fails inside the dataplane. The hydrator stored `last_applied_local_fingerprint` in the same tick, the View was fsynced before dispatch, and the executor writes no row, so nothing ever says the insert failed. Same-host traffic to 10.96.0.21 hangs.

#### Elevator Pitch
- Before: after a failed local-backend insert, `overdrive workload describe checkout` reports healthy backends while same-host connects to the VIP time out, and nothing retries.
- After: run `overdrive serve` + `overdrive deploy checkout.toml` with the insert fault injected, then a same-host connect to the VIP succeeds once the fault clears, and `describe` shows the failed attempt.
- Decision enabled: Ana can believe a healthy `describe` means the local path is programmed, and diagnose a failed attempt from output rather than from a hung client.

#### Domain examples
1. Insert fails once (map full, then space freed): retried after backoff; connect to 10.96.0.21:8080 from the host succeeds.
2. Delete fails on service removal: `DeregisterLocalBackend` re-emitted until the entry is observably gone; no stale VIP entry remains.
3. View lost (`View::default()` after restart) while the map already has the entry: no duplicate effect, no new failure; stays converged.

#### UAT scenarios
- S1 A failed local-backend insert is retried after backoff and ends observably programmed.
- S2 A failed local-backend delete is retried until the entry is gone.
- S3 A View that says "applied" while the local entry is absent does not suppress the re-emit.
- S4 With the entry already present and a default View, nothing changes (idempotent).
- S5 A failure is visible to the operator (observable attempt state), not silent.

#### Acceptance criteria
- [ ] AC1: Fault = local-backend insert error at the dataplane port; owner = `service-map-hydrator` through the action shim; oracle = observable local-backend state equals desired within bounded backoff steps; same-host connect succeeds (Tier 3).
- [ ] AC2: Same for delete.
- [ ] AC3: The View never decides whether the entry is in place; View/actual skew in either direction converges (seeded Tier-1 with dropped actions).
- [ ] AC4: Applying the same register or deregister twice leaves the same observable state.
- [ ] AC5: The local-backend effect is observable to hydrate (D7). DESIGN names the read and its LWW guard (D10, #344).

#### Technical notes
- Candidate mechanisms (options, not decisions): a result row written by the executor, as the remote path already does; a readback of local-backend state through a new hydration read-port. A `Sim*` impl is required for either so DST can drive it.
- Possible pre-slice spike if readback is chosen: DESIGN classifies it contract-shaping or not (`design.md`).
- Remote path (`retries`) already compliant; unchanged.

### US-03: A failed VIP release is retried until the address is actually free

**MOVED TO GH #211 (user ruling D-5, 2026-11-13).** This story is no longer delivered by this feature and this feature does not drive workload deletion. Only AC4 (the executor doc fix) stays here, in slice 03. The text below is the original DISCUSS record, kept as the input #211 consumes.

`job_id: J-OPS-003` · Persona: Ana · @driving_port: `overdrive serve` + `overdrive deploy`, workload deletion (#211, owner of the story after the move)

#### Problem
Ana deletes `payments`. `ReleaseServiceVip` returns `AllocatorRelease` after a redb delete error. The View already holds the spec digest in `released_for_deletion`, so the release is never retried and 10.96.0.14 stays allocated. There is no orphan sweep. If `payments` is re-submitted with the same spec and deleted again, the digest gate may suppress the second release (unverified end to end; O4).

#### Elevator Pitch
- Before: after a failed release, Ana deletes `payments`, and the VIP remains allocated; a later deploy of a new service can exhaust or skip addresses.
- After: run `overdrive serve`, deploy then delete `payments.toml` with the release fault injected, then 10.96.0.14 is reusable by the next deploy within bounded retries.
- Decision enabled: Ana can delete and redeploy services without watching the VIP pool.

#### Domain examples
1. Release fails once, succeeds on retry: VIP free; next deploy `reports` is assigned 10.96.0.14.
2. Delete → redeploy same spec → delete: the second delete releases the VIP (covers the re-submit shape; see O4).
3. Control-plane restart between the failed release and the retry: View lost; decision still from allocator state; VIP released.

#### UAT scenarios
- S1 A failed VIP release is retried and ends with the address free.
- S2 A VIP the allocator still holds for a deleted workload is released regardless of the View.
- S3 A VIP already free is not released again (stability).
- S4 A restart with a default View still releases an orphaned VIP.

#### Acceptance criteria
- [ ] AC1: Fault = `IntentStore::delete` error in the allocator release; owner = `workload-lifecycle` + `release_service_vip` executor; oracle = allocator reports the VIP free within bounded retries.
- [ ] AC2: The decision uses allocator state read through `service_vip_view` (D8); `released_for_deletion` is no longer consulted for whether the effect is in place.
- [ ] AC3: Forgotten View and dropped release both converge.
- [ ] AC4: The executor's doc comment states what it actually does (it propagates `AllocatorRelease`); the verification found it claims the opposite.

#### Technical notes
- New `actual` read: allocator state via `service_vip_view`. DESIGN names its guard (allocator state is not LWW observation, so state why; D10, #344).
- Retry backoff, if any, stays in the View as inputs (`attempts`, last emit), per `development.md` § "Persist inputs, not derived state".

### US-04: Stop actions for failed startup and liveness probes are not lost

`job_id: J-OPS-003` · Persona: Ana · @driving_port: `overdrive serve` + `overdrive deploy`

#### Problem
`service-lifecycle` stamps `startup_last_acted_run` / `liveness_last_acted_run` when it emits `StopAllocation`. A dropped stop is re-emitted only when a newer probe row arrives. The verification judged this weakly self-healing while the allocation stays Running and probes keep running; it is not shown to be strictly safe.

#### Elevator Pitch
- Before: a wedged Service whose `StopAllocation` was dropped may sit unrestarted if no newer probe row arrives.
- After: run `overdrive serve` + `overdrive deploy wedged.toml`, drop the stop action, then `overdrive workload describe` shows the allocation stopped and restarted without waiting on a new probe result.
- Decision enabled: Ana can rely on liveness policy to act, or is told by the assessment that it already does.

#### Domain examples
1. Liveness fails for `api` alloc a1b2; `StopAllocation` dropped; probes keep running: stop re-emitted on the next probe row (current behaviour, expected to hold).
2. Liveness fails; stop dropped; probe runner no longer produces rows for that alloc: stop must still be re-emitted (the unverified case).
3. Startup deadline exceeded; stop dropped; View default after restart: stop still emitted.

#### UAT scenarios
- S1 Dropped stop is re-emitted while probes continue.
- S2 Dropped stop is re-emitted when no newer probe row will arrive (assessment: reproduce or refute).
- S3 With a default View and a failed probe, the stop is emitted.

#### Acceptance criteria
- [ ] AC1: A recorded assessment (reproduction or refutation, seeded Tier 1 plus the production path) states whether S2 is reachable on `serve` + `deploy`.
- [ ] AC2: If reachable, the stop decision comes from `actual` (the allocation's committed state), with the marker deciding only when to retry. If not reachable, the markers are documented as compliant retry pacing and no code changes (the assessment is the deliverable).
- [ ] AC3: Property test (US-05) passes for `service-lifecycle`.

#### Technical notes
- Conditional story; size is 1 to 2 days depending on the assessment. Stale `stable_announced` references in `overdrive-reconciler/tests/acceptance/startup_last_acted_run_evolution.rs` and `probe_run_count.rs` are touched only if the assessment edits those tests (O6).

### US-05: Reconciler authors get a shared multi-tick convergence check

`job_id: J-PLAT-004` · Persona: Devon · Entry point: the repository's default-lane reconciler test run (developer-tool story; the entry point is the test lane, not an operator command).

#### Problem
Devon adds a reconciler with a "done" flag in its View. Every existing check is a scripted scenario or a single-tick pure property, and none generates an arbitrary starting `(actual, view)`, runs multiple ticks, or drops actions. Both latches in this feature passed every test.

#### Elevator Pitch
- Before: a View field that overrules `actual` passes all tests and ships.
- After: run the default-lane test suite (`cargo xtask metal run -- cargo nextest run -p <crate>`), then a reconciler with such a field fails with the reconciler name, the seed, and the tick where it went idle while `actual != desired`.
- Decision enabled: Devon decides to change the View field before review, not after an incident.

#### Domain examples
1. A View field that overrules `actual` (the shape of the deleted `last_applied_local_fingerprint`) fails with "stalled at tick 3, actual != desired".
2. A pure backoff field (`svid-lifecycle` retry memory) passes.
3. `View::default()` start for `workload-lifecycle` converges.

#### UAT scenarios
- S1 Arbitrary independent `(desired, actual, view)` converges within N ticks for every registered reconciler (progress).
- S2 Once converged, no further non-`Noop` actions (stability).
- S3 With random dropped actions the reconciler still converges.
- S4 With a forgotten View (`View::default()`) it still converges.
- S5 Applying any emitted action twice through the sim executor leaves the same observable state.
- S6 A View field that overrules `actual` makes the property fail.

#### Acceptance criteria
- [ ] AC1: All 8 reconcilers registered in `run_server` (`noop-heartbeat`, `workload-lifecycle`, `workflow-lifecycle`, `svid-lifecycle`, `vm-reclamation`, `service-map-hydrator`, `service-lifecycle`, `gateway-svid-lifecycle`) are covered, or a reasoned no-obligation ruling is recorded per exempt one.
- [ ] AC2: Runs in the default lane (no real I/O); seed printed on failure; failing run reproducible from the seed.
- [ ] AC3: Fault stimulus = dropped actions and View/actual skew in both directions; oracle = `actual == desired` within N ticks, then zero actions.
- [ ] AC4: Mutation check on the property: DISTILL names the mutants from the live reconcilers (for example removing the resync pass, or a View field that overrules `actual`); each turns the property red. The local-backend latch is not a mutant (the path is deleted, D-9), and the VIP-release latch moved to #211.

#### Technical notes
- The sim executor must be generic over action kinds. Existing `vm_reclamation` DST has a real sim executor (`apply_actions`); the hydrator DST fabricates a `Completed` row and its "any combination of BPF state" claim is false. Reuse, do not duplicate.
- DISTILL authors the full bodies; scenarios for later slices stay as reasoned `#[ignore = "pending DELIVER ..."]`.
- Boundary with #344: this property does not decide read monotonicity.

### US-06: The rule says what a View may and may not decide @infrastructure

`job_id: J-PLAT-004` · Persona: Devon · No elevator pitch (documentation of a rule; merged into the slice of US-05 so the slice has a user-visible story).

#### Acceptance criteria
- [ ] AC1: `.claude/rules/reconcilers.md` replaces the fingerprint-shaped symptom with the general prohibition in D9, including the test "if the View says done and `actual` says not done, `actual` wins" and "if the effect cannot be observed, make it observable first".
- [ ] AC2: The rule is timeless discipline (principle plus symptoms); no ADR numbers as status, no DELIVER-phase notes.
- [ ] AC3: The `ServiceLifecycleView::last_emitted_backend_fingerprint` carve-out (ADR-0079 § D4/D9, shared-row ownership) is stated as the one known exception it remains today, or the rule says so explicitly.

Note on AC3: that field is a live record-on-emit marker the verification did not list. The user's scope does not remove it. It is O7.

## Wave: DISCUSS / [REF] Story map and prioritization

Backbone (operator path): Deploy → Reconcile tick → Dispatch effect → Observe result → Converged. Author path: Write reconciler → Check convergence.

| Slice | Stories | Walks | Value | Urgency | Effort | Order |
|---|---|---|---|---|---|---|
| 01 periodic-reevaluation | US-01 | Reconcile tick → Converged | 5 | 5 | 3 | 1 |
| 02 delete-same-host-local-backend-path | none (US-02 removed by the deletion, D-9) | Dispatch (removes a dead path) | 3 | 4 | 1 | scheduled first in DELIVER (depends on nothing) |
| 03 act-once-markers | US-04 assessment + US-03 AC4 doc fix (rest of US-03 moved to #211) | Dispatch → Observe | 3 | 3 | 1 | 3 |
| 04 convergence-property-and-view-rule | US-05, US-06 | Write reconciler → Check | 4 | 3 | 3 | 4 |

Priority rationale: slice 01 first because it fixes the broadest class (any hydrate or View-write error wedges a deploy) with the least dependency, and it is the riskiest assumption (ADR-0084 tension). Slice 02 is the deletion of the unreachable local-backend path (its latch was the one DISCUSS thought was production-reachable); it depends on nothing and runs first in DELIVER so later slices do not touch the code. Slice 03 after: smaller. Slice 04 last so the property is green on the fixed reconcilers and red on any reintroduced latch; DISTILL still authors it early so slices 02 and 03 can turn it green incrementally.

## Wave: DISCUSS / [REF] Outcome KPIs

| # | Who | Does what | By how much | Baseline | Measured by |
|---|---|---|---|---|---|
| K1 | Ana | A workload hit by a transient hydrate or View-write error reaches Running without re-submit | 100% of seeded runs within one re-evaluation period of the fault clearing | never (wedged until re-POST) | Tier-1 seeded sim invariant, plus `serve` + `deploy` run with fault injected |
| K2 | Ana | **Withdrawn (D-9):** the local-backend path is deleted; no production path reaches it | n/a | n/a | n/a |
| K3 | Ana | A failed VIP release ends with the address free | 100% within 5 retries | never retried | seeded sim; allocator state readback. **Moved to GH #211 (D-5); not measured by this feature.** |
| K4 | Devon | View fields that decide effect presence in registered reconcilers | 0 | 2 confirmed (hydrator local fingerprint, `released_for_deletion`) + 2 under assessment. The hydrator fingerprint is removed by deleting the unreachable local path (D-9), not by convergence; `released_for_deletion` is #211's. | latch audit at feature close |
| K5 | Devon | Registered reconcilers covered by the shared property | 8 of 8 or ruled exempt | 0 of 8 | test inventory |
| K6 | Devon | Injected latch mutants caught by the property | all mutants DISTILL names (see US-05 AC4) | 0 | mutation check on the property |
| G1 (guardrail) | Platform | Converged-state evaluation cost | at most 3% duty cycle at 1000 targets; zero non-`Noop` actions at fixed point | n/a | sim cost counter; seeded sim |

## Wave: DISCUSS / [REF] Definition of Done

- All slice ACs green; each oracle observed through the production composition (`serve` + `deploy`), not hand-installed wiring.
- Tier-1 seeded invariants print the seed; Tier-3 stimulus recorded for slice 01.
- Mutation gate ≥80% on changed reconciler and runtime code (per-feature gate).
- `reconcilers.md` rule merged; stale executor doc comment corrected.
- No new deferral language without an issue number; limitations introduced (e.g. the chosen period) cite an issue or are dropped.
- Docs and blog gate per `.claude/rules/finalize.md` at FINALIZE.

## Wave: DISCUSS / [REF] Out of scope

| Item | Where tracked |
|---|---|
| `observed_generation` latch | #325 |
| Boot-only veth / cgroup / XDP attach / TPROXY routing convergence | #197, #198, #199, #234 |
| CALM/monotonicity checklist for LWW reads | #344 (separate; cross-referenced) |
| Migrating `EnqueueEvaluation` wakes to `interests()` | #271 |
| Warm reflector cache for hydration | #270 |
| `workflow-lifecycle` wake | not a production defect (D6); see O3 |
| `service-lifecycle` terminal-announcement latch | refuted (D5) |
| VIP release retried from allocator state; `released_for_deletion` replaced; D14 reproduction; same retry rule for dial-by-name frontend `F` | #211 (user ruling D-5, 2026-11-13); this feature does not wait on it |

## Wave: DISCUSS / [REF] DoR validation (9 items)

US-02 column below is historical: the story is removed by D-9.

| # | Item | US-01 | US-02 | US-03 | US-04 | US-05 | US-06 |
|---|---|---|---|---|---|---|---|
| 1 | Problem in domain language | PASS Ana, `payments` wedged | PASS Ana, `checkout` hangs | PASS Ana, VIP leak | PASS (conditional) | PASS Devon | PASS |
| 2 | Persona specific | PASS | PASS | PASS | PASS | PASS | PASS |
| 3 | 3+ domain examples, real data | PASS | PASS | PASS | PASS | PASS | n/a doc |
| 4 | UAT 3 to 7 | PASS 6 | PASS 5 | PASS 4 | PASS 3 | PASS 6 | n/a |
| 5 | AC from UAT | PASS | PASS | PASS | PASS | PASS | PASS |
| 6 | Right-sized | PASS ~3d | PASS ~3d | PASS ~1d | PASS 1 to 2d | PASS ~3d | PASS <0.5d |
| 7 | Technical notes | PASS ADR-0084 tension | PASS candidates only | PASS LWW note | PASS | PASS | PASS |
| 8 | Dependencies tracked | PASS none | PASS none | PASS none | PASS none | PASS on 02/03 for green | PASS |
| 9 | Outcome KPIs numeric | PASS K1, G1 | PASS K2 | PASS K3 | PASS K4 | PASS K5, K6 | PASS K4 |

Plus: `job_id` present on all stories (J-OPS-003 or J-PLAT-004, both existing). Elevator Pitch present on US-01 to US-05; US-06 is `@infrastructure` and shares a slice with US-05.

Honest gaps: US-05's entry point is a test run, which the Elevator Pitch rule disfavours; accepted because J-PLAT-004's persona works in the test lane. US-04 is conditional on an assessment. DoR is passed for DESIGN handoff, subject to O2 and O3 being ruled.

## Wave: DISCUSS / [REF] Open decisions and proposals for the user

Each lists (a) reachable production failure, (b) cost, (c) cheaper alternatives, and my recommendation. No GitHub issue is created.

- **O1 Split slice 01 (P1) into its own feature?** CLOSED: the user chose one feature with four slices (D12).
- **O2 Fix the stale `build_cadence_table` comment (`lib.rs:5020-5025`)?** (a) None; misleading comment only. (b) minutes. (c) leave. Recommendation: fix inside slice 01 only if that function is edited anyway.
- **O3 Wake `workflow-lifecycle`?** (a) Not reachable in production today (D6). (b) small. (c) revisit when the first production workflow lands. Recommendation: no change.
- **O4 Reproduce the possible VIP leak on re-submit (`released_for_deletion` never pruned, same digest)?** CLOSED by D14; D14 itself moved to GH #211 (D-5).
- **O5 Re-evaluation period value.** (a) Governs recovery bound K1. (b) cost scales with targets. (c) reuse the existing 30 s relist and resync period. Recommendation: 30 s, decided in DESIGN. RULED 2026-11-13 (D-2): 30 s compile-time constant; the lack of an operator setting is a scope limitation tracked by #356.
- **O6 Stale `stable_announced` references in two acceptance test fixtures.** (a) None. (b) minutes. (c) leave. Recommendation: remove only if slice 03 touches those files.
- **O7 `ServiceLifecycleView::last_emitted_backend_fingerprint` is a live record-on-emit marker** CLOSED by D13.
- **O8 Update `development.md` text ("an unreadable observation is the runtime's to log-and-skip") and document `next_evaluation_at` in the rules.** CLOSED by D15.

DESIGN-owned (not for the user now): the target enumeration mechanism and the LWW guards named in D10. (The local-backend observability mechanism is gone with the path, D-9.)

## Wave: DISCUSS / [REF] Wave decisions summary

- Risk: no DIVERGE artifacts; job grounding taken from existing jobs.
- Slices numbered 01 to 04; each independently mergeable and production-drivable. Slice 02 (deletion) is scheduled first in DELIVER.
- Walking skeleton: none new (brownfield).
- Back-propagation: `docs/product/jobs.yaml` changelog entry for J-PLAT-004 and J-OPS-003; no new job, no persona change.
- Handoff blockers: the ADR-0084 amendment that DESIGN must put to the user for slice 01.

## Wave: DESIGN / [REF] Input checklist

- ✓ DISCUSS sections above (D1 to D15 locked; O2, O3, O5, O6 open) and `slices/slice-0{1..4}-*.md` (scope as summarised in the story map)
- ✓ `.context/convergence-verification.md`; `.context/issue-351.md`, `.context/issue-344.md` (background only)
- ✓ `docs/product/architecture/brief.md` (structure and Application Architecture section located; appended an extension section per precedent), `c4-diagrams.md`
- ✓ ADR-0084 (read in full through the 2026-08-23 amendments), ADR-0086, ADR-0087 (via `reconcilers.md`), ADR-0079 (via `reconcilers.md`)
- ⊘ ADR-0021, 0023, 0035, 0036, 0048, 0061, 0064: not re-read in full; consulted through ADR-0084's text, `development.md` § "Reconciler I/O" and the code that implements them.
- ✓ Live code: `reconciler_runtime.rs` (tick body, `persist_view` Eq-skip), `lib.rs` (`build_cadence_table`, `spawn_convergence_loop`, router, `list_and_route`, routes), `hydration.rs`, `reconcilers/mod.rs` (trait), `eval_broker.rs`, `service_map_hydrator.rs`, `workload_lifecycle.rs` (release gate, `read_job`), `service_lifecycle.rs` (stop markers), `service_vip_view.rs`, `dataplane.rs` trait, `register_local_backend.rs`, `dataplane_update_service.rs`, `release_service_vip.rs`, `local_backend_map_handle.rs`, `handlers.rs`
- ⊘ Not read: `svid_lifecycle.rs`, `vm_reclamation.rs`, `gateway-svid-lifecycle` bodies; `overdrive-sim` invariants; `persistent_service_vip_allocator.rs`. Their managed-target sets below are stated from the verification doc and the DISCUSS notes and must be confirmed by the crafter against each `hydrate_*`.
- ⊘ Precedent feature-deltas (`reconciler-framework-improvements`, `reconcilers-own-hydration`): layout followed from this file; not re-read.
- ⊘ ADR-0102, ADR-0273, ADR-0260, ADR-0191: cited from their titles and from comments in code; not read.
- ✓ #211 is verified open by the user's ruling of 2026-11-13 (`gh issue view 211 --comments`, including the extension comment https://github.com/overdrive-sh/overdrive/issues/211#issuecomment-6866541631). ⊘ #344 is cited as the repository text cites it; not re-verified in this dispatch. #275 and #356 are cited as the user's ruling names them; not re-fetched in this dispatch.

## Wave: DESIGN / [REF] Finding that changes DISCUSS (read this first)

**Slice 03's production premise does not hold.** `.claude/rules/development.md` § "Ground the premise" requires a named production path. Traced:

- The control-plane routes are submit, describe, stop, restart, alloc status, nodes, cluster info, routes and gateway status (`lib.rs:4897-4906`). There is no workload delete route. Nothing in `handlers.rs` or the action shim deletes `workloads/<id>`; the only `delete(IntentKey::for_workload(..))` is the test fixture `probe_count_fixture.rs:675`. `workload stop` keeps the spec key (`desired.job` stays `Some`).
- `service_vip_release_emission` needs `desired.job.is_none()` and `desired.service_spec_digest.is_some()`. `read_job` returns `(None, None, ..)` when the key is absent and `Some(digest)` only with `Some(job)`. The two conditions cannot both hold from the production hydrate. The release acceptance tests build the state by hand.
- So `ReleaseServiceVip` is not reachable through `serve` + `deploy` today, the `released_for_deletion` latch cannot misfire in production today, and D14's seeded reproduction ("delete again") would have to hand-install a deletion the product cannot perform (D11 forbids this).
- D8 ("decide from allocator state through `service_vip_view`") is also under-determined: the allocator memo is keyed by spec digest, and after withdrawal the digest is no longer derivable from intent. A correct release decision needs deletion semantics (a retained digest, or an allocator index by workload) that exist nowhere yet.

The user ruled on this finding (D-5, 2026-11-13): the VIP-release work moves to GH #211, the deletion feature, which supplies the deletion path and the post-withdrawal digest source. DISCUSS's ground-the-premise table row "`released_for_deletion` latch: Yes" was wrong for the same reason.

## Wave: DESIGN / [REF] Decisions (user-approved 2026-11-13; independent design review pending)

The user approved these rows on 2026-11-13 and 2026-11-15 (see "User approvals" below). The ADRs stay `Proposed` and independent design review has not run. The `quint-modeler` dispatch is still to run.

### User approvals (2026-11-13)

| ID | Verdict | Conditions and changes |
|---|---|---|
| D-1 | APPROVED | ADR-0282 as proposed, plus: cite #275 (host-edge wakeup, open; a future host watcher submits through the same `submit_evaluation`, the pass is its level-triggered backstop); state that no reconciler can ask for a faster sweep than the global period. Includes the read-only accessor `pending_keys(&self) -> Vec<(ReconcilerName, TargetResource)>` on `BrokerDrain`. |
| D-2 | APPROVED | 30 s stays a compile-time constant; the limitation is tracked by #356 ("Make the reconciler resync period operator-configurable", created by the user). |
| D-3 | APPROVED | None. |
| D-4 | WITHDRAWN 2026-11-15 | The premise is unreachable: no production path produces a local backend. A Service backend address is always the Running allocation's `workload_addr` from the guest network plan (`action_shim/mod.rs:1991`, `:2458`; `GUEST_NODE_PREFIX` 100.95.0.0/16, `guest_network.rs:561`, `guest_transport/mod.rs:594`), never `host_ipv4`; the hydrator's local partition (`service_map_hydrator.rs:356-370`) is always empty; non-VM provisioning was deleted (`fc3bc7d04`); no other caller of `register_local_backend` exists. Replaced by D-9. |
| D-4b | WITHDRAWN 2026-11-15 | Same evidence as D-4: the row would record attempts of a path that cannot run. Its ten sub-decisions (D-4b-1 to D-4b-10), the proposed ADR-0284 and the errno proposal are withdrawn with it. |
| D-5 | APPROVED | Slice 03 shrinks to the `release_service_vip` executor doc fix. Replacing `released_for_deletion` (D8) and the D14 reproduction move to #211. This feature does not wait on #211. |
| D-6 | APPROVED | None. |
| D-7 | APPROVED | None. |
| D-8 | APPROVED | None. |
| D-9 | APPROVED 2026-11-15 | Delete the ADR-0053 same-host local-backend path (ADR-0283, which supersedes ADR-0053). It becomes slice 02 and is scheduled first in DELIVER. The user delegated the placement. |
| D-9, D-10 | RE-CONFIRMED 2026-11-15 | The user re-confirmed deleting the whole ADR-0053 local-backend path, including `cgroup_connect4_service`, `LOCAL_BACKEND_MAP`, ADR-0164's datagram clause and the owner's cgroup-placement check, after weighing the alternative of keeping the connect4/datagram pieces for #310. Reason: `ovd_fwd` forwarding between a connected host UDP socket and a vsock stream is independent of the connect4 rewrite; with `LOCAL_BACKEND_MAP` always empty the hook is a pass-through for every owner connect today, so removing it changes no working flow; #310 designs its own destination selection. The option B question (refuse VIP-range datagrams with `FlowRefusal::Policy`) is NOT decided by this re-confirmation; it stays open (see "Open questions"). |
| D-10 | APPROVED 2026-11-15 | ADR-0283 also supersedes ADR-0164's datagram clause (guest datagrams to a VIP use the ADR-0053 `connect4` rewrite) and its startup check (the owner refuses to start unless its cgroup is at or below the `connect4` attach path). ADR-0164's TCP-as-mesh decision (D24a) stays. Guest UDP to a VIP is unsupported until GH #310. |

### Decision rows

| ID | Decision | Records |
|---|---|---|
| D-1 | Reconcilers declare managed targets; the runtime resyncs them each period; local requeue on error is not added; `resync_schedule` and the router's periodic relist are removed | ADR-0282 |
| D-2 | The resync period is a 30 s compile-time constant, not an operator setting; the scope limitation is tracked by #356 | O5, O2, ADR-0282 |
| D-3 | `service-map-hydrator` implements the existing `next_evaluation_at` hook for its retry deadlines | no ADR (existing hook) |
| D-4, D-4b | Withdrawn 2026-11-15 (see approvals table) | none |
| D-5 | Slice 03 is the `release_service_vip` executor doc fix only (it says the error is absorbed; it returns `ShimError::AllocatorRelease`). Replacing `released_for_deletion` (D8) and the D14 reproduction moved to #211 | this delta |
| D-6 | `startup_last_acted_run` and `liveness_last_acted_run` are recorded as compliant retry pacing unless the slice 04 property finds a reachable lost stop | this delta |
| D-7 | The slice 04 property drives the production convergence tick over sim-composed ports; no new sim executor | this delta |
| D-8 | `workflow-lifecycle` gets no dedicated wake (O3 confirmed); it is covered by the uniform pass at zero cost | O3 |
| D-9 | The same-host local-backend path is deleted (slice 02): programs, maps, handles, `Dataplane` operations, actions and executors, the hydrator local partition and its View fingerprint, with their tests | ADR-0283 (supersedes ADR-0053) |
| D-10 | ADR-0164's datagram clause and its owner cgroup-placement startup check are deleted with the path (slice 02). Guest UDP to a VIP is unsupported until GH #310 (UDP service delivery to VM workloads; requires kernel-resident forwarding, no userspace relay; one option for #310 to evaluate is a service `connect4` selecting from `SERVICE_MAP` / `BACKEND_MAP` as `cgroup_connect4_gateway` does). The clause never worked for a VM backend: metal reproduction on Linux 7.0.0-38 at `fb71b400f`, three runs, `spike-scratch/guest-udp-vip-probe/evidence/run{7,8,9}/` (evidence, not coverage): `LOCAL_BACKEND_MAP` 0 entries, `SERVICE_MAP` holds the VIP's TCP+UDP keys, guest UDP to the VIP gets no reply 8/8, guest TCP to the same VIP:port works, and `getpeername` on the owner's connected host UDP socket after `connect` is still the VIP | ADR-0283 (supersedes ADR-0164 in part) |

## Wave: DESIGN / [REF] Domain model (DDD list)

| Term | Meaning |
|---|---|
| Managed target set | The targets for which one reconciler has desired or actual state, in its own target shape, enumerated from intent and observation. |
| Resync pass | One runtime sweep that submits an `Immediate` evaluation for every managed target of every registered reconciler. It is a level wake. |
| Edge wake | A submit caused by a change: an accepted observation row (`interests()`), `EnqueueEvaluation`, a client request, a self-requeue. |
| Evaluation outcome | `ok`, or an error ending the evaluation at hydrate, persist or dispatch. Each error outcome is retried by the next pass, never by a local requeue. |
| Pacing input | A View field recording when to act again (attempts, last attempt time). It never records whether an effect is in place. |

No new aggregates, bounded contexts, events or read-ports.

## Wave: DESIGN / [REF] Component decomposition

| Component | Crate | Change | Owner of |
|---|---|---|---|
| `Reconciler::managed_targets` (new method) | `overdrive-core` | EXTEND trait | the reconciler's target set |
| `AnyReconciler::managed_targets` (forward arm) | `overdrive-reconcilers` | EXTEND | none |
| Eight reconciler impls of `managed_targets` | `overdrive-reconcilers` | EXTEND | their own target sets |
| Resync pass and loop | `overdrive-control-plane` (`reconciler_runtime`, `lib.rs`) | CREATE NEW, replaces cadence phase and router periodic relist | the level wake |
| Interest router | `overdrive-control-plane` | EXTEND by deletion: periodic arm and its two parameters go | edge wakes from rows |
| `ServiceMapHydrator` | `overdrive-reconcilers` | EXTEND: `managed_targets`, `next_evaluation_at` over the remote `retries` map; local partition and fingerprint DELETED (D-9) | the remote service-map effect |
| The same-host local-backend path (inventory in slice 02) | `overdrive-bpf`, `overdrive-dataplane`, `overdrive-core`, `overdrive-control-plane`, `overdrive-reconcilers`, `overdrive-sim` | DELETE | none (no production producer, ADR-0283). Also deletes, in `overdrive-control-plane` and `overdrive-sim`, the guest-flow owner's `OwnerCgroupPlacement` startup check and datagram-to-VIP handling (D-10); inventory in slice 02 |
| Convergence property harness | `overdrive-control-plane` tests | CREATE NEW (test support, DISTILL shapes it) | none |

Dependency direction is unchanged. `overdrive-core` gains no I/O dependency.

## Wave: DESIGN / [REF] Driving and driven ports

Driving (pinned for conformance): `run_resync_pass`, `ReconcilerRuntime::submit_evaluation`, `run_convergence_tick`, `spawn_interest_router` (edge). Driven: the existing intent, observation, view and dataplane ports are unchanged (the `Dataplane` port loses its two local-backend operations, D-9).

## Wave: DESIGN / [REF] Interface contracts (D-1, D-3)

Exact shapes for new or changed public and cross-crate surface. Internal structure is the crafter's.

### D-1a. `Reconciler::managed_targets` (core)

```rust
async fn managed_targets(
    &self,
    ctx: &HydrationContext<'_>,
) -> Result<BTreeSet<TargetResource>, HydrateError>;
```

Required: no default, so omitting it does not compile. `AnyReconciler` gains a forwarding arm with the same signature.

Contract (rustdoc on the trait method):

- Preconditions: none beyond a valid `ctx`.
- Postcondition on `Ok(set)`: `set` contains every target `T` for which this reconciler's `hydrate_desired` or `hydrate_actual` would return a non-empty projection, built from intent and from observation. A target with nothing to do may be present (it evaluates to `Noop`). No target may be omitted for a non-empty projection.
- Edge cases: an empty set is valid. A read failure returns `Err` for the whole call; no partial set is returned. A target shape the reconciler does not parse is never returned.
- Invariants: read-only; no clock, no write, no dispatch. Deterministic over unchanged state. Size is bounded by the number of managed resources.
- Like `hydrate_*`, it carries no `&dyn Clock`.

Per-reconciler sets (confirm in code before DELIVER; sources named, shapes already parsed by each `hydrate_*`):

| Reconciler | Targets enumerated from |
|---|---|
| `workload-lifecycle` | `workload/<id>` for each canonical `workloads/` intent (via `IntentKey::workload_prefix` and `is_canonical_workload_record`) ∪ each distinct `alloc_status` row `workload_id` |
| `service-lifecycle` | the same workload ids restricted to Service kind (kind from the intent discriminator or the row) |
| `svid-lifecycle` | workload ids of `alloc_status` rows ∪ workload ids with a held SVID (`HeldSvidView`) |
| `service-map-hydrator` | `service/<id>` for each `service_id` in `service_backends` rows ∪ `service_hydration_results` rows |
| `workflow-lifecycle` | workflow instances under `IntentKey::workflow_instance_prefix`; empty in production |
| `vm-reclamation`, `gateway-svid-lifecycle` | their single node-scoped target (today `node/<id>`) |
| `noop-heartbeat` | empty set (no-obligation ruling: no desired or actual state) |

The Rust shared scan helper `scan_workload_intents` and the structural classifier (`rust.md` § "A key prefix and its record classifier are ONE SSOT") are the enumeration source; no hand-written prefix literal and no suffix skip list.

### D-1b. Resync driver (control-plane, crate-private)

```rust
pub(crate) const RESYNC_PERIOD: Duration = Duration::from_secs(30);

pub(crate) struct ResyncReport { /* fields are the crafter's */ }
impl ResyncReport {
    pub(crate) fn submitted(&self) -> usize;
    pub(crate) fn enumeration_failures(&self) -> &[(ReconcilerName, HydrateError)];
}

pub(crate) async fn run_resync_pass(state: &AppState) -> ResyncReport;

pub(crate) fn spawn_resync_loop(
    state: AppState,
    clock: Arc<dyn Clock>,
    period: Duration,
    shutdown: CancellationToken,
) -> tokio::task::JoinHandle<()>;
```

`run_resync_pass` behaviour:

- Locks the allocator and listener-fact guards and builds one `HydrationContext`, as the tick does, and releases them before submitting.
- For each registered reconciler in `BTreeMap` order, calls `managed_targets`. On `Err` it records the failure, logs a structured event naming reconciler and error, and continues with the next reconciler. A failure never aborts the pass and never submits a partial set.
- Submits one `Evaluation` per target through `ReconcilerRuntime::submit_evaluation` with `Immediate` eligibility. It writes no observation row and holds no lock across a submit.
- Contract shape: bounded-change. Its whole effect universe is `submit_evaluation` calls.

`spawn_resync_loop`: runs a pass immediately, then anchors the next pass at `previous + period` on the injected clock (re-armed at most once per period, no drift), `biased` on `shutdown.cancelled()`. The clock is the same instance the convergence loop reads. Production passes `RESYNC_PERIOD`; DST passes a sim period.

Retry rule (`development.md` § "Reconciler I/O", ADR-0282 decision 3): `run_convergence_tick_inner` is unchanged in control flow. A hydrate error, a persist error and a dispatch error end the evaluation; the next pass is the retry. The existing dispatch-error self-requeue and `next_evaluation_at` requeue stay.

### D-1c. Removals (same commit as D-1a and D-1b)

`Reconciler::resync_schedule`, `ResyncSchedule`, `ResyncScope`, `resolve_scope`, `AnyReconciler::resync_schedule`, `build_cadence_table`, `arm_next_wake`, `due_resync_evaluations`, the `vm-reclamation` and `gateway-svid-lifecycle` overrides, `INTEREST_ROUTER_RELIST_PERIOD`, and the periodic arm of the router. `spawn_interest_router` reverts to:

```rust
pub fn spawn_interest_router(
    obs: Arc<dyn ObservationStore>,
    subscription: LagAwareSubscription,
    interest_table: BTreeMap<ObservationRowKind, Vec<ReconcilerName>>,
    broker: InterestRouterBroker,
    shutdown: CancellationToken,
) -> tokio::task::JoinHandle<()>;
```

The tests and acceptance fixtures that assert the removed items go in the same commit (`development.md` § "Deletion discipline"). The `vm-reclamation` boot-time evaluation drive stays unless the crafter shows it redundant with the first pass; the design does not require removing it.

### D-3. `ServiceMapHydrator::next_evaluation_at`

Returns the earliest future `last_failure_seen_at + backoff_for_attempt(attempts)` over the remote `retries` map for a service whose observed state differs from desired; `None` otherwise. It reads only its inputs. Existing hook; no trait change.

## Wave: DESIGN / [REF] Reuse Analysis

| Existing | Verdict | Justification and contract shape |
|---|---|---|
| `Reconciler` trait, `AnyReconciler` | EXTEND | One required method; the `hydrate_*` precedent. |
| `HydrationContext`, `HydrateError` | UNCHANGED | `managed_targets` reads through the existing bundle. |
| `ReconcilerRuntime::submit_evaluation`, broker LWW | EXTEND (unchanged) | Pass submits through it (C-A1 holds). Contract shape: bounded-change; universe = pending map keys; assertion = `pending_keys` and `counters`. |
| `resync_schedule` family, cadence table | DELETE | Superseded; same 30 s as both users. |
| Router periodic relist | DELETE | Subsumed by the pass. `Lagged` relist stays. |
| `spawn_convergence_loop` | EXTEND by deletion of the cadence phase | Admission, cap and drain unchanged. |
| `ServiceHydrationResultRow` + executor row | REUSE (remote path only) | Unchanged. |
| `Dataplane` trait | EXTEND by deletion | Loses `register_local_backend` and `deregister_local_backend` and the error variants only they use (D-9). Contract shape of the remaining operations unchanged. |
| Same-host local-backend path (programs, maps, handles, actions, executors, hydrator partition, sim state) | DELETE | No production producer (ADR-0283). Inventory in slice 02. `cgroup_connect4_gateway` and its maps are kept. |
| `ServiceVipView`, allocator | UNCHANGED | The VIP-release work moved to #211 (D-5). |
| `vm_reclamation` `apply_actions` | REJECT for reuse | Pure-core DST executor; slice 04 uses the production shim (D-7). |
| `resync loop` | CREATE NEW | No existing component enumerates managed targets across reconcilers; justified by the verification's four wake gaps. |
| `BrokerDrain::pending_keys` accessor | CREATE NEW (read-only) | Pinned so the conformance driver has a projection; see Formal protocol model. |

## Wave: DESIGN / [REF] Slice 03, 04 and the remaining proposals

### D-5. Slice 03 re-scoped (approved 2026-11-13)

Do now: correct the `release_service_vip` executor doc (AC4: it says the error is absorbed; it returns `ShimError::AllocatorRelease`). Moved to GH #211 ("Implement workload deletion (intent withdrawal) + service/dataplane teardown", open): replacing `released_for_deletion` (D8) and the D14 reproduction. #211 was extended by comment https://github.com/overdrive-sh/overdrive/issues/211#issuecomment-6866541631 to require: release decided from allocator state via `service_vip_view`, retried until the address is free, `released_for_deletion` deleted, designed together with the post-withdrawal digest source (an earlier #211 comment already requires the deletion verb to supply the spec_digest), and the same retry rule for the dial-by-name frontend `F`. `ServiceVipView::assigned_vip(digest)` alone cannot enumerate, which is why the digest source is part of #211's design. This feature does NOT wait on #211; #211 is not a prerequisite. The new View rule (below) lists `released_for_deletion` as a known violation pending #211, so the rule text is truthful.

Alternative: implement D8 now against a hand-built deletion fixture. It loses on D11 (a test installs a state production cannot produce), on `design.md` § "Ground the premise", and because the digest source is undecided.

### D-6. Stop markers (US-04 assessment)

The stop decision is already derived from `actual`: the allocation is `Running` and the producer-recorded probe streak crossed. The markers are pacing keyed to the probe runner's own `run_seq`; accepted ADR-0260 (startup) and the code comment citing ADR-0191 (liveness) state that a failed stop is retried once per new probe row while the allocation stays `Running`. The retry clock is the probe runner. A lost stop is reachable only if the runner stops producing rows for a `Running` allocation. Liveness supervision is not cancelled while the allocation runs (`cancel_role(Startup)` follows a Pass). For the startup runner after its deadline, I did not read the supervisor and do not claim either way. Proposal: record both as compliant pacing; the slice 04 property includes a "probe runner quiescent after a dropped stop" stimulus; only a red result opens a fix (the marker keeps WHEN, retry pacing comes from `next_evaluation_at` plus the pass). O6 folds in only if that fix edits the two fixtures.

### D-7. Slice 04 property shape

- The property drives the production `run_convergence_tick` over a sim-composed `AppState` (the pattern `workload_gc_absent_intent.rs` uses): hydrate, reconcile, View persist and the real action shim run against `Sim*` ports. No generic sim executor is written. A hand-written executor re-implements the shim and is the "driver compares the model with itself" shape that `testing.md` rejects; the hydrator DST's fabricated `Completed` row is that shape today.
- Faults ("dropped actions") are injected through existing or small driven-port seams: service-map update failure, allocator release failure, `SimViewStore::inject_fsync_failure`, observation read failure, `SimDriver` stop failure. That is `testing.md`'s "drive causes, not fabricated consequences". Starting worlds and Views are arbitrary (including a View that says done while the effect is absent); crash-partial worlds are reachable states.
- Oracle: `actual == desired` through the reconciler's own read-port within N passes, then zero non-`Noop` actions (progress, stability); executor idempotency by dispatching each action twice.
- Production surface the property requires: only `run_resync_pass` (the retry step between ticks) and, for the seams, sim-side injectors DISTILL names. Fixture internals are DISTILL's.
- Lane: DISCUSS AC2 says default lane. A composed `AppState` needs an intent store; `lib.rs:522` builds redb with `InMemoryBackend`, which may make a no-file default-lane build possible. If not, the property sits under `integration-tests` and AC2 changes. DISTILL decides (validation item V-5; not contract-shaping).
- Reconcilers.md rule change: see "Proposed rule text". The one exception is `ServiceLifecycleView::last_emitted_backend_fingerprint` (D13), plus `released_for_deletion` (pending #211, D-5) and `observed_generation` (#325).

### D-8 (O3). `workflow-lifecycle`

Confirmed: no dedicated wake. It is registered, so it gets a required `managed_targets`; the production set is empty because the registry is empty. The method closes the wake for the day the first production workflow lands as a side effect, not as scope.

## Wave: DESIGN / [REF] Proposed rule text (drafted here; lands in DELIVER)

Not applied now. Timeless discipline only, per the memory ruling on rules docs.

### `reconcilers.md` — replaces the fingerprint-shaped symptom and the "never both" wakeup paragraph

> **A View decides when to act, never whether an effect is in place.** A View field may carry pacing inputs (attempts, last attempt time, a retry budget, a rate limit). It never records that an effect landed or was emitted. Whether the effect is in place is decided from `actual`, read from the resource the reconciler manages. If the View says done and `actual` says not done, `actual` wins. If the effect cannot be observed, make it observable first; a result row of the last attempt is a receipt, not the resource.
>
> Known exception: `ServiceLifecycleView::last_emitted_backend_fingerprint` stays while the backend row has two writers; it dedups a write the reconciler cannot read back for ownership reasons.
>
> **Every reconciler declares its managed targets.** The runtime re-evaluates all of them every period, from intent and observation, whatever woke them before. `interests()` and `EnqueueEvaluation` are latency; the pass is the guarantee. An evaluation that fails to hydrate, persist or dispatch is not requeued locally; the next pass retries it, and `next_evaluation_at` supplies a precise retry deadline when the reconciler has one.

Symptoms to list: a View field consulted as the diff; a field stamped on emit; a reconciler whose only wake is an edge; a managed-target method that enumerates one observation family only; a test or simulation invariant that seeds the View to represent "converged" instead of asserting on `actual`.

Precedent for the last symptom (recorded here, not in the rule text; the history of the anti-pattern stays valid although the path was later deleted, D-9): commit `6af937ae3` (2026-06-24) added `last_applied_local_fingerprint` because the local effect was unobservable, justified it as "persist the input", and changed the `hydrator-idempotent-steady-state` DST invariant to pre-seed that View field so the test agreed with the marker.

### `development.md` § "Reconciler I/O" (D15)

Replace "an unreadable observation is the runtime's to log-and-skip" with: a hydrate error ends that evaluation with no View write and no dispatch; the runtime logs it and the next resync pass re-evaluates the target. Document `next_evaluation_at` (pure, called only when the tick emitted no non-`Noop` action; `Some` retains the evaluation until that instant) beside `resync_schedule`'s removal and `managed_targets`.

## Wave: DESIGN / [REF] Lifecycle Gate Ownership

State-ownership matrix (existing signals this feature touches):

| Signal or state | Owning component | Promise it makes | Inputs that may gate it | States it must not gate |
|---|---|---|---|---|
| `Running` / `Stable` allocation | `workload-lifecycle` / `service-lifecycle` rows | as in ADR-0087, ADR-0265 | unchanged | any resync, hydrate-read or map-read outcome |
| `service_hydration_results` row | the `DataplaneUpdateService` executor | outcome of the last remote update | unchanged | none new |

The feature adds and moves no gate. It removes one boot refusal: the owner's `OwnerCgroupPlacement` startup check (D-10), which guarded only the removed datagram rewrite and which no production boot reached, because `run_server` enrols the process into `overdrive.slice/control-plane.slice` before the check runs. The deletion removes a path no production caller reaches; it changes no state meaning. The resync pass is a wake, not a gate: it submits evaluations and decides nothing about any state above.

## Wave: DESIGN / [REF] Formal protocol model

**Required** (changes an ordered, concurrent and crash-sensitive protocol). Not written or run here; pending `quint-modeler` dispatch by the user. Directory: `specs/quint/reconciler-evaluation-wake/` (permanent, referenced by DISTILL conformance).

Spec A — evaluation wake (decision D-1). Owners: broker (pending map keyed by `(reconciler, target)`, eligibility `Immediate` or `NotBefore`), convergence loop (cap 8, one in-flight per target), resync pass, edge wakes, tick outcomes (`ok`, hydrate error, persist error, dispatch error). World: a target with `desired != actual` per reconciler. Environment faults as actions: each tick outcome error, enumeration failure for one reconciler, crash between View fsync and dispatch.

- Safety: at most one in-flight evaluation per target; a pending key is unique; a pass never submits a partial set.
- Progress (under fairness of the pass and of admission): every target with `desired != actual` is evaluated again within one period plus queue time, from every error outcome.
- Named runs (each listed in `checks.toml`): `healthy_edge_converges`; `hydrate_error_recovers_by_pass`; `persist_error_recovers_by_pass`; `failed_dataplane_inside_backoff_recovers_without_row_write`; `fresh_deploy_without_row_is_evaluated`; `enumeration_failure_of_one_reconciler_does_not_starve_others`; `edge_wake_during_burst_is_admitted_after_burst`.
- Hazard variants, one per decision: D-1 → remove the pass (hazard `no_pass`: the hydrate-error wedge reappears as a violation of progress).
- Assumptions to discharge: the broker's LWW and eligibility merge (proven in `eval_broker.rs` unit tests; V-4), `submit_evaluation` wakes the loop (ADR-0273).

There is one specification. The local-backend convergence model (former Spec B) is withdrawn with D-4 (D-9).

Pinned observable projections for the conformance drivers (`testing.md` § "Quint conformance"):

| Spec variable | Owner | Accessor | Status |
|---|---|---|---|
| pending evaluation keys | `BrokerDrain` | `pending_keys(&self) -> Vec<(ReconcilerName, TargetResource)>` | NEW, read-only (approved with D-1, 2026-11-13) |
| dispatched and cancelled counts | `BrokerDrain` | `counters()` | existing |
| in-flight targets | convergence loop | model-only (not projected) | ruling: the loop's `active_targets` is local to the task; not exposed |
| eligibility | broker | model-only | ruling: verified by broker unit tests |
| View pacing | `RedbViewStore`/`SimViewStore` | existing `get_view` | existing |

Action mapping: spec actions map to `run_resync_pass`, `submit_evaluation`, `run_convergence_tick` and the shim's driving port, with `Sim*` at the driven ports. A `Sim*` adapter is the arbiter for no variable.

## Wave: DESIGN / [REF] Validation items

Contract-shaping items run before approval; I have not run them and mark none proven.

| ID | Item | Class | Probe to run | Falsification changes |
|---|---|---|---|---|
| V-1, V-2 | Withdrawn 2026-11-15 with D-4 (the readback they validated no longer exists). The V-1 findings file (`docs/feature/reconciler-convergence-guarantees/spike/findings-v1.md`) and the probe `spike-scratch/reconciler-convergence-v1/` validated the withdrawn D-4a and are deleted by the user; nothing in this feature cites them. | n/a | n/a | n/a |
| V-3 | Cost: per-evaluation hydrate time and admission delay of an edge wake queued behind a burst at 1000 workloads | not contract-shaping (pacing is internal) | sim measurement with `SimClock` queue depth, then metal; compare with the 3% guardrail G1 | the pass's internal pacing (stagger), not the contract. `workload-lifecycle` hydrate scans every `alloc_status` row per evaluation; the per-pass cost grows with targets squared until #270. |
| V-4 | Same-key collapse and `NotBefore`→`Immediate` merge under pass plus edge submits | not contract-shaping | existing `eval_broker` tests plus the new run `edge_wake_during_burst_is_admitted_after_burst` | a test |
| V-5 | Default-lane feasibility of the composed property (in-memory redb) | not contract-shaping | DISTILL builds one composed tick in the default lane | the lane of one test |
| V-6 | Whether the startup probe runner keeps producing rows for a `Running` allocation after the startup deadline | not contract-shaping (assessment) | Tier-1 stimulus in the slice 04 property | whether D-6 needs a fix |

No contract-shaping validation item is open. D-1 stands on source reading. The deletion (D-9) adds no kernel assumption.

## Wave: DESIGN / [REF] Quality attributes and enforcement

- Reliability: every wake gap in the verification closes by the pass; recovery bound = period plus queue time.
- Maintainability: one backstop instead of three; a required method makes omission a compile error.
- Testability: managed-target sets and the diff are pure or sim-drivable; the composed property reuses production code.
- Enforcement: the existing dst-lint purity scan over `overdrive-reconcilers/src/**` covers the new method (no clock, no wall time); the compile-guard that `hydrate_*` carry no `&dyn Clock` extends to `managed_targets`; `cargo xtask conformance-lint` covers the two conformance drivers; the boundary test (S-ROH-B-09) is updated for the field set. No new tool.
- No external integrations; no contract-test annotation.
- Paradigm: object-oriented per `CLAUDE.md`.

## Wave: DESIGN / [REF] Changed Assumptions

| Superseded contract (quoted) | New proposal |
|---|---|
| ADR-0084 amendment 2026-08-23: "The level-triggered backstop for the row-backed partition ... is the interest-router's unconditional periodic relist — NOT a per-reconciler `resync_schedule`" | The backstop for every reconciler is the runtime resync pass over `managed_targets`. |
| `reconcilers.md`: "A reconciler declares one model or the other, never both" | Every reconciler is resynced; `interests()` is edge-only. |
| ADR-0053 (Accepted 2026-05-23): same-host backend delivery via `cgroup_sock_addr` | Superseded by ADR-0283 (D-9): the path has no production producer and is removed. |
| ADR-0164: "Datagrams to a VIP use `connect4`. The owner always connects its host datagram socket, so the existing ADR-0053 `connect4` rewrite resolves a VIP to a platform-chosen backend." and "The owner refuses to start unless its own cgroup is the `connect4` attach path or below it." | Superseded by ADR-0283 (D-10). Guest UDP to a VIP is unsupported until GH #310. ADR-0164's TCP decision (D24a) is unchanged. |
| `development.md`: "an unreadable observation is the runtime's to log-and-skip" | A hydrate error ends the evaluation; the next pass retries (D15). |
| DISCUSS ground-the-premise row 3: `released_for_deletion` latch reachable in a real deploy | Not reachable through production today (see Finding); the VIP-release work moved to #211 (D-5, approved 2026-11-13). |

## Wave: DESIGN / [REF] Open questions and user-approval gates

- Scope limitation, tracked: the 30 s resync period is a compile-time constant with no configuration surface (D-2). Tracked by #356 ("Make the reconciler resync period operator-configurable"). RULED 2026-11-13.
- D-4 and D-4b withdrawn, D-9 approved (2026-11-15): the local-backend path is deleted (ADR-0283, slice 02).
- RULED 2026-11-15 (D-10): ADR-0164's datagram clause and startup check are superseded by ADR-0283 and deleted in slice 02. Guest UDP to a VIP is unsupported until GH #310 (UDP service delivery to VM workloads; deferred by the user 2026-10-30; kernel-resident forwarding, no userspace relay; reconciles ADR-0164's guest-prefix UDP refusal). A comment on #310 records the removal. #310 is cited as the user's ruling names it; not re-fetched in this dispatch.
- OPEN, awaiting the user (outcome-changing, not decided here; the 2026-11-15 re-confirmation of D-9/D-10 does not decide it): once the owner's VIP branch is deleted, a guest UDP datagram to a VIP-range address takes the owner's generic path (route-type check, marked socket, `connect`, send to the VIP on the host network). Today it takes the VIP branch (unmarked socket, no route check) and gets no reply. Either way the datagram is not served. Option A: leave it on the generic path (no new behaviour; the datagram leaves the host toward an address nothing answers). Option B: refuse it `FlowRefusal::Policy` for a destination in the configured VIP range, mirroring ADR-0164's TCP fail-closed rule for an unmatched VIP (an existing refusal variant; no new surface, but new behaviour that needs a test and the user's approval). Recommendation: B, because it makes the unsupported state explicit to the guest instead of a silent timeout. Lifting the limitation is #310's work either way.
- Slice 03 re-scope (D-5): ruled. The VIP-release work moved to #211 (verified, with the extension comment). No new issue needed for it.
- Lane for the slice 04 property (V-5).
- Dispatch pending, to be run by the user: `quint-modeler` for `specs/quint/reconciler-evaluation-wake/` (the one specification), with the decision id D-1, then `cargo xtask quint typecheck` and `check`, then evidence record.
- Independent design review has not run; the ADRs stay `Proposed`.
- Not done by design, by instruction: no reviewer dispatch, no spikes run, no issue created by this dispatch, no code or rules file edited.

### DELIVER follow-ups outside `crates/` for slice 02 (DESIGN does not edit rules or website)

- `.claude/rules/debugging.md` § 11 uses `LOCAL_BACKEND_MAP` as the single-node local example: replace the example in the deletion commit. `.claude/rules/development.md` § "`bpf_sock_addr.user_port`" cites the ADR-0053 walking skeleton: the rule stays valid for the gateway's `cgroup_connect4_gateway`; change only the citation and example.
- Website (FINALIZE): `website/content/blog/what-is-ebpf.mdx` (2 hits for the local-backend path); no `website/content/docs` hit.
- Other ADRs touching the path are listed in the DESIGN rework report; ADR-0053 is superseded in full and ADR-0164 in part (D-10).
- `docs/product/architecture/brief.md` and `c4-diagrams.md` were corrected on 2026-11-15 (guest UDP to a VIP unsupported until #310; supersession banners on the ADR-0053 local-backend sections; the `Service connect4` container and its relation removed from the guest-flow C4).

## Wave: DESIGN / [REF] Handoff to DISTILL (after the Quint dispatch and independent design review)

User approvals are recorded above (2026-11-13, 2026-11-15). DISTILL starts only when the Quint dispatch and independent design review are done, and those verdicts are recorded.

Scenario obligations: the slice 01 invariants (three faults plus fresh deploy, enumeration failure, burst admission); slice 02 (the deletion: AC1 to AC5 of the slice file; no new scenario beyond unchanged behaviour of the remote service path and the gateway); slice 03 (executor doc fix, no scenario); slice 04 (the composed property, mutation check per injected latch mutant, and the quiescent-probe-runner stimulus for D-6). Quint conformance drivers per the pinned projections. Each slice stays production-drivable through `serve` + `deploy` with a real reachable fault (D11). The VIP-release scenarios (US-03, D14) belong to #211 and are not in this feature; this feature does not wait on #211.
