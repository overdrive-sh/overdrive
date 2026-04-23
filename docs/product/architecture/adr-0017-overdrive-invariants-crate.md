# ADR-0017 — `overdrive-invariants` crate: first-class invariant taxonomy shared across DST, real-kernel, chaos, agent exerciser, and verification targets

## Status

Proposed. 2026-04-23.

## Context

Whitepaper §18 commits the platform to two mechanically-checkable
correctness properties:

1. **Eventually Stable Reconciliation (ESR)** for reconcilers — progress
   (converges) and stability (remains converged) expressible as a
   temporal-logic formula over `reconcile`'s pre/post-state.
2. **Replay-equivalence + bounded progress** for workflows — given the
   same journal prefix, a workflow replays to bit-identical state, and
   terminates within a declared step budget.

`.claude/rules/testing.md` operationalises both as in-body assertion
macros — `assert_always!`, `assert_eventually!`, `assert_replay_equivalent!`
— and enumerates a fault-injection catalogue that is **exercised
simultaneously by DST (Tier 1) and the chaos reconciler in production**.
That shared catalogue is already in the rules doc; the *invariants* it
is checked against are not yet shared. They live inline in
`crates/overdrive-sim/src/invariants/` (see ADR-0004), which is the
wrong home for anything consumed outside the DST harness.

Four distinct consumers already need the same invariant artifact, and a
fifth is on the horizon:

| Consumer | Status | Notes |
|---|---|---|
| Tier 1 DST (turmoil + Sim\*) | Shipping — ADR-0004 | Invariants live inline in `overdrive-sim` |
| Tier 3 real-kernel integration (QEMU) | Planned — testing.md §Tier 3 | Needs to assert the same safety/convergence properties against real kernel traces |
| Chaos reconciler (whitepaper §18) | Planned — Phase 3+ | §21 fault catalogue already shared with DST; invariants must follow |
| Tier 3.5 agent-driven exerciser | Proposed — Deliverable 2 | Separation-of-authorship demands invariants *not* authored by the exerciser |
| Verus/Anvil-style ESR verification | Pilot — ADR-0018 | Spec surface must be expressible as a Verus `spec fn` over state |

The research question `docs/research/testing/invariant-observer-patterns-comprehensive-research.md`
(Nova, 2026-04-23) answers this with the industry pattern: **invariants
are a separate artifact from both the code-under-test and the scenarios
that exercise it**. The strongest evidence is the 2024–2025 "homogenization
trap" literature (Finding 4.1) showing that LLM-authored tests against
LLM-authored code share error patterns — the independent oracle is what
earns its keep. The P language's "specification machines" at AWS scale
(Finding 1.6), Jepsen's checker/generator/nemesis decomposition
(Finding 1.7), Antithesis's SDK-portable assertions (Finding 2.1), and
FoundationDB's CHECK-phase externalisation (Finding 1.8) are four
independent industrial deployments of the same pattern.

The research's Synthesis S1 recommends extraction into a dedicated
crate with a five-class taxonomy (Safety / Liveness / Convergence-ESR /
Strong Eventual Consistency / Replay-Equivalence), citing academic and
industrial sources per class. This ADR decides how to land that
recommendation inside Overdrive's ADR-0003 crate-class model and the
ADR-0004 single-sim-crate shape.

## Decision

### 1. Create `crates/overdrive-invariants` with class `core`

```toml
# crates/overdrive-invariants/Cargo.toml
[package.metadata.overdrive]
crate_class = "core"
```

The crate lives next to `overdrive-core` and depends only on it. It
declares no dependency on `turmoil`, `tokio`, `rkyv`, `redb`, or any
I/O-bearing crate — by ADR-0003, `core` crate class is DST-lint-scanned
for banned APIs, and that scan is what enforces the reusability
promise. An invariant that could not be evaluated in a simulation
context (e.g. needs wall-clock) would fail the lint; an invariant that
can be evaluated over pure state is trivially reusable across every
consumer in the table above.

**Why not `adapter-sim`**: `adapter-sim` (ADR-0004) is the home for
turmoil-dependent wiring and is banned from the production compile
path. Invariants ride the production compile path (chaos reconciler
in §18 evaluates them live). The crate must be `core`.

