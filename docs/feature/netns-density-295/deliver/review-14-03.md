# DELIVER review — STEP 14-03 conformance through required ports

## Review metadata

| Field | Value |
|---|---|
| Feature / step | `netns-density-295` / `14-03` — `[10-03 revalidate] Conformance through required ports` |
| Review role | Fresh isolated `nw-software-crafter-reviewer`, DELIVER implementation review |
| Review iteration | 1 |
| Approved parent | `1fd6407b0ed682826709933d22df23b416f87ae7` |
| Reviewed HEAD | `c65a6ccce9eb486b8a2d81d4a03604b9d05fc891` |
| Implementation commit | `5aaf29c960782ee0c828c6dd472fd81246a5fc5f` |
| DES follow-up | `c65a6ccce9eb486b8a2d81d4a03604b9d05fc891` |
| Final verdict | **APPROVED** |
| Reviewer-owned changes | This Markdown artifact only |

This review covers the complete parent-to-HEAD diff, the exact dispatch at
`.context/netns-density-295/dispatch-14-03.md`, the current feature-delta and
ADR-0124/ADR-0138/ADR-0219 contracts, the DISTILL-authored conformance body,
the retained DES evidence, and the production caller path. The user-dirty
`AGENTS.md` was preserved. No source, test body, model, design, roadmap, DES
log, or host state was changed by this review.

## Scope and accepted boundary

The accepted step activates exactly
`tests/conformance/tests/shared_guest_network_fail_stop_recovery.rs::shared_owner_fail_stop_shuts_down_before_a_fresh_handler_reopens_admission`.
The test starts the exported server handler in process through
`run_server_with_obs_and_driver`, supplies the existing `BootAdapters` shape,
uses `SimDataplaneComposer` and the supplied `SimSharedGuestNetworkOwner`,
drives workload behavior through the public HTTPS API, and obtains the typed
`ServeShutdownRequest` only through the retained `ServerHandle` delegation.
The scenario constructs a fresh handler over the same retained roots after the
fail-stop. It does not spawn the Overdrive binary, invoke a CLI, use a PID or
exit-code oracle, call a private supervisor, or run an expectation harness.

The current composition matches the accepted NO-OVERRIDE-SEAMS/STORES-IN
shape. `DirectHandlerHarness::start` builds `NodeAdapters::host` over the
temporary data root and replaces the test-selected boot ports by name:
`SimClock`, `SimObservationStore`, `SimVmHostState`, `SimDataplaneComposer`,
`SimGuestDnsFactory`, `SimMtlsIntercept`, and `SimKek`; stores and the CA remain
composition-root-owned, and a fresh bundle is built for the replacement
handler. The call at `tests/conformance/src/lib.rs:618-650` matches the
accepted five-argument supplied-owner entry point documented at
`docs/feature/netns-density-295/feature-delta.md:8144-8158`.

## Diff, API, and design compliance

The cumulative step diff contains exactly three files:

| File | Change | Disposition |
|---|---|---|
| `crates/overdrive-control-plane/src/lib.rs` | Move `shared_owner_unhealthy` emission inside the new-recovery branch; add the approved audit cause, retry fields, and fail-stop cleanup field | Required production behavior |
| `tests/conformance/tests/shared_guest_network_fail_stop_recovery.rs` | Remove only the `pending DELIVER step 14-03` ignore marker from S-ND295-33 | DISTILL activation only |
| `docs/feature/netns-density-295/deliver/execution-log.json` | Append the actual RED/GREEN/COMMIT events | DES audit record |

No public method, type, enum variant, trait, parameter, port, wire field,
persisted value, configuration field, or composition-root signature was added
or changed. The test-support surface was not reshaped. `git diff --check`
passes for the cumulative step diff, and the test-file diff is exactly one
deleted ignore attribute.

The accepted fail-stop vocabulary remains the exact post-cut set from the
feature delta (`RecoveryDeadlineExceeded`, `LeakedQuiescenceHold`,
`ForeignQuiescenceHold`, `SupervisorReturned`, `SupervisorFailed`,
`SupervisorPanicked`, `SupervisorCancelled`, and `RequestChannelClosed`). No
generic string error, new public cause, or unapproved error carrier was used.

## Typed semantic failure mapping

