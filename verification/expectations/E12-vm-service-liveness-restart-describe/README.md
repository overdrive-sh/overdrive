# E12 — VM liveness restart is visible through workload describe

Status: `pending` (DISTILL handoff)
Surface: E — built-product end to end
Execution substrate: `native-metal`
Walking skeleton: no; bounded lifecycle journey

## Expectation

The checked-in workload starts healthy, then returns HTTP 503 from `/live`.
After the declared failure threshold, describe must publish a terminal
allocation row before the existing WorkloadLifecycle restart policy produces
an ordinary replacement. The replacement follows the ordinary VM
Running/startup path and no VM-specific lifecycle state appears. The ledger
records the same allocation identity before, at, and after the restart, the
liveness response bytes, the probe role/status, the observed failure count,
and the allocation `Since` and startup observation timestamps. Replacement
startup is accepted only when both timestamps differ from the pre-restart
observation. The terminal attribution is derived from the observed liveness
threshold transition; the public describe transcript renders its prior
terminal observation as the generic `stopped` reason and does not claim a
liveness-specific reason string.

- Anchor: S-SVM-28 in `docs/feature/service-kind-vm-workloads/distill/test-scenarios.md`.
- Anchor: US-SVM-3 in `docs/feature/service-kind-vm-workloads/feature-delta.md`.
- Anchor: accepted lifecycle gate ownership in the architecture summary.

## Verification

The activated `liveness-restart` mode drives the checked-in example through the
built default-feature binary on native metal. It captures describe before
failure, after the declared number of liveness failures when the allocation is
terminal, and after replacement. It records allocation IDs, restart count,
probe role/status, terminal attribution, state sequence, response bytes,
startup observations, allocation `Since` values, the threshold witness, and
cleanup delta. The replacement's liveness result may already be failed when
its ordinary startup completes because the checked-in guest is intentionally
configured to fail `/live` after a bounded delay; startup and readiness must
still pass. Passing requires a visible terminal liveness stop at or beyond the
declared threshold, a fresh ordinary same-ID replacement, no readiness-owned
restart, no revival of the dead allocation, and zero leaks. `runner.sh` parses
the executed ledger and public describe transcript; it does not invoke Rust
tests or link an Overdrive crate.
