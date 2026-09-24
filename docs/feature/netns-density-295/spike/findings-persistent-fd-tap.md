# Spike Findings — `netns-density-295` persistent-TAP fd handoff (increment-y)

## Verdict: WORKS

On native, non-virtualized x86_64 metal, the fd-handoff mechanism proven by the
prior probe (`netns-density-295-fd-tap`, edge case 5) also holds for the
production-shaped **persistent named TAP** lifecycle, end to end:

1. A network owner created the TAP ioctl-for-ioctl like
   `overdrive_netlink::create_persistent_tap` and closed its fd. The TAP
   persisted, down, owner 4200, with zero queue holders. The owner then
   enslaved it to a bridge.
2. A separate launcher open plus `TUNSETIFF` attached a queue fd to the
   existing TAP and passed it to Cloud Hypervisor v53.0 via `--net fd=[50]`,
   through the production `prlimit`/`setpriv`/seccomp/landlock/cgroup chain.
3. The real guest sent `READY pid=1 port=1234` (1.115 s) while the TAP stayed
   admin-down. There were zero frames on the exact-ifindex capture, all six
   counters (including `rx_dropped`) were zero, and CH left TAP state untouched.
4. The launcher closed its copy and CH alone retained the queue.
5. The owner raised the TAP only after the guard read back. Bidirectional ARP
   and ICMP followed, with every captured timestamp strictly after the barrier.
6. CH exit released the queue. The TAP persisted, and no process on the host
   held a queue fd for it.
7. Set-down plus `RTM_DELLINK` removed it, leaving an exact empty complement.

The failure arm (missing kernel) left the launcher fd and the down persistent
TAP untouched, and a subsequent launcher attach succeeded.

The verdict carries two conditions a DESIGN must pin. Both are proven on metal
and explained by primary source:

- **The launcher's `TUNSETIFF` must request `IFF_VNET_HDR`.** The production
  creator does not set it. CH v53 cannot add it to an already-attached fd,
  because its `TUNSETIFF` gets `EEXIST`, which it ignores. CH also does not
  detect its absence. Without the flag, CH boots to READY and exits 0, but
  every guest frame reaches the TAP with a 12-byte zero virtio-net header in
  front, so L2 is silently corrupt.
- **The network owner must lower the TAP after VMM exit and before any
  relaunch attach.** CH exit leaves the persistent TAP admin-`UP`
  (NO-CARRIER). The kernel turns carrier on at every `TUNSETIFF` attach, so
  attaching to that still-`UP` TAP made it `LOWER_UP` immediately. Host frames
  queued into the new fd before any VMM ran. A down-first reattach stays down
  with zero frames.

This is throwaway evidence. It proposes no production API. The current
`VmNetworkAttachment` / `VmConfig` / `CloudHypervisorVmm` still expose only
named-TAP attachment, and adopting fd handoff remains a DESIGN decision.

## Question tested

Does the prior probe's fd-handoff mechanism hold when the TAP is the production
persistent named TAP, created and closed by a network owner? Specifically, can
a separate VMM-launcher queue fd:

- attach to the existing TAP;
- be passed to CH v53 through the production confinement boundary;
- reach real guest READY while the TAP is down;
- survive the launcher closing its copy;
- support owner-side activation after the guard is live;
- be released by CH exit, leaving the persistent TAP explicitly deletable with
  no residue?

## Execution substrate

- Host: `Linux 7.0.0-29-generic x86_64 GNU/Linux`; `systemd_detect_virt=none`.
- Hypervisor: `/usr/local/bin/cloud-hypervisor`, `cloud-hypervisor v53.0`.
- Tools: `ethtool version 6.19`, `Python 3.14.4`.
- Guest artifacts: kernel `/srv/vm/overdrive-testing/kernel`, rootfs master
  `/srv/vm/overdrive-testing/rootfs.ext4`.
- Every run went through `cargo xtask metal run --`, which covered the
  canonical shared lease, workspace sync, fail-closed native x86_64/KVM
  preflight, and root execution. Before the first run, a read-only recon
  confirmed zero pre-existing `pf*`/`fd*` links, nft tables, run dirs,
  cgroups, staging dirs, CH pids, and tun-fd holders. The probe also refuses
  any pre-existing exact resource or queue holder.

| Run | Command | Evidence (complete stdout/stderr) | Result |
|---|---|---|---|
| 1 | `cargo xtask metal run -- python3 spike-scratch/increment-y-netns-density-295-persistent-fd-tap/spike.py` | `spike-scratch/increment-y-netns-density-295-persistent-fd-tap/evidence/run-main-20260923T163139Z.log` | `VERDICT=WORKS`, `cleanup_complete=True`, `exit=0`, body 2.557 s |
| 2 | same | `.../evidence/run-main-restart-obs-20260923T163347Z.log` | `VERDICT=WORKS` (reproduction plus restart observation), `cleanup_complete=True`, `exit=0` |
| 3 | `... spike.py --variant no-vnet-hdr` | `.../evidence/run-variant-no-vnet-hdr-20260923T163417Z.log` | `VARIANT_COMPLETE=no-vnet-hdr`, `cleanup_complete=True`, `exit=0` |

Run 1 used the probe before the `RESTART_OBS` arm was inserted; that insertion
is the only difference. Runs 2 and 3 used the final file (SHA below).

## Exact launch CLI (run 1)

TAP `pftap4ef2`, ifindex 58837, launcher fd 50, guest MAC `02:00:00:95:4e:f2`:

    prlimit --fsize=268435456 --nofile=256 -- setpriv --reuid=4200 --regid=4200 --groups=991 --no-new-privs -- /usr/local/bin/cloud-hypervisor --cpus boot=1 --memory size=268435456 --kernel /run/overdrive/vm/pf-tap-4ef2/kernel --cmdline 'console=ttyS0 panic=1 root=/dev/vda rw overdrive.net=10.77.239.2/24,gw=10.77.239.1,dns=10.77.239.1' --disk path=/srv/vm/overdrive-testing/pf-tap-4ef2/rootfs.ext4,image_type=raw --serial file=/run/overdrive/vm/pf-tap-4ef2/console.log --console off --vsock cid=3,socket=/run/overdrive/vm/pf-tap-4ef2/vsock --api-socket /run/overdrive/vm/pf-tap-4ef2/api --seccomp true --landlock --net 'fd=[50],mac=02:00:00:95:4e:f2,offload_tso=off,offload_ufo=off,offload_csum=off' --landlock-rules path=/sys/class/net/pftap4ef2,access=r --landlock-rules path=/run/overdrive/vm/pf-tap-4ef2,access=rw