**Why not a new class**: ADR-0003's four-class model is deliberately
closed; adding a fifth (e.g. `spec`) would propagate through `dst-lint`,
acceptance tests, and every doc referencing the taxonomy. The existing
`core` class satisfies the constraints the invariant crate needs
(dependency-free, scan-subject, no I/O). No new class justified.

### 2. Five-class taxonomy as a typed enum, not a trait

```rust
// crates/overdrive-invariants/src/lib.rs
pub enum InvariantClass {
    /// "Something bad never happens." Falsifiable by a finite prefix.
    /// Academic basis: Alpern & Schneider 1985.
    Safety,
    /// "Something good eventually happens." Infinite-suffix falsifiable,
    /// requires fairness. Academic basis: Alpern & Schneider 1985.
    Liveness,
    /// Liveness specialisation: reaches and remains at desired state.
    /// Academic basis: Anvil OSDI '24 (ESR).
    Convergence,
    /// All replicas that have seen the same writes agree.
    /// Whitepaper §4 — Corrosion / CR-SQLite.
    StrongEventualConsistency,
    /// Journal + code produces bit-identical trajectories.
    /// Whitepaper §18 — workflow primitive.
    ReplayEquivalence,
}
```

**Enum, not trait**: the taxonomy is closed by the research evidence
(five classes, traceable to cited sources per class). A trait surface
would invite a sixth class declared ad hoc and invalidate the
"closed-set exhaustive `match`" proof pattern that makes the evaluator
dispatch type-safe. Jepsen's Elle (Finding 1.7) chose the same shape —
consistency models as first-class enum variants rather than generic
predicates — and the research cites this as concrete evidence that
taxonomic structure pays off for shrinking and reporting quality.

### 3. Invariant specification data type

```rust
pub struct InvariantSpec<S: State> {
    pub name:           InvariantName,        // kebab-case newtype
    pub class:          InvariantClass,
    pub predicate:      Predicate<S>,
    pub failure_policy: FailurePolicy,
    pub scope:          InvariantScope,       // where it may be evaluated
}

pub enum Predicate<S: State> {
    /// Pure state predicate — cheap to evaluate continuously.
    /// Shape suits Safety + SEC invariants.
    PureState(fn(&S) -> Verdict),
    /// Trace predicate — needs a prefix of the event stream.
    /// Shape suits Liveness + Convergence + ReplayEquivalence.
    Trace(fn(&TracePrefix<S>) -> Verdict),
}

pub enum FailurePolicy {
    /// Panic on violation (TigerBeetle shape, Finding 2.2).
    /// The invariant author asserts that a violation means the local
    /// process has already entered an unsafe state and must not continue.
    Panic,
    /// Emit a structured event; never fatal (Antithesis shape, Finding 2.1).
    /// The invariant author asserts that a violation is signal, not
    /// state corruption.
    EmitEvent,
}

pub enum InvariantScope {
    /// Safe in every consumer (DST, real kernel, chaos, exerciser).
    Portable,
    /// Simulation-only — cannot be evaluated against a real-kernel trace.
    /// Marked explicitly so the crate's reuse guarantees stay honest.
    SimulationOnly,
}
```

The Predicate split (`PureState` vs `Trace`) mirrors Alpern &
Schneider's topological decomposition (Finding 1.1) and TLA+'s
state-predicate-vs-temporal-formula distinction (Finding 1.2). A
Safety invariant encoded as `Trace` is a misclassification the runtime
can catch at registration time; a Liveness invariant encoded as
`PureState` is likewise detectable. The invariant author cannot lie to
the checker.

The `FailurePolicy` enum is the resolution of the TigerBeetle /
Antithesis conflict surfaced in the research (Finding 2.2 vs 2.1) —
the research does not force a single policy across Overdrive; the
invariant author declares the policy at the spec level, and the
runtime respects it. This matches Synthesis S4: "let the invariant
definition itself declare its failure policy, and let the runtime
choose per context."

### 4. State projection — trait, not impl

