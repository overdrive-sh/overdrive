# ADR-0095: Keep the default streaming cap beyond the default Service startup deadline

## Status

**Proposed** (2026-09-07). This focused DESIGN amendment is authorized only
for the proven `service-kind-vm-workloads` E13 cap/deadline collision.
Independent DESIGN review is required before the original DELIVER `02-03`
crafter changes implementation or evidence.

It amends ADR-0032's default-cap rationale and supersedes the bounded
"60s unchanged" statement in the Service-health brief §80. It does not amend
the Service terminal taxonomy in ADR-0059, the inferred-probe contract in
ADR-0058, or ADR-0094's TCP socket-marking decision.

## Context

The built native-metal E13 unbound-guest control now reaches the intended
guest port after ADR-0094's marked TCP socket correction. Its inferred default
TCP startup descriptor has the existing policy `max_attempts = 30` and
`interval_seconds = 2`, hence `ServiceLifecycle` derives its existing
`startup_deadline = 60s`. `StartupProbeFailed` is emitted only when both the
attempt gate and elapsed-deadline gate hold.

Both existing stream constructors start the same `AppState::streaming_cap`
future concurrently and synthesize their existing stream-only `Timeout` if it
wins. The normal value is the existing public `DEFAULT_STREAMING_CAP = 60s`.
The Service collision therefore occurs in a shared default envelope: it also
governs the existing Job streaming lane. The existing private
`ApiClient::submit_workload_streaming` timeout is likewise the generic client
envelope used by both deploy lanes. For the Service path, the equal
operator-wait and lifecycle deadlines leave no scheduling room for the final
failed observation, reconciliation tick, action-shim write, and lifecycle
broadcast. The observed result is a streaming `Timeout` while the allocation
remains `Running`, rather than the existing `StartupProbeFailed` terminal
required by S-SVM-29/E13.

This is not a new lifecycle failure category. ADR-0059 deliberately reserves
`Timeout` for the streaming loop: it means that the wait budget elapsed while
the reconciler may still converge. Reclassifying it as `StartupProbeFailed`,
or causing the stream to author a lifecycle terminal, would falsely collapse
those distinct owners and semantics.

## Decision

### Default cap and client envelope

Change only the value of the existing
`overdrive_control_plane::DEFAULT_STREAMING_CAP` from **60 seconds to 90
seconds**. `AppState::new` continues to copy that existing constant into its
existing `streaming_cap` field, and both `build_workload_stream` and
`build_service_stream` continue to use that field's one existing
`clock.sleep(cap)` future. The value is a shared default for the existing Job
and Service streaming lanes, not a Service-only cap.

There is no operator-facing configuration for this cap today: no config-file
or CLI option supplies it. The existing `AppState::streaming_cap` field is a
construction/test override, whose explicitly supplied value remains
unchanged; this amendment adds no configuration field or option and no
descriptor-derived cap.

The existing private HTTP request timeout in
`ApiClient::submit_workload_streaming` changes from **90 seconds to 120
seconds**. This generic client envelope serves both existing streaming deploy
lanes. It remains an envelope around the server's typed terminal; the
30-second margin is retained, so the client cannot race and mask the new
90-second server `Timeout` with `CliError::Transport`.

Ninety seconds is intentional: it preserves a 30-second bounded delivery
envelope after the 60-second default startup deadline, while staying below the
120-second operator-patience boundary rejected by ADR-0032. No new constant,
field, method, type, variant, trait, port, parameter, route, command, or
configuration surface is introduced.

### Exact terminal contract

For a default-configured Service whose inferred or explicit default startup
descriptor has the existing 60-second deadline:

1. a final failed result that reaches `ServiceLifecycle` at that deadline is
   evaluated by the existing `attempts >= max_attempts && elapsed >=
   startup_deadline && no_pass` condition;
2. the existing `TerminalCondition::ServiceFailed {
   StartupProbeFailed { .. } }` is still authored only by
   `ServiceLifecycle` and projected through the existing Service stream; and
3. the existing stream therefore emits `ServiceSubmitEvent::Failed {
   reason: StartupProbeFailed { .. } }` before its 90-second cap, then closes.

`Timeout { after_seconds: 90 }` remains the existing streaming-loop-only
terminal when no projectable lifecycle terminal arrives by 90 seconds. It
continues to leave the allocation and lifecycle convergence untouched.
`StreamInterrupted`, `Stable`, `Stopped`, all exit codes, and every
non-default/custom `AppState::streaming_cap` behavior remain unchanged.

An `AppState` construction/test override that explicitly supplies a cap at or
below a Service's own startup deadline retains the existing `Timeout`
behavior; this amendment does not silently override an explicitly constructed
cap. Likewise, a Service with an explicitly configured startup deadline at or
beyond 90 seconds may still reach the existing streaming `Timeout`. The
change closes the proven collision in the shared default path; it does not add
per-Service waiting policy or an operator timeout setting.

### Lifecycle Gate Ownership

**Not applicable:** this changes the duration of an existing streaming
operator-wait timer only. It adds, removes, and moves no lifecycle or health
gate.