The failure arm used the identical wrapper, fd, `--net` argument, disk, and
confinement, but `--kernel /run/overdrive/vm/pf-tap-4ef2/deliberately-missing-kernel`.

**Persistent-TAP creator (network owner), mirrored from
`crates/overdrive-netlink/src/client.rs::create_persistent_tap`:**
- open `/dev/net/tun` (blocking, `O_CLOEXEC`);
- `TUNSETIFF(IFF_TAP|IFF_NO_PI)`;
- `TUNSETOFFLOAD(0)`;
- `TUNSETOWNER(4200)`;
- `TUNSETPERSIST(1)`;
- close.

`ip link set master` stands in for `set_link_master`.

**Launcher:**
- open `/dev/net/tun` with `O_NONBLOCK`;
- `F_DUPFD_CLOEXEC` to fd ≥ 50;
- `TUNSETIFF(IFF_TAP|IFF_NO_PI|IFF_VNET_HDR)` (`0x5002`; `0x1002` in the variant);
- clear `FD_CLOEXEC`;
- `Popen(close_fds=True, pass_fds=(50,))`.

**Activation mirrors `set_tap_up`:** `ethtool -K <tap> tx-checksum-ip-generic off`,
then `ip link set <tap> up`.

**Teardown mirrors the production allocation teardown:** `ip link set <tap> down`,
read-back, then `ip link delete dev <tap>` (`RTM_DELLINK`, like
`Client::del_link`), then read-back.

**Deviations from the production host path:**
- The probe does not reassert the bridge MAC. It does not touch the question.
- The nft bridge guard is a spike-local stand-in for "interception live". It is
  NOT the production TCX classifier plus D9.
- Host-side IPv6 is left enabled on the TAP, as production leaves it. The prior
  probe had disabled it host-side.

## Per-step results

Every step carried a written hypothesis, prediction, and falsification in the
evidence log, emitted before its observations. All quoted lines are pasted
verbatim from run 1 unless marked run 2 or run 3. `…` marks an elided portion
of a long JSON line.

| Step | Prediction | Key evidence (verbatim) | Result |
|---|---|---|---|
| SQ, single-queue attach semantics (scratch TAP `pfx4ef2`, no VMM) | attach while creator fd open → `EBUSY`; re-`TUNSETIFF` on an attached fd → `EEXIST`; `IFF_VNET_HDR` tracks the last attacher | `launcher_attach_while_creator_fd_open=errno=16(EBUSY) Device or resource busy` · `ch_style_reissue_TUNSETIFF_with_vnet_on_attached_fd=errno=17(EEXIST) File exists sysfs_tun_flags=0x1802` | PASS |
| 1, owner creates persistent TAP, leaves it down | `tun_flags=0x1802`, owner 4200, no `UP`, holders `[]` | `STEP1 RESULT=PASS tap=pftap4ef2 ifindex=58837 tun_flags=0x1802 owner=4200 flags=['BROADCAST', 'MULTICAST'] master=pfbr4ef2 holders=[]` | PASS |
| 2, launcher opens its own queue fd | attach ok; `tun_flags` gains `0x4000` iff requested; TAP down | `STEP2 launcher_fd=50 inode=137 device=7 TUNGETIFF=name=pftap4ef2 flags=0x5802` · `diff_vs_step1={… "tun_flags": ["0x1802", "0x5802"]}` | PASS |
| Failure arm (missing kernel) | CH exits non-zero; fd and TAP unchanged; after close the TAP persists; reattach ok | `failure_exit=1` · `diff_vs_launcher_attach={}` · `subsequent_launcher_attach=ok fd=50 inode=137 TUNGETIFF=name=pftap4ef2 flags=0x5802` | PASS |
| 3, CH reaches READY while TAP down | READY; no `UP`; capture 0; six counters 0; TAP state and offloads unchanged | `ready_line=READY pid=1 port=1234 ready_elapsed_seconds=1.114602` · `diff_vs_pre_launch={} offload_digest_pre=7210dc20a2ecce15 offload_digest_ready=7210dc20a2ecce15` · `pre_activation_capture_packets=0 bytes=0` | PASS |
| 4, launcher closes its copy; CH retains | holders are CH only; iff retained; TAP unchanged | `holders=[[20292, 50, 'cloud-hyperviso'], [20292, 67, …], [20292, 120, …], [20292, 121, …], [20292, 125, …], [20292, 128, …]] tap_exists=true flags=['BROADCAST', 'MULTICAST']` | PASS |
| 5, owner raises after interception live | guard read-back exact; `UP,LOWER_UP`; bidirectional ARP/ICMP; every timestamp > barrier | `post_activation_capture_packets=5 bytes=370 records_at_or_before_barrier=0 guest_originated=2 well_formed_guest=2 to_guest=2 icmp_reply_to_guest=True` | PASS |
| 6, CH exit; persistent TAP remains, unheld | exists, admin `UP` retained, NO-CARRIER, persist bit, owner 4200, holders `[]` | `STEP6 RESULT=PASS exists=true flags=['NO-CARRIER', 'BROADCAST', 'MULTICAST', 'UP'] operstate=DOWN carrier=0 master=pfbr4ef2 tun_flags=0x5802 owner=4200 holders=[] (fdinfo scanned=714)` | PASS |
| 7, explicit teardown, no residue | down; delete rc 0; `ip link show` ENODEV; empty complement | `ip_link_show_after rc=1 stderr='Device "pftap4ef2" does not exist.' sysfs_exists=False` · `final_readback={… all false/[] …}` | PASS |
| Restart observation (run 2; extra arm) | attach onto admin-`UP` TAP → `LOWER_UP` plus queued host frames; down-first → nothing | `admin_up_reattach flags=['BROADCAST', 'MULTICAST', 'UP', 'LOWER_UP'] carrier=1 queued=7; down_first_reattach flags=['BROADCAST', 'MULTICAST'] queued=0 stats_delta={…all 0…}` | confirmed |
| No-vnet-hdr variant (run 3) | CH accepts the fd, READY ok, L2 frames carry a 12-byte header prefix, no ICMP | `post_activation_frame … len=54 src=00:00:00:00:00:00 dst=00:00:00:00:00:00 kind=ethertype-0xffff head=000000000000000000000000ffffffffffff020000955554` · `bidirectional_icmp=False` | confirmed |

