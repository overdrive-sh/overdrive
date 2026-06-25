//! axum route handlers for the Phase 1 control-plane API.
//!
//! One function per ADR-0008 endpoint:
//!
//! | Endpoint | Handler |
//! |---|---|
//! | `POST /v1/jobs` | `submit_workload` |
//! | `GET /v1/jobs/{id}` | `describe_workload` |
//! | `GET /v1/cluster/info` | `cluster_status` |
//! | `GET /v1/allocs` | `alloc_status` |
//! | `GET /v1/nodes` | `node_list` |
//!
//! Step 03-01 lands the `submit_workload` body; the other four remain RED
//! scaffolds owned by subsequent deliver steps.

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use overdrive_core::aggregate::{
    AggregateError, IntentKey, Job, ServiceV1, WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::describe::DescribeSpecOutput;
use overdrive_core::id::{SpiffeId, WorkloadId};
use overdrive_core::reconcilers::{
    RESTART_BACKOFF_CEILING, Reconciler, ReconcilerName, TargetResource, WorkloadLifecycle,
};
use overdrive_core::traits::intent_store::{IntentStore, PutOutcome};
use overdrive_core::traits::observation_store::AllocState;
use overdrive_core::traits::observation_store::AllocStatusRow;
use overdrive_core::transition_reason::TerminalCondition;
use serde::Deserialize;

use crate::api::{
    AllocStateWire, RestartBudget, StopOutcome, StopWorkloadResponse, TransitionRecord,
    TransitionSource,
};

use crate::AppState;
use crate::api;
use crate::error::ControlPlaneError;
use overdrive_core::eval_broker::Evaluation;

/// Enqueue a `(job-lifecycle, job/<id>)` evaluation onto the runtime
/// broker. Called from `submit_workload` and `stop_workload` after the
/// `IntentStore` write commits — the edge-triggered ingress half of
/// whitepaper §18 *Triggering Model — Hybrid by Design*. The
/// convergence-loop spawn in [`crate::run_server_with_obs_and_driver`]
/// drains the broker on the next tick and dispatches one
/// [`crate::reconciler_runtime::run_convergence_tick`] call per pending
/// evaluation.
///
/// Per `fix-convergence-loop-not-spawned` Step 01-02 (RCA Option B2):
/// without this enqueue, `IntentStore` writes would commit but no
/// convergence would ever run — `cluster_status.broker.dispatched`
/// would permanently read 0.
fn enqueue_workload_lifecycle_eval(
    state: &AppState,
    workload_id: &WorkloadId,
) -> Result<(), ControlPlaneError> {
    // Source from the trait const per the `refactor-reconciler-static-name`
    // RCA — `WorkloadLifecycle::NAME` is the single compile-time anchor for
    // the kebab-case literal, and the `ReconcilerName::new` validator
    // accepts it by construction.
    let reconciler = ReconcilerName::new(<WorkloadLifecycle as Reconciler>::NAME).map_err(|e| {
        ControlPlaneError::internal(
            "ReconcilerName::new(<WorkloadLifecycle as Reconciler>::NAME)",
            e,
        )
    })?;
    let target_string = format!("job/{workload_id}");
    let target = TargetResource::new(&target_string)
        .map_err(|e| ControlPlaneError::internal("TargetResource::new(job/<id>)", e))?;
    state.runtime.broker().submit(Evaluation { reconciler, target });
    Ok(())
}

/// Parse a `WorkloadId` from a path parameter, attaching `field = Some("id")` to
/// the validation error so HTTP clients can branch on the error origin.
///
/// The `field` discriminator is the contract that lets a client tell
/// path-parameter validation apart from request-body validation —
/// `AggregateError::Id`'s `#[from]` pass-through correctly leaves
/// `field = None` because it has no caller-side context to name. Handlers
/// that DO have caller-side context (the `OpenAPI` path parameter `id`)
/// attach it explicitly through this helper.
fn parse_workload_id_path(job_id_str: &str) -> Result<WorkloadId, ControlPlaneError> {
    WorkloadId::new(job_id_str).map_err(|e| ControlPlaneError::Validation {
        message: e.to_string(),
        field: Some("id".to_owned()),
    })
}

impl From<overdrive_core::traits::observation_store::AllocStatusRow> for api::AllocStatusRowBody {
    fn from(row: overdrive_core::traits::observation_store::AllocStatusRow) -> Self {
        // Phase 1 the action shim writes Driver(Exec) for rows it
        // produced post-spawn; the reconciler emits its own progress
        // markers via Reconciler. Without the source-attribution
        // metadata on the row itself we default to Reconciler — Phase 2
        // adds explicit attribution per ADR-0033 §1.
        let last_transition = row.reason.clone().map(|reason| TransitionRecord {
            from: None,
            to: AllocStateWire::from(row.state),
            reason,
            source: TransitionSource::Reconciler,
            at: format!("(c={},w={})", row.updated_at.counter, row.updated_at.writer),
        });

        // The `started_at` field is populated for rows that have left
        // the Pending state at least once. Phase 1 detects this by the
        // observed state being non-Pending — Phase 2+ will track the
        // first-non-Pending logical timestamp explicitly.
        let started_at = match row.state {
            AllocState::Pending => None,
            _ => Some(format!("(c={},w={})", row.updated_at.counter, row.updated_at.writer)),
        };

        Self {
            alloc_id: row.alloc_id.to_string(),
            workload_id: row.workload_id.to_string(),
            node_id: row.node_id.to_string(),
            state: AllocStateWire::from(row.state),
            reason: row.reason,
            // Resources cannot be reconstructed from the row alone in
            // Phase 1 — the row schema does not carry the requested
            // envelope. The handler that knows the JobSpec overrides
            // this field; the bare conversion uses zeroes.
            resources: api::ResourcesBody { cpu_milli: 0, memory_bytes: 0 },
            started_at,
            exit_code: None,
            last_transition,
            error: row.detail,
        }
    }
}

/// Query parameters for `GET /v1/allocs`.
///
/// `job` selects the snapshot for a specific `WorkloadId`. The query
/// parameter is REQUIRED — a missing `?job=` returns HTTP 400 with
/// `field = Some("job")`. The handler reads the `IntentStore` for the
/// named job, returns 404 if absent, then projects matching rows + the
/// `WorkloadLifecycle` view-cache restart counts into the populated
/// envelope shape per ADR-0033 §1.
#[derive(Debug, Clone, Deserialize)]
pub struct AllocStatusQuery {
    /// Canonical `WorkloadId` to filter on. Required. Missing → HTTP 400
    /// with `field = Some("job")`.
    pub job: Option<String>,
}

impl From<overdrive_core::traits::observation_store::NodeHealthRow> for api::NodeRowBody {
    fn from(row: overdrive_core::traits::observation_store::NodeHealthRow) -> Self {
        Self { node_id: row.node_id.to_string(), region: row.region.to_string() }
    }
}

/// `POST /v1/jobs` — validate, archive via rkyv, commit through the
/// intent store, return `{workload_id, spec_digest, outcome}`.
///
/// Idempotency contract (ADR-0015 §4 amended by ADR-0020):
///
/// * A spec whose rkyv-archived bytes are absent at the canonical
///   `jobs/<WorkloadId>` key returns HTTP 200 with `outcome =
///   IdempotencyOutcome::Inserted` and the canonical `spec_digest`
///   (`PutOutcome::Inserted` path).
/// * A spec whose rkyv-archived bytes are byte-identical to what is
///   already stored at the same key returns HTTP 200 with `outcome =
///   IdempotencyOutcome::Unchanged` and the same `spec_digest`. No
///   write occurred (`PutOutcome::KeyExists { existing }` path with
///   byte equality).
/// * A spec whose rkyv-archived bytes DIFFER from what is stored at
///   the same key returns HTTP 409 Conflict with an `ErrorBody`.
///   Conflict is an HTTP-status concern; the `outcome` field is
///   absent on the 409 path (ADR-0015 §4 amendment).
///
/// `spec_digest` is the lowercase-hex SHA-256 of the canonical
/// rkyv-archived bytes (ADR-0002, development.md §Hashing). On the
/// `Unchanged` path the digest is computed once over the candidate
/// bytes and used both for the byte-equality check (against the
/// `existing` bytes returned by `PutOutcome::KeyExists`) and for the
/// response body.
// Slice 02 step 02-02 — `POST /v1/jobs` 200 response is polymorphic
// on `Accept` per DESIGN [D6] / [D8]. The path declares both content
// types under a single 200 response — `application/json` returns a
// one-shot `SubmitWorkloadResponse`; `application/x-ndjson` returns a
// stream of per-kind streaming-event lines (`JobSubmitEvent` /
// `ServiceSubmitEvent`). utoipa 5.x's `responses(..., content(
// (T1 = "mime1"), (T2 = "mime2") ))` group form is the multi-content-
// type shape (see utoipa-gen 5.4.0 src/path/response.rs §"content"
// branch).
#[utoipa::path(
    post,
    path = "/v1/jobs",
    request_body = api::SubmitWorkloadRequest,
    responses(
        (status = 200, description = "Job accepted (Accept negotiates one-shot vs streaming)",
            content(
                (api::SubmitWorkloadResponse = "application/json"),
                (crate::streaming::ServiceSubmitEvent       = "application/x-ndjson"),
            )
        ),
        (status = 400, description = "Validation error", body = api::ErrorBody),
        (status = 409, description = "Conflict at existing key", body = api::ErrorBody),
        (status = 500, description = "Internal error", body = api::ErrorBody),
    ),
    tag = "jobs",
)]
// The handler body grew past the 100-line clippy default with the
// ADR-0051 `SubmitSpecInput` dispatch added at the top — splitting
// per-arm helpers would obscure the linear "validate → wrap →
// idempotency → branch on Accept" shape that makes the function
// comprehensible. Same precedent: `streaming.rs` (file-level allow)
// and `action_shim/mod.rs::reconciler_action_to_shim` (fn-level allow).
#[allow(clippy::too_many_lines)]
pub async fn submit_workload(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<api::SubmitWorkloadRequest>,
) -> Result<Response, ControlPlaneError> {
    let want_streaming = wants_ndjson(&headers);
    // 1. Dispatch on the wire-side `SubmitSpecInput` discriminator per
    //    ADR-0051 § 4 / OQ-6 and route each arm through its per-kind
    //    validating constructor (`JobV1::from_submit` /
    //    `ServiceV1::from_submit` / `ScheduleV1::from_submit`). This
    //    is the wire → intent boundary; the constructors are the
    //    single validation surface. Field-name preservation per
    //    ADR-0015: scalar-field validation failures
    //    (`AggregateError::Validation { field, message }`) flatten
    //    into the top-level `ControlPlaneError::Validation` variant
    //    with `field: Some(field.to_string())`. Non-validation
    //    `AggregateError` shapes fall through the `#[from]` blanket
    //    conversion to `ControlPlaneError::Aggregate(_)` — `to_response`
    //    still maps them to HTTP 400 with `error: "validation"`.
    //
    //    Step 02-03b NOTE: Service / Schedule arms return a structured
    //    rejection placeholder. The full Service-arm wiring
    //    (allocator integration, `alloc_status` widening, streaming)
    //    lands in step 02-03c per the roadmap split; the Schedule
    //    arm's `from_submit` is a `todo!()` RED scaffold that the
    //    rejection placeholder makes structurally unreachable.
    let intent = match request.spec {
        overdrive_core::api::submit::SubmitSpecInput::Job(jsi) => {
            WorkloadIntent::Job(Job::from_submit(jsi).map_err(|e| match e {
                AggregateError::Validation { field, message } => {
                    ControlPlaneError::Validation { field: Some(field.to_owned()), message }
                }
                other => ControlPlaneError::Aggregate(other),
            })?)
        }
        overdrive_core::api::submit::SubmitSpecInput::Service(ssi) => {
            WorkloadIntent::Service(ServiceV1::from_submit(ssi).map_err(|e| match e {
                AggregateError::Validation { field, message } => {
                    ControlPlaneError::Validation { field: Some(field.to_owned()), message }
                }
                other => ControlPlaneError::Aggregate(other),
            })?)
        }
        overdrive_core::api::submit::SubmitSpecInput::Schedule(_) => {
            return Err(ControlPlaneError::Validation {
                field: Some("spec.kind".to_owned()),
                message:
                    "schedule submission is not yet implemented (ADR-0051 OQ-5 — RED scaffold)"
                        .to_owned(),
            });
        }
    };

    // 2. Wrap the validated workload payload in the kind-agnostic
    //    `WorkloadIntent` aggregate per ADR-0050 and archive canonically
    //    via the typed `WorkloadIntentEnvelope` codec. Two archivals of
    //    the same logical intent produce byte-identical bytes — this is
    //    what makes the idempotency check byte-equality instead of
    //    semantic-equality.
    //
    //    Per ADR-0050 § 2, `WorkloadIntent::{Job,Service,Schedule}` all
    //    pass through the same codec. Per step 02-03d, the Schedule arm
    //    is structurally gated above; the dispatch below routes Job and
    //    Service to their respective response shapes.
    let workload_id = match &intent {
        WorkloadIntent::Job(j) => j.id.clone(),
        WorkloadIntent::Service(s) => s.id.clone(),
        WorkloadIntent::Schedule(s) => s.id.clone(),
    };
    let archived = intent
        .archive_for_store()
        .map_err(|e| ControlPlaneError::internal("rkyv archive of WorkloadIntent", e))?;

    // 3. Derive the canonical intent key (`workloads/<WorkloadId>`)
    //    per ADR-0050 OQ-5 single-cut migration.
    let key = IntentKey::for_workload(&workload_id);

    // 4. Compute the canonical `spec_digest` via the typed codec
    //    method. Per ADR-0050 the digest is over the rkyv-archived
    //    inner `WorkloadIntentV1` payload bytes — distinct from
    //    pre-migration `Job::spec_digest`, but stable across reads
    //    and used by `ServiceVipAllocator` per ADR-0049.
    let spec_digest_hash = intent
        .spec_digest()
        .map_err(|e| ControlPlaneError::internal("spec_digest of WorkloadIntent", e))?;
    let spec_digest = spec_digest_hash.to_string();

    // 4a. Service-arm frontend-address assignment (dial-by-name-responder
    //     step 01-05; ADR-0072 REV-3, GH #243). This is the WRITER seam: the
    //     deploy-time `assign(<job>)` at Service declaration that binds the
    //     stable per-`<job>` frontend address `F` the `name_index` (01-03)
    //     reader answers with. Service-only, mirroring the `service_vip`
    //     allocate's `matches!(intent, WorkloadIntent::Service(_))` guard — a
    //     Job / Schedule submit assigns NO frontend addr (frontends are a
    //     Service-name concern).
    //
    //     ORDERING (D5 review fix): the frontend assign runs BEFORE the VIP
    //     allocate below, NOT after. The frontend allocator is purely in-memory
    //     (empty-on-boot, idempotent, rebuilt from declared intent), so its only
    //     fallible exit is frontend-block EXHAUSTION. By placing it first, an
    //     exhaustion early-return precedes any DURABLE VIP commit — so it can
    //     never leak a fsync'd VIP that has no `WorkloadIntent` (and thus no
    //     `Action::ReleaseServiceVip`) to release it. A later VIP-allocate
    //     failure leaves only a benign in-memory `<job> → F` frontend binding
    //     with no persisted intent: empty-on-boot, idempotent on retry, unread
    //     by any reader for a `<job>` that has no declared Service.
    //
    //     IDEMPOTENT per `<job>` at the allocator layer (FRONTEND-02): an
    //     already-held `<job>` (a byte-identical resubmit reaching the
    //     KeyExists path, OR the boot rebuild having already assigned it)
    //     returns its EXISTING `F` unchanged, consuming no new address. So the
    //     handler adds NO idempotency logic here — calling `assign` on every
    //     Service submit is correct, and a resubmit never consumes a second
    //     address nor changes the binding.
    //
    //     OQ-1: the `<job>` key is derived
    //     `MeshServiceName::new("<id>.<SUFFIX>")` — byte-identical to the
    //     `name_index` reader's `job_of` derivation, so the WRITER's key is the
    //     SAME key the READER looks up (DDN-2 single-owner). An exhausted
    //     frontend block fails the submit CLOSED via the typed
    //     `ControlPlaneError::FrontendRebuild` (HTTP 503; never a silent
    //     reuse). A Service id that is not a valid v1 single-label mesh name is
    //     not mesh-dialable by name (the reader skips it too) — assigning no
    //     frontend addr is the design's intended scope, not a submit failure.
    //
    //     The assigned `F` is held by the `frontend_addr_allocator` value on
    //     `AppState` (the ONE shared instance the readers observe). NOT
    //     released on the happy KeyExists path; the conflict-rollback path below
    //     follows the SAME release-on-conflict-ONLY discipline as the VIP.
    if let WorkloadIntent::Service(service_v1) = &intent
        && let Ok(job) = overdrive_core::id::MeshServiceName::new(&format!(
            "{}.{}",
            service_v1.id.as_str(),
            overdrive_core::id::MeshServiceName::SUFFIX
        ))
    {
        state.frontend_addr_allocator.assign(&job).map_err(|source| {
            ControlPlaneError::FrontendRebuild(
                crate::dns_responder::boot_rebuild::FrontendRebuildError::Exhausted { job, source },
            )
        })?;
    }

    // 4b. Service-arm VIP allocation per ADR-0049 (amended 2026-05-15)
    //     / service-vip-allocator step 02-03d.
    //
    //     Runs AFTER the in-memory frontend assign above (D5 review fix) so the
    //     DURABLE VIP commit is the LAST fallible allocation before the
    //     admission `put_if_absent` — no fsync'd VIP can be stranded by a
    //     subsequent frontend-exhaustion early-return.
    //
    //     Concurrency contract per `.claude/rules/development.md` §
    //     "Concurrency & async" → "Never hold a lock across `.await`":
    //     the allocator lock IS the serialisation point — the
    //     content-addressed memo lookup + the inner store write happen
    //     under one guard so that two concurrent submits for the SAME
    //     spec_digest both see the same VIP (one wins the lock and
    //     issues; the other hits the memo). The guard is dropped
    //     EXPLICITLY before any further `.await` (the admission
    //     `put_if_absent` below); the next `.await` therefore does NOT
    //     hold the allocator mutex.
    //
    //     `PersistentServiceVipAllocator::allocate` itself fsyncs the
    //     allocator entry through the byte-level `IntentStore` before
    //     returning Ok, so the durable allocator memo is committed
    //     before this handler returns the VIP to the client. On
    //     byte-identical resubmit the memo hit short-circuits without
    //     a store write — that is the property S-VIP-04 pins.
    let service_vip = if matches!(intent, WorkloadIntent::Service(_)) {
        let digest_bytes: [u8; 32] = *spec_digest_hash.as_bytes();
        let mut guard = state.allocator.lock().await;
        let vip = guard.allocate(digest_bytes).await?;
        drop(guard);
        Some(vip)
    } else {
        None
    };

    // 5a. Per ADR-0047 §1 / slice 02 of `workload-kind-discriminator`:
    //     persist the workload-kind discriminator at
    //     `IntentKey::for_workload_kind` so the streaming endpoint can
    //     dispatch on per-kind sibling-event enums (ADR-0047 §3 [D7])
    //     and the reconciler runtime's `hydrate_desired` can populate
    //     `WorkloadLifecycleState.workload_kind` for the natural-exit
    //     emission path (ADR-0037 Amendment 2026-05-10).
    //
    //     Step 02-03d: the discriminator is derived from the
    //     `WorkloadIntent` variant. `mut` is preserved because the
    //     `KeyExists` idempotency path below may re-read a previously-
    //     persisted discriminator to keep streaming dispatch stable
    //     across resubmits.
    let mut workload_kind = match &intent {
        WorkloadIntent::Job(_) => WorkloadKind::Job,
        WorkloadIntent::Service(_) => WorkloadKind::Service,
        WorkloadIntent::Schedule(_) => WorkloadKind::Schedule,
    };
    let kind_key = IntentKey::for_workload_kind(&workload_id);

    // 5. Atomic idempotency / conflict detection via `put_if_absent`.
    //    The existence check and the insert happen in a single store
    //    transaction — this closes the TOCTOU window that would open
    //    under a naive `get` (read txn) + `put` (write txn) pair,
    //    where two concurrent submitters for the same key could both
    //    see `None` on the read and both fall through to the write,
    //    silently clobbering the first spec.
    let outcome = match state.store.put_if_absent(key.as_bytes(), archived.as_ref()).await? {
        PutOutcome::Inserted => {
            // Persist the kind discriminator alongside the job.
            // Use `put` (overwrite-OK) rather than `put_if_absent` —
            // a Job at the same key cannot have a different kind by
            // construction (idempotency check on the spec above
            // requires byte-identical re-submission), so writing the
            // discriminator unconditionally is safe and survives a
            // crashed-mid-submit retry that wrote the spec but not
            // the kind on the prior attempt.
            state
                .store
                .put(kind_key.as_bytes(), &[workload_kind.discriminator_byte()])
                .await
                .map_err(|e| ControlPlaneError::internal("persist workload kind", e))?;
            // Maintain the in-memory `ListenerFactStore` on the
            // intent-change write edge per ADR-0062 § Decision (2) /
            // feature-delta sub-decision 2 ("writer-bumped
            // invalidation"). The upsert runs AFTER the intent
            // `put_if_absent` returned `Inserted` AND after the
            // kind-discriminator persist — the intent SSOT is committed
            // first, so a crash between intent-commit and fact-insert is
            // repaired by the next boot's `rebuild_from_intent`
            // re-projection (ADR-0062 crash-consistency).
            //
            // Only `WorkloadIntent::Service(_)` with an allocated VIP
            // contributes listener facts; Job / Schedule intents allocate
            // no VIP (`service_vip` is `None`) and upsert nothing.
            //
            // Lock discipline (`.claude/rules/development.md` §
            // "Concurrency & async" → "Never hold a lock across
            // `.await`"): acquire the `listener_facts` guard, perform the
            // synchronous `upsert`, and DROP it before the subsequent
            // `enqueue_workload_lifecycle_eval` (which is itself sync, but
            // dropping eagerly keeps the no-guard-across-`.await` shape
            // local and future-proof). Mirrors the allocator-guard
            // pattern at step 4a above.
            if let (WorkloadIntent::Service(service_v1), Some(vip)) = (&intent, &service_vip) {
                let mut facts_guard = state.listener_facts.lock().await;
                facts_guard.upsert(workload_id.clone(), vip, &service_v1.listeners);
                drop(facts_guard);
            }
            // Edge-triggered ingress per whitepaper §18: enqueue an
            // evaluation for the job-lifecycle reconciler so the
            // convergence-loop spawn picks it up on the next tick.
            // Per `fix-convergence-loop-not-spawned` Step 01-02.
            enqueue_workload_lifecycle_eval(&state, &workload_id)?;
            api::IdempotencyOutcome::Inserted
        }
        PutOutcome::KeyExists { existing } => {
            if existing.as_ref() == archived.as_ref() {
                // Byte-identical re-submission: 200 OK with
                // `outcome = Unchanged`; the `spec_digest` is stable
                // across N retries by construction (rkyv archival is
                // canonical). Enqueue an evaluation anyway: the
                // resubmit is operator intent to re-converge against
                // any drift, and the broker collapses duplicates per
                // §18 evaluation-broker semantics so a flapping
                // resubmit produces one pending eval, not N.
                //
                // Read the stored kind discriminator so streaming
                // dispatch matches the reconciler's view. The first
                // submit wrote the authoritative kind; a re-submit
                // re-reads the persisted byte rather than re-deriving
                // from the wire (Step 02-03b: wire always carries Job
                // until 02-03c lands the Service arm, but the read-
                // back keeps the path identical to its pre-02-03b
                // semantics).
                if let Some(stored_kind_bytes) =
                    state.store.get(kind_key.as_bytes()).await.map_err(|e| {
                        ControlPlaneError::internal("read workload kind (unchanged path)", e)
                    })?
                    && let Some(b) = stored_kind_bytes.first().copied()
                {
                    workload_kind = WorkloadKind::from_discriminator_byte(b);
                }
                enqueue_workload_lifecycle_eval(&state, &workload_id)?;
                api::IdempotencyOutcome::Unchanged
            } else {
                // Different spec at the same key — 409 Conflict.
                // Conflict is HTTP-status, never a wire `outcome`
                // value (ADR-0015 §4 amended by ADR-0020). No
                // evaluation enqueued — the intent did not change.
                //
                // Release any VIP that was allocated for the rejected
                // spec before returning the error. The allocation at
                // step 4a runs before `put_if_absent`; without this
                // cleanup the VIP leaks permanently — no downstream
                // `Action::ReleaseServiceVip` will fire because the
                // rejected spec never gets a persisted WorkloadIntent.
                if service_vip.is_some() {
                    let digest_bytes: [u8; 32] = *spec_digest_hash.as_bytes();
                    let mut guard = state.allocator.lock().await;
                    guard
                        .release(&digest_bytes)
                        .await
                        .map_err(|e| ControlPlaneError::internal("release VIP on conflict", e))?;
                    drop(guard);
                }
                // `ListenerFactStore` is a deliberate NO-OP on this
                // branch (ADR-0062 § Decision (2)): the rejected spec
                // never had its facts upserted — the upsert lives only on
                // the `Inserted` arm above, which this branch did not
                // take. There is nothing to remove (symmetric with the
                // VIP, which was allocated-then-released for the rejected
                // spec). Adding a `remove_workload` here would evict the
                // EXISTING workload's facts on a conflicting resubmit —
                // exactly the corruption U6 guards against.
                //
                // The FRONTEND allocator is ALSO a deliberate NO-OP here
                // (dial-by-name-responder step 01-05) — but for a DIFFERENT
                // reason than the VIP's allocate-then-release. The VIP is keyed
                // by `spec_digest`, so a conflicting (different-spec) resubmit
                // allocates a SECOND, distinct VIP that leaks unless released.
                // The frontend allocator is keyed by the logical `<job>` (the
                // workload id), which is IDENTICAL on a conflicting resubmit, so
                // the step-4a `assign(<job>)` above was an idempotent no-op that
                // returned the EXISTING `F` — it consumed no new address and
                // left the binding unchanged. There is nothing to release.
                // Calling `release(<job>)` here would EVICT the live workload's
                // frontend `F` on a rejected resubmit — the exact
                // stale-`F`/eviction corruption ASSIGN-02 + U6 forbid. The
                // binding survives the conflict untouched precisely BECAUSE the
                // allocator is `<job>`-keyed and idempotent.
                return Err(ControlPlaneError::Conflict {
                    message: format!("a different spec is already registered at {}", key.as_str()),
                });
            }
        }
    };

    // Branch on Accept header per [D6] / [D8]. JSON lane preserves
    // the existing `SubmitWorkloadResponse` shape unchanged (back-compat
    // S-CP-08). NDJSON lane delegates to the per-kind streaming loop
    // per ADR-0047 §3 [D7] / ADR-0056 / ADR-0059 §Q6:
    //   * Job     → `build_workload_stream`  (typed `JobSubmitEvent`)
    //   * Service → `build_service_stream`   (typed `ServiceSubmitEvent`)
    //   * Schedule is rejected at validation step (HTTP 400); the
    //     unreachable!() arm is the structural defense if that
    //     rejection drifts.
    if want_streaming {
        let body = match workload_kind {
            WorkloadKind::Job => {
                let accepted = crate::streaming::build_workload_accepted(
                    spec_digest,
                    key.as_str().to_owned(),
                    outcome,
                );
                let stream = crate::streaming::build_workload_stream(
                    state.clone(),
                    workload_id.clone(),
                    accepted,
                );
                Body::from_stream(stream)
            }
            WorkloadKind::Service => {
                let accepted = crate::streaming::build_service_accepted(
                    spec_digest,
                    key.as_str().to_owned(),
                    outcome,
                );
                let stream = crate::streaming::build_service_stream(
                    state.clone(),
                    workload_id.clone(),
                    accepted,
                );
                Body::from_stream(stream)
            }
            WorkloadKind::Schedule => unreachable!(
                "Schedule rejected at submit (handlers.rs validation step \
                 returns HTTP 400 before reaching this branch)"
            ),
        };
        let response = Response::builder()
            .status(axum::http::StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/x-ndjson")
            .body(body)
            .map_err(|e| ControlPlaneError::internal("build streaming response", e))?;
        Ok(response)
    } else {
        let body = api::SubmitWorkloadResponse {
            workload_id: workload_id.to_string(),
            spec_digest,
            outcome,
            vip: service_vip.map(|v| v.get().to_string()),
        };
        Ok(Json(body).into_response())
    }
}

/// Returns `true` when the `Accept` header signals the
/// `application/x-ndjson` streaming lane. Missing header, `*/*`, and
/// `application/json` all fall through to the JSON back-compat lane.
///
/// Phase 1 uses simple `contains("application/x-ndjson")` — no
/// q-value parsing — because the CLI sends one explicit value at a
/// time. Phase 2+ may upgrade to RFC 7231 §5.3.2 q-value resolution
/// if multiple Accept values become common.
fn wants_ndjson(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| s.contains("application/x-ndjson"))
}

