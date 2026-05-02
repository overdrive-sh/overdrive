# ADR-0024 — Dedicated `overdrive-scheduler` crate (class `core`); dst-lint scope expansion

## Status

Accepted. 2026-04-27. Decision-makers: Morgan (proposing the
in-`overdrive-control-plane` module placement); user
**OVERRIDE**: dedicated `overdrive-scheduler` crate. Tags:
phase-1, first-workload, application-arch.

## Context

The Phase 1 first-workload feature ships a first-fit scheduler
function (US-01) — a pure synchronous function:

```rust
fn schedule(
    nodes:  &BTreeMap<NodeId, NodeView>,
    job:    &JobView,
    allocs: &[AllocStatusRow],
) -> Result<NodeId, PlacementError>;
```

The scheduler is consumed by the `JobLifecycle` reconciler (US-03)
inside its pure `reconcile` body — a pure helper called from a pure
reconciler, matching Anvil (USENIX OSDI '24)'s `reconcile_core`
pattern. The DISCUSS wave Key Decision 4 framed this as "the
scheduler is a module, not a reconciler variant."

The remaining decision is *where the module lives*. Two options:

- **(a)** Module inside `overdrive-control-plane::scheduler`.
  Lightweight; reuses an existing crate. Loses dst-lint coverage
  (`overdrive-control-plane` is `adapter-host` class — not scanned).
- **(b)** Dedicated `overdrive-scheduler` crate, class `core`. New
  crate, but the `BTreeMap`-only iteration discipline + banned-API
  contract (no `Instant::now`, no `rand::*`, no `tokio::net::*`)
  becomes mechanically enforced by `dst-lint`, not just by
  convention.

Morgan's proposal recommended **(a)** as the simplest-possible
Phase 1 wiring. The user **OVERRIDE** chose **(b)**: dedicated
crate.

The override is correct. The scheduler is a pure function over
deterministic data, lives inside reconciler bodies that the DST
harness exercises continuously, and carries the load-bearing
`BTreeMap`-only iteration discipline (`development.md` §
Ordered-collection choice). Putting it in an `adapter-host` crate
relies on the dst-lint discipline being preserved by review alone.
Putting it in a `core` crate makes the discipline mechanical: a
`HashMap` snuck into the scheduler hot path fails the lint at PR
time.

## Decision

### 1. Create `crates/overdrive-scheduler` with class `core`

```toml
# crates/overdrive-scheduler/Cargo.toml
[package]
name        = "overdrive-scheduler"
description = "Pure-function placement scheduler. First-fit Phase 1; deterministic by construction."
edition.workspace      = true
rust-version.workspace = true
license.workspace      = true
authors.workspace      = true
repository.workspace   = true
publish                = false

[package.metadata.overdrive]
crate_class = "core"

[features]
# Workspace-wide convention. Every member declares this feature so
# `cargo {check,test,mutants} --features integration-tests` resolves
# uniformly under per-package scoping. This crate has no integration-
# shaped tests of its own — the declaration is a deliberate no-op.
# See `.claude/rules/testing.md` § Workspace convention.
integration-tests = []

[dependencies]
overdrive-core.workspace = true   # Resources, NodeId, JobId, Node, Job,
                                  # AllocationId, AllocStatusRow shapes
thiserror.workspace      = true   # PlacementError envelope

[dev-dependencies]
proptest.workspace = true   # determinism + BTreeMap-order invariance

[lints]
workspace = true
```

The crate depends ONLY on `overdrive-core` and the `core`-permitted
helpers (`thiserror`, `proptest` as dev-dep). No `tokio`, no
`rand`, no `std::time::Instant`, no `tokio::net::*` — `dst-lint`
rejects the file at PR time if any banned API appears.

### 2. Workspace registration

```toml
# Cargo.toml at workspace root
[workspace]
members = [
    "crates/overdrive-core",
    "crates/overdrive-cli",
    "crates/overdrive-control-plane",
    "crates/overdrive-host",
    "crates/overdrive-scheduler",       # ← NEW
    "crates/overdrive-sim",
    "crates/overdrive-store-local",
    "xtask",
]
```

The workspace's `every_workspace_member_declares_integration_tests_feature`
xtask test (per `.claude/rules/testing.md` § Workspace convention)
catches a missing feature declaration; the `Cargo.toml` above
satisfies it deliberately.

### 3. Module surface: `overdrive_scheduler::schedule`

```rust
// in overdrive-scheduler/src/lib.rs

use std::collections::BTreeMap;

use overdrive_core::aggregate::{Job, Node};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::traits::observation_store::AllocStatusRow;

/// First-fit placement decision. Pure synchronous function over
/// deterministic inputs.
///
/// The `BTreeMap` parameter type pins iteration order at the type
/// level — `dst-lint` enforces no `HashMap` appears in this crate's
/// source.
///
/// Determinism contract: for any fixed `(nodes, job, current_allocs)`
/// input, two successive calls return equal `Result<NodeId,
/// PlacementError>`. The `aggregate_roundtrip`-shaped proptest in
/// this crate's `tests/` defends the contract.
pub fn schedule(
    nodes:          &BTreeMap<NodeId, Node>,
    job:            &Job,
    current_allocs: &[AllocStatusRow],
) -> Result<NodeId, PlacementError> { … }

/// Placement-failure envelope.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlacementError {
    /// No node has sufficient free capacity for the job's resource
    /// requirements. Carries both the requested envelope and the
    /// largest free envelope across the input nodes for diagnostics.
    #[error("no node has capacity: needed {needed:?}, max free {max_free:?}")]
    NoCapacity {
        needed:   overdrive_core::traits::driver::Resources,
        max_free: overdrive_core::traits::driver::Resources,
    },
    /// The input `nodes` map is empty. Phase 1 single-node should
    /// never produce this; the variant exists for forward-compat
    /// and to allow proptest generators to exercise the boundary.
    #[error("no healthy node in the input set")]
    NoHealthyNode,
}
```

The exact field shape, the `Job` vs `JobView` projection question
(US-01 Technical Notes flag both as acceptable), and the
`Resources::saturating_sub` helper question are crafter-time
decisions — the architectural contract is: pure `fn`, BTreeMap
input, typed error.

### 4. Dependency direction: `overdrive-control-plane → overdrive-scheduler → overdrive-core`

Dependency graph after this ADR:

```
overdrive-core    ←  overdrive-scheduler  ←  overdrive-control-plane
                  ←  overdrive-host
                  ←  overdrive-store-local
                  ←  overdrive-sim (dev/test)