### SQ — kernel attach semantics on a scratch persistent TAP

- **Hypothesis:** a single-queue persistent TAP admits exactly one attached
  queue fd, and `TUN_FEATURES` follow the last attacher.
- **Falsification:** an attach succeeds while another queue fd is attached, or
  `tun_flags` does not track the attacher's flags.

    SQ creator_TUNGETIFF=name=pfx4ef2 flags=0x1802
    SQ launcher_attach_while_creator_fd_open=errno=16(EBUSY) Device or resource busy
    SQ after_creator_close_snapshot={… "holders": [], … "info_data": {"multi_queue": false, "persist": true, "pi": false, "type": "tap", "user": 4200, "vnet_hdr": false}, … "tun_flags": "0x1802"}
    SQ attach_without_vnet_hdr=ok TUNGETIFF=name=pfx4ef2 flags=0x1802 sysfs_tun_flags=0x1802
    SQ second_attach_while_queue_attached=errno=16(EBUSY) Device or resource busy
    SQ ch_style_reissue_TUNSETIFF_with_vnet_on_attached_fd=errno=17(EEXIST) File exists sysfs_tun_flags=0x1802
    SQ after_plain_close holders=[] sysfs_tun_flags=0x1802
    SQ attach_with_vnet_hdr=ok TUNGETIFF=name=pfx4ef2 flags=0x5802 sysfs_tun_flags=0x5802
    SQ after_vnet_close holders=[] sysfs_tun_flags=0x5802
    SQ reattach_without_vnet_hdr=ok sysfs_tun_flags=0x1802
    SQ attach_for_delete_probe=ok holders=[[20210, 3, 'python3']]
    SQ rtm_dellink_with_queue_attached rc=0 stderr='' exists_after=False
    SQ holder_TUNGETIFF_after_delete=errno=77(EBADFD) File descriptor in bad state holders_after=[]

Answers to the "observe, don't assume" questions:

- **Launcher attach while the creator fd is open fails with `EBUSY`.** It
  succeeds only after the creator closes. Exclusivity is kernel-enforced: while
  CH holds the queue, no other component can attach.
- **`IFF_VNET_HDR` is set by whoever attaches, and it sticks.** It survives
  detach (`0x5802` after close). A later attacher overwrites it: a reattach
  without it reverted to `0x1802`. A re-`TUNSETIFF` on an already-attached fd
  returns `EEXIST` and cannot change it. That is exactly the call CH v53 makes.
- **`RTM_DELLINK` succeeds even with a queue attached.** It forcibly detaches
  the holder (`EBADFD` afterwards). Deletion therefore does not require prior
  VMM exit, but deleting under a live VMM yanks its queue.

### Step 1 — network owner creates the persistent TAP and leaves it down

- **Hypothesis:** the production creator sequence yields a persistent, down,
  owner-4200 TAP with zero queue holders after its fd closes.
- **Falsification:** the TAP is absent after creator close or is `UP`, holders
  are non-empty, or persistence/owner differ.

    STEP1 after_creator_close_snapshot={"address": "7a:d3:3e:07:a2:a2", "carrier": "errno=22(EINVAL) Invalid argument", "exists": true, "fdinfo_scanned": 688, "flags": ["BROADCAST", "MULTICAST"], "group": "-1", "holders": [], "ifindex": 58837, "info_data": {"multi_queue": false, "persist": true, "pi": false, "type": "tap", "user": 4200, "vnet_hdr": false}, "master": null, "mtu": 1500, "offload": {"generic-receive-offload": "on", "generic-segmentation-offload": "on", "rx-checksumming": "off [fixed]", "scatter-gather": "on", "tcp-segmentation-offload": "off", "tx-checksum-ip-generic": "off", "tx-checksumming": "off", "tx-udp-segmentation": "off", "udp-fragmentation-offload": "?"}, "offload_digest": "7210dc20a2ecce15", "operstate": "DOWN", "owner": "4200", "stats": {"rx_bytes": 0, "rx_dropped": 0, "rx_packets": 0, "tx_bytes": 0, "tx_dropped": 0, "tx_packets": 0}, "tun_flags": "0x1802"}
    STEP1 RESULT=PASS tap=pftap4ef2 ifindex=58837 tun_flags=0x1802 owner=4200 flags=['BROADCAST', 'MULTICAST'] master=pfbr4ef2 holders=[]

`TUNSETOFFLOAD(0)` already leaves `tx-checksum-ip-generic: off`. The later
owner `ethtool -K … off` was therefore a no-op: `rc=0`, and the digest was
unchanged.

### Step 2 — VMM launcher opens its queue fd; hands it to CH (with the failure arm)

- **Hypothesis:** a separate launcher `open` + `TUNSETIFF(0x5002)` attaches to
  the existing persistent TAP without the creator fd.
