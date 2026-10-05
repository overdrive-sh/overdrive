# Spike findings — Cloud Hypervisor v53.0 kernel vhost-vsock backend (V-1(b))

> Status: PRE-REGISTRATION (written before any native run). Results are appended
> below the plan; nothing in this section is edited after execution.

## Question

Can Cloud Hypervisor v53.0 gain a kernel vhost-vsock device mode (host
`/dev/vhost-vsock`, `VHOST_VSOCK_SET_GUEST_CID`, kernel vhost worker owning the
RX/TX virtqueues so guest vsock traffic terminates in the host kernel AF_VSOCK
stack, not CH's userspace Unix-socket muxer), and does a real booted Linux guest
work over it?

## Substrate decision (recorded before running)

`.claude/rules/testing.md` makes nested/virtualized hosts non-signal for real
microVM boots. The question is about the production VMM booting a real guest, so
CH runs directly on the qualified native x86_64 metal host (no nesting), under the
canonical exclusive metal lease. No experimental kernel or BPF code is loaded. The
only kernel-side change is the **stock** Ubuntu `vhost_vsock` module (and its
stock dependencies `vhost`, `vhost_iotlb`, `vsock`, `vmw_vsock_virtio_transport_common`,
and `vsock_diag` for `ss --vsock`), auto-loaded **into the physical host kernel**
by opening `/dev/vhost-vsock` / AF_VSOCK. They were not loaded before the run
(read-only probe: `lsmod` showed only `kvm_amd`/`kvm`). The harness records
`/proc/modules` before and after and unloads exactly the modules it caused to
load, at the end, so the physical module set is restored.

Guest: stock Ubuntu `7.0.0-29-generic` bzImage (`/srv/vm/overdrive-testing/kernel`,
same build as the host kernel) with a spike initramfs (static busybox, stock
`vsock`/`vmw_vsock_virtio_transport*` modules for that kernel, and the static
Rust probe agent). The guest kernel and modules are unmodified.

Binaries: stock = `/usr/local/bin/cloud-hypervisor` (release v53.0, static-pie);
patched = built on the metal host from `vendors/cloud-hypervisor` branch
`overdrive/vhost-kernel-vsock`. Seccomp is left at the default (ON) for every
launch.

## Pre-registered probes

Litmus protocol (all data probes): the client sends a `LITMUS-REQ` header plus N
deterministic pattern bytes, then **half-closes** (SHUT_WR); the server verifies
every byte, must observe EOF while its write side is open, then replies with a
distinct `LITMUS-RSP` header (carrying the peer address the server's kernel
reports) plus N bytes of a *different* pattern. An echo cannot pass. Seqpacket:
a manifest record then records of sizes 1, 7, 1500, 4096, 65536, 200000; each
`recvmsg` must return exactly one record (no MSG_TRUNC, exact length/content);
responses use different sizes and patterns.

### P1 — baseline control (stock v53.0, `backend` absent ⇒ unix)
- Hypothesis: the harness (guest image, agent, unix-muxer client with
  `CONNECT <port>`) is sound on the stock binary.
- Predicted: guest boots, reports uname 7.0.0-29-generic; host→guest stream
  litmus (4 MiB each way) OK via `CONNECT 5000`; guest→host stream (1 MiB) OK
  to `<socket>_6000`; CH holds a listening AF_UNIX socket; `ss --vsock` shows no
  host AF_VSOCK sockets for this VM.
- Falsification: boot or either litmus fails ⇒ harness broken, stop.

### P2 — boot + identity (patched, `cid=42,backend=vhost-kernel`)
- Hypothesis: the new device activates against `/dev/vhost-vsock` and an
  unmodified guest binds it.
- Predicted: guest `uname -r` = 7.0.0-29-generic; guest
  `IOCTL_VM_SOCKETS_GET_LOCAL_CID` = 42; CH process has a `vhost-<pid>` task (the
  kernel vhost worker, a user-worker thread in CH's thread group); CH's fd table
  contains `/dev/vhost-vsock` and **no** AF_UNIX socket for vsock; `Seccomp: 2`
  on CH threads.
- Falsification: guest CID ≠ 42, no vhost task, a unix socket held for vsock, or
  activation error in the CH log.

### P3 — data paths, both directions (patched, vhost-kernel)
- Hypothesis: STREAM and SEQPACKET work host→guest and guest→host through the
  kernel path.
- Predicted: host→guest stream 4 MiB each way + half-close OK; host→guest
  seqpacket records exact; guest→host stream 4 MiB + half-close OK; guest→host
  seqpacket exact. Host server sees peer `vsock:42:*`; guest server sees peer
  `vsock:2:*`.
- Falsification: any byte/boundary/half-close failure, or seqpacket refused
  (would mean SEQPACKET feature not negotiated).

### P4 — kernel-path proof
- Hypothesis: payload bytes never cross a CH syscall in vhost-kernel mode.
- Method: `strace -f -p <ch>` over read/write/readv/writev/p{read,write}*/
  send*/recv*/splice/sendfile during the same 4 MiB host→guest transfer, for
  BOTH P1 (unix, population control) and P3 (vhost-kernel). Sum returned bytes.
- Predicted: unix ≥ 8 MiB (muxer reads request, writes response); vhost-kernel
  ≪ 64 KiB (eventfd-sized I/O only, ideally 0). `ss --vsock -p` during a held
  connection shows the host socket owned by the probe/python process, peer 42.
- Falsification: vhost-kernel CH syscall bytes ≥ payload size, or no host
  AF_VSOCK socket visible.

### P5 — multiple VMs and duplicate CID
- Hypothesis: distinct CIDs are isolated by the host kernel; a duplicate CID is
  refused at device creation with a typed error.
- Predicted: VMs with CID 101 and 102 run concurrently; host connections reach
  the right guest (`server=cid-101` / `cid-102`); host sees peers 101/102 for
  guest-originated connections; guest 101 → guest 102 connect fails (the vhost
  TX path only accepts packets addressed to the host CID); a third launch with
  CID 101 exits non-zero with `Failed to assign guest CID 101` caused by
  `Address in use (os error 98)` — no panic — and VM 101 keeps working.
- Falsification: cross-delivery, guest-to-guest success, or duplicate launch
  succeeds / panics.

### P6 — source-CID spoofing (V-10)
- Hypothesis: a guest cannot make the host observe a source CID other than its
  assigned one.
- Predicted: guest `bind()` to CID 142, 2, or 1 fails (`EADDRNOTAVAIL`);
  `bind(VMADDR_CID_ANY)`, `bind(42)` and unbound connects succeed and the host
  recorder's `getpeername` reports CID 42 in every case. (Below the socket API,
  `drivers/vhost/vsock.c` drops TX packets whose `src_cid` ≠ the assigned guest
  CID; a crafted-driver attack is not exercised here.)
- Falsification: any host-observed peer CID ≠ 42.

### P7 — lifecycle
- Hypothesis: the CID and vhost resources are released with the device and are
  immediately reusable.
- Predicted: (a) guest poweroff ⇒ CH exits 0 ⇒ immediate relaunch with CID 42
  succeeds; (b) `kill -9` of CH with a host connection held ⇒ the held host
  socket gets an error/EOF, no CH task or vhost task remains, `vhost_vsock`
  refcnt returns to 0, immediate relaunch with CID 42 succeeds; (c) guest reboot
  (twice) ⇒ CH re-creates the VM in-process without `EADDRINUSE`, guest reports
  CID 42 again, data works, CH holds exactly one `/dev/vhost-vsock` fd; final
  `ss --vsock` shows no leftover sockets.
- Falsification: EADDRINUSE on relaunch/reboot, leaked fds/threads, refcnt > 0
  after all CH processes are gone.

### P8 — unix backend regression (patched, `backend` absent)
- Predicted: identical outcome to P1 (including the strace byte contrast).
- Falsification: any divergence from P1.

### P9 (optional) — hotplug
- Predicted: `ch-remote add-vsock cid=43,backend=vhost-kernel` on a running VM
  makes the guest see CID 43 and host↔guest stream works; `remove-device` frees
  the CID (relaunch/re-add with 43 succeeds).
- Not required for the verdict.

---

# Results (appended after execution)

## Verdict: **WORKS** (bounded — see "What is NOT proven")

A patched Cloud Hypervisor v53.0 with a new `--vsock cid=N,backend=vhost-kernel`
mode boots an unmodified Ubuntu 7.0.0-29-generic guest on native x86_64 metal with
seccomp ON. The guest's RX/TX virtqueues are serviced by the host kernel
`vhost_vsock` worker (a `vhost-<pid>` task); host AF_VSOCK sockets addressed to the
guest CID carry STREAM and SEQPACKET traffic in both directions; CH performs **zero**
payload syscalls on that path (vs 8.4 MB of payload syscalls on the unix backend for
the same transfer). Distinct CIDs are isolated, a duplicate CID is refused with a
typed error carrying `EADDRINUSE`, source-CID spoofing through the socket API is
impossible, and the CID is released and immediately reusable after poweroff,
`kill -9`, in-process guest reboot and hot-unplug. The default unix backend is
behaviourally unchanged.

## CH fork commits (branch `overdrive/vhost-kernel-vsock`, base `v53.0` = 9ed824d6d)

| SHA | Summary |
|---|---|
| `9b68dbb5796560190ee4fa7fd020fcca82106655` | virtio-devices: vsock: Add vhost-kernel backed vsock device (`VhostKernelVsock`, `/dev/vhost-vsock`, CID claimed at creation, kernel RX/TX vrings, VMM-owned event queue, typed refusal of IOMMU/snapshot/migration, seccomp thread rules) |
| `41a619d191f6472f79a3f22bcb9576c2a516906c` | vmm: Add `backend=vhost-kernel` to `--vsock` (config/serde/validation, device-manager branch, VMM+vCPU seccomp ioctls, landlock, OpenAPI, docs/vsock.md) |

11 files changed, 863 insertions, 55 deletions. Not pushed.

## Substrate and versions (as executed)

```
Linux em-determined-roentgen 7.0.0-29-generic #29-Ubuntu SMP PREEMPT_DYNAMIC Fri Jul 17 20:52:35 UTC 2026 x86_64 GNU/Linux
none                                   # systemd-detect-virt: physical host, no nesting
model name	: AMD EPYC 8024P 8-Core Processor
cloud-hypervisor v53.0  448af3d4…29ecc  /usr/local/bin/cloud-hypervisor   (stock, static-pie)
guest kernel  b51367c7…d9682  /srv/vm/overdrive-testing/kernel  (bzImage 7.0.0-29-generic)
```
Patched CH built on the host (`cargo build --release --locked`, rustc 1.95.0), sha256
`6627f736…295978`. Guest `uname -r` = `7.0.0-29-generic` (every boot). Runs held
the canonical metal lease (`OVERDRIVE_METAL_LEASE_ACQUIRED token=…`, fail-closed
native preflight passed). Kernel 6.18 (the pinned appliance kernel) was **not** run.

Evidence: `spike-scratch/netns-density-295-ch-vhost-vsock/increment-{a,b,c,d}/evidence/`
(`results.json`, `run.log`, `commands.jsonl`, strace captures, per-VM serial/CH logs,
module lists). increment-a = build + P1; increment-b = full P1–P8 (+P9 harness bug);
increment-c = P7d rapid-relaunch + P9 hotplug + module restore; increment-d = P10
pause/snapshot + x86_64 lint/unit gates.

## Per-probe results (predicted vs actual)

| Probe | Predicted | Actual | Verdict |
|---|---|---|---|
| P1 stock unix baseline | boot, h2g 4 MiB + g2h 1 MiB litmus OK, CH holds unix listener | as predicted (incr-a and incr-b) | PASS |
| P2 boot + identity | guest CID 42, `vhost-<pid>` task, `/dev/vhost-vsock` fd, no vsock unix socket, seccomp 2 | as predicted | PASS |
| P3 data both directions | stream 4 MiB + half-close and seqpacket exact, both directions; peers 42 / 2 | as predicted | PASS |
| P4 kernel-path proof | vhost CH payload syscalls ≪ 64 KiB; unix ≥ 8 MiB; host socket owned by client | vhost **0 payload bytes** (1,026 B total = serial + serial-IRQ eventfd); unix 8,413,773 B | PASS |
| P5 multi-VM + duplicate | 101/102 isolated; guest→guest fails; dup CID exits non-zero with EADDRINUSE chain | as predicted (guest→guest: ETIMEDOUT after 5 s) | PASS |
| P6 spoofing | forged binds EADDRNOTAVAIL; host always sees 42 | as predicted (3 refused, 3 accepted all seen as 42) | PASS |
| P7 lifecycle | poweroff/kill/reboot release CID, immediately reusable, no leaks | as predicted; 20/20 immediate relaunches after kill/poweroff | PASS |
| P8 unix regression (patched) | identical to P1 | identical (8,413,781 B strace contrast) | PASS |
| P9 hotplug (optional) | add → CID 43 works; remove frees it | 2 add/remove rounds, same CID, refcnt back to 0 | PASS |
| P10 pause/snapshot (extra) | snapshot refused with typed error; pause/resume keep data path | as predicted | PASS |

### P1 / P8 — unix baseline (stock and patched default)

```
IDENT local_cid=3 uname=7.0.0-29-generic uptime_s=0.60
RESULT OK stream label=h2g-P1 sent=4194304 received=4194304 half_close=1 local=unix server_saw_peer=vsock:2:1073741825 server=cid-3 elapsed_ms=469
OK stream label=g2h-cid-3 sent=1048576 received=1048576 half_close=1 local=vsock:4294967295:1005462904 server_saw_peer=unix server=cid-host elapsed_ms=11
RESULT SERVED stream label=g2h-cid-3 bytes=1048576 half_close_seen=1 peer=unix local=unix
u_str LISTEN 0 4096 …/increment-a/P1.vsock 57922220 * 0 users:(("cloud-hyperviso",pid=940620,fd=61))
```
P8 (patched binary, no `backend=`): same lines (`label=h2g-P8 … elapsed_ms=373`), CH
exit 0 after guest poweroff, guest virtio features identical to stock
(`VERSION_1` + `IN_ORDER`, bits 32/35), same thread set (`vmm, vmm_signal_hand,
vcpu0, kvm-nx-lpage-re, _vsock0`).

### P2 — boot and identity (vhost-kernel, CID 42)

```
GUEST_BOOT uname_r=7.0.0-29-generic uptime=0.29
GUEST_VIRTIO virtio1 device=0x0013 driver=vmw_vsock_virtio_transport features=0100000000000000000000000000110010000…
LOCAL_CID=42                                  # IOCTL_VM_SOCKETS_GET_LOCAL_CID in the guest
IDENT local_cid=42 uname=7.0.0-29-generic uptime_s=0.58
ch_threads = 941489:cloud-hyperviso 941490:vmm 941491:vmm_signal_hand 941495:vhost-941490 941496:vcpu0 941497:kvm-nx-lpage-re 941498:_vsock0
ch_seccomp_modes = ["0","2"]                  # main thread 0 (as stock), every other thread filter mode 2
 941489  941495 vhost-941490                  # ps -eLo: kernel vhost worker inside CH's thread group
61->/dev/vhost-vsock                          # /proc/<ch>/fd
<vmm> INFO:virtio-devices/src/vsock/vhost_kernel.rs:216 -- Created vhost-kernel vsock _vsock0 with guest CID 42
<vmm> INFO:event_monitor/src/lib.rs:113 -- Event: source = virtio-device event = activated id = _vsock0
/sys/module/vhost_vsock/refcnt = 1 while the VM runs
```
Negotiated guest features: SEQPACKET (bit 1), INDIRECT_DESC (28), EVENT_IDX (29),
VERSION_1 (32). CH's only sockets are its pre-existing signal-handler socketpair
(`15<UNIX-STREAM:[57925632->57942017]>`, present in stock too); no listener. The
`_vsock0` thread is the new device's VMM-side worker (event-queue drain + interrupt
fallback); it made **no** syscalls during any traced transfer (per-tid strace counts:
only tids 941491 signal-handler 1 line and 941496 vcpu0 228/866 lines).

