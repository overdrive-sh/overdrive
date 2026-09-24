# Launcher seccomp filter for TAP-mutating ioctls — native spike findings

**Date:** 2026-09-24  
**Feature:** `netns-density-295`  
**Verdict:** **WORKS**

## Question

Can a narrow seccomp deny-list, installed by the launcher before `exec` and
inherited by Cloud Hypervisor, remain compatible with Cloud Hypervisor v53.0 on
the `--net fd=` path while binding every Cloud Hypervisor thread?

**Answer: yes on the measured substrate.** Cloud Hypervisor v53.0 reached the
guest `READY` beacon and passed both guest-to-host and host-to-guest ICMP after
activation under the launcher filter. All 11 observed Cloud Hypervisor threads,
including the otherwise-unfiltered process main thread, carried the inherited
filter. Each of the 13 deny-listed requests returned `EPERM` on an attached TAP
queue descriptor in the filtered helper's main thread and three threads created
after filter installation. The no-filter control let the kernel handle every
request and returned no `EPERM`.

This is a PROBE result only. It records no promotion decision.

## Substrate

- Runner: native, non-virtualized `x86_64` metal
  (`systemd-detect-virt=none`).
- Kernel: `7.0.0-29-generic` (`uname -r`).
- VMM: `/usr/local/bin/cloud-hypervisor`, `cloud-hypervisor v53.0`.
- Primary source: Cloud Hypervisor tag `v53.0`, commit
  `9ed824d6d08df3e96f7d5f50795d9449ac99f431`.
- Network shape: persistent single-queue TAP, owner uid 0, root launcher attach,
  queue fd handed to Cloud Hypervisor as `fd=[50]`, then Cloud Hypervisor runs as
  uid/gid 4200 with its existing `--seccomp true` and `--landlock` controls.
- Launcher filter: x86_64 classic BPF, installed on the single-threaded child
  before the first `exec`, after `PR_SET_NO_NEW_PRIVS`; default action `ALLOW`,
  and `ERRNO(EPERM)` only when syscall `ioctl` has argument 1 equal to a listed
  request. Cloud Hypervisor's later filters stack on top of it.
- Traffic gate: the increment-y private guest/rootfs harness, a spike-local nft
  ingress guard read back before TAP activation, guest ping to `10.77.239.1`,
  and host ping to guest `10.77.239.2`.

Authoritative native evidence:

```text
[+ 0.058883] ENV uname_r=7.0.0-29-generic
[+ 0.061969] ENV systemd_detect_virt=none
[+ 0.061978] ENV cloud_hypervisor=/usr/local/bin/cloud-hypervisor version=cloud-hypervisor v53.0
```

## Step 1 — Cloud Hypervisor v53.0 source compatibility

**Hypothesis:** the `fd=` path issues only the TUN and `SIOC*` requests needed to
adopt the inherited queue, read/set its MTU, set the vnet header size, and
program negotiated offloads; none overlaps the proposed deny-list.

**Prediction:** `DeviceManager` selects `Net::from_tap_fds`; that path reaches
`Tap::from_tap_fd`, `new_with_tap`, and `activate`, and never reaches the named
path's IP, host-MAC, or link-enable setters.

**Falsification:** any `fd=` call reaches a deny-listed request, the named
`host_mac`/IP/link-enable path, or another ioctl absent from this inventory.

**Actual:** source audit passed at the exact tag commit:

```text
tag=v53.0 commit=9ed824d6d08df3e96f7d5f50795d9449ac99f431
SOURCE_AUDIT_RESULT=PASS exact_fd_path_ioctl_set=libc::TUNGETIFF,libc::TUNSETIFF,libc::TUNSETVNETHDRSZ,libc::SIOCGIFMTU,libc::SIOCSIFMTU,libc::TUNSETOFFLOAD
```

The exact set is:

