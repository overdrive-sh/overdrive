# Journey — Submit a Job Through the Walking-Skeleton Control Plane

**Feature**: phase-1-control-plane-core
**Persona**: Ana, Overdrive platform engineer (same persona as phase-1-foundation). Secondary persona: Omar, a platform operator who will eventually use the same CLI once auth lands in Phase 5. Phase 1 auth posture: unauthenticated against a local control plane (CLI → local REST API on `https://127.0.0.1:7001`).
**Goal**: Ana runs `overdrive job submit`, the control plane accepts the spec (REST + JSON over axum/rustls), the IntentStore commits it via `LocalStore`, a reconciler primitive is registered and idling (no job-lifecycle reconciler in Phase 1), and `overdrive alloc status` returns the committed spec.

## Emotional arc — Confidence Building

```
Skeptical     →     Focused     →     Confident     →     Trusting
(step 1)            (step 2)          (step 3)            (step 4)

"The API stub      "The store    "The reconciler   "The CLI echoes
 is there, but     round-tripped runtime answered   the same spec I
 does the          my job spec   my registration    submitted — the
 walking-          through the   call — the port    whole loop is
 skeleton wire     IntentStore.  is real, not a     alive."
 end-to-end?"                    stub."
```

## ASCII flow

```
[1. submit a job]              [2. commit to intent]         [3. register a reconciler]    [4. inspect allocations]
  CLI opens connection           Handler validates spec        Reconciler trait is in        CLI asks API for alloc
  to local endpoint;             + commits via IntentStore     the runtime registry;         status; API reads
  POSTs the spec as JSON         (LocalStore on redb).         evaluation broker is          observation; CLI prints
  to /v1/jobs.                                                 alive but idling.             what it finds.
  (Emotion: skeptical)           (Emotion: focused)            (Emotion: confident)          (Emotion: trusting)
  ─────────────────────────────────────────────────────────────────────────────────────────────────────────────
  Artifact:                      Artifact:                     Artifact:                     Artifact:
    ${job_spec_bytes}              ${intent_key}                 ${reconciler_registry}        ${alloc_row}
    ${rest_endpoint}               ${commit_index}               ${evaluation_broker_state}    ${commit_index}
```

## TUI Mockups

### Step 1 — `overdrive job submit <spec>`

```
$ overdrive job submit ./payments.toml
    Connecting to https://127.0.0.1:7001 ... ok
    Reading spec from ./payments.toml ... ok
    Submitting job "payments" ...

    Accepted.

        Job ID:        payments
        Intent key:    jobs/payments
        Commit index:  17
        Endpoint:      https://127.0.0.1:7001

    Next: overdrive alloc status --job payments
```

Shared artifacts:

- `${job_spec_bytes}` — the Job aggregate serialised to JSON on the wire (`POST /v1/jobs` body) and archived via rkyv once the handler has validated it. Spec digest is always `ContentHash::of(rkyv_archived_bytes)` — serde-JSON is never hashed. (source: overdrive-core Job aggregate + control-plane handler archive path)
- `${rest_endpoint}` — from `OVERDRIVE_ENDPOINT` env or `--endpoint` flag (source: CLI config); `/v1` path prefix is applied by the HTTP client
- `${intent_key}` — `jobs/<job_id>` (source: `IntentStore` key schema, committed in control plane)
- `${commit_index}` — IntentStore commit counter (source: `LocalStore` transaction metadata)

Emotional state:

- Entry: skeptical — "the CLI previously just logged and exited. Does this actually reach the control plane?"
- Exit: focused — "the control plane acknowledged a specific commit index. I have something to ask for back."

Failure modes:

- Endpoint unreachable — CLI prints an actionable error with `curl`-style remediation
- Spec fails validation — handler returns `400 Bad Request` with a structured JSON error body naming the offending field
- IntentStore put fails (disk full, snapshot corruption) — handler returns `500 Internal Server Error` with the underlying cause embedded in the structured JSON body (no raw stack trace)

### Step 2 — IntentStore commit (no direct user touchpoint; observable via CLI later)

```
   [ control plane ]
   POST /v1/jobs (axum handler)
     └─ deserialises Json<SubmitJobRequest>
     └─ validates newtypes via Job::from_spec (JobId, Resources, …)
     └─ wraps Job in rkyv archive bytes
     └─ IntentStore::put("jobs/payments", bytes) via LocalStore::txn
           └─ redb single-write transaction commits → commit_index = 17
     └─ responds 200 OK with Json<SubmitJobResponse { job_id, commit_index }>
```

Shared artifacts:

- `${intent_key}` — canonical form derived from `JobId::display()`
- `${commit_index}` — LocalStore transaction counter (opaque to CLI; echoed back in the JSON response body)

Integration checkpoint:

