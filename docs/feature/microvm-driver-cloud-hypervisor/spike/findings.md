# SPIKE findings — increments a, c, d, e, f (P1, P2, P4, P5, P6, P7)

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
| **P3** the pinned 6.18 kernel | **NOT RUN** |

Raw evidence: `spike-scratch/increment-{a,c,d,e,f}/` (gitignored). `crates/` untouched
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

## Still open

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
