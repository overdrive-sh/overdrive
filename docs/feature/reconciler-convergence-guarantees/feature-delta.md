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
- ✓ Live code spot-checked: `reconciler_runtime.rs:1584-1744` (hydrate `?` at 1588-1589 and view persist `?` at 1638-1641 both return before the requeue at 1732; dispatch errors do requeue), `service_map_hydrator.rs:456-488` (fingerprint stored on emit), `workload_lifecycle.rs` (`released_for_deletion` stamped at :247, gate :1531).
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
| D5 | The service-lifecycle `stable_announced`/`terminal_announced` latch claim is refuted (`stable_announced` is deleted). None of the `service-lifecycle` View markers is in scope; see D-6. | Verification finding 3; user ruling D-6 |
| D6 | `workflow-lifecycle` being un-woken is NOT a production defect: `serve` builds an empty `WorkflowRegistry` and nothing writes `workflows/` intent in production. Not scope. | Verification finding 5, `development.md` § "Ground the premise" |
| D9 | The View may decide WHEN to act (backoff, budgets, rate limits, a marker keyed to an observed input). It never decides WHETHER the effect is in place; only `actual` decides that. | Issue rule text; user chose the rule change |
| D10 | Every new `actual` read introduced by P2 carries a DESIGN note naming the guard that makes it safe under LWW convergence (cross-reference #344). | User |
| D11 | Each slice is production-drivable: the behaviour runs in the real `run_server` composition reached by `overdrive serve` + `overdrive deploy`, with faults injected at existing `Sim*` port seams (allowed by CLAUDE.md). A test that hand-installs what production does not is not acceptance. | CLAUDE.md § "Build vertical slices" |
| D13 | The new View rule carries no exception. `ServiceLifecycleView::last_emitted_backend_fingerprint`, which the 2026-11-13 ruling would have named as the single exception, no longer exists: ADR-0101 deleted it (no match in `crates/`). | Verified in code, 2026-11-15 |
| D15 | The `development.md` "log-and-skip" text is corrected and `next_evaluation_at` is documented in the rules, in the same commit as slice 01's behaviour change. | User, 2026-11-13 |
| D12 | One feature with three slices (01, 02, 04). Periodic re-evaluation (slice 01) is not split into its own feature; the slices ship independently. | User, 2026-11-13 |

## Wave: DISCUSS / [REF] Ground the premise (per production path)

| Defect | Reachable in a real deploy? | Production path |
|---|---|---|
| Hydrator local-backend latch | **No; deleted by D-9.** | The hydrator emits `RegisterLocalBackend` only for a backend whose address equals `host_ipv4` (`service_map_hydrator.rs:356-370`). Since ADR-0276 every Service backend address is the Running allocation's `workload_addr`, taken from the guest network plan (`action_shim/mod.rs:1991`, `:2459`) and allocated from `GUEST_NODE_PREFIX` 100.95.0.0/16 (`guest_network.rs:561`, `guest_transport/mod.rs:594`), never `host_ipv4`. Non-VM provisioning was deleted (`fc3bc7d04`). The local set is always empty in a real deploy. |
| Hydrator un-woken after `Failed` result | Yes | `DataplaneUpdateService` `Err` writes a `Failed` row; next tick inside backoff yields `Noop` with no `next_evaluation_at`; a steady service gets no further backend-row write to wake it. |
| Hydrate error skips requeue | Yes, rare (store read errors only) | A store read error (`ObservationRead` / `IntentRead`); early `?` at `rt:1588-1589`. `RunningAllocationWithoutWorkloadAddr` (`service_lifecycle.rs:940`) is NOT production-reachable: the shim writes `workload_addr` on every `Running` write from `spec.transport` (`action_shim/mod.rs:1991-1992`, `:2457-2461`), and every production allocation carries a guest transport assignment (ADR-0276; non-VM provisioning deleted, `fc3bc7d04`). It arises only when a test seeds a `Running` row without an address, so it is a Tier 1 stimulus and not a `serve` + `deploy` fault (`development.md` § "Ground the premise"). |
| View write error skips dispatch and requeue | Yes, rare | `persist_view` redb write failure, early `?` at `rt:1638-1641`. |
| Fresh deploy with no `alloc_status` row never relisted | Yes, rare | First `workload-lifecycle` hydrate fails; `handlers.rs:521` submitted once; relist routes only `alloc_status` rows; no boot seeding of existing intents. |
| `workflow-lifecycle` un-woken | No | Empty registry in `serve`. |

## Wave: DISCUSS / [REF] Scope Assessment

Result: **OVERSIZED SIGNALS PRESENT; split considered and rejected by the user (D12). One feature, three independently shippable slices (01, 02, 04).**

- Stories: 3 (US-01, US-05, US-06). Under the 10-story signal.
- Bounded contexts touched: 4 (control-plane runtime, reconcilers crate, dataplane/allocator adapters, sim + core test surface). Over the 3-context signal. (signal 1)
- Independent outcomes that could ship separately: yes, P1 is independent of P2/P3. (signal 2)
- Effort: about 8 days across 3 slices, under the 2-week signal.
- Two signals hit. The feature stays as written until the user rules. Slice 01 is isolated so it can move out without rework.

## Wave: DISCUSS / [REF] Walking skeleton strategy

Brownfield. The skeleton is the existing `overdrive serve` + `overdrive deploy` convergence path; this feature adds no new end-to-end path. Each slice is a thin vertical cut through that path: a fault is injected at an existing driven port (store read, view write, dataplane insert, allocator release), production authors the resulting observation, and the oracle is observable output (`overdrive workload describe`, service map readback, allocator state). No separate walking skeleton is built.

## Wave: DISCUSS / [REF] Driving ports and pre-requisites

Driving ports: `overdrive serve` (`run_server` composition root, convergence loop); `overdrive deploy <SPEC>`; `overdrive workload describe`; `overdrive job stop`. Workload deletion (#211) is not a driving port of this feature: the VIP-release story moved to #211 (D-5). Fault injection goes through existing driven ports only (`IntentStore`, `ObservationStore`, ViewStore, `Dataplane`, VIP allocator), as `Sim*` adapters at Tier 1. All three slice 01 faults (hydrate store-read error, `persist_view` failure, `Failed` dataplane result) are injected at Tier 1 through the existing `Sim*` port seams (`SimViewStore::inject_fsync_failure` and the equivalents for `ObservationStore` / `IntentStore` reads and `SimDataplane`). The pass itself runs in the real `run_server` composition (the production convergence loop, `managed_targets` and broker), so production-drivability holds; only the fault is injected at a port boundary, which CLAUDE.md allows. There is no Tier-3 stimulus for slice 01 (user ruling 2026-11-16).

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
- [ ] AC1: Fault = hydrate error (`ObservationRead`, `IntentRead`; `RunningAllocationWithoutWorkloadAddr` is a Tier 1 stimulus only, not production-reachable); owner path = `run_server` convergence loop + `workload-lifecycle` and `service-lifecycle`; oracle = `workload describe` shows Running and the service map is programmed within one re-evaluation period after the fault clears, with no second `deploy`.
- [ ] AC2: Fault = `persist_view` error (Tier 1, `SimViewStore::inject_fsync_failure`); oracle = same, and no action is dispatched on the failing tick (View is not claimed).
- [ ] AC3: Fault = `Failed` dataplane update result inside backoff; oracle = retried once backoff elapses with no backend-row write.
- [ ] AC4: Every registered reconciler is re-evaluated for every target in desired ∪ actual, enumerated from intent and observation. A target with intent and no `alloc_status` row is included.
- [ ] AC5: At a converged state, re-evaluation emits zero non-`Noop` actions, and evaluation cost per period is bounded by target count and no greater than today's relist (G1).
- [ ] AC6: Tier-1 seeded `overdrive-sim` invariant, seed printed on failure, covers the three fault stimuli and the recovery bound.

#### Technical notes
- The runtime-wide periodic pass changes `reconcilers.md` § "The wakeup model" (a reconciler declares row-backed OR host-backed, "never both") and ADR-0084; reconciled by ADR-0296 (D-1, approved).
- The no-busy-loop rule holds: re-evaluation writes no observation row.
- Period: 30 s constant (D-2). Enumeration: `managed_targets` (D-1a). Interacts with #270 and #271.

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
- S1 Arbitrary independent `(desired, actual, view)` (the View generator includes `View::default()`, the forgotten View) converges within N ticks for every registered reconciler (progress), subject to the `service-lifecycle` scoping in D-6.
- S2 Once converged, no further non-`Noop` actions (stability).
- S3 With random dropped actions the reconciler still converges (same scoping).
- S4 Applying any emitted action twice through the production shim leaves the same observable state.
- S5 A View field that overrules `actual` makes the property fail.

#### Acceptance criteria
- [ ] AC1: Every reconciler registered in `run_server` is covered, or a reasoned no-obligation ruling is recorded per exempt one: 7 always (`noop-heartbeat`, `workload-lifecycle`, `workflow-lifecycle`, `svid-lifecycle`, `vm-reclamation`, `service-map-hydrator`, `service-lifecycle`), plus `gateway-svid-lifecycle` when `config.gateway` is `Some`, so 7, or 8 with the gateway.
- [ ] AC2: Runs in the default lane (no real I/O); seed printed on failure; failing run reproducible from the seed.
- [ ] AC3: Fault stimulus = dropped actions and View/actual skew in both directions; oracle = `actual == desired` within N ticks, then zero actions. For `service-lifecycle` the probe-driven stop obligation is scoped out of this oracle (D-6).
- [ ] AC4: Mutation check on the property: DISTILL names the mutants from the live reconcilers (for example removing the resync pass, or a View field that overrules `actual`); each turns the property red. The local-backend latch is not a mutant (the path is deleted, D-9), and the VIP-release latch moved to #211.

#### Technical notes
- No new sim executor (D-7). The property drives the production `run_convergence_tick` over a sim-composed `AppState`, so hydration, View persist and the real action shim run against `Sim*` ports. The existing `vm_reclamation` `apply_actions` executor is not reused: a hand-written executor re-implements the shim. The hydrator DST fabricates a `Completed` row and its "any combination of BPF state" claim is false; that is the shape D-7 avoids.
- DISTILL authors the full bodies; a scenario for a later slice is marked pending with `#[should_panic(expected = "<its semantic RED>")]`, per `testing.md` § "RED acceptance scaffolds and activation". DISTILL names the semantic RED per scenario. `#[ignore]` is only for a genuinely missing external resource.
- Boundary with #344: this property does not decide read monotonicity.

### US-06: The rule says what a View may and may not decide @infrastructure

`job_id: J-PLAT-004` · Persona: Devon · No elevator pitch (documentation of a rule; merged into the slice of US-05 so the slice has a user-visible story).

#### Acceptance criteria
- [ ] AC1: `.claude/rules/reconcilers.md` replaces the fingerprint-shaped symptom with the general prohibition in D9 (see "Proposed rule text"), including the test "a pacing input decides only when to act again; a marker keyed to an observed input suppresses re-emission only for that same input" and "if the effect cannot be observed, make it observable first".
- [ ] AC2: The rule is timeless discipline (principle plus symptoms); no ADR numbers as status, no DELIVER-phase notes.
- [ ] AC3: The rule names no exception (D13: `last_emitted_backend_fingerprint` no longer exists).

## Wave: DISCUSS / [REF] Story map and prioritization

Backbone (operator path): Deploy → Reconcile tick → Dispatch effect → Observe result → Converged. Author path: Write reconciler → Check convergence.

| Slice | Stories | Walks | Value | Urgency | Effort | Order |
|---|---|---|---|---|---|---|
| 01 periodic-reevaluation | US-01 | Reconcile tick → Converged | 5 | 5 | 3 | 1 |
| 04 convergence-property-and-view-rule | US-05, US-06, executor doc fix (D-5) | Write reconciler → Check | 4 | 3 | 3 to 4 | 4 |

US-04 (the `service-lifecycle` probe-driven stop markers) has no change, no AC and no code: see D-6.

Priority rationale: slice 01 first because it fixes the broadest class (any hydrate or View-write error wedges a deploy) with the least dependency, and it is the riskiest assumption (ADR-0084 tension). Slice 02 is the deletion of the unreachable local-backend path (its latch was the one DISCUSS thought was production-reachable); it depends on nothing and runs first in DELIVER so later slices do not touch the code. The convergence-property slice is last so the property is green on the fixed reconcilers and red on any reintroduced latch; DISTILL still authors it early.

## Wave: DISCUSS / [REF] Outcome KPIs

| # | Who | Does what | By how much | Baseline | Measured by |
|---|---|---|---|---|---|
| K1 | Ana | A workload hit by a transient hydrate or View-write error reaches Running without re-submit | 100% of seeded runs within one re-evaluation period of the fault clearing | never (wedged until re-POST) | Tier-1 seeded sim invariant, faults injected at the `Sim*` port seams |
| K4 | Devon | View fields that decide effect presence in registered reconcilers, in this feature's scope | 0 | 1: the hydrator's `last_applied_local_fingerprint`, removed by deleting the unreachable local path (D-9). `released_for_deletion` is #211's and `observed_generation` is #325's; the act-once markers are pacing keyed to an observed input (D-6). | latch audit at feature close |
| K5 | Devon | Registered reconcilers covered by the shared property | all registered (7, or 8 with the gateway) or ruled exempt | 0 | test inventory |
| K6 | Devon | Injected latch mutants caught by the property | all mutants DISTILL names (see US-05 AC4) | 0 | mutation check on the property |
| G1 (guardrail) | Platform | Converged-state evaluation cost | No regression against today's router relist plus `resync_schedule`: the pass is not above the baseline at any measured N (V-3 table). The 3% duty target is met at 100 workloads and not at 500 or 1000. Scope limitations tracked by #270 (scan cost) and #357 (edge-wake deploy latency during a pass, unchanged from today). Zero non-`Noop` actions at fixed point | n/a | metal measurement (V-3 evidence); sim cost counter; seeded sim |

## Wave: DISCUSS / [REF] Definition of Done

- All slice ACs green; each oracle observed through the production composition (`run_server` convergence loop), not hand-installed wiring; faults are injected only at `Sim*` port seams.
- Tier-1 seeded invariants print the seed.
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
| `service-lifecycle` `terminal_announced` and act-once stop markers | not changed (D-6) |
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
| D-1 | APPROVED 2026-11-13 | ADR-0296 as proposed, plus: cite #275 (host-edge wakeup, open; a future host watcher submits through the same `submit_evaluation`, the pass is its level-triggered backstop); state that no reconciler can ask for a faster sweep than the global period. Includes the read-only accessor `pending_keys` on `BrokerDrain`. |
| D-2 | APPROVED 2026-11-13 | 30 s stays a compile-time constant; the limitation is tracked by #356 ("Make the reconciler resync period operator-configurable", created by the user). |
| D-3 | APPROVED 2026-11-13; KEPT 2026-11-15 | The hydrator's `next_evaluation_at` retry deadline stays (removal rejected by the user). |
| D-5 | APPROVED 2026-11-13 | The `release_service_vip` executor doc fix stays (slice 04). Replacing `released_for_deletion` and the seeded reproduction of the possible VIP leak move to #211. This feature does not wait on #211. |
| D-7, D-8 | APPROVED 2026-11-13 | D-7: the slice 04 property drives the production tick over sim-composed ports; no new sim executor. D-8: `workflow-lifecycle` gets no dedicated wake; the uniform pass covers it at zero cost. Target confirmed 2026-11-15 (row below). |
| D-6 | option (a) APPROVED 2026-11-13 | No change to `service-lifecycle` (see "D-6"). |
| D-1a-i | APPROVED 2026-11-15, option A | See "D-1a-i" under the interface contract. |
| D-8 (target) | CONFIRMED 2026-11-15 | `workflow-lifecycle`'s managed target is the single target `workflow/all`, returned only when a workflow instance exists (empty in production today). |
| D-11 (wording) | RULED 2026-11-15 | ADR-0298's Context states the actual fact about the existing route check (GH #358 records the colliding VIP); the `Policy` refusal stays for all VIP-range destinations. |
| Model check `PendingIsAdmitted` | DROPPED 2026-11-15 (user ruling) | Removed from the Formal protocol model and the validation budget; the modeler removes it from the spec. `edge_wake_during_burst_is_admitted_after_burst` stays. |
| D-9 | APPROVED 2026-11-15 | Delete the ADR-0053 same-host local-backend path (ADR-0297, which supersedes ADR-0053). It becomes slice 02, scheduled first in DELIVER. The user delegated the placement. Re-confirmed the same day for the whole path, including `cgroup_connect4_service`, `LOCAL_BACKEND_MAP`, ADR-0164's datagram clause and the owner's cgroup-placement check, after weighing the alternative of keeping the connect4/datagram pieces for #310. Reason: `ovd_fwd` forwarding between a connected host UDP socket and a vsock stream is independent of the connect4 rewrite; with `LOCAL_BACKEND_MAP` always empty the hook is a pass-through for every owner connect today; #310 designs its own destination selection. |
| D-10 | APPROVED 2026-11-15 | ADR-0164's datagram clause and its startup check are superseded (ADR-0298). ADR-0164's TCP-as-mesh decision (D24a) stays. Guest UDP to a VIP is unsupported until GH #310. |
| D-11 | APPROVED 2026-11-15 (option B) | A guest UDP datagram to a VIP-range destination is refused with the existing `FlowRefusal::Policy` until GH #310 (ADR-0298). No new refusal variant, port or wire surface. The user did not ask for a Tier-3 observable for it. |
| D-12 | APPROVED 2026-11-15 | The Quint modeler's gaps in the spec, pinned as interface contracts: see "Conformance pins (D-12)". Two items change surface: a per-reconciler resync step as a crate-private driving port, and an ordering clause on `pending_keys`. |
| Router wakes | RULED 2026-11-15 | The interest router's boot LIST and `Lagged` relist stay (already ADR-0296 decision 4). ADR-0296's alternatives table records the rejection of removing them. |
| V-3 | RUN 2026-11-15 | A cost measurement against the baseline; it records a measured property and does not choose a mechanism. See V-3. |
| Slice 01 Tier-3 `persist_view` scenario | DROPPED 2026-11-16 (user ruling) | All three slice 01 faults are covered at Tier 1 through the existing `Sim*` port seams. Reasons: it was agent-added, not user-requested; redb 2.6.3 poisons a `Database` permanently after a failed commit (`StorageError::PreviousIo` until reopen) and `RedbViewStore` never reopens, so a real disk error never recovers and the scenario's oracle cannot hold (tracked by #360); and the feature changes only the runtime's response to a persist error, which the Sim seam exercises. |
| D-13 | APPROVED 2026-11-15 (option A) | See "D-13". |
| D-14 | RULED 2026-11-16 | The Quint spec models only production-reachable `(reconciler, target)` pairs, so the quint-connect driver can replay it. The keys are re-keyed (`specs/quint/reconciler-evaluation-wake/README.md`; see "Formal protocol model"). Rejected: a test-only reconciler variant in `AnyReconciler`. |
| D-15 | RULED 2026-11-16 | DISTILL scaffolds the pinned surface with inert bodies: `Reconciler::managed_targets` (D-1a; empty set), `resync_reconciler` (D-1b; `Ok(0)`, not wired) and `BrokerDrain::pending_keys` (D-12; empty `Vec`). Dependent tests are pending RED on semantic assertions. DELIVER replaces the bodies. |
| D-16 | RULED 2026-11-16 | The slice 04 property lives in `overdrive-control-plane` `src/convergence_property/` as a `#[cfg(test)]` module, so it drives re-evaluation through the crate-private `resync_reconciler`. No `#[doc(hidden)]` test seam. |
| D-17 | RULED 2026-11-16 | The spec models the production reconciler-to-reconciler handoff wakes deterministically: `workload-lifecycle` → `service-lifecycle` and `svid-lifecycle`; `service-lifecycle` → `service-map-hydrator` only when a backend row is written. It adds an `svid-lifecycle` key, and models `svid-lifecycle`'s own backoff window and retention and the View-unchanged persist skip. One faithful spec serves both checking and conformance, per Quint's model-based-testing guidance. Rejected: an abstract main for proofs split from a faithful main for replay. |
| D-18 | RULED 2026-11-16 | `service-lifecycle`'s managed set is a Service with an allocation row or an assigned VIP, never intent alone (D-1a row narrowed accordingly). `svid-lifecycle` manages workloads with an allocation row (plus the held-SVID leg of D-1a-i). |
| D-19 | RULED 2026-11-16 | Validation budget growth is accepted, with no time cap. `design.md`'s two-minute rule decides `ci` only. `ProgressByPass` and `RetentionSchedulesBackedOffKey` are `ci = false` (about 80 min and about 10.5 min). The two hazards and the 10 named runs stay `ci = true`. |
| D-20 | APPROVED 2026-11-17, option (a) | `service-map-hydrator`'s `actual` is the resource it manages: the programmed service map, read through a new narrow read-port (see "D-20"; ADR-0299). Its own `service_hydration_results` row is no longer an input to `reconcile`. Rejected: invalidating the result rows at boot (fixes a reboot only, misses a map lost while running). |
| D-21 | APPROVED 2026-11-17, option (a) | The `service-map-hydrator` managed-set row of D-1a covers a Service that has a workload intent or a `service_backends` row. An orphaned `service_hydration_results` row (intent and backend row both gone) is the deletion case and belongs to #211. No list-all method is added to `ObservationStore`. |
| D-22 | RULED 2026-11-17 | The Quint managed-set model matches the approved managed sets (D-1a, D-18, D-21); production and the conformance driver are unchanged. The model adds `VipImpliesHydratorManaged` (a Service intent with an assigned VIP makes the hydrator key managed) and `K4ManagedIffRunningOrHeld` (the `svid-lifecycle` key is managed iff a Running allocation or a held SVID exists, so it can leave the set); the named runs `enumeration_failure_of_one_reconciler_does_not_starve_others` and `edge_wake_during_burst_is_admitted_after_burst` carry the changed expected states. |
| D-20 (read mechanism) | APPROVED 2026-11-17, fingerprint in the resource | The programmed-map read is a 64-bit fingerprint of the applied backend set, stored in two extra slots of each inner `SERVICE_MAP` array and written before the atomic outer swap (see "D-20"; ADR-0299). Inner capacity becomes 16,383; XDP still indexes modulo 16,381. The port returns the fingerprint, not a backend set. Rejected, with the V-6 measurements: the full-table batch scan (5.41 ms p50 per service) and mmap-able inner maps (107 µs p50 plus `BACKEND_MAP` lookups, and the slot-starvation precondition stays). V-6 has run. |

Decision records: D-1, D-2 → ADR-0296; D-9 → ADR-0297; D-10, D-11 → ADR-0298; D-20 → ADR-0299; D-3, D-5 to D-8, D-12 to D-19, D-21 → this delta.

## Wave: DESIGN / [REF] Domain model (DDD list)

| Term | Meaning |
|---|---|
| Managed target set | The targets for which one reconciler has desired or actual state, in its own target shape, enumerated from intent and observation. |
| Resync pass | One runtime sweep that submits an `Immediate` evaluation for every managed target of every registered reconciler. It is a level wake. |
| Edge wake | A submit caused by a change: an accepted observation row (`interests()`), `EnqueueEvaluation`, a client request, a self-requeue. |
| Evaluation outcome | `ok`, or an error ending the evaluation at hydrate, persist or dispatch. Retry rule: ADR-0296 decision 3. |
| Pacing input | A View field that decides only when to act again: attempts and last attempt time, or a marker keyed to an observed input (the probe row's `run_seq`). It never decides whether an effect is in place. |

No new aggregates, bounded contexts or events. One new read-port, `ProgrammedServiceView` (D-20).

## Wave: DESIGN / [REF] Component decomposition

| Component | Crate | Change | Owner of |
|---|---|---|---|
| `Reconciler::managed_targets` (new method) | `overdrive-core` | EXTEND trait | the reconciler's target set |
| `AnyReconciler::managed_targets` (forward arm) | `overdrive-reconcilers` | EXTEND | none |
| Eight reconciler impls of `managed_targets` | `overdrive-reconcilers` | EXTEND | their own target sets |
| Resync pass and loop | `overdrive-control-plane` (`reconciler_runtime`, `lib.rs`) | CREATE NEW, replaces cadence phase and router periodic relist | the level wake |
| Interest router | `overdrive-control-plane` | EXTEND by deletion: periodic arm and its two parameters go | edge wakes from rows |
| `ServiceLifecycle` | `overdrive-reconcilers` | EXTEND: `managed_targets` only (D-6) | the probe-driven stop decision |
| `ServiceMapHydrator` | `overdrive-reconcilers` | EXTEND: `managed_targets`, `next_evaluation_at` over the remote `retries` map; local partition and fingerprint DELETED (D-9); `actual` read from the programmed service map, result-row arms of `should_dispatch` deleted (D-20) | the remote service-map effect |
| `ProgrammedServiceView` (new port), `HydrationContext::programmed_services`, `HydrateError::DataplaneRead` | `overdrive-core` | CREATE NEW (D-20; no existing read operation on the dataplane) | none (read-only) |
| `ProgrammedServiceView` impls | `overdrive-dataplane` (`EbpfDataplane`), `overdrive-sim` (`SimDataplane`) | CREATE NEW | none (read-only) |
| `DataplanePorts`; `ComposedDataplane.dataplane` and `AppState.dataplane` typed as it | `overdrive-control-plane` | CREATE NEW value type, EXTEND two fields (D-20 "Composition wiring") | the guarantee that the write port and the read port are one instance |
| The same-host local-backend path (inventory in slice 02) | `overdrive-bpf`, `overdrive-dataplane`, `overdrive-core`, `overdrive-control-plane`, `overdrive-reconcilers`, `overdrive-sim` | DELETE | none (no production producer, ADR-0297). Also deletes, in `overdrive-control-plane` and `overdrive-sim`, the guest-flow owner's `OwnerCgroupPlacement` startup check and datagram-to-VIP handling (D-10, ADR-0298); inventory in slice 02 |
| Convergence property harness | `overdrive-control-plane` tests | CREATE NEW (test support, DISTILL shapes it) | none |

Dependency direction is unchanged. `overdrive-core` gains no I/O dependency.

## Wave: DESIGN / [REF] Driving and driven ports

Driving (pinned for conformance; D-12): `resync_reconciler` (one reconciler's step of the pass; the pass loops over it), `ReconcilerRuntime::submit_evaluation`, `BrokerDrain::drain_pending` (admission), `run_convergence_tick` (one evaluation, hydrate to dispatch), `spawn_interest_router` (edge). Driven: the existing intent, observation, view and dataplane ports are unchanged except that the `Dataplane` port loses its two local-backend operations (D-9); one read-port is added, `ProgrammedServiceView` (D-20), reached only through `HydrationContext`.

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
| `service-lifecycle` | `workload/<id>` of each Service-kind workload (kind from the intent discriminator or the row) that has an `alloc_status` row or an assigned VIP (`ServiceVipView`); never from intent alone (an intent alone makes only `workload-lifecycle` manage it) |
| `svid-lifecycle` | `workload/<id>` for the workload ids of `alloc_status` rows ∪ the workload ids of held SVIDs (`HeldSvidView`); see D-1a-i. A held SVID can exist with no `alloc_status` row when the `StartAllocation` dispatch holds it (`ensure_intercept_identity`, before `driver.start`) and the row write that follows fails; only an evaluation of that workload can emit its `DropSvid`. A held entry whose `spiffe_id` does not invert is not returned. |
| `service-map-hydrator` | `service/<id>` for each `ServiceId` of a listener of a workload intent (the id `hydrate_desired` derives from the assigned VIP, port and protocol; confirm the derivation in code before DELIVER) ∪ each `service_id` in `service_backends` rows. A `service_hydration_results` row adds no target: the hydrator no longer reads it (D-20), and an orphaned row (intent and backend row both gone) is the deletion case, #211 (D-21). No list-all method is added to `ObservationStore`. |
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

Retry rule: ADR-0296 decision 3 (`run_convergence_tick_inner` is unchanged in control flow). The `development.md` § "Reconciler I/O" text (D15) and the proposed rule text cite that decision.

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

Returns the earliest future `last_failure_seen_at + backoff_for_attempt(attempts)` over the remote `retries` map for a service whose programmed fingerprint differs from the fingerprint of its desired programmable set (D-20); `None` otherwise. It reads only its inputs. Existing hook; no trait change.

### D-20. `service-map-hydrator` reads the programmed service map (APPROVED 2026-11-17, option (a); ADR-0299)

**Context and problem.** The hydrator emits `DataplaneUpdateService` and, until now, decided whether the effect was in place from its own `service_hydration_results` row (the executor's receipt of the last update). After a node reboot, or a restart of `serve` (which recreates the pinned `SERVICE_MAP`, `overdrive-dataplane/src/lib.rs:507-535`), the result rows persist in redb and the kernel maps are empty. A `Completed` row over the desired fingerprint then reads as converged, the hydrator emits nothing, and the service is never reprogrammed. Evidence: the hydrator property's reboot world (`distill/test-scenarios.md` § "Production finding: hydrator receipt vs resource"). A map lost while running has the same effect.

**Existing mechanism.** The `Dataplane` trait has no read operation. `SimDataplane::service_backends_for` and `EbpfDataplane::service_map_contains` are test and diagnostic accessors, not part of any port. The issue tracker was not searchable in this session (no `gh`). The ADRs and this delta name no planned read-back of the service map (grep of `docs/product/architecture` for `service_hydration_results` and `service_backends` finds only the receipt design of ADR-0042). Run `gh issue list --state all --search "service map programmed read"` before acceptance and cite a hit if one exists.

**New read-port (core, `overdrive-core`, `traits/programmed_service_view.rs`).**

```rust
#[async_trait]
pub trait ProgrammedServiceView: Send + Sync {
    async fn programmed_fingerprint(
        &self,
        frontend: &ServiceFrontend,
    ) -> Result<Option<BackendSetFingerprint>, DataplaneError>;
}
```

`BackendSetFingerprint` is the existing `u64` alias (`overdrive-core/src/dataplane/fingerprint.rs`). The port adds no other type. A backend set is not returned: the hydrator needs to know whether the programmed set equals the desired one, and a fingerprint answers that (ADR-0299).

**The fingerprint written with the table.** `update_service(frontend, backends)` with a non-empty `backends` computes `fingerprint(&frontend.vip, &backends)` (the existing function, over the slice it was given, in the order it was given), and writes its low 32 bits to inner slot 16,381 and its high 32 bits to inner slot 16,382 of the new inner array, before the atomic outer swap. The inner array has 16,383 slots (`SERVICE_MAP_INNER_CAPACITY` = 16,381 + 2); the prototype and every inner map carry that capacity, because the kernel rejects an inner map whose `max_entries` differs from the prototype's (`array_map_meta_equal`, `kernel/bpf/arraymap.c:594-601 @ v7.0`). XDP still indexes the table modulo 16,381, so slots 16,381 and 16,382 are never selected. `update_service(frontend, [])` deletes the outer slot (existing behaviour), which discards the fingerprint with the table. The hydrator computes its wanted fingerprint with the same function over the same slice it emits as the action's backends, so both sides hash one input.

Rustdoc contract on `programmed_fingerprint`:

- Preconditions: `frontend` is V4 by construction (`ServiceFrontend::new`). The impl is bound to the same dataplane instance the action shim writes through `Dataplane::update_service`; a view over another instance reads a different resource. The composition root supplies both from one instance.
- Postconditions on `Ok(Some(fp))`: `fp` is the fingerprint of the backend slice of the last successful non-empty `update_service` for the exact key `(frontend.vip, frontend.port, frontend.proto)`, as stored in the resource that is the service map's table for that key. The host impl reads it from the inner array the `SERVICE_MAP` outer slot currently points to: outer lookup (returns the inner map id), `BPF_MAP_GET_FD_BY_ID`, then the two element lookups at slots 16,381 and 16,382, recombined as `low | high << 32`. The sim impl reads the fingerprint its stored service carries. No in-process tracker, receipt or cache is the source. The call mutates nothing (no map, tracker or allocator memo). The result is one state between whole `update_service` calls, never a mix of two: the host takes the apply-commit guard for read for the whole sequence, so the inner map id it resolved cannot be freed before it is opened.
- Edge cases: `Ok(None)` means no slot is programmed for the key. `update_service(frontend, [])` withdraws the slot (its existing contract), so "never programmed", "withdrawn" and "lost with the maps" are indistinguishable by design. A frontend never programmed is `Ok(None)`, not an error. Other ports and protocols on the same VIP are independent keys. A failed `update_service` (including a set with an IPv6 backend, which is refused before any write) leaves the prior fingerprint in place. A failed kernel read, including `ENOENT` on the inner map id, returns `Err(DataplaneError::LoadFailed)` and is never reported as `Ok(None)` (`rust.md` § "Distinct failure modes").
- Scope of the comparison: the fingerprint covers every backend of the applied slice with its allocation, address, weight and health, and the VIP. It does not report whether `BACKEND_MAP` still holds each entry. A `BACKEND_MAP` entry lost while the inner map survives is not detected; both maps are created and destroyed together and only an Overdrive bug can separate them (ADR-0299).
- Invariants: (1) after a successful `update_service(f, B)` with `B` non-empty and no later write, `programmed_fingerprint(f)` equals `Some(fingerprint(&f.vip, &B))`; (2) after `update_service(f, [])` it is `None`; (3) a freshly constructed dataplane instance reads `None` for every frontend (process restart, reboot); (4) two reads with no write between are equal; (5) `SimDataplane` and `EbpfDataplane` return identical results for the same call sequence.

**Reaching `hydrate_actual`.** `HydrationContext` gains `pub programmed_services: &'a dyn ProgrammedServiceView`. `HydrateError` gains `DataplaneRead(String)` (the `Display` of the `DataplaneError`). The hydrator's `hydrate_actual` re-projects the target's frontend exactly as `hydrate_desired` does (the `service_backends` row plus the listener fact through `project_service_desired`), calls `programmed_fingerprint` for it, and returns it as `actual`. A target with no resolvable frontend yields an empty `actual` (its `desired` is skipped too). A read error ends the evaluation (no View write, no dispatch, next pass retries, ADR-0296 decision 3). `ServiceMapHydratorState.actual` becomes `BTreeMap<ServiceId, BackendSetFingerprint>`: a key is present when a slot is programmed, absent when `programmed_fingerprint` returned `None`. `ServiceHydrationStatus` is no longer part of the hydrator's state. The `service_hydration_results` read in `hydrate_actual` is deleted.

**Implementations.** Host: `EbpfDataplane` implements the port over the kernel inner map (not over `service_backends` or any tracker), and its `update_service` writes the fingerprint slots as above. Sim: `SimDataplane` implements it over its stored `services` map, recording the fingerprint of the slice each successful non-empty `update_service` received and dropping it on an empty update. A sim reboot is a fresh `SimDataplane`; a map lost while running is a DISTILL test-support seam. Both ports are reached from one instance through `DataplanePorts` (below); a test composition supplies a `SimDataplane` through its `DataplaneComposer` (`SimDataplaneComposer`), which hands out both ports from that one `SimDataplane`.

**Composition wiring (one instance behind both ports).** `overdrive-control-plane` defines one value type, `DataplanePorts` (`node_adapters.rs`, re-exported at the crate root beside `ComposedDataplane`):

```rust
#[derive(Clone)]
pub struct DataplanePorts { /* private: the write port and the read port */ }

impl DataplanePorts {
    pub fn new<D>(instance: Arc<D>) -> Self
    where D: Dataplane + ProgrammedServiceView + 'static;
    pub fn dataplane(&self) -> &Arc<dyn Dataplane>;
    pub fn programmed_services(&self) -> &Arc<dyn ProgrammedServiceView>;
}

impl<D> From<Arc<D>> for DataplanePorts
where D: Dataplane + ProgrammedServiceView + 'static;
```

The fields are private. The only constructor takes one concrete `Arc<D>` that implements both traits and coerces that same `Arc` to both trait objects, so a write port and a read port over different instances cannot be built (the precondition of `programmed_fingerprint`). A holder of only an `Arc<dyn Dataplane>` cannot build one, because `D` is sized and has no view. The read port stays separate from `Dataplane` (ADR-0299).

- `ComposedDataplane.dataplane` changes from `Arc<dyn Dataplane>` to `DataplanePorts`; `host_ipv4` and `gateway_connect` are unchanged. Four struct-literal sites change: `HostDataplaneComposer::compose` (`DataplanePorts::new(Arc::clone(&ebpf_dataplane))`; `gateway_connect: ebpf_dataplane` as today), `SimDataplaneComposer::compose`, `TestDataplaneComposer::compose` (`shared_network_test_ports.rs`), and the test-local `FixtureComposer` (`tests/integration/public_ingress_gateway_boot_shutdown.rs`). The last three build it from their `Arc<SimDataplane>`.
- `AppState.dataplane` changes from `Arc<dyn Dataplane>` to `DataplanePorts`. `AppState::new` and `AppState::new_with_workflow_engine` take `dataplane: impl Into<DataplanePorts>` in the same position. `run_server` passes the `DataplanePorts` it destructures from `ComposedDataplane` (its only use of the value). A fixture that passes an `Arc<SimDataplane>` compiles unchanged through the `From` impl.
- Readers: `action_shim/mod.rs` (two sites) reads `state.dataplane.dataplane().as_ref()`; `listener_facts.rs` (the reassembly helper) passes `state.dataplane.clone()`; `build_hydration_context` sets `programmed_services: state.dataplane.programmed_services().as_ref()`. The `InertProgrammedServices` scaffold is deleted.
- Construction sites. `AppState` is built through `new` (fixtures; it delegates to `new_with_workflow_engine`) and `new_with_workflow_engine` (`run_server` and two workflow end-to-end tests); there is no other helper and no struct literal outside `lib.rs`. `AppState::new` has 96 call sites in 85 files under `crates/` (one more in `spike-scratch/`, which is throwaway and not maintained); `new_with_workflow_engine` has 3. Under the `From` shape only the callers that hold a trait object change: `tests/acceptance/service_frontend_ipv6_rejected.rs`, `tests/integration/service_map_hydrator_dispatch.rs` (three sites), and the `dataplane` field of `tests/common/vm_dispatch.rs`. Their test doubles (`PanicOnUpdateService`, `FailingUpdateService`) implement `ProgrammedServiceView` (test support, DISTILL's to shape).
- Test obligation: none added. The reboot-world and map-lost-world hydrator tests run through `build_hydration_context` over a sim-composed `AppState` and cannot go GREEN unless the view reads the instance the shim writes through.

**`update_service` and IPv6 backends.** `Dataplane::update_service` refuses a non-empty `backends` that contains a backend whose `addr` is IPv6: it returns `Err(DataplaneError::LoadFailed(..))` before any write. No map, tracker, allocator memo or identity reservation changes, and the frontend's prior programmed state, including its fingerprint (`programmed_fingerprint`), is unchanged. The refusal is the first validation: it precedes the identity-conflict check and the reverse-NAT ownership check, so a set with both defects returns `LoadFailed` from both adapters. `backends.is_empty()` is the withdraw and is unaffected. No new error variant.

The `update_service` rustdoc, which is the SSOT, states this: the precondition no longer says non-IPv4 addresses are skipped from the key set; the key-set postcondition ranges over every backend (each is IPv4 once the call returns `Ok`); the edge case "an IPv6 `addr` contributes no `REVERSE_NAT` key (silently skipped)" becomes the refusal above. Both adapters implement it.

Evidence. The host refuses today: `BackendEntryPod::from_backend` returns `LoadFailed` for an IPv6 address (`overdrive-dataplane/src/maps/mod.rs:129-142`); `plan_service_apply` calls it first, in input order (`overdrive-dataplane/src/lib.rs:1932-1934`), ahead of the identity preflight (`:1947`) and R-59; `apply_service` calls `plan_service_apply` (`:2228`) before `precommit_service_apply` (`:2240-2243`), so nothing is written. Production cannot produce an IPv6 backend: the rows' backends are built as `SocketAddr::new(IpAddr::V4(backend.ip), port)` (`overdrive-reconcilers/src/service_lifecycle.rs:1119-1121`) and the row's VIP is an `Ipv4Addr` (`project_service_desired`, `service_map_hydrator.rs:112`). The contract therefore governs a state no production path reaches; it exists so that the trait is unambiguous and the two adapters agree.

Why refusal and not skip. The fingerprint postcondition covers every backend of the applied slice. An `update_service` that returned `Ok` after dropping an IPv6 backend would record a fingerprint of a set it did not program, and the hydrator would then read equal and judge the service converged. Refusal also keeps `Ok(())` meaning "the requested set is programmed". A hydrator survivor set that contains an IPv6 backend is refused on every attempt, is never in place, and retries on its backoff (D-20 diff, item 1).

What changes. Code (DELIVER): the `Dataplane::update_service` rustdoc in `overdrive-core/src/traits/dataplane.rs` (the precondition at `:223-225`, the postcondition at `:229-235`, the edge case at `:248-249`, and the fingerprint postcondition); `SimDataplane::update_service` (`overdrive-sim/src/adapters/dataplane.rs`) refuses before the identity preflight, and the comment on `reverse_nat_key_for` (`:477-480`, "silently skipped, parity with the production `EbpfDataplane`") is corrected. The host adapter needs no behaviour change. Tests: S-02-D, `ipv6_backend_contributes_no_reverse_nat_key` in `overdrive-sim/tests/sim_dataplane_reverse_nat_per_proto.rs` (`:197-216`, plus the header line `:13` and the local oracle arm `:74`), asserts the skip and is rewritten by the acceptance designer to assert the refusal and an unchanged key set; because the sim accepts the set today it is authored as a pending RED (`should_panic`, naming the refusal) and activated by the DELIVER step that changes the sim. The DISTILL tests already follow this contract and do not change: `a_refused_ipv6_update_leaves_the_prior_read_unchanged` (`overdrive-sim/tests/sim_dataplane_programmed_fingerprint_contract.rs:237-251`) and the refused step in `programmed_service_view_equivalence.rs:129-148`. Docs: the S-02-D scenario in `docs/scenarios/udp-service-support/test-scenarios.md:234-241` is stated as the refusal; the copies in `docs/feature/udp-service-support/distill/test-scenarios.md` and `feature-delta.md:1214` are records of a delivered feature and stay as written. GH #155 (IPv6 support), cited in the existing source comments, is the tracked reason IPv4 only; I did not re-fetch it this session.

**The hydrator's diff.** Pure, in `reconcile`:

1. `in_place`: with a non-empty set of post-gate remote survivors (the only partition once D-9 lands), `actual` for the service holds a fingerprint equal to `fingerprint(vip, survivors)`, the same value the existing `programmed_fingerprint` computation yields; with an empty set, `actual` has no entry for the service (a purge in place). A survivor set that `update_service` refuses (an IPv6 backend) is never in place, because the refused update writes no fingerprint.
2. A dispatch is needed when `!in_place` and pacing allows it. Pacing is the existing retry memory: no entry, or `last_attempted_fingerprint != programmed fingerprint` (a changed desired set dispatches at once), or the backoff window `last_failure_seen_at + backoff_for_attempt(attempts)` has elapsed.
3. When `in_place` and a retry entry exists, the entry is removed.
4. The emitted action is unchanged (`DataplaneUpdateService` with the remote survivors; empty is the purge). `RetryMemory` keeps its three fields and now decides only when to act again. `last_attempted_fingerprint` stays a pacing input that detects a changed desired set; the diff is the programmed fingerprint read from the resource. The hydrator's fingerprint is no longer compared with a receipt, only with the fingerprint the dataplane wrote into the table.
5. `next_evaluation_at` (D-3) ranges over retry entries of services that are not `in_place`.

The result row's role: the executor keeps writing it (ADR-0042 §4, operator-visible outcome and the `Failed` error); the hydrator no longer reads it. Its `should_dispatch` arms on `None`, `Pending`, `Completed` and `Failed` are deleted. Because the hydrator no longer reads result rows, a result row alone makes no target managed (the D-1a row, D-21). Any other consumer of the row is unchanged by this decision.

**Not covered, tracked.** A programmed service whose `service_backends` row is gone has no `desired`, so the hydrator does not purge it. That is the teardown case and belongs to #211 ("Implement workload deletion (intent withdrawal) + service/dataplane teardown", per D-5; I did not re-fetch it this session, so verify its scope with `gh issue view 211 --comments` before relying on this sentence).

**Why the fingerprint is not the View-marker anti-pattern** (`reconcilers.md` § "Symptoms during review"). That smell is a hash the reconciler keeps in its own `View` of what it emitted, stamped before the effect is attempted, which outlives the effect across restarts. This fingerprint lives in the kernel resource, is written by the dataplane in the same atomic outer swap that publishes the table, and is discarded with the table when the maps are recreated or the slot is withdrawn. A fingerprint present in the programmed map therefore means the table it sits in is the one in force. The hydrator's own `last_attempted_fingerprint` stays a pacing input and is never the diff.

**Remaining blind spot.** A `BACKEND_MAP` entry lost while its inner map survives reads as in place. `SERVICE_MAP` and `BACKEND_MAP` are created and destroyed together (`EbpfDataplane::new_with_pin_dir`), and only the appliance's own code writes either, so only an Overdrive bug produces it. The design does not defend it (CLAUDE.md "design for Overdrive's own failures").

**Cost.** One read per hydrator evaluation, and the pass evaluates each service target once per 30 s. Measured on kernel 7.0.0-38-generic with 1,000 frontends of 16 backends each (V-6, `spike/findings-programmed-service-map-read.md` § "Comparison of read mechanisms"): read p50 4.7 µs, p99 6.3 µs; a pass over 1,000 services reads for about 6.2 ms. Apply p50 13.5 ms, no slower than the 17.2 ms baseline. The read holds the apply-commit guard for read for these microseconds and touches no packet path. The write side adds two `u32` element updates to an inner array built before the swap.

**Kernel claims (all `@ v7.0`, read in `vendors/linux`).**

- K-1. A userspace lookup on a `HASH_OF_MAPS` returns the inner map's id: `map_lookup_elem` takes the fd-hash path (`kernel/bpf/syscall.c:329-330`), `bpf_fd_htab_map_lookup_elem` (`kernel/bpf/hashtab.c:2626-2643`) returns `map_fd_sys_lookup_elem(ptr)`, which is `((struct bpf_map *)ptr)->id` (`kernel/bpf/map_in_map.c:131-134`).
- K-2. `BPF_MAP_GET_FD_BY_ID` yields an fd for that id and requires `CAP_SYS_ADMIN` (`bpf_map_get_fd_by_id`, `kernel/bpf/syscall.c:4859-4888`). The node process is root on the appliance. The call returns `ENOENT` once the last reference to a replaced inner map drops, which is why the read runs under the apply-commit guard.
- K-3. The outer swap is atomic with respect to readers: `bpf_fd_htab_map_update_elem` replaces the slot under the hash bucket lock (`kernel/bpf/hashtab.c:2646-2667`), and a program sees the old or the new inner map whole.
- K-4. Every inner map must match the prototype's capacity: `bpf_map_meta_equal` compares type, key size, value size and flags (`kernel/bpf/map_in_map.c:83-92`), and `array_map_meta_equal` adds `max_entries` unless `BPF_F_INNER_MAP` is set (`kernel/bpf/arraymap.c:594-601`). The prototype is therefore created at 16,383, and any fixture that builds an inner array for the production outer map uses 16,383 too.
- Spike evidence for the same facts on 7.0.0-38-generic: the V-6 findings (K-1 to K-3 observed; the fingerprint read ran without XDP attached, so the "XDP never selects slots 16,381 and 16,382" claim is covered by the Tier-3 obligation, not by the spike).

**Probe.** No probe step is added. A root process reading a map by id on the pinned kernel works (K-2, observed in V-6); its failure is not reachable on the appliance. Writing a wrong fingerprint is not probed either: the host/sim equivalence test (Handoff) reads it back on the real kernel.

### D-11. Guest UDP to a VIP-range destination (guest-flow owner, `overdrive-control-plane`)

No new type, variant, port method or wire field. Behaviour of the existing `datagram_associate` once the VIP branch is deleted:

- Precondition: the datagram's destination address lies in one of the configured VIP ranges (the `vip_range` the owner already receives from the ports; it stays).
- Postcondition: the association is refused with the existing `FlowRefusal::Policy`. The refusal precedes the route-type check, the marked-socket creation and `connect_udp`: no host socket is created or connected, and no datagram leaves the host.
- Edge cases: a destination outside the VIP ranges follows the generic path unchanged (the `gateway:53` mark exemption remains). A guest-prefix UDP destination keeps its existing refusal. TCP to a VIP is unchanged (D24a).
- Duration: until GH #310 delivers guest UDP to a VIP. When #310 lands it replaces this refusal.
- Contract shape: pure refusal (return-only); universe of effects = none (no socket, no route lookup).
- Premise and provenance (code, observation, kernel citations): ADR-0298 § Context.
- Kernel and VMM assumptions: none. The refusal needs no kernel behaviour; it removes a socket the kernel would otherwise route.
- Test obligation: Tier 1 only, through the owner's driving port with the sim flow I/O (slice 02 AC7).

### Conformance pins (D-12, APPROVED 2026-11-15)

Answers to the spec's gaps. Each pins an interface contract or states a model-only ruling; test-support shaping is DISTILL's.

**a. Dispatch error.** Settled by ADR-0296 decision 3 (the retry rule). The model follows the code: a tick that emitted an action requeues `Immediate` on every path including `dispatchErr`.

**b. `pending_keys` order.** `BrokerDrain::pending_keys(&self) -> Vec<(ReconcilerName, TargetResource)>` returns the pending keys in first-pending order: ascending by the broker's admission sequence, the order `drain_pending` admits eligible keys. A same-key resubmit does not change a key's position (below). The accessor is read-only, takes no limit or blocked set, and does not filter by eligibility. Implementation note: `BrokerDrain::pending_keys` forwards to a read-only `EvaluationBroker::pending_keys` in `overdrive-core`, the only way to read the broker's private pending map; it adds no write or eligibility surface.

**c. Pass granularity.** `resync_reconciler` (D-1b) is the driving port the conformance driver calls for the model's `PassStep`; the pass is only a loop over it in `BTreeMap` order, so it is covered by the same steps. Reason: enumeration failure is per reconciler, and the convergence loop (a separate task) admits evaluations between two reconcilers' steps, so a whole-pass port cannot replay the interleavings the model checks. Whole-pass replay alone would restrict randomized traces to inter-pass interleavings and leave `enumeration_failure_of_one_reconciler_does_not_starve_others` and `edge_wake_during_burst_is_admitted_after_burst` unreplayable.

**d. Admission, evaluation and outcome injection.**

| Model action | Driving port | Notes |
|---|---|---|
| `Drain` (admission) | `BrokerDrain::drain_pending(limit, blocked_targets, now, now_unix)` with `limit` = cap minus the driver's in-flight count and `blocked_targets` = the driver's in-flight targets | The driver's in-flight set is its stimulus (the cap and per-target exclusion live in the convergence loop, which is not a driving port); it is not projected. |
| `Finish(k, outcome)` | `run_convergence_tick` for the admitted evaluation | One call runs hydrate, reconcile, persist and dispatch; the outcome is chosen by the fault injected at the driven port before the call. |
| edge wake / pass submit | `ReconcilerRuntime::submit_evaluation` / `resync_reconciler` | A dropped edge wake is the driver not calling `submit_evaluation`. |
| crash | the driver drops the runtime and broker and rebuilds the runtime over the same stores, keeping the same `IdentityMgr` (the held SVID set is not reset) | `bulk_load` restores the View, intent and observation survive (WK-5). |
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

**f. `need` (desired != actual) and `has` (managed).** Model-only for projection. They are the driver's stimulus (the world it builds through the intent and observation driven ports) and have no accessor and no projection. The conformance comparison covers the broker state (`pending_keys`, `counters`); whether a needy key converges is the slice 04 oracle (`actual == desired` through the reconciler's read-port), not a conformance projection. The driver builds worlds in which `need` implies `has`, as the model's `NeedImpliesManaged` invariant states, and in which `has` follows the managed-set coupling of the spec (`SharedIntentImpliesBase`): `has(service-lifecycle)`, `has(svid-lifecycle)` and `has(service-map-hydrator)` each imply `has(workload-lifecycle)`; `has(svid-lifecycle)` and `has(service-map-hydrator)` imply `has(service-lifecycle)`; a Service intent with an assigned VIP implies `has(service-map-hydrator)` (D-21; invariant `VipImpliesHydratorManaged`); `has(svid-lifecycle)` holds iff the workload has a Running allocation or a held SVID (D-1a; invariant `K4ManagedIffRunningOrHeld`), so that key can leave the managed set; the other keys' `has` never shrinks. An intent alone makes only `workload-lifecycle` managed (D-18). Only production-reachable `(reconciler, target)` pairs are modelled (D-14); that is what lets the driver replay every trace.

Consequence for `has`: if the driver set `has` equal to what `managed_targets` returns, conformance could not detect an omitted target (the model and production would agree on the omission). Omission is instead covered by a containment obligation DISTILL records for every reconciler: for each world the driver builds, every target whose `hydrate_desired` or `hydrate_actual` projection is non-empty is in the set `managed_targets` returns, with both sides read through the reconciler's own methods and not through the driver's bookkeeping (for `svid-lifecycle`, "projection" excludes `ever_issued`, per the containment note in D-1a). This is the D-1a postcondition made checkable; it adds no production surface.

**g. `inWin` (inside the hydrator backoff window).** Model-only; the conformance `State` does not include it. Reason: the window is `last_failure_seen_at + backoff` over the hydrator's persisted `retries`; a driver that recomputed it from `get_view` plus the backoff function would compare the model with a re-derivation of the same rule, and pinning a new window accessor would add production surface for one check. What retention (D-3) leaves observable is already pinned: after a `dpFail` finish the key is still in `pending_keys` with a `NotBefore` that the broker holds, so the retained key is compared as `pending`. The window arithmetic itself is covered by the unit tests of the hydrator's `next_evaluation_at` (D-3). The Quint invariant `RetentionSchedulesBackedOffKey` and its hazard `wake_noRetention` stay as model checks.

## Wave: DESIGN / [REF] Reuse Analysis

| Existing | Verdict | Justification and contract shape |
|---|---|---|
| `Reconciler` trait, `AnyReconciler` | EXTEND | One required method; the `hydrate_*` precedent. |
| `ServiceLifecycleView` act-once markers and the stop emission | UNCHANGED | D-6. Contract shape: bounded-change; universe = the markers and `StopAllocation` actions of Running allocations; assertion = the existing `probe-count` model checks and unit tests for the probe-driven stop obligation, which is scoped out of the slice 04 drop/skew progress oracle (D-6); the slice 04 property covers every other `service-lifecycle` obligation. |
| `HydrationContext`, `HydrateError` | EXTEND | `managed_targets` reads through the existing bundle. D-20 adds one field (`programmed_services`) and one variant (`DataplaneRead`). Contract shape: pure read; universe = none. |
| `ProgrammedServiceView` | CREATE NEW (D-20) | No existing port reads the programmed map; `SimDataplane::service_backends_for` and `EbpfDataplane::service_map_contains` are inherent test accessors, not a port. A read method on `Dataplane` is rejected (ADR-0299). Contract shape: pure read; universe = none; assertion = the sim/host equivalence test and trait-contract tests (Handoff). The write side is an extension of `Dataplane::update_service` (`EbpfDataplane` and `SimDataplane`): it records the fingerprint of the applied slice in the table it publishes. No trait signature changes; the `Dataplane::update_service` rustdoc gains the fingerprint postcondition and states the IPv6 refusal (D-20 "`update_service` and IPv6 backends"). Contract shape for that extension: bounded-change; universe = the one inner map being built before the swap. The two ports reach the hydrator from one instance through `DataplanePorts` (D-20 "Composition wiring"): pure value; universe = none. |
| `ReconcilerRuntime::submit_evaluation`, broker LWW | EXTEND (unchanged) | Pass submits through it (C-A1 holds). Contract shape: bounded-change; universe = pending map keys; assertion = `pending_keys` and `counters`. |
| `resync_schedule` family, cadence table | DELETE | Superseded; same 30 s as both users. |
| Router periodic relist | DELETE | Subsumed by the pass. `Lagged` relist stays. |
| `spawn_convergence_loop` | EXTEND by deletion of the cadence phase | Admission, cap and drain unchanged. |
| `ServiceHydrationResultRow` + executor row | REUSE (remote path only) | The executor's write is unchanged. The hydrator no longer reads it (D-20). |
| `Dataplane` trait | EXTEND by deletion | Loses `register_local_backend` and `deregister_local_backend` and the error variants only they use (D-9). Contract shape of the remaining operations unchanged. |
| Same-host local-backend path (programs, maps, handles, actions, executors, hydrator partition, sim state) | DELETE | No production producer (ADR-0297). Inventory in slice 02. `cgroup_connect4_gateway` and its maps are kept. |
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

`service-lifecycle` keeps two probe-driven stop markers and is otherwise unchanged by this feature. This is the one place the ruling is stated; other sections point here.

**Provenance (two decisions, two sources).**

- `liveness_last_acted_run`: ADR-0191 decision 5 (user ruling D-1 option (a), #295, 2026-10-13), which decides only "the liveness terminate" marker (ADR-0191 `:109-121`). Defended by the `specs/quint/probe-count/` hazards `streak_bugOncePerStreak` and `streak_bugOnceEver`. "No new row changes nothing" is ADR-0191 decision 6; the not-sampled event (ADR-0192) makes a run that writes no row visible.
- `startup_last_acted_run`: decided by `service-allocation-readiness-and-ending` D8 (approved 2026-11-02, ADR-0260): "a mirror of the liveness retry", an additive `#[serde(default)]` View field, key `(alloc, ProbeIdx::new(0))`, written only when a startup stop is emitted, so a startup stop is retried once per new startup row (`docs/feature/service-allocation-readiness-and-ending/feature-delta.md:95`, contract `:576-604`, field `:593-599`, emission rule `:600-604`). ADR-0191 `:119-121` ("The startup failure is already announced once per allocation ... neither needs this memory") predates D8 and does not cover the startup marker.

A probe-driven `StopAllocation` is emitted at most once per crossed probe row; a failed stop is retried on the next crossed row.

**Why the markers exist, and why that reason holds.** Without a marker a re-read of the same crossed row would emit the stop again on every reconcile pass. The runtime persists the View before dispatch, so the marker is the reconciler's memory of the row it acted on. Each is keyed to an observed input, the probe row's `run_seq`, and written only when a stop is emitted.

**Why they pass the View rule.** The stop decision reads `actual`: the allocation is `Running` and the latest probe row is past its threshold. The marker decides only when to act again. For the same row, a stop already emitted is not re-emitted, even if it was dropped; for a newer crossed row, the decision reopens from `actual` and the stop is emitted.

**Scope in the slice 04 property.** With a dropped probe-driven stop, or a View whose marker equals the latest crossed row's `run_seq`, `service-lifecycle` emits nothing until a newer probe row arrives (ADR-0191 decisions 5 and 6). A drop/skew progress oracle (`actual == desired`, then zero actions) would go red on that by design, so for `service-lifecycle` only, the probe-driven stop obligation is excluded from that oracle; it is bounded by ADR-0191 decisions 5 and 6 and the `specs/quint/probe-count` checks. Its generators draw markers only from rows present (a marker never exceeds the run_seq of a row in the world). Every other `service-lifecycle` obligation (backend membership, readiness, `managed_targets` containment) stays under the oracle.

**`terminal_announced`.** Stamped on a terminal row (`service_lifecycle.rs:549`) and on a startup stop (`:654`); removed at Stable (`:626`); consulted to skip a non-Running allocation (`:563`), to retain the evaluation (`:710`) and to exclude the allocation from liveness actions (`:1029`). It is pacing keyed to observed lifecycle, not a decision on whether a stop is in place. The one dependency is that liveness never runs before Stable: `liveness_begins_at_stable.rs` pins it at the runner (no liveness role or row before the Stable hook), so an allocation holding a startup stop has no liveness row for the exclusion to suppress.

Rejected: emit the stop from `actual` with retry pacing; it reverses ADR-0191 decision 5 and widens #359.

### D-7. Slice 04 property shape

- The property drives the production `run_convergence_tick` over a sim-composed `AppState` (the pattern `workload_gc_absent_intent.rs` uses): hydrate, reconcile, View persist and the real action shim run against `Sim*` ports. No generic sim executor is written. A hand-written executor re-implements the shim and is the "driver compares the model with itself" shape that `testing.md` rejects; the hydrator DST's fabricated `Completed` row is that shape today.
- Faults ("dropped actions") are injected through existing or small driven-port seams: service-map update failure, allocator release failure, `SimViewStore::inject_fsync_failure`, observation read failure, `SimDriver` stop failure. That is `testing.md`'s "drive causes, not fabricated consequences". Starting worlds and Views are arbitrary (including a View that says done while the effect is absent); crash-partial worlds are reachable states.
- Oracle: `actual == desired` through the reconciler's own read-port within N passes, then zero non-`Noop` actions (progress, stability); executor idempotency by dispatching each action twice.
- Production surface the property requires: the resync step between ticks (`resync_reconciler`, D-12) and, for the seams, sim-side injectors DISTILL names. Fixture internals are DISTILL's.
- Lane: DISCUSS AC2 says default lane. A composed `AppState` needs an intent store. `AppState.store` is `Arc<dyn IntentStore>` (`lib.rs:241`, commit `3cef9fb4b`), so a sim-composed `AppState` can take any `IntentStore`, and `lib.rs:521` builds redb with `InMemoryBackend`, which may make a no-file default-lane build possible. If not, the property sits under `integration-tests` and AC2 changes. DISTILL decides (validation item V-5; not contract-shaping).
- Reconcilers.md rule change: see "Proposed rule text".

### D-8. `workflow-lifecycle`

Confirmed: no dedicated wake. It is registered, so it gets a required `managed_targets`; the production set is empty because the registry is empty. The method closes the wake for the day the first production workflow lands as a side effect, not as scope.

## Wave: DESIGN / [REF] Proposed rule text (drafted here; lands in DELIVER)

Not applied now. Timeless discipline only, per the memory ruling on rules docs.

### `reconcilers.md` — replaces the fingerprint-shaped symptom and the "never both" wakeup paragraph

> **A View decides when to act, never whether an effect is in place.** A View field may carry pacing inputs: attempts, last attempt time, a retry budget, a rate limit, or a marker keyed to an observed input (for example the `run_seq` of the probe row a stop was last emitted for) that decides only when to act again. A View field never decides whether an effect is in place. That is decided from `actual`, read from the resource the reconciler manages. A marker keyed to an observed input suppresses re-emission only for that same input: an action already emitted for that input is not emitted again for it, and a newer input reopens the decision from `actual`. If the effect cannot be observed, make it observable first; a result row of the last attempt is a receipt, not the resource.
>
> **Every reconciler declares its managed targets.** The runtime re-evaluates all of them every period, from intent and observation, whatever woke them before. `interests()` and `EnqueueEvaluation` are latency; the pass guarantees re-evaluation, not that a View-gated action is re-emitted. An evaluation that fails to hydrate or persist its View is not requeued locally; the next pass retries it. A dispatch error keeps the tick's requeue; the pass is its backstop. `next_evaluation_at` supplies a precise retry deadline when the reconciler has one.

Symptoms to list: a View field consulted as the diff; a field stamped on emit and consulted to decide whether the effect is in place; a reconciler whose only wake is an edge; a managed-target method that enumerates one observation family only; a test or simulation invariant that seeds the View to represent "converged" instead of asserting on `actual`.

### `development.md` § "Reconciler I/O" (D15)

Replace "an unreadable observation is the runtime's to log-and-skip" with: a hydrate error ends that evaluation with no View write and no dispatch; the runtime logs it and the next resync pass re-evaluates the target. Document `next_evaluation_at` (pure, called only when the tick emitted no non-`Noop` action; `Some` retains the evaluation until that instant) beside `resync_schedule`'s removal and `managed_targets`.

## Wave: DESIGN / [REF] Lifecycle Gate Ownership

State-ownership matrix (existing signals this feature touches):

| Signal or state | Owning component | Promise it makes | Inputs that may gate it | States it must not gate |
|---|---|---|---|---|
| `Running` / `Stable` allocation | `workload-lifecycle` / `service-lifecycle` rows | as in ADR-0087, ADR-0265 | unchanged | any resync, hydrate-read or map-read outcome |
| `service_hydration_results` row | the `DataplaneUpdateService` executor | outcome of the last remote update (a receipt; the hydrator decides whether the map is in place from the programmed map, D-20) | unchanged | none new |

The feature adds and moves no gate. It removes one boot refusal: the owner's `OwnerCgroupPlacement` startup check (D-10), which guarded only the removed datagram rewrite and which no production boot reached, because `run_server` enrols the process into `overdrive.slice/control-plane.slice` before the check runs. The deletion removes a path no production caller reaches; it changes no state meaning. The resync pass is a wake, not a gate: it submits evaluations and decides nothing about any state above.

## Wave: DESIGN / [REF] Formal protocol model

**Required** (changes an ordered, concurrent and crash-sensitive protocol). Written and model-checked by the `quint-modeler`. Directory: `specs/quint/reconciler-evaluation-wake/` (permanent, referenced by DISTILL conformance; its `README.md` states the abstraction). Evidence is recorded only by `cargo xtask quint check --record`. The recorded run (`specs/quint/reconciler-evaluation-wake/evidence/summary.md`) shows 14 of 14 checks matching `expect` (2 properties: `ProgressByPass`, `RetentionSchedulesBackedOffKey`; 2 hazards; 10 named runs; recorded 2026-11-16; Quint 0.32.0, Apalache 0.56.1, Java openjdk 21.0.12.1, kernel 7.0.0-38-generic), and `cargo xtask quint verify-evidence` passes.

The specification models evaluation wake (decisions D-1 and D-3), module `evaluation_wake`. One faithful model serves both checking and conformance replay (D-17); the verified model is the conformance oracle and is not split. Owners: broker (pending FIFO of unique `(reconciler, target)` keys, eligibility `Immediate` or `NotBefore`), convergence loop (cap 8, one in-flight per target), resync pass (stepped one reconciler at a time, D-12), edge wakes, reconciler-to-reconciler handoff wakes, tick outcomes (`ok`, `dpFail`, `hydrateErr`, `persistErr`, `dispatchErr`). Environment faults as actions: each error outcome, enumeration failure for one reconciler, crash between View fsync and dispatch, dropped edge wake, drift.

Keys. Four `(reconciler, target)` keys of one shared workload (a Service with a listener), only production-reachable pairs (D-14). Abstract names sort like the production names, so the pass walks the registered (`BTreeMap`) order `r1S, r2M, r3V, r4W`:

| Model key | Model reconciler | Production reconciler | Target |
|---|---|---|---|
| `k3` | `r1S` | `service-lifecycle` | `t1` = `workload/<id>` |
| `k2` | `r2M` | `service-map-hydrator` | `t2` = `service/<id>` of the workload's own listener |
| `k4` | `r3V` | `svid-lifecycle` | `t1` |
| `k1` | `r4W` | `workload-lifecycle` | `t1` |

`k1`, `k3` and `k4` share `t1`, so the loop's per-target exclusion is exercised. World facts: per key `has` (managed set, from intent and observation) and `need` (desired != actual); plus `vip` (the listener has an assigned VIP) and `held` (an SVID is held for some allocation of the workload); `running` is derived (`has(k1) and not need(k1)`). Two model-only View facts decide whether a View write can fail: `v3` (`service-lifecycle`'s next tick would change its View) and `r4` (`svid-lifecycle`'s stored View holds an `IssueRetry`). `persistErr` is offered only where the tick changes the View, because `persist_view` returns before the durable write when the View is unchanged.

Managed-set coupling (`SharedIntentImpliesBase`): `has(k3)`, `has(k4)` and `has(k2)` each imply `has(k1)`; `has(k4)` and `has(k2)` imply `has(k3)`; a Service intent with an assigned VIP implies `has(k2)` (D-21; `VipImpliesHydratorManaged`); `has(k4)` holds iff the workload has a Running allocation or a held SVID (D-1a; `K4ManagedIffRunningOrHeld`), so `k4` can leave the managed set; the other keys' `has` never shrinks. An intent alone makes only `k1` managed (D-18).

Handoff edges (existing production behaviour, WK-7), each submitted `Immediate` before the tick's own self-requeue:

| When | Handoff | Lands on `ok` |
|---|---|---|
| `k1` acts (allocation-mutating action) | `k3` then `k4` | the SVID is held; `need(k3)`; `has +k3,k4`; `need(k4)` is not set (the following `svid-lifecycle` evaluation is a Noop) |
| `k3` acts with a VIP assigned (`WriteServiceBackendRow`) | `k2` | `need(k2)`, `has +k2` |
| `k3` acts without a VIP (`FinalizeFailed` Stable only) | none | Stable committed |
| `k2`, `k4` act | none | `k4`: SVID held for every Running allocation, held SVID of a non-Running allocation dropped |

A handed-off key that is not needy is admitted and evaluates to a Noop; a submit at an already pending key collapses and counts in `cancelled`.

Backoff windows. `k2` (D-3) and `k4` (existing behaviour) each have a View backoff window and `next_evaluation_at`. `svid-lifecycle` records its `IssueRetry` when `IssueSvid` is emitted, so a failed issue (`dispatchErr` on a Running, unheld allocation) leaves `k4` inside its window; an in-window evaluation emits `Noop` and requeues with a `NotBefore` retention. The `wake_noRetention` hazard switches off the hydrator's retention only. `backoff_for_attempt` is one constant, so `WindowEnds` is one clock advance that ends every open window and makes every retained `NotBefore` eligible.

- Safety (`Safety`, `wake_full`): active within cap; one active per target; pending keys unique; eligibility canonical; need implies managed; backoff window only for needy keys; `SharedIntentImpliesBase`; `WorkloadNeedExcludesSvidIssue` (`need(k1)` and `need(k4)` hold together only through a held SVID to drop). Every conjunct holds by construction of the model and defends no D-1 or D-3 rule, so there is no standalone check; `Safety` is asserted as an `expect` clause in nine of the ten named runs (all but `pass_submit_merges_retained_not_before_to_immediate`). The broker rules' teeth are the `eval_broker` unit tests (V-4) and the conformance driver.
- Progress (`ProgressByPass`, under fairness of the pass timer): every needy key is evaluated again after any error outcome, dropped wake or crash, and an `ok` evaluation clears its need. The bound: progress holds once faults stop (faults are budgeted, `MAXF = 2`; drift, `MAXD = 2`) and an admitted evaluation's dispatch completes. The model abstracts the reconciler: it proves re-evaluation, not that `reconcile` converges `actual` to `desired` (the slice 04 property and each reconciler's tests do). Starvation of a key queued behind a pass burst is covered by the named run `edge_wake_during_burst_is_admitted_after_burst` and the broker's FIFO unit tests (V-4).
- Retention (`RetentionSchedulesBackedOffKey`, `wake_retention`, faults `dpFail` and `dispatchErr`): a key inside its backoff window (the hydrator's, D-3, or `svid-lifecycle`'s existing one) is always scheduled, in flight or retained, so its retry needs neither a row write nor a pass.
- Hazard variants, one per decision, 2 in total: D-1 → `wake_noPass` (remove the pass; dropped wakes disabled; progress is violated, the wedge ADR-0296 names); D-3 → `wake_noRetention` (`next_evaluation_at` returns `None`; retention is violated).
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
| WK-3 | Abstract time: the 30 s period, anchored re-arming and "one period plus queue time" as a duration | A DISTILL obligation on the existing default-lane seeded `overdrive-sim` / `SimClock` harness, driven through the production composition (`run_server` and the convergence loop it spawns) with `BootAdapters.clock` set to the `Arc<SimClock>` the loop already reads (the boot's one clock, which the pass and the interest router also read); no new API. There is no `config.clock` on `ServerConfig`; `BootAdapters.clock` is the injection point and `AppState.clock` is the same instance. The obligation: the resync loop runs a pass at boot and then at `previous + period` with no drift, at most one pass per period, and a fault clearing recovers within one period plus queue time. No new lane. (V-3 was a native-metal cost measurement and does not discharge this.) |
| WK-4 | `managed_targets` returns the whole set or `Err`, from intent and observation | The per-reconciler containment obligation (Conformance pins, f), a DISTILL obligation: every target with a non-empty hydrate projection is in the returned set (`Err` returns no partial set). The conformance driver cannot discharge it, because it sets `has` from the same set. V-5 is a lane-feasibility item and does not discharge it |
| WK-5 | A crash loses exactly the in-memory broker (queue and its `dispatched` / `cancelled` counters) and in-flight set; View, intent and observation survive. The held SVID set is volatile in a real process restart (`svid-lifecycle`'s restart recovery re-issues), but the conformance crash rebuilds the runtime only, so the model does not reset `held` | the `WriteThroughOrdering` DST invariant and a DST crash scenario |
| WK-6 | The only pacing View the model needs is the hydrator's and `svid-lifecycle`'s backoff windows (WK-8); other reconcilers' View pacing never stops a due key from being re-evaluated by a pass (it only turns an evaluation into a Noop) | the slice 04 property |
| WK-7 | The handoff wakes, the landed-action consequences and the world needs are as the cited production sources: which action emits which `EnqueueEvaluation` and in which order; a `k1` start holds its own SVID, so the `svid-lifecycle` evaluation after it is a Noop (neither acts nor requeues); `k3` publishes a row and hands off to the hydrator only with a VIP; an allocation row makes `service-lifecycle` manage the target; `need(k1)` and `need(k4)` exclude each other except through a held SVID to drop | the fidelity probe and the quint-connect replay of the named runs against the real reconcilers |
| WK-8 | The View-change facts: which ticks change which View (`persist_view`'s unchanged-View skip, `service-lifecycle`'s `observed` and `terminal_announced` inserts, `svid-lifecycle`'s `IssueRetry` bump on emit and its removal); the `IssueRetry` is recorded when `IssueSvid` is emitted, so a failed issue leaves the key inside its window; `svid-lifecycle`'s `next_evaluation_at` retains the in-window Noop | the quint-connect replay of the named runs and of the randomized traces against the real reconcilers (the fidelity probe) |

The model checks the pass and edge-wake protocol, not the duration bound (WK-3).

D-20 and the model. The model abstracts `reconcile`: an `ok` evaluation of a needy key clears its need. For `k2` that abstraction holds in every world only because the hydrator reads the programmed map (D-20); with the receipt as `actual` it was false after a reboot. D-20 adds no wake-protocol rule, so it adds no hazard variant and no check. What the modeler changes in the spec is listed under "Handoff to DISTILL".

Pinned observable projections for the conformance drivers (`testing.md` § "Quint conformance"):

| Spec variable | Owner | Accessor | Status |
|---|---|---|---|
| `pending` (keys; order = FIFO) | `BrokerDrain` | `pending_keys(&self) -> Vec<(ReconcilerName, TargetResource)>`, first-pending (admission) order (D-12 b) | NEW, read-only (approved with D-1, 2026-11-13; ordering clause is D-12, approved 2026-11-15) |
| `dispatched`, `cancelled` | `BrokerDrain` | `counters()` | existing (`wake_conf` only) |
| `inWin` (inside a backoff window: the hydrator's, or `svid-lifecycle`'s) | none | model-only (D-12 g): the retained key shows as `pending`; the window arithmetic is covered by the unit tests of each `next_evaluation_at` | ruling |
| `vip`, `held`, `v3`, `r4` | the driver's world | model-only: stimulus and View facts (WK-7, WK-8), not projected | ruling |
| `elig` | broker | model-only | ruling: verified by broker unit tests; the merge run observes eligibility through admission (`pending_keys` and `counters` after `Drain`), the modeler appends that step to `pass_submit_merges_retained_not_before_to_immediate` |
| `active` | convergence loop | model-only (not projected) | ruling: the loop's `active_targets` is local to the task; not exposed. The driver's in-flight set is its stimulus (D-12 d) |
| `has`, `need` | the driver's world | stimulus, not projected (D-12 f); containment of `has` in `managed_targets` is a DISTILL obligation, not a conformance comparison | no accessor |
| `due`, `passPos`, `faults`, `drifts` | none | model-only (timer and budgets) | ruling |

Action mapping (D-12 c, d): `PassStep` → `resync_reconciler`; edge wake → `ReconcilerRuntime::submit_evaluation`; `Drain` → `BrokerDrain::drain_pending`; `Finish(k, outcome)` → `run_convergence_tick` with the outcome's fault injected at its driven port; crash → rebuild the runtime over the same stores, keeping the same `IdentityMgr`; handoff wakes are not driver actions (production's own `EnqueueEvaluation` dispatch produces them inside `Finish`); `Tick` is the period timer and is model-only (the driver calls `resync_reconciler` directly). `WindowEnds` is time passing: the driver advances the injected `SimClock` (the boot's one clock, `BootAdapters.clock`, which the convergence loop and the hydrator's `TickContext` read) past the hydrator's backoff deadline `last_failure_seen_at + backoff`, which is also the retained `NotBefore` of `next_evaluation_at` (D-3). The advance is visible through the existing `drain_pending(.., now, ..)` argument: a retained `NotBefore` key is admitted once `now` reaches its deadline. No new surface. Production has one instant for the end of a backoff window and for the eligibility of the retained `NotBefore`, so the spec has the single action `WindowEnds`, which ends every open window and makes every retained `NotBefore` eligible. `Sim*` adapters inject faults at driven ports and are the arbiter for no variable. The step record `conf_step` (variant per action name) is the spec's, not production's.

## Wave: DESIGN / [REF] Validation items

Contract-shaping items run before approval; an item is marked run only when a findings file is cited (V-6).

| ID | Item | Class | Probe to run | Falsification changes |
|---|---|---|---|---|
| V-3 | Cost: per-evaluation hydrate time and admission delay of an edge wake queued behind a burst at 1000 workloads | not contract-shaping (pacing is internal) | RUN 2026-11-15, native metal, commit `97a417fcb`. Evidence: `spike-scratch/reconciler-resync-cost/evidence/full-20261115T180650Z/results.json` (proposed pass) and `baseline-20261115T211313Z/results.json` (baseline). | No contract changes. The measurement is the cost of the pass against today's baseline on the same host and load; it does not choose the mechanism (D-1 rests on source reading and ADR-0296's rejected alternatives). G1 figures (pass; baseline in brackets): W=100: E 265 [337], 0.29% duty [0.31%]; W=500: E 1225 [1537], 4.19% [4.51%]; W=1000: E 2425 [3037], 15.2% [15.5%]. Not above the baseline at any N. Scan cost is tracked by #270, deploy latency during a pass by #357. |
| V-4 | Same-key collapse and `NotBefore`→`Immediate` merge under pass plus edge submits | not contract-shaping | existing `eval_broker` tests plus the new run `edge_wake_during_burst_is_admitted_after_burst` | a test |
| V-5 | Default-lane feasibility of the composed property (in-memory redb) | not contract-shaping | DISTILL builds one composed tick in the default lane. `AppState.store` is `Arc<dyn IntentStore>` (`lib.rs:241`), so a sim-composed `AppState` can take any `IntentStore`. | the lane of one test |
| V-6 | D-20 host read: feasibility and cost of reading the programmed state of one frontend from the kernel (K-1, K-2) | **contract-shaping, RUN 2026-11-17** | Evidence: `docs/feature/reconciler-convergence-guarantees/spike/findings-programmed-service-map-read.md` (probe `spike-scratch/programmed-service-map-read/`), kernel `7.0.0-38-generic`, root, a real `EbpfDataplane` over a private pin directory. Verdict WORKS. The full-set read (outer lookup, `GET_FD_BY_ID`, batch scan, `BACKEND_MAP`) was exact only at uniform weight and cost 5.41 ms p50 per frontend, so three mechanisms were measured on the same host at 1,000 frontends x 16 backends: batch scan 5.41 ms, mmap 107 µs plus `BACKEND_MAP` lookups, fingerprint in two extra inner slots 4.7 µs (sweep 6.2 ms). The user approved the fingerprint (D-20 read mechanism). Kernel citations K-1 to K-4: see D-20. Reads under the apply-commit guard showed 0 torn reads; unguarded reads tore in 63% of 2,734 reads. A fresh `EbpfDataplane` over the same pin directory read empty. | The fingerprint's one unobserved fact is that XDP never selects slots 16,381 and 16,382 once the inner capacity is 16,383 (the probe ran no XDP program). It is checkable, not contract-shaping: a failure would be a Tier-3 test failure on the existing dataplane tests (Handoff item 3) and would change the slot layout, not the port. |

D-1 stands on source reading. The deletion (D-9) adds no kernel assumption. No contract-shaping item is open: V-6 (D-20) ran on 2026-11-17.

### Validation budget

- Model checks: 14 (4 exhaustive TLC on `wake_full` / `wake_retention` / hazards, 10 evaluator runs), recorded 2026-11-16 in `specs/quint/reconciler-evaluation-wake/evidence/summary.md` (Linux 7.0.0-38-generic, Quint 0.32.0, Apalache 0.56.1, Java 21.0.12.1, 16 jobs): about 90 min wall for the whole run. `ProgressByPass` (`wake_full`, 40 GiB heap, 12 workers) takes 4794.2 s (about 80 min, 6.8 GiB peak); `RetentionSchedulesBackedOffKey` (`wake_retention`, 12 GiB heap, 4 workers) takes 634.0 s (about 10.5 min, 3.0 GiB peak); each hazard takes 12 to 13 s; each named run takes 2.5 to 4.1 s.
- CI lane (`ci = true` in `checks.toml`): 12 of 14 (the two hazards and the 10 named runs). `ProgressByPass` and `RetentionSchedulesBackedOffKey` are `ci = false`: `design.md`'s two-minute rule decides `ci` only, and growth of the validation budget with no time cap is accepted (D-19). Further growth beyond these numbers needs user approval.
- Test lanes added by the design: none. Slice 01 has no Tier-3 scenario (user ruling 2026-11-16); slice 04's property uses the existing default lane (its lane is V-5); one Tier-1 acceptance scenario for D-11; one Quint conformance driver (one specification) under `integration-tests` per `testing.md`. No new CI job, tool, or xtask subcommand.
- Runtime budget: one evaluation per managed target per 30 s; measured per-pass cost is in V-3, the guardrail in G1. D-20 adds the programmed-map read to each hydrator evaluation: 4.7 µs p50 and 6.3 µs p99 per service, about 6.2 ms for a pass over 1,000 services (V-6, kernel 7.0.0-38-generic), under the apply-commit guard for read. It adds no wall time worth a budget line to the V-3 pass cost. The write side adds two `u32` array updates to each `update_service` (apply p50 13.5 ms measured against 17.2 ms without, so no growth). The extra slots add 8 bytes to a 65,524-byte inner array.

## Wave: DESIGN / [REF] Quality attributes and enforcement

- Reliability: every wake gap in the verification closes by the pass re-evaluating the target; recovery bound = period plus queue time. The pass guarantees re-evaluation, not convergence: the model proves the former (an `ok` evaluation clears need, by abstraction); that `reconcile` converges is the slice 04 property. The probe-driven stop is unchanged (D-6).
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
| ADR-0053 (Accepted 2026-05-23): same-host backend delivery via `cgroup_sock_addr` | Superseded by ADR-0297 (D-9): the path has no production producer and is removed. |
| ADR-0164: "Datagrams to a VIP use `connect4`. The owner always connects its host datagram socket, so the existing ADR-0053 `connect4` rewrite resolves a VIP to a platform-chosen backend." and "The owner refuses to start unless its own cgroup is the `connect4` attach path or below it." | Superseded by ADR-0298 (D-10, D-11). Guest UDP to a VIP is refused until GH #310. ADR-0164's TCP decision (D24a) is unchanged. |
| `development.md`: "an unreadable observation is the runtime's to log-and-skip" | A hydrate error ends the evaluation; the next pass retries (D15). |
| ADR-0042 § 3 hydration table: "`actual.actual` \| `service_hydration_results` row for the target `ServiceId`" | The hydrator's `actual` is the programmed fingerprint read through `ProgrammedServiceView` from the resource itself (D-20, ADR-0299). The result row stays an executor receipt. |

## Wave: DESIGN / [REF] D-13. The persisted `ConflictRoute` tag after the cgroup route is deleted (APPROVED 2026-11-15, option A)

**Ruling (2026-11-15).** Option A: remove `ConflictRoute::Cgroup` and `WriteRoute::Cgroup` in place; `ConflictRoute` stays a one-variant enum, both route fields stay in the row, no new envelope version. The user's reason: greenfield project, single-cut migrations, no compatibility versions.

- **Context.** The `reconcile_conflict` observation row (`ReconcileConflictRowV1`, rkyv, redb) carries `first_route` and `second_route` of type `ConflictRoute { Xdp, Cgroup }` (`overdrive-core/src/traits/observation_store.rs`). A `Cgroup` route exists only for `RegisterLocalBackend` / `DeregisterLocalBackend`, which slice 02 deletes, so no production store holds a row with that tag.
- **Check.** The V1 golden fixture (`tests/schema_evolution/reconcile_conflict_row.rs`) uses `Xdp` (discriminant 0). It must still decode, unedited (slice 02 AC5). If it fails, the crafter stops and surfaces it to the user. A new envelope version is never minted silently.

## Wave: DESIGN / [REF] Scope limitations and pending items

- Scope limitation, tracked: the 30 s resync period is a compile-time constant with no configuration surface (D-2). Tracked by #356.
- Scope limitation, tracked: guest UDP to a VIP is refused until GH #310 (D-10, D-11; ADR-0298). #310 is cited as the user's ruling names it; not re-fetched. A comment on #310 records the removal.
- Scope limitation, tracked: the Tier-1 `persist_view` recovery (slice 01) assumes the store can write again after the fault clears. With redb a transient disk error is permanent until restart (the `Database` is poisoned and `RedbViewStore` never reopens). Tracked by #360.
- Scope limitation, tracked: a programmed service whose `service_backends` row is gone is not purged by the hydrator (no `desired`); teardown belongs to #211 (D-20, D-21). Scope per D-5; not re-fetched this session.
- Lane for the slice 04 property (V-5).
- Independent design review has not run; the ADRs stay `Proposed`.
- DELIVER follow-ups outside `crates/` (rules docs, living docs, website): slice 02 § "DELIVER follow-ups outside `crates/`" only.
- Finalization obligation (`finalize.md`): this design used native-metal runs (V-3, the guest UDP probe, the Quint evidence records), and DELIVER and DISTILL runs will add more. After the evidence the finalization record needs has been pulled back (`cargo xtask metal pull`; the Quint evidence under `specs/quint/reconciler-evaluation-wake/evidence` and the `spike-scratch` evidence directories are the known cases), FINALIZE cleans up this feature's working directory on the Metal box (the remote checkout and the build and scratch output its runs left) and records the cleanup in the finalization record.

## Wave: DESIGN / [REF] Handoff to DISTILL (after the Quint dispatch and independent design review)

User approvals are recorded above. V-6 has run (D-20). DISTILL starts when independent design review is done.

Scenario obligations: the slice 01 invariants (three faults plus fresh deploy, enumeration failure, burst admission); slice 02 (the deletion: AC1 to AC7 of the slice file, unchanged behaviour of the remote service path and the gateway; plus the one added behaviour, D-11, Tier 1 only, through the owner's driving port with the sim flow I/O); slice 04 (the composed property, mutation check per injected latch mutant, and the executor doc fix, which has no scenario; `service-lifecycle` is covered by the property like every reconciler, except that its probe-driven stop obligation is scoped out of the drop/skew progress oracle and its generators are constrained as D-6 states). The Quint conformance driver per the pinned projections. The liveness-before-Stable dependency of `terminal_announced` (D-6) is pinned by `crates/overdrive-control-plane/tests/acceptance/liveness_begins_at_stable.rs` (`liveness_supervision_begins_when_the_allocation_becomes_stable`, `first_liveness_row_lands_one_interval_after_stable_with_streak_one`). It asserts at the probe runner that no liveness role is live and no liveness row exists before the Stable hook; it does not run `ServiceLifecycle`. Gap: no test runs `ServiceLifecycle` against a non-Stable allocation and asserts it emits no liveness stop; the dependency rests on the runner never writing a liveness row before Stable. The per-reconciler containment obligation for `managed_targets` (Conformance pins, f). Each slice stays production-drivable through the real `run_server` composition, with faults injected at `Sim*` port seams (D11). The VIP-release scenarios belong to #211 and are not in this feature.

DISTILL rulings (D-14 to D-19): the Quint conformance driver replays the spec's production-reachable keys only (D-14), with the handoff wakes, `svid-lifecycle`'s window and the unchanged-View persist skip modelled (D-17). The pinned surface is scaffolded with inert bodies (D-15): `Reconciler::managed_targets` returns an empty set, `resync_reconciler` returns `Ok(0)` and is not wired, `BrokerDrain::pending_keys` returns an empty `Vec`; the tests that depend on them are pending RED on semantic assertions and DELIVER replaces the bodies. The slice 04 property is a `#[cfg(test)]` module at `crates/overdrive-control-plane/src/convergence_property/` so it can call the crate-private `resync_reconciler`; there is no `#[doc(hidden)]` test seam (D-16). `service-lifecycle`'s managed set follows D-18 and the containment obligation reads it that way.

D-20 and D-21 add these DISTILL obligations (each is the pinned contract made checkable; none adds production surface beyond the D-20 port):

1. **Hydrator property, reboot world.** `hydrator::the_hydrator_converges_from_any_state_despite_dropped_updates_then_stays_converged` keeps its world "`Completed` over the desired set while the dataplane is empty". The oracle reads the programmed fingerprint through `ProgrammedServiceView` (not the result row): the converged state is `programmed_fingerprint(f) == Some(fingerprint(vip, survivors))` (or `None` for an empty survivor set), which means the dataplane carries the desired set. The world goes green only through the new read; it must not pass by seeding or invalidating the result row. Update the reboot world's construction: the "dataplane is empty" half is a fresh `SimDataplane` (a serve restart or reboot reads `None` for every frontend), not an emptied result row, and the dispatch the hydrator makes must be observed as a new fingerprint in the programmed map. Add a "map lost while running" world (a programmed service whose maps are then emptied through a Sim test-support seam DISTILL names). Its RED text names the read (for example `service-map-hydrator did not reprogram an empty dataplane`), so the RED is distinguishable from the other worlds' stall text.
2. **Trait-contract tests for `programmed_fingerprint`** over both adapters, one per clause: read after `update_service(f, B)` equals `Some(fingerprint(&f.vip, &B))`; a changed weight or health flag in `B` changes the read; after `update_service(f, [])` it is `None`; a never-programmed frontend is `Ok(None)`; two protocols or ports on one VIP are independent; a refused update (an IPv6 backend) leaves the prior read unchanged; two reads without a write are equal; a failed read is `Err`, never `Ok(None)` (Sim only: a scripted read failure; the host has no test, see "D-20 rulings").
3. **Host/sim equivalence and Tier-3 observation.** In `crates/overdrive-dataplane/tests/integration/` (a guest placement, `testing.md` § "Test placement"), drive `EbpfDataplane` and `SimDataplane` through one call sequence (apply `B1`, apply `B2`, a weight-only change, withdraw, apply on a second protocol of the same VIP, a mixed IPv4/IPv6 set that is refused) and assert identical `programmed_fingerprint` after each step; the kernel observation is the real `SERVICE_MAP` outer slot and its inner map's slots 16,381 and 16,382, read through the port. Add the restart case: construct a second `EbpfDataplane` over the same pin directory after programming (`new_with_pin_dir` recreates the maps) and assert every frontend reads `None`; re-programming then reads the fingerprint again. Tier 3 proves the host read agrees with the Sim; it is the pairing for any Sim read in a conformance driver.
3a. **Inner capacity 16,383 and XDP slot selection (Tier 3).** The existing XDP dataplane integration tests (`atomic_swap`, `service_map_forward`, `reverse_nat_e2e`, `sanity_mixed_batch`, `maglev_real` and the rest of `crates/overdrive-dataplane/tests/integration/`, plus the `overdrive-bpf` Tier-2 `xdp_service_map_lookup` fixtures where they build an inner array for the production outer map) pass with the inner capacity at 16,383: a fixture that builds an inner map for the production outer map uses the same capacity, because the kernel rejects a mismatch (K-4). One added assertion: with the fingerprint slots set to a sentinel `BackendId` that no backend owns, no packet to the frontend is steered to it across a flow set large enough to cover every table slot, so XDP never selects slots 16,381 and 16,382. Verifier-budget baselines (`cargo verifier-regress`) are unchanged: the program does not change. The 16,381 modulus constant stays the Maglev table size; the 16,383 capacity is a separate constant (the lockstep comments that name one value for both are DELIVER's to correct).
4. **Hydrator unit tests (default lane)**, authored by the acceptance designer at the start of the DELIVER step that changes `ServiceMapHydratorState.actual` to `BTreeMap<ServiceId, BackendSetFingerprint>` (see "D-20 rulings"). `in_place` for an equal fingerprint, for an empty survivor set against an absent `actual` entry, for a differing fingerprint (including a weight-only or health-only change) and for an IPv6 survivor (never in place); a dispatch inside the backoff window is suppressed and the key is retained by `next_evaluation_at`; a changed desired set dispatches at once; an in-place observation removes the retry entry; no `should_dispatch` arm reads a result row. The hydrator `next_evaluation_at` obligation (D-3, mutation M3) ranges over services that are not in place.
5. **Managed-target containment for the hydrator** (Conformance pins, f): `service_backends`-row services and intent-derived services are in the set; a service with only a `service_hydration_results` row is not, and the test records that this is the deletion case (#211).
6. **Mutation obligations for DELIVER:** `in_place` always true; `in_place` comparing a fingerprint that omits `healthy` or `weight`; the dataplane writing the fingerprint after the outer swap, or not writing it on an update; the host read recombining the two words in the wrong order; `hydrate_actual` returning an empty `actual`; the retry entry never removed. The mutant "the host read returns `Ok(None)` on a failed read" is an accepted survivor (see "D-20 rulings").

D-20 rulings (the composition wiring and the IPv6 contract are pinned in D-20 above):

- (a) The hydrator unit tests of item 4 (the fingerprint diff over `actual` as `BTreeMap<ServiceId, BackendSetFingerprint>`) are authored by the acceptance designer at the start of the DELIVER step that changes that type. The type change rewrites the receipt arms of `reconcile`, so it alters behaviour and is not scaffolded in DISTILL.
- (b) No test obligation exists for a failed host read (`ENOENT` on a freed inner map id). The apply-commit guard prevents it, and the appliance cannot produce it. The mutant "the host read returns `Ok(None)` on a failed read" is therefore an accepted survivor, for that reason. The "a failed read is `Err`" clause stays tested on the sim.

Quint change required of the modeler (spec edit is not done here): (a) the `k2` drift action and README row for `k2` change from "a backend row the result row does not cover" to "a backend row whose fingerprint the programmed map does not carry, or the programmed map is lost (a serve restart or reboot): `need(k2) := has(k2)`"; the driver maps it to a backend-row write or to a fresh `SimDataplane`. The read mechanism (fingerprint in the resource) changes no modelled fact: the model sees only "the programmed map covers the key's backend row or does not"; (b) a new assumption WK-9 (id by the modeler): an `ok` evaluation of `k2` clears `need(k2)` in every world, because `actual` is read from the programmed map (D-20), discharged by item 1 and item 3 above (the programmed fingerprint is read from the resource, so a lost or recreated map reads as no fingerprint); (c) README and spec comments that describe the hydrator's `actual` as the result row (the `dpFail` row "no (result row)" and any "result row covers" wording) say programmed map; (d) no new action, invariant, hazard or named run: `dpFail` (the update failing) is unchanged, `need` is already a model variable, and a hazard for D-20 would need the receipt modelled as a second abstraction in a model whose `ProgressByPass` already costs about 80 minutes. The `D-21` managed-set narrowing adds the coupling "a Service intent with an assigned VIP implies `has(k2)`" to the model (`VipImpliesHydratorManaged`); `has(k2)` remains a stimulus.

Test conventions DISTILL applies (current `testing.md`; DISTILL picks per test):

- Pending tests: a scenario whose owner API exists but whose DELIVER step has not landed is `#[should_panic(expected = "<semantic RED>")]`, with DISTILL naming the semantic RED per scenario. `#[ignore]` only for a missing external resource. The Quint conformance driver follows the same rule: if the owner's driving port or pinned accessor (`resync_reconciler`, `BrokerDrain::pending_keys`) does not exist when the driver is written, the file may be gated out only with `#![cfg(any())]` carrying `// conformance-lint: pending-owner-api <the missing port or accessor, and the step that adds it>`, removed in the step that lands the port; otherwise the driver stays compiled and pending by `should_panic`. No other always-false `cfg`.
- Placement: every new test declares its placement on itself (`#[overdrive_test(...)]`, `placement!(...)`, `shared_guest!(...)`, `overdrive_test_default!(...)`), checked by `cargo xtask placement-lint`. Pure sim or core tests are `host_pure`. A test that boots `run_server` / `serve`, or composes `AppState` with real kernel effects, is never host-pure: it takes the guest group `testing.md` names for such a body. The slice 04 property over a sim-composed `AppState` with no real kernel effect is `host_pure` if V-5 puts it in the default lane. The D-11 Tier-1 scenario and the Quint driver run over `Sim*` ports and are `host_pure`.

Proposals for the user (not adopted; each adds scope, per `design.md` § "No unrequested scope"): none.
