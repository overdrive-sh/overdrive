# Slice 1 — Job / Node / Allocation aggregates + canonical intent keys

**Story**: US-01
**Walking skeleton row**: 1 (Model the job shape)
**Effort**: ~1 day
**Depends on**: phase-1-foundation newtypes (JobId, NodeId, AllocationId, PolicyId, InvestigationId, Region, ContentHash, SchematicId, SpiffeId, CertSerial, CorrelationKey).

## Outcome

Ana imports `Job`, `Node`, `Allocation` from `overdrive-core`, constructs them through validating constructors that consume the phase-1-foundation newtypes, archives them via rkyv, and derives intent keys through a single canonical function shared by the CLI and the control plane.

## Value hypothesis

*If* the aggregate structs and the canonical key function don't exist with rkyv-archived determinism, *then* the commit index, spec digest, and intent key shown by the CLI drift from what the control plane commits, and the walking-skeleton round-trip invariant fails before any API work even begins.

## Scope (in)

- `Job`, `Node`, `Allocation` aggregate structs in `overdrive-core`
- Placeholder-only structs for `Policy`, `Investigation` (field subset sufficient to be referenced; not consumed in this slice's tests)
- rkyv derives on each aggregate; serde derives for CLI/config loading
- A canonical intent-key derivation (`IntentKey::for_job(&JobId)` and peers) with a proptest asserting key stability
- A `Resources` struct (cpu_cores, memory_bytes — whatever driver.rs already exposes, reused not reinvented)

## Scope (out)

- Aggregate *behaviour* (methods that mutate state or issue Actions) — Phase 1 treats aggregates as data
- `Policy` Rego/WASM module references — stub only
- `Investigation` lifecycle — stub only
- REST request / response types + OpenAPI schema (Slice 2)

## Target KPI

- 100% of the three aggregates round-trip through rkyv archive → access without loss (proptest)
- `IntentKey::for_job(&JobId)` matches CLI-computed and API-used keys byte-for-byte for all valid JobIds (proptest)

## Acceptance flavour

See US-01 scenarios in `user-stories.md`. Focus: rkyv round-trip, canonical key stability, validating constructors reject malformed inputs before the aggregate is constructable.

## Failure modes to defend

- Aggregate fields default in a way that makes two "empty" aggregates hash-differ across runs
- Intent key derivation lives in two places (CLI and server) and drifts silently
- `Resources` is duplicated (one in `driver.rs`, one on `Job`) with subtly different fields
