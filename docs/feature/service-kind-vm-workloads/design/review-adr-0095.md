# Design review — ADR-0095: default streaming cap versus startup deadline

| Field | Value |
| --- | --- |
| Feature | `service-kind-vm-workloads` |
| Decision | [ADR-0095](../../../product/architecture/adr-0095-service-stream-cap-startup-deadline.md) |
| Review iteration | 1 |
| Date | 2026-09-07 |
| Verdict | **REJECTED** |

## Scope reviewed

ADR-0095 proposes changing only two existing duration values: the control
plane `DEFAULT_STREAMING_CAP` from 60 to 90 seconds and the private timeout
inside the existing streaming HTTP request from 90 to 120 seconds. It forbids
new wire/API/configuration/lifecycle/dependency/ownership surface.

## Evidence and assessment

| Question | Evidence | Result |
| --- | --- | --- |
| Is the collision real? | ADR-0058 fixes the inferred descriptor at 30 attempts x 2 seconds (`adr-0058-default-tcp-startup-probe-inference.md:42-57`). `service_lifecycle.rs:768-773` derives `interval * max_attempts`; `startup_probe_failed_action` requires attempts, elapsed deadline, and no pass (`:1213-1268`). The default stream cap is 60 seconds and is copied into `AppState` (`crates/overdrive-control-plane/src/lib.rs:485-488,680-703`). | Pass. Equal deadlines leave no required delivery time after the final observation. |
| Does the remedy preserve terminal ownership? | `build_service_stream` starts its cap future at `streaming.rs:886-913`, projects a lifecycle terminal at `:917-954`, and synthesizes stream-only `Timeout` at `:973-980`; `service_stream_synth_cap_timeout` is explicitly stream-only (`:800-810`). | Pass. `ServiceLifecycle` still owns `StartupProbeFailed`; the stream owns only `Timeout`. No lifecycle gate moves. |
| Are 90/120 coherent? | ADR-0032 sets the former 60-second default and rejects 120 seconds as a server-side operator-patience threshold (`adr-0032-ndjson-streaming-submit.md:461-472`). The current client has a 90-second private timeout specifically 30 seconds above the 60-second server cap (`crates/overdrive-cli/src/http_client.rs:239-260`). | Pass. 90 seconds provides a bounded 30-second timer window after the default deadline; 120 seconds retains the client/server margin. It is not a guarantee that an arbitrarily stalled process will run within 30 seconds. |
| Are explicit cap injections retained? | Both stream constructors read `state.streaming_cap` directly (`streaming.rs:190-200,886-894`); existing tests inject custom values, including a one-second Service cap (`tests/acceptance/service_submit_dispatch_wiring.rs:447-452`). | Pass. Changing the constructor default does not translate or override explicit construction. |
| Is the configuration/default boundary truthful? | No `ServerConfig::streaming_submit_cap_seconds` field or config parsing site exists. The only live setting is public `AppState::streaming_cap` (`lib.rs:238-250`), populated from the constant in `new_with_workflow_engine` (`:680-703`). Older comments/ADR text call an unimplemented server configuration override available. | Fail — R0095-1. |
| Is the product scope fully declared? | The same default feeds `build_workload_stream` (`streaming.rs:190-234`) and `build_service_stream` (`:886-913`). The same `ApiClient::submit_workload_streaming` method is called by both streaming deploy lanes (`crates/overdrive-cli/src/commands/deploy.rs:598,676`). | Fail — R0095-1. |

The second failure does not justify a Service-only cap: that would require the
new policy/surface ADR-0095 correctly forbids. The shared default is the
smallest implementation, but the contract must name its shared effect.

## Finding R0095-1 — shared default and configuration boundary are misstated

**Severity:** High, blocking design-contract correction.

`DEFAULT_STREAMING_CAP` is written to the one shared `AppState.streaming_cap`
field (`crates/overdrive-control-plane/src/lib.rs:680-703`). That field drives
the Job stream (`crates/overdrive-control-plane/src/streaming.rs:190-234`) and
the Service stream (`:886-913`). The generic client request timeout lives at
`crates/overdrive-cli/src/http_client.rs:234-260` and both deploy lanes call
it (`crates/overdrive-cli/src/commands/deploy.rs:598,676`). There is no
operator-facing configuration path that supplies the cap.

Yet ADR-0095 repeatedly calls this a Service cap and says an “operator/test”
can explicitly supply a cap (`adr-0095-service-stream-cap-startup-deadline.md:44-50,84-89`).
ACD-7 and DDD-10 repeat that Service-only framing
(`design/wave-decisions.md:364-370`; `feature-delta.md:416-421`). This leaves
the Job default `Timeout { after_seconds }` and its client envelope changed
without an accepted product contract, and promises a configuration surface the
source does not provide.

### Required bounded remediation

Revise ADR-0095 and the corresponding brief, feature-delta, and wave-decision
statements only:

1. State that `DEFAULT_STREAMING_CAP` and `ApiClient::submit_workload_streaming`
   are shared default envelopes, so the 90/120-second values also apply to the
   existing Job streaming lane. The VM-Service collision is the reason for the
   global-default change, not a Service-only mechanism.
2. State that no operator configuration exists today. The existing
   `AppState::streaming_cap` is a construction/test override, not a config-file
   or CLI option. Do not implement a configuration option.
3. Add a bounded delivery obligation that the default Job stream uses the
   shared 90-second cap while explicitly injected caps remain unchanged. The
   client-envelope regression must name both streaming lanes, or prove the
   shared `ApiClient` method once at that generic boundary.

