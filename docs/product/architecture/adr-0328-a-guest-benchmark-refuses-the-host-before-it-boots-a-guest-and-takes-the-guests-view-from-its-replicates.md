# ADR-0328 — A guest benchmark refuses the host before it boots a guest and takes the guest's view from its replicates

## Status

**Accepted (2026-11-19).** Approved by the user as PD-4, option (b). Extends [ADR-0302](adr-0302-a-benchmark-refuses-a-host-that-differs-from-the-pinned-test-host-settings.md) (operative). DDD-4 in the [feature delta](../../feature/guest-benchmark-substrate/feature-delta.md).

## Context

ADR-0302 pins governor, SMT, boost, kernel series, `bpf_stats_enabled`, the `bpf-stats` holder scan and a release build, and treats a pinned setting with no readable source as a refusal. A guest has no cpufreq, boost or SMT control source, so applying that list inside a guest would refuse every guest. The host settings still govern the number: the guest's vCPUs run on the host's CPUs. A drifted host must be refused before any guest boots (K3). The record's `run_started` is the first line of an append-only record, written before the first replicate, so it cannot hold anything a guest reports. The guest kit stages the guest kernel and the guest programs; the user controls that kernel ("the guest kernel would never be stale... we control it"), so the guest's kernel is recorded for the reader and is not a pinned setting.

## Decision

1. The host half applies the full existing list to the real host first, unchanged. A host refusal ends the run before any guest boots, as for a native benchmark; the run's only refusals are host refusals.
2. No guest boots before the replicates. There is no probe guest and no guest-side refusal setting. Each replicate's in-guest program prints the guest's kernel release and clocksource on the console, and the family's host half reports them with the staged artifacts' digests and the guest shape to the runner as the replicate's guest view (ADR-0326).
3. The first replicate of the run to report a view fixes the run's guest view: the runner appends one `guest_observed` line before that replicate's line. Later reports are not compared with it. The operator fixes the guest's kernel, the staged artifacts and the guest shape, and the benchmark runs under exclusive admission on a bound checkout, so nothing a replicate's guest reports can differ from the first replicate's (the user's reason for PD-4: "we control it"). The runner fails a replicate whose body returned `Ok` without reporting a view, so every `ok` replicate of a guest record follows the line. A record in which no replicate reported a view has no `guest_observed` line and the report states that.
4. No guest setting is pinned: the guest's kernel and clocksource are recorded and never refuse, and the settings a guest cannot see are recorded as ADR-0326 states.

## Alternatives

- **A probe guest booted after the host check, reading the view before `run_started`** : proves the kit and pins the guest kernel series before any sample. Costs one more guest boot per run (about 1.1 s of about 84 s), a port, a probe error type and two refusal settings, for a stale-kernel check the user does not need. Rejected by the user (PD-4).
- **Apply the existing list in the guest**: refuses every guest. Rejected.
- **Verify only the host and record no guest view**: no reader can tell which kernel or which staged artifacts a guest record ran. Rejected.
- **A pinned guest kernel series**: no guest-side check exists to host it without a probe guest, and the kernel is under the user's control. Not added.
- **The view as a field on every `replicate` line**: repeats tens of bytes per line and still needs the report to pick one view for its single header. Rejected.
- **Comparing every later replicate's view with the first and failing a differing one**: guards a variation no production path of this run produces (the kernel, artifacts and shape are the operator's, fixed for the run), at the price of a comparison, a failure path and a boundary scenario. Not added.
- **An unprepared kit ends the run on the first replicate**: needs a run-level abort a body does not have today, and an end for one cause only. Rejected for the simpler rule that a guest that cannot boot is a failed replicate (ADR-0329); each failure is immediate because the kit check precedes any launch.
- **A `bpf_stats_enabled` pin in the guest**: no guest benchmark declares `bpf_stats` (ADR-0325), so no reachable failure motivates it. Not added.

## Consequences

No extra guest boot. A prepared kit and a bootable guest are shown by the first replicate that boots and reports. An unprepared guest kit is a failed replicate (ADR-0329): every replicate fails with `guest boot: <cause>`, the cause naming `cargo xtask guest-kit-prepare`, no guest boots and no sample is taken; the run is not refused and no measurement is taken on the host in its place. It shows as a run whose every replicate failed with the same reason; US-GBS-01's "refused before any sample" holds for the host's settings only (feature delta CA-4). The guest `debug_assertions` state is not read: the guest programs are built `--release` by `guest-kit-prepare` and no debug build is reachable.

## Links

- [feature delta, DDD-4, DDD-2 and Lifecycle Gate Ownership G-2](../../feature/guest-benchmark-substrate/feature-delta.md)
- [ADR-0326](adr-0326-a-guest-benchmark-record-states-the-guests-conditions-in-run-started-and-its-view-in-one-guest-observed-line.md)
