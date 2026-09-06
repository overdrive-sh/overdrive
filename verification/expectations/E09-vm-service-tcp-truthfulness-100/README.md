# E09 — VM Service TCP success and failure stay truthful for 100 paired trials

Status: `pending` (DISTILL handoff)
Surface: E — built-product end to end
Execution substrate: `native-metal`
Walking skeleton: no; bounded KPI repetition of E08 and its failure control

## Expectation

Run 100 isolated paired trials through the built default-feature product. In
every trial, `service.toml` reaches Stable only after its guest TCP listener is
reachable, then the checked-in plaintext VM client Job succeeds only after its
guest-installed static binary receives `SVM-E08-GUEST-OK` from the Service
name/frontend.
`tcp-startup-failure.toml` never becomes Stable, exits nonzero with
`StartupProbeFailed`, identifies the guest TCP target and last connection
failure, and its negative-control VM peer Job never reaches the failed backend.

- Anchor: S-SVM-25 in `docs/feature/service-kind-vm-workloads/distill/test-scenarios.md`.
- Anchor: US-SVM-1 and K1 in `docs/feature/service-kind-vm-workloads/feature-delta.md`.
- Anchor: ADR-0090 registration-time VM target projection.

## Verification

The activated `tcp-truthfulness-100` mode must create a fresh isolated serve
data directory for each paired trial and write a 100-row result ledger. Each
row records both deploy exits, terminal states, target shown by describe,
both VM peer Jobs' public terminal results, and cleanup delta. The one
marker-owned preparation installs the same static client in the private guest
image used by every trial; no host Exec client is permitted.
The KPI passes only at exactly `100/100` truthful pairs; retries or discarded
trials are failures. No in-process test or crate import is allowed.
