# DESIGN review — ADR-0093: streaming Service Accepted renderer

## Metadata

| Field | Value |
| --- | --- |
| Feature | `service-kind-vm-workloads` (GH #257) |
| Artifact | `docs/product/architecture/adr-0093-streaming-service-accepted-render.md` |
| Review type | Independent focused DESIGN amendment review |
| Iteration | 1 |
| Reviewer | Fresh isolated solution-architecture reviewer |
| Verdict | **APPROVED** |

## Scope reviewed

ADR-0093 is a deliberately narrow correction to the existing streaming
Service deploy presentation. The review checked it against ADR-0090, ADR-0091,
ADR-0092, the architecture brief, the feature delta and DESIGN decisions, the
rejected DELIVER 02-02 review, the E08 expectation, and the current CLI
production path.

The authorized scope is exact: for an existing Service stream that has
`Accepted` followed by `Stable`, compose the existing Accepted renderer once
before the existing Stable renderer once in the existing
`DeployStreamingOutput.summary`. No other terminal path or deployment lane is
changed.

## Evidence and assessment

| Review question | Evidence | Assessment |
| --- | --- | --- |
| Is the premise reachable through the real product path? | `main.rs` selects `deploy_streaming` only for un-detached TTY stdout; `deploy_streaming_service` calls `consume_stream`; `consume_stream` retains `ServiceSubmitEvent::Accepted` at `deploy.rs:730-737`, then returns only `format_service_stable_summary` in its `Stable` arm at `:738-759`. | Proven. The missing Accepted rendering is in the production CLI consumer, not an E08 harness artifact. |
| Is the correction necessary and sufficient? | The rejected E08 capture used detached acknowledgement plus a later idempotent PTY deployment (`deliver/review-02-02.md`, R02-02-1). That cannot prove the expectation's one-command stdout contract. The current consumer already has all acknowledgement fields; the existing `workload_submit_accepted` renderer and stable renderer supply the required text. | Proven and bounded. Private summary assembly is the smallest correction that makes the one-command oracle possible. |
| Does it preserve public shape? | ADR-0093 reuses `DeployOutput`, `DeployStreamingOutput.summary`, `workload_submit_accepted`, `format_service_stable_summary`, the existing Service event stream, and the existing binary print. It forbids a new method, type, field, variant, command, argument, route, or protocol. | Approved. No public API, wire, command, persistence, or dependency surface is introduced. |
| Are unaffected lanes and failures protected? | The ADR explicitly confines composition to `Accepted -> Stable`; it preserves detached/non-TTY JSON acknowledgement, pre-Accepted errors, `Failed`, `Stopped`, stream-close/body-decode errors, cancellation, cleanup, and exit behavior. This matches the separate `should_stream(!detach && is_terminal)` branch in `main.rs:63-133` and the current terminal arms in `deploy.rs:761-834`. | Approved. The amendment neither makes Accepted a separate flush nor alters failure/cancellation semantics. |
| Is lifecycle/probe scope preserved? | ADR-0093 states Lifecycle Gate Ownership is not applicable. ADR-0090/0091 retain ownership of Running, Stable, readiness, liveness termination, and restart/finalization; the proposed change is after existing wire events at the CLI renderer. | Approved. No state owner, lifecycle gate, probe behavior, or VM behavior changes. |
| Are the evidence obligations honest? | ADR-0093 requires (1) a direct ordered Service-stream regression that asserts exact Accepted-before-Stable order and multiplicity in one returned summary and (2) a fresh E08 single un-detached PTY deployment, with detached submission, idempotent resubmission, and concatenation removed. E08 already defines the same one-command public boundary and remains black-box. | Approved. The direct regression is the permanent guard; the fresh built-product capture is independent operator evidence. Neither substitutes shell synthesis for product output. |

## Findings

No blocking findings.

- **praise:** ADR-0093 traces the defect from the E08 transcript through the
  actual TTY dispatch and Service stream consumer, then restricts the remedy to
  the already-existing data and renderers. Its error/cancellation and
  detached/non-TTY exclusions prevent the original evidence correction from
  growing into a streaming-protocol redesign.

## Dispositions

| Item | Disposition |
| --- | --- |
| R02-02-1, fabricated two-command Accepted → Stable ordering | Addressed by accepted design direction: render the retained Accepted event in the same successful streaming summary and require a one-command PTY capture. Implementation remains for the original 02-02 crafter. |
| New public CLI/wire/protocol surface | Rejected by ADR-0093 and verified unnecessary from the current consumer path. |
| Detached/non-TTY, error, cancellation, exit, lifecycle, probe, and VM changes | Explicitly out of scope and must remain unchanged in DELIVER review. |
| Regression protection | Required: direct ordered-stream renderer regression, carrying the repository-required per-test Contract Shape declaration, plus a newly captured E08 black-box transcript. |

## Verification performed

- Read the mandatory repository guidance, relevant reviewer criteria, ADR-0090
  through ADR-0093, the feature DESIGN artifacts, E08 expectation, and the
  rejected 02-02 DELIVER review.
- Traced the current production route: `Command::Deploy` TTY selection in
  `crates/overdrive-cli/src/main.rs` → `deploy_streaming_service` →
  `consume_stream` → one `DeployStreamingOutput.summary` printed by the
  binary.
- Confirmed the reviewed amendment and related DESIGN documents are
  documentation-only, and `git diff --check` reports no whitespace error for
  those reviewed documents.

No implementation or execution-log files were modified by this review. No
test command was run because ADR-0093 is proposed DESIGN, not an implemented
change; the named direct regression and fresh native-metal E08 capture are
DELIVER obligations.

## Verdict

**APPROVED.** ADR-0093 is a necessary, production-path-proven, minimal
presentation amendment. It exactly authorizes one existing Accepted block
before one existing Stable detail in the same successful un-detached Service
stream summary, while preserving all stated boundaries. The original DELIVER
step `02-02` crafter may implement only this design and then obtain the
required direct regression and fresh single-command E08 evidence for its
implementation re-review.
