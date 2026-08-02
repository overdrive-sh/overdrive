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
| P3 | P1/P2 hold on both the dev kernel and the pinned 6.18. — **not run** |
| P4 | Per-launch rootfs copy cost (reflink vs full copy). — **not run** |

## Verdicts

| Probe | Verdict |
|---|---|
| **P1** | **WORKS.** Boots to `Run /init as init process`, PID 1 confirmed. On aarch64 the distro `vmlinuz` must first be unwrapped (UKI → EFI-zboot → zstd) to a raw `Image`; on x86_64 a `bzImage` loads directly. |
| **P2** | **WORKS, both placements.** Beacon + `EXIT 7` received in order, as two distinct reads, with `guest net ifaces = [lo]` — i.e. before any guest networking. Structural: CH's vsock host end is a UNIX socket, which is not network-namespaced. |
| **P5** | **WORKS**, subject to three corrections (below). uid question settled: unprivileged uid + `kvm` group against `0660 root:kvm`. |
| **P6** | **UNPROVEN — explicitly NOT refuted.** `shared=on` guest memory does not survive nested KVM on this host. Proven by cross-VMM diff (QEMU private 3/4 vs QEMU memfd-shared 0/4), not assumed. |

## Promotion decision: **PAUSED**

**Not PROMOTE, not DISCARD, not PIVOT.** Nothing was refuted, so DISCARD and PIVOT are
both wrong; but P6 cannot be settled here, so a clean PROMOTE would overstate the
evidence.

**Rationale (user decision, 2026-08-02):** feature development waits until bare-metal
hardware is available. The blocking constraint is **nested virtualisation on Apple
Silicon**, not the architecture and not Cloud Hypervisor:

- Boots stall ~2 in 3 attempts, always *before* `/init`.
- `--memory shared=on` — required by virtiofs, and therefore by `[D8]` volumes —
  **never boots at all** (0/18).
- Both reproduce under QEMU/KVM on the same host, so neither is a CH defect.

Continuing on this host would mean building Slices 01–04 against a mechanism (`shared=on`)
that cannot be exercised, and gating them on a test environment that cannot distinguish a
regression from a stall.

### What is banked and does NOT need re-running

Mechanism-level, arch-independent, and independent of the nesting problem:

- The vsock beacon design and its netns behaviour (`[D2]`, `[D4]`).
- `CONFIG_VSOCKETS=m` vs `=y` — the appliance kernel should build vsock **in**, or
  `overdrive-init` must load three modules in order before the beacon.
- `/dev/console` must exist statically in the rootfs image.
- Guest→host vsock needs no handshake; `RB_POWER_OFF` → PSCI → CH exits 0.
- `mkfs.ext4 -d` builds the rootfs with no loop mount and no root.
- The three P5 corrections and the settled uid answer.

### What must be re-run on bare metal before the feature proceeds

1. **P6 in full** — guest↔host round-trip, host-side `--readonly` enforcement (`[D8g]`),
   failed-mount errno, virtiofs throughput under `--cache=never`, whether `shared=on`
   changes guest `MemTotal`. Gates Slice 04.
2. **P3** — the pinned 6.18 under CH, on **each** shipping arch. Both x86_64 and aarch64
   ship, so this is two runs, not one.
3. **P4** — per-launch rootfs copy cost (`[D5]` depends on it; research Gap 2).
4. **A confirmation pass on P1/P2/P5** on non-nested hardware. The mechanism results are
   expected to hold; the point is to retire the "pinned to a nested host" caveat.

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
- Installed CH is **v46.0.0** and has `--landlock`. The reference implementation's
  "≥48.0" floor has no evidence behind it and should not be inherited.

## Artifacts

- `spike/findings.md` — full evidence, pasted output, per-probe verdicts.
- `spike-scratch/increment-{a,c,d}/` — probe code and raw `evidence.txt` captures
  (1056 lines). **`spike-scratch/` is gitignored per `.claude/rules/spike.md`, so these
  are NOT committed and will not survive a workspace clean.** The load-bearing extracts
  are reproduced in `findings.md`; the remainder is reconstructible by re-running the
  probes on the target hardware, which must happen anyway.
- `crates/` untouched throughout — verified per increment.
