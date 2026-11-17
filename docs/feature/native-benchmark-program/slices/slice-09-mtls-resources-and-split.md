# Slice 09 — Transparent mTLS: resource cost and forward-versus-return work

Story: US-BENCH-09. Job: J-PLAT-008. Family: `mtls-path`, resource profile.

## Goal
CPU, memory, sockets and concurrency per arm and direction, with forward-path userspace work apart from return-path splice work.

## In scope
- Whole-host CPU (from `/proc/stat`) with kernel and userspace split; no cgroup-scoped figure is reported, and the report states why whole-host is the scope.
- Memory, socket count and connection concurrency per arm and direction.
- Forward-path userspace and return-path splice reported as separate rows for egress; absent for ingress (CA-6).
- Unavailable metrics reported absent, never zero.

## Out of scope
Changing the proxy or kTLS architecture; deciding whether the claim "agent-light" is met (the report informs that decision).

## Learning hypothesis
Disproves: that the forward and return work can be separated from the existing production surfaces. If not, DESIGN defines the observable point.

## Acceptance
- [ ] CPU scope stated; whole-host only, no cgroup figure.
- [ ] Memory, sockets, concurrency per arm and direction.
- [ ] Forward and return work separate.
- [ ] Unavailable metrics named absent.

## Dependencies
Slice 08.

## Estimate
2 to 3 days. Reference class: slice 08.
