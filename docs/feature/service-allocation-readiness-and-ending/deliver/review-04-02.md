# DELIVER Review — Step 04-02: Stream reports recorded verdict before waiting

## Review metadata

| Field | Value |
|---|---|
| Feature | service-allocation-readiness-and-ending |
| Step | 04-02 — Stream reports recorded verdict before waiting |
| Iteration | 1 |
| Reviewer | Codex, fresh isolated DELIVER reviewer |
| Implementation commit | 47552b587637688e63b110d276812edd276b6124 |
| COMMIT audit commit | baaf5494c9abee8495a23d53a14987acaa403a09 |
| Comparison base | 9c02a5e9cccbb88301d21e000201a908bcf14031 (approved step 04-01 audit state) |
| Authoritative contract | Approved roadmap 04-02; feature-delta D11 and D11-E1; ADR-0263; ADR-0037 §4; architecture brief |
| Upstream prerequisite | Step 04-01 review-04-01.md is APPROVED; D11 and its exact D11-E1 stream-error contract are approved |
| Verdict history | Iteration 1: APPROVED |

## Executive summary

The implementation reads the Service workload’s observation rows after subscribing to lifecycle events, selects the workload’s current allocation through the existing reconciler helper, and projects that row’s terminal through the existing Service terminal projection. It only arms the D10 progress deadline after the read returns no projecting verdict. The Lagged branch repeats the same observation read; a failed read yields the exact D11-E1 typed error and closes the stream.

The accepted public shape is preserved: ServiceStreamError has exactly ObservationRead { source: ObservationStoreError } and LineEmission { source: std::io::Error }, and build_service_stream returns the approved Result<Bytes, ServiceStreamError> stream. No HTTP/NDJSON, persisted, CLI wire, handler-route, or new public API change was introduced. The module-private row projection filters by workload and uses current_alloc, which selects the numeric-maximum attempt suffix, rather than ordering by update stamp.

The implementation commit only adds the private row projection and the open-read / relist behavior in streaming.rs, removes the seven assigned pending markers from the Service acceptance test, and removes six assigned pending markers from the Quint conformance drivers. The test bodies and assertions are unchanged. Two named Quint controls that were already active, publish_after_relist_reaches_channel and superseded_attempt_terminal_is_not_reported, remain active.

Independent focused Metal runs passed: all 15 Service acceptance tests and all 8 stream_snapshot Quint tests. The worktree was clean before this review artifact was written. No implementation, test, DES, or commit changes were made by this reviewer.

## Contract and production-path review

### Stream order, current allocation, and projection

The order in streaming.rs:1089-1121 is Accepted, subscribe, read allocation rows, project the current row, then arm the deadline only if no verdict exists. That matches feature-delta D11 item 1 and ADR-0263. The receiver exists during the read, so a terminal committed after the read remains queued; a terminal committed before or during the read is visible in the row set under the approved commit-before-publish order.

service_event_from_rows at streaming.rs:1030-1038 filters rows to the requested WorkloadId, passes those rows to the existing overdrive_reconcilers::current_alloc, checks the selected row’s terminal, and delegates to service_event_from_terminal. current_alloc at workload_lifecycle.rs:1485-1491 uses the numeric maximum parsed attempt suffix, matching the D11 selection rule. The helper does not add or duplicate attempt-selection policy.

The projection passes the row’s alloc_id, terminal, detail, and None for last_exit_code. That matches the exact D11 clause requiring row detail and stating that the row’s stderr_tail field is not used. A missing row, a current row without a terminal, or a terminal whose existing projection is None waits. In particular, the existing service_event_from_terminal mapping at streaming.rs:906-908 keeps StartupProbe and LivenessProbe stops non-verdicts.

On RecvError::Lagged, streaming.rs:1186-1200 rereads the observation rows and returns the current attempt’s projecting terminal if present; otherwise the loop waits under the same pinned deadline. The deadline future is created once, after the successful open read found no verdict. An open-read failure returns before that future is created. A relist failure returns from the stream rather than restarting the deadline or synthesizing Timeout.

### Exact API and typed error contract

