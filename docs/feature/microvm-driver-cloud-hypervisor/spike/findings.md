# SPIKE findings — increments a, c, d, e, f, g, h, i, j, k (P1, P2, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13)

Feature: `microvm-driver-cloud-hypervisor` (GH [#42](https://github.com/overdrive-sh/overdrive/issues/42)).
Slice: `slices/slice-00-spike-ch-boot-and-vsock.md`. Governed by `.claude/rules/spike.md`.
Dates: 2026-08-02 (nested aarch64), **2026-08-10 (bare-metal x86_64)**.

| Probe | Verdict |
|---|---|
| **P1** kernel boots from ext4 `virtio-blk` | **WORKS** — confirmed on both arches |
| **P2** vsock beacon + exit status, incl. netns | **WORKS** — confirmed on both arches |
| **P4** per-launch rootfs copy cost | **WORKS — reflink is ~260× faster and free in space** |
| **P5** `[D7]` confinement flags compose | **WORKS**, with three corrections `[D7]`/US-VM-7 must absorb |
| **P6** virtiofsd + `--memory shared=on` | **WORKS** — on x86_64. `[D8g]` host-side `read_only` **verified**; aarch64 still unmeasured |
| **P7** virtio-blk volumes — the I-6 counterfactual | **BOTH WORK** — neither wins on speed; live host sharing is the only real axis |
| **P8** snapshot / restore (S-1, S-6, S-7) | **WORKS on v53** — via the API; the CLI `--restore` is a silent no-op. **CPU hotplug composes with restore.** |
| **P9** S-2 volumes across restore | **BOTH SURVIVE** — virtiofs included, with a fresh daemon. Overturns the research's checkpoint argument. |
| **P10** S-8 `vhost-user-blk` | **WORKS** — but requires `shared=on` and forfeits rate limiting, exactly like `vhost-user-fs`. Its real edge: the backend survives the VMM. |
| **P11** what `vhost-user-blk` COSTS | **THE TRANSPORT IS FREE** — matched on memory backing and caching, it ties plain `--disk` (622.7 vs 622.5 MiB/s, ranges overlap). `shared=on` costs 55%, and that is a **host THP tunable**, not a vhost-user property. Two corrections fall out: qemu's default export is **not flush-durable**, and P7's "~42% faster" is really ~11%. |
| **P13** `memory_restore_mode=ondemand` | **WORKS — restore becomes O(1) in guest RAM** (12–17 ms at BOTH 2 and 4 GiB, vs `copy`'s 0.65–0.88 s / 1.65–1.74 s; ranges disjoint). But it is **asynchronous eager restore, not lazy paging**: a `uffd-handler` thread backfills all of guest RAM at ~900 MiB/s **whether or not the guest touches anything** (proved by a WALK=0 control), so **a warm pool still costs N × RAM** (`Pss ≈ Rss`, `Private_Dirty` = full guest RAM, both modes). **REFUSED under P5's uid-dropped shape** — `Failed to create userfaultfd / EPERM` — needing host `vm.unprivileged_userfaultfd=1`. `prefault` is a pure ~1.8× cost. |
| **P12** S-3 vsock across snapshot/restore | **THE VM RESTORES, THE CONNECTION DOES NOT** — and the reset is **one tick late**: 13/16 runs the guest got a successful `send()` the host never received. New connections work immediately, so the Running gate is recoverable *if the guest re-dials*. Restore **fails** on a stale socket (`EADDRINUSE`) or a missing directory (`ENOENT`) — recoverable in place. |
| **P3** the pinned 6.18 kernel | **NOT RUN** |

Raw evidence: `spike-scratch/increment-{a,c,d,e,f,g,h,i,j,k}/` (gitignored). `crates/` untouched
throughout, verified per increment via `git status --porcelain -- crates/`.

> ## ⚠ Read this first — "aarch64" and "nested Apple Silicon" are different things
>
> This block was rewritten twice on 2026-08-02 because both earlier versions
> conflated them. **On 2026-08-10 the split was confirmed empirically** by
> re-running on non-nested x86_64 bare metal.
>
> - **aarch64 IS a production target.** arm64-specific findings are real
>   platform requirements, not throwaway.
> - **Nested virtualisation on Apple Silicon was the artifact.** The dev host is
>   an M4 Max, so the Lima guest is arm64 *and* nested. The **nesting** caused
>   the boot stalls and made `shared=on` unusable — **not the architecture**.
>
> | Result | Transfers to… |
> |---|---|
> | vsock beacon + netns behaviour, `CONFIG_VSOCKETS=m` vs `=y`, `/dev/console`, guest→host no-handshake, `mkfs.ext4 -d` | **every arch** — mechanism-level, now confirmed on both |
> | Landlock/uid/rlimit composition (P5), the vsock-UDS ruleset gap, per-thread seccomp, `RLIMIT_FSIZE` × memfd | **every arch** — mechanism-level |
> | Kernel must be unwrapped (UKI → EFI-zboot → zstd) | **aarch64 only.** Confirmed 2026-08-10: on x86_64 `file` reports a bzImage and CH takes it **as-is**. |
> | `console=ttyAMA0` | **aarch64 only** — x86_64 needs `ttyS0`. See the trap below. |
> | Boot stalls ~2/3; `shared=on` never boots | **nothing** — nested-Apple only. Now proven by *removing the nesting*. |

---

## Environments

Two, and every verdict names which one produced it.

**A — nested aarch64 (2026-08-02).** The original increment-a/c/d runs.

```
uname -r          : 7.0.0-28-generic          uname -m : aarch64
cloud-hypervisor  : v53.0 (was v46.0.0 until 2026-08-10 — see § The v46 -> v53 bump)
                    virtiofsd: 1.13.2 (/usr/libexec — NOT on PATH)
/dev/kvm          : crw-rw-rw-  (Lima 0666 udev rule)
host              : Apple M4 Max, macOS 26.1; CH runs NESTED inside Lima
```

**B — bare metal x86_64 (2026-08-10).** Scaleway Elastic Metal, provisioned by
`infra/metal/` (committed as `38870e9e`).

```
uname -r          : 7.0.0-15-generic          uname -m : x86_64
cpu               : AMD EPYC 8024P (svm / AMD-V)   — note: AMD, not Intel
systemd-detect-virt: none                     <-- NOT nested. The whole point.
cloud-hypervisor  : v53.0 (--landlock present; was v46.0.0 — see § The v46 -> v53 bump)
virtiofsd         : 1.13.2
/dev/kvm          : crw-rw---- root:kvm       <-- 0660, the production-realistic shape
storage           : /srv/vm = XFS(reflink=1) on a reclaimed NVMe, 894 GB
```

Environment B is the trustworthy one. Environment A cannot gate microVM boot —
see § The nested-virt stall, now settled.

---

## P1 — kernel boots under CH from an ext4 `virtio-blk` rootfs

### Verdict: **WORKS** — but *not* with the distro kernel as shipped

**Predicted:** the guest reaches userspace and PID 1 runs; console shows the init's output.
**Actual:** it does — after the kernel image is unwrapped. Handing CH
`/boot/vmlinuz-7.0.0-28-generic` directly fails:

```
Error booting VM: VmBoot(UefiLoad(UefiTooBig))
```

`UefiTooBig` is **taxonomy, not mechanism** (`.claude/rules/debugging.md` § 2). The real
chain, per CH v46.0 `vmm/src/vm.rs::load_kernel`: `linux_loader`'s PE loader validates the
arm64 `Image` magic at offset `0x38`, finds zero, rejects with
`InvalidImageMagicNumber`; CH then **silently falls back to treating `--kernel` as UEFI
firmware**, and `arch/src/aarch64/uefi.rs` caps firmware at 3 MiB — so a 23.8 MB file dies
with a size error that says nothing about the actual cause.

Ubuntu 26.04 arm64 `vmlinuz` is a **two-layer wrapper**:

```
UKI PE  (.linux / .uname / .sbat / 32× .dtbauto)
  └── .linux section = nested EFI-zboot PE  (magic "MZ\0\0zimg")
        └── zstd payload
              └── raw arm64 Image  (magic "ARMd" at 0x38)
```

After unwrapping both layers:

```
file -b /var/tmp/spike-increment-a/Image
Linux kernel ARM64 boot executable Image, little-endian, 4K pages
```

and the boot completes:

```
[    2.563015] EXT4-fs (vda): mounted filesystem 2e103873-... r/w with ordered data mode.
[    2.563171] VFS: Mounted root (ext4 filesystem) on device 253:0.
[    2.591919] devtmpfs: mounted
[    2.596809] VFS: Pivoted into new rootfs
[    2.618086] Run /init as init process
=========================================================
init: HELLO from overdrive spike init, pid=1
=========================================================
```

### CONFIRMED on x86_64 (env B, 2026-08-10) — and the arch split holds exactly

First attempt, no retry, no unwrapping:

```
### uname -m          : x86_64
### kernel image      : /var/tmp/spike-increment-a/kernel
###   file            : Linux kernel x86 boot executable, bzImage, version 7.0.0-15-generic ...
--- exact CH argv:
    cloud-hypervisor --cpus boot=1 --memory size=512M \
      --kernel /var/tmp/spike-increment-a/kernel \
      --cmdline 'root=/dev/vda rw console=ttyS0 init=/init panic=1 loglevel=7' \
      --disk path=/run/spike-increment-a/host/rootfs.ext4 ...
[    0.670851] EXT4-fs (vda): mounted filesystem f7af687e-... r/w with ordered data mode.
[    0.744980] Run /init as init process
init: HELLO from overdrive spike init, pid=1
```

**The kernel is the distro `vmlinuz` copied verbatim.** `file` identifies it as a
bzImage and CH loads it directly — no UKI, no EFI-zboot, no zstd. The unwrapping
chain below is genuinely aarch64-only, as predicted.

Boot reaches `/init` in **0.74 s** on metal, versus ~2.6 s on the nested host when
it worked at all.

### The unwrapping requirement is arm64-only — but arm64 is a shipping target, so it stands

Cloud Hypervisor's accepted kernel formats differ by arch (`linux-loader`). The
first draft stated the unwrap requirement unqualified; it is **arm64-only** — but
since arm64 ships, it remains a genuine appliance requirement on that arch:

| Arch | Accepted | Ubuntu `/boot/vmlinuz-*` is… |
|---|---|---|
| **x86_64** | **`bzImage`**, or a PVH-enabled `vmlinux` ELF | **a `bzImage` — loads directly, no unwrapping** |
| **aarch64** | raw PE `Image` | a UKI wrapper — needs the unwrap below |

So on x86_64 there is **no** UKI/zboot/zstd problem and no build-time unwrap step.

**What survives as a real lesson, on every arch:** do not assume a distro
`vmlinuz` is CH-loadable, because **the failure mode is a misleading error**. CH
validated the arm64 magic, rejected it, then *silently reinterpreted `--kernel` as
UEFI firmware* and reported a size cap. Nothing in `UefiTooBig` points at "wrong
image format." Whatever kernel the appliance ships, the driver should fail with a
message that names the format problem — that is a `[D5]` requirement independent
of architecture.

Exact CH argv that produced this:

```
ip netns exec spikens cloud-hypervisor \
  --cpus boot=1 --memory size=512M \
  --kernel /var/tmp/spike-increment-a/Image \
  --cmdline 'root=/dev/vda rw console=ttyAMA0 init=/init panic=1 loglevel=7' \
  --disk path=/run/spike-increment-a/netns/rootfs.ext4 \
  --serial file=/run/spike-increment-a/netns/console.log \
  --console off \
  --vsock cid=3,socket=/run/spike-increment-a/netns/ch.vsock
```

---

## P2 — vsock beacon + exit status, including from inside a netns

### Verdict: **WORKS** — both placements

**Predicted:** host reads both messages, in order, with the exit status matching a
deliberately non-trivial guest exit code (7), in both host-netns and netns placements.
**Actual:** exactly that.

**Netns placement independently confirmed two ways** — the VMM is genuinely elsewhere:

```
--- ip netns identify 3203247 -> 'spikens'
--- netns comparison: listener=net:[4026531833]  vmm=net:[4026536640]
+++ VMM IS IN A DIFFERENT NETWORK NAMESPACE THAN THE LISTENER
```

Guest side — note the beacon is delivered with **no guest networking whatsoever**:

```
init: /dev/vsock present BEFORE insmod = false
init: insmod /modules/vsock.ko -> OK
init: insmod /modules/vmw_vsock_virtio_transport_common.ko -> OK
init: insmod /modules/vmw_vsock_virtio_transport.ko -> OK
init: /dev/vsock present AFTER insmod = true
init: guest net ifaces = [lo] (no networking configured)
init: vsock connected on attempt 1
init: vsock write 22 bytes: "READY pid=1 port=1234"
init: about to fork()
init: forked child pid=69; calling waitpid()
init: reaped pid=69 raw_status=0x700 exited=true code=7
init: vsock write 7 bytes: "EXIT 7"
init: powering off (RB_POWER_OFF)
[    3.584338] reboot: Power down
```

Host side:

```
[HOST t=+0.000s] listening on .../ch.vsock_1234 (pid=3203226, netns=net:[4026531833])
[HOST t=+8.711s] accepted guest-initiated connection
[HOST t=+8.716s] msg#1 (22 bytes) = "READY pid=1 port=1234\n"
[HOST t=+9.044s] msg#2 (7 bytes) = "EXIT 7\n"
[HOST t=+9.249s] EOF from guest
[HOST] separate_reads=2 ordering_ready_then_exit=true exit_status_is_7=true
[HOST] VERDICT: beacon + exit status received, in order, exit==7
--- cloud-hypervisor exit code : 0
```

Three properties worth naming explicitly:

1. **Exit 7 is a real `WEXITSTATUS`**, not a hardcoded string — init forks a child that
   `_exit(7)`s and reaps it (`raw_status=0x700`). This is the mechanism `[D3]` needs.
2. **Ordering is observed, not inferred** — `separate_reads=2` means two distinct
   `read()` returns, so the host can distinguish "ready" from "done" rather than parsing
   one blob.
3. **The channel precedes networking.** `guest net ifaces = [lo]` at beacon time. This is
   exactly the `[D2]` requirement: the Running gate must not depend on the thing it may be
   gating.

### CONFIRMED on x86_64 (env B, 2026-08-10)

Identical behaviour, first attempt:

```
init: /dev/vsock present BEFORE insmod = false
init: insmod /modules/vsock.ko -> OK   (+ the two transport modules)
init: /dev/vsock present AFTER insmod = true
init: guest net ifaces = [lo] (no networking configured)
init: vsock connected on attempt 1
init: reaped pid=72 raw_status=0x700 exited=true code=7
[HOST t=+1.116s] msg#1 (22 bytes) = "READY pid=1 port=1234\n"
[HOST t=+1.421s] msg#2 (7 bytes) = "EXIT 7\n"
[HOST] separate_reads=2 ordering_ready_then_exit=true exit_status_is_7=true
```

All three load-bearing properties hold on both arches: a real `WEXITSTATUS`
(`raw_status=0x700`), ordering *observed* via two distinct reads, and the beacon
arriving with `guest net ifaces = [lo]` — before any networking exists. The
`CONFIG_VSOCKETS=m` module-loading requirement is also identical on x86_64.

### Why the netns half works — structural, not incidental

Confirmed against upstream `cloud-hypervisor/docs/vsock.md`: the host end of CH's vsock is
a **UNIX domain socket on the filesystem**, not `AF_VSOCK`. Guest→host connections are
`accept()`ed on `<socket_path>_<port>` with **no handshake**; the `CONNECT <port>` /
`OK <n>` handshake applies only to the host→guest direction. `/dev/vhost-vsock` is never
used by this path.

**UNIX domain sockets are not network-namespaced.** So a VMM inside a per-workload netns
and a listener in the host netns share the channel by construction. The original worry —
that AF_VSOCK namespace behaviour is kernel-version-dependent — **does not apply**, which
means this result generalises across kernels rather than being pinned to 7.0.

---

## The nested-virt stall — SETTLED 2026-08-10 by removing the nesting

**The probe is flaky, and it is the environment, not Cloud Hypervisor.** This was
inferred on 2026-08-02 from population diffs; it is now **proven** by running the
identical probe on non-nested hardware.

```
bare metal x86_64 (env B):  12/12 booted + beaconed, 0 failed
                            time-to-init  min 0.730s  median 0.744s  max 0.746s
nested aarch64   (env A):   ~1 in 3, stalls varying between the virtio_blk probe
                            and the root-mount boundary
```

A 16 ms spread across twelve consecutive runs, versus a two-thirds failure rate.
The nesting hypothesis is confirmed in the cleanest available form — remove the
suspected cause, and the symptom disappears entirely.

**Consequence: environment B is a trustworthy gate; environment A structurally is
not.** Everything below in this section describes env A and remains accurate as
the account of *why* the dev VM cannot gate microVM boot.

Observed directly during verification: attempt 1 stalled, attempt 2 succeeded. The stall
freezes *before* `/init` — after `md: ... autorun DONE.`, at the root-mount boundary — and
the watchdog kills it (`exit 137`):

```
[    2.518583] md: ... autorun DONE.
<nothing further; killed at 90s>
```

Population diff (crafter's runs): `host` 2/6 booted, `host-novsock` 1/6. **vsock is not
implicated** — most stalls never reach `/init` at all, and the no-vsock population stalls
at the same rate. Cross-check: **QEMU/KVM with the identical `Image` and rootfs stalls 4/8
at the same points**, so it is not CH-specific either.

The cause is nesting: `systemd-detect-virt` → `apple`; the guest kernel logs
`kvm [1]: HYP mode not available`.

**Whenever the guest reached userspace, P1 and P2 held 100% of the time.** The stall never
produced a wrong answer — only a missing one. So the verdicts stand.

**Consequence — smaller than the first draft claimed.** Scoped correctly, this is a
**local-loop** property, not a platform problem:

- **CI is unaffected.** Every job is `runs-on: ubuntu-latest` — x86_64, real `/dev/kvm`,
  no nesting. The authoritative gate does not go through this stack at all. No Graviton,
  no bare metal, no cloud spend is implied.
- **Locally, the asymmetry is usable but only one-way.** A green run is genuine evidence
  (a stall never produced a wrong answer, only a missing one). A red run is
  uninformative. That is fine for *proving a mechanism works* — which is what this spike
  did — and useless for *catching a regression*, because a real break and a stall are
  indistinguishable. Retries paper over it at roughly 10 min/test (each stall burns the
  full 90 s watchdog), which is tolerable for one test and not for a suite.
- **So: don't gate on Lima for microVM boot.** The decision point is Slice 01's first
  integration test, not now. The cheap move there is a preflight that detects
  nested-Apple and emits a third outcome — *cannot render a verdict* — rather than a
  pass/fail. Do **not** spend time tuning the Lima environment: Apple ships nested virt
  as a single boolean with no compatibility contract and no errata channel, so every
  candidate mitigation is trial-and-error against a black box.

The standing value of this finding is **diagnostic**: a future crafter seeing these
stalls will otherwise burn days debugging them as a CH or driver bug. See
`docs/research/testing/trustworthy-tier3-gate-for-microvm-boot-research.md`.

---

## Design implications

| For | Implication |
|---|---|
| `[D5]` kernel image — **arch-dependent, corrected** | On **x86_64 (production)** a distro `bzImage` loads directly; **no unwrap step exists**. The arm64 UKI → EFI-zboot → zstd chain is a dev-host artifact only. **What transfers to every arch:** CH's rejection path is misleading (it silently reinterprets an unloadable `--kernel` as UEFI firmware and reports a size cap), so the driver must surface a format error that names the actual problem. |
| `[D2]` Running gate | Ubuntu builds vsock as **modules** (`CONFIG_VSOCKETS=m`, `CONFIG_VIRTIO_VSOCKETS=m`) while `VIRTIO_BLK=y`. Either set them **built-in** in the appliance kernel, or `overdrive-init` must `finit_module` three `.ko`s in dependency order before the beacon. **Built-in is strongly preferable** — it removes an ordering dependency and a rootfs↔kernel-version coupling from the one path the Running gate rides on. |
| `[D4]` guest agent | Guest→host vsock needs **no handshake**; the agent just connects to CID 2. The `CONNECT`/`OK` protocol is host→guest only. Clean shutdown via `RB_POWER_OFF` → PSCI `SYSTEM_OFF` → CH exits 0. |
| `[D5]` rootfs build | **`/dev/console` must exist statically in the ext4 image** — the kernel opens it as fd 0/1/2 for init before devtmpfs is up; without it there is no console output at all, which reads as a hang. |
| Build tooling | `mkfs.ext4 -d <staging-dir>` builds the rootfs with **no loop mount and no root**. But **virtiofs refuses `mknod`**, so the staging tree cannot live on the repo mount — it must be on a native VM filesystem. |
| `[D7]` / US-VM-7 confinement | Three corrections, all measured — see P5. (a) the **vsock UDS needs an explicit per-VM `--landlock-rules` directory grant**, so each VM's socket dir must hold nothing else; (b) **seccomp verifies per-thread**, not on the thread-group leader; (c) **`RLIMIT_FSIZE = max(rootfs, guest RAM)`** once `shared=on` is in play. uid answer settled: unprivileged uid + `kvm` group vs `0660 root:kvm`. |
| `[D8e]` volume confinement | **HOLDS.** CH never needed the volume *source* directory — only `virtiofsd` touches the data. US-VM-8's non-widening AC stands. `--fs socket=` is auto-derived by CH's implicit ruleset (unlike `--vsock`). |
| `[D8d]` virtiofsd sandbox | `--sandbox=namespace` is the default and **genuinely in effect** — but it is a **mount+net** sandbox (`pid` and `user` namespaces are shared). No silent downgrade to `chroot`. State the posture precisely; do not overclaim. |
| `[D8g]` read-only volumes | **UNVERIFIED.** Host-side `--readonly` was never tested against a guest, because `shared=on` cannot boot on this host. The security framing must not be asserted until it is proven — a guest-side `-o ro` is guest-cooperative and void. |
| Constraint 7 (version floor) | ~~Installed CH is **v46.0.0**, *below* the reference implementation's unexplained "≥48.0" floor — and it works, and it **has `--landlock` and `--landlock-rules`**. So the ≥48.0 figure has no evidence behind it. Do not inherit it.~~ **SUPERSEDED 2026-08-10 — this reasoning was the bug.** "Their floor is unjustified" was right; "therefore stay on v46" did not follow. The correct response to an unjustified floor is to take LATEST. Now **v53.0**; see § "The v46 → v53 bump". The half that survives: name a floor against a *capability*, not a number someone else wrote down. |

---

## P4 — per-launch rootfs copy cost (reflink vs full copy)

### Verdict: **WORKS — reflink is ~260× faster and costs no space**

Env B, `/srv/vm` = XFS(`reflink=1`) on NVMe, 4 GiB source (rootfs-sized), page
cache dropped before each run, space measured by `df` delta:

```
coreutils: cp (GNU coreutils) 9.7

  --reflink=never     3.970 s   + 4096 MiB     <- genuine full copy
  --reflink=auto      0.015 s   +    0 MiB
  --reflink=always    0.016 s   +    0 MiB

filefrag: 0: 0..1048575: 1048640..2097215: 1048576: last,shared,eof
```

**~260× faster, zero additional space, extents confirmed shared.** This closes
research **Gap 2**, which flagged host reflink as a per-VM CoW mechanism as
essentially undocumented in the literature — plausibly because every surveyed
platform is multi-tenant, where reflink does not help.

**`[D5]` stands.** A per-launch reflink copy is effectively free, so the
ext4+`virtio-blk`+reflink design does not need to fall back to squashfs/erofs
plus an in-guest overlay for cost reasons.

### Design implication — the flag is redundant, the filesystem guarantee is not

**coreutils 9.7 defaults `cp` to `--reflink=auto`.** A plain `cp` already
reflinks on a capable filesystem, which is why `--reflink=never` was required to
measure the fallback at all.

So `[D5]` does **not** need to pass the flag. It **does** need to guarantee the
filesystem, because on ext4 the *identical command* silently becomes the 3.97 s /
4096 MiB case with **no error and no warning**. That is the whole failure mode:
not a wrong flag, an unnoticed filesystem. Whatever provisions the appliance's VM
data directory must assert `FICLONE` works — `infra/metal/provision.sh` does this
with a real `cp --reflink=always` probe rather than checking the fstype string.

---

## P5 — do the `[D7]` confinement flags compose with a real boot?

### Verdict: **WORKS** — with three corrections `[D7]` / US-VM-7 must absorb

Raw evidence: `spike-scratch/increment-c/evidence.txt` (685 lines). All four mechanisms
applied simultaneously to the same VM increment-a validated; first attempt reached
`BOOTED + BEACON + EXIT 7`.

Captured live, while the VM was running:

```
Uid:	6001	6001	6001	6001
Gid:	6001	6001	6001	6001
Groups:	991 6001
NoNewPrivs:	1
Max file size             134217728            134217728            bytes
Max open files            256                  256
```

### The uid question — SETTLED, and the production-viable answer

`0660 root:kvm` + group membership **works**. The probe did not settle for the 0666 the
Lima udev rule provides: it `chmod 0660 /dev/kvm`, created `spikevmm` (uid 6001) as a
member of `kvm` (gid 991), and captured `+++ open(/dev/kvm, O_RDWR) OK as spikevmm`,
restoring afterwards. **No 0666 is required.** This closes feature-delta Handoff item 8.

Launch shape that produced it:

```
prlimit --fsize=134217728 --nofile=256 -- \
  setpriv --reuid=spikevmm --regid=6001 --init-groups --no-new-privs -- \
  cloud-hypervisor … --seccomp true --landlock \
    --landlock-rules path=/run/spike-increment-c/vm-a,access=rw
```

### Correction 1 — seccomp must be verified PER-THREAD

`/proc/<pid>/status` reports `Seccomp: 0` **even with `--seccomp true`**, because CH
installs filters on its worker threads, not the thread-group leader:

```
### PER-THREAD (comm : Seccomp : Seccomp_filters : NoNewPrivs):
  cloud-hyperviso    NoNewPrivs:;0 Seccomp:;0 Seccomp_filters:;0
  vmm                NoNewPrivs:;1 Seccomp:;2 Seccomp_filters:;1
  http-server        NoNewPrivs:;1 Seccomp:;2 Seccomp_filters:;1
  vmm_signal_hand    NoNewPrivs:;1 Seccomp:;2 Seccomp_filters:;2
```

**An AC written against `/proc/<pid>/status` would fail on a correctly-confined CH.** It
must read `/proc/<pid>/task/*/status`. This is exactly the vacuous-assertion class the
DISCUSS review was hunting — caught here instead of in Slice 03.

### Correction 2 — CH's implicit Landlock ruleset is INCOMPLETE for the vsock UDS

The first confined run died with an error that **never mentions Landlock**:

```
Error booting VM: VmBoot(DeviceManager(CreateVsockBackend(UnixBind(
  Os { code: 13, kind: PermissionDenied, message: "Permission denied" }))))
```

Two population diffs isolated it:

- `landlock-only-root  uid=0 rl=0 ll=1  rc=1` → **fails at uid 0 with no rlimits**, so
  Landlock alone is the cause; uid-drop and rlimits are innocent.
- Per-device: CH auto-derives rules for `--kernel`, `--disk`, `--serial file=` and
  `--api-socket`, but **not for the vsock UDS it binds itself.**

Two further constraints, both measured:

- A read-only rule is insufficient — `vsock-only+dir-ro-rule` still `EACCES`.
- **The rule cannot name the socket path.** CH validates rule paths for existence at
  config-parse time and the socket does not exist yet:
  `Error validating configuration: Path ".../ch.vsock" provided in landlock-rules does not exist`

**Design constraint for `[D7]`/US-VM-7:** the grant must be the *containing directory*
with `access=rw`, therefore **each VM needs its own socket directory holding nothing
else** — otherwise the grant widens to whatever shares that directory.

### Correction 3 — see P6: `RLIMIT_FSIZE` must be sized against guest RAM, not the rootfs

increment-c sized `RLIMIT_FSIZE` off the rootfs image alone. That is wrong the moment
`shared=on` is used. Details under P6.

### The denial — US-VM-7 AC 1(b) evidence, capturable only here

Run deliberately as **root** so any `EACCES` is necessarily Landlock and not DAC. Landlock
ABI 8; all targets opened *before* `landlock_restrict_self`, then re-attempted after:

```
expect=allow /var/tmp/spike-increment-a/Image            -> OPENED
expect=allow /run/spike-increment-c/vm-a/rootfs.ext4     -> OPENED
expect=allow /dev/kvm                                    -> OPENED
expect=deny  /var/tmp/spike-increment-c/SENTINEL-...     -> DENIED errno=13
expect=deny  /run/spike-increment-c/vm-b/rootfs.ext4     -> DENIED errno=13   <- a SIBLING VM's disk
expect=deny  /var/tmp/spike-increment-a/rootfs.ext4      -> DENIED errno=13
expect=deny  /etc/shadow                                 -> DENIED errno=13
```

**Honest caveat, stated in the evidence file:** this is the same *path set* CH was given,
not a byte-copy of CH's internal ruleset — CH exposes no way to prove the latter.

---

## P6 — virtiofsd + `--memory shared=on`

### Verdict: **WORKS** — every question env A left open is now answered

Measured on **env B** (bare-metal x86_64) on 2026-08-10. Raw evidence:
`spike-scratch/increment-e/evidence.txt` (1163 lines), `cache-compare.txt`, and the
per-mode `transcript-*.txt` / `mem-*.txt` captures.

increment-d is preserved unchanged as the env-A record. **increment-e is the same probe
rebuilt for x86_64** — same guest logic, same modes, same beacon-synchronised `/proc`
capture — so the only deliberate variable is the environment.

**Five modes, all COMPLETED:**

| Mode | What it isolates | Result |
|---|---|---|
| `full` | `shared=on` + 2 fs devices + **all** P5 confinement | **COMPLETED** |
| `full-no-fsd-rule` | same, minus the landlock rule for the vhost-user socket dir | **COMPLETED** |
| `sharedonly` | `shared=on`, no fs devices | **COMPLETED** |
| `noshare` | neither — the `[D8b]` volume-free baseline | **COMPLETED** |
| `full` @ `--cache=auto` | the `[D8c]` comparison | **COMPLETED** |

Guest serial console from the `full` run — `shared=on`, two virtiofsd daemons, and the
entire P5 stack (seccomp + landlock + uid-drop + prlimit) at once:

```
init: HELLO from overdrive spike init (P6), pid=1
init: insmod /modules/virtiofs.ko -> OK
init: /proc/filesystems virtiofs line = "nodev\tvirtiofs"
init: touched 128 MiB of guest memory (every 4K page written)
READY pid=1 port=1234
MEMTOTAL MemTotal:         463132 kB
FS-MOUNT tag=volrw at /mnt/rw -> OK   (read-write volume)
FS-MOUNT tag=volro at /mnt/ro -> OK   (host-side --readonly export, mounted RW by the guest ON PURPOSE)
FS-MOUNT tag=nosuchvolume at /mnt/bad -> rc=-1 errno=22 (Invalid argument (os error 22))
FS-RW-LISTING [from-host.txt]
FS-HOST-TO-GUEST "HOST-WROTE-THIS-9876543210-zyxwvutsrq"
FS-GUEST-TO-HOST wrote "GUEST-WROTE-THIS-0123456789-abcdefghij"
FS-RO-CREATE refused errno=30 (Read-only file system (os error 30))
FS-RO-OVERWRITE refused errno=30 (Read-only file system (os error 30))
FS-RO-READ "PREEXISTING-HOST-CONTENT-DO-NOT-CHANGE"
init: reaped pid=72 raw_status=0x700 exited=true code=7
EXIT 7
DONE
```

### `[D8g]` — host-side `read_only` IS enforced. The security framing now stands.

This was the claim the findings refused to assert for eight days, and it is the most
consequential result here.

The test is built to be **non-cooperative on purpose**: the guest mounts the
`--readonly` export **read-write** and then tries to write it. A guest-side `-o ro` would
have been guest-cooperative and proved nothing.

- Guest create → `errno=30 EROFS`. Guest overwrite of a pre-existing file → `errno=30 EROFS`.
- Host-side, after the run: `guest-should-not-create.txt` **absent**, and
  `preexisting-host-file.txt` still reads `PREEXISTING-HOST-CONTENT-DO-NOT-CHANGE`.
- Reads still work (`FS-RO-READ` returns the host content), so the share is genuinely
  mounted, not merely broken.

Both halves matter: the guest saw a refusal **and** the host tree is untouched. **`[D8a]`'s
security framing may now be asserted** — with the scope stated in *What this does NOT
establish* below.

### The other answers

- **Round-trip works in both directions.** Host→guest read back
  `HOST-WROTE-THIS-…`; guest→host landed **byte-identical** to what the guest reported
  writing (39 bytes, `cmp`-verified). The payloads are byte-distinct per direction, so
  neither assertion can be satisfied by an echo of the other.
- **Failed-mount errno is `EINVAL` (22)** for a tag with no matching device. This is the
  value `overdrive-init`'s refuse-to-exec path must match on — previously a guess.
- **`[D8b]` — `shared=on` does NOT change what the guest sees.** `MemTotal: 463132 kB`,
  identical across `full`, `sharedonly`, and `noshare`.
- **`[D8e]` HOLDS on x86_64.** The volume source directories were granted in **no** trial,
  and the boot plus the full round-trip both succeeded. **Volumes do not widen `[D7]`'s
  hypervisor confinement; US-VM-8's non-widening AC stands.**
- **`--fs socket=` IS auto-derived.** `full-no-fsd-rule` — which grants CH a landlock rule
  for the per-VM directory only, never the vhost-user socket directory — completed with a
  byte-identical round-trip. Unlike `--vsock`, which needs an explicit grant.
- **`[D8d]` `--sandbox=namespace` is real, and the precise claim is unchanged from env A:**
  `mnt` and `net` namespace inodes differ from the shell's; `pid` and `user` do **not**. A
  mount+net sandbox, not a full one. No silent downgrade to `chroot`.
- **`RLIMIT_FSIZE` × memfd reproduces exactly on x86_64** — see below.

### `shared=on` costs *less* host RSS than private memory, and the difference is reclassification

Captured at the beacon: same guest lifecycle point in every mode, 128 MiB already
touched, before any filesystem I/O.

| Mode | VmRSS | RssAnon | RssShmem | memfd | Threads |
|---|---|---|---|---|---|
| `noshare` (private) | 276888 kB | 273232 kB | 4 kB | **none** | 9 |
| `sharedonly` | 265456 kB | 852 kB | 260952 kB | `/memfd:ch_ram` | 9 |
| `full` (+2 fs devices) | 270056 kB | 892 kB | 265512 kB | `/memfd:ch_ram` | 11 |

`shared=on` moves essentially the entire footprint from `RssAnon` to `RssShmem` and lands
**~11 MB lower** than private. Env A saw the same shape in an early-boot sample and could
only call it "not obvious inflation"; measured at the controlled point, it is a
**reclassification, and mildly cheaper**. Two fs devices cost **+2 threads** (`_fs1`,
`_fs2`) and ~4.6 MB.

### `[D8c]` — `--cache=never` is right for bulk writes and wrong for small files

`[D8c]` picked `never` for the volume role **without measuring it**, on the one path that
carries the workload's output. Four interleaved trials per mode, everything else held
fixed. Every sample, no averaging (`cache-compare.txt`):

| `--cache` | 256 MiB streaming write | 1000 files, open+write+fsync+close |
|---|---|---|
| `never` | **1527.7 / 1544.8 / 1555.0 / 1561.2 MiB/s** | 0.42 / 0.43 / 0.44 / 0.48 ms/file |
| `auto` | 404.2 / 408.5 / 411.6 / 421.0 MiB/s | **0.37 / 0.37 / 0.37 / 0.37 ms/file** |

**`never` is ~3.7× faster on the streaming write; `auto` is ~15% faster per small file.**
The ranges do not overlap in either direction. `fsync` time is identical (0.178–0.180 s)
in both, so the entire difference sits in the `write()` path.

The streaming direction is opposite to the naive expectation that caching helps writes.
**No mechanism is claimed here** — this is the measurement, not an explanation of it. What
it supports is narrow and useful: **`[D8c]`'s choice of `never` is correct for the
output-carrying bulk path, and it is a genuine trade-off rather than a free win.** A
metadata-heavy volume would prefer `auto`, which makes cache mode a plausible per-volume
knob rather than a global constant.

### virtiofs overhead, on both filesystems, against a matched baseline

The host baseline runs the **same syscall sequence** as the guest (`open`+`write`+`fsync`+
`close` per file; `bs=1M … conv=fsync` for the stream) **on the same filesystem** as the
shares, so the ratio is virtiofs overhead rather than a filesystem difference.

| Volume filesystem | Stream: guest / host | Overhead | Per file: guest / host | Overhead |
|---|---|---|---|---|
| ext4 (`/dev/md1`, root RAID1) | 0.347 s / 0.258 s | **1.34×** | 0.45 / 0.16 ms | **2.8×** |
| XFS reflink (`/dev/nvme1n1`, `/srv/vm`) | 0.288 s / 0.224 s | **1.29×** | 0.43 / 0.15 ms | **2.9×** |

Streaming through virtiofs costs ~30%; per-file operations cost ~2.8×, which is the number
that matters for small-file workloads. The overhead ratio is stable across both
filesystems while absolute throughput tracks the device — so the ~30%/2.8× figures are
properties of virtiofs, not of this disk.

### What this does NOT establish — do not let these be assumed later

- **x86_64 only. aarch64 `shared=on` remains unmeasured.** Env A cannot boot it and no
  non-nested Arm hardware is available. Both arches ship, so this is a real gap, not a
  formality.
- **Kernel `7.0.0-15-generic`, not the pinned 6.18** (that is P3, still not run).
- **One VM, one vCPU, no contention.** Nothing here says how virtiofsd behaves under
  concurrent VMs or multiple queues.
- **`--cache=always` and DAX were not measured.**
- **`read_only` is proven against the normal VFS write path** — create and overwrite, both
  refused at the daemon, host tree verified untouched. That is the claim `[D8a]` needs. It
  is not an exhaustive audit of every write vector a hostile guest might attempt.

### Harness defects caught before they became findings

Three, all of which would have produced a confidently wrong number:

1. **The small-file baseline did not match the guest.** increment-d's host baseline used a
   shell loop with one trailing `sync` while the guest fsync'd **per file** — a different
   operation, making the host look ~6× faster than it is. Replaced with the identical
   syscall sequence.
2. **Mode `E` is also `full`, so its transcript overwrote mode `A`'s.** The cache=auto
   numbers were sitting in the file labelled cache=never. Transcripts are now keyed by
   mode **and** cache mode.
3. **Moving the volumes to XFS left the baseline on ext4**, which would have reported a
   filesystem difference as virtiofs overhead. Baseline now follows `VOLROOT`, and the run
   header reports the volumes' own filesystem rather than `$OUT`'s.

The first was inherited from env A, where `shared=on` never booted far enough for anyone to
read the number.

## P7 — the virtio-blk volume counterfactual (increment-f)

### Verdict: **BOTH WORK.** The choice is not performance — it is live sharing vs everything else.

**Why this exists.** I-6 splits storage by role: `virtio-blk` rootfs, `virtiofs` volumes.
The rootfs half is argued from measurement (no DAX on CH, FUSE in the hot path). **The
volume half was never measured against the block alternative** — it was carried from the
reference implementation and from a whitepaper §6 citation, and the whitepaper is
explicitly not SSOT. increment-f runs the same payload over a second `virtio-blk` device
so the comparison is a number.

Held identical to increment-e wherever it is not the thing under test: same kernel, same
box, same XFS(reflink=1) volume filesystem, same 128 MiB pre-beacon touch, same payload,
same P5 confinement, same beacon-synchronised `/proc` capture, same matched host baseline.
Deliberate differences: `--disk` instead of `--fs`+virtiofsd, **no `--memory shared=on`**,
and no daemon.

Raw evidence: `spike-scratch/increment-f/` — `vs-virtiofs.txt`, `run-blk.txt`,
`run-ratelimit.txt`, `mem-blk.txt`.

### Throughput cuts both ways — 4 interleaved trials each, no overlap

| Metric | virtiofs (`cache=never`) | virtio-blk | Winner |
|---|---|---|---|
| 256 MiB streaming write | 2247.6 / 2277.8 / 2270.3 / 2325.7 MiB/s | **3243.7 / 3242.1 / 3211.1 / 3189.1 MiB/s** | **block, ~42% faster** |
| 1000 files, open+write+fsync+close | **0.48 / 0.49 / 0.42 / 0.41 ms/file** | 0.56 / 0.56 / 0.56 / 0.59 ms/file | **virtiofs, ~25% faster** |

> **CORRECTED 2026-08-10 by P11 — the streaming row overstates the gap by ~4×.**
> Those are **write-only** MiB/s, and write-only is not comparable across
> mechanisms: it measures how much work each one defers past the timer, not how
> fast it is. Recomputed from **this section's own evidence file**
> (`vs-virtiofs.txt`, untouched), the durable `write + fsync` figure gives
> **blk ÷ virtiofs = 1.110**, not 1.419 — a **~11%** streaming advantage, not
> ~42%. increment-i reproduces 1.114 independently. The *direction* of this
> table stands and so does its conclusion ("performance does not decide this");
> only the magnitude was wrong. The per-file row is unaffected — it was always
> end-to-end. See § P11 → "What this corrects in P7".

Against the matched host baseline (0.220–0.225 s streaming, 0.15 ms/file on the same XFS):

| Mechanism | Streaming overhead | Per-file overhead |
|---|---|---|
| virtio-blk | **1.19×** | 3.7× |
| virtiofs | 1.31× | **3.0×** |

Neither mechanism dominates. This is the same shape as the `[D8c]` cache result: bulk
throughput and metadata-heavy workloads pull in opposite directions. **Performance does not
decide this**, which is precisely why it should not be the argument.

### What virtio-blk gives that virtiofs does not

- **No `--memory shared=on`.** Confirmed: `memfd-ish mapping lines: 0`, `RssShmem: 4 kB`.
  So no memfd, and `RLIMIT_FSIZE` no longer has to cover guest RAM.

  > **RETRACTED 2026-08-10, same day, before anyone relied on it.** This bullet
  > originally continued: *"and — the big one — no nested-virt boot blocker. The virtiofs
  > volume path is permanently un-runnable on the Apple-Silicon dev host; a block volume
  > path runs in Lima."* **The second half was an inference, not a measurement** —
  > extrapolated from increment-a booting a block *rootfs* under nesting. It was challenged,
  > tested, and does not hold up. See § "Does the block path run on the dev host?" below.
- **Rate limiting, demonstrated not asserted.** `--disk bw_size=33554432,bw_refill_time=1000`
  took the same write from **3189 MiB/s to 43.7 MiB/s** (per-file 0.56 → 1.01 ms).
  **`--fs` has no equivalent parameter at all.** For a multi-tenant platform that is a
  real operational gap, not a nicety.
- **No daemon.** No `virtiofsd` process, no socket-wait, no SIGTERM→SIGKILL lifecycle, and
  no "crashed daemon must not look like a clean VM exit" failure mode. The reference
  implementation's `VirtiofsdManager` was 415 lines.
- **No module staging.** `CONFIG_EXT4_FS=y` and `CONFIG_VIRTIO_BLK=y` are built in;
  virtiofs needed `virtiofs.ko` shipped inside the rootfs (`CONFIG_VIRTIO_FS=m`).
- **`readonly=on` is enforced harder — it fails at MOUNT.** The guest could not even mount
  it read-write: `errno=13 EACCES`. virtiofs allowed the mount and refused the individual
  writes with `EROFS`. Both are genuine host-side enforcement; block fails earlier.

### What virtiofs gives that virtio-blk does not — and this is the decision axis

**The host can read and write the share while the guest is running.** A block volume is
single-writer: increment-f's host-side verification has to **loop-mount the image after
shutdown**. That is not a performance difference, it is a capability difference, and it is
the only one that cannot be engineered around.

Two consequences that fall out of it:

- **Block volumes need clean-unmount discipline.** The first increment-f run powered off
  without unmounting; the image was left with a dirty journal and a read-only loop-mount of
  it **failed outright** (`cannot mount /dev/loop0 read-only`). The guest now unmounts
  before power-off (`FS-UMOUNT /mnt/rw -> rc=0`). A real driver inherits this problem: an
  ungracefully-killed VM leaves a volume that needs recovery on next attach. virtiofs has no
  equivalent — the host directory is always consistent.
- **Live log tailing / artifact collection during a run is virtiofs-only.** If nothing needs
  that, block is the better default on every other axis.

### Reflink is intra-filesystem — a constraint on the driver, found the hard way

increment-f's first run put the per-VM disk images under `/run` (tmpfs) next to the
sockets, and the clone failed:

```
cp: failed to clone '/run/.../x.ext4' from '/srv/vm/p6f/volro.ext4': Invalid cross-device link
```

**`--reflink=always` fails loudly. `--reflink=auto` — coreutils ≥9's default for plain
`cp` — would have silently done a FULL COPY instead**, and P4's ~260× advantage would have
evaporated with no error anywhere. So: **per-VM disk artifacts must be staged on the same
filesystem as their master.** This binds the rootfs clone too, not just volumes — a driver
that stages per-VM rootfs images into a tmpfs run directory silently loses P4's entire
result. Sockets and logs on tmpfs, disk images on the volume filesystem.

With that fixed, cloning a 1 GiB + 64 MiB volume pair takes **~9 ms and 868 KiB on disk**
(`1.0G apparent / 868K actual`), confirming P4 in the volume context.

### Errno table for the refuse-to-exec path — mechanism-dependent

`overdrive-init` must match on different values depending on which mechanism a volume uses:

| Failure | virtiofs | virtio-blk |
|---|---|---|
| Volume not attached | `EINVAL` (22), on a nonexistent tag | `ENOENT` (2), on a nonexistent device |
| Host-side read-only, guest mounts RW | mount OK, writes `EROFS` (30) | mount fails `EACCES` (13) |

### Does the block path run on the dev host? UNKNOWN — and the dev host is worse than recorded

The claim that a block volume path would run in Lima (where virtiofs + `shared=on` cannot)
was an **inference from increment-a's block rootfs**, never a measurement of increment-f.
Tested on 2026-08-10:

| Arm | What | Boots (nested aarch64 Lima, kernel 7.0.0-28) |
|---|---|---|
| A | increment-a — rootfs only, no volumes, no `shared=on` | **0/3** |
| F | increment-f — rootfs + 2 virtio-blk volumes, no `shared=on` | **0/3** |

**Arm F's 0/3 is NOT evidence about block volumes, because the baseline is also 0/3.** Both
stall identically: the guest kernel starts (serial console shows the banner and early boot)
and never reaches `/init`. That is the same nested stall § "The nested-virt stall" describes
— it is simply hitting 100% here rather than ~1/12.

Two things follow, and the second is the more important one:

1. **The "block runs in Lima" claim is withdrawn.** It is unsupported. It may still be
   true — the mechanism argument (no `shared=on`, so no `MAP_SHARED` guest memory under
   nested KVM) is untouched by this result — but it is not measured, and it must not be
   used as an argument for block volumes until it is.
2. **The dev-host situation is worse than this document previously recorded.** increment-a
   was ~11/12 on the *old* Lima VM (kernel 6.8). On a freshly created one (Ubuntu 26.04,
   kernel 7.0.0-28) it is 0/3 — no CH guest booted at all, with or without volumes. Whether
   that is the newer guest kernel, the newer host image, or something in the rebuild is
   **not diagnosed**. Until it is, *no* Lima-based CH result should be treated as
   reproducible, including the ~11/12 figure and the 12/12 comparison that retired the
   nested caveat for P1/P2 (that one was measured on bare metal and is unaffected).

Evidence: `spike-scratch/increment-f/nested-check.sh` is the three-arm harness; the runs
above were the arm-A and arm-F halves of it, at reduced payload (32 MiB / 200 files) since
the question was boot reliability, not throughput.

### What this does NOT establish

- **One VM, no contention.** Nothing here says how either mechanism behaves under
  concurrent VMs competing for the same device or daemon.
- **`vhost-user-blk` was not measured** (`--disk vhost_user=on,socket=`). It is a third
  option that keeps the block model and moves the backend to userspace.
  **CLOSED 2026-08-10 by P10 (does it work) and P11 (what it costs).**
- **No durability/crash testing.** "Clean unmount matters" is established; what a
  power-cut mid-write actually costs on either mechanism is not.
- **The rate-limit measurement is one sample** at one setting. It shows the knob works and
  is roughly the right magnitude, not that it is precise.

## The v46 → v53 bump, and the one migration it surfaced

**Why this happened at all.** The CH pin sat at **v46.0** (released 2025-05-23) for 14 months
and 7 releases. It entered the tree in `ed5975d8` — *"feat(core): persist backoff inputs,
recompute deadline each tick (#143)"* — a version pin buried in a reconciler-backoff commit,
where no reviewer would look for one. When the upgrade question *did* surface, this document's
own Constraint 7 and `versions.env` both argued **against** moving, on the grounds that v46
"demonstrably has the capability we actually need." That reasoning read like rigour and was
its opposite: it froze the pin against the requirements known at that moment and talked down
the one prompt that would have moved it. Checkpoint/restore later became the target workload,
and `vhost-user-fs` restore is broken before v52.0 (cloud-hypervisor#6931).

The structural cause, and it is not specific to CH: **every version gate in this repo checks
`installed == pinned`; none checks `pinned == latest`.** A pin can rot indefinitely with a
green board. The sweep that followed found all three single-binary pins stale — CH by 7
releases, **wasmtime by 18 major versions** (v28.0.0 → v46.0.2), pwru by a patch.

### The migration: `image_type=` is mandatory from v53, and its default is not benign

The first v53 run looked like a virtiofs regression: modes `full`, `full-no-fsd-rule`, and
`full @ cache=auto` all failed; `sharedonly` and `noshare` passed. Every failing mode had
`--fs` devices, so the obvious reading was "v53 broke virtiofs." **That reading was wrong.**

```
WARN: DEPRECATION: auto-detection of disk image type is deprecated ...
WARN: Autodetected raw image type. Disabling sector 0 writes.
WARN: <_disk0_q0> Attempting to write to sector 0 on a disk without specifying image_type
ERROR: Fatal error: VmmThread(VmReboot(DeviceManager(CreateVirtioFs(
         VhostUserConnect(VhostUserProtocol(SocketConnect(
           Os { code: 111, kind: ConnectionRefused })))))))
```

The chain: our images are **bare filesystems with no partition table**, so sector 0 *is* the
filesystem. v53 auto-detects `raw` and **silently disables sector-0 writes**. The guest's
write is refused, it faults, `panic=1` reboots it — and on reboot CH cannot reconnect to a
`virtiofsd` that already exited when the first instance disconnected. Hence
`CreateVirtioFs`/`ConnectionRefused`.

**The failure surfaced two layers from its cause, and only on the `--fs` modes** — because a
reboot is fatal *only* when a vhost-user daemon has to be reconnected. The block-only modes
rebooted and carried on, which is exactly why the signature pointed at virtiofs.

Fix: pass `image_type=raw` explicitly on every `--disk`. With that, **all five modes pass on
v53.** This is a **driver requirement**, not probe hygiene: `image_type` must be set on every
disk the `vm` driver attaches, and the auto-detect path must never be relied on.

### The measurements survive the bump

Re-run on v53, six interleaved trials per mechanism (was four on v46):

| Metric | v46 | **v53** |
|---|---|---|
| virtiofs, 256 MiB streaming | 2247.6–2325.7 MiB/s | **2256.4–2323.6 MiB/s** |
| virtio-blk, 256 MiB streaming | 3189.1–3243.7 MiB/s | **3213.3–3365.3 MiB/s** |
| virtiofs, 1000 files | 0.41–0.49 ms/file | **0.40–0.43 ms/file** |
| virtio-blk, 1000 files | 0.56–0.59 ms/file | **0.49–0.59 ms/file** |

Still non-overlapping in both directions, same sign, same rough magnitude. **P5, P6, and P7's
conclusions are unchanged by the bump** — block wins streaming, virtiofs wins per-file.

One honesty note on method: a first 3-trial set produced two outliers (a 3749 MiB/s virtiofs
streaming sample, above block; and a 0.62 ms/file sample) taken while the box was still
settling after re-provisioning. They are not in the table above and were not reported as a
result — the six-trial set replaced them. Had the 3-trial set been written up, it would have
shown the per-file ranges overlapping and reversed the P7 conclusion on noise.

### What this does NOT re-open, and what it does

- **NOT re-opened: the P6/P7 measurements.** They reproduce on v53.
- **RE-OPENED: the checkpoint/restore half of the I-6 recommendation.** The research
  (`docs/research/platform/persistent-microvm-checkpoint-restore-comprehensive-research.md`)
  argued block wins partly because `vhost-user-fs` restore hangs — cloud-hypervisor#6931,
  **fixed in v52.0**. On v53 that specific blocker should be gone. The independent half of
  the argument (the temporal-gap reasoning; the six-for-six convergence that nobody exposes a
  live host-shared filesystem into a checkpointable guest) is untouched. **Snapshot/restore
  has still never been run here at all** — that is probe S-1, and it is now the gating
  question for I-6.

## P8 — snapshot / restore (S-1, S-6, S-7), increment-g

### Verdict: **WORKS on v53** — via the API. The CLI path is a silent no-op.

This is the probe everything downstream of I-6 was waiting on, and it had never
been run. The research doc reasons about checkpoint/restore at length; nobody had
executed it once. Measured on env B, CH **v53.0**.

Raw evidence: `spike-scratch/increment-g/`.

### How the probe avoids a false pass

A restored VM and a *rebooted* VM look nearly identical from outside: both are alive,
both tick, both serve. So the guest holds a **boot nonce that exists only in RAM** — 16
bytes from `/dev/urandom`, read once, never written anywhere — and prints it with a
monotonically increasing counter. Nonce identical + counter continued = memory really
came back. Nonce changed or counter restarted = it rebooted, and "restore works" would
have been a lie. That distinction is the whole probe, and it earned its keep: several
intermediate attempts produced a *live VMM with no restored guest*, which without the
nonce would have read as success.

```
BOOT_NONCE before   a1fbf271a5d10cc51ba547a6b5606e84
BOOT_NONCE after    a1fbf271a5d10cc51ba547a6b5606e84
last tick before    10
last tick after     20        <- resumed at exactly n=11
boot banners after  0
+++ RESTORED FROM MEMORY
```

### S-1 — the flow that works, and the one that does not

**Works — the API:**

```
PUT /api/v1/vm.pause                                        -> 204
PUT /api/v1/vm.snapshot {"destination_url":"file://<dir>"}  -> 204   (~0.05 s)
# kill the VMM; remove BOTH the api socket and its .lock
cloud-hypervisor --api-socket path=<sock>          # NO VM configured
PUT /api/v1/vm.restore {"source_url":"file://<dir>"}        -> 204
PUT /api/v1/vm.resume                                        -> 204
```

**Does NOT work — the CLI `--restore`.** It exists in v53, it parses, and it does
nothing:

- it demands `--kernel`/`--firmware` at clap level *even though the snapshot's own
  `config.json` already names the payload*, and then
- **exits with no error, no log line, and no guest.**

A silent no-op is the worst available failure mode here, because "the VMM process is
running" and "the VM was restored" are indistinguishable without something like the
nonce. **The driver must implement the API flow.**

### S-6 — CPU hotplug still works on a RESTORED VM

**This was the highest-risk unknown in the whole feature.** CPU hotplug is the stated
reason this feature chose Cloud Hypervisor over Firecracker; Firecracker forbids hotplug
on restored VMs; CH's docs are silent. Nobody had checked the two compose.

```
TICK n=21 ... vcpu_online=1 vcpu_present=1
TICK n=22 ... vcpu_online=2 vcpu_present=2 *ONLINED 1*
TICK n=23 ... vcpu_online=2 vcpu_present=2
```

`PUT /api/v1/vm.resize {"desired_vcpus":2}` on the restored VM: the vCPU appears and is
brought online. **They compose. The CH-over-Firecracker rationale survives.**

**A near-miss worth recording.** The first S-6 run reported `vcpus=1` and looked like a
clean negative. It was wrong: `/proc/cpuinfo` lists only **online** CPUs, and this
minimal init has no udev to online a hot-plugged one. The probe now reports `present`
(sysfs `cpuN` dirs) *and* `online` separately, and onlines offline CPUs itself. Without
that split the finding would have been a confident, wrong *"CPU hotplug does not work on
restored VMs"* — which would have argued for abandoning CH.

### S-7 — snapshot size is exactly guest RAM, and it is NOT sparse

| Artifact | Size |
|---|---|
| `config.json` | 1003 B |
| `memory-ranges` | **536 870 912 B — exactly the 512 MiB guest RAM** |
| `state.json` | ~39.9 KB |
| directory total | **513 M apparent AND 513 M on disk** |

Apparent equals on-disk, so **no sparseness on XFS**. Warm-pool storage is therefore
`N × guest RAM`, undeduplicated, until something above CH does better. The snapshot call
returns in ~0.05 s for 512 MiB — that is page-cache speed, not a durability guarantee;
nothing here proves the bytes reached the platter.

### Operational traps, all of which cost time and all of which bind the driver

1. **`<api-socket>.lock`.** v53 keeps a lock file beside the API socket. A SIGKILLed VMM
   leaves it, and the next VMM on that path refuses to start with
   `StartVmmThread(ApiSocketInUse(...))`. Removing only the socket is **not** enough. A
   driver that checkpoints by killing the VMM must clean both, or use a fresh socket path
   per incarnation.
2. **The restored VM re-opens the serial path from the SNAPSHOT's `config.json` and
   TRUNCATES it.** The CLI `--serial` on the restore command is ignored. This destroyed
   the pre-snapshot transcript and briefly read as "restore produced no output". Two
   consequences: the serial path must still exist on the restoring host, and any
   pre-snapshot log must be copied aside first.
3. **`pkill -x cloud-hypervisor` never matches anything.** The kernel truncates `comm` to
   15 chars — the process is `cloud-hyperviso`. Every such pkill was a silent no-op;
   stale VMMs accumulated and surfaced much later as an inexplicable `ApiSocketInUse`.
4. **`pkill -f "cloud-hypervisor --api-socket"` kills the invoking shell**, because the
   pattern matches the ssh/bash command line that contains it.

### What P8 does NOT establish

- **Volumes across restore were not tested.** This probe is rootfs-only. Whether a
  `--disk` volume, and separately a `--fs` virtiofs share, survive a restore on v53 is
  S-2 and is still open — and S-2 is the one that could still move I-6, since
  cloud-hypervisor#6931 is fixed in v52.0.
- **vsock across restore (S-3) was not tested.** Deliberately: increment-g uses the
  serial console precisely so "did restore work" is not entangled with "did the vsock
  peer reconnect".
  **CLOSED 2026-08-10 by P12 (increment-j).** The device survives and *new* connections
  work on the first post-restore tick; the *established* connection is destroyed, and the
  reset arrives one tick after the guest has already written into it successfully.
- **Restore onto a DIFFERENT host was not tested.** Everything here is same-host.
  **Still open, and P12 raised the price:** the snapshot's `config.json` records the vsock
  socket as an absolute path too, and restore fails hard if that path is occupied
  (`EADDRINUSE`) or its directory is missing (`ENOENT`).
- **No durability claim.** See the 0.05 s note above.
- **Clock behaviour is only partly characterised.** `CLOCK_MONOTONIC` continued smoothly
  across the gap (5322 ms → 5840 ms, one tick's worth) rather than accounting for the
  wall-clock time spent checkpointed. A proper before/after comparison of
  `CLOCK_REALTIME` deserves its own probe; it is not claimed here.

## P9 — S-2: volumes across restore. **Both survive. This overturns the research premise.**

Measured on env B, CH **v53.0**, as increment-g modes `blk` / `fs` / `fs-keepalive`.

The subject is deliberately not "does the volume still exist afterwards" — that is weak.
It is **an open file descriptor held across the checkpoint**: the guest opens
`persist.bin` on the volume *before* the snapshot and, every tick, `pwrite`s the tick
number through that same fd, `fsync`s, and `pread`s it back. For virtiofs that descriptor
corresponds to session state inside `virtiofsd`, which lives **outside** the snapshot.

| Arm | Volume | Result |
|---|---|---|
| `blk` | `--disk` virtio-blk | **`vol=ok` on every tick**, before and after. No interruption. |
| `fs` | `--fs` virtiofs, daemon killed with the VMM, **fresh** daemon before restore | **`vol=ok` before AND after**, and the host-side file reads `TICK-000020` — a post-restore guest write landed on the host. |
| `fs-keepalive` | virtiofs, same daemon kept alive | **Unconstructable — see below.** |

In both surviving arms the S-1 verdict also held: nonce identical, counter continued, no
reboot banner.

### The finding that matters: virtiofs is NOT disqualified by checkpoint/restore

`docs/research/platform/persistent-microvm-checkpoint-restore-comprehensive-research.md`
recommended block partly on the grounds that virtiofs cannot survive a checkpoint —
cloud-hypervisor#6931, plus the argument that live migration hands off between two
*simultaneously live* daemons whereas a checkpoint is a temporal gap in which no daemon
exists. **On v53 the gap is real, the daemon does die, and it works anyway.**

```
17:43:11 INFO  virtiofsd] Client connected, servicing requests
17:43:17 WARN  virtiofsd::vhost_user] Front-end did not announce migration to begin, so we
                failed to prepare for it; collecting data now.  If you are doing a
                snapshot, that is OK; otherwise, migration downtime may be prolonged.
17:43:17 INFO  virtiofsd] Client disconnected, shutting down
17:43:18 INFO  virtiofsd] Waiting for vhost-user socket connection...   <- FRESH daemon
17:43:18 INFO  virtiofsd] Client connected, servicing requests
```

virtiofsd names the snapshot case itself and blesses it: *"If you are doing a snapshot,
that is OK."* The guest's pre-existing fd keeps working because the replacement daemon
re-resolves the path; nothing in the FUSE session had to survive.

**So #6931 is fixed as far as this test reaches, and the CH-side half of the research's
argument no longer holds on v53.** The half that never depended on CH — that a block
volume cannot be read by the host while the guest runs — is untouched and remains the
real axis.

### `fs-keepalive` could not be built, and that is itself the answer

The mode was meant to contrast "daemon survives the checkpoint" against "daemon dies".
**It cannot exist:** `virtiofsd` shuts itself down the moment its client disconnects, and
it has no `--persist` / stay-alive option (checked against `--help`). Killing the VMM
therefore always destroys the daemon. The attempt failed exactly as that implies —
`PUT vm.restore` → **HTTP 500**, because no socket was listening.

**Driver consequence:** a virtiofs volume requires the driver to **start a fresh
`virtiofsd` on the same socket path before issuing `vm.restore`**. That is a real,
ordered step in the restore path, not an implementation detail — get it wrong and restore
fails with an opaque 500.

### A harness gap caught before it became a finding — the second in two probes

The first `fs` run reported `vol=none` for every tick and looked like a clean negative:
*virtiofs volume does not survive a checkpoint*. It was nothing of the sort. increment-g's
rootfs staged **no kernel modules at all**, and `CONFIG_VIRTIO_FS=m` — so the mount failed
with `ENODEV` before any of this was exercised. Staging `virtiofs.ko` and `insmod`-ing it
turned the same arm green:

```
init: insmod /modules/virtiofs.ko -> rc=0
init: S-2 mount volrw (virtiofs) at /mnt/vol -> rc=0 errno=0
init: S-2 volume fd=3 OPEN and will be held across the snapshot
```

This is the same shape as P8's CPU-hotplug near-miss (`/proc/cpuinfo` showing only *online*
CPUs). **Twice in two probes, a missing guest-side prerequisite would have produced a
confident, wrong negative** — and in both cases the wrong negative pointed the architecture
somewhere expensive. The general lesson: before recording "X does not work", confirm the
guest could have exercised X at all.

### What P9 does NOT establish

- **One file, one stable path, `--cache=never`.** Not tested: a file *unlinked* or renamed
  on the host during the gap, a share the host mutates while the VM is checkpointed, or
  `--cache=auto` (where guest-cached metadata could plausibly go stale).
- **Same host, same socket path.** Cross-host restore remains untested, and the snapshot
  embeds absolute paths.
- **No concurrency.** One VM, one volume, one fd.
- **Nothing about correctness under a crash** — this was a clean `vm.pause` → snapshot,
  never a power-cut mid-write.

### Where this leaves I-6

Checkpoint/restore no longer decides it: **both mechanisms survive.** The decision falls
back to the axes P7 already measured — live host access (virtiofs only), rate limiting
(`--disk` only), the per-VM daemon and its ordered restore step (virtiofs only),
`shared=on` and its `RLIMIT_FSIZE` interaction (virtiofs only), and the streaming-vs-
per-file throughput split. That is a decision for the user, and it should be taken on
those axes rather than on a checkpoint limitation that this probe just removed.

## P10 — S-8: `vhost-user-blk`. **It works, and it kills two of my own claims.**

Measured on env B, CH **v53.0**, backend `qemu-storage-daemon` (QEMU 10.2.1) — increment-h.
Reuses increment-g's kernel, rootfs and guest binary unchanged, so the only deliberate
variable is how the volume is attached.

**Why it was run:** #97 proposes `overdrive-fs` (content-addressed chunks in Garage +
per-rootfs libSQL + NVMe cache) served over **`vhost-user-fs`**. Fly Sprites converged on
the same *storage model* but a different *guest seam* — the guest sees ext4 on a block
device, with no virtiofs anywhere. `vhost-user-blk` is the block-shaped equivalent seam,
and it had never been measured here.

### It works

| Mode | Result |
|---|---|
| `shared` — vhost-user-blk + `--memory shared=on` | **Boots, `vol=ok` before and after, snapshot 204, restore 204, restored from memory** (nonce identical, tick 10 → 20, zero reboot banners) |
| `noshare` — vhost-user-blk without `shared=on` | **REFUSED at config validation** (below) |
| `plain` — ordinary `--disk` (the control) | Boots, `vol=ok`, restores from memory, **no `shared=on` needed** |

### The correction: two claims I made were wrong

I told the user that choosing block over virtiofs for #97's seam *"would keep rate
limiting, avoid `shared=on` and its `RLIMIT_FSIZE` trap, drop the per-VM daemon's ordered
restore step, and shed the 2.8× per-file FUSE overhead."* Measured against
`vhost-user-blk` specifically:

| Claim | Verdict |
|---|---|
| keeps rate limiting | **FALSE.** `ParsingConfig(Validation(VhostUserRateLimiterNotSupported))` — *"Rate limiting is not supported with vhost-user"* |
| avoids `shared=on` | **FALSE.** `ParsingConfig(Validation(VhostUserRequiresSharedMemory))` — refused before boot |
| drops the daemon's ordered restore step | **TRUE**, and it is the real advantage — see below |
| sheds the 2.8× per-file overhead | **UNMEASURED** for `vhost-user-blk`; the 2.8× was plain `--disk` vs virtiofs |

**The error class is the same one that produced the retracted "block runs in Lima" claim:
generalising from a measured thing (plain `--disk`) to an unmeasured adjacent thing
(`vhost-user-blk`) because they share a word.** Both `shared=on` and rate limiting are
properties of **vhost-user**, not of *block*. Every vhost-user transport pays them.

### The one real advantage, and it is genuine

**The block backend SURVIVES its client's death; `virtiofsd` does not.**

```
=== [4] kill the VMM
    backend daemon still alive after the VMM died: 1  (virtiofsd would be 0)
```

`virtiofsd` shuts down the moment its client disconnects and has no stay-alive option
(P9), which forces the driver to start a fresh daemon on the same socket path *before*
`vm.restore` — an ordered step that fails with an opaque HTTP 500 if missed.
`qemu-storage-daemon` simply keeps serving, and the restore reconnects to the running
backend. One fewer ordered step, and one fewer way for restore to fail.

### The corrected picture for #97's seam

| | `vhost-user-fs` | `vhost-user-blk` | plain `--disk` |
|---|---|---|---|
| Can front a custom backend (`overdrive-fs`) | yes | yes | **no** |
| `shared=on` required | yes | **yes** | no |
| Rate limiting | no | **no** | **yes** |
| Backend survives VMM death | **no** | **yes** | n/a |
| Ordered daemon restart before restore | **required** | not required | n/a |
| Survives snapshot/restore | yes (P9) | yes | yes (P9) |

**This reframes the argument.** `shared=on` and rate limiting are NOT fs-vs-blk
considerations — they belong to *"does this volume need a custom backend at all?"* Any
volume served by `overdrive-fs` pays `shared=on` and forfeits rate limiting **whichever**
vhost-user transport it picks. A volume that does **not** need a custom backend should use
plain `--disk`, which keeps both.

> **P11 amends what `shared=on` actually costs (2026-08-10).** The row above is
> still accurate about *which* mechanisms require it. But increment-i measured
> the price and found it is **55% of durable streaming throughput at the distro
> default — and recoverable to within ~2% by setting
> `/sys/kernel/mm/transparent_hugepage/shmem_enabled=advise` on the host.** The
> penalty is missing transparent huge pages behind the memfd, not vhost-user.
> So "pays `shared=on`" is a much weaker objection than it reads here, provided
> the appliance image sets that knob. See § P11.

Between the two vhost-user seams, the measured discriminators are: **daemon lifetime**
(blk wins clearly), **semantics vs the single-writer constraint** (blk matches it; a
block device is inherently single-writer), and **hydrate-on-access granularity** (fs gives
file-level semantic knowledge; blk gives block-level, which is what Sprites chose).

### What P10 does NOT establish

- **No throughput number for `vhost-user-blk`.** The daemon here is
  `qemu-storage-daemon` standing in for a future `overdrive-fs`; benchmarking it would
  measure QEMU's block layer, not ours. The per-file overhead comparison is therefore
  still open for this transport.
  **CLOSED 2026-08-10 by P11 (increment-i).** The framing above is right that it
  cannot bound `overdrive-fs` — but it is wrong that the measurement buys
  nothing. Plain `--disk` and `vhost-user-blk` write the *same image file* on the
  *same filesystem*, so their delta isolates the transport, and it is **zero**.
  P11 also found the default qemu export is not flush-durable.
- **`qemu-storage-daemon` is not the proposed backend.** It proves the *transport*
  composes; it says nothing about what a Rust chunk-store backend would cost.
- **Backend survival was observed once**, not stress-tested across repeated
  checkpoint cycles, and not with the backend itself restarted mid-flight.
  **UPDATE 2026-08-10 (P11):** re-observed in 5/5 further trials — every
  increment-i `vublk` run logs `backend still alive after the VMM died: 1`.
  Still not stress-tested across repeated checkpoint cycles.

## P11 — what `vhost-user-blk` COSTS (increment-i)

### Verdict: **the transport is FREE. `--memory shared=on` is what costs — and it is a host tunable, not a property of vhost-user.**

Measured on env B, CH **v53.0**, backend `qemu-storage-daemon` (QEMU 10.2.1),
`virtiofsd` 1.13.2 — increment-i. Closes the one thing P10 named as not
established: *"No throughput number for `vhost-user-blk`. [...] The per-file
overhead comparison is therefore still open for this transport."*

**The instrument is not new.** increment-i reuses increment-f's kernel, rootfs,
guest binary (`guest-init-blk`) and 1 GiB volume master unchanged, and drives the
virtiofs arm by invoking increment-e's `run.sh full` verbatim — the same thing
`vs-virtiofs.sh` did for P7. The payload functions in the two guest binaries are
**byte-identical** (`diff` of `measure_throughput` / `measure_per_file_latency` /
`write_file_bytes` returns empty), so every arm is measured with the same
instrument at the same syscall level. Payload is P7's: 256 MiB streamed in 1 MiB
chunks then one `fsync`, plus 1000 files each `open`+`write`+`fsync`+`close`.
Every arm reflink-clones the same pristine master per launch, runs the full P5
confinement stack, and is measured on the same XFS(reflink=1) NVMe.

Raw evidence: `spike-scratch/increment-i/` — `bench-5trial-full.txt`,
`thp-probe-full.txt`, `durability-probe-full.txt`, `matched-direct.txt`,
`mem-{plain,plain-shared,vublk}.txt`.

### The four arms, and why there are four rather than three

`vhost-user-blk` is **refused** without `--memory shared=on`
(`VhostUserRequiresSharedMemory`, P10). Plain `--disk` does not need it. So a
bare plain-vs-vublk comparison silently crosses a memory-backing change as well
as a transport change. The plain arm is therefore run **both ways**, and that
decision turned out to be the whole finding rather than a hygiene footnote.

| Arm | volume attachment | `--memory` |
|---|---|---|
| `plain` | `--disk path=…,image_type=raw` | `size=512M` |
| `plain-shared` | `--disk path=…,image_type=raw` | `size=512M,shared=on` |
| `vublk` | `--disk vhost_user=on,socket=…` | `size=512M,shared=on` |
| `virtiofs` | `--fs tag=…,socket=…` + virtiofsd `--cache=never` | `size=512M,shared=on` |

### 5 interleaved trials per arm — every sample, nothing averaged away

20/20 runs completed; **no trial was discarded**. Distro-default host settings,
CH and qemu default caching.

```
=== 256 MiB DURABLE write, MiB/s incl. fsync  <-- the headline
    plain         999.6 982.2 977.4 979.7 993.4
    plain-shared  455.9 451.0 436.5 444.8 446.4
    vublk         671.0 637.8 657.5 637.0 648.0
    virtiofs      885.6 886.1 888.5 879.0 886.9

=== 256 MiB write-only, MiB/s  (what P7 quotes; deferral-sensitive)
    plain         3431.1 3210.7 3191.0 3211.5 3377.0
    plain-shared  675.1 664.8 632.9 651.0 654.7
    vublk         684.0 650.3 668.6 648.8 659.0
    virtiofs      2292.9 2311.1 2321.4 2259.5 2318.3

=== 1000 files, mean ms/file
    plain         0.56 0.54 0.58 0.56 0.59
    plain-shared  0.61 0.63 0.60 0.60 0.60
    vublk         0.36 0.30 0.34 0.34 0.33
    virtiofs      0.41 0.40 0.42 0.42 0.41

=== write/fsync split, seconds (why write-only is not comparable)
    plain         0.075/0.181 0.080/0.181 0.080/0.182 0.080/0.182 0.076/0.182
    plain-shared  0.379/0.182 0.385/0.183 0.404/0.182 0.393/0.182 0.391/0.182
    vublk         0.374/0.007 0.394/0.008 0.383/0.006 0.395/0.007 0.388/0.007
    virtiofs      0.112/0.177 0.111/0.178 0.110/0.178 0.113/0.178 0.110/0.178
```

Matched host baseline on the same XFS, 20 samples: 256 MiB `dd conv=fsync`
**0.2187–0.2244 s** (≈1141–1171 MiB/s); 1000 files **0.15–0.16 ms/file**.

**Do the ranges overlap? No — every pair is disjoint on both metrics.**

| Metric | plain | plain-shared | vublk | virtiofs | overlap |
|---|---|---|---|---|---|
| durable MiB/s | 977.4–999.6 | 436.5–455.9 | 637.0–671.0 | 879.0–888.5 | **none, all 6 pairs disjoint** |
| ms/file | 0.54–0.59 | 0.60–0.63 | 0.30–0.36 | 0.40–0.42 | **none, all 6 pairs disjoint** |

The guest genuinely exercised the vhost-user device in every trial, and it is
worth stating because a "slower" verdict against a device the guest never
touched would be worthless: `FS-MOUNT dev=/dev/vdb -> OK`, `FS-RW-LISTING
[from-host.txt,lost+found]` (the master's seeded content, so `vdb` *is* the
vhost-user volume and not some other device), and after shutdown the host
loop-mount finds `payload.bin : 268435456 bytes`, `small files : 1000`, and
`from-guest.txt` byte-identical.

### `shared=on` costs 55% of durable throughput, and here is the mechanism

`plain` → `plain-shared` changes **one flag** and nothing else — same disk, same
image, same filesystem, same payload — and durable throughput falls from
977–1000 to 437–456 MiB/s, a **0.45×** collapse with disjoint ranges. The
`/proc` capture at the beacon says why, and it is not subtle:

```
### plain                                   ### plain-shared
RssAnon:         265108 kB                  RssAnon:            924 kB
RssShmem:             4 kB                  RssShmem:        265452 kB
memfd-ish mapping lines: 0                  memfd-ish mapping lines: 1
AnonHugePages:   264192 kB                  AnonHugePages:        0 kB
ShmemPmdMapped:       0 kB                  ShmemPmdMapped:       0 kB
### --- host THP policy ---                 ### --- host THP policy ---
  anon : always [madvise] never               anon : always [madvise] never
  shmem: always within_size advise [never]    shmem: always within_size advise [never]
```

`shared=on` backs guest RAM with a **memfd** instead of anonymous memory. CH
madvise's its guest RAM, so the anonymous case gets 2 MiB transparent huge pages
for essentially all of it (264192 of 265108 kB). The shmem case gets **zero**,
because Ubuntu's default for
`/sys/kernel/mm/transparent_hugepage/shmem_enabled` is `never`. Every guest page
fault in the write path then runs at 4 KiB granularity — and the cost lands
exactly where the split predicts, on the **write** phase (0.078 s → 0.39 s)
while `fsync` is untouched (0.182 s → 0.182 s).

This also explains why `virtiofs` does **not** pay it despite also running
`shared=on`: with `--cache=never` its writes go straight to the daemon and never
stage 256 MiB in the guest page cache, so it never faults in that memory.

**It is recoverable with a host setting.** 3 trials per cell:

| `shmem_enabled` | `plain-shared` durable MiB/s | `vublk` durable MiB/s | `ShmemPmdMapped` kB |
|---|---|---|---|
| `never` (distro default) | 451.6 / 454.0 / 458.5 | 614.0 / 646.6 / 658.9 | 0 / 0 / 0 |
| `advise` | **965.2 / 965.9 / 967.1** | 2936.1 / 2958.0 / 2968.7 | 272384 / 272384 / 266240 |
| `force` | **965.2 / 969.0 / 977.0** | 2885.1 / 3003.3 / 3094.9 | 272384 / 272384 / 274432 |

`advise` is enough — CH already madvise's the mapping — and it restores
`plain-shared` to within ~2% of unshared `plain` (966.1 vs 986.5 MiB/s, 2.07%
below). **The `shared=on` penalty is a
host-configuration defect, not a property of vhost-user or of virtiofs.** Both
mechanisms have been charged for it in this document until now.

### The number I nearly published, and the control that killed it

The `vublk` cells above (2936–3095 MiB/s "durable") are **not durable**, and
this is the most important paragraph in P11. A guest cannot durably write
256 MiB faster than the device beneath it. Measured on the same box, same
filesystem:

```
--- single-threaded, buffered + fsync (the baseline every arm is quoted against):
268435456 bytes (268 MB, 256 MiB) copied, 0.209147 s, 1.3 GB/s
    -> 1163.32 MiB/s
--- 4 concurrent writers, buffered + fsync (what a threaded backend can reach):
    -> 1292.49 MiB/s aggregate
```

So ~3000 MiB/s was never physically available. The decisive experiment is the
`cache.no-flush=on` **control**, which tells qemu to *discard* flush requests
outright — if that changes nothing, the flush was already doing nothing:

```
=== durable MiB/s (incl. fsync)
    writeback  3014.3 3189.0 2963.8      <- qemu-storage-daemon DEFAULT
    noflush    2997.3 3023.4 2892.1      <- flushes explicitly DISCARDED
    direct     1038.8 1041.1 1050.8      <- O_DIRECT
=== write/fsync split, seconds
    writeback  0.081/0.004 0.077/0.004 0.083/0.004
    noflush    0.081/0.004 0.081/0.004 0.084/0.004
    direct     0.223/0.024 0.221/0.025 0.218/0.025
```

`writeback` and `noflush` are indistinguishable. **At `qemu-storage-daemon`'s
default `cache.direct=off`, the guest's `fsync` on a `vhost-user-blk` volume
returns before the data is on the device.** That is a data-loss-on-host-crash
hazard and a driver requirement, not a benchmark footnote: a `vhost-user-blk`
volume must be exported with `cache.direct=on` (or an equivalent
flush-honouring contract) or the guest's durability guarantee is fiction.

Whether qemu is *dropping* the flush or never *advertising* a write cache for
the guest to flush is **not determined here** — the observable is the same
either way, and the `noflush` control settles the part that matters.

The same defect inflates the `vublk` column of the headline table above
(fsync 0.006–0.008 s). Its honest counterpart is in the next section.

### The transport delta, matched on everything that can be matched

`plain-shared` + `direct=on` and `vublk` + `cache.direct=on` write the **same
image file** on the **same filesystem**, with the **same memory backing**
(`shared=on`) and the **same caching contract** (O_DIRECT). The only remaining
difference is the vhost-user transport. 5 trials each, distro-default THP:

| Arm | durable MiB/s | ms/file |
|---|---|---|
| `plain` `direct=on`, **no** `shared=on` | 771.6 / 1005.1 / 1026.1 / 1028.0 / 1108.2 | 0.75–0.79 |
| `plain-shared` `direct=on` | 592.6 / 621.2 / 632.1 / 632.6 / 633.9 | 0.74–0.85 |
| `vublk` `cache.direct=on` | 595.0 / 611.8 / 623.4 / 631.8 / 651.4 | 0.51–0.77 |

**`plain-shared+direct` 592.6–633.9 vs `vublk+direct` 595.0–651.4 — the ranges
OVERLAP, and the means are 622.5 vs 622.7 MiB/s, a 0.03% difference.** Per-file
also overlaps (0.74–0.85 vs 0.51–0.77). **No throughput difference is claimable
between plain `--disk` and `vhost-user-blk` once the memory backing and the
caching contract are held equal. The vhost-user transport overhead is not
measurable at this payload.**

(The unshared `plain+direct` row keeps its 4th trial, 771.6, which is a clear
outlier — its `fsync` took 0.057 s against ~0.025 s everywhere else. It is
printed, not dropped, and it does not change the verdict: even taking the
range's *floor*, 771.6 still sits above `vublk`'s ceiling of 651.4.)

### What this corrects in P7

P7's headline — *"block, ~42% faster"* streaming — is a **write-only** figure,
and write-only is not comparable across mechanisms because the arms defer
different amounts of work past the timer. Recomputed from **P7's own pasted
evidence** in `vs-virtiofs.txt` (6 trials, unchanged):

| P7 metric | blk ÷ virtiofs |
|---|---|
| write-only MiB/s | 1.419 → *"~42% faster"*, as P7 states |
| **incl. fsync (durable)** | **1.110 → the real advantage is ~11%** |

increment-i reproduces this independently at 1.114 (`plain` 986.5 ÷ `virtiofs`
885.2, means over 5 fresh trials). So P7's *direction* holds — block is faster
streaming, virtiofs faster per small file, neither dominates — but the
**magnitude of the streaming advantage is ~11%, not ~42%**. P7's per-file
numbers are unaffected; those were always end-to-end.

### Harness defects caught in myself

Both are stated because the numbers above would have been wrong without them.

1. **I quoted `write only` MiB/s as the headline in the first cut**, inheriting
   P7's framing. The `plain` arm's write-only figure is high *precisely because*
   its bytes are still dirty in the guest page cache when the timer stops
   (0.078 s write / 0.182 s fsync); `vublk`'s is low because it defers almost
   nothing (0.387 s / 0.007 s). Quoting write-only compares **how much each
   transport defers**, not how fast it is. Fixed by making `total = write +
   fsync` the headline and printing the split alongside it so the reader can
   check the claim.
2. **I nearly recorded 2958 MiB/s as a `vhost-user-blk` win.** It survived a
   5-trial interleaved bench, a disjoint-ranges check, and a THP-recovery
   probe — none of which can detect a dropped flush, because all three measure
   the same lie consistently. What caught it was noticing the number exceeded
   the host's own baseline and then running the `noflush` control. **Range
   discipline and trial counts do not protect against a broken durability
   contract; only a control that removes the mechanism does.**

### Design implications

- **`shared=on` is not the cost centre this document has been treating it as.**
  Set `shmem_enabled=advise` on the appliance and the penalty largely vanishes
  (`plain-shared` 454 → 966 MiB/s). This is a host-image setting for the
  Overdrive appliance OS (ADR-0068 territory), and it applies to **every**
  `shared=on` path — virtiofs volumes included, not just vhost-user.
- **Any `vhost-user-blk` backend must honour flushes explicitly.** With
  `qemu-storage-daemon` that is `cache.direct=on`. A future `overdrive-fs`
  (#97) inherits the requirement: if its chunk store acknowledges a flush before
  the bytes are durable, every guest on it has a silent data-loss window.
- **Transport choice is not a throughput decision.** P10 established that
  `shared=on` and the loss of rate limiting belong to *vhost-user*, not to
  *block*. P11 adds that the transport itself is free. So the seam question for
  #97 stays exactly where P10 left it — daemon lifetime, single-writer
  semantics, hydrate-on-access granularity — with no performance tiebreak in
  either direction.
- **The per-file result mildly favours `vublk`** (0.30–0.36 vs plain's
  0.54–0.59 at default caching), but that advantage lives in the same
  non-durable regime as the streaming number; under matched O_DIRECT it becomes
  an overlap. Do not carry it forward as a win.

### What P11 does NOT establish

- **Nothing about `overdrive-fs`.** `qemu-storage-daemon` is a stand-in. These
  numbers are QEMU's block layer plus the vhost-user transport. The transferable
  claim is narrow and stated above: *the transport adds no measurable
  throughput cost*. What a Rust chunk store over Garage would cost is entirely
  unmeasured, and the transport being free is not evidence that a backend
  fronting it would be.
- **One VM, no contention.** Same limitation P7 carried. Nothing here says how
  any arm behaves with multiple VMs competing for one device, one backend
  daemon, or one NVMe queue.
- **One payload shape, one device.** 256 MiB sequential + 1000 small files on
  one NVMe on one AMD EPYC box. No random I/O, no read path, no queue-depth
  sweep, no `num_queues` tuning on either side — both were left at their
  defaults, which is what a driver would get but not what a tuned deployment
  would.
- **The flush mechanism is not diagnosed** — dropped-flush vs
  never-advertised-write-cache. The `noflush` control proves the flush
  contributes nothing; it does not say which.
- **No crash testing.** "The default caching contract is not durable" is
  established by the control, not by pulling power mid-write and counting what
  survived.
- **aarch64 is unmeasured**, as everywhere else in this document. Same blocker
  as P3's and P6's aarch64 halves.
- **`shmem_enabled=advise` was measured, not qualified.** Three trials on one
  box. Its memory-fragmentation and RSS consequences under many concurrent VMs
  — the usual reason distros default it to `never` — were not examined.

## P12 — S-3: vsock across snapshot/restore (increment-j)

### Verdict: **the VM restores, the DEVICE works, the CONNECTION does not — and the reset arrives one tick late.**

Measured on env B, CH **v53.0**. This closes the item P8 deferred on purpose:
increment-g used the serial console precisely so *"did the restore work"* could
not be confounded with *"did the vsock peer reconnect"*. This probe is that
confound, run deliberately. The driver's workload-Running gate rides on this
channel (P2), so it needed its own answer.

Raw evidence: `spike-scratch/increment-j/evidence/`.

Four answers, and the third and fourth are the ones that bind the driver:

| Question | Answer |
|---|---|
| **Q1** does a VM with `--vsock` snapshot and restore at all? | **Yes.** `vm.snapshot` 204, `vm.restore` 204, memory genuinely restored. The vsock device is fully functional afterwards. |
| **Q2** what happens to an ESTABLISHED connection? | **It is destroyed.** Guest sees `EPIPE (32)` on write and `EOF` on read; the host receives **zero** post-checkpoint bytes on it. But the teardown is **one tick late** — see the stale-write window. |
| **Q3** does restore FAIL if the host-side socket path is gone? | **Yes, two distinct ways, both fatal and both typed.** Stale socket file → `EADDRINUSE (98)`. Missing directory → `ENOENT (2)`. Both are HTTP 500 at 0.07 s, and both are **recoverable on the same VMM** by fixing the path and retrying. |
| **Q4** can a FRESH connection be made after restore? | **Yes, and immediately.** With a listener already bound it works on the *first* post-restore tick, end-to-end. With none bound the guest gets `ECONNRESET (104)` — **not** `ECONNREFUSED` — and recovers on the very next dial after a listener appears. |

### How the probe avoids a false pass — twice over

P8's RAM-only boot nonce carries forward unchanged: 16 bytes from `/dev/urandom`
read once, never written, printed with a monotonic tick counter. Nonce identical
+ counter continued = memory really came back; anything else and every sentence
above would be describing a fresh boot.

The second gate is new, and it exists because this spike has twice recorded a
confident **wrong negative** caused by a missing guest-side prerequisite (P8's
`/proc/cpuinfo`-lists-only-online-CPUs near-miss; P9's unstaged `virtiofs.ko`).
"vsock does not survive X" is exactly that shape of claim, so the guest proves it
could exercise vsock *at all* before anything is measured, and the run **aborts**
rather than reporting a negative if it could not:

```
init: /dev/vsock present BEFORE insmod = false
init: insmod /modules/vsock.ko -> rc=0
init: insmod /modules/vmw_vsock_virtio_transport_common.ko -> rc=0
init: insmod /modules/vmw_vsock_virtio_transport.ko -> rc=0
init: /dev/vsock present AFTER insmod = true
init: PREREQ OK socket(AF_VSOCK) = fd 3
init: HELD connect(cid=2,port=1234) OK fd=3 attempt=1
...
--- HELD lines host-side     = 11
--- reconnects host-side     = 11
    +++ PREREQ GATE PASSED: vsock demonstrably worked pre-snapshot.
```

All four arms passed it. And the payloads carry `n=<tick> nonce=<nonce>` so the
**host** transcript, not the guest's return value, is what settles delivery —
which is the only reason Q2's real shape was visible at all.

### The four arms

They differ **only** in what happens to the host side during the checkpoint gap,
so anything they agree on is attributable to the checkpoint rather than to
listener lifecycle. The VMM is always killed *before* the listeners, so a
listener's death can never propagate into the guest's saved socket state.

| Arm | host listeners | `ch.vsock` before restore |
|---|---|---|
| `drop` | killed with the VMM; fresh ones bound only **after** `vm.resume` | unlinked |
| `keep` | the **same processes** survive the checkpoint | unlinked |
| `stalesock` | killed | **left in place**, as the SIGKILL left it |
| `nosockdir` | killed | the whole **directory deleted** |

### Q1 — a VM with `--vsock` snapshots and restores

```
=== [2] vm.pause -> HTTP 204
=== [3] vm.snapshot -> /srv/vm/p12j/snap
    HTTP 204   elapsed .051589524s
    PUT vm.restore -> HTTP 204
=== [6] vm.resume -> HTTP 204

  BOOT_NONCE before          1da1ee2679e76f5f917fc6c5ec71a095
  BOOT_NONCE after           1da1ee2679e76f5f917fc6c5ec71a095
  last tick before           10
  first tick after           11
  last tick after            28
  boot banners after         0
  +++ RESTORED FROM MEMORY: nonce identical, no boot banner, counter continued.
```

The vsock device costs nothing at the snapshot layer: 0.052 s for 512 MiB, the
same page-cache-speed number P8 measured without it.

### Q2 — the established connection is destroyed, and the errno is one tick late

The connection is opened at boot, held across pause → snapshot → kill → restore →
resume, and written + read every 500 ms. Here is the checkpoint boundary in one
transcript (`drop`; the last two pre-snapshot ticks, then the first three after):

```
TICK n=9  nonce=1da1ee… mono_ms=4858 held_w=w=48 held_r=E11/EAGAIN new=ok(w=47)
TICK n=10 nonce=1da1ee… mono_ms=5363 held_w=w=49 held_r=E11/EAGAIN new=ok(w=48)
        <-- pause, snapshot, VMM killed, restore, resume -->
TICK n=11 nonce=1da1ee… mono_ms=5877 held_w=w=49    held_r=E11/EAGAIN new=E104/ECONNRESET
TICK n=12 nonce=1da1ee… mono_ms=6380 held_w=E32/EPIPE held_r=EOF     new=E104/ECONNRESET
TICK n=13 nonce=1da1ee… mono_ms=6885 held_w=E32/EPIPE held_r=EOF     new=E104/ECONNRESET
```

From n=12 the guest sees `EPIPE (32)` writing and `EOF` reading — a clean
peer-closed teardown, not a silent stale socket. **This confirms the expected
consequence of cloud-hypervisor#7958** ("reset on snapshot restore to avoid stale
half-open connections", reported in v52.0): the guest is told, promptly and
unambiguously, that the connection is gone. Two details a driver must not
generalise away: the read side reports **`EOF`, not `ECONNRESET`**, so a
reader-side gate sees an orderly close; and the host's own view ends at the
checkpoint, not at the restore —

```
[HELD t=+6.293s] conn#1 <- HELD n=10 nonce=964052c…
[HELD t=+6.870s] conn#1 CLOSED after 12 lines (total 12)
```

`n=10` is the last tick before the snapshot. **In every one of 16 runs the host
received exactly zero bytes carrying a post-checkpoint tick number.** The
connection never delivers again.

#### The stale-write window — and #7958's reset is *late*, not absent

Look again at n=11 above: `held_w=w=49`. The guest's `send()` **returned success
for 49 bytes**, and the host received none of them. That is a genuine
silently-stale half-open write, on the exact socket #7958 is supposed to have
reset. One observation cannot distinguish a deterministic window from a race, so
each arm was run eight times:

```
MODE       ITER lastB  firstA first_held_w   stale_writes  host_after  verdict
drop       1    10     11     held_w=w=49    1             0           mem_ok=1 STALE-WRITE
drop       2    10     11     held_w=w=49    1             0           mem_ok=1 STALE-WRITE
drop       3    10     11     held_w=w=49    1             0           mem_ok=1 STALE-WRITE
drop       4    10     11     held_w=w=49    1             0           mem_ok=1 STALE-WRITE
drop       5    10     11     held_w=w=49    1             0           mem_ok=1 STALE-WRITE
drop       6    10     11     held_w=E32/EPIPE 0           0           mem_ok=1
drop       7    10     11     held_w=w=49    1             0           mem_ok=1 STALE-WRITE
drop       8    10     11     held_w=w=49    1             0           mem_ok=1 STALE-WRITE
keep       1    10     11     held_w=w=49    1             0           mem_ok=1 STALE-WRITE
keep       2    10     11     held_w=w=49    1             0           mem_ok=1 STALE-WRITE
keep       3    10     11     held_w=w=49    1             0           mem_ok=1 STALE-WRITE
keep       4    10     11     held_w=E32/EPIPE 0           0           mem_ok=1
keep       5    10     11     held_w=w=49    1             0           mem_ok=1 STALE-WRITE
keep       6    10     11     held_w=w=49    1             0           mem_ok=1 STALE-WRITE
keep       7    10     11     held_w=w=49    1             0           mem_ok=1 STALE-WRITE
keep       8    10     11     held_w=E32/EPIPE 0           0           mem_ok=1
```

16/16 runs restored from memory; none contaminated. **13 of 16 (81%) swallowed
exactly one write** before the reset landed; 3 reported `EPIPE` on the first tick.
`stale_writes` never exceeded 1 and `host_after` was 0 in all 16, so the window is
**bounded by one 500 ms tick and probabilistic, not deterministic**. `drop` (7/8)
and `keep` (6/8) behave the same, which rules out listener lifecycle as the cause
— it is the checkpoint.

The precise claim, then: **#7958's reset fires, but the guest can get one
successful-but-discarded write in ahead of it.** "The socket errors immediately"
is wrong 81% of the time.

### Q3 — restore FAILS if the host-side socket path is wrong, in two distinct ways

The snapshot's `config.json` records an absolute path, and CH re-binds it on
restore:

```
--- Q3 evidence: what the snapshot's config.json records for vsock (ABSOLUTE path):
    {
      "id": "_vsock1",
      "pci_segment": 0,
      "cid": 3,
      "socket": "/run/spike-increment-j/vsock/ch.vsock"
    }
```

**`stalesock` — the stale UDS a SIGKILLed VMM leaves behind is fatal:**

```
    PUT vm.restore -> HTTP 500
    response body: ["Error from API","The VM could not be restored","Error from device manager",
                    "Cannot create virtio-vsock backend","Error binding to the host-side Unix socket",
                    "Address in use (os error 98)"]
    cloud-hypervisor: 0.076461s: <vmm> ERROR:vmm/src/lib.rs:2391 -- VM Restore failed:
      DeviceManager(CreateVsockBackend(UnixBind(Os { code: 98, kind: AddrInUse, message: "Address in use" })))
```

**`nosockdir` — the directory must exist; CH will not create it:**

```
    PUT vm.restore -> HTTP 500
    response body: [...,"Error binding to the host-side Unix socket","No such file or directory (os error 2)"]
    cloud-hypervisor: 0.075510s: <vmm> ERROR:vmm/src/lib.rs:2391 -- VM Restore failed:
      DeviceManager(CreateVsockBackend(UnixBind(Os { code: 2, kind: NotFound, message: "No such file or directory" })))
```

So the driver owes the restoring host two things, and they pull in opposite
directions: the **directory must exist** and the **socket file must not**.

**Both are recoverable in place** — the VMM survives its own refusal, and a
retry against the *same* process succeeds:

```
--- Q3 follow-up: remediate the path and retry vm.restore on the SAME VMM (pid 88079)
    (mkdir -p the directory; unlink the stale ch.vsock)
    retry PUT vm.restore -> HTTP 204
```

This is a much better failure mode than P8's `<api-socket>.lock`, which requires a
fresh process. It fails fast (0.07 s), types the cause, and leaves the VMM usable.

### Q4 — a fresh connection works, and the Running gate is recoverable

This is the question that decides whether losing the established connection
matters, and the answer is that it largely does not.

**With a listener already bound (`keep`), a new connection succeeds on the FIRST
post-restore tick** — the same tick whose *held* write was silently swallowed:

```
TICK n=11 … held_w=w=49    held_r=E11/EAGAIN new=ok(w=48) new_ok_total=12
TICK n=12 … held_w=E32/EPIPE held_r=EOF     new=ok(w=48) new_ok_total=13
```

and the host received **17** `NEW` payloads carrying ticks 11–27, i.e. every
single post-restore tick delivered end-to-end. The device is not damaged; only
the pre-existing connection is.

**With no listener bound (`drop`), the guest gets `ECONNRESET (104)`, not
`ECONNREFUSED`** — worth pinning, because a driver branching on `ECONNREFUSED`
would misclassify it. Recovery is immediate once a listener appears; the fresh
listener was bound between n=16 and n=17:

```
n=11 new=E104/ECONNRESET      [RECON2 t=+0.000s] bound …/ch.vsock_1235 (pid=87690)
…                             [RECON2 t=+0.016s] ACCEPT conn#1
n=16 new=E104/ECONNRESET      [RECON2 t=+0.016s] conn#1 <- NEW n=17 nonce=1da1ee…
n=17 new=ok(w=48)             [RECON2 t=+0.016s] conn#1 CLOSED after 1 lines (total 1)
```

The first dial after the bind lands, and its payload reaches the host 16 ms
later. There is no settling period and no device re-initialisation.

### Design implications — what this binds in the driver

- **The Running gate survives checkpointing, but only if the guest re-dials.**
  The channel is fine; the socket is not. A persistent workload that opened its
  beacon once at boot and held it will never speak again after a restore. Either
  the guest agent reconnects on error, or the gate must be re-established by the
  platform after every restore.
- **A successful `send()` is not delivery.** For up to one tick after restore the
  guest can write into a connection the host has already lost, and get success
  back. Any readiness signal that treats a local write as proof is wrong 81% of
  the time in exactly the window that matters. **Readiness must be acknowledged
  by the host, or observed on the host side.** This is the single most
  driver-relevant thing in P12.
- **Handle both `EPIPE` on write and `EOF` on read.** The teardown is an orderly
  close from the reader's point of view, not a reset.
- **`ECONNRESET`, not `ECONNREFUSED`, is "nothing is listening."** A retry loop
  keyed on the wrong errno will not retry.
- **The restoring host must `mkdir -p` the vsock socket directory and `unlink`
  the socket file.** Two paths, opposite requirements, both fatal if wrong. This
  sits alongside P8's `<api-socket>.lock` as a checkpoint-cleanup obligation —
  but unlike the lock, a mistake here is recoverable on the same VMM.
- **Cross-host restore inherits a new blocker.** The absolute vsock path joins
  kernel, disk and serial in the snapshot's `config.json`. It must resolve, be
  empty, and be writable on whichever host restores.

### Harness defects caught in myself

**The first `drop` run reported `HTTP 000` with an empty CH log, which reads
exactly like a Q3 restore refusal. It was not.** A concurrent probe on this
shared box — increments e and i, running under other sessions — SIGKILLed my
restoring VMM through its own `pkill -9 -x cloud-hyperviso`. Written up as-is it
would have been a fabricated finding: *"restore fails when the vsock socket is
cleaned"*, the exact opposite of what `drop` actually does.

Three things came out of that, all now in the probe:

1. **This probe kills only PIDs it recorded.** No name-scoped sweeps — so it
   cannot do to another probe what was done to it. This is a **fourth** `pkill`
   trap on top of P8's three, and the nastiest, because it is invisible in the
   probe's own source.
2. **A quiet-box guard** refuses to start while a foreign probe is live, polling
   rather than failing.
3. **A liveness check distinguishes the two shapes of `HTTP 000`.** `curl` gets
   no response both when CH refuses at the transport level and when the VMM was
   killed underneath it; the probe now tests `kill -0` on the VMM and reports
   `HARNESS DEFECT … this is NOT a Q3 result` rather than a verdict. It fired
   once more during the first repetition sweep (`keep` iteration 1, "VMM died
   during boot") and that row was discarded rather than averaged in.

The clean 16-run sweep quoted above was taken after all three landed, and carries
no contaminated rows.

**`vm.restore` is not idempotent**, and the first draft called it twice — once to
capture the response body, once for the status code. The second call would have
failed against an already-restored VM and reported a bogus refusal. Fixed to one
call before any arm was run.

### What P12 does NOT establish

- **Nothing about restarting the guest-side agent.** The probe holds one
  connection and re-dials a second port; it does not model a real agent's
  reconnect-with-backoff, nor how a workload that is *mid-request* on a lost
  connection behaves.
- **The stale-write window is bounded at 500 ms only because that is the tick
  period.** The true window is somewhere in `(0, 500] ms` and was not measured
  at finer resolution. A guest writing in a tight loop might swallow many more
  than one write; 13/16 runs swallowed exactly one because they only *tried*
  once.
- **Host→guest is untested.** Only the guest→host direction was exercised — the
  no-handshake path P2 established. The `CONNECT <port>` / `OK` handshake
  direction across a restore is unmeasured.
- **One port pair, one connection, no concurrency.** No test of many simultaneous
  vsock connections across a checkpoint, and no test of what happens to a
  connection with unread data buffered in either direction at snapshot time.
- **Same-host only**, like P8 and P9. The absolute-path finding sharpens the
  cross-host question but does not answer it.
- **No repeated checkpoint cycles.** Each arm snapshots once. Whether a VM
  restored twice, or restored from a snapshot taken of a restored VM, behaves the
  same is untested.
- **`--vsock` was never combined with volumes.** P9's `blk`/`fs` arms and this
  probe's vsock arms are disjoint; a workload with both is the realistic shape and
  was not run.
- **aarch64 is unmeasured**, as everywhere else in this document. The mechanism is
  a UNIX-domain socket on the host side (P2), so it is expected to transfer — but
  expected is not measured.

---

## P13 — `memory_restore_mode=ondemand` (increment-k)

### Verdict: **WORKS, and it makes restore latency O(1) in guest RAM — but it does NOT make a warm pool cheaper, and it is REFUSED under the uid-dropped shape P5 committed to.**

Measured on env B, CH **v53.0**, 2 GiB and 4 GiB guests, cold page cache.
This is the first measurement of the userfaultfd restore path added in CH
**v52.0 (#7800)**. S-7 established that a snapshot's `memory-ranges` is exactly
guest RAM and not sparse, so under `copy` the restore cost scales with guest
RAM; `ondemand` is the mechanism that was supposed to change that.

Raw evidence: `spike-scratch/increment-k/evidence/` —
`bench-2048-walk2.txt`, `bench-4096-walk2.txt`, `bench-2048-walk0.txt`
(the control), `api-probe.txt`, `uid-drop.txt`, `uid-drop-sysctl1.txt`,
`s5-n4-2048.txt`.

Three results, and they point in different directions. Taking only the first
would be the mistake:

1. **Latency: a real, large win.** `vm.restore` returns in ~12 ms instead of
   ~850 ms (2 GiB) or ~1.7 s (4 GiB), and it does not grow with guest RAM.
2. **Memory: no win at all.** The VMM reaches full guest RAM within ~2.5 s
   *whether or not the guest touches anything*. `ondemand` moves the read off
   the critical path; it does not avoid it.
3. **Composition: currently blocked.** Under P5's uid-dropped launch shape it
   fails closed with `Failed to create userfaultfd / Operation not permitted`.

### The API spelling is NOT the CLI spelling — and the field name is unvalidated

Two traps, both of which cost a driver silently rather than loudly.

`--help` documents `memory_restore_mode=copy|ondemand`. **The JSON API rejects
both of those.** It accepts only `Copy` and `OnDemand`. Established by sending a
deliberately-invalid value and reading serde's own enumeration back rather than
guessing (`api-probe.txt`):

```
  400  {"source_url":"file:///...","memory_restore_mode":"ondemand"}
       body: ["Failed to deserialize JSON",
              "unknown variant `ondemand`, expected `Copy` or `OnDemand` at line 1 column 82"]
  400  {"source_url":"file:///...","memory_restore_mode":"BOGUS_VALUE"}
       body: ["Failed to deserialize JSON",
              "unknown variant `BOGUS_VALUE`, expected `Copy` or `OnDemand` at line 1 column 85"]
  204  {"source_url":"file:///...","memory_restore_mode":"OnDemand"}
  204  {"source_url":"file:///...","memory_restore_mode":"Copy"}
```

The wrong VALUE is caught loudly with a 400. The wrong FIELD NAME is not caught
at all — `RestoreConfig` does not `deny_unknown_fields`:

```
  204  {"source_url":"file:///...","bogus_field":1}
```

**So a driver that misspells the key gets `Copy` behaviour, a 204, and no
signal whatever.** That is the one failure mode here that produces a wrong
answer silently, and it argues for the driver asserting on the observable
(restore latency, or VMM RSS immediately after restore) rather than trusting
the 204.

Third constraint, from the binary's own string table and confirmed by `--help`:
`'prefault' cannot be combined with 'memory_restore_mode=ondemand'`. **`prefault`
is a `Copy`-only knob**, so the three arms below are the entire option space.

### 5 interleaved trials per mode, per size — every sample, nothing dropped

30 bench trials + 9 control trials. **All 39 passed the correctness check
(`genuine=true`, `status=OK`); none was discarded.** Every trial drops the page
cache after a `sync(2)` and prints `Cached` before/after so a drop that did not
drop is visible.

```
=== 2 GiB guest, cold cache
                     restore_s (the vm.restore call itself)
    copy           0.6465 0.8435 0.8487 0.8755 0.8543
    ondemand       0.0175 0.0122 0.0151 0.0120 0.0126
    copy-prefault  1.5692 1.5606 1.5698 1.5793 1.5625

                     user_visible_ms (restore + resume + resume->first guest tick)
    copy            651.9  849.0  854.1  881.1  859.8
    ondemand         91.3   83.9   80.5   85.8   82.4
    copy-prefault  1578.9 1576.4 1579.3 1593.0 1570.1

                     resume_to_tick_ms (resume returned -> first post-restore tick)
    copy              4.2    4.3    4.2    4.3    4.3
    ondemand         72.5   70.5   64.1   72.6   68.5
    copy-prefault     8.5   14.6    8.3   12.5    6.3

                     rss_at_restore_kb (VMM RSS the instant vm.restore returned)
    copy           2102656 2102656 2102656 2102656 2102656
    ondemand         17404   13008   15488   13032   13444
    copy-prefault  2102716 2102716 2102720 2102720 2102716

=== 4 GiB guest, cold cache
                     restore_s
    copy           1.7253 1.6511 1.7432 1.7105 1.6669
    ondemand       0.0140 0.0143 0.0129 0.0126 0.0133
    copy-prefault  3.0960 3.0696 3.1155 3.0857 3.1032

                     user_visible_ms
    copy           1728.6 1656.6 1759.0 1718.0 1670.3
    ondemand         98.4   83.8   90.9   90.5   80.7
    copy-prefault  3103.6 3081.3 3131.4 3091.3 3117.0

                     rss_at_restore_kb
    copy           4199876 4199872 4199876 4199876 4199876
    ondemand         14632   12544   13488   11692   13792
    copy-prefault  4199936 4199936 4199940 4199936 4199936
```

Device baseline on the same XFS, cold, printed at the head of every bench run:
2 GiB read in **0.830 s O_DIRECT / 0.834 s buffered (2.6 GB/s)**. `copy`'s
0.65–0.88 s sits exactly on it — `copy` is device-bound, as it must be.

**Do the ranges overlap?**

| Metric (2 GiB) | copy | ondemand | copy-prefault | overlap |
|---|---|---|---|---|
| `restore_s` | 0.6465–0.8755 | 0.0120–0.0175 | 1.5606–1.5793 | **none, all 3 pairs disjoint** |
| `user_visible_ms` | 651.9–881.1 | 80.5–91.3 | 1570.1–1593.0 | **none, all 3 pairs disjoint** |
| `rss_at_restore_kb` | 2102656 (constant) | 13008–17404 | 2102716–2102720 | **ondemand disjoint from both; copy vs prefault disjoint by 60 kB** |
| `resume_to_tick_ms` | 4.2–4.3 | 64.1–72.6 | 6.3–14.6 | **copy and copy-prefault OVERLAP**; ondemand disjoint |

| Metric (4 GiB) | copy | ondemand | copy-prefault | overlap |
|---|---|---|---|---|
| `restore_s` | 1.6511–1.7432 | 0.0126–0.0143 | 3.0696–3.1155 | **none, all 3 pairs disjoint** |
| `user_visible_ms` | 1656.6–1759.0 | 80.7–98.4 | 3075.8–3131.4 | **none, all 3 pairs disjoint** |

**The one metric where `ondemand` LOSES is `resume_to_tick_ms`** — 64–73 ms
against `copy`'s 4.2–4.3 ms, disjoint. The guest's first instructions after
resume are faulting their way in, and that cost is real. It is an order of
magnitude smaller than the restore-call saving, so the total still favours
`ondemand` by ~10×, but a driver that measured only "time to first guest
progress after resume" would reach the opposite conclusion.

### Restore latency stops scaling with guest RAM — this is the structural result

| Guest RAM | `copy` restore | `ondemand` restore |
|---|---|---|
| 2 GiB | 0.6465–0.8755 s | 0.0120–0.0175 s |
| 4 GiB | 1.6511–1.7432 s | 0.0126–0.0143 s |

`copy` roughly doubles when RAM doubles, as S-7's non-sparse `memory-ranges`
predicts. **`ondemand` does not move at all** — 12 ms at 2 GiB, 13 ms at 4 GiB.
Its restore call is O(1) in guest RAM, and by extension so is
`rss_at_restore_kb` (~13 MB at both sizes). For an 8 or 16 GiB agent sandbox
that difference keeps growing; this is the one result that generalises beyond
the sizes measured.

### The control that decides whether any of this is lazy paging — WALK=0

P11 nearly published 2958 MiB/s on a 1163 MiB/s device because five interleaved
trials and a disjoint-ranges check cannot detect a broken mechanism; only a
control that removes the mechanism can. The equivalent here: a fast restore
with a `copy`-shaped RSS curve would not be lazy paging at all.

So the guest carries a rate-controlled memory WALK — it re-reads and verifies
its own touched region at a known 80 MiB/s — and the control run sets that rate
to **zero**. If RSS still climbs while the guest touches nothing, the fill is
not demand-driven.

**It still climbs, identically.** `guest_MiB` is the guest's own cumulative
verified walk, and it is `0` at every sample:

```
=== ondemand, WALK=0: the guest touches NOTHING after resume
      t_ms      VmRSS_kB   RssAnon_kB   RssFile_kB  RssShmem_kB  alive hostAvail_kB    guest_MiB
         0         12136         7580         4556            0   true     64242708       <none>
       100         70312        65752         4560            0   true     64140368            0
       200        161044       156484         4560            0   true     64008668            0
       300        253336       248776         4560            0   true     63912964            0
       500        436712       432152         4560            0   true     63732636            0
       750        667000       662440         4560            0   true     63503656            0
      1000        898884       894324         4560            0   true     63269980            0
      1500       1365772      1361212         4560            0   true     62802524            0
      2000       1829216      1824656         4560            0   true     62346268            0
      3000       2102684      2098124         4560            0   true     62106168            0
      6000       2102684      2098124         4560            0   true     62106144            0
     10000       2102684      2098124         4560            0   true     62118432            0

=== copy, WALK=0: full at restore-return, flat forever
         0       2102664      2098104         4560            0   true     62018776       <none>
       100       2102664      2098104         4560            0   true     62033608            0
      1000       2102664      2098104         4560            0   true     62034000            0
     10000       2102668      2098108         4560            0   true     62078196            0
```

`rss_t1000_kb` with the guest walking 80 MiB/s: **878324–885636**. With the
guest walking **nothing**: **872304–900992**. The two are indistinguishable —
in fact the idle guest fills marginally *faster*, because it is not competing
for the same pages.

The climb is dead linear at **~900 MiB/s** and completes in ~2.5 s regardless
of guest behaviour. And the VMM thread dump at t=750 ms names the mechanism
outright — `copy` is burning CPU in `vmm`, `ondemand` in a thread that exists
only in this mode:

```
### ondemand                                  ### copy
tid=99177 comm=uffd-handler    cpu_jiffies=72  tid=99134 comm=vmm         cpu_jiffies=40
tid=99179 comm=vcpu0           cpu_jiffies=10  tid=99138 comm=vcpu0       cpu_jiffies=8
tid=99173 comm=cloud-hyperviso cpu_jiffies=0   tid=99133 comm=cloud-hyp.. cpu_jiffies=0
tid=99174 comm=vmm             cpu_jiffies=0   tid=99135 comm=http-server cpu_jiffies=0
```

**So `OnDemand` is lazy only at the boundary.** It is *not* steady-state demand
paging: `vm.restore` returns immediately with ~13 MB resident, and a
`uffd-handler` thread then streams the entire snapshot in as fast as it can,
whether the guest wants it or not. The honest one-line description is
**"asynchronous eager restore"**, not "lazy paging". There is no knob to stop
the backfill — `prefault` is `Copy`-only.

That distinction is the whole difference between the two claims a reader might
take away:

- ✅ **"Restore returns ~70× faster and the guest resumes in ~85 ms"** — true,
  measured, disjoint ranges, holds at 2 and 4 GiB.
- ❌ **"A restored VM only pays for the memory it touches"** — **false.** It
  pays for all of it within ~2.5 s. Any warm-pool sizing built on the second
  claim would be wrong by the full guest RAM per VM.

### S-5 — N simultaneous restores of one snapshot: N × RAM, in BOTH modes

Reported separately so it cannot dilute the latency result. N=4, 2 GiB each,
`s5-n4-2048.txt`.

**Phase A — the naive form does not work.** Restoring the *same* snapshot
directory into two VMMs fails on the second, because `config.json` records an
absolute rootfs path and v53 takes an exclusive write lock on block devices:

```
  VM0  restore -> HTTP 204   resume -> HTTP 204
  VM1  restore -> HTTP 500   resume -> HTTP 500
    Can't get Write lock for /run/.../rootfs-t1-copy-2048.ext4
      as there is already a ExclusiveWrite lock
    VM Restore failed: LockingError(DiskLockError(LockDiskImage {
      error: AlreadyLocked, lock_type: Write, path: "..." }))
```

A warm pool must therefore materialise a **per-VM snapshot copy with
`config.json` rewritten** (rootfs path and serial path at minimum). That is
cheap on XFS — see the reflink note below — but it is a required driver step,
not an optimisation.

**Phase B — with per-VM copies, all 4 restore fine, and the host pays 4 × RAM:**

```
--- mode=Copy   baseline MemAvailable=64112808 kB
    all 4 launched in 3.535838440s
    t+1s   alive=4/4  sum(VmRSS)=8410920 kB  delta=8391360 kB
    t+15s  alive=4/4  sum(VmRSS)=8410932 kB  delta=8388684 kB

--- mode=OnDemand   baseline MemAvailable=64303188 kB
    all 4 launched in .206582601s          <- 17x faster to LAUNCH
    t+1s   alive=4/4  sum(VmRSS)=3610292 kB  delta=3759844 kB   <- still filling
    t+5s   alive=4/4  sum(VmRSS)=8410984 kB  delta=8543860 kB   <- caught up
    t+15s  alive=4/4  sum(VmRSS)=8410984 kB  delta=8553468 kB
```

`sum(VmRSS)` converges to **8410932 kB (Copy) vs 8410984 kB (OnDemand)** — a
0.0006% difference on 8 GiB. `MemAvailable` fell by ~8.4 GiB in both. The
per-VMM breakdown proves there is no sharing to find:

```
        pid=101391 Rss=2102684 Pss=2099280 Shared_Clean=4536 Shared_Dirty=0 Private_Dirty=2098144 kB
        pid=101406 Rss=2102768 Pss=2099308 Shared_Clean=4620 Shared_Dirty=0 Private_Dirty=2098144 kB
```

**`Pss` is within 0.16% of `Rss`, and `Private_Dirty` is the entire guest RAM.**
Guest memory comes back as private anonymous pages in every VMM under both
modes (`RssAnon` climbs, `RssFile` stays flat at ~4.5 MB — the binary's text).
Nothing is shared, and the identical snapshot on disk does not change that.

**So `ondemand` buys warm-pool LAUNCH latency (0.21 s vs 3.54 s for 4 VMs), not
warm-pool DENSITY.** N suspended-then-restored VMs cost N × guest RAM either
way. If density is the goal, the mechanism has to come from somewhere else
(KSM, or a hypervisor that maps the snapshot shared-and-CoW) — and neither is
in evidence here.

### Storage: the snapshot is still N × RAM on disk, but reflink makes copies free

S-7's result holds unchanged at both sizes — `memory-ranges` is exactly guest
RAM, apparent == on-disk, no sparseness:

```
### snapshot bytes: apparent=4295008452 on_disk=4295012352 (guest RAM = 4294967296 B)
###   config.json                1060 B            4096 B on disk
###   state.json                40096 B           40960 B on disk
###   memory-ranges        4294967296 B      4294967296 B on disk
```

The per-VM snapshot copies Phase A forces are nonetheless nearly free, because
XFS reflink covers them (P4's mechanism, re-confirmed here):

```
  df used before=129415444 kB   after reflink of a 2 GiB file=129415444 kB   delta=0 kB
  df used after a FULL copy of the same 2 GiB file:                          delta=2105564 kB
```

**A correction to my own earlier reading in this probe:** the S-5 run prints
`on disk : 8.1G` for 4 reflinked copies, which looks like reflink failing. It
is not — `du` counts shared extents once per file and therefore *cannot* show
deduplication. `df` is the instrument that can, and it says the reflinked copy
costs zero blocks. I nearly recorded "reflink silently fell back to a full
copy" off the `du` number alone.

### `prefault=on` is a pure cost on this path — do not enable it

| Guest RAM | `copy` restore | `copy` + `prefault` | RSS at restore-return |
|---|---|---|---|
| 2 GiB | 0.6465–0.8755 s | 1.5606–1.5793 s | identical (2102656 vs ~2102718 kB) |
| 4 GiB | 1.6511–1.7432 s | 3.0696–3.1155 s | identical (4199874 vs ~4199937 kB) |

`prefault` costs **~1.8×** the restore time at both sizes, with disjoint ranges,
and buys nothing observable: plain `copy` *already* has the full guest RAM
resident the instant `vm.restore` returns, so there is nothing left to
pre-fault. `resume_to_tick_ms` also fails to improve — 6.3–14.6 ms with
prefault against 4.2–4.3 ms without, i.e. it is no better and the ranges do not
even overlap in prefault's favour. **Leave it off.**

### The composition failure — `OnDemand` is REFUSED under P5's uid-dropped shape

This is the finding that decides whether the feature is usable at all, and it
is not visible from any latency number.

P5 settled that production CH runs **uid-dropped**:
`prlimit … setpriv --reuid=spikevmm --regid=6001 --init-groups --no-new-privs
-- cloud-hypervisor … --seccomp true --landlock`. `OnDemand` is implemented with
userfaultfd. This box has the distro default
`/proc/sys/vm/unprivileged_userfaultfd = 0`, which restricts `userfaultfd(2)` to
processes holding `CAP_SYS_PTRACE`. Two established facts pointing opposite
ways; only a run settles it. Both modes were attempted under the **same** dropped
uid, so a failure that is really about file permissions would appear in both.

```
=== mode=Copy  as uid=6001 (setpriv --no-new-privs), NOT root
  VMM pid=97400  running as uid=6001
  PUT vm.restore -> HTTP 204   in .868561554s
  PUT vm.resume  -> HTTP 204
  VmRSS=2102752 kB       guest ticks observed: 105

=== mode=OnDemand  as uid=6001 (setpriv --no-new-privs), NOT root
  VMM pid=97442  running as uid=6001
  PUT vm.restore -> HTTP 500   in .027233557s
      body: ["Error from API","The VM could not be restored","Memory manager error",
             "Cannot restore VM","On-demand restore failed","Failed to create userfaultfd",
             "Operation not permitted (os error 1)"]
  PUT vm.resume  -> HTTP 500
  guest ticks observed: 0
  VM Restore failed: MemoryManager(Restore(OnDemandRestore(Create(
    Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }))))
```

**It fails CLOSED**, with a typed error naming the exact cause, and does not
silently degrade to `Copy` — which matches `--help`'s claim that `ondemand`
"fails restore if userfaultfd support is unavailable" and is the right
behaviour. But it means **`OnDemand` and the `[D7]` confinement stack do not
compose as shipped.**

The remedy is a single host sysctl, and it is verified rather than assumed —
same script, same dropped uid, only the sysctl changed:

```
vm.unprivileged_userfaultfd = 1
=== mode=OnDemand  as uid=6001 (setpriv --no-new-privs), NOT root
  VMM pid=97634  running as uid=6001
  PUT vm.restore -> HTTP 204   in .024742598s
  PUT vm.resume  -> HTTP 204
  VmRSS=2102764 kB       guest ticks observed: 104
```

(The box was returned to `vm.unprivileged_userfaultfd = 0` afterwards.)

This is the direct analogue of P11's `shmem_enabled=advise`: a capability the
platform wants, gated on a host-image sysctl the distro defaults against. It
belongs to the appliance OS (ADR-0068 territory) — with the caveat that
`unprivileged_userfaultfd=1` widens userfaultfd to every unprivileged process
on the host, not just the VMM, and userfaultfd is a well-known primitive for
stalling kernel faults during exploitation. **That is a security trade-off to
decide deliberately, not a checkbox**, and the alternative (granting CH
`CAP_SYS_PTRACE`) is plainly worse.

### Re-checkpointing an `OnDemand`-restored VM works

The binary carries a `VmError` reading `VM on-demand memory restore is still in
progress`, so CH has a state in which operations are refused. Every trial
therefore re-checkpoints after the sampled window:

```
=========== RE-CHECKPOINT after a ondemand restore ===========
  GET vm.info -> 200  state="Running","memory_ac
  PUT vm.pause -> 204
  PUT vm.snapshot -> 204 in 0.192 s  apparent=2147524634 on_disk=2147528704
```

A VM restored with `OnDemand` can be paused and snapshotted again, producing a
byte-count-identical snapshot. **Warm-pool chaining (restore → run → re-suspend)
works.** Note this ran ~10 s after resume, i.e. *after* the background fill
completed — see § What P13 does NOT establish.

### Harness defects caught in myself

All four are stated because the numbers above would have been wrong without
them, and three of them produced plausible-looking output.

1. **`drop_caches` without `sync(2)` drops nothing.** The first cut wrote `3` to
   `drop_caches` and logged "dropped host page cache". `vm.snapshot` had just
   written 2 GiB in 0.19 s — 11 GB/s, which no device on this box can do — so
   the file was still **dirty**, and dirty pages are not droppable. `copy` then
   "read" 2 GiB in 0.2316 s = 8.8 GB/s against a measured cold device rate of
   2.6 GB/s. **That is the P11 shape exactly: a number faster than the hardware
   beneath it.** Fixed by `sync()` first and by printing `Cached` across the
   drop so a no-op drop is visible (`Cached 4393668 -> 166172 kB`). Cold `copy`
   is 0.85 s, not 0.23 s — the corrected number is 3.7× worse, and it is the one
   in the tables above.
2. **Parsing a console file mid-write.** `read_console` returned whatever bytes
   existed, including a half-written final line. The first run reported
   `nonce=360349041693d9c6c8e5290d5b3e` — 28 hex chars of a 32-char nonce — and
   `walked_mib=<none>`. Unfixed this corrupts the probe in **both** directions:
   a truncated `nonce_before` can never match a complete `nonce_after`, so a
   genuine restore reports as a false negative; and worse, a truncated
   `TICK n=123` parses as `n=12`, so the "first tick above the floor" search can
   fire on a **pre-snapshot** tick and report a resume latency that never
   happened. Fixed by truncating the transcript at its last newline.
3. **S-5 Phase A failed for the wrong reason and I nearly wrote it up.** The
   first run showed both VMs failing and would have supported "N-way restore is
   refused" — but the error was `ENOENT` on the rootfs, because the bench
   deletes its per-run rootfs on exit and the snapshot records an absolute path.
   The disk-lock hypothesis was never tested. Recreating the file at the
   recorded path produced the real result above (`AlreadyLocked`), which is the
   same verdict with a **different mechanism** and a different remedy.
4. **`du` cannot see reflinked extents.** Nearly recorded "reflink silently fell
   back to a full copy" from `du`'s 8.1 G on 4 copies. `df` says the delta is
   zero. Corrected in § Storage above.

A fifth, procedural: **`infra/metal/bootstrap.sh --sync-only` runs
`rsync --delete`, which deleted the first bench's evidence files** from the
remote tree when I re-synced a code edit. Bench logs are now written under
`/var/tmp/spike-increment-k/ev/`, outside the synced tree, and pulled
explicitly.

### Design implications

- **Use `OnDemand` for restore, and assert on the observable rather than the
  204.** The driver sends `{"source_url":…,"memory_restore_mode":"OnDemand"}` —
  **PascalCase**; the CLI's `ondemand` is rejected and an unknown *field* is
  silently ignored, so a misspelling degrades to `Copy` with no error.
- **The appliance OS needs `vm.unprivileged_userfaultfd=1`**, or `OnDemand`
  cannot be used under the `[D7]` uid-dropped shape at all. This is a host-image
  decision with a real security trade-off (§ composition failure above), and it
  sits beside P11's `shmem_enabled=advise` as the second sysctl this feature
  requires of ADR-0068.
- **Never enable `prefault`.** ~1.8× the restore time at both sizes, no
  measurable benefit, and it is `Copy`-only anyway.
- **Do not size a warm pool on lazy memory.** N restored VMs cost N × guest RAM
  within ~2.5 s in both modes; `Pss ≈ Rss` and `Private_Dirty` = full guest RAM
  in every VMM. `OnDemand` buys pool *launch* latency (0.21 s vs 3.54 s for 4
  VMs), not pool *density*. The density argument for suspended VMs still rests
  entirely on VMs that are **not** restored.
- **A warm pool must rewrite `config.json` per VM.** The snapshot records
  absolute rootfs and serial paths, and v53 takes an exclusive write lock on the
  disk image, so N VMs from one snapshot need N snapshot copies with rewritten
  paths. Reflink makes the copy free in space (`df` delta 0) and P4 makes it
  fast, but the rewrite step is mandatory.
- **`vm.snapshot`'s wall time is not the checkpoint cost.** 0.19 s for 2 GiB and
  ~0.4 s for 4 GiB are page-cache speed; the following `sync(2)` took **1.422 s
  (2 GiB)** and **2.844 s (4 GiB)**. P8 flagged this at 512 MiB; it is now
  quantified at both sizes. A driver that reports "checkpointed" on the 204 is
  reporting on unflushed data.

### What P13 does NOT establish

- **Nothing about a partially-filled VM's behaviour under pressure.** Every
  measurement here runs on a box with 61 GiB free, where the background fill
  always completes. What `OnDemand` does when the host cannot satisfy the
  backfill — memory pressure, cgroup limit, many simultaneous restores
  contending — is unmeasured, and that is precisely the warm-pool regime.
- **Re-checkpoint DURING the fill is untested.** The re-snapshot above runs ~10 s
  after resume, long after the ~2.5 s fill. CH carries a distinct error, `VM
  on-demand memory restore is still in progress`, which nothing here triggered —
  so whether `vm.pause`/`vm.snapshot`/`vm.resize` are refused in the first ~2.5 s
  is **open**, and a warm pool that suspends quickly would hit exactly that
  window.
- **No cross-host restore**, as in P8/P9/P12. Everything is same-host. The
  absolute-path and disk-lock findings sharpen that question without answering
  it.
- **The ~900 MiB/s fill rate is not attributed.** It is well under the device's
  2.6 GB/s and well under page-cache speed, so it is bounded by something in the
  uffd path (single handler thread, per-page `UFFDIO_COPY` cost) — but which was
  not measured, and no attempt was made to tune it.
- **`OnDemand` was not combined with volumes, vsock, or CPU hotplug.** P9's
  `blk`/`fs` arms, P12's vsock arms and P8's S-6 hotplug all ran under `Copy`.
  A workload with a volume *and* an `OnDemand` restore is the realistic shape
  and was not run — and `--memory shared=on` (which virtiofs and vhost-user-blk
  both require, P10/P11) changes the memory backing that userfaultfd is
  registered against, so it is not safe to assume it composes.
- **The uid-drop test used `--seccomp true` but not `--landlock`.** P5's full
  stack includes Landlock rules, and P5 already found CH's implicit ruleset
  incomplete for the vsock UDS. Whether Landlock additionally interferes with
  the uffd path is untested.
- **aarch64 is unmeasured**, as everywhere else in this document.
- **No claim about kernels other than 7.0.0-15-generic.** userfaultfd
  permissions and semantics have changed across releases; the pinned 6.18
  appliance kernel (P3, still NOT RUN) is a different subject.

## Still open

- ~~**S-3 — vsock across restore.**~~ **ANSWERED 2026-08-10 by P12 (increment-j).** The
  device and new connections survive; the established connection does not, and the reset
  is one tick late. What P12 leaves open is narrower and listed in § What P12 does NOT
  establish — chiefly the **host→guest** direction (only guest→host was exercised),
  **concurrent connections**, **repeated checkpoint cycles**, and **`--vsock` combined
  with volumes**, which is the realistic workload shape and has never been run as one VM.
- ~~**Does `ondemand` restore make a warm pool cheaper?**~~ **ANSWERED 2026-08-10 by
  P13 (increment-k), and the answer is NO on memory and YES on latency.** What P13
  leaves open is listed in § What P13 does NOT establish; the two that matter for the
  warm-pool design are **the backfill under memory pressure** (every measurement here
  ran with 61 GiB free, so the fill always completed) and **re-checkpoint DURING the
  ~2.5 s fill** — CH carries a distinct `VM on-demand memory restore is still in
  progress` error that nothing in P13 triggered, and a pool that suspends quickly would
  land in exactly that window. Also open: `OnDemand` combined with volumes, vsock, or
  `--memory shared=on`, none of which were run together with it.
- **Whether the appliance takes `vm.unprivileged_userfaultfd=1`.** This is a decision,
  not a measurement. P13 proved the sysctl is *necessary* for `OnDemand` under the
  `[D7]` uid-dropped shape and *sufficient* to fix it, but it widens userfaultfd to
  every unprivileged process on the host, and userfaultfd is a known primitive for
  stalling kernel faults during exploitation. Owner: whoever owns ADR-0068.
- **Cross-host restore.** Everything in P8, P9 and P12 is same-host. A checkpoint that
  cannot move between machines is a much weaker product primitive, and the snapshot embeds
  absolute host paths (kernel, disk, serial, **and — P12 — the vsock socket**) that must
  exist on the restoring host. P12 sharpened the cost: the vsock path is the one entry
  with *contradictory* requirements — its directory must exist and its socket file must
  not — and getting either wrong is a hard `HTTP 500` restore failure.
- **P6 on aarch64.** P6 is answered on x86_64 (increment-e). `shared=on` still cannot be
  measured on env A — it does not boot under nested virt — and no non-nested Arm hardware
  is available. Both arches ship, so the virtiofs + `shared=on` path is proven on **one of
  two** shipping targets. Same blocker as P3's aarch64 half; one piece of hardware closes
  both.
- **P3 (pinned 6.18 kernel).** Not run on either environment. Both x86_64 and aarch64
  ship, so this is two questions, not one. Env B has `7.0.0-15`; the LVH path
  (`cargo xtask integration-test vm --kernels …`) is the route on x86_64, and aarch64
  needs non-nested Arm hardware.
- **P5's `--vsock` landlock correction on env B.** Mostly retired: increment-e's `full`
  mode ran the entire P5 stack — `--seccomp true`, `--landlock` with per-VM rules, the
  unprivileged `spikevmm` uid + `kvm` group, and `prlimit` — through five completed boots
  on x86_64, and the per-thread seccomp shape reproduced exactly (leader `Seccomp: 0`, all
  ten workers `Seccomp: 2`, `NoNewPrivs: 1` throughout). What increment-e did **not**
  re-derive is correction 1, the explicit `--vsock` UDS directory grant: it inherited the
  rule rather than re-testing that omitting it fails.

### Probe portability — fixed 2026-08-10, and one trap worth carrying

The increment-a probes were aarch64-only. Four things were arch-bound; three were
mechanical, one is a genuine trap:

| Bound to arch | Fix |
|---|---|
| `TARGET=aarch64-unknown-linux-musl` | `$(uname -m)-unknown-linux-musl` |
| Kernel prep (UKI unwrap) | x86_64 copies `vmlinuz` verbatim; only aarch64 unwraps |
| `__NR_finit_module` | 273 on aarch64, **313 on x86_64** — verified from the box's own `asm/unistd_64.h:317`, not memory. Unhandled arches remain a `compile_error!` rather than a wrong syscall. |
| **`console=ttyAMA0`** | **`ttyS0` on x86_64.** `ttyAMA0` is the ARM PL011 UART. |

**The console name is the trap.** Get it wrong and there is **no error** — the guest
simply produces no console output at all, which is indistinguishable from a hang. On a
platform where the known failure mode *is* a silent boot stall, that would have been
extremely expensive to diagnose.

---

## Gate recommendation

**P1 + P2 — PROMOTE, and the nested-host caveat is RETIRED.** Confirmed on non-nested
x86_64 bare metal, 12/12 boots with a 16 ms spread. Slice 01 may proceed on the vsock
beacon and the netns placement, carrying the build-time requirements: vsock built-in (or
three modules loaded in order), a static `/dev/console` in the image, and — **on aarch64
only** — a raw `Image` unwrapped from the distro UKI.

**P4 — PROMOTE. `[D5]` stands.** Reflink is ~260× faster and free in space
(0.015 s / +0 MiB vs 3.970 s / +4096 MiB for 4 GiB), extents confirmed shared. Carry the
implication: the `--reflink=auto` flag is redundant on coreutils ≥ 9.0, but the
**filesystem guarantee is not** — on ext4 the same command silently becomes a full copy
with no error.

**P5 — PROMOTE, conditional.** `[D7]` / US-VM-7 must absorb three corrections *before*
Slice 03 builds on them:

1. The vsock UDS needs an **explicit per-VM `--landlock-rules` directory** grant (CH does
   not auto-derive it, a read-only rule fails, and the rule cannot name the socket path) —
   therefore each VM's socket directory must hold nothing else.
2. Seccomp must be verified **per-thread** (`/proc/<pid>/task/*/status`); the
   thread-group leader reports `Seccomp: 0` on a correctly-confined CH.
3. `RLIMIT_FSIZE` must be **`max(rootfs image, guest RAM)`** whenever `shared=on` is used.

The uid question is settled: **unprivileged uid + `kvm` group membership against `0660
root:kvm`**. No 0666 needed.

**P6 — PROMOTE on x86_64.** Run on env B as increment-e: five modes, all completed.
`[D8b]`, `[D8d]`, `[D8e]`, and the `--fs socket=` auto-derivation all hold, and
**`[D8g]`'s host-side `read_only` is now verified** against a guest that mounted the
export read-write and tried to write it — refused with `EROFS`, host tree untouched. The
`[D8a]` security framing may be asserted. Two things go back to design rather than
forward: **`[D8c]`'s `--cache=never`** is confirmed for the bulk path but is ~15% *worse*
per small file, which makes cache mode a candidate per-volume knob rather than a
constant; and **`RLIMIT_FSIZE` must be `max(rootfs image, guest RAM)`**, reproduced
independently on x86_64. The open half is **aarch64**, which no available hardware can
measure.

**P7 — the I-6 volume decision goes back to the user, with numbers.** Both mechanisms
work. Performance does not decide it: block is ~42% faster streaming, virtiofs ~25% faster
per small file, neither overlapping. What *does* decide it is whether volumes need the host
reading or writing them **while the guest runs** — the one thing block cannot do at any
price. If they do, virtiofs, and the platform accepts a per-VM daemon, `shared=on`, no rate
limiting, and a volume path that cannot run on the Apple-Silicon dev host. If they do not,
block wins on every remaining axis. **That question was never asked during DISCUSS** — I-6
recorded *which* mechanism before anything recorded *what volumes are for*. This is a
scoping gap to close with the user, not a probe result to act on unilaterally.

**P8 — PROMOTE, and it retires the largest open risk in the feature.** Snapshot/restore
works on v53 through the API; the CLI `--restore` is a silent no-op and the driver must
not use it. **CPU hotplug composes with restore**, which was the highest-risk unknown
here — it is the stated reason this feature chose Cloud Hypervisor, Firecracker forbids
it, and nobody had checked. Four operational traps (the `<api-socket>.lock`, the
truncating serial re-open, and two `pkill` shapes) are driver requirements, not probe
hygiene. What P8 does **not** settle is volumes across restore (S-2), which is now what
I-6 waits on.

**P11 — no gate, but two things must be carried into DESIGN as requirements.**
Nothing here promotes or blocks a mechanism: the transport is free, so it
supplies no tiebreak for #97's seam, and the P10 discriminators (daemon
lifetime, single-writer semantics, hydrate granularity) remain the whole
decision. Two findings are load-bearing regardless of which seam wins:
**(1)** the appliance image should set
`/sys/kernel/mm/transparent_hugepage/shmem_enabled=advise` — it is worth ~2×
durable streaming throughput on *every* `shared=on` path, virtiofs volumes
included, and is currently left at the distro default `never`; **(2)** any
`vhost-user-blk` backend MUST honour flushes (`cache.direct=on` for
`qemu-storage-daemon`), because the default export acknowledges the guest's
`fsync` before the data is on the device. (1) is a host-image change; (2) is a
correctness requirement on `overdrive-fs` (#97) itself.

**P12 — PROMOTE, with one requirement the Running gate cannot be built without.**
vsock and checkpointing compose: the device works after a restore and a fresh
connection lands on the *first* post-restore tick, so the persistent-microVM
lifecycle is not blocked. The established connection is destroyed, which is
survivable — but **the reset is one tick late, and in 13 of 16 runs the guest got
a successful `send()` the host never received.** So: **workload readiness must be
acknowledged by the host or observed host-side; a local write returning success
is not evidence of delivery.** That is a design constraint on the gate, not probe
hygiene, and it is the one finding here that changes what gets built. Three
smaller requirements ride along: handle `EPIPE` on write *and* `EOF` on read;
treat **`ECONNRESET`**, not `ECONNREFUSED`, as "nothing is listening"; and on
every restore `mkdir -p` the vsock socket directory while `unlink`-ing the socket
file — contradictory requirements on one path, both fatal, though unlike P8's
`<api-socket>.lock` a mistake is recoverable on the same VMM. What P12 does
**not** cover is the shape a real workload actually has: `--vsock` together with
volumes has never been run as one VM.

**P13 — PROMOTE `OnDemand` as the restore mode, and carry two host requirements
plus one deleted assumption.** Restore latency stops scaling with guest RAM
(12–17 ms at both 2 and 4 GiB against `copy`'s 0.65–0.88 s and 1.65–1.74 s, all
ranges disjoint), which is exactly the property a warm pool of agent sandboxes
needs, and it costs nothing to adopt — same API call, one extra field.
Requirements: **(1)** the appliance image must set
`vm.unprivileged_userfaultfd=1`, because under P5's uid-dropped shape
`OnDemand` fails closed with `Failed to create userfaultfd / EPERM` — this is a
second host sysctl beside P11's `shmem_enabled=advise`, and it carries a real
security trade-off that should be decided rather than defaulted; **(2)** the
driver must send the **PascalCase** `"OnDemand"` and must not trust the 204,
because CH rejects the CLI's `ondemand` spelling loudly but ignores an unknown
*field name* silently — a misspelled key degrades to `Copy` with no signal.
**The deleted assumption is the important half:** `OnDemand` is *asynchronous
eager restore*, not lazy paging — a `uffd-handler` thread backfills all of guest
RAM at ~900 MiB/s whether or not the guest touches a byte, proved by a WALK=0
control, and `Pss ≈ Rss` with `Private_Dirty` = full guest RAM across 4
simultaneous restores in **both** modes. **A warm pool of restored VMs costs
N × guest RAM regardless of restore mode.** Any density argument must come from
VMs that are not restored, or from a mechanism not present here.

**P3 — belongs on CI, not here.** See § Not run.

Per the slice's own rule — *a failing probe never silently weakens a claim* — nothing above
was quietly dropped: P5's three corrections are handed to `[D7]`/US-VM-7; `[D8g]`, which
this document refused to assert for eight days, is asserted now **because it was measured**
and not because the deadline arrived; and the one thing env B cannot reach — aarch64
`shared=on` — is carried forward in § Still open rather than rounded up to "P6 works".

## Artifacts

**Environment B is reproducible from the repo:** `infra/metal/` (`38870e9e`, plus
`f63b2fb9`) provisions a Scaleway Elastic Metal box end to end — cloud-init, partitioning
guidance, media checks, the unprivileged VMM identity, XFS/reflink, and the toolchain.
Its README carries the operational traps found while building it.

`spike-scratch/increment-k/` (gitignored, never committed) — **P13,
`memory_restore_mode=ondemand`**: `probe/Cargo.toml`,
`probe/src/bin/{guest_init_ondemand,host_ondemand}.rs`, `build.sh`, `run.sh`
(one trial, or `api-probe` for the JSON-spelling discovery), `bench.sh` (the
interleaved 3-mode bench), `s5.sh` (N simultaneous restores), `uid-drop.sh` (the
P5-composition test). Captured evidence in `evidence/`:
`bench-2048-walk2.txt`, `bench-4096-walk2.txt`, `bench-2048-walk0.txt` (the
WALK=0 laziness control), `api-probe.txt`, `s5-n4-2048.txt`, `uid-drop.txt`,
`uid-drop-sysctl1.txt`.
The guest extends increment-g's snapshot guest (RAM-only boot nonce + tick
counter) with a memory TOUCH phase and a rate-controlled verifying WALK; the
host harness speaks HTTP-over-unix directly rather than via `curl`, because the
headline is a latency and the control is an RSS trajectory sampled on a fixed
schedule. **`crates/` untouched.**

`spike-scratch/increment-j/` (gitignored, never committed) — **P12, S-3 vsock across
snapshot/restore**: `probe/Cargo.toml`,
`probe/src/bin/{guest_init_vsock_snap,vsock_listener}.rs`, `build.sh`, `run.sh`
(four arms: `drop`, `keep`, `stalesock`, `nosockdir`), `repeat.sh` (the
stale-write-window sweep). Captured evidence in `evidence/`:
`run-{drop,keep,stalesock,nosockdir}.txt`,
`console-{before,after}-<arm>.log`, `<arm>-{held,held-after,recon,recon-after,ch-after}.log`,
`repeat-stale-window.txt`. The guest extends increment-g's snapshot init (RAM-only
boot nonce + tick counter) with the P2 vsock mechanics rather than starting fresh,
so "did it restore" and "did vsock survive" are answered by the same transcript.
Payloads carry `n=<tick> nonce=<nonce>` so the **host** log, not the guest's return
value, settles delivery — which is the only reason the stale-write window was
visible. **`crates/` untouched.**

`spike-scratch/increment-i/` (gitignored, never committed) — **P11, what
`vhost-user-blk` costs**: `run.sh` (three block-shaped arms: `plain`,
`plain-shared`, `vublk`; `VUB_CACHE=writeback|direct|noflush` and `DISK_DIRECT=1`
select the caching contract), `bench.sh` (the four-arm interleaved benchmark; the
virtiofs arm invokes increment-e's `run.sh full` verbatim), `thp-probe.sh` (the
`shmem_enabled` recovery sweep, restores the knob on exit), `durability-probe.sh`
(the `noflush` control + device-ceiling check). Captured evidence in
`bench-5trial-full.txt`, `bench.txt`, `thp-probe-full.txt`,
`durability-probe-full.txt`, `matched-direct.txt`,
`mem-{plain,plain-shared,vublk}.txt`, `transcript-*.txt`, `console-*.txt`.
**No probe binary of its own** — it reuses increment-f's kernel, rootfs and
`guest-init-blk` unchanged so the instrument is identical to P7's.
**`crates/` untouched.**

`spike-scratch/increment-f/` (gitignored, never committed) — **P7, the virtio-blk volume
counterfactual**: `probe/Cargo.toml`, `probe/src/bin/{guest_init_blk,host_collector}.rs`,
`build.sh`, `run.sh`, `vs-virtiofs.sh`; captured evidence in `vs-virtiofs.txt`,
`run-blk.txt`, `run-ratelimit.txt`, `mem-blk*.txt`, `transcript-blk*.txt`.
**`crates/` untouched.**

`spike-scratch/increment-e/` (gitignored, never committed) — **P6 on bare-metal x86_64**:
`probe/Cargo.toml`, `probe/src/bin/{guest_init_fs,host_collector}.rs`, `build.sh`,
`run.sh`, `evidence.sh`, `cache-compare.sh`, `rlimit-sweep.sh`; captured evidence in
`evidence.txt`, `cache-compare.txt`, `transcript-<mode>-<cache>.txt`, `mem-<mode>.txt`.
increment-d is left untouched as the env-A record. **`crates/` untouched.**

`spike-scratch/increment-a/` (gitignored, never committed):
`probe/Cargo.toml`, `probe/src/bin/guest_init.rs`, `probe/src/bin/host_listener.rs`,
`build.sh`, `run.sh`, `stability.sh`, `qemu-crosscheck.sh`, `stallpoint.sh`,
`inspect_kernel.py`. **`crates/` untouched** — verified via `git status --porcelain -- crates/`.
