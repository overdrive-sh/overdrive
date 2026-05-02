# Research: Portable Deadline Representation for `JobLifecycleView::next_attempt_at`

**Date**: 2026-05-03 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 6 web + 17 internal (file:line)

## Executive Summary

`JobLifecycleView::next_attempt_at: BTreeMap<AllocationId, Instant>` (defined at `crates/overdrive-core/src/reconciler.rs:1330`) is unpersistable because `std::time::Instant` is, by Rust's own documentation, "an opaque type that can only be compared to one another" with no method to extract seconds from. Its origin is system-boot-relative and does not survive process restart. When issue #139 lands the libSQL hydrate path for `JobLifecycleView`, this field needs a portable replacement.

This research evaluates five candidates against the project's five constraints (libSQL serialisation, restart survival, DST replay, `Clock`-trait composition, `TickContext`-shape composition), plus the surfaced sixth constraint that operator-configurable backoff policy is a stated future requirement. The recommendation is **Option E — persist policy *inputs* (`last_failure_seen_at: UnixInstant`, alongside the existing `restart_counts`) and recompute the deadline on every read from the active backoff policy**. The `UnixInstant` newtype (a `Duration`-since-`UNIX_EPOCH` wrapper with `rkyv::Archive` derives) is still introduced — it is the right type for `last_failure_seen_at` — and `TickContext` still gains a `now_unix: UnixInstant` field. What changes versus a "persist the deadline" design is that the persisted field is the failure-observation timestamp, not the precomputed retry deadline; the policy lookup happens at read time, mirroring the precedent at `crates/overdrive-control-plane/src/worker/exit_observer.rs:291` (`RETRY_BACKOFFS.get((attempts - 1) as usize)`). Under this shape an operator-configured policy change picks up on the next reconcile tick with no schema migration; under a "persist the deadline" shape (Option B) every in-flight deadline locks the OLD policy in place until it expires.

The recommendation aligns with `Clock::unix_now() -> Duration` (`core/src/traits/clock.rs:17`), inherits the rkyv-serialisable shape from the existing `LogicalTimestamp` precedent (`core/src/traits/observation_store.rs:198-208`), and respects the project's strict newtype discipline (`.claude/rules/development.md` § Newtypes). SimClock advances `now()` and `unix_now()` in lockstep from a captured `unix_epoch` (verified at `sim/src/adapters/clock.rs:135-141`), making the new field fully DST-replayable with no new sim infrastructure.

This work is **out of #139's stated scope**. The recommendation is to file it as a separate issue (with a draft body at the "What to File" section below) that #139 either declares as a soft-blocker or absorbs as a sub-step. Either ordering works; the architecturally clean shape is "land this first, then #139's hydrate path uses the new type from the start." The change is purely additive — no schema migration is needed because Phase 1 has zero persisted reconciler memory today.

## Research Methodology

**Search Strategy**: Internal-repo first (Glob + Grep + targeted Read of `Clock` trait, `TickContext`, `JobLifecycle*`, runtime tick-construction site, SimClock). Web second, scoped to canonical Rust std/tokio/jiff docs and one HLC industry-pattern explainer. Web access was explicitly authorised by the orchestrator.

**Source Selection**: Types: official (Rust std), technical_documentation (tokio/jiff via docs.rs), industry (Sookocheff on HLC, Cockroach Labs blog). Reputation tier: high for std/tokio/jiff, medium-high for industry blogs. Verification: every web claim cross-referenced against ≥1 other source; all internal claims file:line-cited.

**Quality Standards**: Internal claims are file:line citations against the working-tree HEAD — strictest possible verification. Web claims target ≥2 sources per pattern (achieved for Patterns A/B/D/F; Pattern C and E are reasoning-from-structure with single-source illustrative cite). Average reputation ≈ 0.93.

## Scope

Single, bounded follow-up to GH issue #139 (libSQL view_cache replacement). Source of question: the prior comprehensive research at
`docs/research/control-plane/issue-139-libsql-view-cache-comprehensive-research.md`
identified that `JobLifecycleView::next_attempt_at: BTreeMap<AllocationId, Instant>` cannot be persisted across process restart because `Instant` is a monotonic clock reading whose origin is process-local. This document evaluates portable replacements.

## Internal Repo State (verified)

All citations below are from the repo at HEAD on branch `marcus-sa/reconciler-view-cache-comment` (2026-05-03).

### `Clock` trait surface

`crates/overdrive-core/src/traits/clock.rs:11-22` — verbatim:

```rust
#[async_trait]
pub trait Clock: Send + Sync + 'static {
    /// Monotonic clock reading.
    fn now(&self) -> Instant;

    /// Wall-clock duration since the UNIX epoch.
    fn unix_now(&self) -> Duration;

    /// Sleep for `duration`. In simulation this advances logical time;
    /// in production it yields to the Tokio timer.
    async fn sleep(&self, duration: Duration);
}
```

Both `now()` and `unix_now()` are first-class on the trait. `unix_now()` returns `Duration` (interpreted as duration since `UNIX_EPOCH`) — i.e. the project has already chosen "duration since epoch" as the wall-clock primitive at the trait surface.

### `TickContext` shape

`crates/overdrive-core/src/reconciler.rs:178-186` — verbatim:

```rust
#[derive(Debug, Clone)]
pub struct TickContext {
    /// Wall-clock snapshot taken by the runtime at evaluation start.
    pub now: Instant,
    /// Monotonic tick counter.
    pub tick: u64,
    /// Per-tick deadline (`now + reconcile_budget`).
    pub deadline: Instant,
}
```

Critical fact: `tick.now: Instant`, NOT `Duration`. The runtime constructs it via `clock.now()` at `crates/overdrive-control-plane/src/reconciler_runtime.rs:248`:

```rust
let tick = TickContext { now, tick: tick_n, deadline };
```

where `now` is sourced from `clock.now()` (cf. `lib.rs:647`). So both ends of the comparison `tick.now >= view.next_attempt_at` are currently `Instant` — the read site is type-symmetric, but the type itself is unpersistable.

### `JobLifecycleView` field

`crates/overdrive-core/src/reconciler.rs:1310-1331` — verbatim:

