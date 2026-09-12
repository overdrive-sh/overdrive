# E10 — HTTP startup status classes agree for Exec and VM Services

Status: `pending` (the corrected incarnation-aware contract requires a fresh
native capture and independent evidence audit)
Surface: E — built-product end to end
Execution substrate: `native-metal`
Walking skeleton: no; bounded cross-driver mechanic matrix
Contract shape: `bounded-change` (current allocation state plus the retained
depth-one failure snapshot, restart count, probe result, and owned-resource complement)

## Expectation

Drive the same checked-in HTTP server through eight specs: Exec and VM crossed
with 204, 302, 404, and 503. Both drivers must make 204 Stable and must reject
302/404/503 as startup failures carrying the numeric status. Redirects are not
followed. Both 503 cells return the nonempty bounded response-body sentinel
`SVM-E10-FAILURE-BODY-MUST-NOT-LEAK`; that deletion-sensitive fixture must
appear zero times in operator-visible output.

The cleanup trajectory is incarnation-aware. A 204 Service is `Running` when
the example issues its operator stop and must finish `Terminated`. For
302/404/503, the nonzero deploy stream is the occurrence surface for the typed
startup failure and the public describe retains the exact failed probe. Run
intent is still present, so the existing WorkloadLifecycle policy authorizes a
same-allocation recovery. The example polls through the old incarnation's
transient terminal row until public describe shows that same allocation
`Running` with a positive restart count, a depth-one prior `Failed` snapshot,
and the failed startup probe. A transient `Failed` or `Terminated` row is not a
settled no-recovery result and does not authorize a blanket terminal union.

Only after that recovered `Running` incarnation is observable does the example
record operator stop intent. Its current state must become `Terminated` with a
stopped reason, while the allocation ID, restart count, prior `Failed` snapshot,
and failed probe remain unchanged in a second observation after the owned
runtime resources are gone. The accepted current session may advance the
stopped observation during teardown; a stale earlier session must not replace
the settled result with a crash-shaped observation.

- Anchor: S-SVM-26 in `docs/feature/service-kind-vm-workloads/distill/test-scenarios.md`.
- Anchor: US-SVM-2 and K2 in `docs/feature/service-kind-vm-workloads/feature-delta.md`.
- Anchor: ADR-0090 explicit/default HTTP target semantics.
- Anchor: ADR-0078 depth-one `last_terminated` plus monotone `restart_count` occurrence semantics.
- Anchor: ADR-0099 accepted Running publication before restart release.
- Anchor: ADR-0100 accepted-session ownership and stale-watcher refusal.

## Verification

The activated `http-status-cross-driver` mode runs all eight checked-in specs
against isolated built-product instances and emits an eight-cell ledger. Every
cell records driver, status, allocation ID, state and restart count before and
after stop, the failure surface, a post-cleanup settled state, the exact
trajectory, rendered probe result, stdout/stderr/describe byte counts, sentinel
occurrence counts for each operator surface, and cleanup delta. Passing
requires the recovered-current/prior-failure pairing above in every failing
cell and the healthy first-incarnation pairing in both 204 cells, the same HTTP
status policy for both drivers, and zero
occurrences of `SVM-E10-FAILURE-BODY-MUST-NOT-LEAK` in deploy stdout, deploy
stderr, workload describe output, and the rendered probe-result field. The
ledger itself records only those zero counts, never the response body.
