# ADR-0242 — Backend unavailability is a typed outcome at the gateway's consuming port

## Status

Proposed — reconstructed on main 549d78fa6, pending independent review (2026-11-01).
Decision D2 approved by the user 2026-11-01 (see [feature-delta § Reconstruction: user decisions](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-reconstruction-user-decisions-and-proposals)).

## Context

A gateway request may arrive when the Service has no selectable healthy backend:
the Service is deploying, the backend has not yet passed readiness, or the VM is
stopping. The earlier draft of this design made the gateway publish demand for a
frontend, version that demand with a revision, require an acknowledgment from
the dataplane, and wake waiting requests. That draft decision was never
implemented and is not part of this design: VM backends reach the Service maps
through the ordinary `ServiceMapHydrator` path without any gateway input (D1,
verified on native metal kernel 7.0 in the
[D4 findings](../../feature/public-ingress-gateway/spike/findings-vm-service-map-programming.md)),
so there is nothing to wait for on the gateway side.

Observed on that substrate: `BACKEND_MAP.healthy` lags the readiness verdict by
about one second, and map entries outlive `job stop`.

## Decision

Backend availability becomes an outcome only at the gateway's own consuming
port, the upstream connector. A connect performs one selection and returns a
typed result: an absent Service entry, an empty slot or a missing backend is
`NoBackend` (public 503); a selected backend whose dial or mTLS then fails is a
connect or mTLS failure (public 502); the gateway never reports a wrong-peer
success. No demand, revision, acknowledgment or wake channel exists, the
connector adds no poll, sleep or retry, and it consumes no time budget beyond
the request's existing deadline. Backend absence never blocks the public
listener bind.

How a selected backend with `BACKEND_MAP.healthy == 0` (the readiness window)
is classified is **pending the user decision
[P-9](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-reconstruction-user-decisions-and-proposals)**:
honour the byte (`NoBackend`, 503) or ignore it (the dial then fails, 502). This
ADR holds under either option.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Gateway-published frontend demand with revision, acknowledgment and wake | Adds a second control channel between the gateway and the dataplane for a problem the ordinary hydrator path already solves; never implemented. |
| A bounded readiness wait inside the connector | Adds a time budget and a second path to the same outcome; the first request after readiness already succeeds, and clients can retry a 503. |

## Consequences

- 503 versus 502 is decided by one observable receipt, not by timing.
- The window between the startup verdict and the first readiness pass returns
  503 or 502 according to P-9; clients retry either way, and the design adds no
  wake to shorten it.
- A stopped backend still present in the maps selects, then is reset at the
  node-shared output chain: 502, never a wrong-peer success.
- Whether the registered selection arm reads `BACKEND_MAP.healthy`, which no
  current BPF program reads, is P-9; V-GW-3 observed the byte lag readiness and
  flap in the feature delta.

## Links

- [Consuming-port availability contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-consuming-port-availability-contract-d2)
- [Contract-shaping validation items](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-contract-shaping-validation-items)
- [ADR-0224](adr-0224-existing-cgroup-bpf-gateway-backend-selection.md), [ADR-0232](adr-0232-gateway-connect-selected-backend-receipt.md), [ADR-0230](adr-0230-gate-public-listener-without-redefining-workload-service-readiness.md), [ADR-0240](adr-0240-single-upstream-attempt-per-public-request.md)
- `kernel/bpf/cgroup.c:1664-1665 @ v7.0`: a program verdict other than 1 makes the `connect(2)` fail with `-EPERM`
