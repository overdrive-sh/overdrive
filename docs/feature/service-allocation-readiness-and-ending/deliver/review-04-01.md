# DELIVER Review — Step 04-01: Progress deadline derived from startup deadline

## Review metadata

| Field | Value |
|---|---|
| Feature | `service-allocation-readiness-and-ending` |
| Step | `04-01` — Progress deadline derived from startup deadline |
| Iteration | 1 |
| Reviewer | Codex, fresh isolated DELIVER reviewer |
| Implementation commit | `13593ea48c6299f25cfeb2077d0a6d4d6f9b5746` |
| COMMIT audit commit | `ae44d674e81cf0cc399ff491305b0d7c206508d6` |
| Comparison base | `b4e5b2a8572c041317f012401c762b4fe9068d8d` (approved step 03-02 audit state) |
| Authoritative contract | Approved roadmap step 04-01; feature-delta D10 and DG-7; ADR-0262 and ADR-0264; architecture brief |
| Upstream prerequisite | Step 03-02 review is `APPROVED`; D10's service deadline and DG-7's pinned signatures are in the accepted contract |
| Verdict history | Iteration 1: **NEEDS_REVISION**; iteration 2: **APPROVED** after both findings were closed |

## Executive summary

The D10 implementation derives the Service stream deadline from the validated startup probes, keeps `streaming_cap` as the floor and Job bound, captures only the latest attempt-ending event observed after this stream opens, and renders that event only as labelled detail on a synthesized `Timeout`. The handler and CLI use the same derivation, the CLI adds the approved 30-second envelope, and the `ServiceFailureReason` archived shape remains unchanged. The pinned DG-7 signatures and the `LastAttemptWire` field layout match the accepted contract. The accepted oracle correction is exact: the production stop event carries `TransitionReason::Stopped { by: Reconciler }`, `TerminalCondition::Stopped { by: StartupProbe }`, and no detail.

The step cannot be approved yet. `AppState::streaming_cap` still documents the nonexistent `[server] streaming_submit_cap_seconds` setting and a 60-second default even though D10 explicitly requires correcting that comment. Separately, D10's interface-change table says the public `service_stream_synth_cap_timeout` takes the latest attempt observation, but neither D10 nor DG-7 pins that public parameter's type or signature; the DISTILL upstream-issues record confirms the parameter remains unpinned. The stream's user-visible timeout is correctly populated by the caller, so this is not a reproduced stream-output defect. It is an unresolved public-interface contract and must return to DESIGN before approval.

## D10 contract and production-path review

### Deadline and ownership

`service_lifecycle::startup_deadline` extracts the previous first-startup-probe calculation: interval times max attempts, with the existing default for no descriptor and the existing checked-multiplication fallback. `streaming::progress_deadline` implements `max(floor, startup_deadline + PROGRESS_DEADLINE_MARGIN)`, where the margin is 30 seconds. The production handler derives the deadline from the validated `WorkloadIntent::Service` and `state.streaming_cap` (`handlers.rs:636–647`); the Job handler continues through the existing Job builder with `state.streaming_cap` unchanged. The pure-function tests cover the default 90-second result, the floor, a longer startup window, first-probe-only derivation, and descriptor-list equivalence with the prior calculation.

The CLI derives the same value from the `Service` returned by `Service::from_submit` (`commands/deploy.rs:651–672`). `ApiClient::submit_workload_streaming` takes that value as its second parameter and applies `server_deadline + PROGRESS_DEADLINE_MARGIN` (`http_client.rs:294–317`). The Job caller still passes `DEFAULT_STREAMING_CAP`, preserving its prior 120-second envelope.

