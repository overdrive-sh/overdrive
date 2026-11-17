# ADR-0306 — VM lifecycle stages are observed through existing production seams

## Status

**Accepted.** Approved by the user 2026-11-17; independent design review resolved 2026-11-17. DDD-11 in the feature delta.

## Context

O3: each lifecycle stage needs an observable boundary on a production surface. The
code was read for every stage. Production already exposes: named tracing events
(`convergence.evaluation.admitted`, `vm.lifecycle.create_enter`, `.created`,
`.ready`, `.stop_enter`, `.cleanup_calls_finished`, `vmm.process.reaped`), the
`BeaconIo` and `Vmm` traits as public adapter fields of `NodeAdapters`, the
per-VM console log file at `VmConfig::run_dir.console_log()`, and
`ObservationStore::subscribe_all_events`. Missing are only boundaries no one
needs: Cloud Hypervisor is launched with its whole configuration on the command
line and has no separate "API up" step. Cloud Hypervisor writes the guest serial
to that file (`--serial file=<run_dir>/console.log --console off`), and the
`VmmDiagnostics` handle holds only the VMM's stderr pipe, so the guest's lines
are never in `console_tail()` (spike V-4: empty in 60 of 60 cycles).

## Decision

No production interface is added for the VM-lifecycle family. The family observes
the stages through the existing seams: a timestamping `BeaconIo` decorator (exact
receipt of `READY` and `EXIT`), a `Vmm` decorator, a tracing layer for the named
events, and a subscription to the observation store for the Stable row. The stage
table in the feature delta names, for each stage, its start and end boundary and
its instrument. Guest-measured durations (the `overdrive-init: <stage>
elapsed_ms=<n>` lines) are read from the console log file at
`VmConfig::run_dir.console_log()`, by the `Vmm` decorator when the exit watch
fires and before it forwards the exit, because cleanup removes the run directory
afterwards. Spike V-4 found both shutdown lines in 60 of 60 cycles this way and no
duration of 0 or 1 ms. The resolution is 1 ms, and a trivially stoppable guest
reports a floor of 11 to 13 ms, so the row reports a runtime-independent floor for
such a guest.

## Alternatives

- **Add a typed stage-event port to the driver**: the named events are already
  pinned by ADR-0102/0103 and by capture-layer tests; a second contract adds
  surface for no new boundary; rejected.
- **Read `VmmDiagnostics::console_tail()`**: the handle never holds the guest
  serial (it is a file, not the stderr pipe); V-4 measured it empty in 60 of 60
  cycles; rejected.
- **A finer guest diagnostic and a production change in `overdrive-init`**: not
  needed, since 1 ms resolution separates replicates (V-4); rejected.
- **Use `vm.lifecycle.ready` as READY receipt**: it trails receipt by one notify
  hop; the `BeaconIo` decorator is exact; rejected.

## Consequences

The family depends on event names and on the console log path; a rename breaks the
family's smoke test. The family boots a guest image that production `overdrive-init`
can run, built with the guest kit (`GuestImageSpec::workload()`), not the shared VM
fixture's image (V-4 fixture finding, DDD-11); the family fixture creates its own
`<data_dir>` under a scoped, RAII-restored `umask(0o022)`, and only the guest kit's
own directories keep group 4200 and mode `0710`. That the image boots under production
`overdrive-init` with the console-log decorator is a DELIVER validation item that
blocks only this family's step (V-10); it is not contract-shaping, because the seam
and the image source are pinned. The existing native
lifecycle test keeps its own layer and does not move onto this framework (user
ruling 2026-11-17).

## Links

- [Decision DDD-11 and the stage table](../../feature/native-benchmark-program/feature-delta.md#ddd-11--vm-lifecycle-stage-observation)
