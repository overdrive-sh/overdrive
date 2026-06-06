# DESIGN Wave Decisions — describe-workload-oneof-discriminator

Feature: GH #183 — WorkloadDescription Service-arm wire-shape widening
(`oneOf` discriminator for `describe_workload`). DESCRIBE-side mirror of
ADR-0051. SSOT: **ADR-0064**. Sign-off: 2026-06-06 (user-locked
OQ-1/4/5/7). Density: lean.

## Key Decisions

| D# | (OQ) | Decision |
|---|---|---|
| D1 | OQ-1 | Distinct `DescribeSpecOutput` kind-discriminated `oneOf` (`#[serde(tag="kind")]`, utoipa → `discriminator: propertyName: kind`) in `overdrive-core::api::describe`. NOT a reuse of `SubmitSpecInput`. `WorkloadDescription.spec: JobSpecInput → DescribeSpecOutput`. |
| D2 | OQ-4 | Service arm carries a **REQUIRED** `vip: ServiceVip` (dotted-quad on the wire). Missing allocator entry → HTTP 500 (`ControlPlaneError::ServiceVipMissing`), never `Option`/empty string. Job + Schedule arms carry no VIP. |
| D3 | OQ-7 | **Read-only** `PersistentServiceVipAllocator::get(&spec_digest)` — the method EXISTS (`persistent_service_vip.rs:251`). Reuse; never `allocate`/`release`. |
| D4 | OQ-5 | All three arms land now (exhaustive enum). `ScheduleV1::to_describe()` is a `todo!("RED scaffold")` — Schedule submit is itself unrealised. |
| D5 | — | New module `crates/overdrive-core/src/api/describe.rs` (sibling of `api::submit`; dep-graph leaf). |
| D6 | — | Single-cut greenfield migration: one atomic commit (types + handler + error variant + consumers + OpenAPI regen + golden). |

## Architecture Summary

Adds a **fourth type-family corner** (HTTP describe wire) to the three
ADR-0051 established. `DescribeSpecOutput` is the read-only output
projection of `WorkloadIntent` (+ the allocator-owned VIP for Service).
The describe handler replaces its HTTP 400 non-Job rejection with an
exhaustive match: Job → `DescribeSpecOutput::Job(JobSpecInput)` (reusing
the existing `From<&Job>` impl); Service → read-only `allocator.get`,
then `DescribeSpecOutput::Service(ServiceSpecOutput { …, vip })`;
Schedule → structured rejection (Phase 1 cannot persist a Schedule).
No new container; the describe read targets the same `IntentStore` +
`ServiceVipAllocator` the control plane already holds.

## Reuse Analysis

- **REUSE**: `From<&Job> for JobSpecInput` (Job render path);
  `PersistentServiceVipAllocator::get` (read-only VIP);
  `ServiceVip` / `ListenerInput` / `ResourcesInput` / `DriverInput`
  wire/id types.
- **EXTEND pattern, CREATE sibling**: `api::describe` mirrors
  `api::submit`; `to_describe` family mirrors `from_submit`.
- **EXTEND**: `WorkloadDescription` struct (field-type flip); utoipa
  schema list; `ControlPlaneError` (new `ServiceVipMissing` variant).
- **CREATE NEW** (only where mandated): `api/describe.rs` (distinct
  module per D1); `ServiceVipMissing` error variant.

## Technology Stack

serde + utoipa (OpenAPI `oneOf`), no rkyv at the wire layer (mirror of
ADR-0051 § 3). `ServiceVip` newtype renders dotted-quad. All
OSS/existing workspace deps; no new dependency.

## Constraints

- Phase 1, single-node — no multi-region/HA. Allocator memo is local;
  `get` is an in-memory `BTreeMap` lookup.
- Read-only describe — no allocator mutation on a GET (HTTP-method
  contract + idempotency).
- VIP read at describe time, never persisted on the response shape
  (persist-inputs-not-derived-state; allocator memo is the source of
  truth per ADR-0049 § 5a).
- Single-cut greenfield migration (no aliases/shims).

## Upstream Changes

- **ADR-0051 § 1 amended** (§ "Amendment (2026-06-06)"): the
  "`WorkloadIntent → SubmitSpecInput` — describe echoes back" boundary
  note is superseded for the describe direction by ADR-0064. Describe
  now uses `DescribeSpecOutput`; the Job arm still reuses `JobSpecInput`
  but wrapped in `DescribeSpecOutput::Job(...)`, not as a bare
  `SubmitSpecInput`.
- **brief.md § 69** added under `## Application Architecture` (extends,
  does not rewrite §§ 1–68).

## Deferrals (surfaced to user; NO issues created)

- `JobSpecInput` module relocation — pre-existing debt tracked in
  ADR-0051 § "Future Work"; this feature reuses the type in place.
- gRPC bridge / SDK generation consuming the describe `oneOf` —
  pre-existing ADR-0051 § "Future Work" item; out of scope.

Neither is a forward pointer in this feature's artifacts; both are
named in the architect's return message for the orchestrator to relay.

## External Integrations

None. No contract tests recommended.