`build_service_stream` arms the injected clock once after emitting `Accepted` and subscribing; it does not reset the clock on later events (`streaming.rs:1071–1085`). For each event from this workload with `to` equal to `Failed` or `Terminated`, it copies `alloc_id`, `reason`, `terminal`, and `detail` separately into the in-memory `LastAttemptWire` (`streaming.rs:1092–1105`). At deadline, it synthesizes the existing `Timeout` with no verdict allocation or stderr and attaches the in-memory value; the stream writes no row (`:1165–1174`). Normal terminal projections set `last_attempt: None` (`:899–990`), so an authoritative restart give-up remains its own failure rather than being replaced by the latest attempt detail. The S-35 never-ready test retains the row equality assertion and separately checks the exact Reconciler reason, StartupProbe terminal, and absent detail.

### Public API and wire shape

The new `startup_deadline` and `progress_deadline` pure functions and the 30-second constant match D10. `build_service_stream` has `progress_deadline: Duration` last, `ApiClient::submit_workload_streaming` has `server_deadline: Duration` second, and `LastAttemptWire` remains at `overdrive_control_plane::streaming` with exactly the approved four fields. `ServiceSubmitEvent::Failed.last_attempt` is optional, serde-defaulted, and omitted when `None`; the current OpenAPI document contains the optional property and `LastAttemptWire` schema. `format_service_failed_block` uses the approved `last_attempt` parameter and renders the D10 timeout wording and labelled block only for `Timeout`.

No additional D10 public type, enum variant, configuration field, or error type was introduced. The `ServiceFailureReason::Timeout { after_seconds }` archived contract remains unchanged. The helper-contract ambiguity below is the sole unresolved API-shape concern.

### Typed failure mapping

| Designed condition | Approved and implemented representation |
|---|---|
| Progress deadline expires | `ServiceSubmitEvent::Failed { alloc_id: None, reason: ServiceFailureReason::Timeout { after_seconds }, stderr_tail: None, last_attempt }`; `last_attempt` is the latest attempt-ending lifecycle observation seen by this stream, or `None` if there was none. |
| A lifecycle terminal is projected | Existing typed `ServiceSubmitEvent` projection with `last_attempt: None`; an authoritative give-up remains the terminal verdict. |
| Lifecycle broadcast closes | Existing synthesized `ServiceFailureReason::StreamInterrupted`, with no `last_attempt`. |
| Client-side request timeout | Existing `CliError::Transport`; the D10 envelope is deadline plus 30 seconds so it does not normally hide the server terminal. |

No missing semantic error distinction or generic-string substitute was found in the D10 failure path. The new helper-signature ambiguity is about a public interface, not an error taxonomy.

## Test coverage, integrity, and TDD evidence

The roadmap names 18 transitioned D10 test identities: five pure derivation tests across the two crates, four composed Service stream scenarios, the default-window startup-stop scenario, two CLI render cases, one CLI request-timeout integration case, and five Quint conformance runs. All listed identities are active in the reviewed commit. The acceptance and integration coverage checks the slow-start stable result, timeout boundary and latest attempt, give-up precedence, a new clock on unchanged redeploy, render wording/detail, and Service/Job HTTP envelopes. The Quint driver exercises the production stream builder against the accepted progress-deadline model. The small pure-function test set covers four distinct behaviors; five test functions are within the `2 × behavior` budget. The composed, integration, and Quint scenarios are acceptance/conformance obligations explicitly required by the approved roadmap, not an excess of unit tests.

Every newly activated D10 test has its own required `CONTRACT_SHAPE` declaration. The source-local pure-function rustdoc requirement does not apply to these tests in `tests/acceptance` or to the Quint driver. The pre-existing Job-stream and CLI Job-envelope controls were left unchanged. Test names describe user-visible behavior, and the composed stream tests enter through the production `submit_workload` handler with the production shim/reconcilers and simulated driven ports. The CLI timeout integration uses a real TLS test server and the production `ApiClient`; no Rust test launches the built Overdrive binary or acts as an expectation runner.

