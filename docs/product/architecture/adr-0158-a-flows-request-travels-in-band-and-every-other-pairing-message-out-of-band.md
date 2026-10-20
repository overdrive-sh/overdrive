# ADR-0158 — A flow's request travels in band and first; every other pairing message travels out of band

## Status

**Accepted — ordering ruling D18, approved by the user 2026-10-05.** GH #295.
Not yet implemented. The byte layouts are in the #295 feature delta,
§ *[REF] Wire contracts* and § *[REF] Per-kind total orders*.

Related decisions, each separately reversible: the carrier of the
out-of-band messages, a dedicated per-VM control session (D18a, ADR-0166);
when each side attaches a flow's pair to the forwarder (EVMOD-PAIR,
ADR-0212); admission as one step with registration, and the stop of a
continuation for a closed flow (U-1, ADR-0213); the host end of a
host-opened flow held by its dialer (ADR-0214) and watched by the host
forwarder (ADR-0216); the bound on
each kind's pairing wait (ADR-0215); the refusal of a half that has ended
before its attach (ADR-0210); the guest's pending TcpConnect bound
(ADR-0150); non-blocking owner connects (ADR-0151).

## Context

A flow's destination, kind and admission verdict must reach the side that
pairs it, and pairing must not lose or duplicate bytes the application or the
remote peer sends before pairing completes.

The guest-capture spike (`spike/guest-vsock-capture-findings.md`, 7.0.0-29)
measured an in-band 4-byte `Paired` on the flow's own vsock connection: it
failed 18/100 egress flows whose destination spoke first (the banner
stranded behind the response) and 1/100 inbound flows. The out-of-band
ordering passed 10,000/10,000 in every stress.

## Decision

- **The request is in band and first.** The opener of each flow vsock
  connection writes one fixed 32-byte request (carrying its launch token,
  ADR-0177) before anything else. No payload ever precedes it. The acceptor
  reads exactly 32 bytes in userspace before anything else touches the
  socket (H-1).
- **Every other pairing message is out of band** — `Paired`, `Refused`,
  `Abort`, and the timeout reply to a host-opened flow. No control byte is
  ever written on a flow vsock connection after its request.
- **A flow's end is never a control message** (ADR-0180).
- **`Refused` and `Abort`** close the flow on the receiving side (ADR-0160).

## Alternatives considered

- **In-band `Paired` on the flow connection.** Falsified (18/100 egress,
  1/100 inbound). Rejected.
- **The acceptor hands its accepted vsock to the forwarder before reading
  the request.** The request would become payload. Rejected.
- **Destination carried in band and decided in the kernel.** Moves policy the
  control plane owns (mesh resolution, refusals) into the kernel, and
  refusals would not be typed values. Rejected.

## Consequences

- The wire codec (request, control message, datagram frame) is one SSOT
  shared by the host owner and `overdrive-init` (feature delta, § *Wire
  contracts*).
- Loss of the carrier of `Paired` aborts the affected flows (ADR-0166).
- The ordering is the host-side protocol of every guest: the host owner reads
  the request and replies out of band the same way whatever guest opened the
  flow (ADR-0207). A guest that runs no Linux kernel is
  [GH #321](https://github.com/overdrive-sh/overdrive/issues/321).
