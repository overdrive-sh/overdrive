# ADR-0305 — Stage boundaries are stamped on the host, and guest stages are durations

## Status

**Accepted.** Approved by the user 2026-11-17; independent design review resolved 2026-11-17. DDD-10 in the feature delta.

## Context

Research F1.9, read in `vendors/linux` at v7.0: host `CLOCK_MONOTONIC` is
NTP-slewed (`kernel/time/timekeeping.c:2163`) while `CLOCK_MONOTONIC_RAW` is not
(`timekeeping.c:366-367`); a guest's clocks have a different, unrecorded zero point
from the host's (`arch/x86/kvm/x86.c:13253`, `kernel/time/timekeeping.c:1801-1843`),
so a guest instant cannot be subtracted from a host instant. The guest's actual
clocksource is not settled.

## Decision

Every stage boundary is stamped by one host process with `CLOCK_MONOTONIC_RAW`
through one clock type in `overdrive-bench`; the record names it. A stage that
happens inside a guest is reported only as a duration the guest measured on one
guest clock between two of its own readings. A guest value is never subtracted
from or added to a host instant, and a duration the guest did not report is absent,
not zero. A duration measured on the injected production `Clock` (the DNS observer)
is recorded with that clock named.

## Alternatives

- **Estimate a host-guest offset over vsock**: adds a guest protocol and a
  measurement step; not needed while every stage can be one-sided; rejected by
  the user (2026-11-17).
- **`Instant` (`CLOCK_MONOTONIC`)**: slewable during a run; rejected for host
  stamps.

## Consequences

Guest receipt latency (host send to guest receive) is not a guest duration and is
not reported as one. VM lifecycle reports one guest-clock duration,
`guest.shutdown-handling`; guest receipt and poweroff appear only as the
host-stamped workload-exit and reaper-exit stage intervals.

## Links

- [Decision DDD-10](../../feature/native-benchmark-program/feature-delta.md#ddd-10--clocks-host-stamps-and-guest-durations)