The only assertion change in the implementation diff is the independently approved DISTILL oracle correction in `service_stream_progress_deadline.rs`: the stop reason is now asserted as `Reconciler`, the separate terminal remains `StartupProbe`, and `detail == None` is added. The deadline, timeout reason, absent verdict allocation/stderr, exact allocation id, and no-row-write complement remain intact. The correction is recorded and reviewed in `distill/progress-deadline-last-attempt-oracle-remediation.md` and its review counterpart. Removing the D10 `#[ignore]` markers activates the pre-authored cases; no case or assertion was deleted, weakened, or skipped to accommodate the implementation.

The execution log records the actual sequence and failed attempts: RED/FAIL at `06:48:34Z`; GREEN/FAIL at `06:57:41Z`, followed by GREEN/PASS at `07:08:16Z`; COMMIT/FAIL at `07:16:25Z`, followed by COMMIT/PASS at `07:21:16Z` on 2026-11-10. The final COMMIT audit commit adds that last pass event. This preserves the RED → GREEN → COMMIT order while retaining the failures and subsequent fixes.

The crafter handoff reports 27/27 focused tests passing after restoring the implementation, including the 18 assigned D10 identities, five Quint runs, and the Job/schema/render controls; it also reports changed-file formatting and Clippy passing. Those test commands are not retained in `.context/` for this step, and I did not rerun cargo, Quint, or mutation testing in this review. I therefore record them as author-reported evidence, not independent test results. The workspace rule reserves mutation testing for the final DELIVER-wave gate. `git show --check` passed for both reviewed commits. The workspace had two pre-existing untracked DISTILL remediation documents; this review preserves both.

## Findings

### F-01 — D10 leaves the signature of a named public timeout helper ambiguous

**Kind:** `SPECIFICATION_AMBIGUITY` — public interface contract. **Severity:** blocker.

**Locations:** `docs/feature/service-allocation-readiness-and-ending/feature-delta.md:1241`; `docs/feature/service-allocation-readiness-and-ending/distill/upstream-issues.md:71`; `crates/overdrive-control-plane/src/streaming.rs:972–978, 1165–1170`.

The D10 interface-change table says `service_stream_synth_cap_timeout` “takes the latest attempt-ending observation and fills `last_attempt`.” The function is public through the public `streaming` module, but its current signature accepts only `after_seconds: u32` and always returns `last_attempt: None`. The production `build_service_stream` caller then mutates that result to attach the observation. The accepted DISTILL upstream-issues record explicitly notes that this helper's “latest attempt-ending observation” parameter type is unpinned and that no test calls it.

The real submit owner path is reachable: `handlers::submit_workload` passes the derived deadline to `build_service_stream` (`handlers.rs:636–649`); that stream records observations and calls the helper at deadline (`streaming.rs:1092–1105, 1165–1170`). This path currently emits the correct timeout wire value. The mismatch is not a claim that the operator-facing stream is wrong; the unresolved contract is that the accepted interface table assigns behavior to a public helper without pinning how the observation is represented or passed. Directly invoking the current helper cannot supply an observation and always produces `last_attempt: None`; the only current caller compensates outside the helper.

The workspace interface rule forbids choosing a convenient new parameter shape when DESIGN has not pinned one. Do not prescribe `Option<LastAttemptWire>`, a reference, or a visibility change in DELIVER. Return this exact interface question to DESIGN: is the public helper's contract part of D10, and, if so, what exact signature does it have; otherwise, should the interface table describe the caller-level projection without assigning it to this helper? Resume this step only after the approved contract is recorded.

### F-02 — `AppState::streaming_cap` still documents a nonexistent setting and stale default

**Kind:** accepted-contract documentation mismatch. **Severity:** high, blocking.

**Location:** `crates/overdrive-control-plane/src/lib.rs:293–298`.

