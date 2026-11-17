# ADR-0302 — A benchmark refuses a host that differs from the pinned test-host settings

## Status

**Accepted.** Approved by the user 2026-11-17; independent design review resolved 2026-11-17. DDD-7 in the feature delta.

## Context

Ruling 2: run at production settings (the test host's own), record them, refuse a drifted
host, and set no noise controls. O1 (accepted by the user 2026-11-16): the
methodology version pins an expected list taken from the appliance image
configuration where it defines a value, otherwise from the first accepted host.
ADR-0068 pins the kernel; it defines no governor, SMT or boost value. The spike on
the Metal host recorded kernel `7.0.0-38-generic`, governor `schedutil`, SMT on,
boost on, `bpf_stats_enabled` 0.

## Decision

Before any sample the runner reads the host through a read-only `HostProbe` port
and compares it with the pinned list of methodology version 1: kernel release
series 7.0, `kernel.bpf_stats_enabled` 0, CPU governor `schedutil` on every
online CPU, SMT on, CPU boost on. The values are the test host's, not appliance
values. A difference is a refusal that names the setting
with its found and pinned values, writes a record holding `run_started` and
`run_refused`, and exits without taking a sample. The CPU model, the online CPU
count and the clocksource are recorded, not refused.
The probe separates a setting that is absent by design (for example the
`intel_pstate` file on an AMD host) from one that cannot be read, and refuses on
the latter. A pinned setting with no source at all cannot be verified and also
refuses, with `found` = `absent`; the record's host block keeps every value,
including `absent` and `unreadable` forms, and `run_refused` names the setting. The probe also scans `/proc/*/fd` for a holder of a
`bpf-stats` descriptor, since the sysctl reads 0 while a descriptor holds stats on
(`kernel/bpf/syscall.c:6014`, `:6551-6558` @ v7.0). A build with debug assertions
is refused as well. The probe's own failure is the first refusal (`host_probe`): no setting
was read, so the record's host block marks every key `unreadable` with the probe error.

## Alternatives

- **Set the noise controls (turbo off, SMT off, ASLR off)**: ruling 2 sets none;
  rejected.
- **Record only, never refuse**: a result from a drifted host would look
  comparable; rejected by ruling 2.
- **Pin a different host's values**: the pinned list is the one accepted host;
  changing it is a methodology version bump.

## Consequences

A different machine class refuses until its values are accepted by a version
bump. The pinned values come from one recorded host and are re-confirmed by the
first accepted run.

## Links

- [Decision DDD-7 and the pinned list](../../feature/native-benchmark-program/feature-delta.md#ddd-7--host-eligibility-and-the-pinned-settings)