- **Falsification:** the attach errors, `tun_flags` does not track the request,
  or the TAP changes admin state.

    STEP2 launcher_fd=50 inode=137 device=7 TUNGETIFF=name=pftap4ef2 flags=0x5802
    STEP2 launcher_fdinfo=pos:	0; flags:	02104002; mnt_id:	30; ino:	137; iff:	pftap4ef2
    STEP2 diff_vs_step1={"info_data": [{"multi_queue": false, "persist": true, "pi": false, "type": "tap", "user": 4200, "vnet_hdr": false}, {"multi_queue": false, "persist": true, "pi": false, "type": "tap", "user": 4200, "vnet_hdr": true}], "tun_flags": ["0x1802", "0x5802"]}
    FAILURE_ARM fd_flags_before=0x1 fd_flags_after=0x0 parent_non_cloexec_fds=[0, 1, 2, 50]
    FAILURE_ARM failure_exit=1 failure_elapsed_seconds=0.031660
    FAILURE_ARM failure_stderr_tail=cloud-hypervisor:   0.015924s: <main> ERROR:/home/runner/work/cloud-hypervisor/cloud-hypervisor/cloud-hypervisor/src/lib.rs:24 -- Fatal error: VmBoot(VmBoot(KernelFile(Os { code: 2, kind: NotFound, message: "No such file or directory" }))) | Error: Cloud Hypervisor exited with the following chain of errors: |   0: Error booting VM |   1: The VM could not boot |   2: Cannot open kernel file |   3: No such file or directory (os error 2)
    FAILURE_ARM diff_vs_launcher_attach={} launcher_fd_inode=137 launcher_TUNGETIFF=name=pftap4ef2 flags=0x5802
    FAILURE_ARM after_launcher_close_snapshot={… "flags": ["BROADCAST", "MULTICAST"], … "holders": [], … "info_data": {… "persist": true, … "vnet_hdr": true}, … "tun_flags": "0x5802"}
    FAILURE_ARM subsequent_launcher_attach=ok fd=50 inode=137 TUNGETIFF=name=pftap4ef2 flags=0x5802 holders=[[20210, 50, 'python3']] tun_flags=0x5802
    STEP2 RESULT=PASS launcher_fd=50 TUNGETIFF=name=pftap4ef2 flags=0x5802 tun_flags=0x5802

**Failure-arm record.** After the failed launch the launcher fd was intact:
same inode, `iff: pftap4ef2`, sole holder the probe process. The persistent TAP
stayed down with unchanged state. After the launcher closed the fd, the TAP
**persisted** with `holders=[]` and `vnet_hdr` still on. A fresh launcher attach
then succeeded, and fd numbering was deterministic (50 again).

Unlike the prior non-persistent probe, a failed launch no longer implies TAP
destruction. The persistent TAP outlives the failed launch's queue. A failure
path must therefore either delete it or keep it for retry, explicitly.

### Step 3 — CH reaches READY while the TAP is down

- **Hypothesis:** CH v53 consumes the inherited queue fd without raising the
  persistent TAP, and the guest reaches READY with zero frames.
- **Falsification:** the TAP is `UP` at READY; any frame or counter (including
  `rx_dropped`) is non-zero; or CH altered tun_flags, owner, or offload.

    STEP3 exec_elapsed_seconds=0.004953 child_pid=20292
    STEP3 child_status={"Gid": "4200\t4200\t4200\t4200", "Groups": "991", "NoNewPrivs": "1", "Seccomp": "0", "Seccomp_filters": "0", "Uid": "4200\t4200\t4200\t4200"}
    STEP3 cgroup=/sys/fs/cgroup/overdrive.slice/workloads.slice/pf-tap-4ef2.scope cgroup_procs=20292
    STEP3 ready_line=READY pid=1 port=1234 ready_elapsed_seconds=1.114602
    STEP3 at_ready_snapshot={… "flags": ["BROADCAST", "MULTICAST"], … "operstate": "DOWN", "owner": "4200", "stats": {"rx_bytes": 0, "rx_dropped": 0, "rx_packets": 0, "tx_bytes": 0, "tx_dropped": 0, "tx_packets": 0}, "tun_flags": "0x5802"}
    STEP3 diff_vs_pre_launch={} offload_digest_pre=7210dc20a2ecce15 offload_digest_ready=7210dc20a2ecce15
    STEP3 ch_tun_fds_at_ready=[{"fd": 50, "fdinfo_flags": "0104002", "iff": "pftap4ef2", "inode": 137}, {"fd": 67, "fdinfo_flags": "0104002", "iff": "pftap4ef2", "inode": 137}, {"fd": 120, "fdinfo_flags": "02104002", "iff": "pftap4ef2", "inode": 137}, {"fd": 121, "fdinfo_flags": "02104002", "iff": "pftap4ef2", "inode": 137}, {"fd": 125, "fdinfo_flags": "02104002", "iff": "pftap4ef2", "inode": 137}, {"fd": 128, "fdinfo_flags": "02104002", "iff": "pftap4ef2", "inode": 137}]
    STEP3 thread_security_at_ready=[{"Name": "cloud-hyperviso", "NoNewPrivs": "1", "Seccomp": "0", "Seccomp_filters": "0", "tid": 20292}, {"Name": "vmm", "NoNewPrivs": "1", "Seccomp": "2", "Seccomp_filters": "1", "tid": 20293}, … {"Name": "_net1_qp0", "NoNewPrivs": "1", "Seccomp": "2", "Seccomp_filters": "2", "tid": 20301}, …]
    STEP3 pre_activation_capture_packets=0 bytes=0

The comparison at READY covers flags, operstate, master, `tun_flags`, owner,
group, `info_data`, MTU, MAC, and the complete `ethtool -k` table. It spans the
guest's virtio-net activation, at which CH issues `TUNSETOFFLOAD(0)`, and it
was **identical** to the post-attach state. In fd mode CH did not alter admin
state, offloads, owner, or persistence.

The guest's `rx_dropped` is also zero. Guest writes to a down TAP are dropped
and counted there, so zero means the guest attempted no transmit while down;
nothing was merely discarded.

The run-3 variant reproduced the same zero-frame READY (1.125789 s).

### Step 4 — launcher closes its copy; CH retains the queue

- **Hypothesis:** once the launcher closes, CH's fds are the only remaining
  queue holders.