### P3 — data paths (vhost-kernel)

```
RESULT OK stream label=h2g-stream sent=4194304 received=4194304 half_close=1 local=vsock:4294967295:1365185862 server_saw_peer=vsock:2:1365185862 server=cid-42 elapsed_ms=50
RESULT OK seqpacket label=h2g-seq records=6 sizes=1,7,1500,4096,65536,200000 boundaries_exact=1 … server_saw_peer=vsock:2:1365185863 server=cid-42 elapsed_ms=4
OK stream label=g2h-cid-42 sent=4194304 received=4194304 half_close=1 … server_saw_peer=vsock:42:1092204129 server=cid-2 elapsed_ms=19
OK seqpacket label=g2h-cid-42 records=6 sizes=1,7,1500,4096,65536,200000 boundaries_exact=1 … server_saw_peer=vsock:42:1092204130 server=cid-2 elapsed_ms=1
RESULT SERVED stream label=g2h-cid-42 bytes=4194304 half_close_seen=1 peer=vsock:42:1092204129 local=vsock:2:6000
RESULT SERVED seqpacket label=g2h-cid-42 records=6 peer=vsock:42:1092204130 local=vsock:2:6001
```

### P4 — kernel-path proof (strace of the whole CH process, same 4 MiB h2g transfer)