/// `GET /v1/jobs/{id}` — read via `IntentStore::get`, rkyv-access the
/// bytes, recompute `spec_digest = ContentHash::of(archived_bytes)`.
///
/// Canonical-hashing contract (ADR-0002 + development.md §Hashing):
/// the `spec_digest` returned here is SHA-256 over the exact byte
/// sequence we pulled out of the `IntentStore` — i.e. the rkyv-archived
/// bytes of a validated `Job`. We deliberately do NOT re-canonicalise,
/// do NOT route through JCS, and do NOT hash `serde_json::to_string(&job)`
/// — any of those would break the byte-identity property the rest of
/// the platform depends on.
///
/// 404 contract (ADR-0015 §3): `IntentStore::get -> Ok(None)` maps to
/// `ControlPlaneError::NotFound { resource: <IntentKey::as_str()> }`,
/// which `to_response` renders as HTTP 404 with an `ErrorBody { error:
/// "not_found", ... }`.
///
/// Post-ADR-0020: the response shape is `{spec, spec_digest}` only.
/// The `commit_index` field is dropped wholesale — there is no
/// store-wide nor per-entry index on the wire (see ADR-0020).
#[utoipa::path(
    get,
    path = "/v1/jobs/{id}",
    params(
        ("id" = String, Path, description = "Canonical WorkloadId"),
    ),
    responses(
        (status = 200, description = "Workload description", body = api::WorkloadDescription),
        (status = 404, description = "Workload not found", body = api::ErrorBody),
        (status = 500, description = "Internal error", body = api::ErrorBody),
    ),
    tag = "jobs",
)]
pub async fn describe_workload(
    State(state): State<AppState>,
    Path(job_id_str): Path<String>,
) -> Result<Json<api::WorkloadDescription>, ControlPlaneError> {
    // 1. Parse the path parameter through the WorkloadId newtype. A malformed
    //    identifier (non-ASCII, wrong length, bad charset) surfaces as
    //    HTTP 400 with `field: Some("id")` via `parse_workload_id_path`.
    let workload_id = parse_workload_id_path(&job_id_str)?;

    // 2. Derive the canonical intent key and read from the authoritative
    //    store. Missing key → NotFound → HTTP 404.
    //
    //    The `NotFound` `resource` string uses `IntentKey::as_str()`
    //    (the canonical `<prefix>/<id>` rendering) rather than
    //    hand-formatting the literal here — doing the latter would
    //    duplicate the job-prefix literal into a second production
    //    file, which the `intent_key_canonical` grep-gate in
    //    `overdrive-core` explicitly forbids.
    let key = IntentKey::for_workload(&workload_id);
    let bytes = state
        .store
        .get(key.as_bytes())
        .await?
        .ok_or_else(|| ControlPlaneError::NotFound { resource: key.as_str().to_owned() })?;

    // 3. Decode the persisted bytes via the typed `WorkloadIntentEnvelope`
    //    codec (ADR-0050 § 4 + ADR-0048 § "Intent persistence
    //    boundary"). Corruption / bit-rot in the redb file surfaces
    //    here as `IntentStoreError::Envelope`; it maps to HTTP 500
    //    via the `#[from]` blanket on `ControlPlaneError::Intent`.
    let intent =
        WorkloadIntent::from_store_bytes(&bytes, &state.intent_redb_path, Some(key.as_str()))?;

    // 4. Canonical spec_digest — SHA-256 over the rkyv-archived
    //    `WorkloadIntentV1` payload bytes per ADR-0050. Computed
    //    against the typed `WorkloadIntent` so the digest matches
    //    submit-handler output regardless of variant. The `ContentHash`
    //    object is RETAINED (not collapsed to only its `String` form):
    //    the Service arm needs `*spec_digest_hash.as_bytes()` to key the
    //    allocator read with the SAME `[u8; 32]` the submit path keys
    //    `allocate(...)` with (handlers.rs §4a). `spec_digest` is the
    //    top-level response field (lowercase-hex).
    let spec_digest_hash = intent
        .spec_digest()
        .map_err(|e| ControlPlaneError::internal("spec_digest of WorkloadIntent", e))?;
    let spec_digest = spec_digest_hash.to_string();

    // 5. Project the persisted intent onto the kind-discriminated
    //    describe-wire `oneOf` per ADR-0064. The describe response is the
    //    inverse-direction sibling of submit's `SubmitSpecInput`: where
    //    submit projects `client JSON → WorkloadIntent`, describe projects
    //    `WorkloadIntent (+ VIP) → client JSON`.
    let spec = match intent {
        // Job carries no platform-derived field; the `to_describe`
        // projection delegates to the existing `From<&Job>` render path
        // (ADR-0064 § 2).
        WorkloadIntent::Job(job) => DescribeSpecOutput::Job(job.to_describe()),
        // Service surfaces the platform-issued VIP — read-only from the
        // allocator memo (ADR-0064 OQ-7), NEVER allocated here. The memo
        // is keyed by the content-addressed digest's `[u8; 32]` bytes,
        // identical to the submit path's `allocate(digest_bytes)` key
        // (handlers.rs §4a). The guard is dropped BEFORE rendering — there
        // is no `.await` between lock and drop, so the allocator mutex is
        // never held across an await point (`.claude/rules/development.md`
        // § "Never hold a lock across `.await`").
        WorkloadIntent::Service(svc) => {
            let digest_bytes: [u8; 32] = *spec_digest_hash.as_bytes();
            let guard = state.allocator.lock().await;
            let vip = guard.get(&digest_bytes);
            drop(guard);
            // A persisted-and-describable Service always has an allocated
            // VIP — submit-time admission allocates before the intent is
            // written (ADR-0049 § 4) and the boot rebuild re-seeds the memo
            // from the intent SSOT (ADR-0049 § 8). A missing entry is a
            // broken allocate-or-rebuild invariant, surfaced as HTTP 500
            // (`ServiceVipMissing`), never `None` on the wire (ADR-0064 OQ-4).
            let vip = vip
                .ok_or(ControlPlaneError::ServiceVipMissing { spec_digest: spec_digest.clone() })?;
            DescribeSpecOutput::Service(svc.to_describe(vip))
        }
        // Schedule describe is unreachable in Phase 1 — no Schedule can be
        // persisted (`ScheduleV1::from_submit` is itself a RED scaffold per
        // ADR-0064 OQ-5). Reject with the same structured `Validation`
        // shape the submit handler uses for the unrealised Schedule path,
        // so client tooling branching on the error discriminator stays
        // consistent across submit and describe.
        WorkloadIntent::Schedule(_) => {
            return Err(ControlPlaneError::Validation {
                field: Some("id".to_owned()),
                message:
                    "describe is not available for Schedule workloads in Phase 1 (Schedule submit is unrealised)"
                        .to_owned(),
            });
        }
    };

    Ok(Json(api::WorkloadDescription { spec, spec_digest }))
}

