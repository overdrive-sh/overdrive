# E09-v2 disposal timeout final review

## Review metadata

- **Review:** bounded disposal wait-policy correction and diagnostic-history rule
- **Reviewer:** Codex, GPT-5.6 Luna, maximum thinking
- **Scope:** the `dispose_failed_service_attempt` poll bound and the new
  `Diagnostic history — append-only within a run` section in
  `.claude/rules/testing.md`
- **Prior review state:** observation capture, preservation regression, native
  evidence paths, F-01, and F-02 were previously reviewed; those findings were
  closed and were not re-audited here.
- **Authority boundary:** the user authorized the 60-to-180-second disposal
  correction. No other source, predicate, retry, timeout, budget, evidence,
  or product behavior was changed by this review.
- **Native execution:** no native run was performed. The retained failed
  capture was inspected mechanically only.

## Verdict

**APPROVED.** The failed-Service disposal poll bound is now 180 seconds, the
existing stop/query commands and terminal predicates remain intact, the
native-window replay turns green while the preservation regression remains
green, and the testing rule accurately defines append-only evidence without
claiming that sampled polling states are a complete internal transition
history.

## Focused source review

`dispose_failed_service_attempt` now sets
`local deadline=$((SECONDS + 180))` at
`examples/service-kind-vm-workloads-v2/run-example.sh:790`. Its stop command
remains bounded at 45 seconds and `query_describe` remains bounded at 10
seconds. The two existing success predicates are unchanged: a public `Failed`
state must retain the startup-probe failure, or a public `Terminated` state
must report `reason: stopped` while the original failure description retains
the startup-probe failure. The 60-second runtime-resource cleanup wait,
ordinary stop's 180-second poll, and the suite's 600-second build/trial
budget were not changed.

The new rule at `.claude/rules/testing.md:242-264` requires preservation of
the first failure; append-only command results and observed state updates in a
log or uniquely named files; run, case/allocation, attempt, phase, timestamps,
elapsed time, stdout/stderr, exit status, sampled states, and query errors;
separate attempt start/completion records for interruption; and preservation
of the original outcome after cleanup retries. It explicitly describes
polling records as observations and says they are not a complete history of
internal state transitions. The rule therefore matches the approved
instrumentation's public sampled-state boundary and does not overclaim
internal lifecycle visibility.

## Focused verification

The following host-safe checks passed:

```text
bash docs/analysis/test-e09-v2-disposal-observation-history.sh                       # exit 0
bash docs/analysis/test-e09-v2-disposal-observation-history.sh --replay-native-disposal-window # exit 0
bash -n examples/service-kind-vm-workloads-v2/run-example.sh docs/analysis/test-e09-v2-disposal-observation-history.sh # exit 0
shellcheck examples/service-kind-vm-workloads-v2/run-example.sh docs/analysis/test-e09-v2-disposal-observation-history.sh # exit 0
git diff --check -- .claude/rules/testing.md examples/service-kind-vm-workloads-v2/run-example.sh # exit 0
```

The preservation regression still reports that the first command, failed
query, Running sample, and timing records survive the cleanup retry. The
native-response replay now reports both paths accepted at the same logical
86-second final sample:

```text
ordinary_stop_control: final_sample_available_after_seconds=86 expected=accepted observed=accepted
disposal_window_regression: final_sample_available_after_seconds=86 expected=accepted observed=accepted
```

This is the expected result of the authorized one-line bound change. It does
not add a product ordering claim or alter the existing public-state
predicates.

## Retained native run status

The evidence was not treated as a satisfied native result. Mechanical parsing
of `tcp-truthfulness-20.tsv` found 20 rows: 15 `pass`/`complete` rows and 5
`not-run-cancelled` rows (trials 14, 16, 18, 19, and 20). The retained run log
records both cohorts and ends with `suite_failed=1 suite_cancelled=1`; its
inner metal-run report records exit status 124 after the remote 600-second
setup/trial window plus 60-second cleanup grace was exhausted.

The wrapper status is distinct: `product-run.meta` records the harness
`metal run` invocation from `21:26:13Z` to `21:37:04Z` with `exit: 1`, while
the inner command's timeout is recorded as 124 in `run.log`. `verification.yaml`
also correctly reports `execution_status: failed` and `runner_exit_code: "1"`.
The expectation README remains `pending`; no satisfaction claim was audited
or inferred. These files support reporting the native run as failed and
incomplete, not as evidence that all 20 cases passed.

## Findings and disposition

No reachable, in-scope defect remains in the two reviewed deltas. The
authorized disposal window correction is minimal, the observed replay now
passes, preservation remains green, and the append-only rule retains the
distinction between sampled public observations and unobserved internal
transitions.

**Final verdict: APPROVED.**