```rust
pub trait State: Send + Sync {
    type Event: Event;
    fn current(&self) -> StateSnapshot<'_>;
    fn events_since(&self, tick: Tick) -> EventStream<'_, Self::Event>;
}

pub trait Event: Send + Sync {
    fn timestamp(&self) -> Tick;
    fn identity(&self) -> Option<&SpiffeId>;
}
```

`overdrive-invariants` ships the `State` trait only. Three distinct
implementations live in adjacent crates:

| Implementation | Home crate | Source of state |
|---|---|---|
| `SimState` | `overdrive-sim` (ADR-0004) | turmoil tick stream |
| `KernelTraceState` | `overdrive-bpf` (Tier 3, testing.md §22) | eBPF ringbuf + `bpftool map dump` projections |
| `LiveClusterState` | `overdrive-control-plane` | `ObservationStore` subscriptions + telemetry ringbuf |

The crate contract is: an invariant written against `&S: State`
evaluates identically in every consumer. The `StateSnapshot<'_>` is a
borrow, not a copy — the production invariant runtime does not pay the
cost of materialising cluster state; the sim runtime does not pay the
cost of synthesising what it already holds.

### 5. Migration — single-cut

Migration is single-cut, per the project-wide preference for greenfield
pre-v1: in the **same PR** that introduces `overdrive-invariants`,
`crates/overdrive-sim/src/invariants/` is deleted and every call site
is rewritten to consume the new crate. No deprecation window, no
`legacy-invariant-api` feature flag, no re-export shim, no two-phase
rollout. Overdrive is pre-v1 and has no external users — a multi-phase
migration would ship two invariant implementations that no one outside
the project ever benefits from.

Concretely, the single PR:

1. **Lands `overdrive-invariants`** with the full name catalogue, class
   taxonomy, `InvariantSpec<S>`, `Predicate<S>`, `FailurePolicy`, and
   `InvariantScope`. Ports the existing `Invariant` enum (9 variants
   as of Phase 1, including the three Phase-1-scaffold reconciler
   invariants from ADR-0013) with `as_canonical`, `Display`, `FromStr`,
   and `ALL` intact.
2. **Rewrites the sim evaluators** from free functions
   (`evaluate_single_leader_from_topology(hosts, leader)`) to
   `Predicate<SimState>` impls registered against the invariant spec.
   The free-function surface is deleted, not retained — the
   per-function unit tests move to assert against the registered
   predicates directly.
3. **Deletes `crates/overdrive-sim/src/invariants/`** entirely. No
   re-export shim. Imports across the workspace are updated to point
   at `overdrive-invariants`.
4. **Updates every call site** in one pass: `overdrive-sim/src/lib.rs`,
   `overdrive-sim/tests/dst/*.rs`, `xtask/src/main.rs` (the `--only
   <NAME>` parser), and `crates/overdrive-sim/tests/invariant_roundtrip.rs`
   (which moves to `crates/overdrive-invariants/tests/`).

This amends ADR-0004 in scope — the sim crate keeps its single-crate
shape and its turmoil wiring; what moves out (and is deleted at source)
is the taxonomy and predicate shape. ADR-0004's "one-crate-for-sim"
decision is unaffected.

The DST suite is expected to build-and-pass against the new crate on
the same commit as the deletion — there is no "PR build where the sim
crate is broken because the old path was removed before the new path
landed." The rewrite is mechanical (same names, same semantics, new
import path and `Predicate` shape), and the name round-trip proptest
(moved into `overdrive-invariants/tests/`) gates the rename before
merge.

### 6. Sharing with Verus — translation layer, not shared syntax

The Verus research (`docs/research/verification/verus-for-overdrive-applicability-research.md`)
establishes two load-bearing facts:

- **Finding 2.2** — Verus does not support temporal logic natively.
  Anvil built a TLA embedding (85 lines of core + 5353 lines of
  reusable lemmas) to express ESR.
- **Finding 1.7** — Verus has known panics on `dyn Trait`; verified
  code uses static dispatch (e.g. `Controller<C: ControllerApi>`).

