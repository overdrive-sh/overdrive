# ADR-0156 — A VM's vsock CID is derived from its admission lease, not allocated or claimed separately

## Status

**Accepted — decision D16 approved by user 2026-10-05, with next-fit assignment
confirmed by user 2026-10-06;
CID-NOCLAIM (no kernel CID claim; uniqueness from the lease and boot
reclamation) approved by user 2026-10-11.** GH #295.
Amends on acceptance: ADR-0118 (assignment order of the address pool).
Recorded in the #295 feature delta, § *[REF] vsock Attachment Replacement
DESIGN*. Depends on ADR-0146.

## Context

Kernel vhost-vsock requires every guest CID to be unique on the host:
`VHOST_VSOCK_SET_GUEST_CID` refuses a CID another device instance holds with
`EADDRINUSE` (`drivers/vhost/vsock.c:808-813 @ v7.0`). A CID is free again
when the device file holding it is released at its last close
(`drivers/vhost/vsock.c:753-760 @ v7.0`); the CH fork spike observed the CID
reusable immediately after the holding VMM exited
(`spike/ch-vhost-vsock-findings.md`, P5, P7, 20/20). The VMM is the only
holder of its device file (ADR-0146). Today every VM uses CID 3, which works
only because each CH instance has its own Unix backend.

ADR-0132 and ADR-0133 make the guest-address lease the single admission
authority, held from assignment until cleanup finishes, including while it is
Retiring. One pool exists per server; nothing is persisted or adopted at boot
(ADR-0118).

