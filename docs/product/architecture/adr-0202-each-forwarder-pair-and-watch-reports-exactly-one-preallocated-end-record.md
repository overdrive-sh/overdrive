# ADR-0202 — Each forwarder pair and watch reports exactly one preallocated end record

## Status

**Accepted — EVMOD-REC, approved by the user 2026-10-17.** GH #295. This ADR
decides only the completion channel: how the owner learns that a pair or
watch it attached ended, and why. It does not decide the pairs' lifetime
(ADR-0199, EVMOD-CTL), how a vsock end is classified (ADR-0180, EVMOD-END),
or how a dialled stream's holder learns its outcome (the `GuestStream`
contract, EVMOD-STREAM-END, under the user's WATCH-ABORT ruling of
2026-10-17): each could be kept while this channel is replaced, and this
channel could serve another rule for each. Follows from ADR-0197 (accepted
2026-10-16) and the user's design input of that ruling: "a real control
interface and pair-completion event in place of the spike ioctl and /proc
polling". The record layout, the `read` and `poll` operations and the port
method that returns a record are in the #295 feature delta, § *[REF] vsock
data path — the in-kernel event-driven forwarder (EVMOD)*, "Control
interface" and "Driven port — `FlowForwarder`".

## Context

A pair the forwarder ends drops its own references to the pair's sockets.
Its owner must then learn that the pair ended, and why, to close its own
descriptors, clear the linger of a cleanly ended TCP socket, discard a
datagram slot's queue, and release the flow's quota place and node flow slot
(ADR-0208, ADR-0209). A watch (a dialled vsock the forwarder classifies but
does not forward, ADR-0216) ends the same way and must be reported to the same owner. The evmod
spike learnt of ends only by polling `/proc/evmod`
(`spike/findings-evmod.md`, caveat 7).

- **An owner that misses an end leaks a quota place for the flow's life.**
  The redirect design lost end records when its fixed ring was full and
  needed a loss count and an abort rule for that case (A7).
- **An owner that takes an earlier flow's end for a later one releases the
  wrong flow.** The redirect reused drain-cell slots and needed a cell epoch
  on every record (A2).
- The owner waits on several inputs at once (listeners, control sessions,
  the forwarder), so the channel must be a descriptor it can poll.

## Decision

- **Every pair and watch produces exactly one end record** in the context
  that attached it — completed, aborted with a cause, or ended by the
  owner's abort — unless the context is released first (ADR-0199). The
  record's storage is allocated with the pair or watch, so queueing it
  cannot fail and no record is dropped.
- **Ids are 64-bit and never reused within a context**, so no record of an
  earlier pair can be taken for a later one and no epoch is needed.
- **The owner waits on the context's descriptor** (`poll` readable while a
  record is queued) and reads whole fixed-size records, oldest first, with
  `read`. A read record is freed.
- **The owner's own abort of an id queues that id's record before the abort
  returns**, so the owner's view after `abort` and the records it reads
  agree.

## Alternatives considered

- **Generic netlink with a multicast end notification.** Multicast delivery
  drops on a full socket buffer (`ENOBUFS`), which would bring back a loss
  count and an abort rule. Rejected.
- **A fixed-size ring of records.** The redirect's shape: bounded memory, but
  a full ring drops records. Rejected.
- **An `eventfd` per pair.** Two descriptors per flow against the owner's fd
  budget, and still no cause. Rejected.
- **Polling `/proc` or the inventory.** A period adds latency to every end
  and gives no cause. Rejected.
- **Reused ids with an epoch per record.** The redirect's A2: every consumer
  must compare epochs, and a missed comparison releases the wrong flow. A
  64-bit id never wraps within a context's life. Rejected.

Measured on Linux 7.0 (P-70, `spike/findings-ev-ctl.md` § 3.3–3.4, § 3.8):
16,384 pairs ending at once in six storm shapes gave exactly one record per
id (0 lost, 0 duplicated); ids were strictly increasing and never reused
across 1,000,001 attaches and watches, with exactly one record per issued
id; records are read whole, oldest first, and `poll` is readable while one
is queued.

## Consequences

- The owner releases a flow's places exactly when it reads the flow's end
  record; there is no loss path and no counter read on a period.
- The kernel holds one queued record per ended, unread pair; an owner that
  stops reading holds the memory of the pairs it ended, bounded by the pairs
  it attached (the node flow capacity on the host).
- The record is a new ABI between C and Rust; a layout mismatch is a boot
  refusal through the probe's version check, and a Tier-3 test pins the
  layout and the one-record-per-id rule under a close storm.
- A context's release queues no record; the pairs it ends are accounted by
  the owner's own end (the process exit, or the `Forwarder` recovery, which
  aborts every flow first; ADR-0199).
