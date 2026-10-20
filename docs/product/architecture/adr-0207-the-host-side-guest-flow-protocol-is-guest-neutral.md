# ADR-0207 — The host side of the guest flow protocol is guest-neutral

## Status

**Accepted — approved by the user 2026-10-17** (EVMOD approval: "the
host-side frame protocol (request, datagram and end classes) is
guest-neutral; the host forwarder never branches on guest type"). GH #295.
The guest hop for a guest that runs no Linux kernel (Unikraft) is separate
work, [GH #321](https://github.com/overdrive-sh/overdrive/issues/321).
Contract: the #295 feature delta, § *[REF] Wire contracts* and § *[REF] vsock
data path — the in-kernel event-driven forwarder (EVMOD)*, "Guest-neutral
host side".

## Context

Every guest flow reaches the host as a vsock STREAM on the node's flow port.
What the host reads from it is a protocol:

- the 32-byte flow request the opener writes first (ADR-0158, ADR-0177);
- for a datagram association, the length-prefixed frame both ways
  (ADR-0149, ADR-0201);
- the end classes of each direction: SHUTDOWN `0x2` a clean half-close,
  `0x3` with a direction open or RST an abort, and the clean full end of
  `0x2` each way before close (ADR-0180).

Linux guests implement the guest side with `overdrive-init` and the
`ovd_fwd` module of their guest kit (ADR-0206). The research on a Unikraft
guest (`docs/research/networking/unikraft-guest-forwarder-research.md`)
found that such a guest has no loadable module, a different socket layer and
gaps in its vsock stack, so its guest side would be built differently — but
the host side it must reach is the same vsock STREAM carrying the same
request, frames and end classes.

## Decision

- **The request, the datagram frame and the end classes are the host-side
  protocol of every guest.** Their layouts and meanings carry no field and
  no value that names a guest kind, kernel or implementation.
- **The host owner and the host forwarder never branch on guest type.** The
  host attaches, watches, classifies ends, frames and unframes datagrams,
  admits, pairs and releases flows the same way for every guest. No host
  code path, configuration or capability check selects behaviour by the
  guest's kernel or image.
- **A guest's own implementation is the guest's concern.** Linux guests use
  the guest kit (ADR-0206). A guest that runs no Linux kernel implements the
  same protocol in its own image; that hop is GH #321, and it changes
  nothing on the host.

## Alternatives considered

- **A guest-kind field in the request, with host branches per kind** (for
  example a host-side userspace relay for a guest whose stack lacks a
  feature). Every guest kind would then add host code, tests and failure
  modes to the one component every flow crosses, and a guest's gap would
  become the host's problem. Rejected.
- **Negotiate protocol features per guest at session start.** Adds a
  capability exchange and a matrix of host behaviours for differences the
  guest can absorb in its own image. Rejected.

## Consequences

- The host side is specified, modelled (`specs/quint/guest-flow-owner`) and
  tested once, for every guest.
- A guest implementation conforms by producing the protocol on the wire; a
  guest that cannot (a vsock stack without a needed behaviour) is fixed in
  the guest, not accommodated on the host.
- GH #321 can proceed without amending a host contract of #295.
