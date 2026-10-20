# ADR-0159 — Forwarding kernel objects are unpinned and owned by their process; process exit fails closed and closes the flows

## Status

**Accepted.** Decision D19 approved by the user 2026-10-05; no runtime audit
of the forwarding links (D19-LINKS) approved 2026-10-08; NO-SURVIVE (VMs do
not survive a `serve` restart until #312) approved 2026-10-11; the in-kernel
forwarder (ADR-0197, EVMOD-ADOPT) approved 2026-10-16 and its pairs' binding
to their control context (ADR-0199, EVMOD-CTL) approved 2026-10-17. GH #295.
Recorded in the #295 feature delta, § *[REF] Ownership of descriptors and
kernel objects*. Surviving a `serve` restart with VMs and flows intact is
#312. Not yet implemented.

## Context

ADR-0115 pinned the TAP classifier's maps and links in bpffs and adopted them
at boot; ADR-0137 then had to converge the dynamic members back to empty.

Forwarding state on each side is of two kinds:

- **Forwarded pairs.** Every pair belongs to the forwarder context that
  attached it, and the context's release aborts every live pair of it before
  it returns (ADR-0199). All host pairs belong to the owner process's one
  context (ADR-0151) and all guest pairs to `overdrive-init`'s (ADR-0150).
- **Guest capture programs** (`connect4`, `sendmsg4`, `getpeername4`,
  `recvmsg4`, `sock_release`) and their maps. A BPF link detaches when its
  last fd closes (the link file's release drops the last reference,
  `kernel/bpf/syscall.c:3347-3353 @ v7.0`, and a cgroup link's release
  detaches it, `kernel/bpf/cgroup.c:1377-1398 @ v7.0`), while a legacy
  `BPF_PROG_ATTACH` to a cgroup stays in the cgroup's program list until it
  is explicitly detached or the cgroup itself is released
  (`kernel/bpf/cgroup.c:316-327 @ v7.0`).

The guest-capture spike killed the host owner mid-transfer: the in-flight
application saw `ECONNRESET`, new connects were reset, DNS failed, and the
owner recovered after restart. V-EV-CTL (P-70) measured a context released by
`close()`, `kill -9` or `exit` resetting every far TCP end before the
parent's `waitpid` returned. On the guest side the owner is
`overdrive-init`, the guest's PID 1: its death panics the guest, the guest
resets, and the reset ends the VMM (ADR-0182); the release of the VMM's
vhost-vsock device then resets every host flow of that guest (ADR-0146,
V-30).

## Decision

- Every forwarding object, on host and guest, lives exactly as long as its
  owner process: no bpffs pin, no pair that outlives its context, no context
  shared with or inherited by another process (`O_CLOEXEC`, ADR-0199).
- **Every guest capture attachment is a BPF link** held by
  `overdrive-init`. No attachment uses an API that survives the process.
- When the owner exits, forwarding stops and every flow the owner held
  closes. No guest flow reaches any peer afterwards.
- Boot never adopts forwarding state. A `serve` restart ends every VM: boot
  reclamation kills every VMM the new process does not supervise before any
  admission (ADR-0083 §D7), and forwarding is rebuilt as VMs relaunch. A box
  reboot ends every VM likewise.
- **No runtime audit of the links (D19-LINKS).** While the owner holds a
  link's descriptor, only an explicit detach removes the link, and on the
  appliance only Overdrive writes these objects (ADR-0068). The capture
  links are therefore not supervisor components: no audit reports them
  damaged and no recovery quiesces forwarding for them. The host forwarder
  context is a supervisor component (ADR-0151, ADR-0199).

## Alternatives considered

- **Pin the objects and adopt them at boot.** The pins would outlive the
  sockets, adding a stale-state class that buys nothing until #312. Rejected.
- **Pairs that outlive the owner process.** Forwarding would continue while
  `serve` is down. Rejected (ADR-0199).
- **Legacy cgroup attach.** Would leave hooks running after the owner exits,
  steering traffic to sockets that no longer exist. Rejected.
- **Audit the links at runtime and recover a missing one under quiescence.**
  No producer of a missing link exists on the appliance while the owner
  holds it; the recovery would be reached only by a test that detaches a
  link itself, and would cost quiescence machinery for every flow on the
  node. Rejected.

## Consequences

- Owner-process loss fails closed by construction (V-8 shows it in the
  production composition).
- `sweep_stale` reduces to verifying that no residue exists.
- The startup probes still check the forwarder context (Earned Trust) and,
  in the guest, that every capture link attached at load.
- A `serve` restart closes every guest flow and ends every VM. Keeping VMs
  running across a restart is #312.
