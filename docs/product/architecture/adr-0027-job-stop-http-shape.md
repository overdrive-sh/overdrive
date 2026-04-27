# ADR-0027 — Job-stop HTTP shape: `POST /v1/jobs/{id}:stop`; separate `IntentKey::for_job_stop` intent key

## Status

Accepted. 2026-04-27. Decision-makers: Morgan (proposing), user
ratification 2026-04-27. Tags: phase-1, first-workload,
application-arch.

## Context

US-03 ships `overdrive job stop <id>` end-to-end as part of the
job-lifecycle reconciler slice. The shim and reconciler dispatch
shape are fixed by ADR-0023; the remaining decision is the **HTTP
endpoint shape** for the stop affordance.

Two options surfaced in the DISCUSS wave:

- **(a)** `POST /v1/jobs/{id}:stop` — verb-as-suffix on the
  resource path; idempotent semantics naturally clear ("posting
  the stop verb on a stopped job is a no-op").
- **(b)** `DELETE /v1/jobs/{id}` — HTTP standard for resource
  removal; idempotent semantics differ ("a DELETE on a missing
  resource is 404 or 204?"; "does DELETE remove the spec or
  just stop the workload?").

Two sub-decisions follow:

- **s1**: how is the stopped state represented in the
  IntentStore? Two shapes: (a) overwrite the existing intent key
  with a "stopped" variant of the spec; (b) write a separate
  intent key (`stop/<job_id>`) recording the operator's stop
  intent.

User decision (D7, s1): `POST /v1/jobs/{id}:stop`; separate
`IntentKey::for_job_stop` intent key.

## Decision

### 1. Endpoint: `POST /v1/jobs/{id}:stop`

The HTTP endpoint for stopping a job is:

```
POST /v1/jobs/{id}:stop
```

Empty request body. Response body:

```json
{
    "job_id": "payments",
    "outcome": "stopped"      // | "already_stopped"
}
```

Response semantics:

- **200 OK** with `outcome: "stopped"` — first stop request
  for a running or pending job. The reconciler will pick up
  the stop intent on the next tick and emit
  `Action::StopAllocation` for each running allocation.
- **200 OK** with `outcome: "already_stopped"` — the job is
  already stopped (or has no surviving allocations). The
  request is idempotent; no state change.
- **404 Not Found** with structured `ErrorBody` — no job
  with the supplied id exists in IntentStore. The CLI exits
  non-zero and surfaces the actionable error per the
  `nw-ux-tui-patterns` shape.
- **409 Conflict** is NOT used. There is no "I want to start
  a stopped job" semantic in Phase 1; future Phase 2+ may
  introduce a `:start` companion verb, in which case 409
  may apply when start and stop conflict on the wire.

### 2. Path-suffix verb (`:stop`) is the chosen REST shape

The `{id}:stop` suffix follows the AIP-136 (Google API Improvement
Proposal) custom-verb convention: `POST /v1/<collection>/<id>:<verb>`.
It signals "this is a verb on a specific resource, not a
resource removal." Examples in production REST APIs include
GitHub's `POST /repos/{owner}/{repo}/actions/runs/{id}:cancel` and
Google Cloud's `POST /v1/projects/{p}/instances/{i}:reset`.

The suffix shape composes naturally with the existing endpoint
table:

```
POST   /v1/jobs                    (existing — submit)
GET    /v1/jobs/{id}               (existing — describe)
POST   /v1/jobs/{id}:stop          (NEW — this ADR)
```

Future verbs (`:start`, `:restart`, `:cancel-pending-rollout`)
extend the same column; the resource path stays canonical
(`/v1/jobs/{id}` is "the resource"; `:verb` modifies the
operation).

### 3. Sub-decision s1: separate `IntentKey::for_job_stop` intent key

The stop affordance writes a SEPARATE intent key, NOT a
modified-spec write at the existing `IntentKey::for_job(&JobId)`
key.

```rust
// in overdrive-core::intent_key

impl IntentKey {
    pub fn for_job(job_id: &JobId) -> Self { … }            // existing

    /// Operator-recorded "stop this job" intent. Written by the
    /// `POST /v1/jobs/{id}:stop` handler; read by the
    /// `JobLifecycle` reconciler's `hydrate_desired` path.
    pub fn for_job_stop(job_id: &JobId) -> Self { … }       // NEW
}
```

Canonical string form for the new key:

```
jobs/<JobId::display>/stop
```

The reconciler's `hydrate_desired` path reads BOTH keys:

```rust
let job_spec     = intent.get(&IntentKey::for_job(&job_id))?;          // Some(spec) | None
let stop_intent  = intent.get(&IntentKey::for_job_stop(&job_id))?;     // Some(_)    | None

let desired_state = match (job_spec, stop_intent) {
    (Some(spec), None)          => DesiredState::Run { spec },
    (Some(_), Some(_))          => DesiredState::Stop,        // op stopped a running job
    (None, _)                   => DesiredState::Absent,      // job never existed (or deleted)
};
```

The reconciler emits `Action::StopAllocation` for each running
allocation when `desired_state == DesiredState::Stop`. Once all
allocations terminate, the reconciler optionally tombstones the
stop key (Phase 2+; Phase 1 leaves both keys in place — they
are observation-equivalent to "this job is stopped" and do not
participate in the Phase 1 storage budget).

### 4. CLI invocation: `overdrive job stop <id>`

```
$ overdrive job stop payments
Stopped job 'payments'.

$ overdrive job stop payments
Job 'payments' was already stopped.

$ overdrive job stop unknown
Error: no job with id 'unknown'.
```

The CLI invokes `POST /v1/jobs/payments:stop` with empty body,
parses the JSON response, and renders per outcome. Exit code 0
on `stopped` or `already_stopped`; exit code 1 on 404.

## Alternatives considered

### Alternative A — `DELETE /v1/jobs/{id}`

Use HTTP DELETE on the resource path. Stop = remove the job.

**Rejected** for three reasons:

1. **DELETE conflates "stop the workload" with "remove the
   spec."** In the AIP-136 / GitHub / Google Cloud patterns,
   DELETE is reserved for *resource removal*. A stopped job
   in Overdrive is still a logical resource — its spec is
   readable via `GET /v1/jobs/{id}` for audit / rollback /
   debugging. Using DELETE for stop forces a confused
   semantic: either DELETE leaves the spec readable
   (violating REST's principle that DELETE means "gone") or it
   removes the spec (and the operator loses observability
   into what was running).
2. **Idempotent semantics are murkier.** REST DELETE is
   conventionally idempotent (multiple DELETEs return the
   same final state, typically 204), but the question
   "DELETE on a missing resource — is that 404 or 204?" is
   contested across ecosystems. The AIP-136 verb-suffix
   pattern is unambiguous: 200 + `outcome: already_stopped`
   on the second call is honest about idempotency without
   relying on convention.
3. **Future companion verbs.** When Phase 2+ ships
   `overdrive job start <id>` to un-stop a job, the
   companion endpoint is `POST /v1/jobs/{id}:start`. With
   DELETE-for-stop, `start` would have to be... POST on what?
   DELETE is one-way; start has nowhere to live. The
   verb-suffix pattern composes; DELETE does not.

### Alternative B — `PATCH /v1/jobs/{id}` with `{state: "stopped"}` body

PATCH-style state mutation. The request body declares the new
state; the server transitions.

**Rejected.** PATCH is a delta operator on a resource
representation; the natural use is "change a field of the
spec." Job lifecycle state is not a spec field — it is an
operator intent that triggers reconciler convergence. PATCH
also opens the question "what other fields can I PATCH?" —
which Phase 1 has no answer for. The verb-suffix pattern
restricts the operator to the verbs the platform supports, no
more.

### Sub-decision alternative s1' — Modified-spec write at existing key

Stopping a job overwrites the existing `IntentKey::for_job(...)`
record with a stopped variant of the spec (e.g. `Job` gains a
`stopped: bool` field, or replicas drops to 0).

**Rejected** for two reasons:

1. **It mutates the spec.** The submitted-and-validated `Job`
   aggregate is the operator's declared intent at submission
   time; mutating it on stop loses fidelity. An operator
   wanting to inspect what they originally submitted has to
   know the system rewrote their spec.
2. **It conflates two operator actions.** "Submit a job" and
   "stop a job" are two distinct events; recording them at the
   same key with the same shape obscures the lineage. A
   separate key per event preserves an honest audit trail
   that future Phase 2+ reconciler-replay tests can reason
   over.

The separate-key shape also matches the natural Phase 2+
extension to `:start` (un-stop): write a `for_job_start_after_stop`
key, or simply tombstone the stop key. Either composes; the
modified-spec shape forces a per-verb decision about how to
unwind the previous mutation.

### Alternative C — Just-DELETE-it for Phase 1, revisit in Phase 2+

Use DELETE for Phase 1 simplicity; revisit when companion verbs
land.

**Rejected.** API shape decisions ratchet. Once
`DELETE /v1/jobs/{id}` is shipped, removing it means a breaking
change for any operator script written against it. Phase 1
walking-skeleton-only is the time to pick the right shape — the
verb-suffix pattern is mature, well-precedented, and
forward-compatible with the verbs Phase 2+ will inevitably need.

## Consequences

### Positive

- **Composes with future verbs.** `:start`, `:restart`,
  `:cancel`, `:reschedule`, `:checkpoint` all extend the same
  column.
- **Idempotent semantics are explicit, not relied on.** The
  response body's `outcome` field signals "this was a no-op"
  without the operator having to interpret HTTP status code
  conventions.
- **Spec is preserved.** `GET /v1/jobs/{id}` continues to
  return the operator's submitted spec verbatim, regardless of
  whether the job is stopped. Audit trail intact.
- **Separate intent key is observation-honest.** The existence
  of a `stop` key in IntentStore is the canonical record of
  "the operator stopped this job at <commit time>." Phase 2+
  audit / replay sees the event distinctly from the original
  submission.
- **Reconciler's `hydrate_desired` reads two cheap keys.** Two
  IntentStore reads per evaluation; both are O(1) point lookups
  in `LocalIntentStore`. No materially new I/O cost.

### Negative

- **AIP-136 verb-suffix is one column wider in API shape.**
  Tools like generic OpenAPI clients (e.g.
  `openapi-generator`-generated Go SDKs) handle path-suffix
  verbs less ergonomically than pure RESTful CRUD. Phase 1 has
  exactly one OpenAPI consumer (the in-tree CLI per ADR-0014),
  which is hand-rolled — no generator pain. Future external
  consumers may need a one-line adaptation per generator.
- **Two intent keys per job.** Storage cost is trivial (two
  small redb entries instead of one); the operator's mental
  model is "every job lives at one path, plus its stop intent
  lives at the same path with a `/stop` suffix." Acceptable
  cost.
- **Phase 2+ cleanup logic.** Tombstoning the stop key after
  all allocations terminate is a Phase 2+ reconciler job. Phase
  1 leaves both keys in place; the storage cost is bounded by
  the operator's job count.

### Quality-attribute impact

- **Maintainability — modifiability**: positive. Adding
  Phase 2+ companion verbs is mechanical (new endpoint, new
  intent key, new reconciler arm).
- **Maintainability — interoperability**: marginally negative.
  Path-suffix verbs are less compatible with naive
  OpenAPI-generator tooling, but the project's hand-rolled
  CLI per ADR-0014 absorbs the cost.
- **Reliability — fault tolerance**: positive. Idempotent
  outcome rendering means duplicate stop calls produce
  predictable output.
- **Security — accountability**: positive. Separate intent
  key gives stop events distinct identity in IntentStore;
  audit logs surface them as their own commit.
- **Performance — time behaviour**: neutral. Two reads per
  reconciler tick instead of one; both O(1).

## Compliance

- **ADR-0008** (REST + OpenAPI over axum/rustls): the new
  endpoint follows the existing `/v1` prefix and uses
  `axum`'s router. The OpenAPI schema (per ADR-0009) is
  regenerated by `cargo xtask openapi-gen` and the
  `cargo xtask openapi-check` CI gate catches drift.
- **ADR-0011** (intent-side `Job` aggregate): preserved
  unchanged. The `Job` aggregate does not gain a `stopped`
  field; intent keys carry the lifecycle event.
- **ADR-0014** (CLI HTTP client + shared types): the `stop`
  request/response types extend
  `overdrive-control-plane::api`; CLI imports them. New types:
  `JobStopRequest` (empty), `JobStopResponse` (job_id +
  outcome), `JobStopOutcome` enum (`Stopped` | `AlreadyStopped`).
- **ADR-0015** (HTTP error mapping): 404 + 200 are the only
  status codes; both go through the existing
  `ControlPlaneError::to_response` envelope.
- **ADR-0023** (action shim): the reconciler's
  `Action::StopAllocation` flows through the existing shim
  signature; no new dispatch shape.

## References

- ADR-0008 — Control-plane external API REST + OpenAPI over
  axum/rustls.
- ADR-0009 — OpenAPI schema derivation; the `openapi-check`
  CI gate.
- ADR-0011 — `Job` / `Node` / `Allocation` aggregates intent
  layer.
- ADR-0014 — CLI HTTP client; shared request/response types.
- ADR-0015 — HTTP error mapping; status-code matrix.
- ADR-0023 — Action shim placement.
- AIP-136 — Custom Methods (Google API Improvement Proposal);
  the verb-suffix convention.
- Whitepaper §18 — Reconciler primitive; convergence on
  declared intent.
- `docs/feature/phase-1-first-workload/discuss/wave-decisions.md`
  — Priority Two item 7 + sub-decision s1.
- `docs/feature/phase-1-first-workload/discuss/user-stories.md`
  — US-03 ships the stop affordance end-to-end.
