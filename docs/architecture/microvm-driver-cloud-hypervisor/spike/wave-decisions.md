# SPIKE decisions — `microvm-driver-cloud-hypervisor`

Governed by `.claude/rules/spike.md`. Evidence: `spike/findings.md`.
Date: 2026-08-02.

## Assumptions tested

| Probe | Assumption |
|---|---|
| P1 | The kernel boots under Cloud Hypervisor from an ext4 `virtio-blk` rootfs. |
| P2 | vsock carries a ready beacon and an exit status — **including when the VMM is in a per-workload netns and the listener is not.** |
| P5 | The `[D7]` confinement flags (Landlock, uid/gid drop, rlimits, default seccomp) compose with a real boot. |
| P6 | `virtiofsd` + `--memory shared=on` compose with the boot **and** with the confinement. |
| P4 | Per-launch rootfs copy cost (reflink vs full copy). |
| P7 | Do virtio-blk volumes work as well as virtiofs? (the I-6 counterfactual) |
| P8 | Does snapshot/restore work, and does CPU hotplug survive it? |
| P9 | Do volumes survive a checkpoint — block AND virtiofs? |
| P10/P11 | Does `vhost-user-blk` work, and what does it cost? |
| P12 | Does vsock survive a checkpoint? |
| P13 | Does `memory_restore_mode=ondemand` deliver warm-pool density? |
| P14 | Is the memory snapshot discardable — is the filesystem alone authoritative? |
| P3 | P1/P2 hold on the pinned 6.18. — **still not run** |

## Verdicts

| Probe | Verdict |
|---|---|
| **P1** | **WORKS.** Boots to `Run /init as init process`, PID 1 confirmed. On aarch64 the distro `vmlinuz` must first be unwrapped (UKI → EFI-zboot → zstd) to a raw `Image`; on x86_64 a `bzImage` loads directly. |
| **P2** | **WORKS, both placements.** Beacon + `EXIT 7` received in order, as two distinct reads, with `guest net ifaces = [lo]` — i.e. before any guest networking. Structural: CH's vsock host end is a UNIX socket, which is not network-namespaced. |
| **P5** | **WORKS**, subject to three corrections (below). uid question settled: unprivileged uid + `kvm` group against `0660 root:kvm`. |
| **P6** | **WORKS** on bare-metal x86_64 (was unprovable on the nested host). `[D8g]`'s host-side `read_only` is **verified** — a guest that mounts the export read-WRITE is refused with `EROFS` and the host tree is untouched. aarch64 still unmeasured. |
| **P4** | **WORKS** — reflink ~260× cheaper than a copy and free in space. Constraint: reflink is intra-filesystem, so per-VM disk images must live on the same filesystem as their master. |
| **P7** | **BOTH WORK.** Neither wins on speed (block faster streaming, virtiofs faster per-file, non-overlapping both ways). Live host access is the only axis that cannot be engineered around. |
| **P8** | **WORKS via the API**; the CLI `--restore` is a silent no-op. **CPU hotplug composes with restore** — the reason this feature chose CH survives. |
| **P9** | **BOTH SURVIVE** a checkpoint, virtiofs included (with a fresh daemon). Overturns the research's "virtiofs cannot checkpoint" premise. |
| **P10/P11** | `vhost-user-blk` **works and the transport is free**, but it requires `shared=on` and forfeits rate limiting exactly like `vhost-user-fs`. Those are **vhost-user** properties, not block ones. |
| **P12** | **VM restores, the connection does not** — and one post-restore `send()` succeeds while being silently discarded. Readiness must be host-acknowledged, never inferred from a successful write. |
| **P13** | `ondemand` makes restore **O(1) in guest RAM** but is asynchronous *eager* restore, not lazy paging — and N restores share **zero** memory. **Refused under our uid-dropped shape.** |
| **P14** | **The memory snapshot IS discardable** — but only if the guest fsynced, and **`fs_quiesce` is mandatory**, not a nicety. At 4 GiB, cold boot **beats** restore. |

## Promotion decision: **PROMOTE** (revised 2026-08-10)

> **Supersedes the PAUSED decision of 2026-08-02.** That decision was correct at the time:
> the blocker was **nested virtualisation on Apple Silicon**, where boots stalled ~2 in 3
> and `--memory shared=on` never booted at all (0/18), reproduced under QEMU/KVM on the
> same host so it was never a CH defect. **Bare-metal hardware removed that blocker**
> (`infra/metal/`), and P1/P2 now reproduce 12/12 with a 16 ms spread. Everything the
> PAUSED decision said "must be re-run on bare metal" has been run.

