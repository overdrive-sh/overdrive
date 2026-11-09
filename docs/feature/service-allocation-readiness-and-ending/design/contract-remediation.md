# Approved contract amendments — service allocation readiness and ending

Status: **all three exact contracts user-approved 2026-11-09** (user: “Approve all three exact contracts”); bounded independent technical review iteration 2 **APPROVED** ([review](review-contract-remediation.md)). This is the bounded response to the independent DELIVER roadmap review's F-01, F-02, F-03 and F-06. The exact contracts are incorporated into the authoritative feature delta. This artifact records the bounded review's evidence and historical handoff. The later independent **full D1–D17 DESIGN review APPROVED at iteration 4** ([review](review.md)); independent DISTILL review and roadmap approval remain pending. The bounded verdict alone did not authorize DELIVER.

## Contract audit and production evidence

| Finding | Disposition | Evidence |
|---|---|---|
| F-01, D11 stream read error | **Confirmed specification ambiguity.** D11 requires `Err` at both reads but pins neither the semantic stream-error variants nor the mapping from the observation port. | `../feature-delta.md` D11 items 5–7 and its “no new public surface” clause; ADR-0263 Decision; `crates/overdrive-control-plane/src/streaming.rs:1032-1044,1050-1113` returns `Result<Bytes, std::io::Error>` and currently does not read; `crates/overdrive-core/src/traits/observation_store.rs:53-68,2035` defines the actual typed read error. |
| F-02, D14 TCP view signature | **Confirmed specification ambiguity.** PR-4 pins the accessor's name and result meaning, but the accepted design omits receiver, return ownership and type. | `../feature-delta.md` D14 item 2 and Interface changes (D13 to D16); `crates/overdrive-core/src/traits/driver.rs:290-364` has a RED scaffold with one particular signature, which is test support rather than DESIGN approval. The consumers are `crates/overdrive-control-plane/src/action_shim/mod.rs:1522-1533` and `crates/overdrive-worker/src/mtls_intercept_worker.rs:2723,2747-2755`. |
| F-03, D14 input rejection | **Confirmed specification ambiguity for Service inputs.** The accepted “sole Service startup descriptor, never TOML” rule omits typed errors and ingress mappings. The raw Job/Schedule probe-field case is already structurally outside the typed per-kind payload and is narrowed below. | `../feature-delta.md` D14 item 1; `crates/overdrive-core/src/aggregate/mod.rs:412-460,605-620`; `crates/overdrive-core/src/aggregate/workload_spec.rs:773-839,1222-1296`; `crates/overdrive-core/src/api/submit.rs:39-135`; `crates/overdrive-control-plane/src/handlers.rs:294-318`. |
| F-06, full DESIGN and DISTILL approval | **Confirmed evidence gap and corrected stale status sentence.** User rulings and the bounded two-iteration technical review establish the three exact amendments; they do not prove independent full-wave approval. | `../feature-delta.md` Approval record and corrected DISTILL Reconciliation; `design/wave-decisions.md` marks the full review pending; ADR-0268 through ADR-0272 remain not yet Accepted; `../feature-delta.md` DISTILL Gate results records only Sentinel round 1 `needs_revision`; `../distill/upstream-issues.md:5` says independent DISTILL review pending. The bounded `design/review-contract-remediation.md` exists, but no full D1–D17 DESIGN or approved DISTILL review artifact was found in this feature directory or its Git history. |

This is an interface-contract audit of three omissions in accepted prose, not a claim of a new production defect. D11's original F-5 has the production reproduction and reachability trace in the feature delta. The current control-plane entry path is `overdrive deploy` → `ApiClient::submit_workload_streaming` → `submit_workload_streaming` handler → `Body::from_stream(build_service_stream(...))` (`crates/overdrive-control-plane/src/handlers.rs:605-659`). The currently implemented `build_service_stream` has no observation read; **D11's accepted replacement** adds `ObservationStore::alloc_status_rows` once at stream open and on each `Lagged` relist. `LocalObservationStore::alloc_status_rows` maps redb and task-join failures to `ObservationStoreError::Io` (`crates/overdrive-store-local/src/observation_backend.rs:938-960`); the sim port can inject a typed fault (`crates/overdrive-sim/src/adapters/observation_store.rs:870-883`). The existing S-42 bodies inject `Unreachable` at open and relist (`crates/overdrive-control-plane/tests/acceptance/service_stream_recorded_verdict.rs:392-438`). Under D11, `build_service_stream` owns that read, yields one error and returns on failure without retry, and relists on `Lagged` before another wait. The `reqwest` CLI consumes HTTP body chunks and maps a body error to `CliError::Transport` (`crates/overdrive-cli/src/commands/deploy.rs:690-706`); a Rust error type does not cross the HTTP wire. No new client protocol is proposed.