| Request | Number | When and source path |
|---|---:|---|
| `TUNGETIFF` | `0x800454d2` | Unconditional queue identity/config read in `net_util/src/tap.rs:255-257`. |
| `TUNSETIFF` | `0x400454ca` | Unconditional attempt to apply `IFF_TAP|IFF_NO_PI|IFF_VNET_HDR` in `tap.rs:268-281`; `EEXIST` on an already-attached queue is explicitly accepted. |
| `TUNSETVNETHDRSZ` | `0x400454d8` | Unconditional `vnet_hdr_len()` setup in `tap.rs:284-286,478-482`. |
| `SIOCGIFMTU` | `0x8921` | Unconditional MTU read through a separate AF_UNIX socket in `virtio-devices/src/net.rs:546-553` → `tap.rs:413-425`. |
| `SIOCSIFMTU` | `0x8922` | Conditional only when `fd=...,mtu=` is configured, `net.rs:732-734` → `tap.rs:434-442`. The measured command supplied no `mtu=`. |
| `TUNSETOFFLOAD` | `0x400454d0` | Device activation in `net.rs:959-965`. |

`Tap::from_tap_fd` also issues `fcntl(F_GETFL/F_SETFL)` to add
`O_NONBLOCK`; those are not ioctls. `DeviceManager` passes the guest MAC and the
optional MTU into `from_tap_fds`; it does not pass a host MAC, host IP, or the
named-path link-enable operation.

## Step 2 — positive control without the launcher filter

**Hypothesis:** without the launcher filter, an attached queue holder at uid
4200 reaches the kernel's TUN ioctl switch, and Cloud Hypervisor's main thread
remains unfiltered by its own per-thread seccomp setup.

**Prediction:** none of the 13 requests returns `EPERM`; mutating requests with
valid arguments succeed, intentionally malformed filter/queue requests may
return their native kernel error; the Cloud Hypervisor leader reports
`Seccomp=0`, `Seccomp_filters=0`; the VM still boots and passes both pings.

**Falsification:** any request is preempted with `EPERM`, the leader already has
a filter, or the unfiltered boot/traffic path fails.

**Actual:** all predictions held. The control helper reported success for
`SIOCSIFHWADDR`, owner/group/persistence/carrier/debug/link changes and the
detach/eBPF filter calls; the deliberately empty `TUNATTACHFILTER` and
single-queue `TUNSETQUEUE` inputs returned `EINVAL`, not `EPERM`:

```text
IOCTL_CHILD [{"ioctls": {"SIOCSIFHWADDR": {"errname": "OK", "errno": 0, "rc": 0}, ...
"TUNATTACHFILTER": {"errname": "EINVAL", "errno": 22, "rc": -1}, ...
"TUNSETOWNER": {"errname": "OK", "errno": 0, "rc": 0},
"TUNSETPERSIST": {"errname": "OK", "errno": 0, "rc": 0},
"TUNSETQUEUE": {"errname": "EINVAL", "errno": 22, "rc": -1}, ...},
"status_before": {"Name": "python3", "NoNewPrivs": "1", "Seccomp": "0", "Seccomp_filters": "0"}, ...}]
IOCTL_CHILD_RESULT mode=control pass=True
BOOT RESULT=PASS mode=control READY; threads=11 leader_seccomp=0 leader_filters=0
TRAFFIC RESULT=PASS mode=control guest_ping=true host_ping=true rx_delta=3 tx_delta=6 all_threads=11
```

This is the causal control for the filtered `EPERM` result. It also reproduces
Cloud Hypervisor's existing main-thread gap from Addendum 2 B2.

## Step 3 — deny-list enforcement and thread inheritance

**Hypothesis:** installing the request-number filter before `exec` causes the
filter to survive the `prlimit` → `setpriv` → Cloud Hypervisor exec chain and to
be inherited by every later thread; stacked Cloud Hypervisor filters cannot
relax its `EPERM` action.

**Prediction:** all 13 requests return `EPERM` in a filtered descendant's main
thread and three later-created threads. At guest `READY`, every Cloud Hypervisor
thread reports seccomp mode 2 and at least one filter, including the process
leader that has zero filters in the control. Relative to control, each matching
thread reports exactly one additional filter.

**Falsification:** any request reaches the kernel, any helper or Cloud
Hypervisor thread lacks the inherited filter, or any matching Cloud Hypervisor
thread does not gain exactly one filter.

**Actual:** every request returned `EPERM` in all four filtered helper threads.
The helper main thread and each created thread reported `Seccomp=2` and
`Seccomp_filters=1`:

```text
IOCTL_CHILD [{"ioctls": {"SIOCSIFHWADDR": {"errname": "EPERM", "errno": 1, "rc": -1},
"TUNATTACHFILTER": {"errname": "EPERM", "errno": 1, "rc": -1},
"TUNDETACHFILTER": {"errname": "EPERM", "errno": 1, "rc": -1},
"TUNSETCARRIER": {"errname": "EPERM", "errno": 1, "rc": -1},
"TUNSETDEBUG": {"errname": "EPERM", "errno": 1, "rc": -1},
"TUNSETFILTEREBPF": {"errname": "EPERM", "errno": 1, "rc": -1},
"TUNSETGROUP": {"errname": "EPERM", "errno": 1, "rc": -1},
"TUNSETLINK": {"errname": "EPERM", "errno": 1, "rc": -1},
"TUNSETOWNER": {"errname": "EPERM", "errno": 1, "rc": -1},
"TUNSETPERSIST": {"errname": "EPERM", "errno": 1, "rc": -1},
"TUNSETQUEUE": {"errname": "EPERM", "errno": 1, "rc": -1},
"TUNSETSTEERINGEBPF": {"errname": "EPERM", "errno": 1, "rc": -1},
"TUNSETTXFILTER": {"errname": "EPERM", "errno": 1, "rc": -1}},
"label": "main", "status_before": {"NoNewPrivs": "1", "Seccomp": "2", "Seccomp_filters": "1"}, ...}, ...]
IOCTL_CHILD_RESULT mode=filtered pass=True
```

Kernel-reported filter counts at `READY` were:

| Cloud Hypervisor thread | Control filters | Filtered filters | Delta |
|---|---:|---:|---:|
| process leader `cloud-hyperviso` | 0 | 1 | +1 |
| `vmm` | 1 | 2 | +1 |
| `http-server` | 1 | 2 | +1 |
| `vmm_signal_hand` | 2 | 3 | +1 |
| `vcpu0` | 2 | 3 | +1 |
| `kvm-nx-lpage-re` | 2 | 3 | +1 |
| `_disk0_q0` | 2 | 3 | +1 |
| `_net1_ctrl` | 2 | 3 | +1 |
| `_net1_qp0` | 2 | 3 | +1 |
| `iou-wrk-*` | 2 | 3 | +1 |
| `_vsock2` | 2 | 3 | +1 |

The same 11-thread filtered set and counts were read again after live traffic.
This is direct evidence that the launcher layer covers Cloud Hypervisor's main
thread as well as every observed worker and composes with its own filters.

## Final deny-list

| Request | Number | Reason for denial |
|---|---:|---|
| `SIOCSIFHWADDR` | `0x8924` | Changes the TAP host-side MAC and can poison the shared bridge FDB. |
| `TUNSETOWNER` | `0x400454cc` | Can grant future by-name attachment to the VMM uid. |
| `TUNSETGROUP` | `0x400454ce` | Mutates the attachment grant surface. |
| `TUNSETPERSIST` | `0x400454cb` | Can remove persistence and make the TAP disappear on final close. |
| `TUNSETCARRIER` | `0x400454e2` | Mutates carrier outside the network owner's activation boundary. |
| `TUNSETDEBUG` | `0x400454c9` | Enables unbounded host kernel-log emission from the holder's TAP. |
| `TUNSETLINK` | `0x400454cd` | Attempts to mutate the netdev link type. |
| `TUNSETTXFILTER` | `0x400454d1` | Installs the legacy tun TX filter; unused by the CH v53 `fd=` path. |
| `TUNATTACHFILTER` | `0x401054d5` | Attaches a classic socket filter; unused by the CH v53 `fd=` path. |
| `TUNDETACHFILTER` | `0x401054d6` | Detaches that filter; unused by the CH v53 `fd=` path. |
| `TUNSETSTEERINGEBPF` | `0x800454e0` | Installs/removes an eBPF steering program; unused by the CH v53 `fd=` path. |
| `TUNSETFILTEREBPF` | `0x800454e1` | Installs/removes an eBPF filter; unused by the CH v53 `fd=` path. |
| `TUNSETQUEUE` | `0x400454d9` | Attaches/detaches a multiqueue queue; unused by this single-queue `fd=` path. |

`TUNSETIFF` cannot be denied: CH v53 deliberately reissues it in
`Tap::from_tap_fd` and accepts `EEXIST`. `TUNSETOFFLOAD` and
`TUNSETVNETHDRSZ` are also required. The narrow filter leaves read-only ioctls,
`SIOCGIFMTU`, optional `SIOCSIFMTU`, and holder-local buffer/offload/vnet-header
configuration outside the deny-list.