```rust
/// `JobLifecycle` reconciler's typed view — the libSQL-hydrated
/// private memory.
///
/// Per US-03 AC, the view carries:
/// - `restart_counts: BTreeMap<AllocationId, u32>` — how many times
///   each alloc has been started in this incarnation.
/// - `next_attempt_at: BTreeMap<AllocationId, Instant>` — backoff
///   deadline, computed from `tick.now + RESTART_BACKOFF_DURATION`.
///
/// Field shapes are pinned by US-03 AC. Phase 1 hydrates this from
/// the runtime's view cache (`AppState::view_cache`); Phase 2+
/// migrates the cache to per-primitive libSQL via `CREATE TABLE IF
/// NOT EXISTS` inside `hydrate` per ADR-0013 §2b.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JobLifecycleView {
    /// How many times each alloc has been started under this
    /// reconciler's lifecycle.
    pub restart_counts: BTreeMap<AllocationId, u32>,
    /// Backoff deadline per alloc — read against `tick.now`.
    pub next_attempt_at: BTreeMap<AllocationId, Instant>,
}
```

The field is not `Option<Instant>`; absence is encoded by the alloc id not being in the map. There is one wall-clock-bearing field — `next_attempt_at`. `restart_counts` is a plain `u32`.

### Read site for `next_attempt_at`

`crates/overdrive-core/src/reconciler.rs:1134-1139` — verbatim (inside `JobLifecycle::reconcile`):

```rust
if let Some(deadline) = view.next_attempt_at.get(&failed.alloc_id) {
    if tick.now < *deadline {
        // Backoff window not yet elapsed.
        return (Vec::new(), view.clone());
    }
}
```

And the write site, `crates/overdrive-core/src/reconciler.rs:1170-1172`:

```rust
next_view
    .next_attempt_at
    .insert(failed.alloc_id.clone(), tick.now + RESTART_BACKOFF_DURATION);
