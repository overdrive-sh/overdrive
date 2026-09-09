# E09-v2 disposal observation-history review

## Review metadata

- **Review:** E09-v2 append-only disposal observation instrumentation
- **Iteration:** 1
- **Reviewer:** Codex, GPT-5.6 Luna, maximum thinking
- **Baseline:** working tree at review time; pre-existing dirty work preserved
- **Scope:** `examples/service-kind-vm-workloads-v2/run-example.sh`,
  `docs/analysis/test-e09-v2-disposal-observation-history.sh`, and the new
  native evidence cited by `docs/analysis/root-cause-analysis-e09-v2-failure-case-lifecycle.md`
- **Authority boundary:** observation capture and host-safe replay only. The
  previously accepted barrier, failed-service predicate, and 180-second
  ordinary-stop changes were not reopened. No production wait-policy,
  predicate, API, design, timeout, or behavior change was made.
- **Native execution:** no native rerun was performed. The retained native
  evidence was inspected read-only.

## Iteration-1 verdict

**CHANGES REQUESTED.** The captured native facts and the host-safe regression
supported the stated narrow diagnosis, but the RCA contained two inaccurate
exact line citations for the retained native evidence. The observation-history
implementation itself did not show a reachable behavioral regression. The
citation correction did not authorize a product fix. The final disposition
after remediation is recorded below.

## Scope and design compliance

The new capture is append-only at the command boundary. `observe_command`
creates a unique capture directory, records the command and begin metadata,
preserves the existing scratch output and `.rc` files, runs the same bounded
command, records its exit and elapsed time, and retains the complete combined
stdout/stderr. Successful `describe` commands add the sampled public state;
failed queries retain their output and exit without fabricating a state.
`observe_stop_attempt` adds a per-attempt directory and begin/end metadata
around the existing stop/disposal helper. `print_case_transcript` enumerates
all observation files, including both the initial `main` attempt and the
`exit-cleanup` retry.

The dynamic Bash scoping of `STOP_OBSERVATION_ATTEMPT` and
`OBSERVATION_CONTEXT` reaches the existing command helpers without changing
their arguments or owner path. The existing 10-second describe bound, the
180-second ordinary stop bound, and the 60-second failed-service disposal
bound remain unchanged. The cleanup trap and cancellation path were not
replaced with a detached task or a second command. A command that is
interrupted can therefore retain begin metadata and partial output without an
invented completion record, as documented by the RCA.

The diagnostic script has `CONTRACT_SHAPE: bounded-change` declarations for
both the replay control and the cleanup-preservation regression. It invokes
the existing shell helpers with fake command responses; it does not invoke
Cargo, an Overdrive test binary, or a production VM. The replay intentionally
remains red while the 60-second disposal policy is unimplemented.

## Verification evidence

### Host-safe preservation regression

`bash docs/analysis/test-e09-v2-disposal-observation-history.sh` passed. It
proved that the first stop command, a failed query, all Running samples, and
their metadata remain byte-for-byte available after the real `worker_cleanup`
retry observes Terminated. The transcript also retained the main and
`exit-cleanup` contexts and the distinct exit values.

`bash -n examples/service-kind-vm-workloads-v2/run-example.sh
docs/analysis/test-e09-v2-disposal-observation-history.sh` passed.

The focused replay
`bash docs/analysis/test-e09-v2-disposal-observation-history.sh
--replay-native-disposal-window` exited 1 with the expected result:

```text
ordinary_stop_control: final_sample_available_after_seconds=86 expected=accepted observed=accepted
disposal_window_regression: final_sample_available_after_seconds=86 expected=accepted observed=rejected
```

This reproduces the runner-window mismatch from the retained native responses:
the ordinary stop helper accepts the final sample after 86 logical seconds,
while failed-service disposal still rejects it at the 60-second bound. It is
evidence for the proposed smallest policy correction (60 to 180 seconds),
which remains unimplemented. It is not evidence of a product ordering defect,
and no seeded real-owner `overdrive-sim` invariant was supplied or claimed.