## Decision 1 — D11-E1: typed Service stream error at the Rust stream boundary

**Exact public contract approved by the user on 2026-11-09:**

```rust
pub enum ServiceStreamError {
    ObservationRead { source: ObservationStoreError },
    LineEmission { source: std::io::Error },
}

pub fn build_service_stream(
    state: AppState,
    workload_id: WorkloadId,
    accepted: ServiceSubmitEvent,
    progress_deadline: Duration,
) -> impl Stream<Item = Result<Bytes, ServiceStreamError>> + Send + 'static;
```

`ServiceStreamError` lives in `overdrive_control_plane::streaming`, implements `std::error::Error + Send + Sync + 'static`, and exposes its typed `source` through `Error::source`. It is an in-process server boundary only. The existing `ServiceSubmitEvent`, NDJSON bytes, response status, handler route, and CLI wire parser stay unchanged. This is the **one narrow exception** to D11's original “no new public surface” statement, now recorded in the authoritative feature delta without relaxing D11's error outcome.

| Designed failure at the stream owner | Exact mapping | Stream action |
|---|---|---|
| Open read: `alloc_status_rows() -> Err(ObservationStoreError::Unreachable { peer })` | `ServiceStreamError::ObservationRead { source: ObservationStoreError::Unreachable { peer } }` | Yield `Err` once, end; never arm the progress deadline. |
| Open read: `ObservationStoreError::Io(error)` | `ObservationRead { source: ObservationStoreError::Io(error) }` | Same; retain `io::ErrorKind` and source. |
| Open read: `ObservationStoreError::Envelope { source }` | `ObservationRead { source: ObservationStoreError::Envelope { source } }` | Same; retain the envelope cause. The local adapter currently logs and skips corrupt rows under ADR-0048, but the port's declared error set includes this variant. |
| `Lagged` relist: any of those three `ObservationStoreError` variants | The **same** `ObservationRead` mapping, with the original variant and payload intact | Yield `Err` once and end; do not resume the receive loop or report a synthesized timeout. |
| Any failure from existing `emit_service_line` while emitting Accepted, a live/recorded terminal, or a synthesized terminal | `ServiceStreamError::LineEmission { source: original std::io::Error }` | Yield `Err` once and end, preserving existing line-emission behavior. |

There is no generic `Other` variant, string-only classification, error-to-absence coercion, retry, or synthetic `ServiceSubmitEvent::Failed` for a read failure. A diagnostic string may identify open versus relist, but the source chain above remains the semantic cause. `ObservationRead` is one semantic failure at the same port; open and relist are two call sites, not separate error meanings. `LineEmission` preserves the independent pre-existing line failure without misclassifying it as an observation read.

**Bounded fallout:** `streaming.rs`'s public stream item and local error yields; the handler's `Body::from_stream` still consumes an `Error + Send + Sync` item; test and Quint-driver `Lines` aliases and S-42 assertions become typed. DISTILL must exercise the three declared observation causes at the relevant stream boundary (open and relist cases already exist for `Unreachable`), assert the original variant/payload and termination, and ensure no undeclared stream-error variant escapes. It should use existing sim fault injection and test lanes. The Job stream and `ObservationStore` trait do not change.

**Material alternatives:** retaining `std::io::Error` and encoding source meaning only in `ErrorKind` or a message leaves the accepted typed-error gate unsatisfied; adding an HTTP wire failure variant would change D11's unchanged wire and CLI contract; separate open/relist variants would multiply public surface without a different caller decision. The approved two-variant error preserves the source type and the accepted wire boundary.

## Decision 2 — D14-A1: owned TCP view of declared listeners

**Exact public contract approved by the user on 2026-11-09:**

```rust
impl AllocationSpec {
    pub fn tcp_service_ports(&self) -> Vec<NonZeroU16>;
}
```