/// `POST /v1/jobs/{id}/stop` — record a stop intent for a previously-
/// submitted job. Per ADR-0027.
///
/// AIP-136 prescribes `POST /v1/jobs/{id}:stop`. axum 0.7 cannot
/// route the `:stop` verb suffix as a single path segment because its
/// matcher (matchit) treats `:` as the path-parameter prefix —
/// `/v1/jobs/:id:stop` is rejected. We use the path-subsegment form
/// `/v1/jobs/{id}/stop` which is industry-standard, semantically
/// equivalent, and free of framework conflict. ADR-0027's guidance
/// stands; only the URL form differs.
///
/// Idempotency: the handler writes `IntentKey::for_workload_stop(<id>)`
/// via `IntentStore::put_if_absent` (atomic compare-and-set). A
/// second call sees `KeyExists` and returns
/// `outcome = AlreadyStopped` — no second write occurs.
///
/// 404 contract: a stop call against an `<id>` that was never
/// submitted (no `IntentKey::for_workload(<id>)` row) returns HTTP 404.
/// The original spec key MUST exist before a stop intent can be
/// recorded — stopping a non-existent job is operator error, not an
/// idempotent no-op.
///
/// Empty request body. The response body is `{ workload_id, outcome }`.
#[utoipa::path(
    post,
    path = "/v1/jobs/{id}/stop",
    params(
        ("id" = String, Path, description = "Canonical WorkloadId"),
    ),
    responses(
        (status = 200, description = "Workload stop recorded", body = StopWorkloadResponse),
        (status = 400, description = "Validation error", body = api::ErrorBody),
        (status = 404, description = "Workload not found", body = api::ErrorBody),
        (status = 500, description = "Internal error", body = api::ErrorBody),
    ),
    tag = "jobs",
)]
pub async fn stop_workload(
    State(state): State<AppState>,
    Path(job_id_str): Path<String>,
) -> Result<axum::Json<StopWorkloadResponse>, ControlPlaneError> {
    // 1. Parse the path parameter through the WorkloadId newtype. Same
    //    field-naming discipline as `describe_workload` — see `parse_workload_id_path`.
    let workload_id = parse_workload_id_path(&job_id_str)?;

    // 2. The job must exist before a stop can be recorded. Reading
    //    the canonical job key is the cheapest 404 check — if the
    //    row is absent we have no stop target, so 404 surfaces with
    //    the same `resource = jobs/<id>` shape as describe_workload's
    //    NotFound path.
    let job_key = IntentKey::for_workload(&workload_id);
    let job_exists = state.store.get(job_key.as_bytes()).await?.is_some();
    if !job_exists {
        return Err(ControlPlaneError::NotFound { resource: job_key.as_str().to_owned() });
    }

    // 3. Atomic put_if_absent on the stop key. The empty value is
    //    deliberate — the key's existence IS the signal. A second
    //    stop call lands on the KeyExists branch and reports
    //    AlreadyStopped without a second write.
    let stop_key = IntentKey::for_workload_stop(&workload_id);
    let outcome = match state.store.put_if_absent(stop_key.as_bytes(), b"").await? {
        PutOutcome::Inserted => StopOutcome::Stopped,
        PutOutcome::KeyExists { .. } => StopOutcome::AlreadyStopped,
    };

    // Evict the workload's listener facts on the stop edge per ADR-0062
    // § Decision (2). `stop_workload` holds ONLY the `WorkloadId` (not
    // the ServiceIds or VIP), so the store's secondary cleanup index —
    // workload → derived ServiceIds — is what makes eviction possible
    // without an intent decode or an allocator lock. `remove_workload`
    // is idempotent: a redundant stop (AlreadyStopped) finds no
    // secondary entry and is a no-op, which is why both outcomes run it
    // unconditionally.
    //
    // Lock discipline (`.claude/rules/development.md` § "Concurrency &
    // async"): acquire the `listener_facts` guard, perform the
    // synchronous `remove_workload`, and DROP it before the subsequent
    // `enqueue_workload_lifecycle_eval`.
    {
        let mut facts_guard = state.listener_facts.lock().await;
        facts_guard.remove_workload(&workload_id);
        drop(facts_guard);
    }

    // Edge-triggered ingress per whitepaper §18: enqueue an
    // evaluation for the job-lifecycle reconciler so the
    // convergence-loop spawn drives the running allocations to
    // Terminated. Both Stopped and AlreadyStopped enqueue: a redundant
    // stop arriving while the prior stop has not yet converged is
    // collapsed by the broker's `(reconciler, target)` keying.
    // Per `fix-convergence-loop-not-spawned` Step 01-02.
    enqueue_workload_lifecycle_eval(&state, &workload_id)?;

    Ok(axum::Json(StopWorkloadResponse { workload_id: workload_id.to_string(), outcome }))
}