The reviewed path maps each semantic failure it can reach to the approved
representation and preserves the fail-closed lifecycle:

| Semantic condition | Approved representation | Reviewed implementation / observable evidence |
|---|---|---|
| Shared-owner audit mismatch for `FlowListeners` | `guest_network.shared_owner_unhealthy` with `component=FlowListeners`, `cause=audit_mismatch`; recovery is held under the existing EXEC gate | `lib.rs:1791-1812`; conformance assertions at `shared_guest_network_fail_stop_recovery.rs:237-241` |
| A completed repair-plus-audit attempt remains unhealthy | Existing `SharedGuestNetworkRecovery.attempts` counter, projected in `guest_network.shared_owner_retry` as numeric `component` and `attempt` fields | `lib.rs:1944-1974`; assertions at `:242-251` |
| Recovery exhausts the five-second/twenty-attempt bound | `SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded` in the existing `ServeShutdownRequest::SharedGuestNetwork` receipt | `lib.rs:1975-1998`; assertions at `:227-235` |
| Fail-stop abandons ownership while shutting down | Existing structured fail-stop event field `cleanup=abandoned_to_shutdown` | `lib.rs:1694-1716`, `:1979-1998`; assertion at `:252-254` |
| Leaked hold | Existing `SharedGuestNetworkFailStopCause::LeakedQuiescenceHold` | `lib.rs:1669-1692`; no taxonomy change |
| Foreign restore | Existing `QuiescenceRestoreError::ForeignHold` mapped to `SharedGuestNetworkFailStopCause::ForeignQuiescenceHold` | `lib.rs:1915-1939`; the added cleanup field is diagnostic only |
| Supervisor return, typed failure, panic, cancellation, or closed request channel | Existing distinct `SharedGuestNetworkFailStopCause` variants, selected by `shutdown_requested` | `lib.rs:1600-1634`; unchanged public request shape |
| Intentional shutdown | Cancellation-and-join path, with no fail-stop cause | ADR-0124 contract and `ServerHandle` shutdown ownership; unchanged by this step |

The unhealthy event is now emitted only when a component enters a new recovery
episode. Repeated audit findings and post-repair findings update the existing
episode without emitting another unhealthy record. A restored episode is
removed before a later independent loss, so a later loss correctly starts a
new episode. This is the required one-record-per-episode behavior and does not
reopen EXEC or weaken the fail-stop transition.

The active step exercises the `audit_mismatch` cause and the
`RecoveryDeadlineExceeded` path. Task-exit and audit-timeout cause variants
are outside this single S-ND295-33 activation and receive no acceptance credit
from this review; no new claim about those separate scenarios is made here.

## Test integrity and external validity

The active body retains both required declarations:

```rust
/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// CONTRACT_SHAPE: bounded-change.
```

The body continues to assert observable outcomes only: public API admission,
the typed shutdown request, fail-stop state, recovery cadence, event fields,
single-request behavior, failed-handler API refusal, fresh-handler boot order,
and fresh-handler public API admission. It does not inspect private fields or
reach into `SharedNetworkSupervisorHandle`.

The RED-to-GREEN comparison shows no assertion deletion, weakening, skip,
renaming, fixture end-state seeding, or test-body rewrite. The only test edit
is removal of the exact pending marker. There is one active integration
behavior and no new unit-test layer; the integration scenario's one-example
shape is appropriate for a production-composition and lifecycle proof. No
mock-dominated SUT, zero-assertion test, tautological oracle, circular oracle,
or expectation-runner boundary violation was found.

## DES and verification evidence

The execution log contains the complete canonical three-phase sequence for
14-03, in order:

| Phase | Status | UTC timestamp |
|---|---|---|
| RED | EXECUTED / PASS | `2026-11-01T08:28:49Z` |
| GREEN | EXECUTED / PASS | `2026-11-01T08:37:06Z` |
| COMMIT | EXECUTED / PASS | `2026-11-01T08:41:55Z` |

The retained receipts supplied with the step record the following:

| Check | Receipt | Result |
|---|---|---|
| Semantic RED through the in-process handler | `a8a77412c67afa6468b522f5` | Exit 100 at the behavioral fail-stop assertion |
| Targeted GREEN | `0ba494ce48cf066d969ea6ad` | Exit 0 |
| Full conformance package | `62077c1aff97b470496363d7` | One active test passed |
| Workspace check | `b71b6f196e03c459ae1fe722` | Passed |
| Formatting | `a6adc2605bb63f359e8399db` | Passed |
| Workspace Clippy | `e84cd66a40052063b6dc974c` | Stopped at reviewed baseline warnings |

The Clippy receipt is not treated as an unqualified pass. The reported
warnings are the pre-existing `expect_used` at
`crates/overdrive-control-plane/src/guest_network.rs:752` and the existing
`unchecked_time_subtraction` in the shared-network acceptance source near its
`SHARED_NETWORK_AUDIT_PERIOD - offset` calculation. No new warning exception
was added and no warning was hidden. The current formal inputs are unchanged;
the retained formal receipt `2228824-1793511799.752` reports 279/279 and
`9ea4762a840e20a5b4851754` verifies the evidence. Formal checks were not
gratuitously rerun.

No mutation testing or Lima run was performed by this review. The conformance
receipt is an ordinary in-process userspace/Sim composition check; no native
kernel, module, nftables, BPF, cgroup, netns, route, sysctl, kTLS, or VM
effect was introduced by the reviewed diff.

## Review findings and dispositions

### `praise:` production-boundary and lifecycle proof is exact

The scenario reaches the real exported handler and public HTTPS API, observes
the existing typed request, proves admission remains closed through the
bounded recovery, proves exactly one request, then proves that a separately
constructed handler over the retained roots completes boot before reopening
public admission. This directly discharges the step's boundary and lifecycle
outcomes without adding a private observer or replacement mechanism.

### `praise:` telemetry changes are minimal and authorized

The production delta supplies only the fields the activated DISTILL body
asserts: one unhealthy event for the episode, numeric retry attempt evidence,
and the approved abandoned-cleanup marker on fail-stop records. The additions
are structured tracing fields and do not change the typed request, gate state,
owner hold, shutdown order, or recovery budget.

### Evidence limitation — non-blocking

This step activates the audit-mismatch S-ND295-33 body only. It does not claim
independent evidence for the separate listener-task-exit or audit-timeout
event-cause branches, nor for the later CLI lifetime's outer cleanup events.
Those branches retain their existing typed/lifecycle owners and are outside
this step's accepted activation set. This is a scope boundary, not a waiver of
the fail-stop contract.

No blocking, high, medium, or low implementation findings remain.

## Quality-gate disposition

| Gate | Result | Evidence |
|---|---|---|
| G1 single acceptance behavior | PASS | Exactly one DISTILL-authored S-ND295-33 body is active |
| G2 semantic RED | PASS | Retained in-process RED receipt fails at the admission/fail-stop behavior |
| G3 assertion failure | PASS | RED is behavioral, not collection/import/infrastructure failure |
| G4 no domain mocks/private SUT | PASS | Exported handler, public HTTPS API, supplied driven-port owner |
| G5 business/domain language | PASS | Fail-stop, recovery episode, admission, fresh-handler boot and typed cause language |
| G6 GREEN | PASS | Targeted and full package receipts exit successfully |
| G7 pre-commit completion | PASS | COMMIT phase recorded after GREEN; full package had one active pass |
| G8 test budget | PASS | One integration behavior; one body, with no redundant unit tests |
| G9 test integrity | PASS | Marker deletion only; body and assertions unchanged |
| Contract Shape | PASS | Outcome anchor and `bounded-change` declaration retained; no banned implementation-shaped name |
| API shape | PASS | No new public surface; existing typed error/request vocabulary preserved |
| Safe execution boundary | PASS | No kernel/VM mutation lane was added or claimed for this in-process test |

## Iteration 1 verdict

**APPROVED.** Step 14-03 activates the exact existing S-ND295-33 conformance
body through the accepted in-process production composition, preserves the
typed fail-stop and fail-closed lifecycle, adds only the authorized structured
telemetry needed by the scenario, and records complete RED/GREEN/COMMIT DES
evidence. The reviewed Clippy baseline remains honestly qualified, and no
mutation, Lima, formal rerun, host mutation, API invention, test weakening, or
adjacent remediation is required.
