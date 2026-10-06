# ADR-0158 — Flows are paired out of band: sockets are installed before their peer can send, early bytes park in the kernel, and `Paired` travels outside the flow connection

## Status

**Proposed — ordering ruling D18 approved by user 2026-10-05; the per-kind
wait bounds follow D15-R3 = (b), approved by user 2026-10-06; the atomic
admission-and-registration step and the stop of a continuation for a closed
flow (U-1, formal model teeth `flows_bugAdmitSplit`,
`flows_bugPairedNoRecheck`) pinned at the user's direction 2026-10-06;
pending independent DESIGN review.** GH #295. Recorded in the #295 feature delta,
§ *[REF] vsock Attachment Replacement DESIGN — PROPOSED 2026-10-05*, which
holds the byte layouts and the per-kind total orders.

The carrier of the out-of-band messages (a dedicated per-VM control session)
is a separate decision, D18a (ADR-0166).

## Context

A flow's destination, kind and admission verdict must reach the side that
pairs it, and pairing must not lose or duplicate bytes the application or the
remote peer sends before pairing completes.

The guest-capture spike (`spike/guest-vsock-capture-findings.md`, 7.0.0-29)
measured the alternatives:

- An in-band 4-byte `Paired` on the flow's own vsock connection, with the
  acceptor installing before or after writing it, failed 18/100 egress flows
  whose destination spoke first (the banner stranded behind the response in
  the guest vsock socket) and 1/100 inbound flows.
- Installing an intake socket after `Paired` stranded early bytes 100/100
  without a re-arm, and could not install at all, 100/100, for a client that
  wrote and then FIN'd (`sock_map` requires `ESTABLISHED`).
- Installing a UDP intake after `Paired` stranded the first datagram 30/30.
- The out-of-band ordering below passed 10,000/10,000 in every stress, with
  zero drain timeouts and zero leg errors.

The kernel facts behind the result: TCP can be re-armed by `SO_RCVLOWAT` but
only while still installable; UDP and vsock deliver one skb per `data_ready`
and cannot be re-armed; a child installed before `accept()` ignores
`data_ready` until accepted; a socket inserted into a SockHash with a verdict
hands every received skb to the verdict, which drops it when no route exists.

In the spike, every acceptor read the 16-byte request in userspace first and
only then inserted its vsock socket into the verdict's SockHash
(`spike-scratch/netns-density-295-v11-vip-v14/vfwd/src/owner.rs:1419-1482`;
`bpf/src/main.rs:437-443`: a route miss returns `SK_DROP`).

## Decision

- **Requests stay in-band and first.** The opener of each flow vsock
  connection writes one fixed 16-byte request before anything else. No payload
  ever precedes it. The acceptor reads exactly 16 bytes in userspace.
- **Every other control message is out of band** (`Paired`, `Refused`,
  `Abort`, `ListenState`). No control byte is ever written on a flow vsock
  connection after its request.
- **Install before the peer can send:**
  - the opener installs its vsock socket immediately after `connect()`
    returns and before writing the request — the peer owner has no handle to
    it yet;
  - intake children are installed at `PASSIVE_ESTABLISHED` and park their
    bytes in a kernel loopback TCP cell until `Paired`;
  - acceptor-side sockets that the acceptor connects (to the destination, to
    leg-F, or to the guest application) are armed before `connect()` and
    installed at `ACTIVE_ESTABLISHED`;
  - UDP intakes are installed from boot and park into framing cells.
- **The acceptor reads, then installs.** The acceptor reads the 16-byte
  request from the vsock socket it accepted in userspace, and only then
  installs that socket, before connecting its peer-facing socket. This is safe
  because no byte can follow the request on that connection before `Paired`:
  the opener's application bytes are parked until `Paired`. Installing before
  the read would hand the request to the verdict, which drops it on a route
  miss.
- **`Paired` releases the parking.** On `Paired` the opener routes its parking
  cell to the flow's vsock socket and re-arms the cell; early bytes then flow
  in order.
- **Admission and registration are one step (U-1).** The acceptor's admission
  check for a flow (allocation Active, live control session, forwarding not
  quiesced, quota) and the flow's registration in that allocation's flow table
  happen as one step relative to quiescence, teardown and control-session
  loss, so every flow those events must abort is already registered when they
  run. A flow is never admitted on a check that one of them has since made
  false.
- **A continuation stops for a closed flow (U-1).** When an acceptor's
  non-blocking connect completes, it routes and sends `Paired` only if the
  flow is still open; for a flow already closed (aborted by quiescence,
  teardown, session loss or `Abort`) it closes the connected socket and sends
  nothing.
- **`Refused` and `Abort`** close the flow on the receiving side (ADR-0160).
- **Bounded waits.** The opener of a datagram association or a host-opened
  inbound flow that receives neither `Paired` nor `Refused` within the pinned
  pairing deadline aborts the flow. A guest-opened TCP flow has no opener
  deadline: the host answers it once its own connect ends, and a non-mesh
  connect is bounded by the host kernel's SYN retries (ADR-0162, D15-R3). A
  pending request counts against the per-allocation quota.
- **Owner connects never block.** Every `connect()` either owner issues while
  pairing is non-blocking and completes on readiness. A blocking connect
  stalls every other flow and the listen-state reports of the same owner
  (spike `v11-vip-v14`, increment c).

## Alternatives considered

- **In-band `Paired` with per-kind install order.** Falsified (18/100, 1/100).
  Rejected.
- **Late install with a re-arm.** Works for TCP only until the peer FINs; never
  for UDP or vsock. Rejected.
- **Acceptor installs its accepted vsock before reading the request.** The
  verdict consumes the request and drops it on the route miss. Rejected.
- **Destination carried in-band and decided in BPF.** Moves policy that the
  control plane owns (mesh resolution, refusals) into BPF, and refusals would
  not be typed values. Rejected.

## Consequences

- The wire codec (request, control message, datagram frame) is one SSOT shared
  by the host owner, `overdrive-init` and both sets of programs, pinned by
  golden-bytes tests and proptests.
- Each side needs pooled parking cells and a `sock_ops` program (ADR-0150,
  ADR-0151).
- Leg-F needs no write gate: the host socket connected to leg-F is installed
  at `ACTIVE_ESTABLISHED` (ADR-0153).
- Loss of the carrier of `Paired` aborts the affected flows (ADR-0166).
