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

- **Ana Moreno** (`ana-platform-engineer`) — operator who deploys Service workloads and trusts the platform to converge to what she declared. Beneficiary of the runtime story (US-01).
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
| D1 | Scope is P1 (periodic re-evaluation), P2 (remove View latches that decide effect presence), P3 (shared multi-tick convergence property), and the `reconcilers.md` rule change. | User |
| D2 | `workload-lifecycle.observed_generation` stays in #325. Not in this feature. | User |
| D3 | Boot-only host-infra convergence (#197 veth, #198 cgroup, #199 XDP/cgroup-sock attach, #234 TPROXY routing) is out of scope. | User |
| D4 | P1 enumerates targets from intent AND observation, not `alloc_status` rows alone. | User |
| D5 | The service-lifecycle `stable_announced`/`terminal_announced` latch claim is refuted: `stable_announced` is deleted, and `terminal_announced` does gate liveness supervision of an allocation whose startup stop was emitted, but it does not decide whether a stop is in place (D-6). `startup_last_acted_run` and `liveness_last_acted_run` stay as ADR-0191 decision 5 decides (D-6). None is in scope. | Verification finding 3; user ruling D-6 |
| D6 | `workflow-lifecycle` being un-woken is NOT a production defect: `serve` builds an empty `WorkflowRegistry` and nothing writes `workflows/` intent in production. Not scope. | Verification finding 5, `development.md` § "Ground the premise" |
| D9 | The View may decide WHEN to act (backoff, budgets, rate limits, a marker keyed to an observed input). It never decides WHETHER the effect is in place; only `actual` decides that. | Issue rule text; user chose the rule change |
| D10 | Every new `actual` read introduced by P2 carries a DESIGN note naming the guard that makes it safe under LWW convergence (cross-reference #344). | User |
| D11 | Each slice is production-drivable through `overdrive serve` + `overdrive deploy` with a fault a real deploy can reach. A test that hand-installs what production does not is not acceptance. | CLAUDE.md § "Build vertical slices" |
| D13 | The new View rule carries no exception. `ServiceLifecycleView::last_emitted_backend_fingerprint`, which the 2026-11-13 ruling would have named as the single exception, no longer exists: ADR-0101 deleted it (no match in `crates/`). | Verified in code, 2026-11-15 |
| D15 | The `development.md` "log-and-skip" text is corrected and `next_evaluation_at` is documented in the rules, in the same commit as slice 01's behaviour change. | User, 2026-11-13 |
| D12 | One feature with three slices (01, 02, 04). Periodic re-evaluation (slice 01) is not split into its own feature; the slices ship independently. | User, 2026-11-13 |

## Wave: DISCUSS / [REF] Ground the premise (per production path)

| Defect | Reachable in a real deploy? | Production path |
|---|---|---|
| Hydrator local-backend latch | **No; deleted by D-9.** | The hydrator emits `RegisterLocalBackend` only for a backend whose address equals `host_ipv4` (`service_map_hydrator.rs:356-370`). Since ADR-0276 every Service backend address is the Running allocation's `workload_addr`, taken from the guest network plan (`action_shim/mod.rs:1991`, `:2458`) and allocated from `GUEST_NODE_PREFIX` 100.95.0.0/16 (`guest_network.rs:561`, `guest_transport/mod.rs:594`), never `host_ipv4`. Non-VM provisioning was deleted (`fc3bc7d04`). The local set is always empty in a real deploy. |
| Hydrator un-woken after `Failed` result | Yes | `DataplaneUpdateService` `Err` writes a `Failed` row; next tick inside backoff yields `Noop` with no `next_evaluation_at`; a steady service gets no further backend-row write to wake it. |
| Hydrate error skips requeue | Yes, rare | `RunningAllocationWithoutWorkloadAddr` raised by `service-lifecycle` hydrate (data-dependent), or store read errors; early `?` at `rt:1589-1590`. |
| View write error skips dispatch and requeue | Yes, rare | `persist_view` redb write failure, early `?` at `rt:1638-1642`. |
| Fresh deploy with no `alloc_status` row never relisted | Yes, rare | First `workload-lifecycle` hydrate fails; `handlers.rs:521` submitted once; relist routes only `alloc_status` rows; no boot seeding of existing intents. |
| `workflow-lifecycle` un-woken | No | Empty registry in `serve`. |

## Wave: DISCUSS / [REF] Scope Assessment

Result: **OVERSIZED SIGNALS PRESENT; split considered and rejected by the user (D12). One feature, three independently shippable slices (01, 02, 04).**

- Stories: 4 (US-01, US-05, US-06, and US-04 resolved with no change). Under the 10-story signal.
- Bounded contexts touched: 4 (control-plane runtime, reconcilers crate, dataplane/allocator adapters, sim + core test surface). Over the 3-context signal. (signal 1)
- Independent outcomes that could ship separately: yes, P1 is independent of P2/P3. (signal 2)
- Effort: about 8 days across 3 slices, under the 2-week signal.
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
- [ ] AC5: At a converged state, re-evaluation emits zero non-`Noop` actions, and evaluation cost per period is bounded by target count and no greater than today's relist (G1).
- [ ] AC6: Tier-1 seeded `overdrive-sim` invariant, seed printed on failure, covers the three fault stimuli and the recovery bound.

#### Technical notes
- The runtime-wide periodic pass changes `reconcilers.md` § "The wakeup model" (a reconciler declares row-backed OR host-backed, "never both") and ADR-0084; reconciled by ADR-0282 (D-1, approved).
- The no-busy-loop rule holds: re-evaluation writes no observation row.
- Period: 30 s constant (D-2). Enumeration: `managed_targets` (D-1a). Interacts with #270 and #271.

### US-04: The probe-driven stop markers stay (no change)

DISCUSS assessed `service-lifecycle`'s `startup_last_acted_run` / `liveness_last_acted_run` as a record-on-emit shape. They are not changed: each is a marker keyed to an observed input (the probe row's `run_seq`) that paces a stop, and ADR-0191 decision 5 (user ruling D-1 option (a), #295, 2026-10-13) requires it. No story, no acceptance criterion, no code change. See D-6.

### US-05: Reconciler authors get a shared multi-tick convergence check

`job_id: J-PLAT-004` · Persona: Devon · Entry point: the repository's default-lane reconciler test run (developer-tool story; the entry point is the test lane, not an operator command).

#### Problem
Devon adds a reconciler with a "done" flag in its View. Every existing check is a scripted scenario or a single-tick pure property, and none generates an arbitrary starting `(actual, view)`, runs multiple ticks, or drops actions. The hydrator's local-backend latch passed every test.

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
- [ ] AC1: Every reconciler registered in `run_server` is covered, or a reasoned no-obligation ruling is recorded per exempt one: 7 always (`noop-heartbeat`, `workload-lifecycle`, `workflow-lifecycle`, `svid-lifecycle`, `vm-reclamation`, `service-map-hydrator`, `service-lifecycle`), plus `gateway-svid-lifecycle` when `config.gateway` is `Some`, so 7, or 8 with the gateway.
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
- [ ] AC1: `.claude/rules/reconcilers.md` replaces the fingerprint-shaped symptom with the general prohibition in D9 (see "Proposed rule text"), including the test "if the View says done and `actual` says not done, `actual` wins" and "if the effect cannot be observed, make it observable first".
- [ ] AC2: The rule is timeless discipline (principle plus symptoms); no ADR numbers as status, no DELIVER-phase notes.
- [ ] AC3: The rule names no exception (D13: `last_emitted_backend_fingerprint` no longer exists).

## Wave: DISCUSS / [REF] Story map and prioritization

Backbone (operator path): Deploy → Reconcile tick → Dispatch effect → Observe result → Converged. Author path: Write reconciler → Check convergence.

| Slice | Stories | Walks | Value | Urgency | Effort | Order |
|---|---|---|---|---|---|---|
| 01 periodic-reevaluation | US-01 | Reconcile tick → Converged | 5 | 5 | 3 | 1 |
| 04 convergence-property-and-view-rule | US-05, US-06, executor doc fix (D-5) | Write reconciler → Check | 4 | 3 | 3 to 4 | 4 |

Priority rationale: slice 01 first because it fixes the broadest class (any hydrate or View-write error wedges a deploy) with the least dependency, and it is the riskiest assumption (ADR-0084 tension). Slice 02 is the deletion of the unreachable local-backend path (its latch was the one DISCUSS thought was production-reachable); it depends on nothing and runs first in DELIVER so later slices do not touch the code. The convergence-property slice is last so the property is green on the fixed reconcilers and red on any reintroduced latch; DISTILL still authors it early.

## Wave: DISCUSS / [REF] Outcome KPIs

| # | Who | Does what | By how much | Baseline | Measured by |
|---|---|---|---|---|---|
| K1 | Ana | A workload hit by a transient hydrate or View-write error reaches Running without re-submit | 100% of seeded runs within one re-evaluation period of the fault clearing | never (wedged until re-POST) | Tier-1 seeded sim invariant, plus `serve` + `deploy` run with fault injected |
| K4 | Devon | View fields that decide effect presence in registered reconcilers, in this feature's scope | 0 | 1: the hydrator's `last_applied_local_fingerprint`, removed by deleting the unreachable local path (D-9). `released_for_deletion` is #211's and `observed_generation` is #325's; the act-once markers are pacing keyed to an observed input (D-6). | latch audit at feature close |
| K5 | Devon | Registered reconcilers covered by the shared property | all registered (7, or 8 with the gateway) or ruled exempt | 0 | test inventory |
| K6 | Devon | Injected latch mutants caught by the property | all mutants DISTILL names (see US-05 AC4) | 0 | mutation check on the property |
| G1 (guardrail) | Platform | Converged-state evaluation cost | No regression against today's router relist plus `resync_schedule`: the pass is not above the baseline at any measured N (V-3 table). The 3% duty target is met at 100 workloads and not at 500 or 1000. Scope limitations tracked by #270 (scan cost) and #357 (edge-wake deploy latency during a pass, unchanged from today). Zero non-`Noop` actions at fixed point | n/a | metal measurement (V-3 evidence); sim cost counter; seeded sim |

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
| `workflow-lifecycle` wake | not a production defect (D6, D-8) |
| `service-lifecycle` `terminal_announced` and act-once stop markers | not changed (D5, D-6; ADR-0191 decision 5) |
| VIP release retried from allocator state; `released_for_deletion` replaced; the seeded reproduction of the possible VIP leak; same retry rule for dial-by-name frontend `F` | #211 (user ruling D-5, 2026-11-13); this feature does not wait on it |

## Wave: DISCUSS / [REF] DoR validation (9 items)

| # | Item | US-01 | US-05 | US-06 |
|---|---|---|---|---|
| 1 | Problem in domain language | PASS Ana, `payments` wedged | PASS Devon | PASS |
| 2 | Persona specific | PASS | PASS | PASS |
| 3 | 3+ domain examples, real data | PASS | PASS | n/a doc |
| 4 | UAT 3 to 7 | PASS 6 | PASS 6 | n/a |
| 5 | AC from UAT | PASS | PASS | PASS |
| 6 | Right-sized | PASS ~3d | PASS ~3d | PASS <0.5d |
| 7 | Technical notes | PASS ADR-0084 tension | PASS | PASS |
| 8 | Dependencies tracked | PASS none | PASS on slice 02 for green | PASS |
| 9 | Outcome KPIs numeric | PASS K1, G1 | PASS K5, K6 | PASS K4 |

Plus: `job_id` present on all stories (J-OPS-003 or J-PLAT-004, both existing). Elevator Pitch present on US-01 and US-05; US-06 is `@infrastructure` and shares a slice with US-05.

Honest gaps: US-05's entry point is a test run, which the Elevator Pitch rule disfavours; accepted because J-PLAT-004's persona works in the test lane. DoR is passed for DESIGN handoff.

DESIGN-owned: the LWW guards named in D10.

## Wave: DISCUSS / [REF] Wave decisions summary

- Risk: no DIVERGE artifacts; job grounding taken from existing jobs.
- Slices 01, 02 and 04; each independently mergeable and production-drivable. Slice 02 (deletion) is scheduled first in DELIVER.
- Walking skeleton: none new (brownfield).
- Back-propagation: `docs/product/jobs.yaml` changelog entry for J-PLAT-004 and J-OPS-003; no new job, no persona change.

## Wave: DESIGN / [REF] Input checklist

- ✓ DISCUSS sections above (D1 to D15 locked) and the slice files (scope as summarised in the story map)
- ✓ `.context/convergence-verification.md`; `.context/issue-351.md`, `.context/issue-344.md` (background only)
- ✓ `docs/product/architecture/brief.md` (structure and Application Architecture section located; appended an extension section per precedent), `c4-diagrams.md`
- ✓ ADR-0084 (read in full through the 2026-08-23 amendments), ADR-0086, ADR-0087 (via `reconcilers.md`), ADR-0079 (via `reconcilers.md`)
- ⊘ ADR-0021, 0023, 0035, 0036, 0048, 0061, 0064: not re-read in full; consulted through ADR-0084's text, `development.md` § "Reconciler I/O" and the code that implements them.
- ✓ Live code: `reconciler_runtime.rs` (tick body, `persist_view` Eq-skip), `lib.rs` (`build_cadence_table`, `spawn_convergence_loop`, router, `list_and_route`, routes), `hydration.rs`, `reconcilers/mod.rs` (trait), `eval_broker.rs`, `service_map_hydrator.rs`, `workload_lifecycle.rs` (release gate, `read_job`), `service_lifecycle.rs` (`terminal_announced`, act-once markers), `service_vip_view.rs`, `dataplane.rs` trait, `register_local_backend.rs`, `dataplane_update_service.rs`, `release_service_vip.rs`, `local_backend_map_handle.rs`, `handlers.rs`
- ✓ ADR-0191 (decisions 5 and 6, read for D-6).
- ⊘ Not read: `svid_lifecycle.rs`, `vm_reclamation.rs`, `gateway-svid-lifecycle` bodies; `overdrive-sim` invariants; `persistent_service_vip_allocator.rs`. Their managed-target sets below are stated from the verification doc and the DISCUSS notes and must be confirmed by the crafter against each `hydrate_*`.
- ⊘ ADR-0102, ADR-0273, ADR-0260: cited from their titles and from comments in code; not read.
- ✓ #211 is verified open by the user's ruling of 2026-11-13 (`gh issue view 211 --comments`, including the extension comment https://github.com/overdrive-sh/overdrive/issues/211#issuecomment-6866541631). ⊘ #344 is cited as the repository text cites it; not re-verified in this dispatch. #275 and #356 are cited as the user's ruling names them; not re-fetched in this dispatch.

## Wave: DESIGN / [REF] Finding that changes DISCUSS

The VIP-release story's premise does not hold (no production path deletes a workload spec); the work moved to GH #211 (D-5).

## Wave: DESIGN / [REF] Decisions (independent design review pending)

The ADRs stay `Proposed` and independent design review has not run. The Quint specification has been recorded.

### User approvals

| ID | Verdict | Conditions and changes |
|---|---|---|
| D-1 | APPROVED 2026-11-13 | ADR-0282 as proposed, plus: cite #275 (host-edge wakeup, open; a future host watcher submits through the same `submit_evaluation`, the pass is its level-triggered backstop); state that no reconciler can ask for a faster sweep than the global period. Includes the read-only accessor `pending_keys` on `BrokerDrain`. |
| D-2 | APPROVED 2026-11-13 | 30 s stays a compile-time constant; the limitation is tracked by #356 ("Make the reconciler resync period operator-configurable", created by the user). |
| D-3 | APPROVED 2026-11-13; KEPT 2026-11-15 | The hydrator's `next_evaluation_at` retry deadline stays. The user rejected a simplifier's proposal to remove it: without it a failed dataplane write is retried up to 30 s later (the next pass) instead of at its ~1 s backoff expiry. |
| D-5 | APPROVED 2026-11-13 | The `release_service_vip` executor doc fix stays (slice 04). Replacing `released_for_deletion` and the seeded reproduction of the possible VIP leak move to #211. This feature does not wait on #211. |
| D-7, D-8 | APPROVED 2026-11-13 | D-7: the slice 04 property drives the production tick over sim-composed ports; no new sim executor. D-8: `workflow-lifecycle` gets no dedicated wake; the uniform pass covers it at zero cost. Target confirmed 2026-11-15 (row below). |
| D-6 | option (a) stands; option (b) WITHDRAWN 2026-11-15 | Option (a) (approved 2026-11-13) is the design: `service-lifecycle` keeps `startup_last_acted_run` / `liveness_last_acted_run` as ADR-0191 decision 5 decides; no change to `service-lifecycle`. Option (b) (the 2026-11-15 re-ruling: emit from `actual`, `StopPacing` View inputs, reuse `backoff_for_attempt`, a `next_evaluation_at` pacing leg) and its pacing ruling are WITHDRAWN: they reversed ADR-0191 decision 5 (user ruling D-1 option (a), #295, 2026-10-13) and widened the terminal-overwrite race (#359). See "D-6". |
| D-1a-i | APPROVED 2026-11-15, option A | See "D-1a-i" under the interface contract. |
| D-8 (target) | CONFIRMED 2026-11-15 | `workflow-lifecycle`'s managed target is the single target `workflow/all`, returned only when a workflow instance exists (empty in production today). |
| D-11 (wording) | RULED 2026-11-15 | ADR-0284's Context states the actual fact about the existing route check (GH #358 records the colliding VIP); the `Policy` refusal stays for all VIP-range destinations. |
| Model check `PendingIsAdmitted` | DROPPED 2026-11-15 (user ruling) | Removed from the Formal protocol model and the validation budget; the modeler removes it from the spec. `edge_wake_during_burst_is_admitted_after_burst` stays. |
| D-9 | APPROVED 2026-11-15 | Delete the ADR-0053 same-host local-backend path (ADR-0283, which supersedes ADR-0053). It becomes slice 02, scheduled first in DELIVER. The user delegated the placement. Re-confirmed the same day for the whole path, including `cgroup_connect4_service`, `LOCAL_BACKEND_MAP`, ADR-0164's datagram clause and the owner's cgroup-placement check, after weighing the alternative of keeping the connect4/datagram pieces for #310. Reason: `ovd_fwd` forwarding between a connected host UDP socket and a vsock stream is independent of the connect4 rewrite; with `LOCAL_BACKEND_MAP` always empty the hook is a pass-through for every owner connect today; #310 designs its own destination selection. |
| D-10 | APPROVED 2026-11-15 | ADR-0164's datagram clause and its startup check are superseded (ADR-0284). ADR-0164's TCP-as-mesh decision (D24a) stays. Guest UDP to a VIP is unsupported until GH #310. |
| D-11 | APPROVED 2026-11-15 (option B) | A guest UDP datagram to a VIP-range destination is refused with the existing `FlowRefusal::Policy` until GH #310 (ADR-0284). No new refusal variant, port or wire surface. The user did not ask for a Tier-3 observable for it. |
| D-12 | APPROVED 2026-11-15 | The Quint modeler's gaps in the spec, pinned as interface contracts: see "Conformance pins (D-12)". Two items change surface: a per-reconciler resync step as a crate-private driving port, and an ordering clause on `pending_keys`. |
| Router wakes | RULED 2026-11-15 | The interest router's boot LIST and `Lagged` relist stay (already ADR-0282 decision 4). ADR-0282's alternatives table records the rejection of removing them. |
| V-3 | RUN 2026-11-15 (proven fact) | See V-3. This feature changes neither the scan cost (#270) nor the deploy latency (#357). |
| D-13 | APPROVED 2026-11-15 (option A) | See "D-13". |

Decision records: D-1, D-2 → ADR-0282; D-9 → ADR-0283; D-10, D-11 → ADR-0284; D-3, D-5 to D-8, D-12, D-13 → this delta.

## Wave: DESIGN / [REF] Domain model (DDD list)

| Term | Meaning |
|---|---|
| Managed target set | The targets for which one reconciler has desired or actual state, in its own target shape, enumerated from intent and observation. |
| Resync pass | One runtime sweep that submits an `Immediate` evaluation for every managed target of every registered reconciler. It is a level wake. |
| Edge wake | A submit caused by a change: an accepted observation row (`interests()`), `EnqueueEvaluation`, a client request, a self-requeue. |
| Evaluation outcome | `ok`, or an error ending the evaluation at hydrate, persist or dispatch. Retry rule: ADR-0282 decision 3. |
| Pacing input | A View field that decides only when to act again: attempts and last attempt time, or a marker keyed to an observed input (the probe row's `run_seq`). It never decides whether an effect is in place. |

No new aggregates, bounded contexts, events or read-ports.

## Wave: DESIGN / [REF] Component decomposition

| Component | Crate | Change | Owner of |
|---|---|---|---|
| `Reconciler::managed_targets` (new method) | `overdrive-core` | EXTEND trait | the reconciler's target set |
| `AnyReconciler::managed_targets` (forward arm) | `overdrive-reconcilers` | EXTEND | none |
| Eight reconciler impls of `managed_targets` | `overdrive-reconcilers` | EXTEND | their own target sets |
| Resync pass and loop | `overdrive-control-plane` (`reconciler_runtime`, `lib.rs`) | CREATE NEW, replaces cadence phase and router periodic relist | the level wake |
| Interest router | `overdrive-control-plane` | EXTEND by deletion: periodic arm and its two parameters go | edge wakes from rows |
| `ServiceLifecycle` | `overdrive-reconcilers` | EXTEND: `managed_targets` only; stop emission and the act-once markers unchanged (D-6) | the probe-driven stop decision |
| `ServiceMapHydrator` | `overdrive-reconcilers` | EXTEND: `managed_targets`, `next_evaluation_at` over the remote `retries` map; local partition and fingerprint DELETED (D-9) | the remote service-map effect |
| The same-host local-backend path (inventory in slice 02) | `overdrive-bpf`, `overdrive-dataplane`, `overdrive-core`, `overdrive-control-plane`, `overdrive-reconcilers`, `overdrive-sim` | DELETE | none (no production producer, ADR-0283). Also deletes, in `overdrive-control-plane` and `overdrive-sim`, the guest-flow owner's `OwnerCgroupPlacement` startup check and datagram-to-VIP handling (D-10, ADR-0284); inventory in slice 02 |
| Convergence property harness | `overdrive-control-plane` tests | CREATE NEW (test support, DISTILL shapes it) | none |

Dependency direction is unchanged. `overdrive-core` gains no I/O dependency.

## Wave: DESIGN / [REF] Driving and driven ports

Driving (pinned for conformance; D-12): `resync_reconciler` (one reconciler's step of the pass; the pass loops over it), `ReconcilerRuntime::submit_evaluation`, `BrokerDrain::drain_pending` (admission), `run_convergence_tick` (one evaluation, hydrate to dispatch), `spawn_interest_router` (edge). Driven: the existing intent, observation, view and dataplane ports are unchanged (the `Dataplane` port loses its two local-backend operations, D-9).

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
| `svid-lifecycle` | `workload/<id>` for the workload ids of `alloc_status` rows ∪ the workload ids of held SVIDs (`HeldSvidView`); see D-1a-i. A held SVID can exist with no `alloc_status` row when the `StartAllocation` dispatch holds it (`ensure_intercept_identity`, before `driver.start`) and the row write that follows fails; only an evaluation of that workload can emit its `DropSvid`. A held entry whose `spiffe_id` does not invert is not returned. |
| `service-map-hydrator` | `service/<id>` for each `service_id` in `service_backends` rows ∪ `service_hydration_results` rows |
| `workflow-lifecycle` | the single target `workflow/all` (the conventional target; its hydrate ignores the target and scans all instances under `IntentKey::workflow_instance_prefix`), returned only when at least one instance exists; so the set is empty in production (confirmed 2026-11-15) |
| `vm-reclamation` | its single node target `node/<id>`, always returned (host-backed: it hydrates from the host, not from intent or observation) |
| `gateway-svid-lifecycle` | its single node target `node/<id>`, always returned; registered only when `config.gateway` is `Some` |
| `noop-heartbeat` | empty set (no-obligation ruling: no desired or actual state) |

Reconcilers whose `hydrate_*` ignore the target (`vm-reclamation`, `workflow-lifecycle`, and `gateway-svid-lifecycle`'s node target) manage exactly one target, their conventional one. For them the containment postcondition above applies to that one target: if the projection `hydrate_*` returns is non-empty, the set contains that target. They do not enumerate per-resource targets, because the target carries no information their hydrate reads.

**Containment for `svid-lifecycle`.** Its `hydrate_actual` adds the global `ever_issued` audit set (every issued SPIFFE id) to the projection of every target (`svid_lifecycle.rs`, `hydrate_svid_actual_held`), so "non-empty projection" holds for any target and would make the postcondition vacuous. For this reconciler the projection that counts is `desired` (the Running allocations of the workload) plus the held `actual` entries; `ever_issued` makes no target managed. Containment is desired ∪ held, as the table states.

**D-1a-i (APPROVED 2026-11-15, option A): how the svid-lifecycle held leg derives the workload id.** `svid-lifecycle` hydrates per workload; `HeldSvidView::held_snapshot` is keyed by `AllocationId` and `HeldSvidFacts` carries `spiffe_id` and `not_after`, not the workload id, and `SpiffeId` has no workload-id accessor. The reconciler parses the SPIFFE path internally (the inverse of `SpiffeId::for_allocation`, which its hydrate already applies forward) to name the workload of a held SVID that has no `alloc_status` row. No new public surface; no `SpiffeId` accessor. Rejected: (B) a public `SpiffeId` accessor (new `overdrive-core` surface); (C) dropping the held leg (an in-memory hold orphaned by a failed row write would have no evaluation until the process restarts).

The Rust shared scan helper `scan_workload_intents` and the structural classifier (`rust.md` § "A key prefix and its record classifier are ONE SSOT") are the enumeration source; no hand-written prefix literal and no suffix skip list.

### D-1b. Resync driver (control-plane, crate-private)

```rust
/// One reconciler's step of the pass (D-12): enumerates its managed targets and
/// submits all of them `Immediate`, or on `Err` submits none. Returns the count submitted.
pub(crate) async fn resync_reconciler(
    state: &AppState,
    name: &ReconcilerName,
) -> Result<usize, HydrateError>;
```

The resync period is 30 s (D-2); its name and visibility are the crafter's. The pass and its loop are internal structure (one caller in `run_server`); the crafter shapes them. Their behaviour is the contract:

- A pass is exactly a loop over `resync_reconciler` for each registered reconciler in `BTreeMap` order; it adds no other effect. The convergence loop may admit evaluations between two reconcilers' steps; that interleaving is real, not a model artefact.
- On `Err` from `managed_targets` the pass logs a structured event naming reconciler and error and continues with the next reconciler. A failure never aborts the pass and never submits a partial set.
- A step submits one `Evaluation` per target through `ReconcilerRuntime::submit_evaluation` with `Immediate` eligibility. It writes no observation row.
- Contract shape: bounded-change. Its whole effect universe is `submit_evaluation` calls.
- The loop runs a pass at boot, then anchors the next pass at `previous + period` on the injected clock (re-armed at most once per period, no drift, inheriting ADR-0084's rule). The clock is the same instance the convergence loop reads.

Retry rule: ADR-0282 decision 3 (`run_convergence_tick_inner` is unchanged in control flow). The `development.md` § "Reconciler I/O" text (D15) and the proposed rule text cite that decision.

### D-1c. Removals (same commit as D-1a and D-1b)

`Reconciler::resync_schedule`, `ResyncSchedule`, `ResyncScope`, `AnyReconciler::resync_schedule`, `build_cadence_table`, the cadence phase of the convergence loop (its next-wake table and scope resolution), the `vm-reclamation` and `gateway-svid-lifecycle` overrides, `INTEREST_ROUTER_RELIST_PERIOD`, and the periodic arm of the router. `spawn_interest_router` reverts to:

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

### D-11. Guest UDP to a VIP-range destination (guest-flow owner, `overdrive-control-plane`)

No new type, variant, port method or wire field. Behaviour of the existing `datagram_associate` once the VIP branch is deleted:

- Precondition: the datagram's destination address lies in one of the configured VIP ranges (the `vip_range` the owner already receives from the ports; it stays).
- Postcondition: the association is refused with the existing `FlowRefusal::Policy`. The refusal precedes the route-type check, the marked-socket creation and `connect_udp`: no host socket is created or connected, and no datagram leaves the host.
- Edge cases: a destination outside the VIP ranges follows the generic path unchanged (the `gateway:53` mark exemption remains). A guest-prefix UDP destination keeps its existing refusal. TCP to a VIP is unchanged (D24a).
- Duration: until GH #310 delivers guest UDP to a VIP. When #310 lands it replaces this refusal.
- Contract shape: pure refusal (return-only); universe of effects = none (no socket, no route lookup).
- Premise and provenance (code, observation, kernel citations): ADR-0284 § Context.
- Kernel and VMM assumptions: none. The refusal needs no kernel behaviour; it removes a socket the kernel would otherwise route.
- Test obligation: Tier 1 only, through the owner's driving port with the sim flow I/O (slice 02 AC7).

### Conformance pins (D-12, APPROVED 2026-11-15)

Answers to the spec's gaps. Each pins an interface contract or states a model-only ruling; test-support shaping is DISTILL's.

**a. Dispatch error.** Settled by ADR-0282 decision 3 (the retry rule). The model follows the code: a tick that emitted an action requeues `Immediate` on every path including `dispatchErr`.

**b. `pending_keys` order.** `BrokerDrain::pending_keys(&self) -> Vec<(ReconcilerName, TargetResource)>` returns the pending keys in first-pending order: ascending by the broker's admission sequence, the order `drain_pending` admits eligible keys. A same-key resubmit does not change a key's position (below). The accessor is read-only, takes no limit or blocked set, and does not filter by eligibility.

**c. Pass granularity.** `resync_reconciler` (D-1b) is the driving port the conformance driver calls for the model's `PassStep`; the pass is only a loop over it in `BTreeMap` order, so it is covered by the same steps. Reason: enumeration failure is per reconciler, and the convergence loop (a separate task) admits evaluations between two reconcilers' steps, so a whole-pass port cannot replay the interleavings the model checks. Whole-pass replay alone would restrict randomized traces to inter-pass interleavings and leave `enumeration_failure_of_one_reconciler_does_not_starve_others` and `edge_wake_during_burst_is_admitted_after_burst` unreplayable.

**d. Admission, evaluation and outcome injection.**

| Model action | Driving port | Notes |
|---|---|---|
| `Drain` (admission) | `BrokerDrain::drain_pending(limit, blocked_targets, now, now_unix)` with `limit` = cap minus the driver's in-flight count and `blocked_targets` = the driver's in-flight targets | The driver's in-flight set is its stimulus (the cap and per-target exclusion live in the convergence loop, which is not a driving port); it is not projected. |
| `Finish(k, outcome)` | `run_convergence_tick` for the admitted evaluation | One call runs hydrate, reconcile, persist and dispatch; the outcome is chosen by the fault injected at the driven port before the call. |
| edge wake / pass submit | `ReconcilerRuntime::submit_evaluation` / `resync_reconciler` | A dropped edge wake is the driver not calling `submit_evaluation`. |
| crash | the driver drops the runtime and broker and rebuilds the runtime over the same stores | `bulk_load` restores the View, intent and observation survive (WK-5). |
| drift | a write to the intent or observation driven port | |

Outcome injection (faults at existing driven ports, Sim adapters; any seam that does not exist yet is DISTILL's test support, not DESIGN surface):

| Outcome | Injected at |
|---|---|
| `hydrateErr`, enumeration failure | a read failure on the `Sim` intent or observation store (the `ObservationRead` / `IntentRead` variants of `HydrateError`) |
| `persistErr` | `SimViewStore::inject_fsync_failure` (existing) |
| `dispatchErr` | a shim error from a driven port the dispatched action uses (for example a `SimDriver` stop failure or a CA issuance failure) |
| `dpFail` | `SimDataplane` configured to fail `update_service`; the executor writes the `Failed` `service_hydration_results` row (a result row, not a dispatch error) |

Snapshot timing: the spec reads the world at `Finish`, which maps to `run_convergence_tick`, where production hydrates. A trace that changes the world between `Drain` and `Finish` of one key reads the later world in both the model and production.

**e. Same-key resubmit keeps its slot.** Broker contract: `submit` for a key already pending replaces the stored evaluation, merges eligibility (`Immediate` wins; two `NotBefore` keep the earlier), increments `cancelled`, and leaves the key's admission sequence unchanged. The key does not move to the back of the queue (`eval_broker.rs` `submit`: the `fifo` is assigned only for a fresh key). A key admitted by `drain_pending` and submitted again takes a new, later slot.

**f. `need` (desired != actual) and `has` (managed).** Model-only for projection. They are the driver's stimulus (the world it builds through the intent and observation driven ports) and have no accessor and no projection. The conformance comparison covers the broker state (`pending_keys`, `counters`); whether a needy key converges is the slice 04 oracle (`actual == desired` through the reconciler's read-port), not a conformance projection. The driver builds worlds in which `need` implies `has`, as the model's `NeedImpliesManaged` invariant states.

Consequence for `has`: if the driver set `has` equal to what `managed_targets` returns, conformance could not detect an omitted target (the model and production would agree on the omission). Omission is instead covered by a containment obligation DISTILL records for every reconciler: for each world the driver builds, every target whose `hydrate_desired` or `hydrate_actual` projection is non-empty is in the set `managed_targets` returns, with both sides read through the reconciler's own methods and not through the driver's bookkeeping (for `svid-lifecycle`, "projection" excludes `ever_issued`, per the containment note in D-1a). This is the D-1a postcondition made checkable; it adds no production surface.

**g. `inWin` (inside the hydrator backoff window).** Model-only; the conformance `State` does not include it. Reason: the window is `last_failure_seen_at + backoff` over the hydrator's persisted `retries`; a driver that recomputed it from `get_view` plus the backoff function would compare the model with a re-derivation of the same rule, and pinning a new window accessor would add production surface for one check. What retention (D-3) leaves observable is already pinned: after a `dpFail` finish the key is still in `pending_keys` with a `NotBefore` that the broker holds, so the retained key is compared as `pending`. The window arithmetic itself is covered by the unit tests of the hydrator's `next_evaluation_at` (D-3). The Quint invariant `RetentionSchedulesBackedOffKey` and its hazard `wake_noRetention` stay as model checks.

## Wave: DESIGN / [REF] Reuse Analysis

| Existing | Verdict | Justification and contract shape |
|---|---|---|
| `Reconciler` trait, `AnyReconciler` | EXTEND | One required method; the `hydrate_*` precedent. |
| `ServiceLifecycleView` act-once markers and the stop emission | UNCHANGED | D-6 (ADR-0191 decision 5). Contract shape: bounded-change; universe = the markers and `StopAllocation` actions of Running allocations; assertion = the existing `probe-count` model checks and unit tests, and the slice 04 property for every other obligation. The View decides WHEN only. |
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
| `BrokerDrain::pending_keys` accessor | CREATE NEW (read-only) | Pinned so the conformance driver has a projection, in first-pending (admission) order; see Conformance pins. Contract shape: pure read; universe = none. |
| `resync_reconciler` | CREATE NEW (crate-private; D-12 approved 2026-11-15) | The per-reconciler step the pass loops over; the conformance driving port. Contract shape: bounded-change; universe = `submit_evaluation` calls for one reconciler. |
| Guest-flow owner `datagram_associate` | EXTEND by deletion plus one refusal (D-11) | The VIP branch goes; a VIP-range destination is refused `FlowRefusal::Policy`. Contract shape: pure refusal; universe = none. Assertion: Tier 1 owner driving port with sim flow I/O. |

## Wave: DESIGN / [REF] Slice 04 and the remaining decisions

### D-5. VIP release (approved 2026-11-13)

Do now: correct the `release_service_vip` executor doc (it says the error is absorbed; it returns `ShimError::AllocatorRelease`). Moved to GH #211 ("Implement workload deletion (intent withdrawal) + service/dataplane teardown", open): replacing `released_for_deletion` and the seeded reproduction of the possible VIP leak. #211 was extended by comment https://github.com/overdrive-sh/overdrive/issues/211#issuecomment-6866541631 to require: release decided from allocator state via `service_vip_view`, retried until the address is free, `released_for_deletion` deleted, designed together with the post-withdrawal digest source, and the same retry rule for the dial-by-name frontend `F`. This feature does NOT wait on #211. The rule text is timeless and names no violation; `released_for_deletion` (#211) and `observed_generation` (#325) are violations tracked by their issues.

Alternative: replace `released_for_deletion` now against a hand-built deletion fixture. It loses on D11 (a test installs a state production cannot produce), on `design.md` § "Ground the premise", and because the digest source is undecided.

### D-6. The probe-driven stop markers stay (ADR-0191 decision 5; no change)

`service-lifecycle` keeps `startup_last_acted_run` and `liveness_last_acted_run` (`service_lifecycle.rs`, View fields). A probe-driven `StopAllocation` is emitted at most once per crossed probe row, and a failed stop is retried on the next crossed row. This is ADR-0191 decision 5 (user ruling D-1 option (a), #295, 2026-10-13), defended by the `specs/quint/probe-count/` hazards `streak_bugOncePerStreak` (a failed stop is never retried while failures continue) and `streak_bugOnceEver` (never retried at all). "No new row changes nothing" stays as ADR-0191 decision 6 decided; the not-sampled event (ADR-0192) makes a run that writes no row visible.

**Why the markers exist, and why that reason holds.** Without them a re-read of the same crossed row would emit the stop again on every reconcile pass; the runtime persists the View before dispatch, so the marker is the reconciler's memory of the row it acted on. Each marker is keyed to an observed input, the probe row's `run_seq`, and is written only when a stop is emitted.

**Why they pass the View rule.** The rule (see "Proposed rule text") lets a View hold pacing inputs, including a marker keyed to an observed input, and forbids a View field from deciding whether an effect is in place. The stop decision reads `actual`: the allocation is `Running` and the latest probe row is past its threshold. The marker decides only when the stop is emitted again (on a newer row). If the View says "acted at run N" and the allocation is still `Running` on a newer crossed row, `actual` wins and the stop is emitted.

**`terminal_announced`.** It is stamped when a terminal row is observed (`:548-550`) and when a startup stop is emitted (`:654`); `stable_announced` is gone. It is consulted to (1) skip re-evaluation of a non-Running allocation (`:563`), (2) retain the evaluation until the startup window ends (`next_evaluation_at`, `:710`), and (3) exclude the allocation from `collect_liveness_actions` (`:1029`) after its startup stop was emitted. ADR-0191 decision 5 states the startup failure "is already announced once per allocation" (`:119-121`); this field is that announcement. It is pacing keyed to observed lifecycle: (1) concerns an allocation whose stop is already in place, (2) is a schedule, and for (3) the effect (the stop) is still decided by `actual` through the startup marker, which re-emits on the next crossed startup row; liveness supervision begins only at Stable (`start_liveness` at the Stable hook, `liveness_begins_at_stable.rs`), and an allocation that received a startup stop never committed Stable, so there is no liveness row for the exclusion to suppress. The field does not decide whether an effect is in place, and the rule does not condemn it. The one dependency is that liveness never runs before Stable.

**Withdrawn design.** The 2026-11-15 re-ruling (option (b)) designed a `StopPacing` View, a stop emitted on every evaluation from `actual`, reuse of `backoff_for_attempt`, a `next_evaluation_at` pacing leg, deletion of the two markers and their tests, and a quiescent-probe-runner expectation. It existed to close the loss case in which no newer probe row arrives after a failed stop. That case is the one ADR-0191 decision 6 accepts ("no new row changes nothing"); the design reversed decision 5 and widened the terminal-overwrite race (#359). It is withdrawn (approvals table).

### D-7. Slice 04 property shape

- The property drives the production `run_convergence_tick` over a sim-composed `AppState` (the pattern `workload_gc_absent_intent.rs` uses): hydrate, reconcile, View persist and the real action shim run against `Sim*` ports. No generic sim executor is written. A hand-written executor re-implements the shim and is the "driver compares the model with itself" shape that `testing.md` rejects; the hydrator DST's fabricated `Completed` row is that shape today.
- Faults ("dropped actions") are injected through existing or small driven-port seams: service-map update failure, allocator release failure, `SimViewStore::inject_fsync_failure`, observation read failure, `SimDriver` stop failure. That is `testing.md`'s "drive causes, not fabricated consequences". Starting worlds and Views are arbitrary (including a View that says done while the effect is absent); crash-partial worlds are reachable states.
- Oracle: `actual == desired` through the reconciler's own read-port within N passes, then zero non-`Noop` actions (progress, stability); executor idempotency by dispatching each action twice.
- Production surface the property requires: the resync step between ticks (`resync_reconciler`, D-12) and, for the seams, sim-side injectors DISTILL names. Fixture internals are DISTILL's.
- Lane: DISCUSS AC2 says default lane. A composed `AppState` needs an intent store; `lib.rs:522` builds redb with `InMemoryBackend`, which may make a no-file default-lane build possible. If not, the property sits under `integration-tests` and AC2 changes. DISTILL decides (validation item V-5; not contract-shaping).
- Reconcilers.md rule change: see "Proposed rule text".

### D-8. `workflow-lifecycle`

Confirmed: no dedicated wake. It is registered, so it gets a required `managed_targets`; the production set is empty because the registry is empty. The method closes the wake for the day the first production workflow lands as a side effect, not as scope.

## Wave: DESIGN / [REF] Proposed rule text (drafted here; lands in DELIVER)

Not applied now. Timeless discipline only, per the memory ruling on rules docs.

### `reconcilers.md` — replaces the fingerprint-shaped symptom and the "never both" wakeup paragraph

> **A View decides when to act, never whether an effect is in place.** A View field may carry pacing inputs: attempts, last attempt time, a retry budget, a rate limit, or a marker keyed to an observed input (for example the `run_seq` of the probe row a stop was last emitted for) that decides only when to act again. A View field never decides whether an effect is in place. That is decided from `actual`, read from the resource the reconciler manages. If the View says done and `actual` says not done, `actual` wins. If the effect cannot be observed, make it observable first; a result row of the last attempt is a receipt, not the resource.
>
> **Every reconciler declares its managed targets.** The runtime re-evaluates all of them every period, from intent and observation, whatever woke them before. `interests()` and `EnqueueEvaluation` are latency; the pass guarantees re-evaluation, not that a View-gated action is re-emitted. An evaluation that fails to hydrate or persist its View is not requeued locally; the next pass retries it. A dispatch error keeps the tick's requeue (decided before dispatch: immediate after an emitted action, else `next_evaluation_at`), and the pass is its backstop. `next_evaluation_at` supplies a precise retry deadline when the reconciler has one.

Symptoms to list: a View field consulted as the diff; a field stamped on emit and consulted to decide whether the effect is in place; a reconciler whose only wake is an edge; a managed-target method that enumerates one observation family only; a test or simulation invariant that seeds the View to represent "converged" instead of asserting on `actual`.

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

**Required** (changes an ordered, concurrent and crash-sensitive protocol). Written and model-checked by the `quint-modeler`. Directory: `specs/quint/reconciler-evaluation-wake/` (permanent, referenced by DISTILL conformance; its `README.md` states the abstraction). Evidence is recorded only by `cargo xtask quint check --record`. The recorded run (`specs/quint/reconciler-evaluation-wake/evidence/summary.md`) shows 15 of 15 checks matching `expect` (3 properties: `Safety`, `ProgressByPass`, `RetentionSchedulesBackedOffKey`; 2 hazards; 10 named runs; Quint 0.32.0, Apalache 0.56.1, kernel 7.0.0-38-generic), and `cargo xtask quint verify-evidence` passes. `PendingIsAdmitted` (dropped by the 2026-11-15 ruling) is not in the recorded run.

The specification models evaluation wake (decisions D-1 and D-3), module `evaluation_wake`. Owners: broker (pending FIFO of unique `(reconciler, target)` keys, eligibility `Immediate` or `NotBefore`), convergence loop (cap 8, one in-flight per target), resync pass (stepped one reconciler at a time, D-12), edge wakes, tick outcomes (`ok`, `dpFail`, `hydrateErr`, `persistErr`, `dispatchErr`). World per key: `has` (managed) and `need` (desired != actual). Environment faults as actions: each error outcome, enumeration failure for one reconciler, crash between View fsync and dispatch, dropped edge wake, drift.

- Safety (`Safety` invariant, `wake_full`): active within cap; one active per target; pending keys unique; eligibility canonical; need implies managed; backoff window only for needy keys.
- Progress (`ProgressByPass`, under fairness of the pass timer): every needy key is evaluated again after any error outcome, dropped wake or crash, and an `ok` evaluation clears its need. The bound: progress holds once faults stop (faults are budgeted, `MAXF = 2`; drift, `MAXD = 2`) and an admitted evaluation's dispatch completes. The model abstracts the reconciler: it proves re-evaluation, not that `reconcile` converges `actual` to `desired` (the slice 04 property and each reconciler's tests do). Starvation of a key queued behind a pass burst is covered by the named run `edge_wake_during_burst_is_admitted_after_burst` and the broker's FIFO unit tests (V-4); the `PendingIsAdmitted` check was dropped by the user's ruling of 2026-11-15.
- Retention (`RetentionSchedulesBackedOffKey`, `wake_retention`): a key inside its hydrator backoff window is always scheduled, so its retry needs neither a row write nor a pass (D-3).
- Hazard variants, one per decision, 2 in total: D-1 → `wake_noPass` (remove the pass; dropped wakes disabled; progress is violated, the wedge ADR-0282 names); D-3 → `wake_noRetention` (`next_evaluation_at` returns `None`; retention is violated).
- Broker rules (pending uniqueness, one active per target, cap, canonical eligibility) are existing behaviour, not decisions of this feature, so they carry no hazard; their teeth are the `eval_broker` unit tests (V-4).

Named runs (all on `wake_conf`, each a `[[check]]` in `checks.toml`), one per critical scenario; these are the runs DISTILL pins with `#[quint_test]`:

| Critical scenario | Named run |
|---|---|
| happy path: an edge wake converges | `healthy_edge_converges` |
| slice 01 fault 1: hydrate error | `hydrate_error_recovers_by_pass` |
| slice 01 fault 2: View persist error | `persist_error_recovers_by_pass` |
| crash between View fsync and dispatch | `crash_between_fsync_and_dispatch_recovers_by_boot_pass` |
| slice 01 fault 3: failed dataplane inside the backoff window (D-3) | `failed_dataplane_inside_backoff_recovers_without_row_write` |
| fresh deploy: intent, no row, no edge wake | `fresh_deploy_without_row_is_evaluated` |
| enumeration failure of one reconciler | `enumeration_failure_of_one_reconciler_does_not_starve_others` |
| edge wake during a pass burst (also per-target exclusion) | `edge_wake_during_burst_is_admitted_after_burst` |
| V-4: a pass submit merges a retained `NotBefore` to `Immediate` | `pass_submit_merges_retained_not_before_to_immediate` |
| the world changes between a key's `Drain` and its `Finish`: `Finish` acts on the world read at `Finish`, as production hydrates at the start of `run_convergence_tick` (approved 2026-11-15; holds, recorded) | `world_changes_between_drain_and_finish` |

Assumptions (the `ASSUMPTIONS` block of `evaluation_wake.qnt`) and what discharges each:

| Id | Assumption | Discharged by |
|---|---|---|
| WK-1 | Broker rules: same-key collapse keeping its slot, `Immediate` merge, oldest-first admission | V-4: `eval_broker` unit tests and the runs above |
| WK-2 | `submit_evaluation` wakes the convergence loop | ADR-0273 acceptance tests |
| WK-3 | Abstract time: the 30 s period, anchored re-arming and "one period plus queue time" as a duration | V-3: seeded sim on `SimClock` |
| WK-4 | `managed_targets` returns the whole set or `Err`, from intent and observation | The per-reconciler containment obligation (Conformance pins, f): every target with a non-empty hydrate projection is in the returned set. The conformance driver cannot discharge it, because it sets `has` from the same set |
| WK-5 | A crash loses exactly the in-memory broker and in-flight set; View, intent and observation survive | the `WriteThroughOrdering` DST invariant and a DST crash scenario |
| WK-6 | Another reconciler's View pacing never blocks a due re-evaluation | the slice 04 property |

The model checks the pass and edge-wake protocol, not the duration bound (WK-3).

Pinned observable projections for the conformance drivers (`testing.md` § "Quint conformance"):

| Spec variable | Owner | Accessor | Status |
|---|---|---|---|
| `pending` (keys; order = FIFO) | `BrokerDrain` | `pending_keys(&self) -> Vec<(ReconcilerName, TargetResource)>`, first-pending (admission) order (D-12 b) | NEW, read-only (approved with D-1, 2026-11-13; ordering clause is D-12, approved 2026-11-15) |
| `dispatched`, `cancelled` | `BrokerDrain` | `counters()` | existing (`wake_conf` only) |
| `inWin` (inside the hydrator backoff window) | none | model-only (D-12 g): the retained key shows as `pending`; the window arithmetic is covered by the D-3 hook's unit tests | ruling |
| `elig` | broker | model-only | ruling: verified by broker unit tests |
| `active` | convergence loop | model-only (not projected) | ruling: the loop's `active_targets` is local to the task; not exposed. The driver's in-flight set is its stimulus (D-12 d) |
| `has`, `need` | the driver's world | stimulus, not projected (D-12 f); containment of `has` in `managed_targets` is a DISTILL obligation, not a conformance comparison | no accessor |
| `due`, `passPos`, `faults`, `drifts` | none | model-only (timer and budgets) | ruling |

Action mapping (D-12 c, d): `PassStep` → `resync_reconciler`; edge wake → `ReconcilerRuntime::submit_evaluation`; `Drain` → `BrokerDrain::drain_pending`; `Finish(k, outcome)` → `run_convergence_tick` with the outcome's fault injected at its driven port; crash → rebuild the runtime over the same stores; `Tick` is the period timer and is model-only (the driver calls `resync_reconciler` directly). `Sim*` adapters inject faults at driven ports and are the arbiter for no variable. The step record `conf_step` (variant per action name) is the spec's, not production's.

## Wave: DESIGN / [REF] Validation items

Contract-shaping items run before approval; I have not run them and mark none proven.

| ID | Item | Class | Probe to run | Falsification changes |
|---|---|---|---|---|
| V-3 | Cost: per-evaluation hydrate time and admission delay of an edge wake queued behind a burst at 1000 workloads | not contract-shaping (pacing is internal) | RUN 2026-11-15. Native metal, AMD EPYC 8024P (16 threads), Linux 7.0.0-38, commit `97a417fcb`; real runtime, broker, reconcilers and handler over `LocalIntentStore` / `LocalObservationStore` / `RedbViewStore`, Sim adapters only at driven ports; 80% Job / 20% Service, converged. Evidence: `spike-scratch/reconciler-resync-cost/evidence/full-20261115T180650Z/` (`results.json`, the proposed pass) and `baseline-20261115T211313Z/` (`results.json`, the baseline; `analysis.txt` exists only here and tabulates the baseline, the same-run proposed pass and the prior-run proposed pass). | Held as a proven fact; no contract changes. Measured (proposed pass, prior run of `analysis.txt`; baseline in brackets): W=10: E 49 [67], 10 ms wall [13]; W=100: E 265 [337], 86 ms [94], 0.29% duty [0.31%]; W=500: E 1225 [1537], 1256 ms [1352], 4.19% [4.51%]; W=1000: E 2425 [3037], 4554 ms [4636], 15.2% [15.5%] (~5 s CPU per 30 s). The pass is not above the baseline (today's router periodic relist plus `resync_schedule`, E = 3W+1) at any N; it submits fewer evaluations (no `service-lifecycle` on Job targets) and adds `service-map-hydrator` per service and intent-only workloads. Cost grows faster than linearly in E because `workload-`, `svid-` and `service-lifecycle` scan the whole `alloc_status` table per evaluation, and `vm-reclamation` per row. Edge-wake (deploy) admission delay during a burst ~ burst wall (4.3 to 4.6 s at 1000), identical for baseline and pass; idle at most 2 ms. Causes: FIFO admission with no priority lane (ADR-0102 :71), the VIP allocator and `listener_facts` mutexes held across both hydrates (`reconciler_runtime.rs:1586-1587`) serialising the 8 slots, and the full-table scans. Tracked outside this feature: scan cost by #270, deploy latency by #357. |
| V-4 | Same-key collapse and `NotBefore`→`Immediate` merge under pass plus edge submits | not contract-shaping | existing `eval_broker` tests plus the new run `edge_wake_during_burst_is_admitted_after_burst` | a test |
| V-5 | Default-lane feasibility of the composed property (in-memory redb) | not contract-shaping | DISTILL builds one composed tick in the default lane | the lane of one test |

No contract-shaping validation item is open; D-1 stands on source reading. The deletion (D-9) adds no kernel assumption.

### Validation budget

- Model checks: 15 (5 exhaustive TLC on `wake_full` / `wake_retention` / hazards, 10 evaluator runs), recorded in `specs/quint/reconciler-evaluation-wake/evidence/summary.md` (Linux 7.0.0-38, 16 jobs): 65.3 s wall for the whole run in parallel; 160.7 s of check time in total (TLC 133.5 s, evaluator runs 27.2 s). Per check: `ProgressByPass` 63.9 s (the longest), `Safety` 25.9 s, retention 18.3 s, each hazard 12.7 s, each evaluator run 2.3 to 3.1 s.
- CI lane (`ci = true` in `checks.toml`): 14 of 15, which excludes the one slow temporal check (`ProgressByPass`); 96.8 s of check time (TLC 69.6 s, runs 27.2 s). Growth beyond these numbers needs user approval.
- Test lanes added by the design: none beyond the existing default lane (slice 04's property: its lane is V-5), one Tier-1 acceptance scenario for D-11, and one Quint conformance driver (one specification) under `integration-tests` per `testing.md`. No new CI job, tool, or xtask subcommand.
- Runtime budget: one evaluation per managed target per 30 s; measured per-pass cost is in V-3, the guardrail in G1.

## Wave: DESIGN / [REF] Quality attributes and enforcement

- Reliability: every wake gap in the verification closes by the pass re-evaluating the target; recovery bound = period plus queue time. The pass guarantees re-evaluation, not convergence: the model proves the former (an `ok` evaluation clears need, by abstraction); that `reconcile` converges is the slice 04 property. The probe-driven stop keeps ADR-0191's act-once-per-crossing rule and is not re-emitted without a new row (D-6).
- Maintainability: one backstop instead of three; a required method makes omission a compile error.
- Testability: managed-target sets and the diff are pure or sim-drivable; the composed property reuses production code.
- Enforcement: the existing dst-lint purity scan over `overdrive-reconcilers/src/**` covers the new method (no clock, no wall time); the compile-guard that `hydrate_*` carry no `&dyn Clock` extends to `managed_targets`; `cargo xtask conformance-lint` covers the conformance driver; the boundary test (S-ROH-B-09) is updated for the field set. No new tool.
- No external integrations; no contract-test annotation.
- Paradigm: object-oriented per `CLAUDE.md`.

## Wave: DESIGN / [REF] Changed Assumptions

| Superseded contract (quoted) | New proposal |
|---|---|
| ADR-0084 amendment 2026-08-23: "The level-triggered backstop for the row-backed partition ... is the interest-router's unconditional periodic relist — NOT a per-reconciler `resync_schedule`" | The backstop for every reconciler is the runtime resync pass over `managed_targets`. |
| `reconcilers.md`: "A reconciler declares one model or the other, never both" | Every reconciler is resynced; `interests()` is edge-only. |
| ADR-0053 (Accepted 2026-05-23): same-host backend delivery via `cgroup_sock_addr` | Superseded by ADR-0283 (D-9): the path has no production producer and is removed. |
| ADR-0164: "Datagrams to a VIP use `connect4`. The owner always connects its host datagram socket, so the existing ADR-0053 `connect4` rewrite resolves a VIP to a platform-chosen backend." and "The owner refuses to start unless its own cgroup is the `connect4` attach path or below it." | Superseded by ADR-0284 (D-10, D-11). Guest UDP to a VIP is refused until GH #310. ADR-0164's TCP decision (D24a) is unchanged. |
| `development.md`: "an unreadable observation is the runtime's to log-and-skip" | A hydrate error ends the evaluation; the next pass retries (D15). |

## Wave: DESIGN / [REF] D-13. The persisted `ConflictRoute` tag after the cgroup route is deleted (APPROVED 2026-11-15, option A)

**Ruling (2026-11-15).** Option A: remove `ConflictRoute::Cgroup` and `WriteRoute::Cgroup` in place; `ConflictRoute` stays a one-variant enum, both route fields stay in the row, no new envelope version. The user's reason: greenfield project, single-cut migrations, no compatibility versions.

- **Context.** The `reconcile_conflict` observation row (`ReconcileConflictRowV1`, rkyv, redb) carries `first_route` and `second_route` of type `ConflictRoute { Xdp, Cgroup }` (`overdrive-core/src/traits/observation_store.rs`). A `Cgroup` route exists only for `RegisterLocalBackend` / `DeregisterLocalBackend`, which slice 02 deletes, so no production store holds a row with that tag.
- **Check.** The V1 golden fixture (`tests/schema_evolution/reconcile_conflict_row.rs`) uses `Xdp` (discriminant 0). It must still decode, unedited (slice 02 AC5). If it fails, the crafter stops and surfaces it to the user. A new envelope version is never minted silently.

## Wave: DESIGN / [REF] Scope limitations and pending items

- Scope limitation, tracked: the 30 s resync period is a compile-time constant with no configuration surface (D-2). Tracked by #356.
- Scope limitation, tracked: guest UDP to a VIP is refused until GH #310 (D-10, D-11; ADR-0284). #310 is cited as the user's ruling names it; not re-fetched. A comment on #310 records the removal.
- Lane for the slice 04 property (V-5).
- Independent design review has not run; the ADRs stay `Proposed`.
- DELIVER follow-ups outside `crates/` (rules docs, living docs, website): slice 02 § "DELIVER follow-ups outside `crates/`" only.

## Wave: DESIGN / [REF] Handoff to DISTILL (after the Quint dispatch and independent design review)

User approvals are recorded above. DISTILL starts when independent design review is done.

Scenario obligations: the slice 01 invariants (three faults plus fresh deploy, enumeration failure, burst admission); slice 02 (the deletion: AC1 to AC7 of the slice file, unchanged behaviour of the remote service path and the gateway; plus the one added behaviour, D-11, Tier 1 only, through the owner's driving port with the sim flow I/O); slice 04 (the composed property, mutation check per injected latch mutant, and the executor doc fix, which has no scenario; `service-lifecycle` is covered by the property like every reconciler, with its act-once markers unchanged, D-6). The Quint conformance driver per the pinned projections. The per-reconciler containment obligation for `managed_targets` (Conformance pins, f). Each slice stays production-drivable through `serve` + `deploy` with a real reachable fault (D11). The VIP-release scenarios belong to #211 and are not in this feature.

Proposals for the user (not adopted; each adds scope, per `design.md` § "No unrequested scope"): none.