- **Falsification:** the TAP disappears or changes, CH loses the iff
  association, or another holder remains.

    STEP4 after_launcher_close_snapshot={… "flags": ["BROADCAST", "MULTICAST"], … "holders": [[20292, 50, "cloud-hyperviso"], [20292, 67, "cloud-hyperviso"], [20292, 120, "cloud-hyperviso"], [20292, 121, "cloud-hyperviso"], [20292, 125, "cloud-hyperviso"], [20292, 128, "cloud-hyperviso"]], … "tun_flags": "0x5802"}
    STEP4 RESULT=PASS holders=[[20292, 50, 'cloud-hyperviso'], [20292, 67, 'cloud-hyperviso'], [20292, 120, 'cloud-hyperviso'], [20292, 121, 'cloud-hyperviso'], [20292, 125, 'cloud-hyperviso'], [20292, 128, 'cloud-hyperviso']] tap_exists=true flags=['BROADCAST', 'MULTICAST']

CH holds **six** descriptors for the one queue:

- fd 50 is the inherited fd, which CH never closes. It has no `O_CLOEXEC`.
- fd 67 or 68 is `from_tap_fds`' `dup`.
- fds 120, 121, 125, and 128 carry `O_CLOEXEC`.

All six resolve to `iff: pftap4ef2`. The TAP is single-queue and a second
attach is `EBUSY` (SQ), so all six must be duplicates of the one attached open
file description. Queue lifetime therefore ends only when CH closes all of
them, which in practice means process exit.

### Step 5 — network owner raises the TAP after interception is live

- **Hypothesis:** after the guard reads back, the owner's offload disable plus
  `UP` yields `UP,LOWER_UP` and bidirectional traffic, entirely after the
  barrier.
- **Falsification:** a read-back mismatch, no bidirectional progress, or any
  capture timestamp at or before the barrier.

    STEP5   table bridge pfguard4ef2 { # handle 2758
    STEP5   	chain input { # handle 1
    STEP5   		type filter hook input priority -300; policy drop;
    STEP5   		iifname "pftap4ef2" ether type arp counter packets 0 bytes 0 accept # handle 2
    STEP5   		iifname "pftap4ef2" ip protocol icmp counter packets 0 bytes 0 accept # handle 3
    STEP5   	}
    STEP5   	chain forward { # handle 4
    STEP5   		type filter hook forward priority -300; policy drop;
    STEP5   	}
    STEP5   }
    STEP5 owner_disable_tx_offload rc=0 stdout='' stderr=''
    STEP5 pre_up_snapshot={… "flags": ["BROADCAST", "MULTICAST"], … "stats": {"rx_bytes": 0, "rx_dropped": 0, "rx_packets": 0, "tx_bytes": 0, "tx_dropped": 0, "tx_packets": 0}, …}
    STEP5 activation_wall_ns=1790181112006878811 activation_elapsed_seconds=0.002445
    STEP5 active_snapshot={… "carrier": "1", … "flags": ["BROADCAST", "MULTICAST", "UP", "LOWER_UP"], … "master": "pfbr4ef2", … "operstate": "UP", … "stats": {"rx_bytes": 0, "rx_dropped": 0, "rx_packets": 0, "tx_bytes": 200, "tx_dropped": 0, "tx_packets": 2}, …}
    STEP5 bridge_readback=58837: pftap4ef2: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 master pfbr4ef2 state forwarding priority 32 cost 2
    STEP5 traffic_elapsed_seconds=0.020556 tap_stats={"rx_bytes": 140, "rx_dropped": 0, "rx_packets": 2, "tx_bytes": 430, "tx_dropped": 0, "tx_packets": 5}
    STEP5 post_activation_capture_packets=5 bytes=370 records_at_or_before_barrier=0 guest_originated=2 well_formed_guest=2 to_guest=2 icmp_reply_to_guest=True
    STEP5 first_frame_after_barrier_ms=40.338
    STEP5 post_activation_frame ts=1790181112047217231 pkttype=1 len=42 src=02:00:00:95:4e:f2 dst=ff:ff:ff:ff:ff:ff kind=arp-request head=ffffffffffff020000954ef2080600010800060400010200
    STEP5 post_activation_frame ts=1790181112047247086 pkttype=4 len=42 src=f6:d6:9e:51:f2:6a dst=02:00:00:95:4e:f2 kind=arp-reply head=020000954ef2f6d69e51f26a08060001080006040002f6d6
    STEP5 post_activation_frame ts=1790181112047469831 pkttype=3 len=98 src=02:00:00:95:4e:f2 dst=f6:d6:9e:51:f2:6a kind=icmp-echo-request head=f6d69e51f26a020000954ef20800450000549cd540004001
    STEP5 post_activation_frame ts=1790181112047485545 pkttype=4 len=98 src=f6:d6:9e:51:f2:6a dst=02:00:00:95:4e:f2 kind=icmp-echo-reply head=020000954ef2f6d69e51f26a080045000054bc9a00004001
    STEP5 post_activation_frame ts=1790181112101642572 pkttype=4 len=90 src=7a:d3:3e:07:a2:a2 dst=33:33:00:00:00:16 kind=ipv6 head=3333000000167ad33e07a2a286dd60000000002400010000
    STEP5 RESULT=PASS guard_readback=exact UP=['BROADCAST', 'MULTICAST', 'UP', 'LOWER_UP'] master=pfbr4ef2 rx=2 tx=5 frames=5 all_after_barrier=true icmp_reply_to_guest=true

The pre-activation window, up to the barrier, is covered by two measurements:

- **The all-EtherType capture socket** was bound to ifindex 58837 before either
  CH launch. It held zero records at READY.
- **A counter read immediately before `UP`** showed all six counters, including
  `rx_dropped`, at zero.

The post-activation socket reopens after the `UP` read-back, per prior edge
case 1. In the gap before it bound, the counters show `tx_packets: 2` and
`rx_packets: 0`. Those two frames were host-originated, sent toward the guest,
and after the barrier by construction: the host cannot transmit on an
admin-down device. The 90-byte `33:33:00:00:00:16` frame above is the same
class, an IPv6 MLD report from the TAP's own link-local.

The guest-originated count in that gap is zero (`rx_packets: 0`).

### Step 6 — CH exit closes its queue; the persistent TAP remains

- **Hypothesis:** CH exit closes its queue, and the persistent TAP remains with
  no holder anywhere.
