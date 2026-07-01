# backend-instance-replacement — Feature Evolution

**Feature ID**: `backend-instance-replacement`
**Branch**: `marcus-sa/backend-instance-replacement`
**Duration**: 2026-06-29 (DISCUSS + DESIGN) — 2026-07-01 (DELIVER + finalize close)
**Status**: Delivered — 8/8 DELIVER roadmap steps complete
(`01-01..01-04`, `02-01`, `03-01`, `03-02`, `04-01`) across 4 phases. Every step
logged its TDD phases in `execution-log.json` and reached `COMMIT EXECUTED PASS`.
The terminal DoD gate (04-01) is green: all three previously-`#[ignore]`'d
Tier-3 oracle acceptance tests (S-DBN-WS-STABLE, S-DBN-CHURN,
S-DBN-NXDOMAIN-02-RECOVERY) pass on the pinned-6.18 appliance kernel matrix,
driving the production `overdrive workload restart` verb.
**ADRs**: [ADR-0073](../product/architecture/adr-0073-backend-instance-replacement-workload-restart-generation-precursor.md)
(the feature's design record — `overdrive workload restart` + the desired-run
generation precursor); [ADR-0070](../product/architecture/adr-0070-mtls-connection-liveness-kernel-timeout-plus-per-connection-self-supervision.md)
**amended 2026-07-01** (the directional clean-close class — forward a clean
backend FIN as a leg-F half-close; the mid-DELIVER production fix, see "The
mid-DELIVER pivot" below).
**Closes**: GH [#249](https://github.com/overdrive-sh/overdrive/issues/249) `[D1]`
(backend instance replacement). Unblocks the three dial-by-name (J-MESH-001)
oracle ATs that carried `#[ignore = "…#249…"]` markers.

---

## What shipped

An operator verb that **replaces a declared workload's backend instance** — end
the current instance (`A1`, its `workload_addr`) and bring up a fresh one (`A2`,
a new `workload_addr`) while the `workloads/<id>` intent stays declared. The gap
this closed: Overdrive had three lifecycle paths that each did something *else*
— `job stop` writes a sticky suspend sentinel (no counterpart start), crash-restart
reuses the alloc-id/slot (no new identity), deletion (#211) withdraws intent (the
opposite operation). None of them cycles a workload to a *fresh instance of the
same spec*, which is the `kubectl rollout restart` shape an operator reaches for
when an instance comes up wedged.

Operator runs `overdrive workload restart <id>` → `POST /v1/jobs/:id/restart` →
the `restart_workload` handler atomically bumps a desired-run `generation` and
clears any stop sentinel → the `WorkloadLifecycle` reconciler observes
`observed_generation < generation`, ends the current instance if running, and
places a fresh one with a new `AllocationId` + `workload_addr`. The workload's
stable dial-by-name frontend `F` survives the cycle byte-stable; in-flight
clients to the dying instance fail fast; a subsequent connect lands on the new
backend.

Four phases, each a vertical slice through the production entry points
(`overdrive serve` + the new `overdrive workload restart` verb):

### Phase 01 — the replace action / walking skeleton (US-BIR-1)

The end-to-end restart loop, four steps:

1. **`TxnOp::IncrementU64` store primitive** (01-01) — a NEW read-modify-write
   `TxnOp` variant on the `IntentStore` port that reads the big-endian `u64` at a
   key (absent ⇒ 0) and writes `current + 1` (saturating) *inside the same redb
   write transaction* as every sibling op in the batch. This is the atomic
   monotonic increment the generation bump needs; `Put` (blind write — TOCTOU),
   `put_if_absent` (insert-if-absent, no increment), and `get`+`Put` (the rejected
   race) cannot express it. Ships with a trait behavior contract
   (preconditions / postconditions / edge cases / the monotonic-atomic observable
   invariant) and an `integration-tests`-gated concurrency acceptance test
   (`tests/acceptance/txn_increment_u64.rs`: N concurrent `txn`s ⇒ final == N).
   Not throwaway — #180's revision-lineage model reuses it verbatim.
2. **Desired-run generation precursor + current-instance-scoped reconciler veto**
   (01-02) — a standalone sibling intent key `workloads/<id>/generation` (8-byte
   BE `u64`, NOT an rkyv aggregate field, so no ADR-0048 envelope bump / golden
   fixture); a `WorkloadLifecycleState.generation` hydrate + a
   `WorkloadLifecycleView.observed_generation` (`#[serde(default)]`) memory field;
   and the load-bearing reconciler edit that gates the line-520 operator-stop veto
   on `restart_pending = observed_generation < desired.generation` **and scopes it
   to the workload's current instance** via a new pure `current_alloc` helper (the
   numerically-highest `mint_alloc_id` attempt suffix, not `BTreeMap`/lexical
   order). Stamps `observed_generation = desired.generation` on the placement tick
   only.
3. **`restart_workload` HTTP handler + `POST /v1/jobs/:id/restart` route + api
   types** (01-03) — mirrors `stop_workload` 1:1: parse → check-exists (404 if the
   `workloads/<id>` aggregate is absent) → one `IntentStore::txn` doing
   `[IncrementU64 gen, Delete stop]` → enqueue a `job-lifecycle` evaluation → 200
   `{ workload_id, outcome }`. `RestartOutcome ∈ { Restarted, Resumed }` is
   cosmetic, classified from the check-exists `/stop` read (present ⇒ `Resumed`,
   absent ⇒ `Restarted`) before the mutation — the label never gates placement,
   which is the reconciler's generation gate.
4. **`overdrive workload restart` CLI verb** (01-04) — a NEW top-level `workload`
   subcommand namespace (operator-mandated NOT under `job`; aligns with #220's
   planned `workload describe`), the `commands::workload::restart` handler, and the
   `ApiClient::restart_workload` http-client method. Closes the e2e vertical slice:
   a real `overdrive serve` + `overdrive workload restart` cycles a workload.

### Phase 02 — stable frontend survives the cycle (US-BIR-2, stable-F half)

**02-01** un-ignored the `S-DBN-WS-STABLE` Tier-3 oracle AT and rewired its
blocked stop/redeploy cycle to drive the production restart route. Proves the
dial-by-name frontend `F`-binding stays byte-stable across an instance
replacement (the `FrontendAddrAllocator`'s idempotent `assign` — withhold-not-
release — is untouched). Test-only; no production source changed.

### Phase 03 — in-flight churn fails fast (US-BIR-2, churn half)

This phase surfaced a **pre-existing datapath gap** and grew from a bare oracle
un-ignore into a genuine production fix (see "The mid-DELIVER pivot"):

- **03-01 — the A1 half-close-forward pump fix.** The intercept-worker's
  bidirectional pump now forwards a clean backend FIN to the client-facing leg-F
  as a half-close (`shutdown(dst, SHUT_WR)`) on a *source clean close*, gated on
  `!state.stop` (the ADR-0070 amendment's sole discriminator). A graceful
  `restart` SIGTERMs the backend → its socket FINs cleanly, and a clean directional
  FIN was previously invisible to both v1 liveness mechanisms — self-teardown fires
  only on `TransportDeath`, and `TCP_USER_TIMEOUT` reaps only *unacked* death — so
  the datapath absorbed the FIN (`PumpExit::Graceful` non-reclaim) and the in-flight
  client hung. The forward makes the in-flight client observe EOF near-instantly.
  Landed with unit + mutation coverage on the pump logic and a long-lived
  full-duplex test backend (the T1/T2 test-model fix). NOT `sock_destroy` (#61
  scope stays out); half-close correctness (D-MTLS-16) is retained.
- **03-02 — un-ignore `S-DBN-CHURN`.** With the A1 fix landed, the Tier-3
  in-flight-churn fail-fast oracle AT was un-ignored and driven against the
  production restart verb; it now goes green.

### Phase 04 — the terminal Tier-3 oracle gate (DoD)

**04-01** un-ignored the third and final oracle AT
(`S-DBN-NXDOMAIN-02-RECOVERY` — the withhold-not-release `getent` recovery
observable) and confirmed all three oracle ATs green on the pinned-6.18 matrix.
This is the feature's Definition-of-Done gate.

---

## The mid-DELIVER pivot — 03-01 blocker → ADR-0070 half-close amendment + phase-03 re-scope

The single most consequential event of DELIVER. It is recorded here because it is
exactly the shape the "No effort/time budget cuts" and "Build vertical slices
through production entry points" rules are meant to produce.

**What happened.** The original roadmap sized phase 03 (churn) as a bare
test-only oracle un-ignore, matching the DISCUSS `[REF]` sizing ("SHIPPED …
intercept-worker `TCP_USER_TIMEOUT` legs"). The first 03-01 attempt un-ignored
`S-DBN-CHURN` and ran it against the production verb — and it **FAILED** on two
clean-VM runs (observed 30.7s vs `CHURN_BOUND=30s`). The in-flight death never
surfaced on the open connection within bound.

**Why the blocker was correct to raise, not force green.** The crafter surfaced a
BLOCKER to the orchestrator rather than reaching for any of the forbidden green
levers (bump `CHURN_BOUND`, tune the worker `TCP_USER_TIMEOUT`, synchronous
old-gen teardown) — every one of which was ruled out by the step's byte-identical
assertions + test-only-edit boundary. The COMMIT was withheld (`GREEN did not
pass`) per the Test Integrity Iron Rule and CLAUDE.md's "no partial COMMIT
against an incomplete deliverable."

**The root cause** (RCA:
`docs/analysis/root-cause-analysis-in-flight-churn-fail-fast-gap.md`, Rex): a
graceful `restart` FINs the backend socket cleanly, and a clean directional FIN
was invisible to *both* v1 liveness mechanisms — (B) self-teardown fires only on
`TransportDeath`; (C) `TCP_USER_TIMEOUT` reaps only *unacked* death. The datapath
absorbed the FIN via `PumpExit::Graceful` non-reclaim and never propagated EOF to
the client-facing leg-F, so the in-flight client hung. The test wasn't wrong about
the requirement; the datapath had a real gap.

**The resolution.** ADR-0070 was **amended** (through the architect, 2026-07-01)
with a new "directional clean-close class": forward the FIN as a leg-F half-close.
Phase 03 was **re-scoped** from one bare-un-ignore step into two: **03-01** (the
A1 production pump fix — unit + mutation tested) *before* **03-02** (the churn
un-ignore that now goes green on top of it). The feature-delta and the affected
DISCUSS `[REF]` sizing were marked SUPERSEDED-by-amendment rather than silently
rewritten.

**The lesson.** A Tier-3 oracle AT that was designed as "test-only" earned its
keep by refusing to go green over a real datapath gap. The honest response was to
grow production scope (a new ADR amendment + a real pump fix) rather than weaken
the test — precisely because there was no lever inside the step's boundaries that
was both green-making and correct.

---

## Key design decisions (from `design/wave-decisions.md` / ADR-0073)

| # | Decision | Why |
|---|---|---|
| DDD-1/2 | Verb = `overdrive workload restart <id>` (new top-level `workload` namespace), one verb with rollout-restart breadth (running → stop-then-start; stopped → start) | Operator-mandated; aligns with #220's `workload describe`; matches `kubectl rollout restart`. `job` namespace stays `list`/`stop` only. |
| DDD-3/4 | Mechanism = a **thin** desired-run `generation: u64` precursor; reconciler places when `observed_generation < generation`. ONLY `generation`/`observed_generation` — NO revision rows / `RevisionId` / retention / status | Supersedes the stale line-520 observation-veto with intent-driven placement; the forward-compat seam #64/#253/#254 reuse verbatim. Pulling #180's full lineage forward is the rejected Alt-C over-build. |
| DDD-5 | Generation = standalone sibling key `workloads/<id>/generation`, 8-byte BE `u64` | Not an rkyv aggregate field ⇒ no ADR-0048 envelope bump / golden fixture; sibling-key precedent (`/stop`, `/kind`). Folds into `workloads/<id>/current` when #180 lands. |
| DDD-6/13 | Reconciler gates the veto on `restart_pending` **AND scopes it to the current instance** (`current_alloc(...).is_some_and(is_operator_stopped)`, NOT `any(...)` across history) | Clearing the sentinel alone is necessary-but-NOT-sufficient. The `any(...)` form let a retained superseded `payments-0 / Operator` row re-arm the veto after the fresh placement and wedge the fresh instance's later crash (the iteration-3 review Critical). Reuses the existing alloc-id-suffix monotonicity — no rkyv `AllocStatusRow` change. |
| DDD-7 | Only `restart` bumps; `deploy` never does | Bug-3 preserved (`fix-exec-driver-exit-watcher`): a same-spec re-deploy cannot resurrect an operator-stopped workload — after a deploy `observed == desired`, so the current-instance veto stands. |
| DDD-8 | HTTP route `POST /v1/jobs/:id/restart` (mirror `stop`), NOT `/v1/workloads/:id` | Consistency with the live `/v1/jobs` family; the `jobs/` HTTP prefix is independent of the `workloads/` IntentKey prefix + the `workload` CLI verb — the same split `job stop` already ships. |
| DDD-9 | TOCTOU-safe: generation bump + sentinel delete in ONE `txn` via `TxnOp::IncrementU64`; NO `Conflict` retry | `development.md` § "Check-and-act must be atomic" — redb serializes writers, so the increment is atomic + monotonic with no window. The prior `Put`-gen + retry-on-`Conflict` relied on a conflict `LocalIntentStore::txn` never produces (returns `Committed` unconditionally) — a lost-bump + backwards-wedge bug. |
| DDD-10 | Idempotency = **level-triggered coalescing** (not per-generation consumption) | A "replace the instance" op is definitionally a level, not a command queue. Generation advances monotonically per call (audited); the reconciler converges to one fresh instance for the latest generation. Sequential restarts each cycle; concurrent/pre-placement restarts coalesce. Per-generation consumption would graft an edge-triggered replay queue onto the reconciler — the anti-pattern ADR-0064's two-primitive doctrine rejects. |
| ADR-0070 amendment | Forward a clean backend FIN as a leg-F half-close (`shutdown(dst, SHUT_WR)`), gated on `!state.stop` | The mid-DELIVER datapath fix (above). NOT `sock_destroy` (#61); half-close correctness (D-MTLS-16) retained. |

The design passed through **four DESIGN-review iterations**, each resolving one
Critical: (1) the unproduceable `TxnOutcome::Conflict` atomicity blocker →
`TxnOp::IncrementU64`; (2) the cardinality contract over-claim → level-triggered
coalescing; (3) the `any(...)`-over-all-history stale-veto re-arm → current-
instance-scoped veto; (4) a handoff-index correction in `brief.md`. Iteration 4's
verdict was `conditionally_approved` with no remaining correctness blocker.

---

## Steps completed (from `execution-log.json`)

| Step | TDD phases | Outcome |
|---|---|---|
| 01-01 | PREPARE / RED_ACCEPTANCE / RED_UNIT / GREEN / COMMIT | all PASS |
| 01-02 | PREPARE / RED_ACCEPTANCE / RED_UNIT / GREEN / COMMIT | all PASS |
| 01-03 | PREPARE / RED_ACCEPTANCE / RED_UNIT (N/A — ATs fully specify) / GREEN / COMMIT | all PASS |
| 01-04 | PREPARE / RED_ACCEPTANCE / RED_UNIT (N/A — thin sibling of stop) / GREEN / COMMIT | all PASS |
| 02-01 | PREPARE / RED_ACCEPTANCE / RED_UNIT (N/A — oracle un-ignore) / GREEN / COMMIT | all PASS |
| 03-01 (attempt 1) | GREEN **FAIL** (S-DBN-CHURN 30.7s vs 30s bound) → COMMIT SKIPPED (blocked) | blocker surfaced, re-scoped |
| 03-01 (re-scoped A1) | PREPARE / RED_ACCEPTANCE (N/A — Tier-2 unit AT drives A1) / RED_UNIT / GREEN / COMMIT | all PASS |
| 03-02 | PREPARE / RED_ACCEPTANCE / RED_UNIT (N/A — oracle un-ignore) / GREEN / COMMIT | all PASS |
| 04-01 | PREPARE / RED_ACCEPTANCE / RED_UNIT (N/A — oracle un-ignore) / GREEN / COMMIT | all PASS |

The `RED_UNIT` skips are all classified `NOT_APPLICABLE` with recorded rationale
(oracle un-ignore steps with no new pure-decision surface, or thin handler
siblings whose ATs fully specify the decision logic) — consistent with
`distill/red-classification.md` § "oracle ATs are NOT MISSING_FUNCTIONALITY
scaffolds".

---

## Issues encountered / review verdicts

All DELIVER steps that touched production source carried an adversarial code/test/
evidence review:

| Step | Verdict | Notes |
|---|---|---|
| 01-01 | `approved` | prior stale blockers closed |
| 01-02 | `APPROVED_WITH_RESIDUAL_RISK` | two prior blockers closed (`b228982d`, `c74d2d55`) — R5 leaves the draining instance alone during replacement |
| 01-03 | `APPROVED_WITH_TRACE_NIT` | mutation-evidence blocker closed via `mutants-01-03.md` manual kill proofs (one unviable whole-function replacement, no tool signal) |
| 01-04 | `APPROVED_WITH_TRACE_NITS` | CLI tests strengthened to deterministic per-direction outcome assertions + a stopped-workload `Resumed` scenario |
| 02-01 | `APPROVED` | scope correctly limited to the oracle un-ignore |
| 03-01 | `APPROVED_WITH_NITS` | pump call sites covered through real pump-level tests; the non-source/dst-EOF ambiguity documented + regressed; `!state.stop` pinned as the sole discriminator |

Recurring blocker class across 01-01..01-03: **mutation-evidence gaps** — several
steps landed a mutation-evidence artifact (`mutants-0X-0X.md`) in a follow-up
commit to close a review BLOCKER, sometimes with manual `+1 → +0` kill proofs
where cargo-mutants generated an unviable whole-function replacement and produced
no tool signal.

---

## Lessons learned

- **A "test-only" oracle can force real production scope, and that is the system
  working.** Phase 03's churn AT refused to go green over a genuine datapath gap;
  the correct response was an ADR amendment + a real pump fix + a phase re-scope,
  not a weakened bound. Surfacing the blocker cost one message; forcing green
  would have shipped a wrong liveness contract.
- **Atomic-increment is a store primitive, not a call-site pattern.** The review
  Critical killed the `get`+`Put`+retry-on-`Conflict` draft because the store
  cannot produce that conflict. The fix was to add the atomicity to the port
  (`TxnOp::IncrementU64`, read-modify-write inside the write txn), where redb's
  single-writer serialization makes it correct by construction — and it is reused
  verbatim by #180/#64/#253/#254.
- **Scope the veto to the current instance, not all history.** The
  never-delete-alloc-rows invariant (which the feature relies on for `A1 ≠ A2`)
  means a retained superseded operator-stop row would re-arm an `any(...)`-shaped
  veto and wedge a later crash of the fresh instance. Keying off the
  latest-placed instance (numeric `mint_alloc_id` suffix max, NOT lexical
  `BTreeMap` order) fixed it with no new per-row rkyv field.
- **Keep the forward-compat seam thin.** A `u64` sibling key + two struct fields
  is the whole generation surface; the full revision-lineage model stays deferred
  to #180. The seam is reused, not rebuilt, by the rolling-deploy / zero-downtime /
  multi-replica features.

---

## Migrated permanent artifacts

- **Design record**: [ADR-0073](../product/architecture/adr-0073-backend-instance-replacement-workload-restart-generation-precursor.md)
  (already permanent); [ADR-0070 amendment](../product/architecture/adr-0070-mtls-connection-liveness-kernel-timeout-plus-per-connection-self-supervision.md)
  (already permanent).
- **Feature-delta (design SSOT)**: `docs/architecture/backend-instance-replacement/feature-delta.md`
  (migrated from the feature workspace).
- **Acceptance scenarios**: `docs/scenarios/backend-instance-replacement/test-scenarios.md`
  (migrated from the feature workspace).
- **Root-cause analysis**: `docs/analysis/root-cause-analysis-in-flight-churn-fail-fast-gap.md`
  (already permanent — the churn-gap RCA that drove the ADR-0070 amendment).
- **History**: the feature workspace `docs/feature/backend-instance-replacement/`
  is preserved (roadmap, execution log, per-step reviews + mutation evidence, slice
  briefs, DISCUSS/DESIGN/DISTILL artifacts). This evolution doc is the summary; the
  workspace is the full record.