| backend | traced syscalls | bytes returned | where |
|---|---|---|---|
| unix (stock, P1) | 4,483 | **8,413,773** | `write fd=88<UNIX-STREAM>` 4,194,391 + `read fd=88<UNIX-STREAM>` 4,194,340 + eventfds |
| unix (patched, P8) | 4,484 | **8,413,781** | same shape |
| vhost-kernel h2g (P3) | 228 | **1,026** | `write fd=18<serial.log>` 114 + `write fd=34<eventfd>` 912 (114 × 8 = serial IRQ) |
| vhost-kernel g2h (P3) | 866 | **3,897** | serial 433 + serial-IRQ eventfd 3,464 (433 × 8) |

`ss --vsock -a -n -p` during a held host→guest connection (python AF_VSOCK client):
```
v_str ESTAB  0 0  *:1365185865  42:5000  users:(("python3",pid=941391,fd=6))
```
The host endpoint is a host-kernel AF_VSOCK socket owned by the client process; CH
holds no vsock socket. (The population control makes the strace method credible:
the same filter sees every payload byte on the unix backend.)

### P5 — two VMs, isolation, duplicate CID

```
RESULT OK stream label=multi-a … server=cid-101      RESULT OK stream label=multi-b … server=cid-102
RESULT SERVED stream label=g2h-cid-101 … peer=vsock:101:1960468448 local=vsock:2:6100
RESULT SERVED stream label=g2h-cid-102 … peer=vsock:102:1336510947 local=vsock:2:6100
PEER cid=102 connect=failed err=[connect: Connection timed out (os error 110)] elapsed_ms=5006   # guest 101 -> guest 102
 941613  941622 vhost-941615        941614  941621 vhost-941616     # one kernel worker per VM; refcnt 2
duplicate launch (cid=101) exit code 1:
Error: Cloud Hypervisor exited with the following chain of errors:
  0: Error booting VM
  1: The VM could not boot
  2: Error from device manager
  3: Cannot create vhost-kernel virtio-vsock device
  4: Failed to assign guest CID 101 to vhost-vsock
  5: failure in vhost ioctl: Address already in use (os error 98)
IDENT local_cid=101 …   RESULT OK stream label=after-dup … server=cid-101   # VM 101 unaffected
```
Guest→guest is dropped by `drivers/vhost/vsock.c` (v7.0, below), not by CH.

