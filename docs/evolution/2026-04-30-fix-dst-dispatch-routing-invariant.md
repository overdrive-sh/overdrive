# fix-dst-dispatch-routing-invariant — Feature Evolution

**Feature ID**: fix-dst-dispatch-routing-invariant
**Type**: Bug fix (`/nw-bugfix` → `/nw-deliver`) — missing test coverage at the DST tier
**Branch**: `marcus-sa/phase1-first-workload`
**Date**: 2026-04-30
**Commits**:
- `37dcf70` — `test(sim): add DST invariant pinning §8 dispatch-routing contract` (Step 01-01 — single cohesive commit; new `Invariant::DispatchRoutingIsNameRestricted` variant + 4-branch evaluator + `Evaluation` mirror + `DispatchRecord` snapshot + `drive_dispatch_routing` harness helper + 8 inline witnesses + happy/negative end-to-end tests).

**Status**: Delivered.

**Predecessor**: `docs/evolution/2026-04-30-fix-eval-reconciler-discarded.md` (commit `e6f5e5e`). That fix closed the §8 storm-proofing dispatch-routing contract at the **unit/acceptance tier** (`runtime_convergence_loop.rs::eval_dispatch_runs_only_the_named_reconciler`). The precursor RCA's "Out-of-scope follow-up" section explicitly named the matching DST-tier gap; this delivery closes it.

---

## Defect

The §8 storm-proofing dispatch-routing contract — *"a drained `Evaluation { reconciler: R, target: T }` MUST dispatch only the named reconciler R against T, not fan out across the registry"* — had a unit/acceptance pin (commit `e6f5e5e`) but no DST-side pin. The DST invariant catalogue at `crates/overdrive-sim/src/invariants/mod.rs:30-100` enumerated 14 invariants; two covered the broker boundary (`DuplicateEvaluationsCollapse`, `BrokerDrainOrderIsDeterministic`); none covered the dispatcher boundary. The §8 contract has two halves — entry collapse (broker side) and dispatch routing (dispatcher side) — and the catalogue covered one. A future regression that re-introduced dispatch fan-out would have been caught only by the acceptance suite's fixed timeline, not by the deterministic simulation harness that whitepaper §21 designates as the project's primary safety net for concurrency / timing / partition bugs.

This was *not* a defect in production code. The defect was **missing test coverage at the DST tier** — a structural asymmetry in the invariant catalogue that left the §8 dispatch-routing contract exposed under any failure mode the acceptance suite's fixed timeline could not reach (tick-cadence regressions, batched-drain optimisations, fault-recovery branches, drain-order dependent dispatch, partial shutdowns deregistering a reconciler between drain and dispatch, timing races on stale registry snapshots).

## Root cause

The DST reconciler-primitive invariants (`AtLeastOneReconcilerRegistered`, `DuplicateEvaluationsCollapse`, `ReconcilerIsPure`) landed at step 04-05 when the registry contained one reconciler (`NoopHeartbeat`) and the broker keyed on `(name, target)` for forward-compatibility. Three invariants were authored for three properties of a singleton-registered, broker-keyed, trivially-routed reconciler. Dispatch routing was not load-bearing: the registry was a singleton, so any drained eval would unambiguously dispatch the only reconciler regardless of whether the dispatcher consulted `eval.reconciler`.

When `JobLifecycle` joined the registry at commit `8f4aaa7`, the dispatch-routing contract became load-bearing — but the catalogue was not extended. The precursor RCA `fix-eval-reconciler-discarded` identified this exact gap (`bugfix-rca.md:290-292` named the missing invariant, the file it would live in, and the property it would assert) and **deliberately deferred** closure to a separate work item rather than bundling — correct on scope grounds but tracked only in feature-doc references, not on a scheduled queue. The catalogue therefore remained sized to the single-reconciler era for the four-day window between the precursor's close and this delivery.