**PROMOTE, with the feature's scope narrowed — see below.** Nothing was refuted. Every
mechanism Slices 01–05 depend on is measured on non-nested hardware at the shipping
CH version.

## Scope for the DESIGN wave: **boot a VM through `serve` + `deploy`. Nothing else.**

**User decision, 2026-08-10.** The spike ran well ahead of the feature it belongs to. #42
is the `vm` **driver**: an operator writes a TOML spec, `overdrive deploy` submits it,
`overdrive serve` boots a Cloud Hypervisor VM, the workload runs, the exit status is
honest, and stop/restart behave. That is Slices 01–05, which already exist and have not
been touched since DISCUSS.

Checkpoint/restore, persistent rootfs, warm pools, the chunk store and the guest agent
belong to **#96 / #97 / #100** and are explicitly OUT of this feature's design.

This split is deliberate and matches CLAUDE.md § *"Build vertical slices through
production entry points"*: the first cut must close a real loop through `serve` +
`deploy`, and everything that cannot be driven that way is a later, separately-drivable
slice.

### Binds THIS feature (#42) — the DESIGN wave must carry these

| Finding | Consequence for the driver |
|---|---|
| P5 correction 1 | The vsock UDS needs an explicit per-VM `--landlock-rules` **directory** grant; CH does not auto-derive it, a read-only rule fails, and the rule cannot name the socket path. Each VM's socket dir must hold nothing else. |
| P5 correction 2 | Seccomp is verified **per-thread**; the thread-group leader reports `Seccomp: 0` on a correctly-confined CH. An AC against `/proc/<pid>/status` would fail against correct behaviour. |
| P5 correction 3 | `RLIMIT_FSIZE` = `max(rootfs image, guest RAM)` whenever `shared=on` is in play; sizing off the rootfs alone kills the VM with an opaque `SIGXFSZ`. |
| **P10/P11** | `image_type=raw` is **mandatory** on every `--disk` from v53. The auto-detect fallback *"disables sector 0 writes"*, and our bare-filesystem images fault and reboot two layers from the cause. |
| P4 | Reflink is **intra-filesystem**. Per-VM disk images must be staged on the same filesystem as their master — a driver that stages into a tmpfs run dir silently loses the ~260× win, and `--reflink=auto` degrades to a full copy with no error. |
| `[D5]` | CH's kernel-rejection path is misleading on every arch: an unloadable `--kernel` is reinterpreted as UEFI firmware and reported as a size cap. The driver must surface a format error naming the real problem. |
| P1 | On aarch64 the distro `vmlinuz` must be unwrapped (UKI → EFI-zboot → zstd) to a raw `Image`; x86_64 `bzImage` loads directly. |
| P2 | Guest→host vsock needs no handshake and is not network-namespaced; `/dev/console` must exist statically in the rootfs image; `CONFIG_VSOCKETS=m` means three modules load in order before the beacon. |
| P11 | Host `shmem_enabled=advise` is worth ~2× on **every** `shared=on` path. An appliance-image decision, flagged not taken. |

### Banked for #96 / #97 / #100 — do NOT design against these now