A VMM can outlive the `serve` process that launched it only across a `serve`
crash. Boot reclamation kills every VM the new process does not supervise
before the new process admits any workload (ADR-0083 §D7,
`vm_reclamation_boot::converge`, which removes each reclaimed VM's cgroup
scope; VMs do not survive a `serve` restart until #312, ADR-0159).

## Decision

Each guest's CID is `GUEST_CID_BASE + lease_offset`, where `lease_offset` is
the offset of the allocation's `workload_addr` within the node's guest prefix.

- The CID is computed in exactly one place: the pool's `assign`.
- It is never persisted, and no Overdrive process claims it on the kernel.
  The VMM sets it on its own device (ADR-0146).
- The offset is free again only when the lease is released, and the lease is
  released only after the allocation's VMM has exited.
- **Next-fit assignment.** `assign` considers free offsets starting at an
  in-memory cursor that advances past each assigned offset and wraps. The
  cursor is not persisted; it starts at the first offset on each boot.
- **Boot reclamation precedes every `assign`.** Boot reclamation runs to
  completion before the first `assign` of the process.

CID uniqueness among Overdrive's VMs follows from these rules. Within one
`serve` process, the lease is unique over Admitted and Retiring leases and
outlives its VMM, so no two live VMMs of the process share a CID. Across a
`serve` restart, the previous process's VMMs are killed by boot reclamation
before any lease is assigned. A killed VMM's device is released before its
cgroup can be removed: the exiting VMM drops its file table
(`kernel/exit.c:971 @ v7.0`, `fs/file.c:518-526 @ v7.0`) and runs the
deferred final `fput`, which calls the device's release
(`kernel/exit.c:976 @ v7.0`; `fs/file_table.c:528-531` and `:468-469 @ v7.0`),
before the task leaves its cgroup in `finish_task_switch`
(`kernel/sched/core.c:5193 @ v7.0`, `kernel/cgroup/cgroup.c:7072-7091 @ v7.0`);
a cgroup with a member task cannot be removed
(`kernel/cgroup/cgroup.c:6168-6169 @ v7.0`). The VMM's kernel vhost worker
holds no file table (`kernel/vhost_task.c:130 @ v7.0`).

## Alternatives considered

- **A separate CID allocator.** A second admission authority that can drift
  from the lease. Rejected.
- **A random CID with retry on collision.** Non-deterministic, breaking DST
  seed reproducibility. Rejected.
- **First-fit assignment.** Reuses the lowest free address and CID
  immediately after release. Next-fit spreads reuse across the prefix.
  Rejected in favour of next-fit (M-9).
- **Claim the CID on the host kernel at assignment.** Defends against a VMM
  the pool does not lease holding a CID, which boot reclamation removes
  before any assignment; the claimed descriptor would also have to reach the
  VMM without `serve` keeping a copy (ADR-0146). Rejected (CID-NOCLAIM).

## Consequences

- One lease names two derived identities, `workload_addr` and CID, released
  together and last (ADR-0133).
- `workload_addr` reuse is spread across the prefix instead of always taking
  the lowest free address.
- A launch whose CID is held by another device instance exits pre-READY
  through the VMM's typed start failure (ADR-0146). Under the rules above no
  production path produces that state.
- The derivation is a pure function. The assignment discipline is covered by
  the feature delta's § *Formal protocol model*; the ordering of boot
  reclamation before assignment, and of the device release before cgroup
  removal, is a validation item of the feature delta's validation plan
  (V-RECLAIM).

## Amendment 2026-11-23 — the lease outlives an unreaped VMM (accepted 2026-11-23)

Approved by the user on 2026-11-23 as decision 12 of GH #348 (option a): a
guest-address lease is released only once the allocation's cgroup scope is
observed absent; while the scope is present the lease is retired and held, and
the stop still completes.

Amended by ADR-0348 (decision 11 of GH #348). ADR-0348 makes a stop whose
VMM is killed but not yet reaped a completed stop. The text above is kept as
the contract of the implemented code; the amendment changes three of its
claims.

- **"The lease is released only after the allocation's VMM has exited"**
  becomes: only after the VMM has been *reaped*, observed as the allocation's
  cgroup scope being absent. A stop that ends with the VMM unreaped retires
  the lease and keeps it Retiring, so its offset stays held and next-fit
  skips it. The release happens on the network-reclaim path after the reap.
  The process holds the device until its last descriptor closes
  (`vhost_vsock_dev_release`, `drivers/vhost/vsock.c:753-760 @ v7.0`), and the
  scope can be removed only when no task of it remains
  (`kernel/cgroup/cgroup.c:6254-6272`, `:6309-6311 @ v7.0`).
- **"Under the rules above no production path produces that state"** (a launch
  whose CID is held) stays true only with the release gate: without it, a
  stop of an unreaped VMM followed by a next-fit wrap produces it. The holder
  set the lease covers is now Admitted, Retiring, and *ended but not yet
  reaped*; the last is a Retiring lease.
- **Lease lifecycle:** Retiring now lasts until the VMM is reaped, which can
  be much later than the stop (or until reboot if it never reaps). A Retiring
  lease still counts against the attachment cap. Boot reclamation is
  unchanged: after a restart no lease exists, and reclamation completes only
  after every unsupervised VMM's scope is removed.

The mechanism is pinned in the feature delta of the node-local persistent
volume, § *A VMM kill's reap wait is bounded*, item 11. Alternatives
(release at stop; a separate held-CID set; release owned by `VmReclamation`)
are recorded in ADR-0348.

## Amendment 2026-11-23 (second) — "scope absent" means removed after empty; boot reserves a stuck VM's offset (accepted 2026-11-23)

Approved by the user on 2026-11-23 as decisions 16 and 14 of GH #348. This
ADR is implemented, so the earlier text stays as the contract of the shipped
code and this section states what changes.

- **The release gate (changes the first amendment).** The first amendment
  gates release on the allocation's scope being observed absent, on the
  premise that a scope can be removed only once no task is left. The spike of
  2026-11-23 showed that on the running kernel `rmdir` removes a scope whose
  last task is stuck in exit, while the task still holds the device. The gate
  is now: the scope is absent *and* the removal was the populated-first removal
  of ADR-0349, which removes a scope only after `cgroup.events` reports
  `populated 0`. Every scope removal in Overdrive takes that order, so absence
  is a proof of emptiness. The citations for the exit ordering above
  (`kernel/cgroup/cgroup.c:6168-6169`) describe the `v7.0` source; the design
  does not depend on whether `rmdir` sleeps or succeeds early.
- **"Boot reclamation precedes every `assign`" (changes the Decision).** Boot
  reclamation no longer refuses the boot for a VMM it cannot remove. It waits
  the bound, and for each scope still populated it reserves that VMM's offset
  in the pool before the first `assign` (ADR-0350). The invariant that
  matters stands in a different form: before the first `assign`, every offset
  held by a leftover VMM is either removed or reserved, so no launch is handed
  a CID a leftover VMM holds.
- **"Under the rules above no production path produces that state."** Stays
  true with the reservation. The holder set the pool covers is now Admitted,
  Retiring, ended-but-not-reaped (a Retiring lease), and reserved-at-boot
  (also a Retiring lease, counted toward the attachment cap).
- **One derivation.** The CID is still computed in exactly one place: the
  pool's offset-to-plan derivation, which `assign` and the boot reservation
  both use.
- **The address is on record before the VMM.** Approved by the user on
  2026-11-23 (ruling N-2). After `assign` hands out an offset and the network
  is provisioned, and before the VMM can be created, the allocation's row is
  written in the existing `Pending` state with `workload_addr` set; every later
  row of the allocation carries it forward. Boot reads the offset to reserve
  from that row, so every VMM that can hold a CID has its address on record,
  including a launch parked in guest boot at a crash. The VMM's command line is
  not read. The row's persisted format does not change.

The contract is in the feature delta of the node-local persistent volume,
§ *Boot proceeds past a stuck VM*. Alternatives are recorded in ADR-0350.