### Retained native evidence

The evidence directory contains `verification.yaml`, `product-run.meta`,
`run.log`, and `product-run.out`. Read-only inspection confirmed:

| Fact | Retained value | Result |
| --- | --- | --- |
| Seed / substrate | seed 1, `native-metal`, `executed_in_lima: false` | confirmed |
| Run window | `2026-09-08T20:53:26Z` to `2026-09-08T20:59:16Z` | confirmed |
| Runner result | exit 1, `execution_status: failed` | confirmed |
| Source identity | `b653e1ad1758d11be33be457849b284f78141333` | matches current `HEAD` |
| Dirty-source status | dirty, with the diagnostic changes listed | confirmed |
| Command/attempt metadata | 1,642 sections | independently counted |
| c008 original disposal | stop exit 0; 53 Running describes; 60,786 ms; exit 1 | confirmed |
| c009 original disposal | stop exit 0; 53 Running describes; 60,855 ms; exit 1 | confirmed |
| c010 original disposal | stop exit 0; 53 Running describes; 60,862 ms; exit 1 | confirmed |
| Cleanup retry | reaches sampled Terminated/stopped state | confirmed |
| Full feasibility | 600-second setup/trial plus 60-second cleanup | remains pending/unproven |

The exact c008, c009, and c010 metadata paths cited by the RCA for the
original stop, last Running query, disposal attempt, cleanup attempt, and
cleanup Terminated query were present and matched their stated exits, states,
and timings. The typed `StartupProbeFailed` deployment evidence ranges were
also present. The retained ledger marks c008-c010 as failed at the disposal
stop and later cases as not-run/cancelled; it does not claim a passing native
run.

The dirty evidence manifest and patch show that the native run included the
append-only observation code. The evidence is therefore fresh for the stated
seed and source state; it is not a fabricated capture. This review does not
infer behavior beyond the recorded public command/query observations or claim
to observe every internal transition.

### Focused static check

`git diff --check` passed for the reviewed diagnostic changes. Plain
`shellcheck` passed for `run-example.sh`, but the combined command returned
nonzero for the new diagnostic script because it reports informational
`SC2329`, `SC2030`, and `SC2031` diagnostics at the intentionally indirect
adapter functions and logical `SECONDS` clock. This is recorded as F-02 below;
it did not alter the runtime result of the host-safe checks.

## Findings

### F-01 — RCA line references do not identify the cited native records

- **Severity:** blocking documentation accuracy defect
- **Reachability/evidence:** The newest RCA section states at
  `root-cause-analysis-e09-v2-failure-case-lifecycle.md:785-787` that
  `run.log:11` contains the exclusive lease token and `run.log:18` contains
  the canonical fail-closed native preflight. Reproduction:

  ```sh
  nl -ba verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/evidence/run.log | sed -n '7,22p'
  ```

  The output shows the lease heading at line 11 and the token at line 12;
  line 18 is blank and the preflight marker is at line 19. The evidence files
  themselves are valid, but these exact citations do not point to the records
  they claim to identify.
- **Required disposition:** Correct those two RCA references to the exact
  retained lines (token line 12 and preflight line 19), or cite the heading
  and payload explicitly. This is a documentation-only remediation. Do not
  change the command, timeout, predicate, native evidence, or product code.
- **Iteration 1 disposition:** Open; no remediation was performed because this
  review was authorized to write only this artifact.

### F-02 — New diagnostic script is not clean under plain ShellCheck

- **Severity:** medium mechanical check defect
- **Reachability/evidence:** Reproduction:

  ```sh
  shellcheck examples/service-kind-vm-workloads-v2/run-example.sh \
    docs/analysis/test-e09-v2-disposal-observation-history.sh
  ```

  `run-example.sh` is clean, while the new script exits nonzero with
  informational `SC2329` reports for dynamically invoked adapter functions
  and `SC2030`/`SC2031` reports for the deliberate logical `SECONDS` clock at
  the replay and preservation adapters. The warnings describe intentional
  test seams, but the checked-in script does not declare that intent to the
  static checker.
