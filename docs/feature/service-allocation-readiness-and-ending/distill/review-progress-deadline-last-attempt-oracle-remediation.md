# Independent DISTILL review — D10 last-attempt reason oracle

## Metadata and verdict

| Item | Value |
|---|---|
| Scope | One S-35 acceptance assertion in `service_stream_progress_deadline.rs::a_never_ready_service_times_out_at_the_progress_deadline_naming_the_last_attempt` |
| Authority | Accepted D10 `LastAttemptWire` projection of the latest attempt-ending `LifecycleEvent`; D7 startup-stop terminal and unchanged action-shim stop reason |
| Protected work | Original step 04-01 crafter's D10 implementation, activated tests, DES phases and eventual commit |
| Verdict | **APPROVED** for the bounded typed oracle correction; D10 GREEN remains for the original crafter to restore and prove |

## Exact contract and production path

D10 item 4 pins `LastAttemptWire { alloc_id, reason, terminal, detail }` as four separate fields taken from the latest attempt-ending `LifecycleEvent` that the open Service stream observed (`feature-delta.md:769–780,813–818`). The synthesized timeout remains `ServiceSubmitEvent::Failed { alloc_id: None, reason: Timeout { after_seconds }, stderr_tail: None }`; `last_attempt` is labelled detail, not an authoritative give-up verdict. D10 does not authorize deriving `TransitionReason` from `TerminalCondition` or assigning a new reason class.

For this never-ready Service, the production `ServiceLifecycle` gate returns `Action::StopAllocation { terminal: Some(Stopped { by: StartupProbe }) }` under D7. The action shim's StopAllocation arm writes a `Terminated` row with `reason: Some(TransitionReason::Stopped { by: Reconciler })`, `detail: None`, and the action's unchanged `Stopped { by: StartupProbe }` terminal (`action_shim/mod.rs:3332–3398`). `lifecycle_event_from_occurrence` copies the accepted occurrence's reason, detail and terminal into separate `LifecycleEvent` fields (`:500–539`). The correct D10 stream projection is therefore **reason = Reconciler; terminal = StartupProbe; detail = None**. The old test expected StartupProbe in both reason and terminal, which would require an unapproved terminal-to-reason mapping and would reject the existing production event.

## No-weakening audit

The amended test changes only the typed `last_attempt.reason` assertion to `TransitionReason::Stopped { by: Reconciler }` and asserts `detail == None` (`service_stream_progress_deadline.rs:120–140`). It retains the first attempt's exact allocation id, `last_attempt.terminal == Some(Stopped { by: StartupProbe })`, the 90-second clock boundary, the synthesized `Timeout` with no verdict allocation or stderr, and the before/after allocation-row equality proving the stream wrote no row. The separate typed terminal still identifies the startup probe as the stop initiator; no string classification or cause erasure was introduced. The `/// CONTRACT_SHAPE: bounded-change.` declaration remains. Other active S-35 bodies and the Job-stream control are untouched by DISTILL; removed `#[ignore]` markers in the diff belong to the original crafter's activation.

## Verification and limit

The original crafter reported 17/18 activated tests with this old reason assertion failing while its partial D10 implementation was present, then reverted that implementation before this remediation. The former GREEN trace log is not retained in `.context`; this review does **not** claim to have reproduced the corrected reason/terminal pair at runtime. Its expected mapping is established by the accepted D10 contract and the current production event writer above. The original crafter must restore the D10 implementation and make this exact assertion GREEN.

I independently ran the focused acceptance selection through `cargo xtask metal run -- cargo nextest run -p overdrive-control-plane --features integration-tests --test acceptance --no-fail-fast -E 'test(a_never_ready_service_times_out_at_the_progress_deadline_naming_the_last_attempt) | test(a_job_stream_keeps_the_streaming_cap_and_its_timeout_line)'` (run ID `57672862aa6223d1a827ab92`). It compiled and returned **one pass, one expected RED**: the Job-stream control passed; the never-ready Service printed seed `3502` and failed at the missing `last_attempt` before reaching the corrected field assertion, consistent with reverted D10 production. The author reports changed-file `rustfmt --check --edition 2024`, Contract Shape and `git diff --check` passing; my read-only `git diff --check` passed. No mutation or broader suite was run for this review.

**Verdict: APPROVED for this bounded DISTILL oracle correction.** The test now asserts the exact reason/terminal/detail triplet the production `LifecycleEvent` carries and retains the deadline, verdict and no-row-write obligations. This approval neither changes production behavior nor claims step 04-01 GREEN; the original crafter owns restoration, verification, DES and commit, followed by independent DELIVER review.