The five-whys terminus (per the preserved RCA): **the DST invariant catalogue was designed during a single-reconciler era when dispatch routing was trivially correct; when the registry grew to two reconcilers the catalogue was not extended; the gap was later identified by the precursor RCA but deliberately deferred for scope reasons; the deferral was tracked only in feature-doc references, not as a scheduled work item, leaving the catalogue structurally asymmetric — §8 entry-collapse covered, §8 dispatch-routing not — across the full lifetime of Phase 1's two-reconciler registry.**

A secondary contributing factor: the harness's `harness_registered_reconcilers` literal `1` at `harness.rs:496` was structurally divergent from production (which registers two reconcilers today) but invisible because `AtLeastOneReconcilerRegistered`'s predicate `>= 1` passes under both values. The dispatch-routing invariant's absence is the more consequential expression of the same staleness.

## Fix — RCA Option A1

**Add a DST invariant `DispatchRoutingIsNameRestricted` that pins the contract: for any drained `Evaluation { reconciler: R, target: T }`, exactly one reconciler — R — runs through the dispatch path against T per tick.** Single cohesive commit, all 4 files in `overdrive-sim`, no production-code changes.

1. **Catalogue addition** — `crates/overdrive-sim/src/invariants/mod.rs`: new `Invariant::DispatchRoutingIsNameRestricted` enum variant; canonical kebab name `"dispatch-routing-is-name-restricted"`; added to `Invariant::ALL` and `Invariant::as_canonical` alongside the other reconciler-primitive invariants.
2. **Evaluator** — `crates/overdrive-sim/src/invariants/evaluators.rs`: new `Evaluation` mirror struct (parallel to the `BrokerCountersSnapshot` precedent — sim crate stays a leaf adapter, no `overdrive-control-plane` import), new `DispatchRecord` snapshot struct, and `evaluate_dispatch_routing_is_name_restricted` function with four branches:
   - **Vacuous-pass** on empty input (∀ ∅ holds trivially; without this the empty-input case would misreport under cardinality).
   - **Cardinality check**: `record.dispatched.len() == submitted.len()` — surplus is a fan-out regression, deficit is a missed dispatch.
   - **Per-eval routing check**: every submitted `(R, T)` matches exactly one dispatched entry; permutation-invariant on the target axis (drain order is owned by `BrokerDrainOrderIsDeterministic`, not this invariant).
   - **Smoking-gun check**: no dispatch entry names a reconciler outside the submitted set — catches the precise bug shape the precursor closed (a registry-iteration regression dispatching `noop-heartbeat` against a `job-lifecycle`-only submission).
3. **Harness wiring** — `crates/overdrive-sim/src/harness.rs`: new `drive_dispatch_routing` helper following the `drive_broker_collapse_multi_key` pattern; mirrors `lib.rs:465-481`'s post-fix shape (drain → for eval in pending → dispatch named reconciler against named target) without depending on `overdrive-control-plane`. New dispatch arm in `Harness::evaluate`'s match.
4. **End-to-end tests** — `crates/overdrive-sim/tests/invariant_evaluators.rs`: happy and negative tests driving the new invariant through the harness, asserting on result name + status (matching the existing `EntropyDeterminismUnderReseed` shape).
5. **Inline witnesses** — 8 library-level unit tests in `evaluators.rs` covering each branch + edge case (clean single-eval, clean multi-eval distinct targets, vacuous empty, cardinality mismatch extra, cardinality mismatch missing, smoking-gun unsubmitted reconciler, wrong-routing named reconciler not dispatched).

**Rejected: RCA Option A2** (add a `DispatchRecorder` trait to `overdrive-core` and route both production and the harness through it). Dispatch routing is not a nondeterminism boundary — it is a deterministic *contract*. Adding a trait surface for it would inflate the production hot path's argument list to satisfy a test that has a cheaper closure via the harness mirror. Option A1 matches the established broker-mirror convention (`harness.rs:511-544` / `:588-640`), preserves the dep graph (sim crate stays `adapter-sim`-class with no inverted edge to `overdrive-control-plane`), lands ~100 lines total, and pairs cleanly with the sibling `DuplicateEvaluationsCollapse` invariant — together the two cover the §8 storm-proofing surface end-to-end.

