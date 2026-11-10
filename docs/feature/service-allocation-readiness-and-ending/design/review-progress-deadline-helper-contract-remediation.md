# Independent DESIGN review — D10-H1 progress-deadline helper contract

## Metadata

- Feature: `service-allocation-readiness-and-ending`
- Artifact reviewed: `design/progress-deadline-helper-contract-remediation.md` and the D10-H1 addition to `feature-delta.md`
- Review date: 2026-11-10
- Iteration: 1
- Scope: DELIVER step 04-01 review F-01, the public `service_stream_synth_cap_timeout` signature only
- Verdict: **APPROVED** — bounded exact-interface clarification under the existing D10 approval
- Authority limit: this verdict does not approve step 04-01 implementation, close F-02, re-review the whole DESIGN wave, or authorize step 04-02

## Accepted contract and actual boundary

The user's 2026-11-02 D10 ruling approved every proposed D10 shape choice, including the Service stream's optional `last_attempt` detail (`feature-delta.md:99`). D10 item 4 pins the owned wire field `ServiceSubmitEvent::Failed.last_attempt: Option<LastAttemptWire>` and the exact four-field `LastAttemptWire` representation (`feature-delta.md:765-774`); ADR-0264 records the same optional, labelled detail. The original D10 interface table assigned `service_stream_synth_cap_timeout` the job of taking the latest attempt-ending observation and filling that field, but left the argument type and full public signature unstated. This is the documented specification ambiguity in `deliver/review-04-01.md` F-01. The current helper is `pub` in the public `overdrive_control_plane::streaming` module (`streaming.rs:972-979`, `lib.rs:127`), and a separate acceptance-test crate calls it (`service_submit_event_taxonomy.rs:276-283`). It is a public cross-crate contract, rather than a private implementation detail.

The production entry and owner path is `handlers::submit_workload` → `build_service_stream` (`handlers.rs:636-649`) → that stream's event loop → timeout synthesis. The stream observes only its own workload's `Failed` or `Terminated` `LifecycleEvent`s and stores the latest projection as `Option<LastAttemptWire>` (`streaming.rs:1085-1105`). Its deadline arm converts the duration to `u32`, calls the public helper, then currently mutates `ServiceSubmitEvent::Failed.last_attempt` on the returned event (`streaming.rs:1165-1173`). The timeout wire result is already correct; F-01 concerns the helper's declared responsibility and public shape. The producer does not read a row, choose a durable verdict, retry, or change the restart authority.

## Exact proposed interface and semantic mapping

```rust
pub fn service_stream_synth_cap_timeout(
    after_seconds: u32,
    last_attempt: Option<LastAttemptWire>,
) -> ServiceSubmitEvent;
```

The function is pure and infallible in `overdrive_control_plane::streaming`. For every `after_seconds` and optional owned input it returns exactly `ServiceSubmitEvent::Failed { alloc_id: None, reason: ServiceFailureReason::Timeout { after_seconds }, stderr_tail: None, last_attempt }`. The input is moved into the already approved owned wire field. `build_service_stream` remains the observer and passes its current optional value by value at the deadline; the helper populates the field without a caller mutation. No other caller or helper selects an attempt.

| Designed condition | Typed input and output | Owner/consumer behavior | Assessment |
|---|---|---|---|
| This Service stream observed no attempt-ending event since open | `None` → `Failed { alloc_id: None, reason: Timeout { after_seconds }, stderr_tail: None, last_attempt: None }` | The stream emits its single timeout line; the existing direct helper test supplies `None` after the signature change | Complete; no cause is invented. |
| This Service stream observed one or more attempt-ending events | Its latest `Some(LastAttemptWire { alloc_id, reason, terminal, detail })` → the same `Failed { Timeout }` carrying that exact owned value | The stream emits one timeout line; the CLI displays the value as labelled last-attempt detail, never as the timeout's verdict | Complete; no field or source information is dropped. |
| An authoritative Service terminal arrives before the deadline | Helper is not called | Existing terminal projection wins and carries `last_attempt: None` | Complete; give-up authority is unchanged. |
| The progress deadline fires, but `emit_service_line` fails | The helper still returns an event; the existing stream layer yields `ServiceStreamError::LineEmission { source }` | The stream ends with the approved typed line-emission error | Complete; no new error class is introduced by the helper. |

The helper has no `Result` path or semantic error set. Every input state maps to one declared `ServiceSubmitEvent` representation; no generic error, string cause, `Other` variant, or undeclared event can escape. `ServiceFailureReason::Timeout`, `ServiceSubmitEvent` variants, the serde/OpenAPI shape, CLI rendering, request-timeout envelope, persisted rows and configuration remain unchanged. The ordinary stream boundary still owns any line-emission error. This satisfies the typed-error gate without adding a typed error to an infallible function.

## Necessity, bounded fallout, and approval status

The exact `Option<LastAttemptWire>` by-value argument follows the already approved owned `Failed.last_attempt` field and the in-memory value the stream already constructs. It is sufficient to make the accepted helper responsibility true. A borrowed input would require cloning into the owned wire field; a raw `LifecycleEvent` would transfer the stream's approved filtering/projection responsibility to the helper. Neither is required. The public helper remains public, and no new type, port, field, variant, state, owner, wire format, error taxonomy or behavior is introduced.

The bounded implementation fallout is the signature and body at `streaming.rs:972-979`, the single production timeout call at `streaming.rs:1167`, and the direct cross-crate no-observation call at `service_submit_event_taxonomy.rs:283`. The current implementation **does not yet match** the exact pin: it takes only `after_seconds` and its caller mutates the returned event. `rg` found these two call sites and no others under `crates/`. The composed S-35 stream acceptance scenario already exercises a present last attempt. No new test lane or seam is needed. The `AppState::streaming_cap` comment was corrected separately by commit `63741cf9d`; this review does not alter or review that remediation's DELIVER verdict.

D10-H1 is a clarification of a previously named helper's missing parameter shape. The approved D10 contract already determines the helper's owner responsibility, the optional semantic value and its owned representation. Choosing that same owned value as the helper argument adds no behavior-determining decision or broader public surface beyond making the previously approved helper contract executable. **The existing 2026-11-02 user approval suffices for this bounded clarification** after this independent technical review; a new user decision is not needed for D10-H1. If implementation later changes the owner, wire meaning, error behavior or a forbidden state, that would be a new material decision requiring its own approval. None is present here.

## Verification and verdict

- Read-only comparison of the proposed D10-H1 text and normative `feature-delta.md` diff against D10 items 3–4, the original interface table, ADR-0262, ADR-0264, step 04-01 review F-01, and the actual helper, stream and test callers.
- `rg -n 'service_stream_synth_cap_timeout\(' crates --glob '*.rs'` — one production call and one cross-crate acceptance-test call, plus the definition.
- Normative diff check: D10-H1 adds the exact helper signature and provenance note and corrects historical “unpinned/private” status prose. It removes or relaxes no approved `zero`, `must`, `never`, `exactly`, or required outcome. No source, test, DES event, roadmap, or commit was changed by this reviewer.
- No cargo test or native run: the finding is an exact public-contract omission; the stream's output was already reviewed as correct, and this review does not claim a new runtime failure.

**APPROVED for D10-H1 only.** The original step 04-01 crafter may implement the exact pinned helper contract and bounded call-site fallout, then return to the original DELIVER reviewer for iteration 2. That reviewer retains ownership of F-02 disposition and of the full step verdict. The active roadmap's strict step/review sequence remains in force.