- **Falsification:** the TAP disappears, a holder remains, or
  persistence/owner changed.

    STEP6 main_exit=0 cancellation_elapsed_seconds=0.031659 ch_pid_alive=False
    STEP6 after_ch_exit_settled_snapshot={"address": "7a:d3:3e:07:a2:a2", "carrier": "0", "exists": true, "fdinfo_scanned": 714, "flags": ["NO-CARRIER", "BROADCAST", "MULTICAST", "UP"], "group": "-1", "holders": [], "ifindex": 58837, "info_data": {"multi_queue": false, "persist": true, "pi": false, "type": "tap", "user": 4200, "vnet_hdr": true}, "master": "pfbr4ef2", "mtu": 1500, …, "operstate": "DOWN", "owner": "4200", "stats": {"rx_bytes": 140, "rx_dropped": 0, "rx_packets": 2, "tx_bytes": 430, "tx_dropped": 3, "tx_packets": 5}, "tun_flags": "0x5802"}
    STEP6 diff_active_vs_exit={"carrier": ["1", "0"], "flags": [["BROADCAST", "MULTICAST", "UP", "LOWER_UP"], ["NO-CARRIER", "BROADCAST", "MULTICAST", "UP"]], "operstate": ["UP", "DOWN"]}
    STEP6 tap_ip_detail=58837: pftap4ef2: <NO-CARRIER,BROADCAST,MULTICAST,UP> mtu 1500 qdisc fq_codel master pfbr4ef2 state DOWN mode DEFAULT group default qlen 1000 |     link/ether 7a:d3:3e:07:a2:a2 brd ff:ff:ff:ff:ff:ff promiscuity 1 allmulti 1 minmtu 68 maxmtu 65521  |     tun type tap pi off vnet_hdr on persist on user 4200  |     bridge_slave state disabled …
    STEP6 RESULT=PASS exists=true flags=['NO-CARRIER', 'BROADCAST', 'MULTICAST', 'UP'] operstate=DOWN carrier=0 master=pfbr4ef2 tun_flags=0x5802 owner=4200 holders=[] (fdinfo scanned=714)

`holders=[]` comes from reading every `/proc/*/fdinfo/*` on the host (714
entries) for `iff: pftap4ef2`.

After CH exit the TAP state was:
- **admin `UP` retained**: nothing lowered it;
- NO-CARRIER, operstate `DOWN`;
- bridge port `disabled`;
- master unchanged;
- persistence, owner, and `vnet_hdr` unchanged.

`tx_dropped` rose 1 → 3 within 0.5 s. The host kept transmitting IPv6 control
frames at a TAP with no queue.

**Safe to delete?** Yes: nothing was attached. SQ shows deletion would also
succeed with a holder attached, by forcibly detaching it.

### Step 7 — explicit teardown removes the TAP with no residue

- **Hypothesis:** set-down plus `RTM_DELLINK` removes the persistent TAP, and
  the spike's remaining resources remove to an exact empty complement.
- **Falsification:** the TAP survives delete, or any residue remains.

    STEP7 after_set_down flags=['BROADCAST', 'MULTICAST'] operstate=DOWN holders=[]
    STEP7 rtm_dellink rc=0 stderr=''
    STEP7 ip_link_show_after rc=1 stderr='Device "pftap4ef2" does not exist.' sysfs_exists=False
    STEP7 holders_after_delete=[] fdinfo_scanned=714
    STEP7 final_readback={"bridge_exists": false, "cgroup_exists": false, "new_cloud_hypervisor_pids": [], "nft_table_exists": false, "probe_open_tun_fds": [], "run_dir_exists": false, "scratch_exists": false, "scratch_queue_holders": [], "staging_exists": false, "tap_exists": false, "tap_queue_holders": []}
    STEP7 RESULT=PASS tap deleted, exact empty complement
    CLEANUP residue_found_by_fallback=[]
    cleanup_complete=True

`residue_found_by_fallback=[]` means the `finally` cleanup owner found nothing
to remove. The explicit step-7 teardown alone produced the empty complement.
The same held in runs 2 and 3.

### Restart observation (run 2) — attaching to a TAP left admin-UP

- **Hypothesis:** `tun_set_iff` turns carrier on at attach. A relaunch attach
  onto a persistent TAP left admin-`UP` after VMM exit therefore makes it
  `LOWER_UP` before any VMM runs. A down-first reattach stays down.
- **Falsification:** an attach onto an admin-`UP` TAP stays NO-CARRIER with
  nothing queued.

    RESTART_OBS reattach_while_admin_up_snapshot={… "carrier": "1", … "flags": ["BROADCAST", "MULTICAST", "UP", "LOWER_UP"], … "holders": [[21031, 50, "python3"]], … "operstate": "UP", …}
    RESTART_OBS queued_frames_while_admin_up=7
    RESTART_OBS queued_frame len=122 head=0000000000000000000000003333000000160234c342c6d986dd6000000000380001000000000000 as_vnet12=ipv6 as_vnet10=ethertype-0xc6d9
    RESTART_OBS queued_frame len=98 head=0000000000000000000000003333ff42c6d90234c342c6d986dd6000000000203aff000000000000 as_vnet12=ipv6 as_vnet10=ethertype-0xc6d9
    RESTART_OBS queued_frame len=82 head=0000000000000000000000003333000000026626515d688786dd6000000000103afffe8000000000 as_vnet12=ipv6 as_vnet10=ethertype-0x6887
    RESTART_OBS after_close flags=['NO-CARRIER', 'BROADCAST', 'MULTICAST', 'UP'] carrier=0 holders=[]
    RESTART_OBS reattach_after_set_down_snapshot={… "carrier": "errno=22(EINVAL) Invalid argument", … "flags": ["BROADCAST", "MULTICAST"], … "operstate": "DOWN", …}
    RESTART_OBS queued_frames_after_set_down=0 stats_before_attach={"rx_bytes": 140, "rx_dropped": 0, "rx_packets": 2, "tx_bytes": 1072, "tx_dropped": 2, "tx_packets": 12}
    RESTART_OBS RESULT=OBSERVED admin_up_reattach flags=['BROADCAST', 'MULTICAST', 'UP', 'LOWER_UP'] carrier=1 queued=7; down_first_reattach flags=['BROADCAST', 'MULTICAST'] queued=0 stats_delta={'rx_packets': 0, 'tx_packets': 0, 'rx_bytes': 0, 'tx_bytes': 0, 'rx_dropped': 0, 'tx_dropped': 0}

