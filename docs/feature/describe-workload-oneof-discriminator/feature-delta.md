# Feature Delta — describe-workload-oneof-discriminator (DESIGN)

Feature: `describe-workload-oneof-discriminator` — GH #183.
Wave: DESIGN (application/component scope). Density: lean (Tier-1 [REF]
only). SSOT decision: **ADR-0064**
(`docs/product/architecture/adr-0064-describe-side-spec-output-discriminator.md`).

This is the DESCRIBE-side mirror of ADR-0051. It widens the
`GET /v1/jobs/{id}` describe **response** from a Job-only `JobSpecInput`
to a kind-discriminated `oneOf` (`DescribeSpecOutput`), lands the
Service describe arm (surfacing the platform-issued VIP), and removes
the HTTP 400 non-Job rejection at `handlers.rs:628-635`.

---

## Wave: DESIGN / [REF] Decisions (D-numbered = locked OQs)

User-locked open questions (sign-off 2026-06-06), recorded verbatim as
ADR-0064 OQ resolutions:

| D# | Decision | Resolution |
|---|---|---|
| D1 (OQ-1) | Describe wire shape | **Distinct `DescribeSpecOutput`** kind-discriminated `oneOf` in `overdrive-core::api::describe`. NOT a reuse of `SubmitSpecInput` (which cannot carry the VIP and would re-couple submit↔describe cadence). |
| D2 (OQ-4) | Service VIP field | **REQUIRED** `vip: ServiceVip`, not `Option`. Missing allocator entry → HTTP 500 (`ControlPlaneError::ServiceVipMissing`), never a silent `Option`/empty string. |
| D3 (OQ-7) | VIP retrieval | **READ-ONLY `allocator.get(&spec_digest)`** — the method ALREADY EXISTS (`PersistentServiceVipAllocator::get`, `persistent_service_vip.rs:251`). Reuse; never call mutating `allocate`/`release`. |
| D4 (OQ-5) | Schedule arm | **Land all three arms now** (exhaustive enum). `ScheduleV1::to_describe()` is a `todo!("RED scaffold")` — Schedule submit is itself unrealised. |
| D5 | Module placement | `crates/overdrive-core/src/api/describe.rs` (NEW, sibling of `api::submit`; dep-graph leaf). |
| D6 | Migration | Single-cut greenfield: flip `WorkloadDescription.spec`, update every describe consumer, remove the HTTP 400 + #183 pointer, OpenAPI regen + golden update — one commit. |

