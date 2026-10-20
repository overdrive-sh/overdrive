# ADR-0214 — The host end of a host-opened flow is held by its dialer, never forwarded

## Status

**Accepted.** That a host platform client's dial yields a stream the dialer
reads and writes itself (INB-1) was approved by the user 2026-10-11 (rulings
"Finding 1" and "Finding 1 amended", recorded on
[#303](https://github.com/overdrive-sh/overdrive/issues/303#issuecomment-6113302506)).
GH #295. Not yet implemented. Contract: the #295 feature delta, § *[REF]
Driving port — guest stream dial `GuestStreamDial`*.

Related decisions, each separately reversible: the dial itself, its
admission and its typed results (INB-1, ADR-0173); the host forwarder's
watch of the dialled vsock, from which the stream takes the classified end
(EVMOD-END, ADR-0216); what an owner's abort of a watch does (WATCH-ABORT,
ADR-0211); the temporary leg-S relay over the stream (ADR-0186).

## Context

Leg-S and the VM probe runner reach a VM port through the guest-flow owner's
`GuestStreamDial` (ADR-0173). Both consume the bytes in userspace: leg-S
relays them to its kTLS socket until #303 (ADR-0186), and an HTTP probe
parses them. A kernel forward of the host end therefore has no target.

## Decision

- **The owner opens the flow and hands over its host end.** For a dial the
  owner opens the flow vsock, writes the request, and hands the vsock to the
  dialer as its `GuestStream` once the guest answers `Paired`.
- **The dialer holds the host end.** It reads and writes the vsock itself;
  the host forwarder never attaches it to a pair.
- **The guest side is forwarded** in a Linux guest's kernel as for any flow
  it accepts (ADR-0212); a guest that runs no Linux kernel is
  [GH #321](https://github.com/overdrive-sh/overdrive/issues/321).

## Alternatives considered

- **Forward the host end in the host kernel.** The dialers consume the bytes
  in userspace, so a kernel forward has no target. Rejected.
- **Host-local delivery to the guest prefix** (a `local` route, steering or
  intake listeners, so the dialers keep a plaintext TCP connect as for exec
  workloads). Rejected by INB-1: the guest prefix is not host-local
  (ADR-0173).
- **The owner holds the vsock and relays the bytes to the dialer over a
  second channel.** A second userspace copy per dialled byte and a second
  end to propagate, for no fact the dialer lacks. Rejected.

## Consequences

- A dialled stream holds a node flow slot, and for `Relay` a quota place,
  but no host pair (ADR-0173, ADR-0208); its places are released at its drop
  or abort (ADR-0209).
- The stream's lifetime is the dialer's: dropping it ends the flow.
- The dialer cannot learn the class of the guest's end from its own `read`
  (ADR-0180); it takes it from the watch (ADR-0216).
