# ADR-0210 — A socket half that has ended before attach is refused, and its flow is aborted

## Status

**Accepted — EVMOD-ATTACH-END, approved by the user 2026-10-17**, under the
user's ruling of the same day that an attach refused because a half reset or
closed is a flow abort. GH #295. Not yet implemented. Contract: the #295
feature delta, § *[REF] vsock data path — the in-kernel event-driven
forwarder (EVMOD)*, "Control interface" ("States attach and watch accept",
"Attach refusals") and § *Failure projection*.

Related decisions, each separately reversible: attach at pairing and the
states attach accepts (EVMOD-PAIR, ADR-0212); how an end already present on a
still-connected half is classified (ADR-0180).

## Context

Each side attaches a flow's pair to the forwarder at pairing (ADR-0212), so a
half can change state before its attach. A half whose peer only ended its
direction is still connected; its end is carried out by the forwarder's
first run (ADR-0180). A half can also have **ended**, with no connection
left to forward:

- a TCP half that received a reset: `tcp_reset` leaves it in `CLOSE` with
  `sk_err` pending, `ECONNRESET`, or `EPIPE` when it was in `CLOSE_WAIT`
  (`net/ipv4/tcp_input.c:4905-4931 @ v7.0`);
- a vsock half the device release's orphan sweep reset: `sk_err =
  ECONNRESET`, and `sk_state` set to `SS_UNCONNECTED`
  (`drivers/vhost/vsock.c:746-750 @ v7.0`), whose value 1
  (`include/uapi/linux/net.h:49-50 @ v7.0`) equals `TCP_ESTABLISHED`
  (`include/net/tcp_states.h:13 @ v7.0`), so `sk_err` must be read before
  the state;
- a vsock half in `CLOSING`, which `virtio_transport_do_close` sets on an RST,
  or on `0x3` with an empty queue
  (`net/vmw_vsock/virtio_transport_common.c:1249-1252 @ v7.0`).

The shapes are reachable on the production path: the guest owner closes a
flow vsock before the host attaches it (a pending intake child that reset,
the pairing deadline, session loss); an intake child, a destination or an
application resets before its attach. Both spike modules refused these
states and the owner's flow abort that followed left no clean EOF at any far
end (P-70 § 3.6; P-71: `eg-rst` refused in at least 99 %, `eg-drst`'s host
attach refused). The spike modules returned the half's own errno
(`ECONNRESET`, `EPIPE`) or `ENOTCONN` for a vsock `CLOSING` half; `ENOTCONN`
is also what attach returns for a half that never connected, an owner
defect.

## Decision

- **Attach and watch refuse a half that has ended** — a pending `sk_err`,
  checked before the state, or a vsock half in `CLOSING` — in the locked
  section that would install the overrides, before anything is installed on
  that half. A refused attach leaves neither half hooked.
- **The refusal has an errno class of its own**, `ECONNABORTED`, used by no
  other refusal, and carries the half's own cause (its `sk_err`, else
  `ECONNRESET`) and which half it was. It is never confused with a
  never-connected half.
- **The owner aborts the flow** by its existing path for a flow that is not
  yet `Paired` (or, for the guest's attach on `Paired`, the abort of a flow
  the host already attached), each TCP half closed with `SO_LINGER{1,0}`,
  and counts the refusal. A refused half is never hooked, so no end record
  follows; the owner's abort is the flow's only end.

## Alternatives considered

- **Return the half's own errno, as the spike modules did.** `ENOTCONN`
  collides with the never-connected owner defect, and a pending `sk_err` may
  equal another refusal's errno. Rejected.
- **Attach an ended half and abort it in the first run, yielding an end
  record.** It records an end for a flow that was never forwarded, a state
  neither spike module accepted, and goes against the user's ruling.
  Rejected.

## Consequences

- An attach error is either an owner defect (`ENOTCONN` and the other
  classes) or a flow event (`ECONNABORTED`); the owners project only the
  second into a flow abort.
- Which errno the module returns is module code, checked by the module's
  suite in DELIVER; the kernel facts the class rests on are cited above.
- The refusal is the host's handling of what any guest sends (ADR-0207; a
  guest that runs no Linux kernel is
  [GH #321](https://github.com/overdrive-sh/overdrive/issues/321)).
- The Quint module `fwd_pair` checks the refusal and the flow abort
  (feature delta, § *Formal protocol model*, third iteration).
