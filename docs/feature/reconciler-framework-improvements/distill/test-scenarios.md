# DISTILL test-scenarios — `reconciler-framework-improvements` (GH #266)

**Specification only.** These GIVEN/WHEN/THEN blocks are the acceptance spec for
Piece A (cadence) + Piece B (interests) locked in **ADR-0084**. Per
`.claude/rules/testing.md` there are **no `.feature` files in this repo** — the
DELIVER crafter translates each scenario below into Rust `#[test]` /
`#[tokio::test]` (DST + proptest + unit). Nothing here is executable; nothing here
touches `crates/`.

Requirements SSOT: `adr-0084-reconciler-cadence-and-interest-declarations.md`
(the LOCKED surface) + the SYSTEM/APPLICATION sections of `../feature-delta.md` +
`../design/wave-decisions.md`. Scenarios reference **only** ADR-0084's locked
surface: `resync_schedule`, `ResyncSchedule{period, scope}`, `ResyncScope{LocalNode}`,
`interests` (`-> &'static [ObservationRowKind]`), `ObservationRowKind{AllocStatus, …8}`,
the total `ObservationRow::kind()`, the loop's `resolve_scope`, the next-wake table
(`BTreeMap<ReconcilerName, UnixInstant>`), and `spawn_interest_router` — plus the
REUSED `broker.submit` / `Evaluation` / `TargetResource` / `ReconcilerName` /
`subscribe_all_events`. No surface beyond this is invented (constraint honoured;
zero blockers).

---

## Prior-wave gate checklist

