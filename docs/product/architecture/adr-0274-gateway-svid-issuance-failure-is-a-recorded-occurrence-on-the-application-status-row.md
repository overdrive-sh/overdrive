# ADR-0274 — A failed gateway SVID issuance is a recorded occurrence on the Gateway Application status row

## Status

Proposed — pending independent review. User decision 2026-11-04 (R-45); provenance of the related rulings is in the feature delta.

## Context

`GatewaySvidLifecycle` reissues the gateway SVID until it holds one. The executor calls
`ca_issuance::issue_and_audit`, which fails when the CA cannot sign or when the `issued_certificates`
audit write fails. The failure's only sink is a tracing event. The appliance OS has no operator shell, so
tracing alone is not an operator surface. `GatewayIdentityStatus` is a current-state projection: a node that
has just started and a node whose CA has failed on every tick both read `Absent`. A convergent record cannot
answer "did it happen" (`.claude/rules/development.md`); the precedents are ADR-0078 (`LastTerminated` plus
`restart_count`) and `PublicCertifiedKeyStatusRowV1.last_install_failure`.

## Decision

Record each confirmed failed gateway SVID issuance as an occurrence on `GatewayApplicationStatusRowV1`: a
monotone failure count and a depth-1 snapshot of the latest failure (cause class `Ca` or `Audit`, and the
instant). The count is never cleared by a later success and survives a restart. The record is written from
the executor's confirmed `Err` after `issue_and_audit` returns, never from emit-time retry memory, and it
lives on the row beside, not inside, the current-state identity value. The Gateway Application owner remains
the row's sole writer; the status response carries both fields.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Keep tracing only | Not an operator surface on an appliance OS with no shell; "just started" and "CA failing every tick" stay indistinguishable. |
| Add a `Failed` variant to `GatewayIdentityStatus` | It is a current-state value, so it clears on the next success and cannot answer "did it fail". |
| A durable per-failure audit or pending-request list | Unbounded, and the audit write is the write that may be failing. |
| The executor writes the row directly | The owner is the row's sole writer; a second writer would race it under whole-row last-write-wins. |

## Consequences

- An operator reads the cause class, the count and the instant from `GET /v1/gateway/status`; error text
  never leaves the node.
- The record is written from the executor's confirmed error, not from the reconciler's emit-time retry memory,
  which is unchanged.
- Adds no gate and no modelled variable; the Quint specification is unchanged.

## Links

- [SVID issuance failure occurrence](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-svid-issuance-failure-occurrence) (contracts, rulings and their provenance)
- [ADR-0225](adr-0225-dedicated-gateway-svid-identity-lifecycle.md), [ADR-0227](adr-0227-redacted-operator-gateway-status.md), [ADR-0078](adr-0078-crash-and-recover-is-durably-observable-last-terminated-plus-restart-count.md)
- [ADR-0275](adr-0275-workload-svid-issuance-failure-is-a-recorded-occurrence-on-its-own-row.md) (the workload counterpart)
