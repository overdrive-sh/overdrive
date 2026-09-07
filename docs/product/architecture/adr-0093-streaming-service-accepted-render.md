# ADR-0093: Render the existing Accepted acknowledgement in a successful streaming Service deploy

## Status

**Proposed** (2026-09-06). The user authorized this bounded DESIGN amendment
after the E08 reviewer proved that the current one-command operator oracle is
impossible: the Service streaming consumer receives `Accepted`, retains it as
data, and renders only the later `Stable` detail. Independent DESIGN review is
required before the original DELIVER step `02-02` crafter resumes.

This amends only the CLI presentation contract of the existing streaming
Service deploy lane. It does not amend ADR-0090/0091/0092's probe, ingress,
adapter, dependency, or lifecycle decisions.

## Context

E08's accepted public boundary is one fresh, un-detached, PTY-backed command:

```text
overdrive deploy examples/service-kind-vm-workloads/service.toml
```

Its single stdout transcript must contain one `Accepted` acknowledgement before
one `Stable` terminal detail. The control-plane NDJSON stream already emits
that order. In `crates/overdrive-cli/src/commands/deploy.rs`, however,
`consume_stream` consumes `ServiceSubmitEvent::Accepted` into its existing
accepted fields and returns a `DeployStreamingOutput` only when a terminal
event arrives. The `Stable` arm builds its summary solely with the existing
stable renderer. The binary wrapper prints that summary, so the operator never
sees the already-consumed acknowledgement.

The rejected E08 implementation combined a detached acknowledgement from one
deploy invocation with a Stable result from a second invocation. That shell
composition cannot establish the one-command product contract and must not be
used as a substitute for product rendering.

## Decision

For a streaming Service deployment that receives the existing `Accepted` event
and then the existing `Stable` terminal event, the existing Service stream
consumer returns its existing `DeployStreamingOutput` with `summary` composed
in this exact order:

1. the exact existing `workload_submit_accepted` acknowledgement rendered from
   the accepted event's existing fields (`Workload ID`, intent key, digest,
   idempotency outcome, endpoint, and next command); then
2. the exact existing `format_service_stable_summary` terminal detail rendered
   from the same operation's `Stable` event.

The binary continues to print that one existing `summary` value. Thus an
un-detached TTY Service success prints exactly one `Accepted.` block followed
by exactly one `Service '<id>' is stable ...` line, in that order, and exits
zero. The acknowledgement and terminal detail refer to the same stream and
the same submitted Service; neither is obtained by a second command.

The implementation reuses the existing `DeployOutput` acknowledgement renderer
and the existing `DeployStreamingOutput.summary` field. It may make only the
bounded private assembly change needed in the existing Service stream consumer
to retain/render the accepted fields before concatenating the existing Stable
detail. It introduces no public method, type, enum variant, field, command,
argument, route, protocol event, daemon, persistence row, lifecycle state, or
probe behavior.

### Error and cancellation boundary

This amendment is success-only. Pre-Accepted parse, transport, HTTP-status,
or body-decode failures keep their current stderr and exit-code behavior.
`Failed` and `Stopped` terminal summaries, stream-close/body-decode failures,
and process cancellation retain their current rendering, exit semantics, and
cleanup behavior; this change does not turn an Accepted event into a separate
flush, background task, cancellation protocol, or partial-output guarantee.
The existing validation remains authoritative: a Service terminal event before
Accepted is still the existing body-decode error.

### Lifecycle Gate Ownership

**Not applicable:** this is a renderer-only ordering correction after existing
wire events. It adds, removes, and moves no allocation, Service, readiness, or
liveness gate. `Running`, `Stable`, backend eligibility, liveness termination,
and `WorkloadLifecycle` restart/finalization retain their existing owners.

## Alternatives considered

1. **Keep the E08 two-command transcript composition — rejected.** It combines
   different product operations and can pass while the streaming command never
   renders Accepted.
2. **Print Accepted from the shell or PTY recorder — rejected.** The shell is
   evidence infrastructure, not the operator-product renderer; it would hide
   the production defect and duplicate presentation behavior outside the CLI.
3. **Add a streaming callback, event variant, command flag, or second output
   type — rejected.** The existing stream, output field, and render functions
   already carry the required information. New surface would exceed the
   proven correction.
4. **Change detached or non-TTY acknowledgement behavior — rejected.** E08
   selects the existing un-detached TTY streaming lane. Detached/non-TTY JSON
   acknowledgement behavior is not implicated and remains unchanged.

## Consequences

- Positive: a single real streaming Service command truthfully exposes its
  accepted admission before its Stable result, enabling the accepted E08
  operator oracle.
- Positive: the correction reuses existing wire data and render text, keeping
  the public CLI and control-plane contracts stable.
- Negative: the Service streaming summary becomes the existing multi-line
  acknowledgement plus its existing Stable detail rather than Stable detail
  alone; consumers that deliberately parse TTY human output must tolerate the
  documented acknowledgement prefix.

## Evidence obligations

- A direct CLI renderer/stream test feeds the existing ordered Service
  `Accepted` then `Stable` NDJSON events through the current consumer and
  asserts one exact existing Accepted block precedes one exact existing Stable
  detail in its single returned summary. It must assert the order and
  multiplicity, not merely substring presence.
- E08 runs one fresh, un-detached `overdrive deploy service.toml` command under
  its PTY recorder and asserts the same ordered pair in that one captured
  stdout transcript. The runner removes detached first submission,
  idempotent resubmission, and transcript concatenation.
- The built default-feature E08 journey remains the end-to-end evidence for
  VM guest TCP/HTTP health, peer VM Job traffic, and cleanup. It must not
  replace this renderer correction with shell output synthesis.

## Delivery handoff

Return to the original DELIVER step `02-02` crafter. First make the smallest
existing-CLI renderer/stream change authorized above, with no new public
surface and no lifecycle/probe changes. Add the direct ordered-stream/render
regression, then simplify E08 to one fresh un-detached PTY-backed Service
deploy and recapture it through `verification/harness/run-expectation.sh E08`.
The original step reviewer must re-review both the product output and the
single-command evidence before the wave advances.

## References

- `verification/expectations/E08-vm-service-guest-health/README.md`
- `docs/feature/service-kind-vm-workloads/deliver/review-02-02.md`
- `docs/feature/service-kind-vm-workloads/feature-delta.md`
- `docs/feature/service-kind-vm-workloads/design/wave-decisions.md`
- ADR-0090, ADR-0091, ADR-0092
