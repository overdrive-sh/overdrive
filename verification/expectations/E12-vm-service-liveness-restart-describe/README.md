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
The ledger records the same allocation identity before, at, and after the
restart, the liveness response bytes, the probe role/status, and the terminal
attribution derived from the observed liveness-failure transition. The public
describe transcript renders the prior terminal observation as the generic
`stopped` reason; it does not claim a liveness-specific reason string.

- Anchor: S-SVM-28 in `docs/feature/service-kind-vm-workloads/distill/test-scenarios.md`.
- Anchor: US-SVM-3 in `docs/feature/service-kind-vm-workloads/feature-delta.md`.
- Anchor: accepted lifecycle gate ownership in the architecture summary.

## Verification

The activated `liveness-restart` mode drives the checked-in example through the
built default-feature binary on native metal. It captures describe before
failure, at the liveness terminal decision, and after replacement. It records
allocation IDs, restart count, probe role/status, terminal attribution, state
sequence, response bytes, and cleanup delta. The replacement's liveness result
may already be failed when its ordinary startup completes because the checked-
in guest is intentionally configured to fail `/live` after a bounded delay;
startup and readiness must still pass. Passing requires a visible
liveness-caused restart, no readiness-owned restart, no revival of the dead
allocation, and zero leaks. `runner.sh` parses the executed ledger and public
describe transcript; it does not invoke Rust tests or link an Overdrive crate.
