# DESIGN wave decisions: node-local persistent volume (GH #348)

**All decisions are PROPOSED. None is accepted until the user approves it.** Approve or reject by
number. The exact contracts are in `../feature-delta.md`; each ADR holds one decision. Scope: all
priorities (the issue's DESIGN questions 1 to 8 are answered in the feature delta).

## Decisions requested

| Id | Proposed decision | Alternatives rejected | ADR |
|---|---|---|---|
| **VD-1** | A VM workload declares at most one volume: `[vm.volume] size = "40GiB"` (`GiB` or `TiB`, at least 1); `replicas = 1` required; part of the immutable spec (envelope V2). | `[[vm.volume]]` array; flat key; byte integer; `MiB` unit and granule rule; `replicas > 1`. | 0310 |
| **VD-2** | The volume belongs to the workload (keyed by `WorkloadId`), is derived from the workload intent (no volume record, no volume verb), survives stop, restart and crash, and only withdrawing the workload intent deletes it. A stopped workload keeps its volume. | Separate volume record and verbs; key on the allocation; delete on stop. | 0311 |
| **VD-3** | Backing is a raw `dm-linear` logical volume in VG `overdrive` with extents reserved at creation, named `ovd-<32 hex of sha256(workload id)>`, outside the reflink filesystem. | File on XFS; thin LV; loop device; partition per volume; id in the name; a name registry. | 0312 |
| **VD-4** | The image provides built-in `CONFIG_BLK_DEV_DM`, `CONFIG_EFI_PARTITION` and `CONFIG_DEVTMPFS` (V-1), the `lvm` tools with their library closure (one-shot, no daemon, no udev), a wiped GPT partition labelled `overdrive-volumes`, and on it a PV and the VG `overdrive` (4 MiB extents). Overdrive sets `LVM_SYSTEM_DIR=/run/lvm` on every `lvm` spawn. Overdrive only verifies; it never creates, formats or wipes. | Overdrive creates the PV/VG on a blank partition; XFS on an LV in the same group; hand-written dm tables; an empty `/etc/lvm` in the image instead of the variable. | 0313 |
| **VD-5** | A boot probe verifies the store (tools and dm usable, VG on the labelled partition, a scratch create/node/remove); an absent partition is soft, a present but broken store refuses startup (`health.startup.refused`, `volume_backing.probe`). | Lazy probe; always refuse when absent; trust command exit status; probe kernel claim semantics every boot. | 0314 |
| **VD-6** | The volume is the second `--disk`: `image_type=raw`, `direct=on`, write-back, `sparse=true`, `serial=ovd-volume`; Overdrive never formats it; write-through not offered; the platform chowns the device node to the confined uid. | Write-through; buffered I/O; default or per-volume serial; platform formats. | 0315 |
| **VD-7a** | Fork change F1: a failed write-through fsync completes the guest's request with an I/O error on every engine; the fork pin and checksums change (once for 7a to 7c). | Upstream-only; workaround in Overdrive; not exposing write-through. | 0316 |
| **VD-7b** | Fork change F2: a writable block device is opened `O_EXCL` (the kernel claim). | Upstream-only; OFD lock alone (rejected, the alias hole, `run-031`); a second Overdrive claim path. | 0319 |
| **VD-7c** | Fork change F3: an explicit image type skips format sniffing. | Upstream-only; workaround in Overdrive. | 0320 |
| **VD-8** | A replacement's launch waits for the volume's kernel claim (one `O_EXCL` attempt, no OFD lock) to clear, for at most 30 s, not for VMM process exit; past the bound, a typed volume start failure (cause `Held`), charged to the restart budget; the claim-poll interval is the crafter's. | Process exit as "free"; budget-only retry; reconciler-side deferral; fd passing; a fork exit status; also taking the OFD lock. | 0317 |
| **VD-9** | The launch path runs an idempotent `ensure` (create if absent, activate if inactive, as after every reboot) before each launch; failures are typed start failures. No volume reconciler, status row or lifecycle gate in this slice. | Host-backed `volume-lifecycle` reconciler + `VolumeStatus` row + start gate (reviewed and model-checked, then removed); create at admission; create in the shim. | 0318 |
| **VD-10** | **Slice scope.** This slice delivers create, attach, restart, stop, reboot and describe, all driven by `serve` + `deploy`. Reclamation, erasure, a status row, a start gate, multi-node pinning and growth are not in it. Two consequences: (a) on a real appliance it is drivable only once the image provides the volume store (L5); (b) with no deletion each distinct workload keeps its volume's space permanently. Deletion's entry contract E1 to E3 is part of this decision. | Keep reclaim, zeroing, status row and gate here, Sim-only until #211 (rejected: no production trigger, roughly half the mechanism, `CLAUDE.md` vertical-slice rule). | none |

Placement pinning, growth and the relation to #97 are not decisions: they bind no code in this slice and are answers 6 to 8 in the feature delta, with the directions carried by limitations L2 and L3.

## Scope limitations to approve (each needs a tracking issue; none created)

| # | Limitation | Proposal |
|---|---|---|
| L1 | one volume per workload | new issue "Several named volumes per VM workload" |
| L2 | size fixed at creation | new issue "Grow a node-local volume while detached" |
| L3 | pinned only implicitly (single node); no multi-node pin or volume-space-aware placement | new issue "Pin a workload with a node-local volume to its volume's node" (verified: #134, #261, #282 do not cover it) |
| L4 | nothing deletes or erases a volume until workload deletion exists; a volume's state is visible only through allocation rows | existing #211: comment carrying the entry contract E1 to E3 and the status row/describe/gate decision |
| L5 | the image does not yet define the wiped partition, tools, built-in dm or VG | existing #75: comment listing the image requirements |

Verified read-only with `gh issue view --comments` and `--json body`: #43, #75, #96, #97, #134, #211, #261, #282, #320,
#337, #22 (all open; scope as cited). Not covered by an existing issue: volume affinity, disk capacity accounting, appliance
disk layout.

## Review outcome and points the user may overturn

The review history lives in git. These are the points the user may overturn, each with the proposal and what overturning it costs:

- **Slice scope (VD-10).** Reclaim, erasure, a status row and a start gate are not in this slice because workload deletion (GH #211) gives them no
  production trigger. Overturning it brings back roughly half the mechanism, drivable only by simulation until #211 lands.
- **Two fork changes a reviewer proposed dropping (evidence in ADR-0316 and ADR-0319):**
  - **F2 (VD-7b, the kernel claim) is kept; the subtractive reviewer proposed dropping it** and using the OFD lock alone, because the alias
    hole is reachable only through an Overdrive path bug. Kept: the lock is keyed on the node's inode, so a second device node boots a
    second VMM on one disk (spike B3, both alive) and with F2 the attempt fails `EBUSY` (`run-031`); the same open is what `claim_state`
    relies on. Dropping it leaves single-writer resting on a path invariant.
  - **F1 (VD-7a) is kept; the reviewer proposed not exposing a guest-writable cache mode.** Kept: the guest selects the cache mode, and
    without F1 a failed write-through fsync is swallowed (20 of 20 on io_uring and AIO, spike A3).
- **Policy values.** The 30 s claim bound, the 30 s `ensure` deadline and their worst-case lane hold of about 90 s are policy values derived from
  the measured drain (2.4 s) and the existing boot wait, not validation items; the cost is a charged typed start failure, revisited if one is
  ever observed in production.
- **"Not limitations" rulings** in the feature delta (single writer so `replicas = 1`, data lost with the node, guest owns the filesystem, the
  closed set of group name, extent size, label and serial) are proposals to confirm, not tracked issues.
- **Quint spec kept as recorded.** Two structurally unviolable invariants and one of two redundant named runs could be dropped, but changing the
  spec re-runs the 280 s exhaustive check and re-records evidence; revisit if the model is next edited.

## Validation status

- **V-1 PASSED** (contract-shaping; native metal host, Linux 7.0.0-38): five steps pass on the pinned `--config`, plus
  `LVM_SYSTEM_DIR=/run/lvm` on every spawn (chosen over an empty `/etc/lvm` in the image; ADR-0313). Evidence:
  `spike/findings.md` § "V-1", `spike/evidence/v1-001` to `v1-011`. Absorbed: the pinned invocation and command surface in the
  feature delta § Port; `CONFIG_EFI_PARTITION=y` and `CONFIG_DEVTMPFS=y` in the image requirements (ADR-0313); `ensure` activates a
  present-but-inactive volume (every reboot leaves one); the probe's measured cost (about 95 ms).
- **Quint evidence recorded and verified**: 14 of 14 checks met `expect`; `verify-evidence` passes for all eight subsystems. The spec
  needed no change for V-1, nor for the `claim_state` projection rule (a conformance-driver comparison rule in the feature delta).
- **Open**: none.

## Quint

`specs/quint/node-local-volume/`: 14 checks, all with the expected verdict (safety exhaustive over 4 allocation ids, fault budget
2; liveness; 9 named runs; 3 hazards, one per modelled decision VD-2, VD-8, VD-9). No counterexample against the design. A defect found
in a larger model (a reclaim racing a launch that passed the claim check, then a crash) is carried as entry contract E3.