| Signal or state | Owner | Unchanged promise | This amendment must not do |
| --- | --- | --- | --- |
| Allocation `Running` | action shim + selected driver | Driver start completed and Running was committed | Delay, revoke, or reinterpret `Running` |
| `StartupProbeFailed` | `ServiceLifecycle` startup branch | Failed startup attempts exhausted the existing deadline | Be authored, synthesized, or reclassified by the stream |
| Service `Stable` | `ServiceLifecycle` startup branch | Existing startup success predicate passed | Be delayed or made a different lifecycle state |
| `Timeout` | Service streaming loop | Operator wait budget elapsed without a stream terminal | Become a lifecycle terminal or alter allocation state |
| Eligibility/liveness/restart | existing `ServiceLifecycle` and `WorkloadLifecycle` owners | Existing readiness, liveness, and unified-budget contracts | Be changed by a startup stream wait |

The ordering remains: `Accepted` is emitted first; the lifecycle terminal is
projected if it arrives before the cap; otherwise the cap emits the existing
`Timeout`; the stream closes after exactly one terminal. A late lifecycle
success or failure after `Timeout` stays on its existing row/describe path and
cannot resurrect the already closed stream.

## Alternatives considered

1. **Leave both defaults at 60 seconds — rejected.** E13 reproduces the race
   on the real owner path: a truthful refusal cannot surface the existing
   startup terminal before the equal stream cap.
2. **Turn the cap `Timeout` into `StartupProbeFailed` — rejected.** The stream
   has no authority to assert attempt exhaustion or a probe failure; ADR-0059
   defines `Timeout` as a distinct streaming-only result.
3. **Change the inferred startup descriptor's 30 × 2s policy — rejected.** It
   would change the established inference/startup-policy contract merely to fit
   the unrelated wait timer, and affects probe behavior beyond the proven gap.
4. **Derive a Service-specific cap from descriptors — rejected.** The existing
   stream receives only state, workload id, and accepted event. Adding a
   descriptor lookup, parameter, field, or new policy couples a generic stream
   to lifecycle policy and invents scope not needed for the default collision.
5. **Use 120 seconds as the server default — rejected.** ADR-0032 already
   rejected that wait as beyond the operator-patience threshold. Ninety seconds
   supplies the required 30-second delivery envelope without crossing it.
6. **Keep a 90-second client timeout after raising the server cap — rejected.**
   Equal client/server timers can replace the server's typed `Timeout` with a
   transport error, recreating the same class of masked terminal at the next
   boundary.

## Consequences

- Default streaming deployments for both existing Job and Service lanes may
  wait up to 90 seconds before their existing `Timeout`, rather than 60
  seconds.
- The default 60-second startup-failure path has room to publish its existing
  `StartupProbeFailed` terminal; E13 must not accept `Timeout` as an
  alternative.
- Both existing streaming deploy lanes use the same private 120-second client
  envelope rather than the former 90-second envelope.
- Existing public API and wire shapes are unchanged. The only source changes
  are existing duration values and their documentation/tests.
- Explicitly constructed `AppState` caps and long explicit startup policies
  retain their current independent semantics; no hidden cap translation or
  operator configuration is introduced.

## Evidence obligations

- A seeded `overdrive-sim` ordering invariant drives the existing Service
  stream/lifecycle composition with the standard 60-second startup failure and
  proves that the emitted terminal is `StartupProbeFailed`, not `Timeout`; it
  must print its seed on failure. The invariant uses existing clock and stream
  seams and adds none.
- Existing Service stream tests pin the boundary: with the default cap the
  standard failure terminal wins; with an explicitly injected cap that expires
  first, the existing `Timeout` still wins and does not write/rewrite lifecycle
  state; a late lifecycle terminal cannot add a second stream terminal.
- A Job-stream regression pins that the unchanged default construction uses
  the shared 90-second cap, while an explicitly injected cap remains
  unchanged. A generic `ApiClient::submit_workload_streaming` regression pins
  its private 120-second envelope for both streaming deploy lanes, without a
  public timeout option.
- E13 re-runs through built default-feature `overdrive serve` and
  `overdrive deploy` on qualified native metal. The unbound inferred guest
  listener must produce `StartupProbeFailed`, remain ineligible, report its
  inferred guest target/failure, and deny the peer VM Job. `Timeout` is a
  failure of the acceptance contract.

## Compatibility and delivery handoff

The change is behaviorally compatible with existing Service streaming
semantics: the event enum, JSON/NDJSON shape, terminal taxonomy, terminal
owners, and stream close rule are unchanged. The default wait is longer;
consumers that impose their own 60-second deadline must account for the
documented 90-second server default. No migration or persistence action is
required.

Return to the original `02-03` crafter only after independent DESIGN approval.
Implement exactly the two existing timeout-value changes, their bounded shared
Job/Service and client-envelope tests, the required simulation invariant, and
E13 evidence recapture. Do not alter probe policy, lifecycle logic, stream
projection, TCP semantics, configuration shape, or unrelated expectations.

## References

- ADR-0032 — streaming cap and its operator-wait semantics
- ADR-0058 — inferred default TCP startup descriptor
- ADR-0059 — `Timeout` is streaming-loop-only; `StartupProbeFailed` is a
  lifecycle-projected terminal
- ADR-0094 — prerequisite TCP socket marking correction
- `crates/overdrive-control-plane/src/{lib.rs,streaming.rs}`
- `crates/overdrive-reconcilers/src/service_lifecycle.rs`
- `crates/overdrive-cli/src/http_client.rs`
- `docs/feature/service-kind-vm-workloads/distill/test-scenarios.md` S-SVM-29
- `verification/expectations/E13-vm-service-inferred-tcp-startup/README.md`