## Verification

- **Regression test (DST tier)** — `dispatch-routing-is-name-restricted` runs on every `cargo xtask dst` pass. Pre-fix: the invariant did not exist; the §8 dispatch-routing contract had no DST-tier witness. Post-fix: 15 invariants pass including the new one; the four-branch evaluator catches every fan-out / wrong-routing / cardinality regression shape.
- **Prior regression preserved** — the unit/acceptance pin `eval_dispatch_runs_only_the_named_reconciler` (commit `e6f5e5e`) continues to pass. Together with the new DST invariant, the §8 storm-proofing dispatch-routing contract is now witnessed at three independent boundaries: real production through fixed-timeline acceptance pin, harness mirror through DST pin, and broker through counter pin. Any single regression that slips one layer faces two more.
- **Mutation gate** — 100% kill rate (2/2 caught). The low mutation count is explained by diff-scope resolution; the 8 inline witnesses + the end-to-end happy/negative tests exercise all four evaluator branches per the reviewer's dimension-5 analysis.
- **Reviewer (nw-software-crafter-reviewer) verdict** — APPROVED with zero required changes; zero testing-theater patterns detected across all 9 review dimensions.
- **Quality gates green** — `cargo nextest run --workspace` (544/544), `cargo xtask dst` (15 invariants pass), `cargo xtask dst-lint`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --doc -p overdrive-sim`, `cargo nextest run --workspace --features integration-tests --no-run` (macOS typecheck).
- **DES integrity** — `verify_deliver_integrity` exit 0; the single step has a complete DES trace.

## Lessons learned

- **Two-half contracts need two-half coverage.** The §8 storm-proofing contract has two halves — entry collapse (broker side) and dispatch routing (dispatcher side). When a contract decomposes structurally, the invariant catalogue must mirror the decomposition. The catalogue's blind spot here was visible at design time: `DuplicateEvaluationsCollapse` inspects `BrokerCountersSnapshot { queued, cancelled, dispatched }` — the counters do not record *which* reconciler each dispatch named, so a bug that swapped the dispatched reconciler would move the counters identically and produce an identical invariant snapshot. The observation seam was structurally blind to the dispatch-routing failure mode. A review of the catalogue at the moment `JobLifecycle` joined the registry would have caught this; the dep-graph constraint (`adapter-sim` cannot depend on `adapter-host`) was a workaround surface, not a coverage excuse.
- **Named-future-feature deferrals need a queue, not just a document.** The precursor RCA at `bugfix-rca.md:290-292` and the evolution doc at `2026-04-30-fix-eval-reconciler-discarded.md:71-73` both explicitly identified this gap and explicitly deferred its closure. Three independent deferral records existed; none scheduled the closure. The "tracked as a future fix-* feature" wording was a promise the project's process did not enforce. The procedural lesson is structural: deferrals to a separate work item need to land on a scheduled queue at the moment of deferral, not just on documents that the next agent has to discover by reading wave-decisions.
- **Harness staleness compounds silently.** The literal `1` at `harness.rs:496` was structurally divergent from production for the entire lifetime of Phase 1's two-reconciler registry, but invisible because the only invariant consulting the count had a `>= 1` predicate. The harness's mental model of the runtime had not been re-synced with production since step 04-05. A periodic harness-fidelity audit — even a five-minute sweep across hard-coded literals — would surface this class of drift before it becomes load-bearing.
- **Mirror discipline transfers across primitives.** The `drive_broker_collapse` / `drive_broker_collapse_multi_key` pattern was designed for one specific seam (broker entry collapse); the dispatch-routing fix shows the pattern transfers cleanly to a peer seam (dispatcher routing). The shape of the fix — minimal data-only handoff at the evaluator boundary, harness mirror that names the production code path it mirrors, evolution-doc note tying mirror-vs-production drift to the unit/acceptance test that covers the production side — is reusable for any future DST invariant whose observation seam crosses the `adapter-sim` ↔ `adapter-host` boundary.

## Out-of-scope follow-up

Per RCA §Out of scope, the following are explicitly tracked separately rather than rolled into this delivery:

- **Multi-reconciler ticks (Phase 2+).** When the broker dispatches a batch in one tick (today: one drained eval per `drain_pending` iteration), the invariant must accumulate `DispatchRecord` across the batch. The current single-tick single-reconciler-target shape is sufficient for Phase 1.
- **Multi-region scenarios.** Cross-Corrosion-peer dispatch — when a regional control plane drains an eval routed from another region — has its own dispatch-routing contract that involves the §4 per-region Raft boundary. Phase 2+ concern; would need its own invariant + observation seam.
- **Workflow-primitive dispatch routing.** Workflows (whitepaper §18 peer primitive to reconcilers) have a different lifecycle and a different invariant family (replay-equivalence + bounded progress). The dispatch-routing contract does not transfer; workflows have their own equivalent under `ReplayEquivalentEmptyWorkflow` (already in the catalogue) and future workflow-replay invariants.
- **Refreshing `harness_registered_reconcilers` literal `1` at `harness.rs:496`.** Now stale (production has two reconcilers registered); `AtLeastOneReconcilerRegistered` passes under both `1` and `2`, so the staleness is invisible. Track separately if harness fidelity becomes a concern; not blocking for this work item.
- **Generalised invariants crate (ADR-0017).** ADR-0017 proposes an `overdrive-invariants` crate with a first-class invariant taxonomy. The new invariant fits the existing per-function evaluator pattern without requiring ADR-0017 to land first; if and when ADR-0017 ships, this invariant migrates alongside its peers with no contract change.

## References

- RCA: `docs/feature/fix-dst-dispatch-routing-invariant/deliver/bugfix-rca.md` (preserved in feature workspace; user-validated 2026-04-30). The 5-Whys chain, cross-validated forward chain, contributing factors, Option A1-vs-A2 trade analysis, files-affected breakdown, risk assessment, and full evaluator code shape are captured there in full and reproduced in compressed form above.
- Predecessor evolution doc: `docs/evolution/2026-04-30-fix-eval-reconciler-discarded.md` (commit `e6f5e5e` closed the §8 storm-proofing dispatch-routing contract at the unit/acceptance tier; this delivery closes it at the DST tier).
- Sibling fix: `docs/feature/fix-noop-self-reenqueue/deliver/bugfix-rca.md` (commit `7a60743` filtered `Action::Noop` from `has_work`, narrowing the §18 self-re-enqueue gate). The two precursor fixes plus this delivery jointly defend the §8 storm-proofing contract end-to-end at unit and DST tiers.
- ADR: `docs/product/architecture/adr-0013-control-plane-reconciler-runtime.md` §8 (storm-proofing invariant: 1 dispatch per distinct `(reconciler, target)` key per tick).
- ADR: `docs/product/architecture/adr-0003-crate-class.md` (sim crate is `adapter-sim`-class; cannot depend on `adapter-host`).
- ADR: `docs/product/architecture/adr-0004-sim-host-split.md` (sim ↔ host dep graph; mirror discipline rationale).
- Whitepaper §18 *Reconciler and Workflow Primitives* — *Triggering Model — Hybrid by Design*, *Evaluation Broker — Storm-Proof Ingress*.
- Whitepaper §21 *Deterministic Simulation Testing* — properties (safety / liveness / convergence), `assert_always!` shape.
- Test discipline: `.claude/rules/testing.md` § Tier 1 — Deterministic Simulation Testing, § Mutation testing, § "RED scaffolds and intentionally-failing commits".
- Source artifacts: `roadmap.json` (single-step plan with all ACs) and `execution-log.json` (DES trace) live under `docs/feature/fix-dst-dispatch-routing-invariant/deliver/` for the immediate post-mortem window; per the project's finalize protocol the per-feature directory is preserved (the wave matrix derives status from it) while session markers are removed.
