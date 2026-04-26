# Shared Artifact Registry — phase-1-control-plane-core

Data that flows across journey steps. Every `${variable}` has a single source of truth and documented consumers.

## Registry

```yaml
shared_artifacts:

  job_spec_bytes:
    source_of_truth: >
      The Job aggregate is serialised to JSON on the REST wire edge
      (`POST /v1/jobs` request body) and archived via rkyv once the
      handler has validated it through the aggregate constructor. The
      aggregate is defined in overdrive-core (this feature introduces
      the struct); archival happens once in the control plane's submit
      handler before IntentStore::put is called. JSON is at the edge;
      rkyv is at the store; spec_digest is always ContentHash::of over
      the rkyv-archived bytes.
    consumers:
      - "CLI submit JSON request body (POST /v1/jobs)"
      - "API validator (rkyv-access path before commit)"
      - "IntentStore value at jobs/<job_id>"
      - "alloc status spec-digest display"
    owner: "overdrive-core (Job aggregate) + control plane (archive path)"
    integration_risk: >
      HIGH — if the CLI and the API take different archival paths, the
      spec digest displayed by alloc status diverges from what the user
      can compute locally. This breaks deterministic hashing (see
      development.md §"Hashing requires deterministic serialization").
      rkyv is the committed canonicalisation per ADR-0002.
    validation: >
      A proptest in overdrive-core: for any valid Job, the archived bytes
      are stable across N invocations. An acceptance test: submit a spec,
      then call alloc status, assert the spec_digest shown equals
      ContentHash::of(archived_bytes) computed locally from the same file.

  rest_endpoint:
    source_of_truth: >
      CLI --endpoint flag (default https://127.0.0.1:7001) or
      OVERDRIVE_ENDPOINT env var. Resolved once in the CLI's Cli::parse
      flow. The `/v1` version prefix is appended to this base URL when
      composing individual endpoint paths (e.g. `/v1/jobs`,
      `/v1/cluster/info`). HTTP/2 over rustls with a self-generated
      local-dev certificate in Phase 1.
    consumers:
      - "CLI connect path"
      - "CLI success and failure output"
    owner: "overdrive-cli"
    integration_risk: >
      LOW — the endpoint is an operator-facing input that the CLI
      displays back. Not exchanged across module boundaries.
    validation: >
      CLI unit test asserts default equals https://127.0.0.1:7001 and
      env override takes precedence over the flag default. Additional
      assertion: the `/v1` path prefix is applied by the client, not
      carried in the base URL.

  intent_key:
    source_of_truth: >
      Canonical key derivation `jobs/<JobId::display()>` implemented once
      in overdrive-core (proposed: `IntentKey::for_job(&JobId)` or
      equivalent). The CLI derives it for display; the API uses the same
      function for IntentStore::put. One function, one output.
    consumers:
      - "CLI submit success output"
      - "IntentStore::put target key (control plane)"
      - "IntentStore::get target key (alloc status read path)"
    owner: "overdrive-core"
    integration_risk: >
      HIGH — if the derivation lives in two places, the reader can end
      up looking at a different row than the writer committed to. The
      mitigation is to expose ONE function in overdrive-core and require
      both the CLI and the control plane to call it.
    validation: >
      A test that takes a JobId and asserts the derived key equals the
      canonical prefix + the newtype's Display output byte-for-byte.

  commit_index:
    source_of_truth: >
      LocalStore transaction sequence exposed by the IntentStore
      implementation. In Phase 1 this is the monotonically increasing
      redb transaction counter surfaced through a new accessor on the
      store.
    consumers:
      - "SubmitJobResponse JSON body (POST /v1/jobs 200 OK)"
      - "CLI submit output"
      - "cluster status output"
      - "alloc status output"
    owner: "overdrive-store-local (surface) + overdrive-core IntentStore trait (contract)"
    integration_risk: >
      MEDIUM — the value is informational to the operator in Phase 1 but
      becomes load-bearing in Phase 2+ (idempotent resubmit, raft log
      identity). If the implementation makes it non-monotonic now,
      downstream assumptions break silently.
    validation: >
      A property test asserts commit_index strictly increases across
      successive put/txn calls on a LocalStore instance.

  reconciler_registry:
    source_of_truth: >
      ReconcilerRuntime::registered() inside the control plane. This is
      the runtime's internal registry; exposed through a read-only API
      for `cluster status` and also readable from DST invariants.
    consumers:
      - "cluster status output (CLI)"
      - "DST invariant: at_least_one_reconciler_registered"
      - "ReconcilerRuntime internal dispatch"
    owner: "control plane (ReconcilerRuntime)"
    integration_risk: >
      HIGH — a runtime that compiles but registers zero reconcilers at
      boot would silently void the #17 reconciler-primitive feature. A
      fresh DST invariant catches the empty registry; the CLI display
      makes it visible to operators.
    validation: >
      DST invariant asserts the default runtime boots with ≥ 1 registered
      reconciler. Acceptance test asserts `cluster status` lists at least
      one reconciler on a fresh control plane.

  evaluation_broker_state:
    source_of_truth: >
      EvaluationBroker's internal counters (queued, cancelled). The
      broker implements the cancelable-eval-set pattern described in
      whitepaper §18 — on a duplicate (reconciler, target) key while one
      is pending, the prior evaluation is moved to the cancelable set
      and reaped in bulk.
    consumers:
      - "cluster status output"
      - "DST invariant: duplicate_evaluations_collapse"
      - "Reconciler runtime internal back-pressure decisions (Phase 2+)"
    owner: "control plane (EvaluationBroker inside ReconcilerRuntime)"
    integration_risk: >
      HIGH — the cancelable-eval-set is the native-not-retrofitted
      mitigation that keeps the runtime from collapsing under
      correlated-failure storms. If this regresses silently, Phase 2+
      reconciler scale becomes a Nomad-shaped incident waiting to
      happen.
    validation: >
      DST scenario fires N concurrent evaluations against the same key
      and asserts only one dispatched + N-1 cancelled counter. Inspection
      via `cluster status` makes the counter visible to operators.

  alloc_row:
    source_of_truth: >
      alloc_status row in the ObservationStore. In Phase 1 no scheduler
      or driver populates the row, so reads return zero rows by design.
      The CLI renders an explicit empty state.
    consumers:
      - "alloc status output (CLI)"
      - "(Phase 1+) gateway service-endpoint resolution"
      - "(Phase 1+) scheduler read path for bin-packing feedback"
    owner: "overdrive-core ObservationStore trait + future writers"
    integration_risk: >
      MEDIUM — the table shape is locked from phase-1-foundation
      (brief §6: alloc_status { alloc_id, job_id, node_id, state,
      updated_at }). No new schema in this feature. Risk is limited to
      the CLI empty-state rendering.
    validation: >
      Acceptance test asserts zero-row responses render the explicit
      empty-state message (not a blank table) and name the next feature.

  spec_digest:
    source_of_truth: >
      ContentHash::of(rkyv_archived_job_bytes) — ContentHash is the
      SHA-256 newtype already shipped in phase-1-foundation. The archival
      path lives in overdrive-core (this feature introduces the Job
      aggregate; archival is the existing rkyv path).
    consumers:
      - "alloc status output (human-visible digest)"
      - "(Phase 1+) idempotent submit detection"
      - "(Phase 1+) audit logs"
    owner: "overdrive-core"
    integration_risk: >
      HIGH — if the CLI and the control plane derive the digest
      differently, operators see drift between the local file hash and
      what the platform reports. The mitigation is the same as for
      job_spec_bytes: one archival path, proven by rkyv's by-construction
      determinism (ADR-0002) and proptest.
    validation: >
      Proptest: for any valid Job, ContentHash::of(archive(J)) equals the
      digest the API echoes back after a round-trip commit + read. The
      test runs through the API, not by calling archive() directly twice.

  openapi_schema:
    source_of_truth: >
      The control-plane OpenAPI 3.1 schema document — derived from the
      Rust request / response types via `utoipa` or `aide` (DESIGN picks)
      and checked into the workspace (recommended path
      `api/openapi.yaml`) or regenerated on build with a CI gate that
      fails on drift. The CLI and the control plane either share the
      generated Rust types directly (OpenAPI-generated client) or
      hand-roll request / response types that are asserted against the
      schema in CI.
    consumers:
      - "CLI HTTP client request / response types"
      - "Control plane axum handler extractors and response types"
      - "(Phase 2+) external SDKs generated from the OpenAPI document"
    owner: "(DESIGN wave decision) — proposed: `api/openapi.yaml` at the workspace root, regenerated from Rust types via `utoipa` or `aide`"
    integration_risk: >
      HIGH — if the Rust types drift from the checked-in OpenAPI
      document and no CI gate catches it, the CLI and the server can
      agree in review but disagree on the wire. The mitigation is the
      schema-lint gate in CI (`openapi-check`-style) that compares the
      generated schema against the checked-in document on every PR.
      Secondary mitigation: no hand-rolled request / response type on
      either side may shadow the schema-aligned form.
    validation: >
      Build-time: the schema-lint gate passes (generated schema equals
      the checked-in document). Acceptance test: the CLI submits
      against a real server instance and the round-trip succeeds
      without per-side field adaptation outside the schema-aligned
      types.
```