No new API, field, configuration, lifecycle behavior, or architecture
mechanism is required.

## Required delivery evidence after remediation

1. A seeded existing-seam simulation/integration ordering proof: the standard
   60-second Service startup failure projects `StartupProbeFailed`, not
   `Timeout`, before the 90-second cap; print the seed on failure.
2. Boundary tests for default Service terminal-first behavior, injected
   cap-first `Timeout` with no lifecycle write, one-terminal closure, and no
   late-terminal reopening.
3. The shared-default Job regression required above and a 120-second private
   `ApiClient` envelope regression, with no public timeout option.
4. A fresh built-default-feature native-metal E13 run via `overdrive serve`
   and `overdrive deploy`: the unbound inferred guest listener must report
   `StartupProbeFailed`, remain ineligible, expose its inferred target/failure,
   and deny its peer Job. `Timeout` is not an accepted alternative.
5. A scope audit confirming no probe policy, TCP marking, terminal taxonomy,
   wire shape, lifecycle, config schema, or public command/API changed.

## Dispositions

| Item | Disposition |
| --- | --- |
| Equal 60-second inferred-startup / stream-cap collision | Proven from the current owner paths; timer separation is necessary. |
| `StartupProbeFailed` versus `Timeout` ownership | Preserved; a stream-authored lifecycle terminal is not authorized. |
| 90-second server / 120-second client values | Sound subject to R0095-1. |
| Service-specific cap, descriptor-derived cap, or new operator knob | Rejected as unnecessary new policy/API surface. |
| Shared Job default and generic client envelope | Missing from the present amendment; must be declared and tested. |
| Claimed existing operator configuration | Absent from source; must be described as absent, not added. |

## Verification performed

Read the mandatory repository rules and DESIGN skill; ADR-0032, ADR-0056,
ADR-0058, ADR-0059, ADR-0094, ADR-0095; the active brief, feature delta,
wave decisions, DISTILL/E13 contracts; and the current control-plane,
reconciler, streaming, handler, and CLI production paths. The available E13
capture ends at native-metal preflight, so it is not treated as delivery proof
in this review. No production code, test, roadmap, execution log, or
pre-existing dirty file was changed.

## Verdict

**REJECTED.** The timer remedy is source-grounded, minimal, and preserves the
correct lifecycle/stream split, but it hides the shared Job and generic-client
behavior of the two edited values and inaccurately implies an operator cap
override. Apply R0095-1's bounded documentation/evidence correction and send
the amendment through independent re-review.

---

## Iteration 2 — remediation re-review

**Scope:** Re-review of the R0095-1 documentation and delivery-obligation
remediation only. No production, test, runner, roadmap, or proposal mechanism
was changed by this review.

### R0095-1 disposition

| Required correction | Re-review evidence | Disposition |
| --- | --- | --- |
| Declare the shared Job and Service default effect | ADR-0095 now identifies the one `AppState::streaming_cap` default as feeding both stream constructors and the generic client as serving both deploy lanes (`adr-0095-service-stream-cap-startup-deadline.md:24-30,47-53,61-66,147-159`). The brief, ACD-7, and DDD-10 repeat that shared contract (`brief.md:5776-5794`; `design/wave-decisions.md:364-373`; `feature-delta.md:416-423`). This matches the default construction (`crates/overdrive-control-plane/src/lib.rs:680-703`), the Job and Service stream reads (`crates/overdrive-control-plane/src/streaming.rs:190-200,886-894`), and both callers of the generic client (`crates/overdrive-cli/src/commands/deploy.rs:598,676`). | Resolved. |
| Correct the configuration boundary | ADR-0095, the brief, ACD-7, and DDD-10 now say there is no operator-facing cap configuration and correctly limit `AppState::streaming_cap` to construction/test injection (`adr-0095-service-stream-cap-startup-deadline.md:55-59`; `brief.md:5789-5794`; `design/wave-decisions.md:370-373`; `feature-delta.md:421-423`). Source contains the field/default construction but no config-file or CLI parsing path (`crates/overdrive-control-plane/src/lib.rs:238-250,680-703`). | Resolved. |
| Require bounded shared-default evidence | ADR-0095 requires a Job-default-cap regression while preserving explicitly injected caps, and one generic private client-envelope regression covering both deploy lanes (`adr-0095-service-stream-cap-startup-deadline.md:161-176`). ACD-7 carries the same obligations (`design/wave-decisions.md:391-396`). These prove the shared seams without adding a public timeout surface. | Resolved. |

### Re-review assessment

The remediation accurately exposes the unavoidable global-default effect of
changing the existing constants: Job and Service streams both receive the
90-second default, while the existing generic private CLI request envelope is
120 seconds for both streaming deploy lanes. It neither represents the
construction/test field as an operator control nor authorizes such a control.
The stated evidence obligations cover the changed shared behavior and retain
the original Service ordering proof. No new public API, wire contract,
configuration, lifecycle behavior, or architecture mechanism is introduced.

### Verification performed

Re-read the amended ADR-0095, brief §80, ACD-7, and DDD-10 against the current
control-plane streaming/default-construction and CLI deploy/client call paths.
`git diff --check` reports no whitespace errors for the reviewed artifacts.
No delivery evidence is claimed by this DESIGN review; the required execution
and test evidence remains a DELIVER obligation.

## Verdict

**APPROVED.** R0095-1 is fully remediated. No reachable source-proven blocker
remains for this bounded amendment.
