# ADR-0159 — Forwarding programs, maps and attachments are unpinned links owned by their process; process exit fails closed and closes the flows

## Status

**Proposed — approved by user (ruling D19 2026-10-05; no runtime audit of
the forwarding links, D19-LINKS, 2026-10-08); pending independent DESIGN
review.** GH #295. Recorded in the #295 feature delta, § *[REF]
vsock Attachment Replacement DESIGN — PROPOSED 2026-10-05*. Surviving a
service restart with flows intact is deferred to **#312**.

## Context

ADR-0115 pinned the TAP classifier's maps and links in bpffs and adopted them
at boot; ADR-0137 then had to converge the dynamic members back to empty.

A SockHash entry holds a reference to its socket, and the socket's closing
removes it. All host pair sockets belong to the owner process (ADR-0151) and
all guest pair sockets to `overdrive-init` (ADR-0150), so neither can outlive
its process. A pinned route or tuple map would outlive them.

Program attachments differ by API: a BPF link detaches when its last fd
closes, while a legacy `BPF_PROG_ATTACH` to a cgroup persists until explicitly
detached.

The guest-capture spike showed process loss on each side:

- host owner killed mid-transfer: the in-flight application saw
  `ECONNRESET`; new connects were reset; DNS failed; both owners recovered
  after restart;
- guest owner killed mid-transfer: cgroup hooks were gone; new TCP and UDP
  failed with `ENETUNREACH`; host servers saw no new connection; the in-flight
  application saw a clean EOF with 0 bytes (ADR-0160 addresses this).

## Decision

- Every forwarding program, map and attachment, on host and guest, is created
  without bpffs pins and lives exactly as long as its owner process.
- **Every attachment is a BPF link** held by the owner: TCX, `sock_ops`,
  cgroup socket-address hooks, `sock_release`, SK_SKB on SockHash maps, and
  `fexit`. No attachment uses an API that survives the process.
- When the owner exits, forwarding stops and every flow the owner held closes.
  No guest flow reaches any peer afterwards.
- Boot never adopts forwarding state. A box reboot ends every VM; forwarding
  is rebuilt as VMs relaunch.
- **No runtime audit of the links (D19-LINKS).** While the owner holds a
  link's descriptor, only an explicit detach removes the link, and on the
  appliance only Overdrive writes these objects (ADR-0068). The `lo` unframe,
  `sock_ops` and drain-counter links are therefore not supervisor components:
  no audit reports them damaged and no recovery quiesces forwarding for them.
  The one kernel producer of a link's disappearance — the kernel removing the
  interface a TCX link is attached to — removes that interface's traffic
  with it (ADR-0165).
- Scope: forwarding objects. The guest-prefix steering program, link and map
  (ADR-0171) forward nothing; they decide local delivery for the guest prefix
  and are node infrastructure, pinned so that they outlive `serve` as the
  shared `local` route does (ADR-0152). Their map holds only open listeners,
  so process exit still fails closed.

## Alternatives considered

- **Pin the objects and adopt them at boot.** The pins would outlive the
  sockets, adding a stale-state class that buys nothing until #312. Rejected.
- **Pin only the programs.** Programs without their maps carry no state worth
  keeping. Rejected.
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
- The startup probe still checks that every link attached at load (Earned
  Trust); after that, link presence is a property of the owner holding them.
- A control-plane restart closes every guest flow; VMs keep running and open
  new flows once the owner returns.