| Gate | Result |
|---|---|
| DESIGN present | ✓ ADR-0084 + feature-delta SYSTEM/APPLICATION + design/wave-decisions.md |
| DISCUSS present | ✗ **WARN** — no DISCUSS wave for this feature (requirements source = research §6 + GH #266 + ADRs). ACs derived from DESIGN. |
| DEVOPS present | ✗ **WARN** — deliberately no DEVOPS wave; this is an in-process framework change, no deployment/env matrix. |
| Reconciliation HARD GATE | ✓ **PASS** — only `design/wave-decisions.md` present; 0 contradictions. |
| deliverable_type | `application` (Rust). Type-specific verification = the four-reviewer gate; no plugin/skill reviewer. |

---

## Driving surface (no CLI/HTTP port)

This feature has **no operator verb, no CLI subcommand, no HTTP endpoint**. The
driving surface is the **convergence loop / `run_server` production composition**:
`spawn_convergence_loop` (gains the cadence next-wake table) and the new
`spawn_interest_router` task, both composed at the production entry
`run_server_with_obs_and_driver` (`lib.rs:1447`) — the same entry that already
spawns the convergence loop at `lib.rs:2315`. The walking-skeleton scenario
(S-266-01) **boots through that production entry** (Sim obs + `SimClock`) and
asserts the entry itself **spawns `spawn_interest_router`** — never a hand-assembled
router and never a hand-call of the spawn fn (honours the vertical-slice rule: "no
test installs the one production call site the feature omitted"). The spawn-wiring
teeth are *does `run_server_with_obs_and_driver` spawn the router?*, not *does the
spawn fn exist / compose with the loop?*. No test installs real infra a production
`serve` does not install itself; the Sim composition (`SimClock` + observation
store) is the primary lane, with a full `run_server` Lima boot as the fallback.

## Ubiquitous-language note (Pillar 1)

The SUT **is** the reconciler framework, so its own vocabulary — *reconciler,
resync, interest, observation row, convergence, evaluation broker, target,
tick* — **is** the ubiquitous domain language (brief.md §24-28; ADR-0013/0021/0035/0036/0081).
These are domain terms, not technical jargon. Technical leakage to reject at
review would be host-implementation terms (`tokio::broadcast`, `redb` bytes,
struct field names, SQL) — none of which appear in a scenario title or GWT step
below. Assertions are on **port-exposed observables** (broker counters, drained
evals, reconcile-invocation records), never on internal struct fields (Mandate 8
Universe discipline; `next_wake[..]` table internals are NEVER asserted — only the
*submits* it produces are).

## SUT state machines documented (C2a)

**Cadence next-wake (per reconciler with `Some(ResyncSchedule)`):**
`Unarmed → Armed(next_wake=now+period)` at registration →
`Elapsed(now ≥ next_wake) → submits (R, resolved target) + Re-armed(next_wake += period)`.
A reconciler returning `None` has **no state** in this machine (no table entry).
Illegal event: a second re-arm within one period (forbidden by C-A2 — asserted
S-266-03/S-266-19).

**Interest fan-out (List-then-Watch):**
`Subscribed → Listed(submit per pre-existing interested row) → Watching`; in
`Watching`: `Row(accepted) → row.kind() → table lookup (BTreeMap<ObservationRowKind,
Vec<ReconcilerName>>): interested reconcilers derive target inline + submit |
uninterested kind: drop`;
`Lagged{missed} → Relist (re-read snapshot, re-submit) → Watching`. A non-accepted
(LWW-loser) write is never delivered as `Row`, so it is a no-transition (S-266-16).

## Universe of port-exposed observables (Mandate 8, Rust host)

DST/unit assertions draw from this fixed observable set — never internal fields:

- `broker.pending_keys` — the set of distinct `(ReconcilerName, TargetResource)`
  keys pending (observed via `drain_pending()` output / `BrokerCounters.queued`).
- `broker.counters` — `{queued, cancelled, dispatched}` (`BrokerCounters`, a
  port-exposed snapshot type).
- `reconcile_invocations[(reconciler, target)]` — that the loop invoked reconcile
  for a `(reconciler, target)` this tick (Sim spy on the dispatch path).
- `resolve_scope(scope, node_id)` return — the resolved `TargetResource` list
  (pure-fn return, port-exposed).
- `ObservationRow::kind()` return — `ObservationRowKind` (total, no-wildcard
  discriminant; pure-fn return). The router's inline `workload/<id>` target
  derivation is **router-internal**, not a standalone observable — it is asserted
  via the submit's `TargetResource` (S-266-12 / S-266-10), never as a separate fn return.

Rust host uses `assert_always!` / `assert_eventually!` (Tier-1 DST idiom,
`testing.md` §21) as the state-delta/Universe equivalent; the "universe = these
names, nothing else mutates unexpectedly" discipline is expressed by asserting the
**full** submit set per stimulus (not just presence of one submit).

---

## Piece A — cadence scenarios

### S-266-01 — Walking skeleton: a reconciler's watched rows change and it converges, driven by the production loop
`@walking_skeleton @dst @piece-a @piece-b @contract-shape:bounded-change`
**Outcome (elevator pitch, ubiquitous language):** *A reconciler declares which
observed rows concern it and is woken to converge when they change — driven by the
production convergence loop, with the loop naming no reconciler by hand.*

- **Given** the runtime is booted through its **production composition entry**
  `run_server_with_obs_and_driver` (`lib.rs:1447`) — the same entry that spawns the
  convergence loop at `lib.rs:2315` — wired with a Sim observation store +
  `SimClock`, and the four current consumers declaring
  `interests() = &[ObservationRowKind::AllocStatus]`
- **And** that production entry — **not the test** — spawns `spawn_interest_router`
  as part of its composition (the test hand-calls no spawn fn)
- **When** an accepted `alloc_status` transition for workload `W` is written
  through the production path
- **Then** each of the four interested reconcilers is evaluated for target
  `workload/W` (observed via `reconcile_invocations`), driven by the router that
  `run_server_with_obs_and_driver` wired — not by any hand-installed submit
- **And** the loop source names no reconciler and no `alloc_status`-consumer list
  (the fan-out is table-driven)

**Tier:** Walking Skeleton / composition-root — **primary lane:** Tier-1 DST that
boots `run_server_with_obs_and_driver` with Sim obs + `SimClock`; **fallback:** a
full `run_server` Lima boot under `integration-tests`. **Spawn-wiring teeth:** the
assertion is *does `run_server_with_obs_and_driver` spawn the router?* — a
structural/boot check that the router task is live after the production entry
returns — NOT *does the spawn fn exist or compose with the loop?*. **Observable:**
the router task is spawned by the production entry, and after a write through the
production path `reconcile_invocations == {(each consumer, workload/W)}`. **Traces:**
ADR-0084 §5 single-cut migration; Consequences (fan-out fires on any accepted
write); vertical-slice rule ("no test installs the one production call site the
feature omitted").

### S-266-02 — A scheduled reconciler is resynced once per period
`@dst @piece-a @property @contract-shape:bounded-change`
- **Given** reconciler `R` declares `resync_schedule() = Some(ResyncSchedule {
  period: P, scope: LocalNode })` and the loop owns local `NodeId = n`
- **When** `SimClock` advances across `k` whole periods with no row changes
- **Then** the broker receives **exactly `k`** submits of `(R, node/n)` — one per
  period, routed through `broker.submit`

**Tier:** Tier-1 DST (`@given`/PBT over `P`, `k`, `n`). **Observable:** the count of
`(R, node/n)` submits observed **in the broker** equals **exactly `k`** after `k`
periods — the positive, falsifiable teeth for C-A1: a resync that bypassed
`broker.submit` via a side channel makes the in-broker count `< k` (miss), and a
per-tick re-arm makes it `> k` (overshoot); only routing every resync through the
broker exactly once per period yields `k`. **Traces:** ADR-0084 §4 (loop change),
SD-2/SD-3/SD-4, RN-1.

### S-266-03 — Resync re-arms at the period boundary, not before, not every tick
`@dst @piece-a @property @error @contract-shape:bounded-change`
- **Given** `R` armed at `t0` with `period = P` (next-wake = `t0 + P`)
- **When** `SimClock` reads `t0 + P − ε`, then `t0 + P`, then `t0 + 2P`
- **Then** **no** `(R, node/n)` submit before `P` elapses; **exactly one** at
  `t0 + P`; **one more** at `t0 + 2P` (never one-per-tick within a period)

**Tier:** Tier-1 DST, boundary (C1b) + the `next_wake <= now` decision and the
`next_wake += period` re-arm (mutation targets). **Observable:** submit count per
tick window. **Traces:** ADR-0084 §4 C-A2; §4.3 no-storm.

### S-266-04 — A reconciler with no schedule is never resynced
`@dst @piece-a @error @contract-shape:bounded-change`
- **Given** reconciler `R` uses the default `resync_schedule()` (returns `None`)
- **When** the loop runs for many periods under `SimClock`
- **Then** the broker receives **zero** resync-origin submits for `R` (no cadence
  table entry is built for `R`)

**Tier:** Tier-1 DST (C1a empty / C4b inverse). **Observable:** `assert_always`
no `(R, *)` submit whose origin is the cadence path. **Traces:** ADR-0084 §1
default `None`; SD-6 (empty/None ⟺ host-backed ⟺ resync-only partition).

### S-266-05 — Two reconcilers with distinct periods each fire on their own cadence (loop carries no hardcode)
`@dst @piece-a @property @contract-shape:bounded-change`
- **Given** reconciler `X` declares `Some{period: 10s, LocalNode}` and `Y`
  declares `Some{period: 30s, LocalNode}`, local `NodeId = n`
- **When** `SimClock` advances 60s with no row changes
- **Then** `X` receives 6 submits of `(X, node/n)` and `Y` receives 2 submits of
  `(Y, node/n)`, each driven purely from its declaration

**Tier:** Tier-1 DST (C5b flag orthogonality + structural no-hardcode proxy: an
arbitrary new declaration is driven with no per-reconciler constant). **Observable:**
per-reconciler submit counts. **Traces:** ADR-0084 §4 ("loop carries no reconciler
name, no cadence constant, no hardcoded target scheme"). *Companion structural
check (DELIVER reviewer, CM-style): a source scan confirms the loop names no
reconciler and no cadence constant — mirrors `eval_broker_does_not_import_clock_transport_entropy`.*

### S-266-06 — The cadence hook is pure; `reconcile` stays pure/sync
`@unit @property @contract-shape:pure-function`
- **Given** the `Reconciler` trait after the additive `resync_schedule` method
- **When** the trait signature is pinned at compile time
- **Then** `resync_schedule(&self) -> Option<ResyncSchedule>` takes only `&self`
  (no clock, no `now`, no I/O, no DB handle)
- **And** the existing `reconciler_trait_signature_is_synchronous_no_async_no_clock_param`
  guard (`mod.rs:271`) still passes (`reconcile` unchanged)

**Tier:** Unit / compile-time assertion (proptest not applicable — a signature
invariant). **Observable:** the type assertion compiles. **Traces:** ADR-0084 §1;
"ADR-0036 stands"; feature-delta A7.

### S-266-07 — `LocalNode` scope resolves to exactly the local node target, totally
`@unit @property @contract-shape:pure-function`
- **Given** `ResyncScope::LocalNode` and an arbitrary valid `NodeId = n`
- **When** the loop's `resolve_scope(LocalNode, n)` runs
- **Then** it returns exactly `[TargetResource("node/<n>")]` — a total mapping
  over the single-variant enum (no `todo!` / `unreachable` arm)

**Tier:** Unit / proptest over `n` (C1). Mutation target: the scope→target
derivation. **Observable:** `resolve_scope` return value. **Traces:** ADR-0084
§4.4; SD-4; the `WholeManaged`-dropped rationale (§2 — total resolver).

---

## Piece B — interest scenarios

### S-266-08 — An interested reconciler wakes when its observed rows change
`@dst @piece-b @property @contract-shape:bounded-change`
- **Given** reconciler `R` declares `interests() = &[ObservationRowKind::AllocStatus]`
- **When** an accepted `alloc_status` row for workload `W` is delivered on the
  change feed
- **Then** the router submits `(R, workload/W)`

**Tier:** Tier-1 DST (`@given` over `W`, over which reconcilers are interested).
**Observable:** `assert_eventually` `(R, workload/W)` in `broker.pending_keys`.
**Traces:** ADR-0084 §5 (fan-out); A2.

### S-266-09 — A host-state reconciler (empty interests) is never event-woken
`@dst @piece-b @error @contract-shape:bounded-change`
- **Given** a host-backed reconciler `H` using the default `interests()`
  (returns `&[]`)
- **When** any accepted `alloc_status` write is delivered on the change feed
- **Then** `H` is **never** submitted by the router (it is resync-only — the
  interest declaration IS the partition key)

**Tier:** Tier-1 DST (C5a partition; negative). **Observable:** `assert_always`
no `(H, *)` submit originates from the router. **Traces:** ADR-0084 §5.3; SD-6.

### S-266-10 — Migration preserves behaviour: the four consumers wake exactly as the deleted submits did
`@dst @piece-b @property @contract-shape:bounded-change`
- **Given** the four current consumers (`workload-lifecycle`,
  `backend-discovery-bridge`, `service-lifecycle`, `svid-lifecycle`) each declare
  `&[ObservationRowKind::AllocStatus]` and the four `exit_observer` submits are
  deleted
- **When** an accepted `alloc_status` transition for workload `W` is delivered
- **Then** the router's submit set equals **exactly**
  `{(workload-lifecycle, workload/W), (backend-discovery-bridge, workload/W),
  (service-lifecycle, workload/W), (svid-lifecycle, workload/W)}` — the same
  `(reconciler, target)` set the deleted `exit_observer.rs:234/254/295/320`
  submits produced

**Tier:** Tier-1 DST (the load-bearing migration-equivalence property).
**Observable:** the **full** `broker.pending_keys` set after the write equals the
named 4-set (Universe discipline: assert the whole set, not one member).
**Traces:** ADR-0084 §5 single-cut migration; Consequences (exit-observer tests
migrate to fan-out).

### S-266-11 — `ObservationRow::kind()` totally discriminates all 8 row variants to their `ObservationRowKind`
`@unit @property @contract-shape:pure-function`
- **Given** each of the 8 `ObservationRow` variants (`AllocStatus`, `NodeHealth`,
  `ServiceHydration`, `ServiceBackend`, `ReconcileConflict`, `IssuedCertificate`,
  `WorkflowTerminal`, `Signal`)
- **When** `row.kind()` runs
- **Then** each variant maps to its corresponding `ObservationRowKind` variant
  (`AllocStatus => ObservationRowKind::AllocStatus`, `NodeHealth =>
  ObservationRowKind::NodeHealth`, … all 8) — a **total, exhaustive, no-wildcard**
  projection

**Tier:** Unit / parametrize over all 8 variants (closed-world finite → parametrize,
not PBT per the falsifier-gate). Mutation target: **every `kind()` arm** (flip any
arm to a wrong `ObservationRowKind` variant → must be caught). **Observable:**
`ObservationRow::kind()` return per variant. **Traces:** ADR-0084 §2/§5
(`ObservationRow::kind()` total no-wildcard; the type owns its discriminant, owned
**beside `ObservationRow`** in `traits/observation_store.rs`). *Companion: a new
`ObservationRow` variant must fail compilation in `kind()` until mapped — a
compile-fail drift-closure expectation the crafter pins.*

### S-266-12 — An interested reconciler's `AllocStatus` row derives its workload-scoped target through the router
`@dst @piece-b @property @contract-shape:bounded-change`
- **Given** an accepted `alloc_status` row for workload `W` and a reconciler `R`
  interested in `ObservationRowKind::AllocStatus`
- **When** the router derives the broker target **inline from the row**
- **Then** the derived `TargetResource` is `workload/<W>` and `(R, workload/W)` is
  submitted

**Tier:** Tier-1 DST / proptest over `W`. This scenario now carries the **full weight
of the target-derivation mutation surface** — the pure-fn sibling (formerly S-266-21)
is gone; derivation is router-local. Mutation target: the router's inline
`AllocStatus(row) → workload/<row.workload_id>` derivation. **Observable:** derived
target in the submit. **Traces:** ADR-0084 §5 (fan-out → derive target inline →
submit).

### S-266-21 — [REMOVED in 2026-08-23 lean rework — derivation is router-local; covered by S-266-12 + S-266-10]
**Rationale (one line):** target derivation is no longer a standalone pure function —
the router derives the workload-scoped target **inline** from the `AllocStatus` row
(`ObservationRow::kind()` → table lookup → `workload/<row.workload_id>`), so there is
no pure-fn target-derivation sibling to pin directly. Its mutation surface is carried
by **S-266-12** (derive-through-router, proptest over `W`) and **S-266-10** (migration
equivalence — the full 4-consumer submit set). Kept as a tombstone (not renumbered) to
preserve traceability.

### S-266-13 — The fan-out fires on every accepted `alloc_status` write (equal-or-broader, no under-firing)
`@dst @piece-b @property @contract-shape:bounded-change`
- **Given** the four interested consumers, and a write population that includes
  **at least one write on the old `exit_observer` `RetryOutcome::Wrote` path** —
  the path whose old nudge set is the **non-empty** four-consumer set
  `{workload-lifecycle, backend-discovery-bridge, service-lifecycle, svid-lifecycle}`
  (per S-266-10), **not** `∅` — plus accepted writes the old path did not reach
- **When** any accepted `alloc_status` write occurs
- **Then** the fan-out submits for **every** accepted `alloc_status` write, never
  fewer targets than the deleted path (fan-out ⊇ old nudge set); and for the
  exit-observer-path write the submitted set is **⊇ the 4-consumer old nudge set**,
  so the ⊇ has teeth (a fan-out that dropped a consumer fails the containment
  against a non-empty old set — an ⊇ against `∅` would be vacuously true)

**Tier:** Tier-1 DST (property over accepted writes; the write population is
constrained to include ≥1 exit-observer-path write so `old-nudge-set` is the
non-empty 4-set, cross-ref S-266-10). **Observable:** for every accepted write,
`broker.pending_keys ⊇` the old nudge set; for the exit-observer-path write,
`broker.pending_keys ⊇ {(each of the 4 consumers, workload/W)}`. **Traces:**
ADR-0084 Consequences ("strictly more correct level-triggering"); feature-delta
§Negative; S-266-10 (the exact-equality counterpart).

### S-266-14 — After a lag gap the router relists and no interested target is left un-woken
`@dst @piece-b @error @contract-shape:bounded-change`
- **Given** the interested consumers and a change feed that emits
  `SubscriptionEvent::Lagged { missed }` mid-stream (rows dropped)
- **When** the router receives `Lagged`
- **Then** it relists the interested snapshot family (`alloc_status_rows()`) and
  re-submits per derived target — every interested target present in the snapshot
  is woken after the gap (no permanently-missed row)

**Tier:** Tier-1 DST (C7b interruption). **Observable:** `assert_eventually` post-
`Lagged` that every snapshot workload target has a submit. **Traces:** ADR-0084 §5
(`Lagged` → relist); `observation_store.rs:1734` mandatory `Lagged` handling.

### S-266-15 — List-then-Watch: pre-existing rows wake interested reconcilers without waiting for a change
`@dst @piece-b @error @contract-shape:bounded-change`
- **Given** interested consumers and a pre-existing `alloc_status` snapshot (rows
  written **before** the router subscribes)
- **When** `spawn_interest_router` boots (subscribe **first**, then list the
  snapshot)
- **Then** each pre-existing interested row yields a submit (an interested
  reconciler wakes without needing a subsequent change)
- **And** no accepted write in the subscribe→list boot window is missed

**Tier:** Tier-1 DST (C3 existing rows 0/1/many + boot-window edge). **Observable:**
`assert_eventually` a submit per snapshot workload target; `assert_always` no
missed boot-window write. **Traces:** ADR-0084 §5 steps 1-2 (subscribe-first closes
the `tokio::broadcast` boot-window gap).

### S-266-16 — A non-accepted (LWW-loser / no-op) write wakes nobody
`@dst @piece-b @error @contract-shape:bounded-change`
- **Given** the interested consumers
- **When** a rejected / no-op `alloc_status` write occurs (an LWW loser — the
  watcher never delivers it as a `Row`)
- **Then** the router submits nothing (fires on genuine accepted changes only,
  matching the exit-observer's "nudge only on change" gate)

**Tier:** Tier-1 DST (negative / robustness). **Observable:** `assert_always`
no submit for a non-delivered write. **Traces:** ADR-0084 §5 ("accepted write /
LWW winner only").

### S-266-17 — The interest hook is pure static routing metadata
`@unit @property @contract-shape:pure-function`
- **Given** the `Reconciler` trait after the additive `interests` method
- **When** the trait signature is pinned at compile time
- **Then** `interests(&self) -> &'static [ObservationRowKind]` takes only `&self`,
  returns borrowed `'static` data (no payload, no clock, no I/O), and `reconcile` is
  unchanged (`mod.rs:271` guard passes)

**Tier:** Unit / compile-time assertion. **Observable:** the type assertion
compiles; `interests` is `Copy`/borrowed-static. **Traces:** ADR-0084 §1;
feature-delta A7.

---

## Load-bearing DST invariants (headline acceptance targets)

### S-266-18 — Convergence reaches a fixpoint — action → write → wake → reconcile does not loop forever
`@dst @property @piece-a @piece-b @error @contract-shape:bounded-change`
- **Given** an interested, **convergent** reconciler `R` that authors no
  `alloc_status` rows
- **When** a full `Action → alloc_status write → fan-out wake → reconcile` cycle
  runs under `SimClock`
- **Then** the system reaches a fixpoint: reconcile eventually emits no
  self-perpetuating write and the broker quiesces to empty (no infinite re-wake)

**Tier:** Tier-1 DST. **Observable:** `assert_eventually` `broker.counters.queued
== 0` and stays 0 (quiescence). **Traces:** ADR-0084 §5 "Design rule (no busy-loop)"
(a)+(b); Consequences (acceptance-designer pins the fixpoint invariant).

### S-266-19 — No resync-storm: a redundant resync submit coalesces at the already-pending resync key
`@dst @property @piece-a @error @contract-shape:bounded-change`
- **Given** a reconciler `R` with `resync_schedule() = Some(ResyncSchedule {
  period: P, scope: LocalNode })` and the loop's local `NodeId = n`, with a prior
  eval already pending at the resync key `(R, node/n)` (an earlier resync fire, or
  an edge / self-re-enqueue submit at the same key, not yet drained)
- **When** a resync submit for `(R, node/n)` occurs while that prior eval at the
  **same key** `(R, node/n)` is still pending (a redundant same-key submit)
- **Then** the broker LWW-collapses to **≤ 1** pending eval at `(R, node/n)` and
  `broker.counters.cancelled` for that key increases by **exactly 1** — never
  fewer (a missed collapse) and never more (a double-count)

**Tier:** Tier-1 DST (or `eval_broker` unit) (C4a idempotency / C2b illegal
event-per-state). **Home:** Piece-A loop step (01-02). **Observable:**
`assert_always` `broker.pending_keys` holds ≤ 1 entry for `(R, node/n)`; the
**exact** `broker.counters.cancelled` delta for the `(R, node/n)` key is `1` per
redundant resync submit. **Traces:** ADR-0084 §4.3 Open-Question-5 verdict (resync
side); SD-3; C-A1/C-A2. *The fan-out write-flood collapse (the `workload/W` side)
is the sibling **S-266-22** — a `node/…` resync key and a `workload/…` fan-out key
can never share a broker key, so the two coalescing paths are asserted separately.*

### S-266-22 — No fan-out storm: a write-flood coalesces to one pending eval per distinct interested target
`@dst @property @piece-b @error @contract-shape:bounded-change`
- **Given** a reconciler `R` with `interests() = &[ObservationRowKind::AllocStatus]`
  for workload `W`, driven by the **live**
  interest router (`spawn_interest_router` → `broker.submit`)
- **When** `N` accepted `alloc_status` writes for the same workload `W` arrive so
  the interest-router submits `N` times at the same key `(R, workload/W)` before
  the broker drains
- **Then** the broker collapses to **≤ 1** pending eval at `(R, workload/W)` per
  drain and `broker.counters.cancelled` for that key increases by **exactly
  `N − 1`** — never fewer (a missed collapse) and never more (a double-count)

**Tier:** Tier-1 DST through the **live router** (C4a idempotency / C7 flood).
**Home:** the router step (02-02). **Observable:** `assert_always`
`broker.pending_keys` holds ≤ 1 entry for `(R, workload/W)` per drain; the
**exact** `broker.counters.cancelled` delta for the `(R, workload/W)` key is
`N − 1`. **Traces:** ADR-0084 §4.3 (broker coalescing) / §5 (fan-out on accepted
write → derive target → submit); SD-3. *The resync-side coalescing (the `node/…`
key) is the sibling **S-266-19** — the two keys never collide, so each coalescing
path is pinned separately.*

### S-266-20 — Single-clock determinism: seed → bit-identical cadence + fan-out trajectory
`@dst @property @piece-a @piece-b @contract-shape:bounded-change`
- **Given** a fixed seed, a fixed change-feed delivery order, and a fixed cadence
  schedule
- **When** the runtime replays the same inputs
- **Then** the trajectory of `(tick, submitted (reconciler, target) evals, dispatch
  order)` is **bit-identical** across replays (cadence next-wake bookkeeping and
  fan-out submits are deterministic under `SimClock`)

**Tier:** Tier-1 DST, `assert_replay_equivalent!`-style, seed printed on failure.
**Observable:** the full submit/dispatch trajectory. **Traces:** ADR-0084
Consequences (single-loop/single-clock DST preserved; no `ReflectorApplyBeforeHydrate`
needed under B-2); feature-delta A7.

### S-266-23 — Periodic relist recovers interested wakes after a transient boot-LIST error on a quiet stream (core liveness AT)
`@dst @property @piece-b @error @contract-shape:bounded-change`
- **Given** a `serve` restart where surviving allocations sit in the boot
  snapshot, an interest router with `interests() =
  &[ObservationRowKind::AllocStatus]`, and an injected `alloc_status_rows()`
  read failure on the boot LIST (a transient CR-SQLite DB-busy / lock
  contention — `register` submits no initial evaluation) plus a **quiet**
  change stream (no further accepted writes)
- **When** the injected `Clock` advances past `relist_period`
- **Then** the router's unconditional periodic relist re-reads the
  now-succeeding snapshot and submits one `Evaluation` per interested
  `(reconciler, workload/<id>)` — worst-case wake latency **one
  `relist_period`** (≤ 30 s), never the pre-amendment **unbounded** "until an
  unrelated write"

**Tier:** Tier-1 DST (`SimClock` + `SimObservationStore::
inject_alloc_status_rows_failure`). **Home:** the router step. **Observable:**
`assert_eventually` `broker` holds `(r-a, workload/w1)` only AFTER
`clock.tick(relist_period)`; before the tick the broker is empty (the boot LIST
logged-and-skipped). **Traces:** ADR-0084 § Amendment 2026-08-23 "The gap" +
"Decision"; SD-6 (the row-backed level-triggered backstop).

### S-266-24 — The periodic relist is unconditional-periodic, not idle-debounce: a `Row` arrival does not reset the deadline
`@dst @property @piece-b @contract-shape:bounded-change`
- **Given** an armed periodic relist with deadline `arm + relist_period` and a
  drained baseline broker
- **When** the clock advances halfway through the period, an accepted `Row`
  arrives (routed by the watch arm), and the clock then advances the remaining
  half — reaching exactly the ORIGINAL deadline
- **Then** the relist fires at the ORIGINAL `arm + relist_period` (the mid-period
  `Row` left `next_relist_at` UNCHANGED); an idle-debounce impl would have pushed
  the deadline to `half + relist_period` and NOT fired — the RED teeth
  distinguishing the two semantics

**Tier:** Tier-1 DST (`SimClock`). **Home:** the router step. **Observable:**
`assert_eventually` the relist submit appears after the clock reaches the
original deadline, despite the mid-period `Row`. **Traces:** ADR-0084 §
Amendment 2026-08-23 watch-loop semantic #3 (re-armed at most once per period;
`Row`/`Lagged`/`None` leave `next_relist_at` unchanged).

### S-266-25 — No relist storm: periodic-relist submits coalesce at the already-pending interested key
`@dst @property @piece-b @error @contract-shape:bounded-change`
- **Given** an interested reconciler `R` for workload `W`, a quiet stream, and an
  already-pending eval at `(R, workload/W)` (from the boot LIST) that is NOT
  drained
- **When** the clock advances `N` full `relist_period`s, driving `N` periodic
  relists that each re-submit `(R, workload/W)`
- **Then** the broker LWW-collapses to **≤ 1** pending eval at `(R, workload/W)`
  (never a storm) and `broker.counters.cancelled` increases by **exactly `N`**
  (one per redundant relist submit) — the router analogue of S-266-19 / S-266-22

**Tier:** Tier-1 DST (`SimClock`). **Home:** the router step. **Observable:**
`assert_always` `broker.counters.queued ≤ 1` across the `N` relists; the exact
`broker.counters.cancelled` reaches `N`. **Traces:** ADR-0084 § Amendment
2026-08-23 "No storm" (O(interested targets) coalesced submits once per period;
the acceptance-designer pins the ≤-once/period + coalesce + no-busy-loop
invariant — the router analogue of S-266-19).

---

## Contract-shape summary (2026-05-15 mandate)

Every scenario carries a `@contract-shape:` tag. Mapping mirrors the design
Reuse-Analysis contract-shape column:

| Shape | Scenarios | Component |
|---|---|---|
| `pure-function` | S-06, S-07, S-11, S-17 | `resync_schedule` / `interests` (return-only); `resolve_scope`; `ObservationRow::kind()` total discriminant |
| `bounded-change` | S-01, S-02, S-03, S-04, S-05, S-08, S-09, S-10, S-12, S-13, S-14, S-15, S-16, S-18, S-19, S-20, S-22 | loop next-wake table + `spawn_interest_router` — universe = `broker.submit((reconciler, target))` + `next_wake` writes |
| `unbounded-preservation` | **none** | No preview/dry-run/plan surface exists in this feature (design-confirmed) — the frame-problem "silent write" bug class is non-representable here by construction |

---

## Tier mapping

| Tier | Scenarios | Notes |
|---|---|---|
| **Tier-1 DST (PRIMARY, default lane, `Sim*`)** | S-01 (WS-composition), S-02, S-03, S-04, S-05, S-08, S-09, S-10, S-12, S-13, S-14, S-15, S-16, S-18, S-19, S-20, S-22 | `SimClock` + observation store; `assert_eventually`/`assert_always`; seed-reproducible. 17 scenarios. |
| **Unit / proptest / compile-time** | S-06 (signature), S-07 (`resolve_scope` proptest), S-11 (`ObservationRow::kind()` parametrize over 8 variants), S-17 (signature) | Pure-fn + trait-purity. 4 scenarios. |
| **Walking-skeleton / vertical slice** | S-01 | Boots `run_server_with_obs_and_driver` (Sim obs + `SimClock`); asserts it spawns `spawn_interest_router` (+ the cadence next-wake table); full `run_server` Lima boot is the fallback. |

Error/edge scenarios: S-03, S-04, S-09, S-14, S-15, S-16, S-18, S-19, S-22 = **9 / 21 = 43%** (≥ 40% target met; S-266-21 removed, S-266-11 reframed to a total discriminant so no longer `@error`).

---

## Mutation surface (DELIVER mandatory targets, `testing.md` §mutation)

1. **`ObservationRow::kind()` — every arm** (S-266-11). Flip any `kind()` arm to a
   wrong `ObservationRowKind` variant → must be caught.
2. **Cadence next-wake `<=` decision** (`next_wake[name] <= now`) + the
   `next_wake += period` re-arm (S-266-03, S-266-02). Swap `<=`↔`<`, drop the
   re-arm → caught.
3. **Interest-router routing** — the `ObservationRowKind → interested reconcilers`
   table lookup + the inline `AllocStatus(row) → workload/<row.workload_id>`
   derivation, DST-covered by S-266-08, S-266-10, S-266-12 (the pure-fn
   target-derivation sibling is gone — derivation is router-local).
4. **`resolve_scope(LocalNode, n) → node/<n>`** derivation (S-266-07).
5. **Broker coalescing** — the LWW key-collapse on `(ReconcilerName,
   TargetResource)` that both new submit sources route through, exercised on
   **both paths**: the resync side (`node/…` key) by S-266-19 and the fan-out side
   (`workload/…` key) by S-266-22. *(Existing `eval_broker.rs` surface; re-covered
   because both new submit sources depend on it — a `node/…` resync key and a
   `workload/…` fan-out key never collide, so each path is pinned separately.)*

Reconciler `reconcile` bodies remain a mutation surface per `testing.md`; the two
new methods are pure declarations (`resync_schedule`/`interests`) — cargo-mutants
may generate few/no mutants for a `const`-returning body, so criterion coverage
rests on S-06/S-07/S-11/S-17 asserting the returned data + the routing tests
asserting its consumption.

---

## Test-placement plan (crafter guidance — no files created here)

| Concern | Crate / dir | Lane |
|---|---|---|
| `ObservationRow::kind()` exhaustive over 8 variants (S-11) | `overdrive-core` (`crates/overdrive-core/src/traits/observation_store.rs`, beside `ObservationRow` + `ObservationRowKind`) co-located unit + the compile-fail drift-closure | default |
| `resolve_scope` totality (S-07) | `overdrive-core` (`crates/overdrive-core/src/reconcilers/mod.rs`) co-located unit | default |
| Trait purity / signature guards (S-06, S-17) | `overdrive-core/tests/` alongside the existing `reconciler_trait_signature_is_synchronous_no_async_no_clock_param` | default |
| Broker coalescing — resync side (S-19) + fan-out side (S-22) | `overdrive-core/src/eval_broker.rs` co-located + `overdrive-control-plane` DST driving `spawn_interest_router` (S-22) | default |
| Cadence submission / boundary / independence / determinism (S-02, S-03, S-04, S-05, S-20) | `overdrive-control-plane` DST tests driving `spawn_convergence_loop` under `SimClock` (or `overdrive-sim` DST invariant catalogue) | default (Tier-1) |
| Interest wake / migration-equivalence / Lagged / boot-window / no-op / fixpoint (S-08…S-16, S-18) | `overdrive-control-plane` DST driving `spawn_interest_router` + broker under `SimClock` + `Sim`/`Local` observation store | default (Tier-1) |
| Walking skeleton (S-01) | `overdrive-control-plane/tests/` booting the production entry `run_server_with_obs_and_driver` (`lib.rs:1447`) with Sim obs + `SimClock` and asserting it spawns `spawn_interest_router` (the router is wired by `run_server`, not the test); **primary lane:** Tier-1 DST booting the production entry; **fallback:** full `run_server` Lima boot. | default (Tier-1) or `integration-tests` |

**Pure-fn placement rationale (F2):** `ObservationRow::kind()` and `resolve_scope`
are pure fns over core types (`ObservationRow`/`ResyncScope`/`NodeId`/
`TargetResource`), dst-lint-clean, core default lane, mutation-testable without
`integration-tests`. `ObservationRow::kind()` (with its compile-fail drift-closure)
lives **beside `ObservationRow` + `ObservationRowKind`** in
`crates/overdrive-core/src/traits/observation_store.rs`; `resolve_scope` lives in
`crates/overdrive-core/src/reconcilers/mod.rs`, matching the roadmap placement. The
`workload/<id>` target derivation is **router-local** (no pure-fn sibling);
`spawn_interest_router` (the router that *calls* `kind()` and derives targets inline)
stays in `overdrive-control-plane`.

Migration cut (deleting `exit_observer.rs:234/254/295/320`): the pre-existing
`exit_observer` acceptance tests asserting "enqueues bridge/service/svid" are
**rewritten** in the same cut to assert the fan-out equivalence (S-266-10) —
per ADR-0084 Consequences. Deletion discipline: production submits and their
now-stale assertions go in one commit.

## RED-scaffold convention (document — the DELIVER crafter applies, not DISTILL)

Per `.claude/rules/testing.md` — **not** `.feature`, **not** `NotImplementedError`:

- **Test-side scaffold:** `#[should_panic(expected = "RED scaffold")]` with a
  `panic!("Not yet implemented -- RED scaffold (S-266-NN / <scenario>)")` body.
- **Production-side scaffold:** `todo!("RED scaffold: <one-line spec>")` gated with
  `#[expect(clippy::todo, reason = "RED scaffold; lands GREEN in step <id>")]`.
- Genuinely-external blockers only: `#[ignore = "reason"]`. Not for "impl doesn't
  exist yet."

## AT-completeness audit (Phase 2.5 — 15-item mechanical checklist)

| Item | Verdict | Evidence |
|---|---|---|
| C1a empty/zero/min | ✓ | S-04 (None schedule → 0 resync), S-09 (empty interests → 0 wake), S-15 (empty snapshot boot) |
| C1b partition boundary | ✓ | S-03 (`next_wake−ε` / `next_wake` / `+period`) |
| C2a state machine documented | ✓ | § "SUT state machines documented" (cadence next-wake + fan-out) |
| C2b illegal-event-per-state | ✓ | S-03 (no double re-arm within a period), S-16 (Row from a non-accepted write), S-19 (redundant resync submit at a pending `node/n` key), S-22 (write-flood at a pending `workload/W` fan-out key) |
| C3 count 0/1/N | ✓ | S-15 (0/1/many snapshot rows), S-10 (1 row → N interested reconcilers) |
| C4a apply-twice / idempotency | ✓ | S-22 (N writes for same W coalesce to 1), S-19 (redundant resync submit coalesces), S-02 (resync re-fire coalesces) |
| C4b inverse without prerequisite | ✓ | S-04 (None schedule → no cadence entry), S-09 (empty interests → no event-wake); a row kind no reconciler declared interest in yields no route (table lookup empty, S-11 `kind()` totality × S-09) |
| C5a mode-flag combos | ✓ | interest partition key × schedule: S-08/S-09 (non-empty vs empty interests), S-02/S-04 (Some vs None schedule); host-state = empty∧Some (resync-only) |
| C5b flag orthogonality | ✓ | S-05 (independent cadences), S-09 (interests don't drive cadence & vice-versa) |
| C6a malformed input | ✓ | S-11 (`kind()` totally handles every row family — no variant panics/`todo!`s), S-16 (non-accepted write) |
| C6b each declared error triggers | ✓ (rationale) | The fan-out has **no typed error-return surface** — it is a submit-or-no-op router; the one control signal is `Lagged`, exercised by S-14 (→ relist). Counted passing with documented rationale. |
| C6c closed error set | ✓ (rationale) | No error escapes the router (it submits or drops); `ObservationRow::kind()` returns a closed, total `ObservationRowKind` — no wildcard, compile-fail on an unmapped variant (S-11) — the "no other outcome" guarantee. |
| C7a degraded-resource | ✓ (rationale) | In-process framework; the resource-pressure analogue is broker flood — S-22 (fan-out write-flood coalesces) + S-19 (redundant resync coalesces). |
| C7b interruption mid-flow | ✓ | S-14 (`Lagged` mid-stream), S-15 (subscribe→list boot window) |
| C7c concurrent actors | ✓ (rationale) | Broker is single-threaded (`eval_broker.rs` header) — concurrency collapses to submit ordering; S-22 (concurrent fan-out writes coalesce) + S-19 (redundant resync coalesces) + S-20 (deterministic under a fixed delivery order) cover the multi-source case. |

**Passing: 15 / 15 → verdict COMPLETE (≥ 13).** All gaps are
`AT_GAP_IN_DELIVERY_SCOPE` (filled here); **zero `SPECIFICATION_AMBIGUITY`
blockers** — C2 (state machines), C5 (mode-flag partition key), C6 (closed
`ObservationRow::kind()`/`Lagged` contract) are each fully specified in ADR-0084 (§2/§4/§5, SD-6),
so no upstream re-entry is needed. Completeness telemetry:
`(266, C1-C7, 0 unfilled gaps, severity_max = none)`.

## `verification/` operator-catalogue note

Per `.claude/rules/verification.md`, **no expectation graduates to
`verification/expectations/`** for this feature. Piece A + Piece B are internal
reconciler-framework wiring with **no operator-observable surface** (no CLI verb,
no HTTP endpoint, no `overdrive describe`/`status` output change). The behaviour is
proven by the Tier-1 DST + unit tiers above (the "what, forever"); there is no
qualitative operator `why` to capture. Manufacturing an operator expectation here
would dilute the catalogue signal — explicitly declined.

## Traceability — ADR-0084 decision → scenarios

| ADR-0084 decision | Scenarios |
|---|---|
| §1 `resync_schedule` additive, pure, default `None` | S-02, S-04, S-06 |
| §1 `interests` additive, pure, default `&[]` | S-08, S-09, S-17 |
| §2 `ResyncScope::LocalNode` single-variant | S-07 |
| §2 `ObservationRowKind` single source of truth (total `ObservationRow::kind()`) | S-11, S-12 |
| §3 `AnyReconciler` forwarders (no erasure change) | S-06, S-17 (reconcile guard passes) |
| §4 loop next-wake table (C-A1 broker.submit, C-A2 once/period) | S-02, S-03, S-05, S-19 |
| §4.4 `resolve_scope(LocalNode)=node/<id>`, total | S-07 |
| §5 List-then-Watch (subscribe-first, list, watch) | S-15 |
| §5 step 3 `ObservationRow::kind()` exhaustive no-wildcard | S-11 |
| §5 fan-out on accepted write → derive target → submit | S-08, S-12, S-13, S-16, S-22 |
| §5 `Lagged` → relist | S-14 |
| §5 single-cut migration (delete 4 submits + 4 `interests()`) | S-10, S-01 |
| §5 no-busy-loop (author≠consumer, convergent) | S-18 |
| SD-3 / OQ5 broker coalesces resync + fan-out (no storm) | S-19 (resync side, `node/n` key), S-22 (fan-out side, `workload/W` key) |
| SD-6 empty interests ⟺ host-backed ⟺ resync-only | S-04, S-09 |
| A7 / Consequences single-loop/single-clock DST, purity preserved | S-06, S-17, S-20 |
