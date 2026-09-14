# ADR-0111 — Exec-affected specification and intent envelopes reset forward-only to V1

## Status

**User-approved on 2026-09-14; iteration-1 findings remediated; awaiting
independent DESIGN re-review.** The user explicitly directed a greenfield,
forward-only reset because there are no production users. This record is not
implementation authority until the review approves the complete #293 design
bundle.

## Context

Two current envelope owners directly embed the workload-driver union:

- `ServiceSpecEnvelope` in `overdrive-core/src/aggregate/service_spec.rs`
  has one `V3(ServiceSpecV3)` arm, and `ServiceSpecV3.driver` is the parser
  `DriverInput::{Exec, Vm}`.
- `WorkloadIntentEnvelope` in `overdrive-core/src/aggregate/mod.rs` has V1
  payloads whose `WorkloadDriverV1` is Exec-only and V2 payloads whose
  `WorkloadDriverV2` contains Exec and VM across Job, Service, and Schedule.

Removing Exec changes both archived shapes. There are no production users and
the user rejected backwards readers, migration bridges, legacy variants, and
compatibility branches. Preserving V2 VM data is therefore not a requirement.

## Decision

Reset exactly these two envelope owners to new, incompatible VM-only V1
families.

- `ServiceSpecEnvelope` contains only new `V1(ServiceSpecV1)` and its live
  parser driver union contains only VM.
- `WorkloadIntentEnvelope` contains only new `V1(WorkloadIntentV1)`; the live
  Job, Service, Schedule, and WorkloadDriver aliases all point at their new V1
  types, and the driver union contains only VM.
- Delete the old Service V3 and workload-intent V1/V2 payload types,
  conversions, discriminant history, fixtures, and readers.
- Add only new V1 golden fixtures for the current shapes.
- Add no retired-payload error, detector, upgrader, fallback, or migration
  method.

The exact type contracts and the proof that no other envelope embeds these
driver unions live in the feature delta.

## Alternatives considered

### Append a new version and up-convert old VM payloads

Rejected by the user. It is a backwards reader and retains the old two-driver
schema as an implementation dependency despite the absence of production data.

### Retain old Exec variants only to reject them explicitly

Rejected by the user. A typed retired-variant branch is still a compatibility
branch and keeps legacy payload types and fixtures alive.

### Reset unrelated envelopes at the same time

Rejected. `ProbeResultRow`, node health, service hydration/backend,
reconciliation conflict, CA, and workflow-start envelopes do not embed the
removed driver union. Resetting them would be unrelated schema churn.

## Consequences

Positive:

- No legacy Exec specification or intent type remains in compiled code.
- The current schema has one meaning and one fixture per affected owner.
- No migration component, state, error, or compatibility lifecycle is added.

Negative:

- The two affected source families, their fixtures, and every call site must
  change atomically; partial landing would leave aliases and codecs disagreeing.
- V1 is reused as a new incompatible generation, so version numbers convey
  only the post-cut contract, not repository history.

## Links

- [GH #293](https://github.com/overdrive-sh/overdrive/issues/293)
- [Exact feature contract](../../feature/remove-legacy-exec-workload-driver/feature-delta.md)
- [ADR-0048](adr-0048-rkyv-versioned-envelope.md)
