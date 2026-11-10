# D10-H1 — progress-deadline timeout helper contract

Status: **exact DESIGN clarification approved by independent review** (2026-11-10). This is the bounded response to DELIVER step 04-01 review F-01 in `deliver/review-04-01.md`. It changes no wire format, persisted row, configuration or accepted timeout outcome. Step 04-01 remains in review until its original crafter remediates the implementation and the DELIVER reviewer re-reviews it.

## Gap and owner path

The accepted D10 interface table said that the public `service_stream_synth_cap_timeout` takes the latest attempt-ending observation and fills `last_attempt`, but did not pin the parameter type. DISTILL's `upstream-issues.md` explicitly recorded that omission. The current `pub fn service_stream_synth_cap_timeout(after_seconds: u32) -> ServiceSubmitEvent` in the public `overdrive_control_plane::streaming` module (`streaming.rs:972-979`; `lib.rs:127`) always produces `last_attempt: None`. The production `build_service_stream` currently compensates by mutating the returned event after the call (`streaming.rs:1085,1092-1105,1165-1173`). Its output is correct; the review finding concerns the named public helper's contract. A cross-crate acceptance test calls the helper directly (`control-plane/tests/acceptance/service_submit_event_taxonomy.rs:276-283`), confirming it is a real public boundary rather than private structure.

The production path is `handlers::submit_workload` → `build_service_stream` with the derived deadline (`handlers.rs:636-649`) → observe this workload's `Failed`/`Terminated` lifecycle events as `Option<LastAttemptWire>` → synthesize `Timeout` when the injected clock fires. D10 item 4 and ADR-0264 already approve the exact owned `ServiceSubmitEvent::Failed.last_attempt: Option<LastAttemptWire>` wire field and the `LastAttemptWire { alloc_id, reason, terminal, detail }` payload. `None` means this stream saw no attempt-ending event. An observed attempt is labelled detail, never a verdict or a stored row.

## Exact interface and mapping

The authoritative feature delta § D10 interface changes now pins:

```rust
pub fn service_stream_synth_cap_timeout(
    after_seconds: u32,
    last_attempt: Option<LastAttemptWire>,
) -> ServiceSubmitEvent;
```

The public helper is a pure, infallible function in `overdrive_control_plane::streaming`. It consumes the optional owned value and returns exactly `ServiceSubmitEvent::Failed { alloc_id: None, reason: ServiceFailureReason::Timeout { after_seconds }, stderr_tail: None, last_attempt }`. `after_seconds` retains its existing `u32` meaning; `last_attempt` is the same owned type as the already approved wire field. It neither selects an attempt nor reads a store: `build_service_stream` remains the owner of observation and passes its current `Option<LastAttemptWire>` by value when the deadline fires. The helper is the owner of populating the returned field, as the existing D10 table intended.

| Designed condition | Exact helper input and output | Caller behavior |
|---|---|---|
| No attempt-ending lifecycle event observed since this stream opened | `last_attempt: None` → `Failed { Timeout { after_seconds }, last_attempt: None, alloc_id: None, stderr_tail: None }` | Existing one-argument no-observation test supplies `None`. The stream writes no row. |
| At least one attempt-ending event observed | Latest observed event projected by `build_service_stream` to `Some(LastAttemptWire)` → same `Failed { Timeout }` with that exact value | The stream passes its owned value, makes no post-return mutation, and emits one timeout line. The value remains labelled detail, not a reason or verdict. |
| An authoritative terminal arrives before the deadline | This helper is not called | Existing terminal projection carries `last_attempt: None`; the give-up remains authoritative. |

There is no new error type or semantic failure mapping: this synthesis is infallible, and existing stream line-emission failure remains `ServiceStreamError::LineEmission`. `ServiceFailureReason`, `ServiceSubmitEvent` variants, serde/OpenAPI shape, CLI rendering and request timeout are unchanged.

## Bounded fallout and alternatives

The original step 04-01 crafter changes only the helper signature/body and the timeout arm's call site in `streaming.rs`, plus compiler-required calls such as the existing `service_submit_event_taxonomy.rs` test (which passes `None`). `rg` finds one production call and one cross-crate acceptance-test call. The existing S-35 composed timeout scenario already exercises a present last attempt through the production stream; no new test lane or production seam is proposed. Review F-02's `AppState::streaming_cap` source-comment correction remains separate and owned by that crafter.

`Option<&LastAttemptWire>` would force a clone into the approved owned wire field. Passing a raw `LifecycleEvent` would repeat the stream's approved filtering/projection at a helper boundary and add no owner benefit. Keeping the one-argument helper while mutating its output in the caller leaves the accepted table's helper responsibility false. Making the helper private would remove a currently public, cross-crate-used API and does not resolve the accepted named-helper row. The owned `Option<LastAttemptWire>` is the smallest exact shape that directly moves the already approved in-memory value into the already approved wire field.

## Ruling and review requirement

D10 was approved in full by the user on 2026-11-02 (feature delta § Approval record); ADR-0264 accepted the optional last-attempt detail. D10-H1 pins the previously missing **signature of the already named public helper** using that same approved owned representation and helper responsibility. It adds no behavior-determining choice, public type, wire field, error class, state or owner. On that basis it is a DESIGN contract clarification under the existing user ruling, not a request for a redundant user approval. The independent DESIGN review in `review-progress-deadline-helper-contract-remediation.md` approved the derivation and exact signature before the original step 04-01 crafter resumed. If later evidence finds a material difference from approved D10 semantics, record that difference as a new proposed decision for the user rather than silently broadening this clarification.