`NonZeroU16` is `std::num::NonZeroU16`. The method is a pure read of `self.declared_listeners`: include each `Listener.port` whose `protocol == Proto::Tcp`, in declaration order, with no deduplication, sorting, UDP inclusion or mutation. A new owned vector is returned on each call, so the view cannot outlive or borrow `AllocationSpec`. This pins the already accepted PR-4 model rather than changing it. It matches the current RED scaffold's candidate signature, but the scaffold is not approval evidence.

**Bounded fallout:** the action shim's `declared_tcp_listen_ports` converts this owned `Vec<NonZeroU16>` to `Vec<ListenPort>`; the worker's `start_shared_allocation` derives its `BTreeSet` capability and iterates the returned vector for inbound installs. The existing `AllocationSpec.service_ports` and `WorkloadLifecycleState.service_ports` replacements by `declared_listeners` remain exactly PR-4. No `Driver` trait method, new field, persisted format, or wire change is added. The S-55 property and native inbound-rule test remain the verification surfaces.

**Material alternatives:** a borrowed iterator would make the cross-crate return type/lifetime more complex at both current callers; a borrowed slice cannot represent a TCP-only projection over an interleaved TCP/UDP declaration; retaining both stored fields reintroduces the drift PR-4 explicitly rejected. The owned vector is the smallest signature matching the accepted semantics and actual consumers.

## Decision 3 — D14-E2: typed rejection of an invalid GuestListeners declaration

**Exact public contract approved by the user on 2026-11-09:** D14 item 1 already says `ProbeMechanic::GuestListeners` is valid only as the sole startup descriptor of a Service and cannot be written in operator TOML. The amendment pins the error representation for inputs that violate that rule. It adds only the following variants to existing public error enums; it does not add a provenance flag, a new accepted role, or validation of unrelated probe fields.

```rust
// overdrive_core::aggregate::AggregateError
GuestListenersInvalidRole {
    vector_role: ProbeRole,
    descriptor_role: ProbeRole,
    index: usize,
},
GuestListenersNotSoleStartup { count: usize },

// overdrive_core::aggregate::workload_spec::ParseError
GuestListenersNotOperatorConfigurable {
    role: ProbeRole,
    probe_idx: usize,
},
```

`ProbeRole` is the existing `overdrive_core::observation::ProbeRole` with `Startup`, `Readiness` and `Liveness`. `index` and `probe_idx` are zero-based positions in the named role vector. `vector_role` is the containing `ServiceSpecInput` vector; `descriptor_role` is the submitted `ProbeDescriptor.role` field. The two can disagree on raw JSON, so checking only one would admit a `GuestListeners` descriptor whose effective role is ambiguous. The variants are semantic causes. Their `Display` strings name the role and index for diagnosis; callers must branch on variants and payloads, not text.

| Input at the production ingress | Exact typed mapping and precedence | Caller-visible result |
|---|---|---|
| Raw `SubmitSpecInput::Service` has `GuestListeners` in `readiness_probes` or `liveness_probes`, regardless of its `ProbeDescriptor.role`; or has it in `startup_probes` but its descriptor role is not `Startup` | `Service::from_submit` returns `AggregateError::GuestListenersInvalidRole { vector_role, descriptor_role, index }`, with the values of the first invalid `GuestListeners` in vector order Startup, Readiness, Liveness and then index order. This check precedes the sole-startup and general probe-count checks; existing scalar, driver, listener and mechanic-value validation retains its current precedence. | The handler retains `ControlPlaneError::Aggregate(the exact AggregateError)`; `to_response` returns HTTP 400 `ErrorBody { error: "validation", message: diagnostic, field: Some(...) }` per accepted ADR-0015. `field` is `"<vector_role>_probes[<index>].mechanic"` for a non-startup vector and `"startup_probes[<index>].role"` for a startup vector with a mismatched descriptor role. |
| Raw `SubmitSpecInput::Service` has at least one `GuestListeners` in `startup_probes`, every such descriptor has role `Startup`, but the startup vector's length is not exactly one | `Service::from_submit` returns `AggregateError::GuestListenersNotSoleStartup { count: startup_probes.len() }` before `ProbeCountExceeded`; `count` includes every startup descriptor. | The same typed `ControlPlaneError::Aggregate` chain and HTTP 400 `ErrorBody` with `error: "validation"`, `field: Some("startup_probes")`, and diagnostic `message`. |
| Raw `SubmitSpecInput::Service` has exactly one startup descriptor, its mechanic is `GuestListeners` and its role is `Startup` | Neither new error applies. Existing validation of other fields and the dial-bearing probe cap remains in force. | The existing Service admission path accepts the descriptor when its other requirements hold. The accepted D14 rule does not distinguish a raw JSON client from the platform's own projected descriptor, so this amendment adds no origin/provenance test. |
| Operator TOML Service has `type = "guest_listeners"` (case-insensitive) in any of `[[health_check.startup]]`, `.readiness` or `.liveness` | `WorkloadSpecInput::from_toml_str` returns `ParseError::GuestListenersNotOperatorConfigurable { role, probe_idx }` for the first such entry in parser role order Startup, Readiness, Liveness, then index. The shared mechanic parser recognizes that one platform-only token before its `UnknownProbeType` arm; every other unknown type continues to use `UnknownProbeType`. | No HTTP request or intent write. Both CLI deploy modes preserve the typed error as `CliError::ParseError(source)`; the existing `CliError::ParseError` exit code applies. The non-streaming `deploy` fallback and streaming `deploy_streaming` currently stringify other parse failures, so adding this one explicit pass-through arm to each is bounded necessary caller fallout. |
| Operator TOML Job or Schedule has any `[[health_check.*]]` block, including a `guest_listeners` spelling | The existing earlier kind gate returns `ParseError::ProbesNotAllowedOnKind { kind: "job" or "schedule", guidance: existing per-kind guidance }` before parsing a mechanic (`workload_spec.rs:827-839`). | Existing typed parser error. No new GuestListeners-specific variant or scenario is needed for this already covered category. |