Within 0.5 s of a bare launcher attach, with no VMM and no fresh guard proof,
the TAP was `LOWER_UP` and 7 host IPv6 control frames (MLD, NS, RS) had been
queued toward the not-yet-running guest. They carry a 12-byte zero header. The
vnet-hdr **size** set by CH's `TUNSETVNETHDRSZ(12)` persists on the TAP after
CH exits, just as the `IFF_VNET_HDR` flag does.

A guest attached through that queue could also transmit before READY and
before interception. The down-first reattach showed no `UP`, nothing queued,
and a zero counter delta.

### No-vnet-hdr variant (run 3) — which attach flags CH v53 needs

- **Hypothesis:** CH v53 cannot set `IFF_VNET_HDR` on an already-attached fd
  and does not check for it.
- **Prediction:** CH accepts a queue attached with the production creator's
  flags only, reaches READY, and then sends virtio-net headers onto a TAP that
  does not expect them.

    ENV variant=no-vnet-hdr launcher_flags=0x1002
    STEP2 launcher_fd=50 inode=137 device=7 TUNGETIFF=name=pftap5554 flags=0x1802
    STEP3 ready_line=READY pid=1 port=1234 ready_elapsed_seconds=1.125789
    STEP3 pre_activation_capture_packets=0 bytes=0
    STEP5 post_activation_frame ts=1790181267369390654 pkttype=3 len=54 src=00:00:00:00:00:00 dst=00:00:00:00:00:00 kind=ethertype-0xffff head=000000000000000000000000ffffffffffff020000955554
    STEP5 post_activation_frame ts=1790181268403603689 pkttype=3 len=54 src=00:00:00:00:00:00 dst=00:00:00:00:00:00 kind=ethertype-0xffff head=000000000000000000000000ffffffffffff020000955554
    STEP5 post_activation_frame ts=1790181269427488456 pkttype=3 len=54 src=00:00:00:00:00:00 dst=00:00:00:00:00:00 kind=ethertype-0xffff head=000000000000000000000000ffffffffffff020000955554
    STEP5 RESULT=OBSERVED OBSERVED bidirectional_icmp=False well_formed_guest_frames=0 guest_frames=0 stats={'rx_packets': 3, 'tx_packets': 13, 'rx_bytes': 162, 'tx_bytes': 1182, 'rx_dropped': 0, 'tx_dropped': 0}
    STEP6 main_exit=0 cancellation_elapsed_seconds=0.031782 ch_pid_alive=False

Each guest frame is 12 zero bytes followed by the real 42-byte ARP request
(`ffffffffffff 020000955554 0806…`), 54 bytes in all. The guest retried ARP at
about 1 s intervals, and ICMP never completed.

CH raised no error at any point: it booted, reached READY, and exited 0. The
flag requirement is therefore invisible to launch success. Only an L2 or
traffic check exposes it.

## Edge cases

1. **Admin state left behind.** The persistent TAP retains admin `UP` after VMM
   exit (step 6), and any later attach raises carrier (restart observation).
   This is the one place the persistent lifecycle differs materially from the
   prior non-persistent probe, where the TAP vanished with its last fd.
2. **Sticky per-TAP settings.** `IFF_VNET_HDR` and the vnet-hdr size are
   per-TAP and sticky. The last attacher sets the flag, and CH sets the size.
   This matters for any other component that reattaches by name. One such
   caller is `set_persistent_tap_owner` in
   `crates/overdrive-control-plane/src/veth_provisioner.rs`, which
   `TUNSETIFF`s without `IFF_VNET_HDR`. While a VMM holds the queue it would
   get `EBUSY`. Between launches it would clear `IFF_VNET_HDR`.
3. **Six duplicates of one queue.** CH keeps its inherited fd open, without
   `O_CLOEXEC`, for its whole life, and duplicates it five times. The one queue
   is released only when all of them close, which in practice means CH exit.
4. **Deletion yanks a live queue.** `RTM_DELLINK` with a queue attached
   succeeds and detaches the holder (`EBADFD`). Ordering is therefore a design
   choice, not a kernel constraint.
5. **Host IPv6 on the TAP.** The TAP runs host IPv6 addrconf (`addrgenmode
   eui64`, bridge port). Right after `UP`, and at every carrier-on, it emits
   MLD/NS/RS toward the guest, and `tx_dropped` grows while no queue is
   attached. The prior probe disabled host IPv6 on the TAP; production does
   not. This does not affect the pre-activation invariant, which the counters
   and the capture cover. It is relevant to what the post-activation
   classifier sees.
6. **Measurement notes.** Carrier reads return `EINVAL` on an admin-down
   interface, and are recorded as such. `ethtool 6.19` prints no
   `udp-fragmentation-offload` key, recorded as `?`. CH's leader thread is
   `Seccomp=0` while every worker is `Seccomp=2` (prior edge case 3,
   reproduced).

## Design implications for the replacement DESIGN

These are evidence-backed constraints for the DESIGN to pin, not a proposed API.

- **Creation and persistence.** A network owner can own creation and
  `TUNSETPERSIST` exactly as today. After `create_persistent_tap` returns, the
  persistent TAP exists with zero queue holders, so a separate launcher can
  attach. The creator's fd must be closed before any launcher attach, or the
  attach gets `EBUSY`. Production already closes it inside
  `create_persistent_tap`.
- **Attach flags are part of the contract.** The launcher's `TUNSETIFF` must
  include `IFF_VNET_HDR`, and must not include `IFF_MULTI_QUEUE` unless the
  creator created a multi-queue TAP. Per `tun.c:2722`, a mismatch is `EINVAL`;
  this is source-only, not exercised here. `IFF_VNET_HDR` cannot be deferred to
  CH. The design must also say who else, if anyone, may reattach by name, given
  the sticky last-attacher semantics.
