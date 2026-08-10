# SPIKE findings — increments a, c, d (P1, P2, P4, P5, P6)

Feature: `microvm-driver-cloud-hypervisor` (GH [#42](https://github.com/overdrive-sh/overdrive/issues/42)).
Slice: `slices/slice-00-spike-ch-boot-and-vsock.md`. Governed by `.claude/rules/spike.md`.
Dates: 2026-08-02 (nested aarch64), **2026-08-10 (bare-metal x86_64)**.

| Probe | Verdict |
|---|---|
| **P1** kernel boots from ext4 `virtio-blk` | **WORKS** — confirmed on both arches |
| **P2** vsock beacon + exit status, incl. netns | **WORKS** — confirmed on both arches |
| **P4** per-launch rootfs copy cost | **WORKS — reflink is ~260× faster and free in space** |
| **P5** `[D7]` confinement flags compose | **WORKS**, with three corrections `[D7]`/US-VM-7 must absorb |
| **P6** virtiofsd + `--memory shared=on` | **STILL UNPROVEN** — was unexercisable on the nested host; not yet re-run on metal |
| **P3** the pinned 6.18 kernel | **NOT RUN** |

Raw evidence: `spike-scratch/increment-{a,c,d}/` (gitignored). `crates/` untouched
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
cloud-hypervisor  : v46.0.0                   virtiofsd: 1.13.2 (/usr/libexec — NOT on PATH)
/dev/kvm          : crw-rw-rw-  (Lima 0666 udev rule)
host              : Apple M4 Max, macOS 26.1; CH runs NESTED inside Lima
```

**B — bare metal x86_64 (2026-08-10).** Scaleway Elastic Metal, provisioned by
`infra/metal/` (committed as `38870e9e`).

```
uname -r          : 7.0.0-15-generic          uname -m : x86_64
cpu               : AMD EPYC 8024P (svm / AMD-V)   — note: AMD, not Intel
systemd-detect-virt: none                     <-- NOT nested. The whole point.
cloud-hypervisor  : v46.0.0 (--landlock present)
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
| Constraint 7 (version floor) | Installed CH is **v46.0.0**, *below* the reference implementation's unexplained "≥48.0" floor — and it works, and it **has `--landlock` and `--landlock-rules`**. So the ≥48.0 figure has no evidence behind it. Do not inherit it. The real floor should be named against a capability, and P5 will supply the rest. |

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

### Verdict: **UNPROVEN — explicitly NOT refuted**

> **Status 2026-08-10:** everything below was measured on env A (nested aarch64),
> where `--memory shared=on` cannot boot at all. **Env B removes that blocker**, so
> P6 is now runnable and simply has not been run there yet. The findings below —
> the `RLIMIT_FSIZE` × memfd interaction, `[D8e]`, `[D8d]` — are mechanism-level and
> stand; the unanswered half is unanswered only because the probe has not been
> rebuilt for x86_64.

Raw evidence: `spike-scratch/increment-d/evidence.txt` (371 lines).

**`shared=on` guest memory does not survive nested virtualisation on Apple Silicon**, so
the round-trip half of P6 cannot be exercised here by any VMM. The falsification chain is
complete rather than assumed:

| Trial | Reached `/init` |
|---|---|
| CH, no `shared=on` | **11/12** |
| CH, `shared=on` (root, unconfined) | **0/4** |
| CH, `shared=on` (confined) | 0/4 |
| CH, `shared=on,prefault=on` / `thp=off` / `128M` | 0/4 each |
| **QEMU 10.2.1, private memory** | **3/4** |
| **QEMU 10.2.1, `memory-backend-memfd,share=on`** | **0/4** |

**A different VMM failing identically** ⇒ the mechanism is `MAP_SHARED` guest memory under
nested KVM, not Cloud Hypervisor and not confinement. Same class as increment-a's stall.
All `shared=on` runs freeze at the same point, right after the `virtio_blk` probe.

### What IS answered — arch-independent, and it matters

**`shared=on` × `RLIMIT_FSIZE` — a genuine P5×P6 composition finding, and the reason
running these together was worth it.** Every `shared=on` attempt under increment-c's
rlimit died with CH exit **153** (`128+SIGXFSZ`):

```
noshare-fsize256      size=512M              FSIZE=256 MiB  rc=124  (no rlimit failure)
sharedon-fsize256     size=512M,shared=on    FSIZE=256 MiB  rc=153  *** SIGXFSZ ***
sharedon-fsize768     size=512M,shared=on    FSIZE=768 MiB  rc=124  (no rlimit failure)
sharedon-256M-fsize192  size=256M,shared=on  FSIZE=192 MiB  rc=153  *** SIGXFSZ ***
sharedon-256M-fsize384  size=256M,shared=on  FSIZE=384 MiB  rc=124  (no rlimit failure)
```

`shared=on` backs guest RAM with a **memfd**, and a memfd is a *file* for `RLIMIT_FSIZE`:

```
/proc/3452499/maps:  f1c1c811c000-f1c1e811c000 rw-s 00000000 00:01 7218  /memfd:ch_ram (deleted)
```

The threshold tracks **`--memory size`**, not the disk. **So `RLIMIT_FSIZE` must be
`max(rootfs image, guest RAM)` whenever `shared=on` is in play.** Sizing it off the
rootfs — which is what increment-c did and what a reasonable implementer would do — makes
every volume-carrying VM die with an opaque signal.

Also confirmed:

- **`[D8e]` HOLDS.** The volume *source* directories were granted in **no** trial and CH
  created the fs device anyway. **Volumes do not widen `[D7]`'s hypervisor confinement;
  US-VM-8's non-widening AC stands.**
- **`--fs socket=` IS auto-derived** by CH's implicit ruleset — unlike `--vsock`. Verified
  with the socket directory as a *sibling* of the granted path (a first attempt nested it,
  which was vacuous, since Landlock rules are path-beneath).
- **`--sandbox=namespace` is the default and genuinely in effect — stated precisely:**
  `mnt` and `net` namespace inodes differ from the shell's; `pid` and `user` do **not**. It
  is a mount+net sandbox, not a full one. **No silent downgrade to `chroot`** — the
  reference implementation's failure is not reproduced.
- `--socket-group=<vmm user>` yields `srwxrwx--- root:spikevmm` — the clean way for a
  uid-dropped CH to reach the daemon. `virtiofsd` **1.13.2**, `/usr/libexec/virtiofsd`,
  not on `PATH`. `--readonly` exists as a host-side flag.
- **`shared=on` memory accounting is a reclassification, not obvious inflation**
  (early-boot sample, not the controlled post-beacon one): `shared=on` → VmRSS 86432 kB
  (RssShmem 83224 / RssAnon 316); private → VmRSS 93320 kB (RssAnon 90428 / RssShmem 0).

### What is NOT answered — do not let these be assumed later

- Guest↔host round-trip through the share.
- **Host-side `--readonly` enforcement against an uncooperative guest (`[D8g]`).** Never
  tested against a guest. **`[D8a]`'s security framing must not be asserted until it is** —
  a guest-side `-o ro` is guest-cooperative and void.
- Failed-mount errno from inside the guest (the evidence `overdrive-init`'s refuse-to-exec
  path is built against).
- virtiofs throughput/latency under `--cache=never`. Host-side baseline only: 32 MiB
  write+fsync in 0.0717 s ≈ 446 MiB/s; 200 small files in 0.0203 s.
- Whether `shared=on` changes the guest's observed `MemTotal` (`[D8b]` × US-VM-5 / GH #92).

**A vacuous result was caught and suppressed:** `run.sh` initially printed *"the read-only
file is unchanged → refused host-side"* after a run in which the guest never booted. That
reads as evidence when nothing was attempted. The script now skips the block entirely
unless the guest completed.

## Still open

- **P6's round-trip half — the reason the metal box exists, and NOT yet re-run there.**
  It was unexercisable on the nested host (`shared=on` never boots), which env B removes.
  A new increment-d probe is needed for x86_64. Until it runs, `[D8g]`'s host-side
  read-only enforcement stays **unverified** and must not be asserted.
- **P3 (pinned 6.18 kernel).** Not run on either environment. Both x86_64 and aarch64
  ship, so this is two questions, not one. Env B has `7.0.0-15`; the LVH path
  (`cargo xtask integration-test vm --kernels …`) is the route on x86_64, and aarch64
  needs non-nested Arm hardware.
- **P5 confirmation on env B.** P5's verdict is currently pinned to the nested aarch64
  host. Its mechanisms are arch-independent, but the caveat is not retired until it is
  re-run.

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

**P6 — STILL DEFERRED, but the blocker is gone.** `[D8]`'s mechanism was never refuted,
only unexercisable: `shared=on` cannot boot under nested virt (proven by cross-VMM diff —
QEMU private memory 3/4, QEMU memfd-shared 0/4). **Env B removes that constraint**, so P6
is now runnable and simply has not been run. It needs a new increment-d probe built for
x86_64. This is the highest-value remaining work and the reason the metal box exists. `[D8d]` and `[D8e]` are confirmed as far as this host
allows. **`[D8g]`'s host-side read-only security framing remains unverified and must not be
asserted until it is.**

**P3 — belongs on CI, not here.** See § Not run.

Per the slice's own rule — *a failing probe never silently weakens a claim* — nothing above
was quietly dropped: P5's three corrections are handed to `[D7]`/US-VM-7, and P6's
unverified `[D8g]` claim is flagged for restatement rather than left standing.

## Artifacts

**Environment B is reproducible from the repo:** `infra/metal/` (`38870e9e`, plus
`f63b2fb9`) provisions a Scaleway Elastic Metal box end to end — cloud-init, partitioning
guidance, media checks, the unprivileged VMM identity, XFS/reflink, and the toolchain.
Its README carries the operational traps found while building it.

`spike-scratch/increment-a/` (gitignored, never committed):
`probe/Cargo.toml`, `probe/src/bin/guest_init.rs`, `probe/src/bin/host_listener.rs`,
`build.sh`, `run.sh`, `stability.sh`, `qemu-crosscheck.sh`, `stallpoint.sh`,
`inspect_kernel.py`. **`crates/` untouched** — verified via `git status --porcelain -- crates/`.