The `overdrive-invariants` crate therefore **does not attempt to
express invariants in a syntax that is simultaneously executable Rust
and Verus `spec fn`**. The executable `Predicate<S>` lives in the
crate; the Verus-spec counterpart lives alongside the verified
reconciler in the pilot sub-workspace (ADR-0018), written by hand
against the same `State` trait's logical shape.

The two are held consistent by a *name-and-class contract*: every
verified reconciler's Verus spec carries the same `InvariantName` and
`InvariantClass` as its executable counterpart. A compile-time test
in `overdrive-invariants::tests` round-trips the name set against
the pilot crate's exported spec names to catch drift. This is the
same lightweight bridge pattern Anvil uses between its Rust executable
and TLA embedding (Anvil §4.4) — there is no published evidence that a
heavier-weight shared DSL is productive.

If the Verus pilot (ADR-0018) succeeds and the pilot team identifies
a shared-syntax win, a follow-up ADR can revisit. Today's evidence
does not support front-loading that investment.

### 7. External integrations

None. The invariants crate is a pure-logic `core` crate — no external
APIs to contract-test. The `State` trait's three implementations each
have their own external boundaries (turmoil, aya-rs, Corrosion),
covered in the crates that own them.

## Alternatives considered

### Alternative A — Keep invariants in `overdrive-sim`, export a public module

**Rejected.** ADR-0004 bans `turmoil` from the production compile
path. The chaos reconciler (§18) and the real-kernel integration
harness (testing.md §22) are both production-compile; depending on
`overdrive-sim` to consume its invariant module would pull `turmoil`
into crates that must not have it. The DST-lint gate would reject
this. Either the rule erodes or the code doesn't compile; neither
is acceptable.

### Alternative B — Add a new `spec` crate class