/// `GET /v1/cluster/info` — mode, region, reconciler registry, broker
/// counters.
///
/// Post-ADR-0020 the response is the four-field shape
/// `{mode, region, reconcilers, broker}`. The `commit_index` field is
/// dropped wholesale (no rename to `writes_since_boot` — that
/// preserves the same race conditions and reset-on-restart gap under
/// a different name; see ADR-0020 §Considered alternatives §D).
/// Activity-rate observability is provided by `broker.dispatched`
/// plus the `reconcilers` list; cluster-level commit-rate signals
/// belong on Phase 5's metrics endpoint.
#[utoipa::path(
    get,
    path = "/v1/cluster/info",
    responses(
        (status = 200, description = "Cluster status", body = api::ClusterStatus),
        (status = 500, description = "Internal error", body = api::ErrorBody),
    ),
    tag = "cluster",
)]
pub async fn cluster_status(
    State(state): State<AppState>,
) -> Result<Json<api::ClusterStatus>, ControlPlaneError> {
    // Phase 1 scope — `mode` is always "single" (HA arrives Phase 2+)
    // and `region` is always "local" (multi-region arrives Phase 5+).
    // These are hard-pinned here rather than derived from config so a
    // stray config typo cannot put a non-canonical value on the wire.
    let counters = state.runtime.broker().counters();
    // `ReconcilerRuntime::registered` already returns names in canonical
    // (Ord-sorted) order — the registry is a `BTreeMap` — so JSON wire
    // stability is by construction here. No `.sort()` needed.
    let reconcilers: Vec<String> =
        state.runtime.registered().into_iter().map(|n| n.to_string()).collect();

    Ok(Json(api::ClusterStatus {
        mode: "single".to_string(),
        region: "local".to_string(),
        reconcilers,
        broker: api::BrokerCountersBody {
            queued: counters.queued,
            cancelled: counters.cancelled,
            dispatched: counters.dispatched,
        },
    }))
}