Quality attributes (ISO 25010): **Maintainability/Modularity** (the
fourth type family keeps describe-wire evolution decoupled from submit
per Pattern C); **Functional correctness** (VIP surfacing closes #183);
**Security/Integrity** (read-only describe — no allocator mutation on a
GET). No performance/scalability concern (one in-memory `BTreeMap`
lookup added to a read path).

## Wave: DESIGN / [REF] Domain & Type Model

Four type families for "a workload" (the fourth added by this feature):

```
WorkloadSpec    (TOML parser   — operator → parser)   ADR-0047
SubmitSpecInput (HTTP request  — client → server)     ADR-0051
WorkloadIntent  (persisted     — server-internal)     ADR-0050
DescribeSpecOutput (HTTP response — server → client)  ADR-0064  ← NEW
```

`DescribeSpecOutput` is the read-only **output projection** of
`WorkloadIntent` (+ the allocator-owned VIP for the Service arm). It is
the inverse-direction sibling of `SubmitSpecInput`: submit validates
`JSON → WorkloadIntent`; describe renders `WorkloadIntent (+ VIP) →
JSON`. Type sketches are in ADR-0064 §§ 1–3.

## Wave: DESIGN / [REF] Component Decomposition

| Component | EXTEND / CREATE | Location | Responsibility |
|---|---|---|---|
| `DescribeSpecOutput` enum + `ServiceSpecOutput` + `ScheduleSpecOutput` | CREATE | `crates/overdrive-core/src/api/describe.rs` (new) | Describe-wire `oneOf` shape; serde + utoipa. |
| `api` module wiring | EXTEND | `crates/overdrive-core/src/api/mod.rs` | Add `pub mod describe;` + re-export (one line, sibling of `submit`). |
| `JobV1::to_describe()` / `ServiceV1::to_describe(vip)` / `ScheduleV1::to_describe()` | CREATE (Job delegates to existing `From<&Job>`) | `crates/overdrive-core/src/aggregate/mod.rs` | Per-kind render constructors; inverse of `from_submit`. Schedule = RED scaffold. |
| `WorkloadDescription` struct + utoipa schema list | EXTEND | `crates/overdrive-control-plane/src/api.rs:153-157, 356-372` | `spec` field type `JobSpecInput → DescribeSpecOutput`; register new schemas. |
| `describe_workload` handler | EXTEND | `crates/overdrive-control-plane/src/handlers.rs:581-638` | Replace HTTP 400 `let-else` (628-635) + #183 pointer (623-627) with exhaustive match; add read-only `allocator.get` for Service arm. |
| `ControlPlaneError::ServiceVipMissing` variant + HTTP-500 mapping | CREATE | `crates/overdrive-control-plane/src/error.rs` (+ ADR-0015 mapping site) | Internal-invariant violation when a persisted Service has no allocator entry. Dedicated `#[from]`-style variant, not `internal(String)`. |
| `PersistentServiceVipAllocator::get` | REUSE (exists) | `crates/overdrive-dataplane/src/allocators/persistent_service_vip.rs:251` | Read-only VIP lookup. No new allocator method. |
| CLI describe command + response parsing | EXTEND | CLI describe command site | Parse `DescribeSpecOutput` instead of `JobSpecInput`. |
| OpenAPI golden + gate test | EXTEND | `api/openapi.yaml`, `tests/integration/openapi_gate.rs` | Regen via `cargo openapi-gen`; update golden. |

## Wave: DESIGN / [REF] Ports

**Driving port** — `GET /v1/jobs/{id}` (the describe endpoint,
`describe_workload` handler). Inbound HTTP; returns
`WorkloadDescription { spec: DescribeSpecOutput, spec_digest }`.

**Driven ports** (both read-only at describe time):
- `IntentStore::get(key)` — read the persisted `WorkloadIntent` bytes
  (already used by the handler today).
- `PersistentServiceVipAllocator::get(&ServiceSpecDigest)` — read-only
  VIP lookup for the Service arm. EXISTS (`&self`, sync, in-memory memo).
  Accessed via `state.allocator.lock().await.get(&digest)` (the
  `Arc<Mutex<…>>` is taken briefly; the `get` operation is non-mutating;
  `ServiceVip` is `Copy` so the lock drops before rendering).

## Wave: DESIGN / [REF] Technology Choices

- **serde** (`Serialize`/`Deserialize`) + **utoipa** (`ToSchema`) — same
  wire-layer stack as ADR-0051. The describe enum renders as an OpenAPI
  `oneOf` with `discriminator: propertyName: kind`.
- **No rkyv at the wire layer** — describe wire is JSON-only; ADR-0048's
  rkyv envelope discipline governs the intent layer only (mirror of
  ADR-0051 § 3). No `rkyv::Archive` derive, no envelope enum.
- `ServiceVip` newtype (`overdrive-core::id`, ADR-0049 § 2) — already
  `Display`/`FromStr`/`Serialize`/`Deserialize`-complete; renders as a
  dotted-quad string on the wire.
- All OSS / existing workspace deps. No new dependency, no proprietary
  component.

## Wave: DESIGN / [REF] Reuse Analysis (MANDATORY — default EXTEND)

| Overlapping component | Existing? | Decision | Rationale |
|---|---|---|---|
| Wire-layer `oneOf` enum module | `api::submit` (ADR-0051) | **EXTEND pattern, CREATE sibling** | `api::describe` mirrors `api::submit` structurally; the module precedent + the `pub mod` wiring are reused. A distinct module (not a shared one) is required because submit is a request shape and describe a response shape (D1). |
| Job describe render | `From<&Job> for JobSpecInput` (`aggregate/mod.rs:896`) | **REUSE verbatim** | The Job arm has no platform-derived field; the existing impl IS the render path, wrapped in `DescribeSpecOutput::Job(...)`. Zero behavioural change to the realised Job describe path. |
| VIP retrieval | `PersistentServiceVipAllocator::get` (`persistent_service_vip.rs:251`) | **REUSE** | Read-only `get` already exists (added for `BackendDiscoveryBridge`, brief § 63). No new method (D3). |
| Per-kind validating/render constructor pattern | `from_submit` family (ADR-0051 § 4) | **EXTEND pattern** | `to_describe` mirrors `from_submit` as the inverse direction. |
| `ServiceVip` / `ListenerInput` / `ResourcesInput` / `DriverInput` types | exist (ADR-0049 / 0051) | **REUSE** | `ServiceSpecOutput` composes existing wire/id types; no duplication. |
| Typed error → HTTP mapping | `ControlPlaneError` + ADR-0015 | **EXTEND** | New `ServiceVipMissing` variant follows the `ViewStoreBoot`/`Cgroup` dedicated-variant precedent; not `internal(String)`. |
| Describe response struct | `WorkloadDescription` (`api.rs:153`) | **EXTEND** | Single field-type flip on an existing struct; not a new struct. |

No CREATE-NEW where an EXTEND was available. The only genuinely new
files are `api/describe.rs` (mandated distinct by D1) and the new error
variant.

## Wave: DESIGN / [REF] Open Questions

None load-bearing remain — OQ-1/4/5/7 are user-locked (above). Two
pre-existing, out-of-scope items, surfaced for the orchestrator (see
return message), NOT written as forward pointers here:

- `JobSpecInput` module relocation (`aggregate::` → `api::`) — the
  same incoherence ADR-0051 § "Future Work" already records; this
  feature reuses `JobSpecInput` in place and neither worsens nor fixes
  it. No new issue created.
- gRPC bridge / SDK generation consuming the describe `oneOf` schema —
  same ADR-0051 § "Future Work" item; out of scope.

## Wave: DESIGN / [REF] Migration & Cascade

Single-cut (ADR-0064 § 6). Consumer cascade estimate: **~6–10 files**
(see ADR-0064 § Consequences → "Consumer cascade" for the breakdown).
Precise count is a DELIVER PREPARE-pass `grep` concern; the load-bearing
property is the cascade is bounded and lands in one atomic commit
alongside OpenAPI regen + golden update.

## Wave: DESIGN / [REF] C4

This change adds no new container. The describe path is a read against
the same `IntentStore` + `ServiceVipAllocator` the control-plane
container already holds (brief § 67 L2 topology unchanged). A
component-level Mermaid view of the describe projection path is at
`design/c4-component-describe.md`.
