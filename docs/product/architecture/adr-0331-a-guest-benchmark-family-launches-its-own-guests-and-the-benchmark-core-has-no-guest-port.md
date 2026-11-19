# ADR-0331 — A guest benchmark's family launches its own guests, and the benchmark core has no guest port

## Status

**Accepted (2026-11-19).** Approved by the user as PD-7, with the shape PD-4 option (b) leaves it. DDD-7 in the [feature delta](../../feature/guest-benchmark-substrate/feature-delta.md).

## Context

Launching a guest is the guest kit's job (`overdrive-testing`: `GuestKit`, `guest_lane`). `overdrive-bench` depends on `overdrive-testing` only as a dev-dependency, and the family crate (`overdrive-bench-families`) already depends on it. A spike crate must be able to declare a guest benchmark by calling `run_main` as it does today (`spike.md`: a spike crate depends on workspace crates by path and adds nothing under `crates/`). Because no guest boots before the replicates (ADR-0328), the runner needs nothing from a guest before it writes `run_started`; what it learns from a guest arrives through the replicate (ADR-0326).

## Decision

The family's host half boots each replicate's guest through the guest kit and passes what the guest reported to the runner through `Replicate` (`report_guest_view`, `retain_console`). `overdrive-bench` has no guest port, no guest adapter and no new dependency: it gains only the guest record vocabulary (the `guest` substrate, the `guest` block, the `guest_observed` line, `GuestView`) and the two `Replicate` methods. `overdrive-bench` stays dev-only; `run_main` and every spike `main` are unchanged. A spike crate that declares a guest benchmark depends on `overdrive-testing` by path to launch its guest, as the maintained family does.

## Alternatives

- **A crate-internal `GuestSubstrate` port in `overdrive-bench` with a production adapter `KitGuestSubstrate`** (it would exist only to run a probe guest): needs `overdrive-testing` as a normal dependency of the benchmark core, so `bench-report` would compile the guest kit; and a port with no method the runner calls. Rejected.
- **The adapter in `overdrive-bench-families`, passed through a new `run_main` variant**: a spike crate would depend on the families crate (the control plane, worker and dataplane closure) and every spike `main` would need the new call. Rejected.
- **The port in `overdrive-testing`**: `overdrive-testing` would depend on `overdrive-bench`, closing a dependency cycle with the existing dev-dependency. Rejected.

## Consequences

ADR-0296's dependency list is unchanged. `bench-report` and the runner do not compile the guest kit. The runner's guest behaviour (writing the `guest_observed` line once, failing a replicate that reports no view) is unit-tested with replicate bodies that report scripted views; the family's launch path is covered by the retained run and the smoke test. `HostValue` becomes `pub` (re-exported) so a family can build a `GuestView`.

## Links

- [feature delta, DDD-7, Components and ports](../../feature/guest-benchmark-substrate/feature-delta.md)
- [ADR-0328](adr-0328-a-guest-benchmark-refuses-the-host-before-it-boots-a-guest-and-takes-the-guests-view-from-its-replicates.md)