/// `GET /v1/allocs?job=<id>` — observation read on `alloc_status`.
///
/// Reads through the `ObservationStore::alloc_status_rows` trait method
/// (not the concrete `SimObservationStore` type) so Phase 2's
/// `CorrosionStore` swap is a single trait-object replacement with no
/// handler changes.
///
/// The `job` query parameter is REQUIRED. The handler reads the
/// `IntentStore` for `<id>`'s `Job`, returns 404 if absent, then
/// projects matching rows + the `WorkloadLifecycle` view-cache restart
/// counts into the populated envelope shape per ADR-0033 §1. A missing
/// `?job=` query parameter returns HTTP 400 with
/// `field = Some("job")`.
#[utoipa::path(
    get,
    path = "/v1/allocs",
    responses(
        (status = 200, description = "Allocation snapshot for the named job", body = api::AllocStatusResponse),
        (status = 400, description = "Validation error (missing or malformed job query)", body = api::ErrorBody),
        (status = 404, description = "Job not found", body = api::ErrorBody),
        (status = 500, description = "Internal error", body = api::ErrorBody),
    ),
    tag = "observation",
)]
// The handler reads + 404s the intent, projects per-variant identity /
// resources / VIP / listeners, filters rows, derives the restart budget,
// reads the kind discriminator, and (built-in-ca #215) projects the
// issued-certificate summary — a linear sequence of read-and-project
// steps that reads top-to-bottom. The pure projections are already
// extracted to `issued_certificates_for_rows` / `restart_budget_from_rows`;
// the remaining length is irreducible orchestration.
#[expect(
    clippy::too_many_lines,
    reason = "linear read-and-project orchestration; pure projections already extracted"
)]
pub async fn alloc_status(
    State(state): State<AppState>,
    Query(query): Query<AllocStatusQuery>,
) -> Result<Json<api::AllocStatusResponse>, ControlPlaneError> {
    // The `job` query parameter is required. A missing `?job=` is a
    // client error (HTTP 400), not a wildcard read — slice 01 step
    // 01-03 made `?job=<id>` the canonical shape per S-AS-09.
    let Some(ref job_str) = query.job else {
        return Err(ControlPlaneError::Validation {
            message: "missing required query parameter: job".to_owned(),
            field: Some("job".to_owned()),
        });
    };

    // Validate the WorkloadId (returns 400 with field=Some("job") on bad
    // input — the path-param helper handles this naming).
    let workload_id = WorkloadId::new(job_str).map_err(|e| ControlPlaneError::Validation {
        message: e.to_string(),
        field: Some("job".to_owned()),
    })?;

    // Read the workload aggregate — 404 if absent.
    let key = IntentKey::for_workload(&workload_id);
    let bytes = state
        .store
        .get(key.as_bytes())
        .await?
        .ok_or_else(|| ControlPlaneError::NotFound { resource: key.as_str().to_owned() })?;
    let intent =
        WorkloadIntent::from_store_bytes(&bytes, &state.intent_redb_path, Some(key.as_str()))?;
    let spec_digest_hash = intent
        .spec_digest()
        .map_err(|e| ControlPlaneError::internal("spec_digest of WorkloadIntent", e))?;
    let spec_digest = spec_digest_hash.to_string();

    // Per ADR-0050 / step 02-03d — project per-variant identity,
    // resource envelope, replica count, and (Service-only) allocated
    // VIP from the typed `WorkloadIntent`.
    let (resources_body, replicas_desired, response_vip, listeners) = match &intent {
        WorkloadIntent::Job(job) => (
            api::ResourcesBody {
                cpu_milli: job.resources.cpu_milli,
                memory_bytes: job.resources.memory_bytes,
            },
            job.replicas.get(),
            None,
            // Job carries no listeners — the operator-facing surface
            // renders no Listeners section for non-Service kinds.
            Vec::new(),
        ),
        WorkloadIntent::Service(svc) => {
            // Service-arm VIP resolution per ADR-0049 (amended
            // 2026-05-15) / step 02-03d. Lookup the allocator memo
            // keyed by content-addressed `spec_digest`. The
            // allocator is the single source of truth for the issued
            // VIP; no per-listener VIP is rendered (per Q1.A — listeners
            // are `(port, protocol)` only).
            let digest_bytes: [u8; 32] = *spec_digest_hash.as_bytes();
            let guard = state.allocator.lock().await;
            let vip = guard.get(&digest_bytes);
            drop(guard);
            (
                api::ResourcesBody {
                    cpu_milli: svc.resources.cpu_milli,
                    memory_bytes: svc.resources.memory_bytes,
                },
                svc.replicas.get(),
                vip.map(|v| v.get().to_string()),
                // Project the persisted Service intent's listeners
                // (port + Proto) onto the wire response in declaration
                // order. Persist-inputs discipline: project the actual
                // intent listeners, never synthesise. The CLI render
                // layer renders each as `<port>/<protocol>`.
                svc.listeners.clone(),
            )
        }
        WorkloadIntent::Schedule(sched) => (
            api::ResourcesBody {
                cpu_milli: sched.job.resources.cpu_milli,
                memory_bytes: sched.job.resources.memory_bytes,
            },
            sched.job.replicas.get(),
            None,
            Vec::new(),
        ),
    };

    // Filter rows to this workload and project, stamping the spec
    // resource envelope on each row body (the bare conversion zeroes
    // resources; the handler is the only call site that knows the spec).
    let raw_rows = state
        .obs
        .alloc_status_rows()
        .await
        .map_err(|e| ControlPlaneError::internal("alloc_status_rows", e))?;
    let workload_rows: Vec<AllocStatusRow> =
        raw_rows.into_iter().filter(|row| row.workload_id == workload_id).collect();

    // Per ADR-0037 §4: derive the RestartBudget from the durable
    // `AllocStatusRow.terminal` field rather than from a recomputed
    // projection over `view.restart_counts`. The reconciler is the
    // single writer of terminal claims; the durable row is the
    // single source of truth for "is this workload's replica budget
    // exhausted?".
    let restart_budget = restart_budget_from_rows(&workload_rows);

    // Built-in-ca #215 consumer-side (D-OC-7): fetch audit rows, then
    // project per running alloc (see `issued_certificates_for_rows`).
    let issued_cert_rows = state
        .obs
        .issued_certificate_rows()
        .await
        .map_err(|e| ControlPlaneError::internal("issued_certificate_rows", e))?;
    let issued_certificates = issued_certificates_for_rows(&workload_rows, &issued_cert_rows);

    let rows: Vec<api::AllocStatusRowBody> = workload_rows
        .into_iter()
        .map(|row| {
            let mut body = api::AllocStatusRowBody::from(row);
            body.resources = resources_body;
            body
        })
        .collect();
    let replicas_running =
        u32::try_from(rows.iter().filter(|r| matches!(r.state, AllocStateWire::Running)).count())
            .unwrap_or(u32::MAX);

    // Per ADR-0047 §1 / step 02-02 [D4]: read the workload-kind
    // discriminator from the dedicated `workloads/<id>/kind` intent record.
    // Phase-1 greenfield: missing record defaults to `Service` (the
    // kind-agnostic shape the reconciler emulated before slice 02-04).
    let kind_byte = state
        .store
        .get(IntentKey::for_workload_kind(&workload_id).as_bytes())
        .await?
        .and_then(|bytes| bytes.first().copied());
    let kind = kind_byte
        .map(overdrive_core::aggregate::WorkloadKind::from_discriminator_byte)
        .unwrap_or_default();

    Ok(Json(api::AllocStatusResponse {
        workload_id: Some(workload_id.to_string()),
        spec_digest: Some(spec_digest),
        replicas_desired,
        replicas_running,
        rows,
        restart_budget: Some(restart_budget),
        kind: Some(kind),
        vip: response_vip,
        listeners,
        issued_certificates,
    }))
}