### P6 — source-CID spoofing (V-10)

```
SPOOF case=forged-cid-own+100 connect=refused err=[bind: Cannot assign requested address (os error 99)]
SPOOF case=forged-cid-host-2 connect=refused err=[bind: Cannot assign requested address (os error 99)]
SPOOF case=forged-cid-1-local connect=refused err=[bind: Cannot assign requested address (os error 99)]
SPOOF case=bind-any connect=ok local=vsock:4294967295:1092204131 host_says=[SEEN peer=vsock:42:1092204131]
SPOOF case=bind-own connect=ok local=vsock:42:1092204132 host_says=[SEEN peer=vsock:42:1092204132]
SPOOF case=unbound connect=ok local=vsock:4294967295:1092204133 host_says=[SEEN peer=vsock:42:1092204133]
RECORDED peer=vsock:42:1092204131 claim=[case=bind-any claimed_cid=4294967295 …]
RECORDED peer=vsock:42:1092204132 claim=[case=bind-own claimed_cid=42 …]
RECORDED peer=vsock:42:1092204133 claim=[case=unbound claimed_cid=unbound …]
```
Below the socket API, upstream v7.0 `drivers/vhost/vsock.c:551-557` (sha256
`2f9e8abb…efff51`, `linux-reference/drivers-vhost-vsock.c`) only delivers guest TX
packets with `src_cid == vsock->guest_cid && dst_cid == local CID`; everything else is
freed. A guest-root crafted-driver attack was not exercised; that guarantee is cited
from source, not measured. Ubuntu's 7.0.0-29 may carry distro patches over v7.0.