## Step 4 — filtered Cloud Hypervisor boot and bidirectional traffic

**Hypothesis:** because the final deny-list is disjoint from the exact `fd=`
path, Cloud Hypervisor v53.0 boots and operates the inherited TAP normally.

**Prediction:** `READY pid=1 port=1234`; CH retains the exact queue inode; after
guard read-back and activation, both guest→host and host→guest pings succeed,
both TAP directions advance, and no captured frame predates activation.

**Falsification:** boot exits or misses `READY`, the inherited queue is lost,
either ping fails, either counter direction is zero, or a frame timestamp is at
or before the activation barrier.

**Actual:** the filtered run passed:

```text
BOOT RESULT=PASS mode=filtered READY; threads=11 leader_seccomp=2 leader_filters=1
TRAFFIC host_ping_rc=0 stdout='PING 10.77.239.2 ... 1 packets transmitted, 1 received, 0% packet loss ...'
TRAFFIC activation_ns=1790251811186042490 before_stats={"rx_packets": 0, ... "tx_packets": 0}
after_stats={"rx_bytes": 238, "rx_dropped": 0, "rx_packets": 3, "tx_bytes": 438, "tx_dropped": 0, "tx_packets": 5}
TRAFFIC capture_count=8 pre_activation=0 guest_src=3 host_src=5
TRAFFIC RESULT=PASS mode=filtered guest_ping=true host_ping=true rx_delta=3 tx_delta=5 all_threads=11
VERDICT=WORKS mode=filtered
```

The capture includes both request/reply directions:

```text
frame={"dst":"16:40:eb:f8:07:02","kind":"icmp-request",...,"src":"02:00:00:aa:12:f5"}
frame={"dst":"02:00:00:aa:12:f5","kind":"icmp-reply",...,"src":"16:40:eb:f8:07:02"}
frame={"dst":"02:00:00:aa:12:f5","kind":"icmp-request",...,"src":"16:40:eb:f8:07:02"}
frame={"dst":"16:40:eb:f8:07:02","kind":"icmp-reply",...,"src":"02:00:00:aa:12:f5"}
```

## Step 5 — cleanup

**Hypothesis:** terminating Cloud Hypervisor and deleting only resources named
with this run's tag restores the exact pre-run complement.

**Prediction:** no new Cloud Hypervisor pid, TAP, scratch TAP, bridge, queue
holder, nft table, cgroup, run directory, staging directory, or open probe TUN
fd remains.

**Falsification:** any exact resource remains or cleanup touches a resource
outside the run tag.

**Actual:** both authoritative runs ended with the empty complement. Filtered:

```text
CLEANUP final_readback={"bridge_exists": false, "cgroup_exists": false,
"new_ch_pids": [], "nft_exists": false, "open_tun_fds": [],
"run_dir_exists": false, "scratch_exists": false, "scratch_holders": [],
"tap_exists": false, "tap_holders": [], "work_exists": false}
CLEANUP_FALLBACK []
cleanup_complete=True
```

## Edge cases and limits

- The filter program and numeric ioctl values in this probe are pinned to the
  measured x86_64 ABI. A production implementation must select the correct
  audit architecture and derive request values from the target libc/kernel
  headers; an architecture mismatch must fail closed.
- The filter matches request number, not fd identity. It therefore denies these
  requests on every descriptor in Cloud Hypervisor. That is intentional for
  this `fd=` launch shape, but a future `host_mac=` or named-TAP path could need
  `SIOCSIFHWADDR`/link ioctls and would be incompatible. Cloud Hypervisor
  upgrades and launch-shape changes need the same source audit plus a native
  boot/traffic check.
- `SIOCSIFMTU` is part of the `fd=` source path only when `mtu=` is configured;
  it remains allowed. The authoritative native command supplied no `mtu=`.
- The deny probe used an attached scratch TAP queue owned by uid 4200. It did
  not inject remote syscalls into live Cloud Hypervisor threads. Instead it
  proves the exact filter's return action in a filtered main thread and
  later-created threads, while `/proc/<pid>/task/*/status` proves the same
  inherited filter is present on every observed Cloud Hypervisor thread.
