# Adversarial Implementation Review — Step 02-02

**Feature:** canonical-workload-address-inbound-tproxy (GH #241)
**Step:** `02-02` — ServiceMapHydrator GATE — three-way subnet-membership split
**Commit under review:** `a46a3163`
**Scenario:** S-GATE (D-GATE / D-GATE-PRED)
**Reviewer:** nw-software-crafter-reviewer (adversarial pass, Opus) + orchestrator corroboration
**Date:** 2026-06-23

## Verdict: **NEEDS_REVISION → RESOLVED / APPROVED** (2026-06-23, see `02-02.md`)

Resolution at commit `92702af5`: D1 mutation evidence captured (4/4 caught = 100.0%, embedded
verbatim in `02-02.md`), D2 mixed-service backends-content test added, D3 all-mesh guard
ratified + pinned with a ≥2-tick test, D4 + nitpicks addressed. Full record in `02-02.md`.

- **Blocking: 1** — D1 (mutation-gate evidence; recurring gap that blocked 01-02 AND 02-01).
- **High: 1** — D2 (gate's load-bearing mixed-service behavior entirely untested).
- **Medium: 1** — D3 (invented, un-surfaced production behavior: the all-mesh recompute guard).
- **Non-blocking: 1** — D4 (V6-VIP path bypasses the gate; Phase-1-unreachable).

The code is design-faithful on the **named API surface**: zero invented *public* surface (no mesh-flag
field, no new predicate input), a mandatory `workload_subnet: Ipv4Net` ctor param threaded from the single
`WORKLOAD_SUBNET_BASE` source, and the reviewer-mandated pre-filter-before-partition shape. The three
S-GATE arms pass and no existing test was weakened. It does **not** meet its own acceptance bar: the
mandatory mutation gate has no recoverable evidence (and the only recoverable artifact contradicts the
claim), the gate's whole reason to exist — per-backend filtering in a *mixed* service — has zero test
coverage, and a real behavioral decision was baked into production silently.

---

## Blocking

### issue (blocking) — D1 · mutation-gate evidence is unverifiable AND contradicted on disk
**Location:** AC #6 (`roadmap.json:113`); commit `a46a3163` claims "Mutation gate: 3/3 caught = 100.0%".

AC #6 mandates ≥80% kill-rate on the hydrator partition via
`cargo xtask mutants --diff origin/main --features integration-tests` (Lima-routed). Evidence state:

- No committed mutation-summary artifact; host `target/xtask/mutants-summary.json` absent (Lima writes to
  the **guest** target dir — known host-path trap). No `02-02.md` evidence file. `.develop-progress.json`
  records no kill-rate.
- The **only** recoverable `target/xtask/mutants.out` in the Lima guest is from **Jun 23 01:46** (the
  baseline / 01-xx era, NOT the 02-02 commit landed at 15:32). Its `outcomes.json` shows
  `total 1, {"Failure": 1}` — a single **failed-baseline** outcome — with **empty** `caught.txt`,
  `missed.txt`, `timeout.txt`, `unviable.txt`. Its `mutants.diff` covers `action_shim/mod.rs`,
  `reconciler_runtime.rs`, `streaming.rs`, `exit_observer.rs` — **not** `service_map_hydrator.rs`.

So nothing on disk substantiates "3/3 = 100%" for the hydrator partition, and the only recoverable run is a
stale failed baseline over unrelated files. (The single Failure may be the known
`--features integration-tests` nft-baseline race; either way, no passing hydrator-surface run is
recoverable.) **This is the identical gap that produced NEEDS_REVISION on 01-02 and 02-01** — third
consecutive step.

**Required to clear:** re-run scoped + Lima-routed
(`cargo xtask mutants --diff origin/main --features integration-tests --package overdrive-core --file
crates/overdrive-core/src/reconcilers/service_map_hydrator.rs`), read the kill-rate from the **guest**
summary / run log, and record it (with the mutant list and HEAD `a46a3163`) in a `02-02.md` resolution
section — exactly as 02-01 was resolved.

---

## High

### issue — D2 · the gate's load-bearing mixed-service behavior is untested
**Location:** `mesh_backend_lb_gate.rs:78-95` (`desired_with_backend` builds a *single*-backend service),
`:220` (PBT generates a single address); the pre-filter at `service_map_hydrator.rs:374`.

The gate exists to filter, *per backend*, mesh members out of a service that also carries non-mesh
backends — keeping the non-mesh ones in the emitted `DataplaneUpdateService.backends`. **No test exercises a
mixed service.** Every gate test is single-backend; the existing acceptance suite explicitly opts out
(`service_map_hydrator_reconcile.rs:40-43`: "No backend address in this suite falls inside it"). Across all
three surfaces **no assertion observes the emitted `backends` vector content** — only action counts
(`register_local_backend_count` / `dataplane_update_service_count`). A regression that leaks a mesh backend
into the remote action's `backends` vec, or drops a surviving non-mesh backend, passes all three S-GATE arms
and the PBT untouched. This also blunts D1: a content-level mutation in a mixed set has no observing
assertion to kill it.

**Fix:** add one mixed-service test — `ServiceDesired` with `[10.99.0.6 (mesh), 10.96.0.50 (remote),
host_ipv4 (local)]` — asserting exactly one `DataplaneUpdateService` whose `backends` contains **only**
`10.96.0.50`, plus one `RegisterLocalBackend` for `host_ipv4`. This is the single test that pins the gate's
real contract.

---

## Medium

### thought / question — D3 · invented, un-surfaced production behavior (the all-mesh recompute guard)
**Location:** `service_map_hydrator.rs:420` (`if !(local_is_empty && remote_is_empty) { …record
fingerprint/bump retries… }`), doc-comment `:412-419`.

**Verified trace:** an all-mesh service (the feature's *primary* Path-A class) empties both `local` and
`remote` → zero actions → with this guard, the View row is never recorded → every later tick
`should_dispatch(None, fp, None, now)` hits `_ => true` → `need_dispatch == true` every 100ms tick forever.
The service busy-recomputes the partition each tick. Not in any AC; no test ticks a fully-gated service
twice.

**Adversarial counter-assessment (NOT blocking):** the non-convergence is benign — a fully-gated mesh
service has no convergence target (nft-TPROXY owns delivery, out of this reconciler's scope). Tracing the
counterfactual: **without** the guard the service would record `last_attempted_fingerprint = Some(fp)` (an
affirmative lie that it "programmed at fp") and, since `backoff_for_attempt` is degenerate-constant at 1s,
would re-dispatch + **fsync a View row ~once/sec forever**. **With** the guard: zero actions, zero
`ViewStore` I/O, no phantom fingerprint — only a cheap pure recompute. The guard is *strictly more honest*,
not "worse." The original "arguably worse" hypothesis does **not** survive the trace.

**Why still a finding:** per CLAUDE.md "Implement to the design — STOP and surface the gap," this guard is a
genuine design decision over a corner the design left open (what a fully-gated service records). It is the
*milder* form (not a new public method/type/param, so not the `TerminalErrorKind::Retryable` precedent), but
it was chosen and shipped silently with a self-justifying comment rather than surfaced for ratification.

**Fix:** (1) surface the all-mesh disposition to the orchestrator/PO for explicit ratification; (2) add a
test that ticks a fully-gated mesh service ≥2 times asserting zero actions AND `next_view.retries` stays
empty across ticks — converting the undocumented behavior into a pinned, intentional contract.

---

## Non-blocking

### nitpick — D4 · V6-VIP early-return bypasses the gate (Phase-1-unreachable)
**Location:** `service_map_hydrator.rs:336-357`. The V6-VIP arm emits `DataplaneUpdateService` with
`backends: desired_svc.backends.clone()` (ALL backends, mesh included) and `continue`s — the mesh filter
(`:367-379`) runs only on the V4-VIP path. AC #1's gate language is unconditional. Unreachable in Phase-1
(V4-only services), so no live exposure. Note the unreachability in a comment, or apply `is_mesh_backend` to
the V6 arm for uniformity.

### nitpick — sim invariant macro-body indentation
**Location:** `crates/overdrive-sim/src/invariants/service_map_hydrator.rs:754,833`. The two
`proptest!{}`-body `canonical(...)` call sites are mis-indented (rustfmt skips macro bodies, so lefthook did
not normalize them). Cosmetic.

### nitpick — subnet literal re-declared in two core test files
`10.99.0.0/16` is re-declared in `mesh_backend_lb_gate.rs:51` and
`service_map_hydrator_reconcile.rs` (vs the real `WORKLOAD_SUBNET_BASE` const the sim invariant uses).
Acceptable per "core MUST NOT depend on the wiring crate," but a silent-drift risk if the const changes.

---

## Praise

- **praise — one-source wiring, mandatory ctor param, zero defaulting** (`lib.rs:2451-2459`;
  `service_map_hydrator.rs:280-285`): the construction site threads the *same*
  `veth_provisioner::WORKLOAD_SUBNET_BASE` the provisioner carves `/30`s from; `canonical` takes
  `workload_subnet: ipnet::Ipv4Net` as a **mandatory** positional param (no builder, no production default —
  a forgetful call site fails to compile); core takes the generic `Ipv4Net` value with **no
  core→control-plane dependency**. Textbook development.md § "Port-trait dependencies" + D-GATE-PRED.
- **praise — pre-filter before partition** (`service_map_hydrator.rs:374`): `.filter(|b|
  !is_mesh_backend(b)).partition(...)` honors the roadmap reviewer's refinement #1 — mesh **cannot** leak
  into remote via the partition closure.
- **praise — existing suite invariants preserved**: `service_map_hydrator_reconcile.rs:40-43` documents
  non-collision with 10.99/16; the gate is a verified no-op for the prior suite, so the "byte-for-byte
  unchanged for non-mesh arms" AC holds. Sim invariant scenarios (10.0.x / 10.1.x) likewise don't collide —
  invariants not silently weakened.

---

## Quality gates

| Gate | Status | Note |
|---|---|---|
| G1 single acceptance | PASS | S-GATE three arms |
| G2 valid failure | PASS | RED_ACCEPTANCE EXECUTED/PASS |
| G3 assertion failure | N/A | RED_UNIT SKIPPED — rationale partially false (mixed-service unit surface uncovered, D2) |
| G4 no domain mocks | PASS | pure reconciler, port-to-port |
| G5 business language | PASS | mesh/local/remote arm naming |
| G6 all green | PASS | GREEN EXECUTED/PASS |
| G7 100% before commit | PASS | COMMIT EXECUTED/PASS |
| G8 test budget | PASS | 4 tests ≤ 6 budget |
| G9 no test modification | PASS | existing suite additively migrated; no assertion weakened |
| Mutation gate (AC #6) | **FAIL** | claim unverifiable; only recoverable artifact is a stale failed baseline over unrelated files (D1) |

**Test integrity:** testing-theater / fixture-theater / G9-test-modification all **clear** — the count-based
assertions are genuine; the deficiency is coverage *breadth* (D2), not theater.

---

## Required to reach APPROVED

1. **D1 (blocker)** — capture real mutation evidence for the hydrator partition at HEAD `a46a3163`
   (≥80% kill-rate), recorded in `02-02.md`.
2. **D2 (high)** — add the mixed-service test asserting emitted `backends` content.
3. **D3 (medium)** — surface the all-mesh disposition for ratification + pin it with a ≥2-tick test.

D4 and the two nitpicks are advisory.