### P7 — lifecycle

```
P7a poweroff:   CH exit 0; refcnt 0; immediate relaunch cid=42 -> IDENT local_cid=42 uptime_s=0.58
P7b kill -9:    CH exit -9; /proc/<pid> gone; held host socket -> recv error ConnectionResetError(104, 'Connection reset by peer') after 0.000s
                immediate relaunch cid=42 -> IDENT local_cid=42; RESULT OK stream label=after-kill sent=1048576 …
P7c reboot x2 (same CH pid): IDENT … uptime_s=13.45 -> 1.75 -> 1.77 ; /dev/vhost-vsock fds 1 -> 1 -> 1 ; total fds 85 -> 85 -> 85
                vhost worker tid 941495 -> 941529 -> 941545 (old worker gone, new one per re-created VM)
                post-reboot stream + seqpacket litmus OK; no EADDRINUSE in ch.log
P7d (incr-c) 20 cycles alternating kill -9 / poweroff, relaunch the same CID with no wait:
                relaunch_failures = 0; refcnt_at_prev_exit = 0 and vhost tasks = 0 in all 20 cycles
final: ss --vsock empty; no cloud-hypervisor process
```
Note: in increment-b two `vhost_state` samples read `refcnt=1` with no CH process
alive (after P7b kill and at the P7 end). Those coincide with a host AF_VSOCK socket
to the dead guest still being closed; an AF_VSOCK socket bound to the vhost transport
holds a reference on the `vhost_vsock` module, so refcnt is not a pure device-fd
count. This explanation is inferred, not measured. The load-bearing evidence is
the 20/20 immediate relaunches (P7d), where no host socket was open across the exit
and refcnt read 0.