## Consistency check questions (answered)

- **Does every ${variable} in TUI mockups have a documented source?** Yes — see the table above.
- **If `commit_index` format changes, would all consumers automatically update?** Yes — the value is formatted once in the JSON response body and displayed verbatim everywhere. No consumer reparses it.
- **Are there hardcoded values that should reference a shared artifact?** The default endpoint `https://127.0.0.1:7001` is a CLI default; documented as such. Not exchanged across module boundaries; acceptable.
- **Do any two steps display the same data from different sources?** No — `intent_key` goes through one overdrive-core function in every step; `spec_digest` goes through one ContentHash::of() path; `commit_index` is produced by the LocalStore transaction and passed through.

## Quality gates

- [x] **Journey completeness** — all four steps have goal, command, mockup, artifacts, emotional annotation, integration checkpoint, failure modes, gherkin.
- [x] **Emotional coherence** — Skeptical → Focused → Confident → Trusting. Confidence Building pattern. No jarring transitions; every step deposits an observable win.
- [x] **Horizontal integration** — Three artifacts (`intent_key`, `commit_index`, `spec_digest`) appear across multiple steps. Each has a single source of truth and `must_match_across` validation.
- [x] **CLI UX compliance** — Clig.dev shape. `overdrive <noun> <verb>`. Actionable errors. Honest empty states. First output within 100ms on localhost.

## Changelog

| Date | Change |
|---|---|
| 2026-04-23 | Initial registry for phase-1-control-plane-core. |
| 2026-04-23 | Transport pivot: `grpc_endpoint` → `rest_endpoint` (https + `/v1` prefix); `grpc_service_shape` → `openapi_schema` (OpenAPI 3.1 derived from Rust types via `utoipa` / `aide`, gated by CI schema-lint). |