- **fd lifetime.**
  - The launcher's copy may be closed as soon as CH has exec'd; steps 3–4 show
    this.
  - A failed launch leaves the launcher fd attached and the TAP persistent. The
    failure owner must close the fd, which does not destroy the TAP.
  - After that, the owner must either delete the TAP or keep it for retry,
    explicitly.
  - Queue lifetime ends at CH exit, because CH holds six duplicates.
- **Activation and deactivation ownership.** Only the network owner raised the
  TAP; CH never did. The new, persistent-specific requirement is deactivation.
  After VMM exit, the owner must set the TAP down before any relaunch attach.
  Otherwise the attach itself activates the link before READY and before
  interception, which breaks the zero-frame invariant on restart. The
  production teardown already runs `set_tap_down` before delete. A restart or
  reuse path needs the same ordering pinned.
- **Delete.** `RTM_DELLINK` on the persistent TAP is the explicit teardown, and
  it succeeds whether or not a queue is attached. After VMM exit nothing is
  attached (step 6), so ordering deletion after VMM exit avoids yanking a live
  guest queue.
- **Owner uid (source inference, not tested).** In fd mode CH never opens the
  TAP by name. Its `TUNSETIFF` on the passed fd returns `EEXIST` before any
  capability or owner check (`tun.c:3082`). The `TUNSETOWNER(4200)` grant is
  therefore not exercised by CH. It remains a standing authorization for uid
  4200 to attach whenever no queue is held. Whether to keep it is a DESIGN
  decision. This probe ran with the production owner grant throughout and did
  not test its removal.
- **Queues.** This remains one fd, one queue pair, and CH `num_queues=2`, with
  no `IFF_MULTI_QUEUE`. The prior probe's multiqueue implications carry over
  unchanged, and multiqueue is additionally a creation-time decision (see
  Attach flags).

## Primary-source correlation

### Cloud Hypervisor v53.0

- [`Tap::from_tap_fd`](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v53.0/net_util/src/tap.rs#L235-L289):
  - `TUNGETIFF`;
  - re-issues `TUNSETIFF(IFF_TAP|IFF_NO_PI|IFF_VNET_HDR)` and **ignores
    `EEXIST`** (L268–L282, comment at L269);
  - then `TUNSETVNETHDRSZ` (L285–L286).

  On an attached fd that `TUNSETIFF` always returns `EEXIST`, so CH cannot
  supply `IFF_VNET_HDR`. This explains the run-3 corruption and the no-change
  diff at READY.
- The named-TAP path supplies the flag itself in
  [`open_named`](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v53.0/net_util/src/tap.rs#L193).
  That is why production's creator never needed it.
- [`Tap::enable`](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v53.0/net_util/src/tap.rs#L457)
  performs the `SIOCSIFFLAGS` that raises the link. It is called only from
  [`open_tap`](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v53.0/net_util/src/open_tap.rs#L100),
  the named path, never from the fd path. This explains why the TAP stayed down
  through READY.
- [`Net::from_tap_fds`](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v53.0/virtio-devices/src/net.rs#L701-L751)
  `dup()`s each supplied fd ("so that it can survive reboots", L720) and never
  closes the original. This is consistent with the inherited fd 50 remaining
  open in CH.
- At virtio activation CH calls
  [`set_offload(virtio_features_to_tap_offload(acked))`](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v53.0/virtio-devices/src/net.rs#L961).
  The mapping is in
  [`net_util/src/lib.rs`](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v53.0/net_util/src/lib.rs#L167-L186).
  With `offload_{tso,ufo,csum}=off` no guest offload is acked, so the value is
  `0`, the same value the production creator already set. This explains the
  unchanged `ethtool -k` digest.

### Linux v7.0 `drivers/net/tun.c`

- [`tun_attach` returns `EBUSY`](https://github.com/torvalds/linux/blob/v7.0/drivers/net/tun.c#L706)
  when a single-queue TAP already has one queue.
- [`TUNSETIFF` on an attached fd returns `EEXIST`](https://github.com/torvalds/linux/blob/v7.0/drivers/net/tun.c#L3082).
- The [existing-device attach path overwrites `TUN_FEATURES`](https://github.com/torvalds/linux/blob/v7.0/drivers/net/tun.c#L2747)
  from the attacher's flags, which gives the sticky last-attacher
  `IFF_VNET_HDR`.
- A [multi-queue flag mismatch returns `EINVAL`](https://github.com/torvalds/linux/blob/v7.0/drivers/net/tun.c#L2722).
- `tun_set_iff` ends with
  [`netif_carrier_on`](https://github.com/torvalds/linux/blob/v7.0/drivers/net/tun.c#L2821)
  (unless `IFF_NO_CARRIER`). This is the restart hazard.
- On last-queue detach,
  [`netif_carrier_off`, and unregister only if not `IFF_PERSIST`](https://github.com/torvalds/linux/blob/v7.0/drivers/net/tun.c#L617-L621).
  This explains step 6: the TAP persists, NO-CARRIER, admin state untouched.
- Guest writes to an admin-down TAP are
  [dropped with `-EIO`](https://github.com/torvalds/linux/blob/v7.0/drivers/net/tun.c#L1895)
  and [counted in `rx_dropped`](https://github.com/torvalds/linux/blob/v7.0/drivers/net/tun.c#L1968).
  This is why the zero `rx_dropped` at READY is load-bearing.

### Documentation and prior probe

- Kernel [TUN/TAP documentation](https://docs.kernel.org/networking/tuntap.html):
  `TUNSETIFF`, `TUNSETPERSIST`, persistent-device semantics.
- The prior probe's findings and code are preserved unmodified at
  `spike-scratch/netns-density-295-fd-tap/spike.py`, SHA-256
  `a641f45872240e704b4eaff5a3d474ad2c34c665f9508657d4dc18df7dc2e1a9`
  (re-verified).

## Spike code

`spike-scratch/increment-y-netns-density-295-persistent-fd-tap/spike.py`, SHA-256
`9839150dd80a50462229f2cb635f2227137eb3ac6a2d05801f9d0649a50e8de2` (the final
file, used by runs 2 and 3). Evidence logs are under
`spike-scratch/increment-y-netns-density-295-persistent-fd-tap/evidence/`.