### P9 — hotplug (optional)

```
add-vsock cid=43,backend=vhost-kernel -> {"id":"_vsock0","bdf":"0000:00:02.0"} ; IDENT local_cid=43 ; RESULT OK stream label=hotplug-1 … server=cid-43
remove-device _vsock0 -> refcnt 0, CH holds no /dev/vhost-vsock fd
add-vsock again (same CID 43) -> {"id":"_vsock1",…} ; IDENT local_cid=43 ; RESULT OK stream label=hotplug-2 … server=cid-43
```

### P10 — pause / snapshot refusal (extra)

```
pause -> host connect while paused: TimeoutError(110)       (backend stopped; connect times out, not queued)
snapshot -> Error: ch-remote exited with the following chain of errors:
  … 5: Failed to snapshot migratable component
    6: vsock device _vsock0 uses the vhost-kernel backend, which does not support snapshot
snapshot dir contents: []
resume -> IDENT local_cid=44 ; RESULT OK stream label=after-resume … ; RESULT OK seqpacket … boundaries_exact=1 ; CH alive, exit 0 on poweroff
```

### Gates on the CH fork

- Lima aarch64 (compile surface): `cargo check --workspace --all-targets` clean;
  `cargo clippy --locked --all --all-targets --tests --examples -- -D warnings` clean;
  `nextest --workspace --lib` for virtio-devices + vmm: 188/188 pass.
- Metal x86_64 (incr-d): clippy `-D warnings` default features rc=0, `--no-default-features --features kvm` rc=0;
  `cargo test --lib -- vsock` 59 + 3 pass (incl. both new `vhost_kernel` unit tests),
  `-- config::` 30 pass (incl. `test_config_validation` with the new backend cases, `test_vsock_config_serde`).
- `cargo +nightly fmt --all -- --check` clean.

### Physical host restoration

The run auto-loaded the stock vsock stack into the physical kernel (`vhost`,
`vhost_iotlb`, `vhost_vsock`, `vsock`, `vmw_vsock_virtio_transport_common`,
`vsock_diag`, and on the first run also `vsock_loopback`, `vmw_vmci`,
`vmw_vsock_vmci_transport`, which AF_VSOCK socket creation pulls in). increment-b's
`modprobe -r` cleanup left five of them loaded. increment-c unloaded them at
start and end, and increment-d confirms the physical host is back to the pristine
set: `vsock_stack_vs_pristine_increment_a = {"now": [], "pristine": []}`, with no
cloud-hypervisor processes left. No experimental kernel or BPF code was loaded.

