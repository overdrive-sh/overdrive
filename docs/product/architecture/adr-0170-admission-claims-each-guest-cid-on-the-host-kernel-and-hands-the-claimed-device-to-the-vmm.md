# ADR-0170 — Admission claims each guest CID on the host kernel and hands the claimed vhost-vsock device to the VMM

## Status

**Proposed — decision D16-CLAIM, approved by user 2026-10-06 (direction:
correct design over simple); pending independent DESIGN review.** GH #295.
Recorded in the #295 feature delta, § *[REF] vsock Attachment Replacement
DESIGN — PROPOSED 2026-10-05*. Depends on ADR-0146 (kernel vhost-vsock
backend of the vendored Cloud Hypervisor) and ADR-0156 (the CID is derived
from the admission lease). Kernel facts the decision rests on are validation
item V-25.

## Context

Kernel vhost-vsock requires every guest CID to be unique on the host. The
host kernel enforces this itself: `VHOST_VSOCK_SET_GUEST_CID` takes the CID
for the calling `/dev/vhost-vsock` instance under the kernel's
`vhost_vsock_mutex`, and fails `EADDRINUSE` when another instance (or the
guest-to-host transport of a nested setup) already holds it. The claim lasts
until the last reference to that open file is closed. The CH fork spike
observed both halves: a second VM launched with a held CID exited with
`Failed to assign guest CID 101 to vhost-vsock` caused by `EADDRINUSE`, and
the CID was free again immediately after the holding VMM exited
(`spike/ch-vhost-vsock-findings.md`, P5, P7).

Other software on the host can use vhost-vsock (another VMM, a test harness,
a VM that survived a crashed `serve`). Without a claim of its own, Overdrive
learns that a leased CID is taken only when Cloud Hypervisor fails at device
creation and exits before READY. No owner reads CH's diagnostic text, so that
exit cannot be told apart from any other pre-READY exit. Every rule built on
that inference — remember which offsets failed, avoid them, fall back when all
are remembered — guesses at a fact the kernel can state exactly.

The vendored fork opens `/dev/vhost-vsock` itself and claims the CID at
device creation (`virtio-devices/src/vsock/vhost_kernel.rs`). The forwarder
startup probe already opens `/dev/vhost-vsock` and sets a CID from inside the
`overdrive serve` process (stage `VhostCidRegister`).

## Decision

- **Claim at assignment.** The admission pool's `assign` takes a lease only
  together with a kernel claim on its CID: it walks free offsets in next-fit
  order and, for each, asks the host kernel to claim
  `GUEST_CID_BASE + offset` on a new `/dev/vhost-vsock` instance
  (`VHOST_VSOCK_SET_GUEST_CID`, no `VHOST_SET_OWNER`). The first successful
  claim becomes the lease. The claim and the assignment are one step: no
  lease exists without its claim, and no check is separate from the claim.
- **`EADDRINUSE` is the only skip.** An offset whose claim fails
  `EADDRINUSE` is held by another vhost user at that moment; `assign` moves
  to the next free offset. Nothing about it is remembered: the next `assign`
  asks the kernel again. Any other claim failure stops `assign` with a typed
  error naming the CID and the cause; it is never treated as "held".
- **Typed refusal with the true cause.** If every free offset is held by
  other vhost users, `assign` refuses with its own non-terminal error, not
  the pool-exhaustion refusal. The exhaustion refusal still means no free
  offset at all.
- **The claimed device travels to the VMM, once.** The claim is a move-only
  handle carried in the allocation's transport handoff. The VMM adapter takes
  it at launch and passes the claimed file to the fork as an inherited
  descriptor (`--vsock cid=N,backend=vhost-kernel,fd=K`), the same pre-exec
  descriptor handoff the TAP queue path used (ADR-0128, superseded by
  ADR-0146 for its TAP content). The fork does
  not open `/dev/vhost-vsock` or set the CID in this mode; it becomes the
  device owner (`VHOST_SET_OWNER`) and starts the rings. `overdrive serve`
  closes its own copy as soon as the VMM process holds the file, so the claim
  then lives exactly as long as the VMM.
- **Release.** A claim never handed to a VMM (launch not reached, VMM spawn
  failed) is released when the lease releases it or the handle is dropped. A
  handed-off claim is released by the VMM's exit, as today. The CID offset
  stays leased until cleanup completes (ADR-0133); a CID taken by another
  vhost user in between is found held at the next claim and skipped.
- **Earned Trust.** At startup a claim probe opens `/dev/vhost-vsock`,
  claims `GUEST_CID_PROBE`, verifies that a second instance claiming the same
  CID fails `EADDRINUSE`, closes the first, verifies that the CID can be
  claimed again, and closes everything. Any deviation refuses startup.

The exact port, handle, error variants and launch argument are pinned in the
feature delta (§ *Core vocabulary*, § *VMM backend contract*, § *Composition*).

## Alternatives considered

- **Infer a clash from a pre-READY VMM exit and remember failed offsets per
  workload.** Every pre-READY exit is
  treated as a possible clash, so the rule needs an exclusion set, a
  preference order and a fallback for when every free offset is excluded; the
  fallback re-launches onto known-clashing CIDs, and the guarantee resets at
  every `serve` restart. It guesses at a fact the kernel states. Rejected.
- **Check the CID before launch, then let CH claim it.** A check separate
  from the act: another vhost user can take the CID between the check and
  CH's claim. Rejected (`.claude/rules/rust.md` § "Check-and-act must be
  atomic").
- **Read CH's diagnostic text for `EADDRINUSE`.** Makes an unstructured
  string a contract, and the clash still costs a VMM launch. Rejected.
- **`serve` keeps its copy of the claimed file until the lease is released.**
  The CID would stay Overdrive's through cleanup, but vhost holds the owning
  VMM's address space (`get_task_mm`) until the device's last reference
  closes, so a dead VM's guest memory would stay allocated for as long as its
  cleanup is pending. Rejected: losing the CID to another vhost user during
  cleanup is harmless, because the next claim finds it held and skips it.
- **A separate CID allocator.** A second admission authority beside the lease
  (ADR-0156 alternatives). Rejected.

## Consequences

- A launch never fails because its CID is held elsewhere. Pre-READY exits for
  any other cause no longer influence which offset a later `assign` returns.
- The guarantee holds across `serve` restarts with no in-memory state: after a
  crash, a VMM that survived still holds its CID, and the new process's
  `assign` skips it.
- `assign` performs one device open and one ioctl per offset it tries; a
  pathological host with many foreign-held CIDs in the range costs one ioctl
  per held offset per `assign`, counted for observability.
- The fork gains an `fd=` mode for its vhost-kernel vsock device (a new fork
  revision, ADR-0161); in that mode CH needs no Landlock access to
  `/dev/vhost-vsock` and does not issue `VHOST_VSOCK_SET_GUEST_CID`. A guest
  reboot in that mode cannot re-create the device (the open file already has
  an owner); guest reboot is not supported today, and V-25 records how the
  VMM ends in that case.
- `/dev/vhost-vsock` access becomes a property of the `serve` identity, which
  the claim probe checks, rather than of the VMM launch identity.
- The kernel facts — a claim on an instance with no owner, the handoff of a
  claimed file to another process that then becomes its owner, exclusivity
  while any reference is open, release at the last close — are validation item
  V-25, which blocks DISTILL of the CID-claim scenarios.
- The claim discipline is checked by the formal model of the guest-flow owner
  (ADR-0168, `specs/quint/guest-flow-owner/`, module `cid_lease`).