- **Required disposition:** Add narrow local ShellCheck annotations or an
  equivalent mechanically clear formulation for those intentional adapters.
  Keep the replay inputs, helper entry points, command bounds, and assertions
  unchanged. No production or policy change is required.
- **Iteration 1 disposition:** Open; no remediation was performed because this
  review was authorized to write only this artifact.

## Remediation and iteration record

| Iteration | Review activity | Findings | Disposition | Verdict |
| --- | --- | --- | --- | --- |
| 1 | Static audit, host-safe preservation regression, expected-red native-response replay, retained native evidence/path verification | F-01, F-02 | Both remain open; no code or RCA edits made by reviewer | CHANGES REQUESTED |

At the time of the iteration-1 entry above, no second iteration had occurred;
approval required a follow-up review after the two narrow
documentation/static-check dispositions were recorded. The native runner
remains failed/pending, the 600+60 feasibility question remains unproven, and
the proposed 60-to-180 disposal wait correction remains a design proposal
rather than an implemented behavior change.

## Iteration 2 re-review

The original author remediated F-01 and F-02 within the requested bounds. No
production helper, assertion, timeout, predicate, native evidence, API, or
design behavior changed.

### F-01 disposition — closed

The RCA now identifies the lease token at `run.log:12` and the fail-closed
native preflight at line 19. The retained evidence was checked directly:

```sh
sed -n '12p' verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/evidence/run.log
sed -n '19p' verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/evidence/run.log
```

The first line contains the recorded lease token and the second contains the
preflight marker. The correction is limited to the RCA citation, so F-01 is
closed.

### F-02 disposition — closed

The diagnostic script now has narrow local ShellCheck annotations immediately
beside the intentionally indirect adapter functions and logical `SECONDS`
clock. The replay and preservation helper bodies and assertions remain the
same. The focused checks now pass:

```text
bash -n examples/service-kind-vm-workloads-v2/run-example.sh docs/analysis/test-e09-v2-disposal-observation-history.sh   # exit 0
shellcheck examples/service-kind-vm-workloads-v2/run-example.sh docs/analysis/test-e09-v2-disposal-observation-history.sh # exit 0
git diff --check -- examples/service-kind-vm-workloads-v2/run-example.sh                                                        # exit 0
```

The narrow annotations resolve the mechanical lint finding without weakening
the regression or changing any command, cancellation, or budget behavior, so
F-02 is closed.

### Regression and evidence re-check

`bash docs/analysis/test-e09-v2-disposal-observation-history.sh` passed again.
The expected-red replay was also rerun without a native execution:

```text
ordinary_stop_control: final_sample_available_after_seconds=86 expected=accepted observed=accepted
disposal_window_regression: final_sample_available_after_seconds=86 expected=accepted observed=rejected
replay_exit=1
```

The red disposal result is preserved and remains the evidence for the
unimplemented 60-to-180-second proposal. It does not establish a product
ordering defect. The prior native evidence validation remains unchanged:
seed 1 native-metal, 20:53:26Z–20:59:16Z, exit 1, dirty source, 1,642
metadata sections, c008-c010 with 53 Running samples before each 60.8-second
disposal expiry, and cleanup reaching Terminated. Full 600+60 feasibility
remains pending/unproven. No native rerun or mutation test was performed.

## Final disposition

| Iteration | Findings | Disposition | Verdict |
| --- | --- | --- | --- |
| 1 | F-01, F-02 | Open | CHANGES REQUESTED |
| 2 | F-01, F-02 | Both closed by the original author; focused checks pass | APPROVED |

**Final verdict: APPROVED.** The append-only observation instrumentation and
its preservation regression are accepted for the bounded investigation. The
captured facts justify the runner timeout diagnosis and the narrow proposed
wait-policy correction, while the product correction itself remains outside
this review and unimplemented.