/// Project, per RUNNING alloc, the single latest-by-`issuance_ordinal`
/// issued-certificate audit row whose SPIFFE identity matches that alloc
/// (built-in-ca #215 consumer-side, D-OC-7 / ADR-0067 #215-boundary).
///
/// The selection key is the global monotonic
/// [`IssuanceOrdinal`](overdrive_core::id::IssuanceOrdinal), NOT
/// `issued_at`: a fixed/seeded `SimClock` can stamp two issuances for one
/// SPIFFE ID with an equal `issued_at`, and on that tie a `max_by_key`
/// over the timestamp resolves by the audit store's serial-keyed iteration
/// order — surfacing a STALE serial (a CSPRNG draw, no relation to recency)
/// as "current". The ordinal is strictly increasing across issuances, so the
/// newest cert is selected deterministically and recency-correctly even when
/// the clock ties (feature-delta § D1-AMEND-4).
///
/// Persist-inputs discipline: projected from audit-row FACTS at read time
/// (`serial` / `spiffe_id` / `issuer_serial` / `not_after`) — NO cert
/// bytes, NO private key, no cached "current cert". A running alloc with no
/// matching row contributes nothing (the empty `Vec` is omitted from the
/// JSON response via `skip_serializing_if`). Iterates `workload_rows`
/// (deterministic observation-store order); the result `Vec` order follows
/// that deterministic order — no `HashMap` is introduced.
fn issued_certificates_for_rows(
    workload_rows: &[AllocStatusRow],
    issued_cert_rows: &[overdrive_core::ca::issued_certificate_row::IssuedCertificateRow],
) -> Vec<api::IssuedCertSummary> {
    workload_rows
        .iter()
        .filter(|row| matches!(row.state, AllocState::Running))
        .filter_map(|row| {
            let spiffe = SpiffeId::for_allocation(&row.workload_id, &row.alloc_id);
            issued_cert_rows
                .iter()
                .filter(|c| c.spiffe_id == spiffe)
                .max_by_key(|c| c.issuance_ordinal)
                .map(|c| api::IssuedCertSummary {
                    serial: c.serial.clone(),
                    spiffe_id: c.spiffe_id.clone(),
                    issuer_serial: c.issuer_serial.clone(),
                    not_after: c.not_after,
                })
        })
        .collect()
}

