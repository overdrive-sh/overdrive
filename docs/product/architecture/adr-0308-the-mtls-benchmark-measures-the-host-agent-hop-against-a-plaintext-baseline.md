# ADR-0308 — The mTLS benchmark measures the host agent hop against a plaintext baseline

## Status

**Accepted.** Approved by the user 2026-11-17; independent design review resolved 2026-11-17. DDD-13 in the feature delta.

## Context

The mTLS family compares a plaintext baseline with the composed mTLS path in both
directions (handshake, first byte, sustained throughput, tail latency, CPU, memory,
sockets). Production exposes no success-path timing on the path. The existing rigs
compose the real components in process: the worker egress rig (real enforcement,
real intercept, a rustls mesh peer on a veth) and the inbound relay rig (real
enforcement over a loopback leg C, with the guest side as an in-memory
`GuestStreamDial`). The egress dialer is plaintext by design (CLAUDE.md, East-west
mTLS tests).

## Decision

The family measures the host agent hop. Egress: a plaintext client, the real
agent, a mesh peer; baseline arm: the same client to a plaintext peer on the same
peer host over the same lease. Ingress: a rustls client playing the originating
agent, the real agent, an in-memory guest stream; baseline arm: a plaintext client
straight to an equivalent in-memory sink. The report states both definitions. The
handshake row is the duration of `MtlsEnforcement::enforce`, measured by a timing
decorator injected into the worker; first-byte, throughput and tail latency are
measured at the client socket; CPU is whole-host `/proc/stat` with the user, system,
IRQ and softirq split (no cgroup figure: interrupt and kernel-worker time is
charged outside the benchmark's cgroup); memory and socket
counts come from the host and `/proc/self`. Tail latency is closed-loop
request/response and says so. The per-direction rows (egress) sum per-thread CPU by
the pump thread names (ADR-0309). Spike V-3 shows the pumps account for essentially
all work of the process outside the fixture's own threads (1.000 and 0.999 to 1.000
in the two directions), so the report names the fixture threads separately and
states the pump share excluding them. A pump saturates one core (about 890 MB/s), so
the light thread's CPU is below the tick resolution of `/proc` and the heavy-to-light
ratio is reported as a lower bound. The mesh-peer fixture sets
`send_tls13_tickets = 0`, as production peers do: a TLS 1.3 session ticket makes the
kTLS-RX return `splice` fail with EINVAL and tears the connection down.

## Alternatives

- **A real guest for ingress**: adds vsock and virtio cost to the same number and
  needs the VM stack, and extends the measurement of the ADR-0069 host proxy that
  in-guest kernel mTLS ([#303](https://github.com/overdrive-sh/overdrive/issues/303))
  replaces; rejected.
- **The wire-capture correctness check inside the benchmark**: stays a Tier-3 test.
- **A shared peer serving TLS and plaintext on one port**: not a real endpoint;
  rejected.

## Consequences

The ingress number excludes the guest transport, and ingress has no forward/return
split; both are properties of the host-hop benchmark, which is the baseline for the
comparison with in-guest kernel mTLS in
[#303](https://github.com/overdrive-sh/overdrive/issues/303) (which uses #287 for
that comparison). The host proxy is being replaced by #303, so widening its
measurement is not planned work.

## Links

- [Decision DDD-13](../../feature/native-benchmark-program/feature-delta.md#ddd-13--mtls-boundary-rigs-and-baseline)