The comment says the stream default is 60 seconds and claims it is configurable with `[server] streaming_submit_cap_seconds`. D10 explicitly says the stale setting reference at this location is corrected with this feature (`feature-delta.md:788–794`). The same source defines `DEFAULT_STREAMING_CAP` as 90 seconds and says `AppState::streaming_cap` is a construction/test override with no operator configuration (`lib.rs:566–569`); the production `AppState` constructor sets it from that constant (`:614–621, 797`). A workspace search found the configuration key only in the stale comment and historical artifacts, not in a current server config field or parser. The implementation's behavior follows D10, but the source documentation still advertises a removed/nonexistent configuration surface and contradicts the accepted default.

Update only this comment to say the field is a construction/test override, production defaults to 90 seconds, and no operator setting supplies it. Do not add a configuration option. This is a specifically named D10 correction, not adjacent documentation cleanup.

## Contract Shape compliance

All transitioned D10 tests have a per-test `CONTRACT_SHAPE` declaration. The two pure properties in `startup_deadline.rs` and the three in `progress_deadline_derivation.rs` use the exact `/// CONTRACT_SHAPE: pure-function.` line. The five Quint identities and composed/CLI acceptance tests carry their own declarations. The unchanged Job controls are pre-existing and were not transitioned in this step. No banned test-name pattern or testing-theater assertion was introduced in the reviewed diff. No state-delta obligation applies to the pure renderer or the stream's explicitly declared output/row universe.

## Review dimensions and quality gates

| Dimension | Result |
|---|---|
| Exact D10 / DG-7 API shape | **Pass** for the pinned functions, parameters, field and module path; **blocked** by F-01 for the separately named public helper contract. |
| Production owner path / external validity | Pass — handler to production Service stream; client uses validated Service and the same constant floor. |
| Typed failure mapping | Pass — Timeout, normal terminal, stream closure, and client transport timeout retain their approved typed representations. |
| Acceptance completeness | Pass — all 18 roadmap-assigned identities are active; Job behavior controls remain. |
| Test integrity | Pass — only pending markers were removed; the reason/terminal correction was approved upstream and adds a typed detail assertion. |
| TDD phase evidence | Pass — final RED, GREEN, and COMMIT outcomes are present in order; intermediate failures remain recorded. |
| Test budget | Pass — five pure-function test methods against four distinct behaviors; composed, client-integration and Quint checks are required acceptance/conformance evidence. |
| Commit scope and attribution | Pass — implementation commit touches the 13 assigned code/test files plus the DES log; audit commit adds the COMMIT pass event. Marcus remains Git author and each Codex-created commit contains exactly `Co-Authored-By: Codex <codex@openai.com>`. |
| RPP scan | L1–L2 reviewed; no actionable in-scope smell found. |
| Mutation gate | Not run — correctly deferred to the single final DELIVER-wave gate. |

## Remediation and verdict

| Finding | Disposition |
|---|---|
| F-01 — named public timeout helper signature is not pinned | **Open; return to DESIGN.** Do not change its API in DELIVER until the exact interface contract is approved. |
| F-02 — stale `AppState::streaming_cap` documentation | **Open; bounded source-comment correction required** after the interface contract is resolved and this step resumes. |

**Iteration 1 verdict: NEEDS_REVISION.** The D10 production stream output and pinned APIs pass review, but the public helper contract is ambiguous and the explicitly required configuration/default documentation correction is absent. The implementation must not advance to step 04-02. Route F-01 to DESIGN for an exact ruling; after approval, return the same step to its original crafter for any necessary bounded remediation, correct F-02, rerun required GREEN evidence, and return it to this step's reviewer for re-review. No implementation, test, DES, roadmap, or commit changes were made by this reviewer; this artifact is the only file written by the reviewer.

## Iteration 2 — remediation re-review

### Revisions reviewed

Two commits from the original step 04-01 crafter close the iteration-1 findings:

| Commit | Scope | Review result |
|---|---|---|
| `63741cf9d125487747ed28677479dc0d10936bef` | Corrects the `AppState::streaming_cap` rustdoc only | Closes F-02. The field is documented as the construction/test override and Service deadline floor; production defaults to 90 seconds and no operator setting supplies it. |
| `971560e027bae6b12f21e33dc91aa0ec991943da` | Implements the DESIGN-approved D10-H1 helper signature, updates its production and test call sites, and records the approved DESIGN/DISTILL artifacts | Closes F-01. |