The canonical TOML type token corresponding to the Rust `GuestListeners` variant is `guest_listeners`, following the variant's existing `snake_case` wire naming. The parser's established case-insensitive type matching makes `GUEST_LISTENERS` the same forbidden token. A different spelling such as `GuestListeners` (without the underscore) remains `ParseError::UnknownProbeType { probe_idx, found: "guestlisteners".to_owned() }` under the current lowercase parser; it is rejected, but it does not denote the canonical mechanic token. This distinction prevents a new alias from being granted by accident.

For raw **Job or Schedule JSON**, the typed `SubmitSpecInput::Job(JobSpecInput)` and `::Schedule(ScheduleSpecInput)` payloads have no probe fields, and `deny_unknown_fields` is declared on the wire shapes (`api/submit.rs:39-65,115-135`, `aggregate/mod.rs:795-830`). A Rust caller cannot construct a Job/Schedule payload containing a `ProbeDescriptor`; a raw JSON document with `startup_probes`, `readiness_probes` or `liveness_probes` is rejected by JSON deserialization before `Service::from_submit`. The submit handler's `Json(request)` extractor is before its body and therefore cannot map that malformed document into `AggregateError`. F-03 must **not** demand a new `Service::from_submit` branch, typed `GuestListeners` error, or test-only wire seam for the non-Service raw case. Its existing structural wire rejection and the existing Job/Schedule TOML `ProbesNotAllowedOnKind` coverage are the relevant boundaries. No new HTTP error contract for a malformed non-Service JSON body is introduced here.

The `ControlPlaneError::Aggregate` pass-through preserves both new Rust errors; the existing `AggregateError::Validation { field, message }` flattening in `submit_workload` is **not** used for them. `to_response` needs only bounded match arms to derive the exact `ErrorBody.field` values above. The HTTP `error` code remains `"validation"` and the status remains 400, as ADR-0015 requires for aggregate validation; the wire body gains no field or semantic error code. `ProbeMechanic::validate()` continues to accept the fieldless value itself, because placement and provenance are constructor/parser ingress rules, not mechanic-value rules. The existing `ProbeCountExceeded` and dial-bearing cap behavior is unchanged.

**Material alternatives:** placing “wrong role” or “not sole” in `AggregateError::Validation.message` would hide the semantic branch inside prose; treating the known `guest_listeners` TOML token as `UnknownProbeType` would misname a platform-only mechanic as unknown; adding an HTTP error code or a new `ControlPlaneError` variant would depart from ADR-0015's validation mapping without helping the Rust owner distinguish causes; adding a provenance token to the raw API would be a new policy absent from D14. The approved variants and mappings enforce exactly the accepted restriction at the two ingresses that can represent it.

## F-06 approval reconciliation