## What is NOT proven

- **No Aya / SockHash / SK_SKB program was loaded.** Running the selected transport's
  BPF forwarding against these CH-hosted sockets is the next step. This spike only
  establishes that CH can supply the kernel vhost-vsock endpoint that design assumes.
- **Not the production Overdrive composition.** CH was launched by a spike harness,
  not by `overdrive serve` / the VM driver. Guest is an initramfs agent, not the
  production guest image or `overdrive-init`.
- **Kernel 7.0.0-29-generic only** (host and guest). The pinned 6.18 appliance kernel
  was not run.
- One vCPU and 512 MiB per guest; at most 2 concurrent VMs. No density, throughput or
  latency claims. The elapsed_ms figures are incidental.
- The kernel's guest-TX `src_cid` check is cited from source; no crafted-driver attack.
- Live migration refusal is implemented (`start_dirty_log` / `start_migration` return
  errors) but was not exercised. Snapshot refusal was exercised (P10).
- The interrupt-fallback path (no irqfd notifier → worker forwards call eventfd) was
  never taken: the Linux guest uses MSI-X, so it is untested at runtime.
- Memory hotplug (`add_memory_region` → `VHOST_SET_MEM_TABLE` refresh) was not exercised.
- No CH upstream integration test was added (`cloud-hypervisor/tests/integration.rs`).

## Gate recommendation: **PROMOTE** to the production VMM path (as a vendored CH fork)

The kernel vhost-vsock endpoint the transport design depends on is available from
the production VMM with a small, self-contained change (~860 lines). The default unix
backend is unchanged. Promote in two steps: (1) carry the fork branch in the
Overdrive VMM build and run the next spike (Aya SockHash forwarding over CH-hosted
guests) against it; (2) pursue upstreaming in parallel.

### Possible upstreaming blockers / reviewer asks

1. **API shape.** `VsockConfig.socket` changed from `PathBuf` to `Option<PathBuf>`.
   This is serde-compatible (JSON for existing unix configs is unchanged; P8 and
   `test_vsock_config_serde`), but the Rust struct changes for in-tree constructors
   (config tests updated; the out-of-workspace fuzz targets do not construct
   `VsockConfig` and were not built). Upstream may prefer a different selector name
   or a tagged-enum config.
2. **Integration test.** Upstream will want a guest-image integration test
   (`test_vsock_vhost_kernel`), which needs `vhost_vsock` on their CI runners.
3. **Migration/snapshot semantics.** This change refuses them. QEMU instead migrates
   vhost-vsock by sending `VIRTIO_VSOCK_EVENT_TRANSPORT_RESET` on the event queue
   after restore. Upstream may want that implemented instead of a refusal.
4. **Interrupt fallback thread.** One extra thread per device, idle when MSI-X irqfds
   exist. A reviewer may prefer requiring irqfd or folding the drain elsewhere. Under
   density this is one parked thread per VM; the kernel vhost worker adds another
   task per VM.
5. **`VIRTIO_F_IN_ORDER` / `ACCESS_PLATFORM` not offered.** The kernel does not support
   them for vsock. IOMMU and confidential VMs (forced ACCESS_PLATFORM) are refused at
   creation.
6. **Seccomp.** Adds `VHOST_SET_MEM_TABLE`, `VHOST_VSOCK_SET_GUEST_CID` and
   `VHOST_VSOCK_SET_RUNNING` to the VMM thread, and `VHOST_VSOCK_SET_RUNNING` to vCPU
   threads (device reset runs on a vCPU thread).
7. **Copyright header** on the new file names "Overdrive contributors". Adjust to
   whatever attribution the CH project expects.

### Open items needing a user decision

- Whether the vendored CH fork (submodule `vendors/cloud-hypervisor`) becomes the
  production VMM build, and how it is pinned/published. Nothing is pushed.
- The production consequence of P10: while a VM is paused, host connects to it time
  out (`ETIMEDOUT`) rather than queue. This only matters if Overdrive pauses VMs.
- Recording the PROMOTE/PIVOT/DISCARD decision in `spike/wave-decisions.md`. This
  spike did not edit that file.