/// Derive a [`RestartBudget`] from the durable
/// [`AllocStatusRow.terminal`] field per ADR-0037 §4. The budget is
/// exhausted iff any of the job's rows carries
/// `Some(TerminalCondition::BackoffExhausted { .. })` — the
/// reconciler-emitted terminal claim. Per § Persist inputs, not
/// derived state: `exhausted` is recomputed on every read from the
/// durable inputs (`row.terminal`) so a future per-job budget policy
/// change does not require a row migration.
///
/// `used` reflects the attempts count from the `BackoffExhausted`
/// variant when terminal, else 0; `max` is sourced from
/// [`RESTART_BACKOFF_CEILING`], the `WorkloadLifecycle` policy ceiling —
/// single source of truth shared with the reconciler so the wire
/// field cannot drift from the runtime policy.
fn restart_budget_from_rows(rows: &[AllocStatusRow]) -> RestartBudget {
    let exhausted_attempts = rows.iter().find_map(|row| match &row.terminal {
        Some(TerminalCondition::BackoffExhausted { attempts }) => Some(*attempts),
        _ => None,
    });
    let used = exhausted_attempts.unwrap_or(0);
    let exhausted = exhausted_attempts.is_some();
    RestartBudget { used, max: RESTART_BACKOFF_CEILING, exhausted }
}

/// `GET /v1/nodes` — observation read on `node_health`.
///
/// Symmetric to `alloc_status` — reads through
/// `ObservationStore::node_health_rows`. Fresh store → HTTP 200 with
/// explicit `{"rows": []}`.
#[utoipa::path(
    get,
    path = "/v1/nodes",
    responses(
        (status = 200, description = "Node rows", body = api::NodeList),
        (status = 500, description = "Internal error", body = api::ErrorBody),
    ),
    tag = "observation",
)]
pub async fn node_list(
    State(state): State<AppState>,
) -> Result<Json<api::NodeList>, ControlPlaneError> {
    let rows = state
        .obs
        .node_health_rows()
        .await
        .map_err(|e| ControlPlaneError::internal("node_health_rows", e))?
        .into_iter()
        .map(api::NodeRowBody::from)
        .collect();
    Ok(Json(api::NodeList { rows }))
}