- The bytes read back from `IntentStore::get("jobs/<id>")` MUST rkyv-access into a `Job` value equal to the one committed
- The handler MUST NOT commit if `JobId::from_str(...)` rejects the identifier — the validating constructor is the gate
- JSON is the wire format; rkyv is the store format; the two serialisations are decoupled and the spec_digest is always computed over the rkyv-archived bytes

Failure modes:

- rkyv archive of the validated `Job` diverges from what the client can compute locally from the same spec (symptom: DST hash drift between handler input and IntentStore content)
- Handler committed the job but responded with `commit_index = 0` (off-by-one)

### Step 3 — Reconciler primitive is alive but idling

```
$ overdrive cluster status
    Connecting to https://127.0.0.1:7001 ... ok

    Cluster
        Mode:          single
        Region:        local
        Commit index:  17

    Reconcilers
        noop-heartbeat          idling (evaluations queued: 0, cancelled: 0)

    Note: Phase 1 ships the reconciler primitive (trait + runtime +
    evaluation broker). The job-lifecycle reconciler lands in
    phase-1-first-workload.
```

Shared artifacts:

- `${reconciler_registry}` — list of registered reconcilers from the runtime (source: `ReconcilerRuntime` in the control plane)
- `${evaluation_broker_state}` — cancelable-eval-set metrics (source: runtime's broker)

Emotional state:

- Entry: focused — "I've committed a spec. Did the runtime *actually* wire a reconciler primitive in?"
- Exit: confident — "a reconciler is registered and the evaluation broker's counters are real, not a stub."

Integration checkpoint:

- At least ONE reconciler (the `noop-heartbeat` placeholder) MUST be registered at boot — otherwise we cannot prove the trait + runtime contract holds end-to-end
- The evaluation broker MUST accept at least one `Evaluation` and drain it without panic (demonstrable via the DST harness; surfaced here for operator visibility)
- The broker's "cancelled" counter MUST increment when a second evaluation arrives for the same `(reconciler, target)` key while one is pending (tested in DST, visible here as `cancelled: N`)

Failure modes:

- Runtime compiles but `ReconcilerRuntime::registered()` returns empty — lifecycle bug
- Evaluation broker key-collapse under duplicate keys doesn't fire (cancelled stays at 0 forever)

### Step 4 — `overdrive alloc status --job payments`

```
$ overdrive alloc status --job payments
    Connecting to https://127.0.0.1:7001 ... ok

    Job "payments"
        Intent committed at:   index 17
        Spec digest:           sha256:7f3a9b12…
        Replicas requested:    1
        Allocations:           0 (none placed — scheduler lands in
                                phase-1-first-workload)

    Observation
        No alloc_status rows for this job (nothing is running yet).
```

Shared artifacts:

- `${alloc_row}` — `alloc_status` observation row or "no rows" (source: `ObservationStore::read`)
- `${spec_digest}` — SHA-256 of the rkyv-archived JobSpec (source: ContentHash newtype from phase-1-foundation)

Emotional state:

- Entry: focused — "the spec went in. Is the loop closed?"
- Exit: trusting — "the CLI echoes back what I submitted, and it honestly says 'nothing is running' because that's deferred — no smoke and mirrors."

Integration checkpoint:

- The spec digest displayed by `alloc status` MUST equal the digest derivable from the `overdrive job submit` input file
- If `alloc_status` rows are absent, the CLI MUST say so explicitly (not pretend to have data)

Failure modes:

- CLI displays stale cached data instead of reading through the API
- `alloc status` hides the empty state and prints an empty table (bad empty-state design)

## Integration validation (shared-artifact consistency)

| Artifact | Appears in | Must match across | Failure message |
|---|---|---|---|
| `${job_spec_bytes}` (rkyv digest) | Steps 1, 2, 4 | Steps 1, 4 | "If the CLI-computed digest differs from what `alloc status` echoes, the IntentStore round-trip is broken." |
| `${intent_key}` | Steps 1, 2 | Steps 1, 2 | "The key the CLI prints on submit must equal the key the API uses for the IntentStore put. Divergence means key-derivation lives in two places." |
| `${commit_index}` | Steps 1, 3, 4 | Steps 1, 4 | "The commit index echoed after submit must equal the index `alloc status` reads back, or readers are looking at a different IntentStore transaction than the writer committed to." |

## Emotional coherence

```
Skeptical → Focused → Confident → Trusting
```

Confidence Building pattern. No jarring transitions. Every step deposits a small, observable win (acknowledged commit, reconciler registered, spec round-tripped).

## CLI UX compliance

- Command shape `overdrive <noun> <verb>` (matches phase-1-foundation precedent)
- `--help` available on every subcommand (reused scaffold from CLI stub)
- Output is human-readable by default; `--json` deferred to Phase 2 when the OpenAPI schema is stable enough to publish for broad SDK consumption
- Errors answer "what / why / how to fix" per `nw-ux-tui-patterns`
- First output within 100ms (local REST call over HTTP/2 + rustls; no remote network)
- No spinners needed — every call is <100ms in the walking-skeleton envelope
