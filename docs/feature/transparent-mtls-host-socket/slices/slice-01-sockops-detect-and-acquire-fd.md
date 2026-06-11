# Slice 01 — sockops detects the host-socket connection and the agent acquires its socket

> **Productionises the Slice-00 spike's detect+acquire step.** If the spike
> returned FAIL (Cilium fallback), this slice re-shapes to the fallback's
> interception mechanism — but the observable (the connection is detected and
> brought under platform control before the workload writes) is unchanged.

**Job**: J-SEC-003 | **Feature**: transparent-mtls-host-socket (GH #26) | **Stories**: US-MTLS-01

## Goal (one sentence)

A kernel sockops program detects a host-socket workload's `ACTIVE_ESTABLISHED` /
`PASSIVE_ESTABLISHED` transition synchronously (before connect()/accept() returns),
and the node agent acquires the workload's own socket fd (process: `pidfd_getfd`) —
so the connection is under platform control for the handshake to follow, before the
identity-unaware workload can write a cleartext byte.

## IN scope

- A kernel sockops program firing on a host-socket workload's `ACTIVE_ESTABLISHED`
  / `PASSIVE_ESTABLISHED` transition, synchronously in kernel context (before
  connect()/accept() returns to the workload).
- The node agent acquiring the workload's own socket fd (process: `pidfd_getfd`)
  for the detected connection.
- Correctly NOT firing for a guest-stack workload's connection (TCP in the guest
  kernel — no host `struct sock`; that is #222's scope).
- The sockops program + its maps/link bpffs-pinned (`pinning = ByName`,
  `/sys/fs/bpf/overdrive/`) — the prerequisite for restart survival (Slice 05).

## OUT scope

- The rustls handshake → Slice 02.
- The kTLS install + agent-exit + wire capture → Slice 03.
- The race-window gate (sk_msg DROP-until-armed) → Slice 04 (the gate insertion
  POINT is here; the fail-closed proof is Slice 04).
- The WASM in-process-fd variant → Slice 05.

## Learning hypothesis

- **Disproves if it fails**: "the platform can detect a host-socket connection
  in-kernel synchronously (before the workload writes) and acquire the workload's
  socket fd." If detection fires too late (after the workload could write) or the
  fd cannot be acquired, the in-kernel-mediation model has a hole that no
  downstream slice can close.
- **Confirms if it succeeds**: the detection+acquire foundation is sound; the
  handshake (S02), install (S03), and gate (S04) build on a connection that is under
  platform control before any cleartext can escape.

## Acceptance criteria

- [ ] A kernel sockops program fires on a host-socket workload's `ACTIVE_ESTABLISHED` / `PASSIVE_ESTABLISHED` transition, synchronously in kernel context (before connect()/accept() returns) — observable in a Tier-3 test (the program runs; the connection is gated before the workload writes).
- [ ] The node agent acquires the workload's own socket fd (process: `pidfd_getfd`) for the detected connection.
- [ ] A guest-stack workload's connection (TCP in the guest kernel) does NOT trigger the host detection path and does not error (correctly out of scope — #222).
- [ ] The sockops program + its maps/link are bpffs-pinned (`pinning = ByName`, `/sys/fs/bpf/overdrive/`) — prerequisite for Slice 05's restart survival.
- [ ] `cargo xtask lima run -- cargo nextest run -p <crate> --features integration-tests` green for the new detection/acquire acceptance test (real kernel, not `--no-run`).

## Dependencies

- Slice 00 (the spike validated the detect+acquire mechanism) — must PASS (or the
  fallback re-shapes this slice).
- sockops + `pidfd_getfd` on the 6.18 kernel (ADR-0068) — guaranteed.
- The bpffs-pin discipline (`pinning = ByName`, already used for HASH_OF_MAPS).

## Effort estimate

~1 day (≤6h). Reference class: the sockops attach + bpffs pin mirror the existing
dataplane pin discipline; the `pidfd_getfd` acquisition is proven by the spike.

## Pre-slice SPIKE

Not needed — Slice 00 (the spike) validated the detect+acquire mechanism on the real
kernel. This slice productionises it.

## Taste-test note

A thin vertical cut: ships the sockops program + bpffs pin + fd-acquisition.
Production-data observable (the program fires on a real connection; the fd is
acquired — Tier-3, real kernel). Disproves a real assumption (in-kernel detection
precedes the workload's write). Carries one value story (US-MTLS-01) whose
observable is the detected+gated connection — not an infra-only shell (the detection
is the necessary first step of the enforcement the user can verify end-to-end by
S03). The guest-stack negative case keeps the scope boundary (#222) honest.