The feature delta's Approval record contains user rulings for D1–D17, the DG amendments and the 2026-11-09 exact D11-E1/D14-A1/D14-E2 amendments. The bounded two-iteration contract review is recorded in `design/review-contract-remediation.md` and approved those three exact shapes. It is **not** an independent full D1–D17 DESIGN verdict. No artifact or verdict for the formerly claimed round 7 has been found. The `DISTILL / Reconciliation` sentence that said “DESIGN was approved at review round 7” was corrected as a factual status note, without changing any user ruling. The D13–D16 ADRs remain user-approved in their stated scopes and **not yet Accepted**. D1–D12, D17 and their listed accepted subdecisions retain their recorded user approval. V41 and V42 remain held; V39 remains partial.

If an actual independent round-7 review artifact is recovered, record its exact path, reviewed revision, verdict and any post-review changes, then assess whether it covers the current design. Without that evidence, dispatch a **fresh independent DESIGN review of the complete current D1–D17 contract**, with adversarial and subtractive passes, the three approved amendments, ADR status/provenance consistency, and held model/spike evidence. Record the full review on disk. Do not infer full-wave approval from the stale sentence, a green model check, the bounded contract review, or user approval of a decision. Once that review approves, update affected ADR status and the DESIGN index with a link to its verdict; only then claim the full DESIGN gate is satisfied.

DISTILL likewise has only a recorded Sentinel round-1 `needs_revision` and says its independent review is pending. Its later test fixes and DG rulings are work performed, not an approval. The independent DISTILL reviewer must review the revised suite and record a full on-disk verdict before the roadmap can be approved.

## Separate downstream handoff

1. **DISTILL, F-01:** update S-42 and the `stream_snapshot` conformance driver to assert the approved typed `ObservationRead` source and its `Unreachable`/`Io`/`Envelope` mappings where the port permits injection, plus the existing end-after-error and no-timeout outcomes. Keep the Job stream out of scope (GH #340). Any `LineEmission` check must be grounded in a reachable emitter failure; do not add a production seam to manufacture one.
2. **DISTILL, F-03 and F-04:** author Service raw-API cases for the approved `GuestListenersInvalidRole` (readiness, liveness, and a startup-vector descriptor whose own `role` is not Startup) and `GuestListenersNotSoleStartup` (including its precedence against `ProbeCountExceeded`), plus Service TOML cases for `GuestListenersNotOperatorConfigurable` in each role. Verify exact variants/payloads at the aggregate/parser ports and the pinned HTTP 400 `ErrorBody`/CLI mappings through real ingress, without matching diagnostic message text. Retain existing Job/Schedule TOML kind-rejection coverage. Do not invent a typed non-Service `GuestListeners` payload or redundant raw non-Service acceptance case: that payload is structurally unavailable. Add the per-test Contract Shape declarations to the transitioned S-55, S-34 and S-46 bodies, including exact `/// CONTRACT_SHAPE: pure-function.` on live source-local pure properties. Update RED classification and the 05-07 activation list. Then obtain an independent DISTILL approval artifact.
3. **DELIVER roadmap, F-05:** `deliver/roadmap.json` now has **one final wave-level mutation gate after all 17 step reviews**, with per-step execution and kill-rate criteria removed. Its `final_wave_gate.target_sources` preserves all 17 original production-target notes and all 12 removed step criteria, the >=80% threshold, and the D17 signal-position and stored-permit mutant evidence supporting the S-48(b) waiver. This is gate placement consolidation only, with no new mechanism or test lane. `validation.status` remains `pending` until the independent roadmap re-review approves.
4. **DELIVER execution:** the historical `deliver/execution-log.json` has 01-01 RED/GREEN but no COMMIT and remains untouched. Resume only through the repository's fresh-agent, strict step/review sequence after the DESIGN, DISTILL and roadmap gates are satisfied. No earlier step is declared complete by this amendment.

## Review boundary and acceptance

The user approved **Decision 1 (D11-E1)**, **Decision 2 (D14-A1)** and **Decision 3 (D14-E2)** as the three exact contracts on 2026-11-09. The authoritative DESIGN contract now contains their signatures, variants, payloads, mappings and D11's narrow exception to its original “no new public surface” clause. ADR-0263, ADR-0269, the DESIGN index and the architecture brief link those amendments. The bounded review approved their technical shapes at iteration 2. The independent full D1–D17 DESIGN review described above, independent DISTILL approval and roadmap re-review remain required before DELIVER resumes. This artifact is not itself an executable roadmap.