No new public type, variant, method, parameter, or wire shape was added in this step. ServiceStreamError was already present as the D11-E1 scaffold. Its two variants and build_service_stream item type match feature-delta § D11 interface changes and ADR-0263 exactly.

| Designed failure | Implemented mapping | Evidence |
|---|---|---|
| alloc_status_rows fails at stream open | ServiceStreamError::ObservationRead { source: original ObservationStoreError }; yielded once, then stream ends before waiting or arming Timeout | streaming.rs:1103-1109; service_stream_recorded_verdict.rs read-failure helper and open-read test |
| alloc_status_rows fails after Lagged | Same ObservationRead envelope with original source, yielded once, then stream ends | streaming.rs:1186-1192; service_stream_recorded_verdict.rs relist-failure test |
| ObservationStoreError::Unreachable { peer } | Original variant and peer payload remain in source | service_stream_recorded_verdict.rs: read_failure and assert_read_failure |
| ObservationStoreError::Io(error) | Original I/O error remains in source, including kind and message | service_stream_recorded_verdict.rs: assert_read_failure |
| ObservationStoreError::Envelope { source } | Original envelope variant and payload remain inspectable through the nested source | service_stream_recorded_verdict.rs: assert_read_failure |
| Accepted, recorded-terminal, live-terminal, or synthesized-line emission fails | ServiceStreamError::LineEmission { source: original std::io::Error }; yielded once, then stream ends | streaming.rs:1090-1095, 1111-1116, 1175-1183, 1194-1200, 1202-1219 |

The thiserror source attributes preserve the nested error chain. The focused read-failure acceptance cases assert the source variant/payload and that the next poll is Ended. They cover Unreachable, Io, and Envelope at both read sites. No error is converted to absence, encoded in a string, or mapped to an unapproved public variant. The local corrupt-row behavior remains governed by ADR-0048, as D11-E1 requires. No typed-error contract gap was found.

### Test integrity, scope, and external validity

The implementation diff for service_stream_recorded_verdict.rs consists only of removing seven pending #[ignore] attributes. The stream_snapshot.rs diff consists only of removing six pending #[ignore] attributes. No existing assertion, expected value, test body, test name, or setup was edited. The active tests retain their CONTRACT_SHAPE declarations; no source-local pure-function property was added or transitioned.

The Service acceptance target exercises both boundaries required by D11: S-39 uses the production submit_workload handler and composed reconcilers/shim, while S-40 through S-42 drive the production build_service_stream entry point with SimObservationStore and scripted lifecycle events. The Rust tests execute the in-process production composition and do not spawn the built Overdrive binary or run verification expectations.

The 15 focused Service acceptance tests cover recorded Stable, each give-up family, operator stop, current/superseded attempt selection, restartable probe stops, subscribe/read ordering, Lagged recovery, deadline interaction, line projection, and typed open/relist failures. The 8 focused stream_snapshot runs include the six D11 runs plus the randomized run and the two existing controls; the named superseded-attempt and publish-after-relist controls remain active.

No new unit/PBT methods were added, so the unit-test budget rule is not implicated. No fixture implements the production decision in place of build_service_stream. The D11-specific work remains within the requested bounded slice.

### DES, commits, and mechanical checks

The execution log records the actual step sequence and outcomes:

| Phase | Result | Recorded time (UTC) |
|---|---:|---|
| RED | FAIL | 2026-11-10 08:05:34 |
| GREEN | PASS | 2026-11-10 08:12:47 |
| COMMIT | PASS | 2026-11-10 08:15:35 |

The RED result is the expected semantic failure for the active stable-redeploy scenario, not a collection or import failure, per the crafter’s recorded evidence. The COMMIT result is persisted in audit commit baaf5494c9abee8495a23d53a14987acaa403a09.

The implementation commit changes four files: streaming.rs, the D11 Service acceptance file, the D11 stream_snapshot conformance file, and execution-log.json. It adds no handler, CLI, stored-format, config, or unrelated subsystem changes. The separate audit commit adds the COMMIT phase event only. Both commits preserve Marcus as Git author and carry exactly the required Codex co-author trailer, plus Step-Id and Task-Id. git show --check passed on both commits.

