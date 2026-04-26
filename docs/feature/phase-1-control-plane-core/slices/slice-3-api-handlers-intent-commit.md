# Slice 3 — API handlers commit to IntentStore + ObservationStore reads

**Story**: US-03
**Walking skeleton row**: 3 (Commit intent)
**Effort**: ~1 day
**Depends on**: Slice 1 (aggregates), Slice 2 (REST service surface).

## Outcome

The SubmitJob axum handler archives the Job aggregate via rkyv and writes through `IntentStore::put` on `LocalStore`. DescribeJob / AllocStatus / NodeList axum handlers read back via `IntentStore::get` and `ObservationStore::read`. LocalStore exposes a monotonic `commit_index()` surfaced in every response that commits. Handlers map typed errors to HTTP status codes with structured JSON bodies.

## Value hypothesis

*If* the API commits a spec and Describe can't read it back byte-identical, *then* the IntentStore contract is broken and every downstream claim about durability / audit / idempotency fails — and phase-1-foundation's guarantees do not carry through to operator-observable behaviour.

## Scope (in)

- SubmitJob axum handler (`POST /v1/jobs`):
  - Extracts `Json<SubmitJobRequest>` from the request body
  - Constructs a typed Job aggregate via the validating constructor (newtype validators fire)
  - rkyv-archives the aggregate through the canonical path
  - Calls `IntentStore::put(IntentKey::for_job(&job_id), archived_bytes)`
  - Responds `200 OK` with `Json<SubmitJobResponse> { job_id, commit_index }`
- DescribeJob axum handler (`GET /v1/jobs/{id}`):
  - Calls `IntentStore::get(IntentKey::for_job(&job_id))`
  - rkyv-accesses the bytes back into a Job aggregate
  - Computes `ContentHash::of(archived_bytes)` as the spec_digest
  - Responds `200 OK` with `Json<JobDescription> { spec, commit_index, spec_digest }` or `404 Not Found` with a structured error body
- AllocStatus axum handler (`GET /v1/allocs`):
  - Calls `ObservationStore::read` on the `alloc_status` table (schema locked in phase-1-foundation brief §6)
  - Responds `200 OK` with a possibly-empty row set
- NodeList axum handler (`GET /v1/nodes`):
  - Calls `ObservationStore::read` on the `node_health` table
  - Responds `200 OK` with a possibly-empty row set
- `LocalStore::commit_index()` — new read-only accessor exposing the monotonic redb transaction sequence
- Validating-constructor gate: the handler rejects malformed input with `400 Bad Request` *before* any IntentStore write

## Scope (out)

- Any write to the ObservationStore — nothing in Phase 1 writes observation rows yet; the scheduler + drivers (phase-1-first-workload) are the first writers
- `StopJob` / `DeleteJob` endpoints — deferred to phase-1-first-workload alongside the job-lifecycle reconciler
- Watch / streaming responses — deferred
- Idempotent re-submit detection — deferred (requires scheduler integration for meaningful semantics)

## Target KPI

- Round-trip invariant: for any valid submitted Job `J`, `GET /v1/jobs/{id}` returns a response whose `spec == J` after rkyv access (proptest through the real API)
- `commit_index` strictly increases across successive submits on the same LocalStore instance (property test)
- HTTP status code per rejection tier: validation failure → `400 Bad Request`, unknown resource → `404 Not Found`, duplicate-intent-key (if surfaced) → `409 Conflict`, unexpected infra failure → `500 Internal Server Error`
- Error responses carry a structured JSON body (`{"error": "...", "message": "...", "field": "..."}`); no raw stack traces reach the client

## Acceptance flavour

See US-03 scenarios. Focus: rkyv archive → store → read → access round-trip, monotonic commit_index, validating-constructor gate, HTTP status-code discipline.

## Failure modes to defend

- Handler commits even when the newtype validator would reject (gate bypassed)
- rkyv archival path differs between CLI-local compute and handler commit → spec_digest drifts
- `commit_index` resets or non-monotonic after a restart
- ObservationStore reads return stale data from a cache instead of round-tripping through the store
- Error responses collapse distinct failure classes into `500 Internal Server Error`, losing the information the CLI needs to render actionable output