P8 (snapshot/restore + CPU hotplug), P9 (volumes across checkpoint), P12 (vsock across
checkpoint — readiness must be host-acknowledged), P13 (`ondemand` is eager not lazy; no
pool density; refused under uid-drop; needs `vm.unprivileged_userfaultfd=1`), P14
(**memory is discardable, `fs_quiesce` is mandatory, cold boot beats restore at 4 GiB**),
and the I-6 storage-seam analysis (`vhost-user-blk` vs `vhost-user-fs`, and the finding
that #97's own single-writer constraint argues for block). All are recorded in
`findings.md` with pasted evidence and stay there until those features open.

### Banked from the nested host — did NOT need re-running (historical)

Mechanism-level, arch-independent, and independent of the nesting problem:

- The vsock beacon design and its netns behaviour (`[D2]`, `[D4]`).
- `CONFIG_VSOCKETS=m` vs `=y` — the appliance kernel should build vsock **in**, or
  `overdrive-init` must load three modules in order before the beacon.
- `/dev/console` must exist statically in the rootfs image.
- Guest→host vsock needs no handshake; `RB_POWER_OFF` → PSCI → CH exits 0.
- `mkfs.ext4 -d` builds the rootfs with no loop mount and no root.
- The three P5 corrections and the settled uid answer.

### What that list demanded, and what happened to it

1. ~~**P6 in full**~~ — **DONE.** All five modes pass; `[D8g]` verified.
2. **P3** — the pinned 6.18 under CH, on **each** shipping arch. **STILL NOT RUN.** Belongs
   on CI (the LVH kernel-matrix path), not on this box.
3. ~~**P4**~~ — **DONE.** ~260×, plus the intra-filesystem constraint above.
4. ~~**A confirmation pass on P1/P2/P5**~~ — **DONE.** 12/12 boots, 16 ms spread; the
   nested-host caveat is retired for P1/P2. P5's stack ran through five completed boots;
   only correction 1's *negative* case (omitting the vsock grant) was not re-derived.

**Still open, and none of it blocks #42:** P3; aarch64 for P6 and for `shared=on`
generally (no non-nested Arm hardware); cross-host restore; and host→guest vsock across a
restore (P12 measured guest→host, which is the direction the Running gate uses — #100's
agent uses the other one).

## Corrections the DESIGN wave must carry

`[D7]` / US-VM-7 — all three measured, each of which would otherwise have been a Slice 03
defect:

1. **The vsock UDS needs an explicit per-VM `--landlock-rules` directory grant.** CH
   auto-derives rules for `--kernel`, `--disk`, `--serial file=` and `--api-socket`, but
   **not** for the vsock socket it binds itself. Failure is
   `CreateVsockBackend(UnixBind(EACCES))`, which never mentions Landlock. A read-only rule
   is insufficient, and the rule **cannot name the socket path** (CH validates path
   existence at config-parse time, before the socket exists) — so the grant must be the
   containing directory, and **each VM's socket directory must hold nothing else.**
2. **Seccomp must be verified per-thread** (`/proc/<pid>/task/*/status`). The thread-group
   leader reports `Seccomp: 0` on a *correctly* confined CH; the filters live on `vmm`,
   `http-server`, `vcpu0` et al. An AC against `/proc/<pid>/status` would fail against
   correct behaviour.
3. **`RLIMIT_FSIZE` must be `max(rootfs image, guest RAM)`** whenever `shared=on` is used.
   `shared=on` backs guest RAM with a memfd, and a memfd is a *file* for `RLIMIT_FSIZE`;
   sizing off the rootfs alone kills every volume-carrying VM with an opaque `SIGXFSZ`.

`[D5]` — CH's kernel-rejection path is misleading on every arch: an unloadable `--kernel`
is silently reinterpreted as UEFI firmware and reported as a size cap. The driver must
surface a format error that names the actual problem.

`[D8]` — `[D8e]` (volumes do not widen hypervisor confinement) and `[D8d]`
(`--sandbox=namespace` in effect, mount+net only) are **confirmed**. **`[D8g]`'s host-side
read-only security framing is UNVERIFIED and must not be asserted until P6 runs.**

## Constraints discovered

- **The Lima dev VM cannot gate microVM boot.** A green run is genuine evidence; a red run
  is uninformative, because a real regression and a nested-virt stall are
  indistinguishable. Usable for proving a mechanism works, useless for catching a
  regression.
- **Do not tune the Lima environment.** Apple ships nested virt as a single boolean with
  no compatibility contract, no errata channel and no known-issues list, so every candidate
  mitigation is trial-and-error against a black box. See
  `docs/research/testing/trustworthy-tier3-gate-for-microvm-boot-research.md`.
- **`.claude/rules/testing.md:1532`** specifies the per-release aarch64 tier on a
  "self-hosted Graviton runner". A non-`.metal` Graviton instance **cannot run KVM at
  all** — AWS does not expose virtualization extensions to Graviton guests. Since aarch64
  is a shipping target, that line needs `*.metal`, not deletion.
- ~~Installed CH is **v46.0.0**~~ — **now v53.0.** The v46 pin sat for 14 months and 7
  releases because a comment in `versions.env` argued that "v46 demonstrably has the
  capability we actually need". That reasoning was the bug: the right answer to an
  unjustified floor is to take **latest**, not to keep the oldest build that passes
  today's tests. Every version gate in this repo checks `installed == pinned` and none
  checks `pinned == latest`, so pins rot behind a green board. See
  `infra/provision/versions.env`, which now leads with the rule.

## Artifacts

- `spike/findings.md` — full evidence, pasted output, per-probe verdicts.
- `spike-scratch/increment-{a,c,d,e,f,g,h,i,j,k,l}/` — probe code and raw evidence
  captures across 14 probes. **`spike-scratch/` is gitignored per `.claude/rules/spike.md`, so these
  are NOT committed and will not survive a workspace clean.** The load-bearing extracts
  are reproduced in `findings.md`; the remainder is reconstructible by re-running the
  probes on the target hardware, which must happen anyway.
- `crates/` untouched throughout — verified per increment.