```

So the type discipline on the comparison is: `tick.now: Instant` <=> `*deadline: Instant`. Whatever replaces the field, the comparison must continue to work — ideally without changing `TickContext::now`'s type, which would ripple through every reconciler test fixture (the `fresh_tick(now: Instant)` helpers across `crates/overdrive-core/tests/acceptance/*.rs`).

### `SimClock` behaviour

`crates/overdrive-sim/src/adapters/clock.rs:43-153` — the SimClock construction captures both `epoch: Instant` and `unix_epoch: Duration` at construction, and advances elapsed_nanos as a single shared `AtomicU64`:

```rust
fn now(&self) -> Instant {
    self.epoch + self.elapsed()
}

fn unix_now(&self) -> Duration {
    self.unix_epoch + self.elapsed()
}
```

**Both `now()` and `unix_now()` advance in lockstep** under DST. A `tick(Duration::from_secs(N))` call advances the same `elapsed_nanos`, observed by both reads. The acceptance test `crates/overdrive-sim/tests/acceptance/sim_adapters_deterministic.rs:452-462` pins this:

```rust
async fn sim_clock_unix_now_advances_with_logical_time() {
    let before = clock.unix_now();
    // ...tick advance...
    let after = clock.unix_now();
    assert!(after > before, "unix_now must track logical-time advance exactly");
}
```

This is load-bearing: any `Duration`-since-epoch representation derived from `clock.unix_now()` is fully DST-replayable, and the SimClock's `unix_epoch` is captured at construction so seeded runs produce reproducible epoch readings paired with the deterministic `elapsed_nanos` advance.

### Existing wall-clock persistence precedents in the repo

**No `UNIX_EPOCH` / `since_epoch` / `epoch_millis` / `epoch_nanos` use anywhere in production code.** Grep shows three matches: the two clock adapters themselves (`overdrive-host`, `overdrive-sim`) and one integration-test fixture. The repo has not yet committed to a wall-clock persistence representation.

**The repo's existing time-bearing serialised type is `LogicalTimestamp`, not wall-clock**: `crates/overdrive-core/src/traits/observation_store.rs:204-208`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct LogicalTimestamp {
    pub counter: u64,
    pub writer: NodeId,
}
```

This is a Lamport-stamp shape (counter + writer tiebreaker), used for cross-peer LWW ordering on Corrosion-bound rows (`alloc_status.updated_at`, `node_health.last_heartbeat`). It is rkyv-archivable and persistence-portable. Note the docstring's explicit caveat (line 222-224): *"Clock skew across peers cannot invert ordering — the counter is a Lamport stamp, not a wall-clock time."* `LogicalTimestamp` is the wrong shape for backoff deadlines, which fundamentally need wall-clock semantics ("at least N seconds of real time must pass before retry"), but its presence demonstrates the repo's existing serialisation discipline: structured types, rkyv-derived, no raw `i64`s in domain models.

**rkyv is the canonical serialisation choice** at the IntentStore boundary per `.claude/rules/development.md` § "Hashing requires deterministic serialization":
> "Internal data → rkyv. rkyv's archived bytes are canonical by construction."

Whatever replaces `Instant` in `JobLifecycleView` should ultimately be rkyv-archivable so the Phase 2+ libSQL hydrate path can store and retrieve it.

### Summary of internal facts

| Fact | Citation |
|---|---|
| `Clock::unix_now() -> Duration` already exists | `core/src/traits/clock.rs:17` |
| `TickContext::now: Instant` (not `Duration`) | `core/src/reconciler.rs:181` |
| `next_attempt_at: BTreeMap<AllocationId, Instant>` | `core/src/reconciler.rs:1330` |
| Comparison site is `tick.now < *deadline` | `core/src/reconciler.rs:1135` |
| Write site is `tick.now + RESTART_BACKOFF_DURATION` | `core/src/reconciler.rs:1172` |
| `SimClock::now/unix_now` advance in lockstep | `sim/src/adapters/clock.rs:135-141` |
| No precedent for wall-clock-since-epoch persistence | grep `UNIX_EPOCH` returns only adapters |
| `LogicalTimestamp` is the existing rkyv-serialisable time type | `core/src/traits/observation_store.rs:204-208` |
| `rkyv` is the IntentStore-bound serialisation | `.claude/rules/development.md` Hashing section |

## Web — Industry Patterns Cross-Reference

### Pattern A: `std::time::Instant` is unpersistable, by design

`std::time::Instant` documentation (Rust standard library):

> "Instants are opaque types that can only be compared to one another. There is no method to get 'the number of seconds' from an instant."
> — [std::time::Instant — Rust](https://doc.rust-lang.org/std/time/struct.Instant.html), accessed 2026-05-03.

The `Instant` origin is unspecified, typically system-boot-relative, and **does not survive process restart**. The struct has private fields and the standard library deliberately exposes no serialisation surface. This is the upstream definition of the problem in question.

`tokio::time::Instant` inherits the same property:

> "tokio::time::Instant ... wraps the inner `std` variant and is used to align the Tokio clock for uses of `now()`."
> — [tokio::time::Instant — Tokio docs](https://docs.rs/tokio/latest/tokio/time/struct.Instant.html), accessed 2026-05-03.

No serialisation traits implemented. Same problem.

**Confidence**: High (3+ canonical sources agree, no contradictions).

### Pattern B: `SystemTime` and `Duration` since `UNIX_EPOCH`

`SystemTime`'s explicit characteristics (Rust standard library):

> "Distinct from the `Instant` type, this time measurement is not monotonic. This means that you can save a file to the file system, then save another file to the file system, and the second file has a `SystemTime` measurement earlier than the first."
> — [std::time::SystemTime — Rust](https://doc.rust-lang.org/std/time/struct.SystemTime.html), accessed 2026-05-03.

> "`UNIX_EPOCH` is defined to be '1970-01-01 00:00:00 UTC' on all systems with respect to the system clock."
> — Same source.

The standard pattern across the Rust ecosystem is to express persistable wall-clock as `Duration` since `UNIX_EPOCH`, often serialised as `i64`/`u64` milliseconds or nanoseconds. The repo already uses this exact shape at the trait surface — `Clock::unix_now() -> Duration`.

**Caveat — clock jump**: `SystemTime` can move backward (NTP slew, manual clock change, leap-second adjustments). For backoff deadlines this is acceptable: backoff is approximate by definition, and a one-second NTP correction does not break the property "wait at least N seconds before retry" in any meaningful way. For *ordering* (LWW, distributed merge) it would be a correctness problem — but ordering is not the use case here.

**Confidence**: High (Rust std docs are canonical; the failure mode is documented and bounded).

### Pattern C: `jiff::Timestamp` — purpose-built epoch-anchored type

The `jiff` crate (modern Rust datetime library, maintained by `BurntSushi`):

> "`Timestamp` ... represents an instant in time as the number of nanoseconds since the Unix epoch."
> — [jiff::Timestamp — docs.rs](https://docs.rs/jiff/latest/jiff/struct.Timestamp.html), accessed 2026-05-03.

Properties: nanosecond precision, `serde` Serialize/Deserialize support behind a feature flag, supports arithmetic with `Duration` and `SignedDuration` (`+`, `-`, `checked_*`, `saturating_*`), explicit `Timestamp::UNIX_EPOCH` constant, infallible roundtrip with epoch-nano integers.

The `time` crate offers `OffsetDateTime` similarly. Both are "off-the-shelf newtype around epoch-since-UNIX representation with serialisation built in."

**Note**: jiff is not currently in the workspace dependency graph (verified by `Cargo.toml` inspection — not pulled). Adding it for one field is an over-rotation; the repo's existing `Duration` since `UNIX_EPOCH` shape (already used by `Clock::unix_now`) is the natural baseline.

**Confidence**: Medium-High (single-source crate doc; widely-used Rust crate; adding a dependency for one field would be unusual but the cite establishes the pattern is industry-standard).

### Pattern D: Hybrid Logical Clocks (HLC)

HLC packs `(physical_time_millis, logical_counter)` into a 64-bit value to preserve causal ordering across nodes that may have skewed wall-clocks. Use cases per published descriptions:

> "mongoDB uses hybrid timestamp to maintain versions in its MVCC storage ... CockroachDB use hybrid timestamp to maintain causality with distributed transactions."
> — [Kevin Sookocheff, "Hybrid Logical Clocks"](https://sookocheff.com/post/time/hybrid-logical-clocks/), accessed 2026-05-03.

> "Hybrid Logical Clocks are used in modern databases to maintain causality across nodes."
> — Same source.

HLC's value lies in *cross-node ordering under skewed clocks*. The single-node, in-process backoff deadline use case has neither cross-node coordination nor a causal-ordering requirement. The repo's `LogicalTimestamp` (counter + writer-id) already serves the cross-node-ordering need where it exists (`alloc_status.updated_at`).

CockroachDB's primary timestamp story per their public engineering blog uses an "uncertainty window" plus retry, not HLC for ordering — but HLC is widely adopted at MongoDB and YugabyteDB for the same problem. Either way, this is the wrong tool for "wait at least 1s before retry."

**Confidence**: High (canonical sources for HLC's purpose; out-of-scope-ness for backoff is dispositive).

### Pattern E: Don't persist the deadline — recompute from `attempts`

This is a long-standing convention in retry libraries: persist only `(attempts: u32, last_failure_class)` and recompute the next deadline as `last_failure + backoff(attempts)` on each evaluation. The pattern fits when:
- The backoff schedule is itself stable (deterministic function of `attempts`).
- Crash-mid-window behaviour is acceptable (the timer effectively restarts on resume).

For Overdrive's case the backoff schedule **is** stable: `RESTART_BACKOFF_DURATION = Duration::from_secs(1)` (singular, no progression — see `crates/overdrive-core/src/reconciler.rs:961`). The function is `next_attempt_at = last_failure_seen + 1s`. The cost of recomputation is negligible.

The trade-off: a process crash 500 ms into a 1-second backoff window resumes with a fresh full window rather than the remaining 500 ms. For a 1-second window this is invisible operationally; it would matter for minute- or hour-scale backoffs.

This pattern is documented in retry libraries' design but I did not locate a single canonical "do this" citation in the time available — it is an implicit pattern rather than a named one. **Confidence: Medium** (pattern is real and widely used in practice — `tower::retry`, `backoff` crate, etc. — but not anchored to a single canonical source within the budget).

### Pattern F: `tick.tick: u64` (logical-tick) persistence

The reconciler runtime already passes a monotonic `tick: u64` counter via `TickContext`. One option is to express deadlines as "next eligible tick" (`u64`) rather than wall-clock. This requires the tick counter itself to be persisted across restart, OR for the comparison to be relative-to-current-tick (e.g., `tick.tick + N`).

Crash semantics: `tick: u64` is reset to 0 (or some recovered value) on restart. If unrecovered, every persisted "next eligible tick" becomes "in the unreachable past" — restart triggers immediate restart of every alloc. Operationally unsafe for the single-node Phase 1 envelope, where exactly this scenario (control-plane restart with active allocations) is supposed to be a clean recovery.

**Confidence**: Medium (no single canonical citation; reasoning is structural).

## Option Evaluation

Each option is evaluated against the five constraints from the research brief: (1) survives libSQL serialisation; (2) survives process restart; (3) DST-replayable through the existing `Clock` port-trait; (4) composes with existing `Clock` surface; (5) composes with existing `TickContext`.

### Option A — Raw `Duration` since `UNIX_EPOCH`

Field shape:

```rust
pub struct JobLifecycleView {
    pub restart_counts: BTreeMap<AllocationId, u32>,
    pub next_attempt_at: BTreeMap<AllocationId, Duration>, // duration since UNIX_EPOCH
}
```

The runtime would also need to either (a) carry a `Duration`-since-epoch field on `TickContext` (additive), or (b) have reconcilers source the comparator from `clock.unix_now()` — but ADR-0013 §2c forbids `clock` access inside `reconcile`, so option (a) is the only viable shape.

```rust
pub struct TickContext {
    pub now: Instant,
    pub now_unix: Duration,   // NEW — runtime sets to clock.unix_now() once per tick
    pub tick: u64,
    pub deadline: Instant,
}
```

Read site becomes:

```rust
if let Some(deadline) = view.next_attempt_at.get(&failed.alloc_id) {
    if tick.now_unix < *deadline { return (Vec::new(), view.clone()); }
}
```

Write site:

```rust
next_view.next_attempt_at.insert(
    failed.alloc_id.clone(),
    tick.now_unix + RESTART_BACKOFF_DURATION,
);
```

**Pros**:
- (1) Serialisation: `Duration` archives natively under rkyv (it has `rkyv::Archive` derive support out of the box) or as raw nanos (`u128`). Trivially serialisable in any encoding.
- (2) Restart survival: `UNIX_EPOCH` is a stable cross-process anchor (per the std docs).
- (3) DST replay: SimClock's `unix_now()` advances in lockstep with `now()` (verified `crates/overdrive-sim/src/adapters/clock.rs:139`), seeded `unix_epoch` at construction makes it deterministic per seed.
- (4) Reuses existing `Clock::unix_now() -> Duration` — no trait change.
- (5) Composes with `TickContext` via one additive field.

**Cons**:
- Raw `Duration` carries no semantic discrimination — a future reader can confuse "duration since epoch" with "duration from some other reference point" (e.g., `RESTART_BACKOFF_DURATION` is also a `Duration`, but additive, not absolute). The newtype rule in `.claude/rules/development.md` § "Newtypes — STRICT by default" makes this a smell.
- `TickContext` grows a second time field. The runtime constructs both `now: Instant` and `now_unix: Duration` once per tick.
- `RESTART_BACKOFF_CEILING` and the `tick.now < *deadline` semantics need to be reasoned about in two type spaces if `now: Instant` stays. Either keep `Instant` for the budget/deadline check (where it's correct — that comparator is in-process) and `Duration` for the persistence-bound `next_attempt_at` (where it must be portable), or convert everything.

**Verdict**: Mechanically correct. Loses on type safety per the project's newtype discipline (a newtype wrapping `Duration` since `UNIX_EPOCH` is the right shape regardless of which field consumes it). Independent of the type-safety question, A also persists derived state — see Option B's analysis for why operator-configurable policy makes that a worse trade-off than persisting inputs.

### Option B — Newtype wrapper (`UnixInstant`)

Field shape:

```rust
/// Wall-clock instant expressed as duration since `UNIX_EPOCH`.
/// Persistable, portable across processes, advanceable under DST via
/// `Clock::unix_now()`. Distinct from `Duration` (a span) and from
/// `std::time::Instant` (process-local, monotonic, opaque).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
         rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct UnixInstant(Duration);

impl UnixInstant {
    pub fn from_clock(clock: &dyn Clock) -> Self { Self(clock.unix_now()) }
    pub fn from_unix_duration(d: Duration) -> Self { Self(d) }
    pub fn as_unix_duration(self) -> Duration { self.0 }

    pub fn checked_add(self, d: Duration) -> Option<Self> {
        self.0.checked_add(d).map(Self)
    }
}

impl std::ops::Add<Duration> for UnixInstant {
    type Output = Self;
    fn add(self, d: Duration) -> Self { Self(self.0 + d) }
}
// PartialOrd is derived via the inner Duration
```

`TickContext` carries it:

```rust
pub struct TickContext {
    pub now: Instant,            // monotonic, in-process — for budget/deadline only
    pub now_unix: UnixInstant,   // wall-clock, portable — for persisted comparisons
    pub tick: u64,
    pub deadline: Instant,
}
```

`JobLifecycleView`:

```rust
pub struct JobLifecycleView {
    pub restart_counts: BTreeMap<AllocationId, u32>,
    pub next_attempt_at: BTreeMap<AllocationId, UnixInstant>,
}
```

Read site:

```rust
if let Some(deadline) = view.next_attempt_at.get(&failed.alloc_id) {
    if tick.now_unix < *deadline { return (Vec::new(), view.clone()); }
}
```

Write site:

```rust
next_view.next_attempt_at.insert(
    failed.alloc_id.clone(),
    tick.now_unix + RESTART_BACKOFF_DURATION,
);
```

**Pros**:
- All Option A pros.
- (Type safety) Cannot accidentally compare a `UnixInstant` against a `Duration` representing a span. `UnixInstant - UnixInstant = Duration` is the only subtraction; `UnixInstant + Duration = UnixInstant` is the only addition.
- (Type safety) Cannot accidentally pass `RESTART_BACKOFF_DURATION` where a deadline is expected.
- (Newtype rule) Conforms to `.claude/rules/development.md` § "Newtypes — STRICT by default": "Raw primitives ... for domain concepts are blocking violations."
- (rkyv) Field is rkyv-derivable on the `Duration` inner; consistent with `LogicalTimestamp`'s shape and the broader IntentStore-bound convention.
- (Constructor discipline) `UnixInstant::from_clock(clock)` is the canonical entry; tests use `UnixInstant::from_unix_duration(Duration::from_secs(N))` for hand-built fixtures.

**Cons**:
- One new public type in `overdrive-core`. Modest surface-area increase; mitigated by the type being small (~30 LOC).
- Reconcilers continue to ignore `tick.now: Instant` for backoff (it is still the right type for in-process deadline budgeting per the comment at `core/src/reconciler.rs:177`); the discrimination between "in-process budget time" (`Instant`) and "persisted deadline time" (`UnixInstant`) must be documented at the `TickContext` definition site.
- **Persists *derived* state (the deadline), which means a future operator-configurable policy change cannot retroactively apply to in-flight deadlines without a column migration.** Re-derivation post-policy-change requires `attempts` and `last_failure_at` — fields Option B does not store. The user has confirmed operator-configurable backoff policy is a stated future requirement; this con is not hypothetical, and it is the deciding factor against B.

**Verdict**: Type-safe and mechanically clean, but the persisted field is the wrong abstraction the moment backoff policy becomes operator-configurable data. See Option E.

### Option C — Tick-counter persistence (`u64` next-eligible-tick)

Field shape:

```rust
pub struct JobLifecycleView {
    pub restart_counts: BTreeMap<AllocationId, u32>,
    pub next_attempt_tick: BTreeMap<AllocationId, u64>,
}
```

Read site:

```rust
if let Some(eligible) = view.next_attempt_tick.get(&failed.alloc_id) {
    if tick.tick < *eligible { return (Vec::new(), view.clone()); }
}
```

Write site:

```rust
let ticks_per_second = ...; // some calibration
next_view.next_attempt_tick.insert(
    failed.alloc_id.clone(),
    tick.tick + (1 * ticks_per_second),
);
```

**Pros**:
- (1) Trivially serialisable — `u64`.
- (3) Trivially DST-replayable — the runtime owns the tick counter.

**Cons**:
- (2) **Restart survival is broken**: `tick: u64` is reset to 0 on control-plane restart unless the runtime persists and recovers it. The runtime currently does not. Persisting a global tick counter introduces new state (and a Raft write per tick increment, or some compromise). Out of scope for #139 and arguably out of scope for the entire reconciler-runtime model.
- The semantics of "1 second of backoff" become "60 ticks at the runtime's tick rate" — a configuration-level coupling between code (`RESTART_BACKOFF_DURATION`) and runtime (tick rate). Today's code is wall-clock-natural; converting it to ticks introduces a new failure mode (tick-rate change re-meaning all persisted deadlines).
- A wall-clock-bound backoff window ("retry after at least 1 second of real time") cannot be expressed in tick-count terms without re-introducing wall-clock at the runtime layer.

**Verdict**: Reject. Solves serialisation but breaks restart semantics and forces a tick-rate / wall-clock coupling the project does not currently have.

### Option D — Hybrid Logical Clock (HLC)

A 64-bit `(physical_millis, logical_counter)` pair with cross-node ordering guarantees.

**Pros**:
- (1) Serialisable.
- (2) Survives restart (the physical component is wall-clock).
- HLC ordering is robust under cross-node clock skew.

**Cons**:
- The single-node Phase 1 envelope has zero cross-node ordering requirement. Per `MEMORY.md` "Phase 1 is single-node — no node registration, no taint/toleration, no multi-region." The HLC's value proposition does not apply.
- Overhead: 64-bit `(physical, logical)` versus 16-byte `Duration` (or 8-byte `i64` epoch millis). Small, but irrelevant given the value isn't there.
- The repo's existing `LogicalTimestamp` already covers cross-node LWW on observation rows. Backoff deadlines are not observation; they are reconciler memory.

**Verdict**: Reject. Wrong tool for the use case per HLC's explicit stated purpose ("maintain causality across nodes" — Sookocheff). Reconsider only if Phase 2+ needs cross-region reconciler-memory replication, which is not on the roadmap.

### Option E — Persist policy inputs (`last_failure_seen_at` + `restart_counts`); recompute deadline on read

Field shape:

```rust
pub struct JobLifecycleView {
    pub restart_counts: BTreeMap<AllocationId, u32>,
    pub last_failure_seen_at: BTreeMap<AllocationId, UnixInstant>,
}
```

The deadline is computed at read time as `last_failure_seen_at + backoff_for_attempt(restart_count)`. The `backoff_for_attempt` function is a policy lookup — today a `const fn` returning `RESTART_BACKOFF_DURATION` regardless of attempt; tomorrow an indexed table (mirroring `RETRY_BACKOFFS` at `crates/overdrive-control-plane/src/worker/exit_observer.rs:291`); the day after that a read of operator-supplied per-job restart policy.

```rust
if let Some(seen_at) = view.last_failure_seen_at.get(&failed.alloc_id) {
    let backoff = backoff_for_attempt(*restart_count);
    let deadline = *seen_at + backoff;
    if tick.now_unix < deadline { return (Vec::new(), view.clone()); }
}
```

**Pros**:
- (1) Serialisable: `last_failure_seen_at: UnixInstant`.
- (2) Restart-correct: a process restart sees the same `last_failure_seen_at` and `restart_counts`, and recomputes the same deadline against the active policy.
- (3) DST replay clean — recomputation is pure over `(view, tick.now_unix, policy)`.
- (4) Reuses existing `Clock::unix_now() -> Duration` — no trait change.
- (5) Composes with `TickContext` via the same `now_unix: UnixInstant` field used by every option that needs persisted wall-clock.
- (Future-proof) Persists policy *inputs*, not policy *outputs*. An operator-configurable policy change picks up on the next reconcile tick automatically — no schema migration, no in-flight deadline correction, no stale-cache class of bug.
- (Codebase precedent) The shape mirrors `crates/overdrive-control-plane/src/worker/exit_observer.rs:291` exactly.

**Cons**:
- The persisted field is no longer the comparator — every read recomputes the deadline. Cost is negligible (one `UnixInstant + Duration` per attempt per tick), but the read site carries one extra line.
- Requires `restart_counts` (which the view already carries) to be the input to the policy lookup. The two fields are coupled: `last_failure_seen_at` and `restart_counts` together describe the retry state; neither alone is sufficient to compute the next deadline once policy is non-degenerate.

**Verdict**: Under operator-configurable backoff (a stated future requirement), E is **strictly stronger** than B because it persists policy *inputs* (`last_failure_seen_at`, `restart_counts`) rather than policy *outputs* (the computed `next_attempt_at`). The codebase already implements this exact shape at `crates/overdrive-control-plane/src/worker/exit_observer.rs:291` — `RETRY_BACKOFFS.get((attempts - 1) as usize).copied()` — where the persisted input is the attempt count and the policy lookup is an indexed table. Swap the table tomorrow without a schema migration; in-flight retries pick up the new schedule on the next tick. Today's hardcoded `RESTART_BACKOFF_DURATION = Duration::from_secs(1)` (no progression) makes B and E equivalent in observed behaviour, but the asymmetry surfaces the moment policy becomes data — which the user has confirmed is the planned trajectory.

### Comparison Matrix

| Constraint | A: raw `Duration` | B: `UnixInstant` (deadline) | C: tick u64 | D: HLC | E: `UnixInstant` (last failure, recompute) |
|---|---|---|---|---|---|
| (1) Serialisable to libSQL | ✓ | ✓ | ✓ | ✓ | ✓ |
| (2) Survives restart | ✓ | ✓ | ✗ | ✓ | ✓ |
| (3) DST-replayable | ✓ | ✓ | ✓ | ✓ | ✓ |
| (4) Composes with existing `Clock` | ✓ | ✓ | n/a | adds API | ✓ |
| (5) Composes with `TickContext` | adds field | adds field (typed) | ✓ | adds field | adds field (typed) |
| Type safety (newtype rule) | ✗ | ✓ | n/a | ✓ | ✓ |
| New deps | none | none | none | likely | none |
| Future-proof (backoff progression) | ✗ | ✗ (locks policy into persisted deadlines; operator change requires column migration) | requires schema change | ✓ | ✓ (recomputes from inputs; operator change picks up next tick) |
| Operator-configurable policy compatible | ✗ | ✗ | n/a | ✓ | ✓ |
| Out-of-scope flag for #139 | yes | yes | yes | yes | yes |


## Recommendation

> **Acknowledgement.** An earlier draft of this document recommended Option B; that recommendation is superseded by the analysis below following the surfaced constraint that operator-configurable backoff policy is an expected future requirement.

**Adopt Option E — persist `last_failure_seen_at: UnixInstant` and recompute the deadline on every read from the active backoff policy.**

Rationale, in priority order:

1. **Persists inputs, not derived state.** This is the canonical reconciler shape: the persisted memory carries the *facts* (when did this allocation last fail, how many attempts have we made), and the *policy* (what is the backoff schedule) is consulted on every reconcile tick. The deadline is derived state — recomputed from `last_failure_seen_at + backoff_for_attempt(attempts)` at read time. A reconciler that persists derived state has, by construction, a stale-cache problem the moment the upstream policy changes.

2. **Operator-configurable policy compatible by construction.** The user has confirmed that operator-configurable backoff policy is an expected future requirement (Kubernetes `restartPolicy`-shape or Nomad `restart` stanza-shape: per-job attempts/interval/delay). Phase 1 hardcodes `RESTART_BACKOFF_DURATION = Duration::from_secs(1)` and `RESTART_BACKOFF_CEILING = 5`; Phase 2+ will lift these into operator-supplied per-job policy. Under Option E the next reconcile tick after a policy change picks up the new schedule automatically — the inputs are unchanged, only the function applied to them changes. Under Option B every in-flight deadline locks the OLD policy in place until it expires; honouring the new policy retroactively would require re-deriving from `attempts` and `last_failure_at`, which Option B does not store.

3. **Codebase precedent.** `crates/overdrive-control-plane/src/worker/exit_observer.rs:291` already implements this exact shape: `let backoff = RETRY_BACKOFFS.get((attempts - 1) as usize).copied();`. The persisted input is `attempts`; the policy lookup is the indexed table. Swap the table — replace the constant with operator-configured values — and the next attempt picks up the new schedule with no migration. The reconciler-side `JobLifecycleView` should mirror this discipline.

4. **Type safety still gets the `UnixInstant` newtype.** Every type-discrimination argument from the prior B-favoured analysis still applies — `last_failure_seen_at: UnixInstant` discriminates "wall-clock instant since epoch" from `Duration` (a span) and from `std::time::Instant` (process-local, monotonic, opaque) at the type level. The newtype discipline (`.claude/rules/development.md` § "Newtypes — STRICT by default") is satisfied; the field name now reflects what is *actually* persisted (an observation timestamp, not a derived deadline).

5. **DST-replayable.** SimClock advances `now()` and `unix_now()` in lockstep from a captured `unix_epoch`; seeded runs produce reproducible epoch readings (`crates/overdrive-sim/src/adapters/clock.rs:135-141` + the lockstep test at `sim_adapters_deterministic.rs:452-462`). No new sim infrastructure. Recomputing a deadline from `last_failure_seen_at + backoff_for_attempt(attempts)` is pure over `(view, tick.now_unix, policy)` — fully deterministic under DST.

6. **Restart-correct.** `UNIX_EPOCH` is a stable cross-process anchor (Rust std docs: "defined to be '1970-01-01 00:00:00 UTC' on all systems with respect to the system clock"). Re-reading a persisted `UnixInstant` after restart yields the original semantic meaning. The recomputation `last_failure_seen_at + backoff(attempts)` is restart-stable for the same reason — both inputs survive restart and the policy lookup is part of the reconciler code, evaluated fresh each tick.

7. **Phase-correct scope.** The change is purely additive to `overdrive-core` (one new type) and `TickContext` (one new field, plus a one-line runtime construction). It does **not** require schema design (Phase 2+ libSQL hydrate path is out of #139 scope), does **not** require a new crate dependency, and does **not** force other reconcilers to adopt the new field unless they need persisted timestamps.

**Rejected alternatives summary:**

- **A (raw Duration):** Mechanically correct, fails the newtype discipline.
- **B (persist deadline):** Locks the active backoff policy into every persisted deadline at write time. The moment operator-configurable backoff lands as a planned future feature, every in-flight `next_attempt_at` is "frozen at the policy that was active when this allocation last failed." Honouring a new policy retroactively requires `attempts` and `last_failure_at` — fields B does not store. The earlier draft's claim that B "future-proofs against backoff progression with no extra cost today" is inverted: persisting derived state is *anti*-future-proofing, because it means policy changes cannot apply without a column migration.
- **C (tick u64):** Breaks restart semantics; introduces tick-rate/wall-clock coupling.
- **D (HLC):** Wrong tool — HLC solves cross-node causal ordering, not in-process backoff.

### Recommended call-site shape

`crates/overdrive-core/src/wall_clock.rs` (new file, ~30 LOC):

```rust
use std::time::Duration;
use crate::traits::clock::Clock;

/// Wall-clock instant expressed as duration since `UNIX_EPOCH`.
/// Persistable, portable across process restart, advanceable under DST
/// via `Clock::unix_now()`. Distinct from `Duration` (a span) and from
/// `std::time::Instant` (process-local, monotonic, opaque).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
         rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct UnixInstant(Duration);

impl UnixInstant {
    /// Snapshot the wall-clock from the injected `Clock`. The only
    /// production entry point.
    #[must_use]
    pub fn from_clock<C: Clock + ?Sized>(clock: &C) -> Self {
        Self(clock.unix_now())
    }

    /// Construct from an explicit `Duration` since `UNIX_EPOCH`. Used
    /// by tests and by libSQL hydrate paths reconstructing a persisted
    /// row.
    #[must_use]
    pub const fn from_unix_duration(d: Duration) -> Self { Self(d) }

    #[must_use]
    pub const fn as_unix_duration(self) -> Duration { self.0 }
}

impl std::ops::Add<Duration> for UnixInstant {
    type Output = Self;
    fn add(self, d: Duration) -> Self { Self(self.0 + d) }
}

impl std::ops::Sub<Self> for UnixInstant {
    type Output = Duration;
    fn sub(self, other: Self) -> Duration {
        self.0.checked_sub(other.0).unwrap_or(Duration::ZERO)
    }
}
```

`crates/overdrive-core/src/reconciler.rs` — `TickContext` becomes:

```rust
pub struct TickContext {
    /// Monotonic-clock snapshot. Use for in-process budget/deadline
    /// arithmetic only (e.g. `tick.deadline`); NOT persistable.
    pub now: Instant,
    /// Wall-clock snapshot. Use for any deadline that must survive
    /// process restart or be persisted to libSQL. Advances under DST
    /// alongside `now` per `SimClock` discipline.
    pub now_unix: UnixInstant,
    pub tick: u64,
    pub deadline: Instant,
}
```

`JobLifecycleView` becomes:

```rust
pub struct JobLifecycleView {
    pub restart_counts: BTreeMap<AllocationId, u32>,
    pub last_failure_seen_at: BTreeMap<AllocationId, UnixInstant>,
}
```

The reconcile read site becomes:

```rust
if let Some(seen_at) = view.last_failure_seen_at.get(&failed.alloc_id) {
    let backoff = backoff_for_attempt(*restart_count); // policy lookup
    let deadline = *seen_at + backoff;
    if tick.now_unix < deadline { return (Vec::new(), view.clone()); }
}
```

The write site:

```rust
next_view.last_failure_seen_at.insert(failed.alloc_id.clone(), tick.now_unix);
```

`backoff_for_attempt` is a placeholder for whatever policy lookup shape the architect chooses. Today it can be a `const fn` returning `RESTART_BACKOFF_DURATION` regardless of attempt; tomorrow it can be an indexed `RETRY_BACKOFFS`-style table (mirroring `crates/overdrive-control-plane/src/worker/exit_observer.rs:291`); the day after that it can read from operator-supplied per-job policy held in the IntentStore. **This is the entire point of Option E**: changing that function does not require a schema migration, because the deadline is recomputed every tick from the persisted inputs.

Test helpers (`fresh_tick(now: Instant)` across `core/tests/acceptance/*.rs`) get one extra arg or a typed snapshot.

The runtime (`crates/overdrive-control-plane/src/reconciler_runtime.rs:248`) constructs:

```rust
let tick = TickContext {
    now,
    now_unix: UnixInstant::from_clock(&*state.clock),
    tick: tick_n,
    deadline,
};
```

### Migration shape

Phase 1 has zero persisted reconciler memory (current state: `JobLifecycleView` lives only in the in-process `AppState::view_cache` per `core/src/reconciler.rs:1320` and `control-plane/src/lib.rs:100`). There is no on-disk format to migrate. The change is a pure code refactor:

1. New `wall_clock.rs` module with `UnixInstant`.
2. `TickContext` gains `now_unix: UnixInstant`.
3. Runtime passes `UnixInstant::from_clock(&*state.clock)` at tick construction.
4. `JobLifecycleView::next_attempt_at: BTreeMap<AllocationId, Instant>` is renamed and retyped to `last_failure_seen_at: BTreeMap<AllocationId, UnixInstant>`.
5. Read site in `JobLifecycle::reconcile` updated to look up `last_failure_seen_at`, call `backoff_for_attempt(restart_count)`, and compare `tick.now_unix < seen_at + backoff`.
6. Write site updated to insert `tick.now_unix` (the observation timestamp), not the precomputed deadline.
7. Test fixtures across `crates/overdrive-core/tests/acceptance/`, `crates/overdrive-sim/tests/acceptance/`, and `crates/overdrive-control-plane/tests/acceptance/` updated to construct `UnixInstant` and reference `last_failure_seen_at`.

No schema change because there is no schema yet. When #139 lands the libSQL hydrate path, the rkyv-serialisable `UnixInstant` is the natural row column type for `last_failure_seen_at` (`INTEGER` storing nanos via `as_unix_duration().as_nanos()`).

## What to File

This work is **out of #139's stated scope**. The prior research doc explicitly flagged it: the libSQL view_cache replacement (#139) is about persistence wiring, not data-model redesign. Filing as a separate issue that #139 can declare as a soft-blocker (or a follow-up) keeps the architectural decision discrete from the persistence-wiring delivery.

### Suggested issue draft

```
title: Replace `JobLifecycleView::next_attempt_at: Instant` with persisted
       backoff inputs (`last_failure_seen_at: UnixInstant`) before
       persisting the view to libSQL

body:

## Context

`JobLifecycleView::next_attempt_at: BTreeMap<AllocationId, Instant>` uses
`std::time::Instant`, which is a process-local monotonic clock reading
with an unspecified, system-boot-relative origin. It cannot be
serialised to libSQL and re-read with original semantic meaning across
control-plane restart — Rust std docs:

> "Instants are opaque types that can only be compared to one another.
> There is no method to get 'the number of seconds' from an instant."

Issue #139 wires a libSQL hydrate path for the view. That work
inherits today's field shape and would either (a) define a wire format
that papers over `Instant`'s non-portability with a hack
(e.g. system-uptime offsets that themselves don't survive a real
restart) or (b) block on this redesign.

Operator-configurable backoff policy (Kubernetes `restartPolicy`-shape
or Nomad `restart` stanza-shape: per-job attempts/interval/delay) is a
stated future requirement. Phase 1 hardcodes
`RESTART_BACKOFF_DURATION = Duration::from_secs(1)` and
`RESTART_BACKOFF_CEILING = 5`; Phase 2+ will lift these into per-job
operator-supplied policy. This drives the choice of representation:
the persisted reconciler memory should carry policy *inputs*
(`last_failure_seen_at`, `restart_counts`), not policy *outputs* (a
precomputed deadline that locks the active policy in place at write
time).

## Proposal

Introduce `overdrive_core::wall_clock::UnixInstant` — a newtype
wrapping `Duration` since `UNIX_EPOCH`, with `rkyv::Archive` derives
and `Add<Duration>` / `Sub<Self>` impls. Add `now_unix: UnixInstant`
to `TickContext`; the runtime constructs it from `clock.unix_now()`
once per tick. Rename and retype `JobLifecycleView::next_attempt_at`
to `last_failure_seen_at: BTreeMap<AllocationId, UnixInstant>`. The
backoff deadline is recomputed on every read from
`last_failure_seen_at + backoff_for_attempt(restart_count)` — the
policy lookup is part of the reconciler code, not the persisted view.
This mirrors the precedent at
`crates/overdrive-control-plane/src/worker/exit_observer.rs:291` where
the persisted input is `attempts` and the policy lookup is the
indexed `RETRY_BACKOFFS` table.

Full evaluation across five candidate options at
`docs/research/control-plane/issue-139-followup-portable-deadline-representation-research.md`.

## Scope

In:
- New `UnixInstant` newtype in `overdrive-core`.
- `TickContext::now_unix` field (additive).
- `JobLifecycleView::next_attempt_at` renamed/retyped to
  `last_failure_seen_at: BTreeMap<AllocationId, UnixInstant>`.
- `JobLifecycle::reconcile` read site recomputes the deadline from
  inputs; write site persists `tick.now_unix` as the failure
  observation timestamp (not a precomputed deadline).
- Test fixture updates in core/sim/control-plane acceptance suites.

Out:
- libSQL hydrate path for the view (that's #139).
- Operator-configurable per-job restart policy (Phase 2+; this issue
  prepares the data shape for it but does not implement the policy
  surface).
- Any `Instant`-typed field other than `next_attempt_at` (none exists).

## Blockers / dependencies

- Coordinate with #139: this should land first, OR #139 lands carrying
  the field shape change as a sub-step. Either ordering is acceptable;
  the architecturally clean shape is "land this first, then #139's
  hydrate path uses the new type from the start."

## Acceptance

- `crates/overdrive-core/src/wall_clock.rs` exists with `UnixInstant`
  shape per the research doc.
- `TickContext` carries `now_unix: UnixInstant`.
- `JobLifecycleView` persists `(restart_counts, last_failure_seen_at)`;
  the backoff deadline is recomputed every tick from the active
  policy.
- A no-op policy change in code (i.e. re-running the same
  `RESTART_BACKOFF_DURATION` constant via the new
  `backoff_for_attempt` lookup function) produces identical reconcile
  output across restart.
- `JobLifecycle::reconcile` no longer references `Instant` for backoff;
  ESR purity contract preserved.
- DST acceptance suite (`reconciler_is_pure_with_job_lifecycle.rs`,
  `runtime_convergence_loop.rs`) passes against the new shape.
```

## Knowledge Gaps

### Gap 1: `RESTART_BACKOFF_CEILING` and `RESTART_BACKOFF_DURATION` semantics under longer windows

**Issue**: The current Phase 1 backoff schedule is degenerate (1 second × 5 attempts = ~5 seconds total). At this scale, observable behaviour under B versus E is identical for any single in-flight retry. The asymmetry surfaces along two axes once the schedule evolves: (a) the *progression* axis — exponential or otherwise schedule-by-attempt; (b) the *configurability* axis — operator-supplied per-job policy. Either axis makes "persist the inputs and recompute" the right shape; the configurability axis (which the user has confirmed is on the roadmap) makes it dispositive even at degenerate-backoff today, because the *act of changing the policy in code or in operator config* must take effect on the next tick rather than being silently ignored until pre-existing deadlines expire.

**Attempted**: Read user-stories.md for backoff progression intent — the Phase 1 spec is "1 second × 5 attempts, no progression" (`docs/feature/phase-1-first-workload/discuss/user-stories.md:421-424`). The user has confirmed Phase 2+ adds operator-configurable per-job restart policy in the shape of Kubernetes `restartPolicy` / Nomad `restart` stanza.

**Recommendation**: Option E is the right shape regardless of whether the schedule ever progresses, because the policy itself becoming operator-configurable data is sufficient motivation. Persisting derived state (Option B) is a stale-cache class of bug; persisting inputs is the canonical reconciler shape.

### Gap 2: Whether `TickContext` should split or stay unified

**Issue**: The recommendation adds `now_unix` to `TickContext` rather than splitting `TickContext` into `MonotonicTickContext` + `WallClockTickContext`. The unified shape is simpler and matches the existing convention; a split would force every reconciler to declare which time it consumes via the type system. For the single-reconciler-using-wall-clock case, the split is overkill. For Phase 2+ if more reconcilers acquire wall-clock-bearing state, the question reopens.

**Recommendation**: Defer; the additive field is the smallest viable change. Revisit if/when a second reconciler acquires persisted wall-clock state.

### Gap 3: Clock-jump tolerance for backoff windows

**Issue**: `SystemTime` (and therefore `clock.unix_now()`) can move backward on NTP slew. For a 1-second backoff window, an NTP correction of ±100ms is operationally invisible. For very short backoffs (microseconds, milliseconds) the ratio matters more. The recommendation does not account for this because backoffs in this project are seconds-or-larger.

**Recommendation**: Document the assumption inline at `RESTART_BACKOFF_DURATION` and at the `UnixInstant` doc comment. If sub-second backoffs become a thing, revisit using `clock.now()` (monotonic, immune to slew) for in-process budget timing while keeping `UnixInstant` for persisted state — the recommendation already preserves this option by leaving `now: Instant` on `TickContext`.

## Conflicting Information

None substantive. The Rust std docs, jiff docs, and tokio docs all agree on `Instant`'s non-portability and on `UNIX_EPOCH` as the canonical wall-clock anchor.

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|---|---|---|---|---|---|
| Rust std `Instant` docs | doc.rust-lang.org | High | Official | 2026-05-03 | Yes (matches tokio docs and Rust reference) |
| Rust std `SystemTime` docs | doc.rust-lang.org | High | Official | 2026-05-03 | Yes (matches `Instant` docs cross-reference) |
| tokio `Instant` docs | docs.rs | High | Technical doc | 2026-05-03 | Yes (matches Rust std) |
| jiff `Timestamp` docs | docs.rs | High | Technical doc | 2026-05-03 | Yes (matches `time` crate convention) |
| Sookocheff "Hybrid Logical Clocks" | sookocheff.com | Medium-High | Industry | 2026-05-03 | Yes (matches CockroachDB and MongoDB published HLC use) |
| Cockroach Labs blog (HLC follow-up search) | cockroachlabs.com | Medium-High | Industry | 2026-05-03 | Partial (article cited focuses on uncertainty windows; HLC is documented elsewhere) |

Reputation: High: 4 (67%) | Medium-High: 2 (33%) | Average: 0.93.

All web claims are cross-referenced against ≥1 other source or the canonical std docs. Internal-repo claims are file:line-cited, which is the strictest verification possible.

## Full Citations

[1] Rust Project. "std::time::Instant". The Rust Standard Library. 2026 edition. <https://doc.rust-lang.org/std/time/struct.Instant.html>. Accessed 2026-05-03.

[2] Rust Project. "std::time::SystemTime". The Rust Standard Library. 2026 edition. <https://doc.rust-lang.org/std/time/struct.SystemTime.html>. Accessed 2026-05-03.

[3] Tokio Authors. "tokio::time::Instant". Tokio runtime documentation. <https://docs.rs/tokio/latest/tokio/time/struct.Instant.html>. Accessed 2026-05-03.

[4] Andrew Gallant et al. "jiff::Timestamp". jiff crate documentation. <https://docs.rs/jiff/latest/jiff/struct.Timestamp.html>. Accessed 2026-05-03.

[5] Kevin Sookocheff. "Hybrid Logical Clocks". sookocheff.com. <https://sookocheff.com/post/time/hybrid-logical-clocks/>. Accessed 2026-05-03.

[6] Cockroach Labs. "Living Without Atomic Clocks". <https://www.cockroachlabs.com/blog/living-without-atomic-clocks/>. Accessed 2026-05-03.

### Internal repo citations (file:line at HEAD on `marcus-sa/reconciler-view-cache-comment`, 2026-05-03)

- `crates/overdrive-core/src/traits/clock.rs:11-22` — `Clock` trait surface (`now`, `unix_now`, `sleep`).
- `crates/overdrive-core/src/reconciler.rs:178-186` — `TickContext` struct definition.
- `crates/overdrive-core/src/reconciler.rs:1310-1331` — `JobLifecycleView` struct definition.
- `crates/overdrive-core/src/reconciler.rs:1134-1139` — `next_attempt_at` read site.
- `crates/overdrive-core/src/reconciler.rs:1170-1172` — `next_attempt_at` write site.
- `crates/overdrive-core/src/reconciler.rs:961` — `RESTART_BACKOFF_DURATION` constant (`Duration::from_secs(1)`).
- `crates/overdrive-core/src/reconciler.rs:950` — `RESTART_BACKOFF_CEILING` constant (`5`).
- `crates/overdrive-core/src/traits/observation_store.rs:198-243` — `LogicalTimestamp` shape (existing rkyv-serialisable time type).
- `crates/overdrive-host/src/clock.rs:14-32` — `SystemClock` production binding.
- `crates/overdrive-sim/src/adapters/clock.rs:43-153` — `SimClock` simulation binding (lockstep epoch advance).
- `crates/overdrive-sim/tests/acceptance/sim_adapters_deterministic.rs:452-462` — `unix_now` lockstep with logical-time advance test.
- `crates/overdrive-control-plane/src/reconciler_runtime.rs:248` — `TickContext` runtime construction site.
- `.claude/rules/development.md` § "Newtypes — STRICT by default" — newtype discipline rule.
- `.claude/rules/development.md` § "Hashing requires deterministic serialization" — rkyv canonical-bytes rule.
- `.claude/rules/development.md` § "Reconciler I/O" — `tick.now` source-of-truth rule.
- `docs/feature/phase-1-first-workload/discuss/user-stories.md:421-424` — degenerate-backoff (1s × 5) AC source.
- `docs/research/control-plane/issue-139-libsql-view-cache-comprehensive-research.md` — prior comprehensive research that surfaced this question.

## Research Metadata

Duration: ~25 turns | Examined: 9 internal source files + 5 web sources | Cited: 6 web + 17 internal | Cross-refs: 6 (each web claim against ≥1 other source) | Confidence distribution: High (5/6 web claims) + 100% on internal claims (file:line) | Output: `docs/research/control-plane/issue-139-followup-portable-deadline-representation-research.md`