- The thread snapshots are at guest `READY` and again after bidirectional live
  traffic. A thread created later inherits the pre-exec filter from its creator;
  no TSYNC operation is needed because the filter is installed before Cloud
  Hypervisor creates any thread.
- The control's `EINVAL` for empty `TUNATTACHFILTER` and single-queue
  `TUNSETQUEUE` arguments is expected and useful: the filtered `EPERM` occurs
  before those native argument/state checks.
- The policy is intentionally a deny-list. Requests outside the table remain
  allowed. This probe establishes compatibility and closure for the specified
  TAP mutation/filter/queue-attach set, not a complete Cloud Hypervisor syscall
  allow-list.

## Design implications

### ADR-0142

The launcher seccomp alternative is natively viable on the CH v53 `fd=` path
and closes `SIOCSIFHWADDR` before the bridge FDB can be poisoned. The ADR can
record it as an adopted defence layer using the exact deny-list above. Per the
user's ruling, this does not replace the registered-destination-MAC TCX egress
check or audit read-back: those remain independent delivery and detection
layers, and the filter is pinned to a VMM version, launch shape, and
architecture.

### ADR-0130

The filter closes the queue-holder mutations at their source:
`TUNSETOWNER`, `TUNSETGROUP`, `TUNSETPERSIST`, `TUNSETCARRIER`,
`TUNSETDEBUG`, `TUNSETLINK`, and the unused filter/queue-attach controls all
return `EPERM`. Owner uid 0 and descriptor custody remain the attachment
boundary. The audit read-back of host-side MAC, owner, persistence, and TAP
debug level remains useful as independent evidence and damage detection.

### ADR-0129

The safe descriptor-mapping launch boundary must also install this filter in
the child before the final Cloud Hypervisor exec. The native result shows that a
single-thread pre-exec installation plus `no_new_privs` is enough: the filter
survives the exec chain, the CH main thread gains the missing filter, later
threads inherit it, and CH's own filters stack without relaxing it. The fd
mapping/close-range operations and all launcher-required setup must complete in
the order recorded by ADR-0129 before control reaches Cloud Hypervisor; the
filter must be present by that final exec boundary.

## Probe and evidence

- Probe directory:
  `spike-scratch/increment-aa-netns-density-295-tap-ioctl-seccomp-20260924T120003Z/`
- Native probe:
  `spike-scratch/increment-aa-netns-density-295-tap-ioctl-seccomp-20260924T120003Z/spike.py`
- Native probe SHA-256:
  `8c74b7ba9448ac1d035e2198f988dee97032d00e42574386497683cff754b36a`
- Source audit:
  `spike-scratch/increment-aa-netns-density-295-tap-ioctl-seccomp-20260924T120003Z/source_audit.py`
- Source audit SHA-256:
  `02cf5ea159911752f0a9434a7c1c8662ee30f7ceeb918bf526fd4356a98721b7`
- Authoritative source evidence:
  `evidence/00-source-audit-20260924T120929Z.log`
- Authoritative positive control:
  `evidence/04-control-final-20260924T120937Z.log`
- Authoritative filtered run:
  `evidence/05-filtered-final-20260924T120957Z.log`

Preserved superseded/failed captures:

- `00-source-audit-20260924T120809Z.log` and
  `00-source-audit-20260924T120823Z.log`: the source audit's initial `git grep`
  invocation was invalid; no source verdict was produced.
- `00-source-audit-20260924T120857Z.log`: passed its then-stated assertion but
  omitted conditional `SIOCSIFMTU`; source showed that omission, so the premise
  was invalidated and corrected before the authoritative audit.
- `01-control-20260924T120607Z.log`: helper could not traverse the metal checkout
  after uid drop; cleanup completed. The corrected helper inherits an already
  open source fd, without changing checkout permissions.
- `02-control-20260924T120653Z.log` and
  `03-filtered-20260924T120711Z.log`: mechanism runs passed, but were superseded
  after the source inventory was corrected to include conditional
  `SIOCSIFMTU` and the final probe digest changed.

Every native run used foreground `cargo xtask metal run -- ...`; `/usr/bin/script`
captured its complete combined stdout/stderr directly into the uniquely named
evidence file without a truncating pipeline. No file under `crates/` was
created or modified by this probe, and no repository index, commit, design
artifact, delivery artifact, or spike `wave-decisions.md` entry was changed.
