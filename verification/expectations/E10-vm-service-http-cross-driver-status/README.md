# E10 — HTTP startup status classes agree for Exec and VM Services

Status: `pending` (DISTILL handoff)
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

- Anchor: S-SVM-26 in `docs/feature/service-kind-vm-workloads/distill/test-scenarios.md`.
- Anchor: US-SVM-2 and K2 in `docs/feature/service-kind-vm-workloads/feature-delta.md`.
- Anchor: ADR-0090 explicit/default HTTP target semantics.

## Verification

The activated `http-status-cross-driver` mode runs all eight checked-in specs
against isolated built-product instances and emits an eight-cell ledger. Every
cell records driver, status, deploy exit, terminal state, rendered probe
result, stdout/stderr/describe byte counts, sentinel occurrence counts for
each operator surface, and cleanup delta. Passing requires the expected result
in all 8/8 cells, byte-equal status policy between the two drivers, and zero
occurrences of `SVM-E10-FAILURE-BODY-MUST-NOT-LEAK` in deploy stdout, deploy
stderr, workload describe output, and the rendered probe-result field. The
ledger itself records only those zero counts, never the response body.