### F-01 disposition — closed

The accepted D10 interface table now pins the exact public signature:

```rust
pub fn service_stream_synth_cap_timeout(
    after_seconds: u32,
    last_attempt: Option<LastAttemptWire>,
) -> ServiceSubmitEvent;
```

The bounded D10-H1 DESIGN clarification and its independent DESIGN review record the rationale and approval under the existing D10 ruling. The implementation at `streaming.rs:973–982` matches that exact signature and constructs the existing timeout failure with the owned `last_attempt` input directly; the caller no longer mutates the returned event (`:1169–1175`). The only call sites are the production deadline branch, which passes the latest optional observation owned by `build_service_stream`, and the existing cross-crate taxonomy property, which passes `None` (`service_submit_event_taxonomy.rs:275–293`). The latter's assertion body is otherwise unchanged. No new public type, error, wire field, lifecycle state, or behavior was added.

The production projection remains exact: `build_service_stream` filters by workload, records the latest `Failed`/`Terminated` observation with separate `reason`, `terminal`, and `detail` values (`streaming.rs:1097–1109`), then moves that `Option<LastAttemptWire>` into the helper when the clock reaches the deadline. The S-35 assertion continues to distinguish `Reconciler` reason from `StartupProbe` terminal and `None` detail. An authoritative terminal still bypasses the timeout helper and remains the stream verdict. The design clarification, independent review, and updated DISTILL oracle artifacts are committed beside the feature-delta pin.

### F-02 disposition — closed

The updated `AppState::streaming_cap` comment at `lib.rs:293–296` now says the value is a construction/test override and the floor for the Service progress deadline, states the 90-second production default, and states that no operator setting supplies it. `DEFAULT_STREAMING_CAP` and its production constructor use remain unchanged. The remediation adds no config key or setting.

### Verification, limits, and scope

The crafter reports **21/21 focused D10/helper controls passing**, with targeted formatting, Clippy, and `git show --check` passing. The reported focused run covers the existing D10 scenarios and helper controls; no tests or DES events were added in the remediation. I did not rerun cargo, Quint, or mutation testing. The original execution log retains the initial step's RED/GREEN/COMMIT history; the remediation's test counts are recorded here as crafter-reported evidence, not independent command results. Mutation testing remains deferred to the final DELIVER-wave gate. My independent `git show --check` for commits `63741cf9d` and `971560e02`, and `git diff --check` across the remediation range, passed.

The implementation commit preserves Marcus as Git author and carries exactly `Co-Authored-By: Codex <codex@openai.com>`, `Step-Id: 04-01`, and `Task-Id: service-allocation-readiness-and-ending`. The documentation correction commit has the same author and trailers. The two DISTILL artifacts that were untracked during iteration 1 were preserved and committed with the remediation. The current worktree has only this review artifact as untracked; this reviewer did not overwrite or discard any existing work.

| Finding | Iteration-2 disposition |
|---|---|
| F-01 — named public timeout helper signature was not pinned | **Closed.** D10-H1 pins the exact owned `Option<LastAttemptWire>` parameter, and production and cross-crate callers match it. |
| F-02 — stale `AppState::streaming_cap` documentation | **Closed.** Source documentation now states the exact construction/test override, production default, and no-operator-setting contract. |

**Final verdict: APPROVED — iteration 2.** Both iteration-1 findings are resolved within the accepted D10 scope. The stream still emits the approved Timeout with the latest observed attempt as labelled detail, preserves terminal/give-up ownership and Job behavior, and matches the newly pinned public helper signature. No unresolved typed-error or public-API contract gap remains for step 04-01. The step may advance to 04-02 only after this full Markdown review artifact is present on disk.