**Rejected.** ADR-0003's four-class model (`core | adapter-host |
adapter-sim | binary`) is deliberately closed. A fifth class would
require updating `dst-lint`, the `FromStr` parser in xtask, every
`Cargo.toml` that declares a class, and every doc that enumerates
the classes. The existing `core` class already satisfies the
constraints (no I/O, DST-lint-scanned, no turmoil). The delta is a
tax without a benefit.

### Alternative C — Invariants-as-trait instead of invariants-as-enum

**Rejected.** Finding 1.7 (Jepsen Elle) is direct evidence that
taxonomic enums beat generic predicates for shrinking, reporting,
and harness dispatch. A trait-of-invariants invites an "`impl
Invariant for MyRandomStruct`" that bypasses the five-class
taxonomy, producing exactly the homogenisation trap Finding 4.1
warns against (inline authors invent new classes rather than
justify existing ones). The enum is closed at the API surface;
invariant authors choose a class and stand behind it.

### Alternative D — Pure runtime monitor synthesis from a temporal-logic DSL

**Rejected.** Finding 1.3 (Havelund & Rosu 2005) establishes that
runtime monitor synthesis from LTL/MTL works academically, but
Gap 3 in the research doc is explicit: "no published bridge between
Havelund-Rosu monitor synthesis and Anvil-style ESR." Building that
bridge is research, not engineering. The invariant crate ships
hand-written predicates; the synthesis-from-formula path is a
future revisit if and when the research matures.

### Alternative E — Single `FailurePolicy` across the platform

**Rejected.** Research Conflict 1 is explicit: TigerBeetle's
fail-stop and Antithesis's emit-and-log are **both defensible**,
chosen for different domain priorities. Forcing one policy across
every Overdrive invariant would be an architectural bet the
research evidence does not support. The per-invariant declaration
is the honest compromise.

## Consequences

### Positive

- **Principle 10 (Earned Trust) directly honoured**: every invariant
  the platform depends on is named, classified against cited academic
  sources, and exercised against real state by at least one
  runtime. Invariants cannot rot inline as a test-body macro.
- **Reusability proven by construction**: ADR-0003's dst-lint scan
  ensures the crate stays dependency-minimal. An invariant that can't
  be reused is one that would pull in banned APIs, and the lint
  catches that.
- **Five-class taxonomy is a speed bump against homogenisation trap**
  (Finding 4.1). An author reaching for an invariant must justify
  the class; the compiler rejects free-form predicates without one.
- **Verus translation remains tractable**: by keeping the Rust
  `Predicate` and any Verus `spec fn` in name-and-class correspondence
  rather than shared syntax, each side can evolve on its own schedule.
- **Migration is single-cut, with no compat shim debt**: the workspace
  never carries two parallel invariant implementations. The long-term
  maintenance cost is zero — there is no feature-flagged legacy path
  to eventually delete, no deprecation schedule to track, no grace
  window to forget. ADR-0004 stands unchanged in scope.

### Negative

- **Second crate in the critical-path compile graph**. `core` class
  crates gate the DST lint scan; adding one adds scan surface. The
  marginal cost is small (dst-lint iterates on already-loaded
  `syn` ASTs), but it is non-zero.
- **Single PR is larger than a phased migration would be**. The
  landing PR touches the new crate, the sim crate's deleted
  subdirectory, every call site across the workspace, the xtask
  parser, and the moved roundtrip tests. Short-term review burden is
  higher than a feature-flagged phase-1-only PR would be; the
  trade-off is zero long-term shim debt. Per project policy
  (greenfield, pre-v1, no external users) this trade is the right
  one.
- **Per-invariant `FailurePolicy` declaration is a design decision
  every author must make**. The crate cannot choose for them. If
  authors default to `Panic` everywhere "for safety" we inherit
  TigerBeetle's policy by omission; if they default to `EmitEvent`
  we inherit Antithesis's. Mitigation: the author's README
  enumerates the two policies with concrete examples and frames the
  choice as "is this local state I own or cluster state I observe?"
- **The `KernelTraceState` and `LiveClusterState` implementations
  do not exist yet**. Tier 3 and chaos-reconciler consumption of
  the invariants crate cannot be validated in Phase 1; it becomes
  a Phase 2+ commitment. The Phase 1 deliverable is the `core`
  crate + `SimState` migration; downstream consumption follows as
  those crates come online.

### Quality-attribute impact (ISO 25010)

- **Maintainability — modularity**: positive. Invariant taxonomy lives
  in one place; changes cascade via the type system.
- **Maintainability — testability**: strongly positive. Invariants
  unit-testable in isolation of any runtime; `State` trait mockable.
- **Reliability — fault tolerance**: positive. Invariants evaluated
  in production chaos runs against the same specification used in
  DST; the §21 / §22 fault catalogue composes with a shared oracle
  rather than per-tier assertions.
- **Portability — replaceability**: neutral. The crate has no
  external license or runtime implications beyond what already
  exists in the workspace.

## Enforcement

- **Crate class declaration** — ADR-0003 machinery; dst-lint asserts
  `overdrive-invariants` is `core` and has no banned API imports.
- **Name round-trip proptest** — the existing
  `crates/overdrive-sim/tests/invariant_roundtrip.rs` moves into
  `crates/overdrive-invariants/tests/` and grows to assert every
  variant's `as_canonical` / `Display` / `FromStr` stays bijective
  across the class taxonomy.
- **Verus-name-consistency test** — a compile-fail test (trybuild,
  per testing.md) asserts the verified-reconciler pilot's spec names
  are a subset of `InvariantName::ALL`. Fires at the pilot crate's
  build, not the core crate's.
- **Class-vs-predicate consistency test** — a unit test walks every
  registered `InvariantSpec` and asserts that `Safety` / `SEC` specs
  carry `Predicate::PureState` and that `Liveness` / `Convergence` /
  `ReplayEquivalence` specs carry `Predicate::Trace`. A misclassified
  invariant fails this test at registration.
- **Architectural rule enforcement** — per principle 11 (enforceable
  architecture rules), the language-appropriate tool for enforcing
  "`overdrive-invariants` may not depend on `turmoil`, `rkyv`, `redb`,
  `tokio::net`, or any crate declaring `crate_class != core`" is
  **dst-lint** (ADR-0006) already, extended with a manifest-level check
  on direct dependencies of `core` crates. No new tooling required.

## Supersedes / Relates

- **Relates to ADR-0003** — uses the `core` class without amendment.
- **Amends ADR-0004** — the sim crate keeps its single-crate shape;
  the invariant taxonomy moves out. Sim adapters, harness, turmoil
  wiring, and `tests/dst/*.rs` layout per ADR-0005 are unchanged.
- **Relates to ADR-0005** — invariant-evaluator unit tests live in
  `crates/overdrive-invariants/tests/*.rs` (plumbing layout per
  ADR-0005); DST scenario tests stay in `crates/overdrive-sim/tests/dst/`
  and import the invariants from the new crate.
- **Relates to ADR-0006** — `cargo xtask dst` and `--only <NAME>`
  continue to resolve names against the invariant catalogue, now
  imported from `overdrive-invariants` rather than `overdrive-sim`.
  No xtask surface changes.
- **Relates to ADR-0013** — the three reconciler-primitive invariants
  (`at-least-one-reconciler-registered`, `duplicate-evaluations-collapse`,
  `reconciler-is-pure`) migrate to the new crate's Safety /
  ReplayEquivalence classes without semantic change.
- **Preconditions ADR-0018** — the Verus pilot consumes `InvariantName`
  and `InvariantClass` as the name-and-class contract. ADR-0018 may
  land before or after this ADR lands, but the pilot cannot meaningfully
  start without either this crate or an equivalent spec-surface stub.

## Open Questions (for user decision)

1. **Default `FailurePolicy`.** Since migration is single-cut, every
   invariant is (re)registered in the same PR and gets an explicit
   `FailurePolicy` at registration — no "unlabelled during migration"
   transient. The open question is therefore whether the crate exposes
   a `Default` impl at all (and if so, `Panic` or `EmitEvent`) for
   future invariants, or requires explicit declaration from day one.
   Neither default is evidence-forced. Architect recommendation: no
   `Default` impl — force authors to choose at registration.
2. **`KernelTraceState` scope.** The Tier 3 implementation of `State`
   has non-trivial surface — parsing `bpftool map dump`, correlating
   with the telemetry ringbuf, projecting onto Overdrive's logical
   state. Is this in scope for this ADR's Phase 2 landing, or does
   it need its own ADR? (Architect recommendation: separate ADR.
   The Tier 3 surface is big enough to warrant its own decision
   record.)
3. **`SimulationOnly` invariants — allow or reject?** The
   `InvariantScope::SimulationOnly` variant is present in the
   proposed API but contradicts Finding 5.1's guidance ("do not
   define invariants that can only be evaluated in simulation").
   The variant is a pressure valve; removing it forces every
   invariant to be portable. Is the pressure valve worth the
   architectural looseness?
4. **Verus-name-consistency enforcement boundary.** The consistency
   test between `overdrive-invariants::InvariantName::ALL` and the
   pilot crate's Verus spec names — does this live in the core crate
   (trybuild catches pilot-crate drift at core-crate build) or the
   pilot crate (core crate has no knowledge of the pilot)? The ADR
   proposes the pilot crate; the inverse is defensible.

## References

- Research: `docs/research/testing/invariant-observer-patterns-comprehensive-research.md`
  (Nova, 2026-04-23). Findings 1.1, 1.2, 1.4, 1.6, 1.7, 1.8, 2.1,
  2.2, 2.4, 3.1, 3.2, 3.5, 4.1, 5.1 cited inline.
- Research: `docs/research/verification/verus-for-overdrive-applicability-research.md`
  (Nova, 2026-04-23). Findings 1.7, 2.2, 4.1, 4.4 cited inline.
- ADR-0003 — core-crate labelling (four-class model).
- ADR-0004 — `overdrive-sim` single-crate decision.
- ADR-0005 — test distribution layout.
- ADR-0006 — CI wiring for dst-lint and dst.
- ADR-0013 — reconciler primitive runtime (source of three invariants
  migrating to the new crate).
- Whitepaper §18 (Reconciler and Workflow Primitives).
- `.claude/rules/testing.md` Tier 1 (DST) — invariant catalogue.
- Alpern, B., & Schneider, F. B. "Defining Liveness." *IPL* 21(4),
  1985. (Academic basis for Safety/Liveness split.)
- Sun, X. et al. "Anvil: Verifying Liveness of Cluster Management
  Controllers." OSDI '24, Best Paper. (Academic basis for ESR.)