```

`overdrive-scheduler` does NOT depend on `overdrive-control-plane`,
`overdrive-host`, `overdrive-store-local`, or `overdrive-sim`. It
sits between `overdrive-core` (its sole non-test dep) and the
runtime crates that consume it.

The graph remains acyclic: `overdrive-control-plane` already
depends on `overdrive-core`; adding a dep on
`overdrive-scheduler` (which itself depends only on
`overdrive-core`) introduces no cycle. ADR-0003 (crate-class
labelling) and ADR-0016 (overdrive-host extraction) both record
the existing graph; the new edge is consistent with both.

### 5. dst-lint scope expansion is automatic

`xtask::dst_lint::scan(...)` walks the workspace and filters to
crates whose `package.metadata.overdrive.crate_class == "core"`.
With `crate_class = "core"` declared in
`crates/overdrive-scheduler/Cargo.toml`, the scheduler is
automatically in scope for the next `cargo xtask dst-lint`
invocation. No xtask code change is required.

The scheduler's `tests/` and `dev-dependencies` are NOT scanned —
only `src/**/*.rs`. proptest, rstest, and other dev-only deps can
freely use anything they need.

The xtask self-test that asserts the core-class set is non-empty
(per ADR-0003) continues to pass — the set was already non-empty
(`overdrive-core`); adding `overdrive-scheduler` makes it
size-2.

## Alternatives considered

### Alternative A — Module inside `overdrive-control-plane::scheduler` (ORIGINALLY PROPOSED)

```
crates/overdrive-control-plane/src/
├── lib.rs
├── reconciler_runtime.rs
└── scheduler.rs                  ← module placement
```

The scheduler lives as a sibling module to `reconciler_runtime`
inside `overdrive-control-plane`. The `JobLifecycle` reconciler
imports it via `crate::scheduler::schedule`.

**This was Morgan's proposal.** It is the simplest-possible
Phase 1 wiring: no new crate, no new dep edge, the scheduler is
right next to its only Phase 1 caller.

**Rejected on user OVERRIDE** for two reasons:

1. **`dst-lint` scope**: `overdrive-control-plane` is class
   `adapter-host` — *not* scanned. A `HashMap` accidentally
   imported into the scheduler hot path passes the lint
   silently, and the determinism property is preserved by
   convention alone (review + the in-crate proptest). Convention
   erodes; mechanical enforcement does not. Putting the
   scheduler in a `core`-class crate makes `dst-lint`'s
   `BTreeMap`-only enforcement structural.
2. **Symmetry with the §18 reconciler-trait story**: `overdrive-core`
   already hosts the pure `Reconciler` trait. Pure helpers called
   from pure reconciler bodies belong in `core`-class crates, not
   in `adapter-host`-class crates. The scheduler matches the same
   shape — pure function, deterministic, no I/O — and belongs in
   the same class.

### Alternative B — Module inside `overdrive-core::scheduler`

The scheduler lives inside `overdrive-core` itself, alongside
`reconciler.rs` and `aggregate/mod.rs`.

**Rejected.** `overdrive-core` is the trait-and-newtypes home; it
holds *the seams* (ports, identifiers, error envelopes). Domain
logic that consumes those seams belongs in adjacent crates. The
scheduler is consumer-side logic — it imports `Node`, `Job`,
`Resources`, `AllocStatusRow` from `overdrive-core` and produces
a placement decision. Lumping it into `overdrive-core` would
inflate that crate's surface and conflate "the crate that defines
the contract" with "the crate that implements logic against the
contract."

`overdrive-scheduler` as a sibling `core`-class crate hits the
right abstraction layer: same enforcement guarantees, separable
from the trait-and-newtypes substrate.

### Alternative C — Wait until Phase 2 multi-driver to extract

Land the scheduler as `overdrive-control-plane::scheduler` for
Phase 1; extract to a dedicated crate when Phase 2's multi-driver
work makes the boundary worth surfacing.

**Rejected on user OVERRIDE.** "We'll extract it later" routinely
slips past Phase 2 into Phase 3+; the extraction happens *under
pressure* when an actual multi-crate consumer needs it, by which
point the in-`overdrive-control-plane` placement has accumulated
implicit dependencies on the host crate's internals. Doing the
extraction *now*, when the scheduler's only consumer is one
reconciler, locks in the right shape from the start. The Phase 1
crafter cost is one new `Cargo.toml`, one `lib.rs`, and one new
workspace member — well below the boundary-decision cost of
deferring.

This is also the architectural rule the project has been
enforcing throughout Phase 1: ADR-0016 extracted `overdrive-host`
from `overdrive-core` for the same reason, ADR-0017 extracted
`overdrive-invariants`, ADR-0004 deliberately kept
`overdrive-sim` whole rather than splitting it. The pattern
is *extract per architectural class, eagerly, when the seam is
clear*. The scheduler's seam is clear.

## Consequences

### Positive

- **`dst-lint` mechanically enforces the BTreeMap-only and
  banned-API discipline.** A `HashMap` use, an `Instant::now()`
  call, a `rand::random()` call, or a `tokio::time::sleep` in the
  scheduler's source fails the lint at PR time. The determinism
  contract becomes a structural property of the crate, not a
  review concern.
- **Phase 2+ multi-consumer story is unblocked.** When Phase 2's
  cert-rotation reconciler or future right-sizing reconciler
  needs scheduler-shaped helpers, they import
  `overdrive-scheduler` directly — no need to depend on
  `overdrive-control-plane` (which would pull in axum, rustls,
  hyper, etc. along the way). The scheduler crate is the
  minimum-surface dependency for placement logic.
- **Compile-time exhaustiveness for placement strategies.**
  Phase 1 ships first-fit; Phase 2+ may add bin-packing or
  spread strategies as additional `pub fn`s on the same crate.
  The variants live in one place, expressed as functions or as
  a typed strategy enum, with a single `core`-class home.
- **Symmetry with the rest of the architecture.** Pure helpers
  → `core`-class crates; I/O wiring → `adapter-host`-class
  crates; sim adapters → `adapter-sim`-class crates;
  binaries → `binary`-class crates. The four-class rule
  (ADR-0003) holds without exception.
- **The crate boundary is stable across Phase 1 → Phase 2+.**
  When the placement function gains capacity-pressure inputs,
  affinity rules, anti-affinity rules, or right-sizing
  feedback, all changes happen inside `overdrive-scheduler`'s
  `lib.rs`. No call-site disruption.

### Negative

- **One more crate in the workspace.** The workspace grows from
  six to seven Rust crates (excluding `xtask`). Each new
  workspace member adds a small CI overhead (a few seconds per
  `cargo check` / `cargo clippy` / `cargo test` invocation).
  The cost is paid once and amortises across every PR.
- **One more `Cargo.toml` to keep in sync.** Workspace
  conventions (the `integration-tests = []` no-op feature, the
  `[lints] workspace = true` declaration, the
  `package.metadata.overdrive.crate_class` line) all need to be
  reproduced in the new crate. Every existing workspace member
  has these; the xtask `every_workspace_member_declares_integration_tests_feature`
  test catches the most important one mechanically.
- **Phase 1 has exactly one consumer of the new crate.**
  `overdrive-control-plane` is the sole non-test dependent in
  Phase 1 (the `JobLifecycle` reconciler). The full
  multi-consumer story unfolds in Phase 2+. The crate is
  "right-sized for Phase 2" in Phase 1, which a strict
  YAGNI reviewer might call premature. The override decision
  accepts this: the pattern of late extraction under
  inheritance pressure has empirically worse outcomes than
  early extraction with a clear architectural justification
  (ADR-0016, ADR-0017 are the prior precedents).

### Quality-attribute impact

- **Maintainability — modifiability**: positive. New placement
  strategies live in one crate; downstream consumers import a
  single function or type.
- **Maintainability — testability**: positive. The crate has
  one entry point; the proptest suite exercises the
  determinism contract directly without going through any
  other crate.
- **Maintainability — analyzability**: positive. `cargo doc -p
  overdrive-scheduler` produces a focused doc for the
  placement subsystem.
- **Reliability — fault tolerance**: neutral. Pure function;
  no failure modes beyond the typed `PlacementError`.
- **Performance — time behaviour**: neutral. The function call
  is identical regardless of where it lives.
- **Compatibility — interoperability**: positive. A future
  third-party scheduler implementation
  (`overdrive-scheduler-bin-pack`?) gains a clear template.

### Migration

This is a Phase 1 change with no Phase 0 or external code to
migrate. The crate lands in the same PR as US-01's first-fit
implementation. The `JobLifecycle` reconciler in US-03 imports
`overdrive_scheduler::schedule` from the start.

## Compliance

- **ADR-0003 (crate-class labelling)**: `crate_class = "core"`
  declared; the `dst-lint` scan picks it up automatically.
- **ADR-0016 (`overdrive-host` extraction)**: same extraction
  pattern (pure adapter-class boundary surfaces in their own
  crate). Precedent.
- **ADR-0017 (`overdrive-invariants` crate)**: same
  extraction pattern (a `core`-class peer crate holding code
  that depends on `overdrive-core` only). Precedent.
- **ADR-0006 (CI dst gates)**: `cargo xtask dst-lint`
  automatically scans the new crate; no CI config change
  required.
- **`development.md` § Ordered-collection choice**: the
  scheduler's `BTreeMap`-only discipline is now mechanically
  enforced by `dst-lint`, not just by convention.
- **`testing.md` § Workspace convention**: the `integration-tests
  = []` no-op feature is declared; the
  `every_workspace_member_declares_integration_tests_feature`
  xtask test continues to pass.

## References

- ADR-0003 — Core-crate labelling via
  `package.metadata.overdrive.crate_class`.
- ADR-0006 — `cargo xtask dst-lint` is the required CI check.
- ADR-0013 — Reconciler primitive; pure-helper-from-pure-reconciler
  pattern.
- ADR-0016 — `overdrive-host` extraction; the extraction precedent.
- ADR-0017 — `overdrive-invariants` crate; the extraction
  precedent.
- ADR-0021 — `AnyState` enum; the lifecycle reconciler that
  consumes the scheduler.
- ADR-0023 — Action shim placement.
- Whitepaper §4 — Scheduler; bin-packing allocator over
  declared resource requirements.
- USENIX OSDI '24 *Anvil* — pure-helper-from-pure-reconciler
  pattern.
- `docs/feature/phase-1-first-workload/discuss/wave-decisions.md`
  — Key Decision 4 + Priority One item 4 enumerate the two
  options; this ADR captures the user override.
- `docs/feature/phase-1-first-workload/discuss/user-stories.md`
  — US-01 is the scheduler implementation.
- `.claude/rules/development.md` § Ordered-collection choice —
  the structural rule this ADR makes mechanically enforced.