Independent verification run by this reviewer:

- cargo xtask metal run -- cargo nextest run -p overdrive-control-plane --features integration-tests --test acceptance -E 'test(service_stream_recorded_verdict::)' --no-fail-fast — 15 passed, 456 skipped.
- cargo xtask metal run -- cargo nextest run -p overdrive-control-plane --features integration-tests --lib -E 'test(quint_stream_snapshot_)' --no-fail-fast — 8 passed, 618 skipped.

The crafter reports strict Clippy and formatting checks passed via Metal; those results were not independently rerun in this review. Mutation testing was not run, as required for an individual DELIVER step; it remains the final DELIVER-wave gate. No nextest-affected hook run was attempted by this reviewer.

## Review dimensions and quality gates

| Dimension | Result |
|---|---|
| Exact D11 / D11-E1 API shape | Pass — exact public error variants and stream item type; no extra public surface |
| Current attempt and row projection | Pass — workload filtered first; existing numeric current_alloc and Service terminal projection reused |
| Subscribe-before-read / Lagged behavior | Pass — subscription precedes open read; Lagged relists; the fixed deadline is not rearmed |
| Typed error mapping | Pass — each designed read and line-emission failure maps to its approved typed variant and preserves its source |
| Acceptance completeness | Pass — all assigned tests active and focused 15-test Service target independently passed |
| Conformance | Pass — 8 stream_snapshot runs independently passed; existing controls stayed active |
| Test integrity | Pass — only the assigned pending markers were removed; bodies/assertions unchanged |
| External validity | Pass — composed S-39 goes through submit_workload; stream-focused scenarios call the production builder |
| TDD phase evidence | Pass — RED fail, GREEN pass, COMMIT pass recorded in order |
| Test budget | Pass — no unit/PBT tests added; acceptance and conformance obligations are not unit-test budget inflation |
| Commit scope and attribution | Pass — bounded four-file implementation commit and audit-only commit; required author/trailers preserved |
| RPP | L1-L2 scanned; no in-scope actionable refactoring finding identified |
| Mutation gate | Not run — correctly deferred to the final DELIVER-wave gate |

## Unproven hypothesis — not a finding

The review noticed a possible wire-equality edge for a crash with captured stderr, but did not promote it to a finding because the repository requires a bounded failing reproduction through the production owner path before accepting a defect.

The reachable source path appears to permit a prior Failed row whose reason is WorkloadCrashedImmediately with a nonempty stderr_tail, whose detail is None, and whose separate stderr_tail field is populated: worker/exit_observer.rs:502-542 writes that shape and patches the same tail into the typed reason. At the restart ceiling, workload_lifecycle.rs:1197-1245 can project a pre-startup crash to ServiceFailed { EarlyExit }. The Action::FinalizeFailed path carries the prior row’s reason, detail, and stderr_tail into the terminal row at action_shim/mod.rs:2021-2138; the observation occurrence carries reason/detail at observation_backend.rs:595-605 and action_shim publishes it at :2225-2231. The live Service branch then uses event.reason.WorkloadCrashedImmediately.stderr_tail at streaming.rs:1153-1166, while service_event_from_rows passes row.detail at :1034-1038. The S-41 acceptance helper at service_stream_recorded_verdict.rs:585-601 compares projections using event.detail, rather than driving the live branch with a nonempty crash tail.

This is not an accepted finding: no bounded regression was run that reaches the stream with this production-owned terminal row and demonstrates differing wire bytes. No implementation, test, or design change is requested on this basis. If a later investigation promotes it, first reproduce it through the production exit/finalization/stream path and record the result; only then decide whether the accepted D11 projection premise needs to return to DESIGN. This note does not block this review verdict.

## Remediation and verdict

There are no accepted findings and no remediation remains for step 04-02.

**Final verdict: APPROVED — iteration 1.** The implementation follows the pinned D11/D11-E1 contract for the reviewed paths, preserves the required error taxonomy and public API, activates only the assigned cases, and passes the independent focused Metal runs. Mutation testing remains deferred to the final wave gate. No implementation, test, DES, or commit changes were made by this reviewer; this review artifact is the only file written by the reviewer.
