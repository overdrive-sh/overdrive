# ADR-0011 — Intent-side `Job` aggregate and observation-side `AllocStatusRow` stay separate types

## Status

Accepted. 2026-04-23.

## Context

DISCUSS Key Decision 6 + US-01 Technical Notes flagged a design question:

> `JobSpec` as it currently exists in
> `crates/overdrive-core/src/traits/observation_store.rs` is a
> placeholder used by the ObservationStore row shape. DESIGN must
> decide whether to rename / replace it or keep it as an
> observation-side DTO distinct from the intent-side `Job` aggregate.

Inspection of `observation_store.rs` (loaded at DESIGN time) shows the
file exports `AllocStatusRow`, `AllocState`, `LogicalTimestamp`, and
related row shapes — the Phase 1 minimal schema per brief §6. The
name "`JobSpec`" itself is referenced only as the DISCUSS-wave shorthand
for whatever-intent-side-type-carries-the-job-definition; the current
file uses `AllocStatusRow` (with a `job_id: JobId` field) as its
observation-side representation. The "placeholder JobSpec" named in
user-stories.md is a vestigial reference that survived rewording.

Three options were on the table:

- **(a) Rename the intent-side aggregate to something other than `Job`**
  (e.g. `WorkloadSpec`). Avoids any terminology drift.
- **(b) Replace the observation-side row-shape with the intent-side
  aggregate** — one struct type across both layers.
- **(c) Keep them separate**. Intent-side `Job` aggregate is the
  authoritative declaration written to `IntentStore`; observation-side
  `AllocStatusRow` (already present) is the LWW-gossiped row shape
  describing the live state of allocations of that job.

## Decision

**Option (c): intent-side `Job` aggregate and observation-side
`AllocStatusRow` stay separate types in different modules. The
observation-side file stops carrying anything that looks like an
intent-side `JobSpec`.**

Concretely:

- **`overdrive-core::aggregate::Job`** — new module. The intent-side
  authoritative Job aggregate. Derives `rkyv::Archive`,
  `rkyv::Serialize`, `rkyv::Deserialize`, `serde::Serialize`,
  `serde::Deserialize`. Validating constructor `Job::from_spec(...)`.
  Fields: `id: JobId`, `replicas: NonZeroU32`, `resources: Resources`
  (the one already in `traits/driver.rs` — reused, not duplicated),
  plus future fields landing in later phases.
- **`overdrive-core::aggregate::{Node, Allocation, Policy, Investigation}`**
  — co-located in the same module. `Node` and `Allocation` are
  first-class aggregates; `Policy` and `Investigation` are stubs
  with the ID newtype as primary field per US-01 AC.
- **`overdrive-core::traits::observation_store::AllocStatusRow`** — unchanged.
  Already present. Its job is to carry `(alloc_id, job_id, node_id,
  state, updated_at)` as gossiped observation; not to duplicate the
  intent-side `Job` shape.
- **Any `JobSpec`-named struct in `observation_store.rs`** — if one
  survives, it's either deleted (if unused) or renamed to a name that
  makes its observation-side role obvious (e.g. `AllocObservationPayload`).
  The crafter verifies this at implementation.

### Naming rule

Intent-side aggregates live in `overdrive-core::aggregate::*`.
Observation-side row shapes live in
`overdrive-core::traits::observation_store::*`. The module path is the
disambiguator; both may carry a `JobId` field, but neither is called
`Job*` or `*Spec` in the other's module.

### Resources deduplication

`Resources` lives in `traits/driver.rs` today. `Job::from_spec` consumes
the existing `Resources` type — no duplicate declaration. If a Phase 2+
need introduces a Job-specific resources variant, that's a new type
with a distinct name, not a second `Resources`.

## Considered alternatives

### Alternative A — Rename intent-side aggregate to `WorkloadSpec`

**Rejected.** Whitepaper §4 names the aggregate `Job` explicitly in the
core data model. The whitepaper, user-stories, and journey artifacts all
use `Job` as the ubiquitous-language term for the intent. Renaming to
`WorkloadSpec` would create terminology drift between the code and the
domain vocabulary established by the DISCOVER wave. The collision with
the observation-side vestigial `JobSpec` is a vestige problem, not a
naming problem — we delete the vestige.

### Alternative B — One struct across both layers

**Rejected.** The intent / observation split (whitepaper §4, brief §4)
is load-bearing; crossing it at the type level is exactly the bug class
the trait non-substitutability enforcement was built to prevent. Using
one struct for both would require the struct to carry both intent fields
(replicas, resources) and observation fields (state, updated_at) —
either mixing them (semantic bug) or making half the fields `Option`
(which mutates the struct's identity depending on which store it came
from, a runtime-typed bug).

## Consequences

### Positive

- Intent and observation stay decoupled at the type level — consistent
  with the compile-time non-substitutability invariants from
  phase-1-foundation.
- `Job::from_spec` is the single validating path into the intent
  aggregate; no shortcut through the observation side.
- Resources stays single-source (driver.rs), honouring US-01's
  no-duplication AC.
- The vestigial `JobSpec` reference is actionable: the crafter either
  deletes it or renames it to make its role obvious.

### Negative

- Minor duplication of `JobId` references across the two layers — but
  that is the inherent cost of the intent/observation split and is
  already paid everywhere else.
- Anyone writing a new type needs to decide which layer it belongs to.
  The naming rule above makes the decision mechanical.

### Quality-attribute impact

- **Maintainability — modularity**: positive. Each layer's types stay
  inside their trait's module.
- **Maintainability — modifiability**: positive. Adding a field to
  `Job` is an intent-side concern; adding a field to `AllocStatusRow`
  is an observation-side concern; the two evolutions are independent.

### Enforcement

- A unit test in `overdrive-core` asserts `Job` and `AllocStatusRow`
  are distinct Rust types (tautological but documentary).
- The existing compile-fail test that asserts `IntentStore` /
  `ObservationStore` non-substitutability catches any accidental
  cross-layer usage.
- The crafter verifies no `JobSpec`-named struct remains in
  `observation_store.rs` after implementation; if one is required
  for test scaffolding, it is renamed to carry the observation-side
  role in its name.

## References

- `docs/whitepaper.md` §4 (Core Data Model)
- `docs/product/architecture/brief.md` §4 (state-layer discipline),
  §6 (ObservationStore row shapes)
- `docs/feature/phase-1-control-plane-core/discuss/user-stories.md`
  US-01 Technical Notes
- `docs/feature/phase-1-control-plane-core/slices/slice-1-aggregates-and-canonical-keys.md`
