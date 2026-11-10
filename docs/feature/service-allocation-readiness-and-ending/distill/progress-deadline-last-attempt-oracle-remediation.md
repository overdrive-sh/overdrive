# Step 04-01 DISTILL remediation: progress-deadline last-attempt oracle

## Contract and owner path

D10 item 4 in the accepted feature delta defines `LastAttemptWire` as a projection of the latest attempt-ending `LifecycleEvent` seen by the open Service stream: `reason` is the event's `TransitionReason`, `terminal` is the event's separate `TerminalCondition`, and `detail` is the event's optional detail. The timeout is a synthesised `Failed { alloc_id: None, reason: Timeout { after_seconds }, stderr_tail: None }`; the last attempt is labelled detail, not the verdict. D10 specifies no translation from `terminal` into `reason`.

The never-ready scenario's startup gate returns `Action::StopAllocation { terminal: Some(Stopped { by: StartupProbe }) }` (`service_lifecycle.rs:1183-1205`). The production `StopAllocation` arm writes the resulting `Terminated` row with **`reason: Some(TransitionReason::Stopped { by: Reconciler })`**, `detail: None`, and the action's unchanged StartupProbe terminal (`action_shim/mod.rs:3363-3391`). `lifecycle_event_from_occurrence` copies `reason`, `detail`, and `terminal` into separate `LifecycleEvent` fields (`action_shim/mod.rs:512-539`). A D10 stream projection of that event therefore has Reconciler as its reason and StartupProbe as its terminal. The old test expected StartupProbe in *both* fields, which would require an unapproved terminal-derived reason policy.

## Bounded oracle correction

Only `a_never_ready_service_times_out_at_the_progress_deadline_naming_the_last_attempt` in `service_stream_progress_deadline.rs` changed. It now asserts `last_attempt.reason == Stopped { by: Reconciler }`, retains `last_attempt.terminal == Stopped { by: StartupProbe }`, and checks `last_attempt.detail == None` from the same stop event. Its 90 s deadline, synthesised timeout reason, absent verdict allocation and stderr, exact first-attempt allocation id, and no-row-write complement remain unchanged. The per-test `/// CONTRACT_SHAPE: bounded-change.` declaration and the crafter's active test markers remain intact.

## Evidence and handoff

The original step 04-01 crafter reported 17/18 activated tests passing with this reason mismatch while its partial D10 implementation was present. That implementation was reverted before this remediation, and its former GREEN trace log was not present in `.context` at this handoff; this note does not claim to have replayed it. With the current reverted production, the focused Metal test fails earlier because `last_attempt` is still `None` (seed `3502`, exit 100), both before and after the oracle edit. That is the expected D10 semantic RED until the original crafter restores the implementation. The Job-stream cap control passed in the after-edit focused run.

| Check | Result |
| --- | --- |
| Metal focused never-ready timeout before and after the oracle edit | Both compile and fail at the missing `last_attempt`, seed `3502`, exit 100 under reverted D10 production. |
| Metal after-edit Job stream cap control | Passed, 1/1; Job behavior remains unchanged. |
| Changed-file `rustfmt --check --edition 2024`, `git diff --check`, Contract Shape audit | Passed. |

No production code, model, DES event, step activation, or commit changed in this remediation. Independent DISTILL review is the next gate; the original step 04-01 crafter then restores its D10 implementation and reruns GREEN, including this exact reason/terminal assertion.
