# E12 — VM liveness restart is visible through workload describe

Status: `pending` (DISTILL handoff)
Surface: E — built-product end to end
Execution substrate: `native-metal`
Walking skeleton: no; bounded lifecycle journey

## Expectation

The checked-in workload starts healthy, then returns HTTP 503 from `/live`.
After the declared failure threshold, describe must show the liveness failure
and an allocation restart under the existing restart policy. The original
allocation's terminal state remains authoritative; the replacement follows
the ordinary VM Running/startup path and no VM-specific lifecycle state appears.

- Anchor: S-SVM-28 in `docs/feature/service-kind-vm-workloads/distill/test-scenarios.md`.
- Anchor: US-SVM-3 in `docs/feature/service-kind-vm-workloads/feature-delta.md`.
- Anchor: accepted lifecycle gate ownership in the architecture summary.

## Verification

The activated `liveness-restart` mode captures describe before failure, at the
liveness terminal decision, and after replacement. It records allocation IDs,
restart count, probe role/status, terminal reason, state sequence, response
bytes, and cleanup delta. Passing requires a visible liveness-caused restart,
no readiness-owned restart, no revival of the dead allocation, and zero leaks.
