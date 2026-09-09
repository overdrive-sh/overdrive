# E10 — HTTP startup status classes agree for Exec and VM Services

Status: `satisfied` (native capture and independent evidence audit complete; see [final audit](../../../docs/analysis/review-02-04-final-evidence.md))
Surface: E — built-product end to end
Execution substrate: `native-metal`
Walking skeleton: no; bounded cross-driver mechanic matrix

## Expectation

Drive the same checked-in HTTP server through eight specs: Exec and VM crossed
with 204, 302, 404, and 503. Both drivers must make 204 Stable and must reject
302/404/503 as startup failures carrying the numeric status. Redirects are not
followed. Both 503 cells return the nonempty bounded response-body sentinel
`SVM-E10-FAILURE-BODY-MUST-NOT-LEAK`; that deletion-sensitive fixture must
appear zero times in operator-visible output.

The cleanup trajectory is state-specific: a 204 Service is running when the
example issues its operator stop and must finish `Terminated`. For 302/404/503,
the deploy stream must first report the startup failure and the public describe
must show the failed probe. The example then records stop intent. The authored
failure may remain `Failed` with `StartupProbeFailed` while its owned runtime
resources are removed. If an already-queued replacement reached `Running`
before that intent was reconciled, its ordinary operator stop may finish
`Terminated`; that alternate row is accepted only with public evidence of a
replacement restart and an operator-attributed prior termination. A bare
crash-shaped `Terminated` row is not accepted. A VM replacement rejected
before creating a second VMM may instead leave a `Failed` row with the public
`bind beacon listener` start error; that exact typed rejection is recorded as a
separate trajectory, not silently treated as the original startup failure.

- Anchor: S-SVM-26 in `docs/feature/service-kind-vm-workloads/distill/test-scenarios.md`.
- Anchor: US-SVM-2 and K2 in `docs/feature/service-kind-vm-workloads/feature-delta.md`.
- Anchor: ADR-0090 explicit/default HTTP target semantics.

## Verification

The activated `http-status-cross-driver` mode runs all eight checked-in specs
against isolated built-product instances and emits an eight-cell ledger. Every
cell records driver, status, deploy exit, terminal state, observed cleanup
trajectory, rendered probe result, stdout/stderr/describe byte counts, sentinel
occurrence counts for each operator surface, and cleanup delta. Passing
requires the expected result and state-specific cleanup trajectory in all 8/8
cells, byte-equal status policy between the two drivers, and zero
occurrences of `SVM-E10-FAILURE-BODY-MUST-NOT-LEAK` in deploy stdout, deploy
stderr, workload describe output, and the rendered probe-result field. The
ledger itself records only those zero counts, never the response body.
