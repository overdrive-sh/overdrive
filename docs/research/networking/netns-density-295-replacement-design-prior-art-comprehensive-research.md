# Research: Prior Art and Primary Sources for the `netns-density-295` Correctness-Recovery Replacement DESIGN (D-295-R1 … R18)

**Date**: 2026-09-23 (completed 2026-09-24) | **Researcher**: nw-researcher (Nova) | **Confidence**: High overall for the kernel/VMM/orchestrator mechanisms; Medium for several comparative judgements (see per-finding ratings) | **Sources**: 43 citation groups (~70 distinct primary URLs/files)

> Scope: evidence for or against the technical claims that decisions D-295-R1 to
> D-295-R18 (feature-delta § "Correctness-Recovery Replacement DESIGN"; ADR-0127 to
> ADR-0139) rest on. This document states agreement or divergence with prior art. It
> makes no design recommendation. D-295-R17 (serve lifetime / SIGTERM) and
> kernel-version differences are out of scope by instruction.

## Executive Summary

Most of the replacement DESIGN's VMM, kernel, and process-lifecycle decisions match established practice.

- **Descriptor handoff (D-1, D-2, D-3).** Handing Cloud Hypervisor a TAP queue descriptor opened by the privileged parent is how libvirt/QEMU, Kata Containers (to Cloud Hypervisor, over `SCM_RIGHTS`), and Android AVF (to crosvm) work. Android AVF does it with the same `command-fds` crate D-3 proposes. Cloud Hypervisor's own source shows the fd path (`from_tap_fds`) `dup`s the descriptor and never raises the link; only the named path calls `tap.enable()`.
- **Kernel TAP semantics.** The kernel's single-queue attach semantics support D-2's attach contract: `EBUSY` on a second attach, feature flags overwritten per attach, carrier on at every attach, admin state untouched at last detach.
- **Admission (D-6, D-8).** The linearization pattern — optimistic placement plus one serialized authority that may refuse — matches the Kubernetes scheduler/kubelet split and Nomad's plan applier.
- **Cleanup (D-10, D-11, D-12).** Convergent, retry-retaining cleanup matches the CNI DEL contract, Cilium's DEL, and Kubernetes finalizers. Flush-and-rebuild of owned state at boot matches kube-proxy full syncs and Cilium's stale-entry deletion.
- **Supervision and audit (D-13, D-15).** Bounded recovery followed by fail-stop has the Erlang/OTP and systemd shape. Periodic audit-and-repair of rules and set members matches Calico Felix.
- **Fail-closure (D-18).** For R18, `ip-rule(8)` confirms the pref-0 `local` rule precedes any added rule, which supports rejecting R18-A. `nft(8)` verdict semantics support an independent drop table. Istio's own bug reports and security guidance show the leak class is real, and that an independent enforcement layer is the recommended defence.

**One decision's premise is contradicted by primary source: D-4 / ADR-0130.**
- **The ADR's premise.** It asserts that with no owner, "the kernel permits a queue attach only to a caller holding `CAP_NET_ADMIN`".
- **What the kernel does.** `tun_not_capable()` in `drivers/net/tun.c` requires `CAP_NET_ADMIN` only when an owner or group is *set* and the caller does not match. With neither set, any process that can open `/dev/net/tun` may attach to an existing, unattached TAP. `/dev/net/tun` is mode 0666 under systemd's default udev rule.
- **Corroboration.** Firecracker's own CI depends on exactly this: jailed uid 1234 opens TAPs created without an owner.
- **Consequence.** Removing the uid-4200 grant *widens* kernel-level attach authority. It does not narrow it. Evidence obligation E4 ("attach as uid 4200 gets `EPERM`") is predicted by source to fail.
- **How prior art isolates TAPs.** By owner uid (crosvm), by fd custody plus non-persistence (libvirt), or by a per-VM network namespace (Firecracker). None of these is present for an ownerless, persistent, host-netns TAP.

**Decisions where prior art is split or silent:**
- **D-7.** Mature orchestrators disagree on whether terminating workloads count. The Kubernetes kubelet counts them until sandbox teardown completes, and calls the exception a bug. Nomad releases them at desired-stop. D-7 lies between the two.
- **D-14.** No surveyed system locally kills every workload on a node when the agent cannot confirm isolation. Cilium stays fail-static, Istio scopes its repair to broken pods, and Kubernetes' node-wide eviction is API-side, delayed 300 s, and rate-limited.
- **D-13's cadence.** The 1 s audit is one to two orders of magnitude tighter than Felix's defaults.
- **Mechanisms with no external precedent.** D-5 (holding the host TAP admin-down across guest boot as the gate) and D-18's guard table (dropping classifier-marked TCP in a separate table) have none. Both are consistent with documented kernel and nftables semantics, and both rest on the repository's own native evidence or on the E14 RED the DESIGN already requires.

## Research Methodology

- **Search strategy.**
  - Primary source first: kernel `tun.c`; Cloud Hypervisor, Firecracker, libvirt, Kata, and AOSP source; Kubernetes, Nomad, Cilium, Calico, and Istio source.
  - Then official documentation: docs.kernel.org, man pages, kubernetes.io, docs.cilium.io, istio.io, developer.hashicorp.com, and AWS.
  - Then crates.io/docs.rs metadata.
  - The user-supplied Cilium checkout at `e99150f8` was grepped locally.
  - Web searches were used only to locate primary artifacts.
- **Source selection.** Types: primary source code, specifications, and official documentation. Reputation: high (1.0), plus a few project-official domains outside the configured trusted list, marked in *Source Analysis*. Verification: each major claim cross-checked against an independent system or against native observations already in the repository, noted per finding.
- **Quality standards.**
  - Target: 3 sources per claim, with a minimum of 1 authoritative primary source.
  - Verbatim quotes where load-bearing.
  - Web-fetched summaries were treated as untrusted. One summarised "fail-open" characterisation contradicted by source was rejected (*Conflict 3*). One `nft(8)` fetch that echoed the prompt was re-fetched with a neutral prompt before use.
  - Interpretations are labelled.

## Claims Under Test (extracted from the DESIGN)

| # | Claim (paraphrased from feature-delta / ADR-0127–0139) | Decision(s) | RQ |
|---|---|---|---|
| C1 | CH accepts one inherited TAP queue fd via `--net fd=[N]`; the fd path does not raise the TAP; one fd ⇒ `num_queues` 2 | D-1, D-2 | RQ1 |
| C2 | The parent opens the queue with `O_CLOEXEC`, maps it to child fd 3, and drops its copy with the `Command` right after `spawn()`; no other descriptor above 2 is inherited | D-2, D-3 | RQ1 |
| C3 | `command-fds` is a suitable, maintained, Apache-2.0 wrapper with tokio support | D-3 | RQ1 |
| C4 | Single-queue attach: a second attach gives `EBUSY`; the attach must request `IFF_VNET_HDR`; the TAP must be down at every attach (carrier on at attach); after the last queue detaches the TAP persists with admin state unchanged | D-2, D-5 | RQ2 |
| C5 | With no owner, only `CAP_NET_ADMIN` callers can attach, so the root launcher is the only attacher; the kernel closes cross-guest attach independently of VMM confinement | D-4 | RQ2 |
| C6 | No guest frame before intercept-live: provision down, activate after the exact success event, then EXEC | D-5 | RQ3 |
| C7 | The address pool `assign`/`replace` is the sole admission linearization point; placement is advisory over a snapshot | D-6, D-8 | RQ4 |
| C8 | Retiring leases (VMM quiescence proven, cleanup pending) do not count toward the admission cap | D-7 | RQ5 |
| C9 | Cleanup must be awaited, convergent (absent members are not errors), and retain ownership on failure; infallible `Drop` is the wrong place | D-10, D-11 | RQ6 |
| C10 | After abrupt process loss, boot must clear dynamic members, rebuild the program, then admit | D-12 | RQ7 |
| C11 | Bounded recovery (20 attempts / 5 s), then fail-stop; 1 s audit period | D-13, D-15 | RQ8 |
| C12 | Unconfirmed quiescence ⇒ kill every workload VMM on the node, then fail-stop | D-14 | RQ8 |
| C13 | Without the IP nft program, intercept-marked TCP is forwarded in cleartext or delivered to host `0.0.0.0` listeners | D-18 | RQ9 |
| C14 | A fwmark blackhole rule cannot stop host-local delivery because pref-0 `lookup local` comes first | D-18 | RQ9 |
| C15 | An independent nft table dropping still-marked TCP after the intercept chain fails closed for both paths | D-18 | RQ9 |
| C16 | Required, constructor-injected intercept and DNS ports are a conventional composition | D-16 | RQ10 |

## Findings

### RQ1 — Passing a pre-opened TAP fd to a VMM (D-1, D-2, D-3)

#### Finding 1.1: Handing a VMM a TAP descriptor opened by a privileged parent is established practice in QEMU/libvirt, Kata, Android AVF/crosvm and Cloud Hypervisor. Firecracker is the exception.
**Evidence**:
- QEMU: `-netdev tap,fd=h`: "Connect to an already opened TAP file descriptor h." `fds=x:y:...:z` passes one pre-opened descriptor per queue.
- libvirt (`virNetDevTapCreate`, Linux) opens `/dev/net/tun` in the root daemon with `IFF_TAP | IFF_NO_PI`. It adds `IFF_MULTI_QUEUE` when `tapfdSize > 1`, and `IFF_VNET_HDR` under `VIR_NETDEV_TAP_CREATE_VNET_HDR`. It then hands the descriptors to the QEMU child through `virCommandPassFD`.
- Kata Containers (`clh.go`) keeps `netDevicesFiles map[string][]*os.File`, filled from `netPair.VMFds`. It sends them to Cloud Hypervisor as `SCM_RIGHTS` ancillary data on an HTTP `PUT /api/v1/vm.add-net` (`oob := syscall.UnixRights(fds...)`; `conn.WriteMsgUnix(...)`) during `bootVM()`, before the VM runs.
- Android AVF `virtmgr` (Rust, `android/virtmgr/src/crosvm.rs`) uses `use command_fds::CommandFdExt;` and `command.preserved_fds(preserved_fds);`. It passes the TAP to crosvm as `command.arg("--net").arg(format!("tap-fd={tap_fd}"))`.
- Cloud Hypervisor documents `--net fd=3`, "which the shell has opened to point to the /dev/tapN device" (macvtap example, shell redirection `3<>"$tapdevice"`). The `--net` grammar is `fd=<[fd1,fd2,...]>`.
- **Firecracker does not accept a descriptor.** `tap.rs` always opens `/dev/net/tun` itself (`O_RDWR | O_NONBLOCK | O_CLOEXEC`) and calls `TUNSETIFF` by name with `IFF_TAP | IFF_NO_PI | IFF_VNET_HDR`. The jailer "Close[s] all open file descriptors … except input, output and error", `mknod`s `/dev/net/tun` inside the chroot, joins `--netns`, and drops to the configured uid/gid.

**Sources**: [QEMU invocation docs](https://www.qemu.org/docs/master/system/invocation.html); [libvirt virnetdevtap.c](https://github.com/libvirt/libvirt/blob/master/src/util/virnetdevtap.c); [Kata clh.go](https://github.com/kata-containers/kata-containers/blob/main/src/runtime/virtcontainers/clh.go); [AOSP virtmgr crosvm.rs](https://android.googlesource.com/platform/packages/modules/Virtualization/+/refs/heads/main/android/virtmgr/src/crosvm.rs); [CH macvtap-bridge.md](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/macvtap-bridge.md); [CH config.rs](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/vmm/src/config.rs); [Firecracker tap.rs](https://github.com/firecracker-microvm/firecracker/blob/main/src/vmm/src/devices/virtio/net/tap.rs); [Firecracker jailer.md](https://github.com/firecracker-microvm/firecracker/blob/main/docs/jailer.md). All accessed 2026-09-23.
**Confidence**: High (five independent systems, all primary source).
**Bears on**: D-1 (matches the libvirt, Kata, AVF, and QEMU shape), D-2.
**Analysis**: Where the privileged side keeps TAP authority, the parent opens the device and the VMM inherits it. Firecracker takes the other route: the VMM opens the TAP by name itself, and isolation comes from a per-VM netns plus the jailer.

#### Finding 1.2: In Cloud Hypervisor, the fd path and the named path are different code paths. Only the named path raises the TAP.
**Evidence**:
- `virtio-devices/src/net.rs` `from_tap_fds` calls `libc::dup(*fd)` with the comment "Duplicate so that it can survive reboots". It sets only the MTU; no `enable` or `IFF_UP` appears in the constructor. Offloads are applied later, in `activate`.
- `net_util/src/open_tap.rs` `open_tap_rx_q_0` calls `tap.enable().map_err(Error::TapEnable)?` and `tap.set_vnet_hdr_size(...)`.
- `config.rs` validation reads: `let actual_queues = fds.len() * 2; if actual_queues != self.num_queues { return Err(ValidationError::VnetQueueFdMismatch(...)) }`.

**Sources**: [CH net.rs](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/virtio-devices/src/net.rs); [CH open_tap.rs](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/net_util/src/open_tap.rs); [CH config.rs](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/vmm/src/config.rs). Accessed 2026-09-23, `main` branch.
**Cross-reference**: The repository's own native spikes (`.context/netns-density-295-fd-tap-spike-findings.md`; `spike/findings-persistent-fd-tap.md`) observed the same behaviour at v53.0: the TAP stayed down through READY, and CH held an inherited descriptor after the parent closed its copy.
**Confidence**: High for `main`. The source reading is on `main`, not the v53.0 tag; the spike cites v53.0 line links for the same functions.
**Bears on**: D-1 and D-2 (supports "CH does not raise an fd-supplied TAP" and "one fd ⇒ `num_queues` 2").
**Analysis**: Because CH `dup`s the descriptor, the parent's copy can close at any time after the child has exec'd and CH has constructed the device. D-2 closes the parent copy after `spawn()` returns. CH `dup`s during device construction, after exec, so the parent can close early only if the child's inherited copy stays open until then. It does, because it is not CLOEXEC in the child.

#### Finding 1.3: When the parent closes its copy: libvirt closes after spawn, command-fds when the `Command` drops, and Kata's close point was not found.
**Evidence**:
- libvirt `virCommandPassFD` doc: "Transfer the specified file descriptor to the child, instead of closing it on exec … If the flag VIR_COMMAND_PASS_FD_CLOSE_PARENT is set then fd will be closed in the parent no later than Run/RunAsync/Free."
- `command-fds` `FdMapping { pub parent_fd: OwnedFd, pub child_fd: RawFd }` takes ownership of the parent fd. The `OwnedFd`s are moved into the `pre_exec` closure and live until the `Command` is dropped.
- Kata `clh.go`: no `.Close()` on `netDevicesFiles` entries was found in `clh.go`, and `cleanupVM()` does not close them. This is an **unverified gap**. The close may happen elsewhere in Kata's network code, which was not read.

**Sources**: [libvirt vircommand.c](https://github.com/libvirt/libvirt/blob/master/src/util/vircommand.c); [command-fds src/lib.rs](https://github.com/google/command-fds/blob/main/src/lib.rs); [Kata clh.go](https://github.com/kata-containers/kata-containers/blob/main/src/runtime/virtcontainers/clh.go). Accessed 2026-09-23.
**Confidence**: High for libvirt and command-fds; Low for Kata.
**Bears on**: D-2. Its "the `Command` is dropped immediately after `spawn()` … the parent's copy lives exactly as long as the `Command` value" matches command-fds' documented ownership model and libvirt's CLOSE_PARENT semantics.

#### Finding 1.4: Landing the descriptor at a fixed number, and the concurrent-spawn leak
**Evidence**:
- **libvirt.** In the child after `fork`, passed fds are made inheritable with `virSetInherit(fd, true)`, which clears `FD_CLOEXEC`. `virCommandMassClose()` (`close_range` or an iterative fallback) then closes every other descriptor.
- **command-fds.** `pre_exec` uses `dup2`: "This closes child_fd if it is already open as something else, and clears the FD_CLOEXEC flag on child_fd". Parent/child number collisions are first moved aside with `F_DUPFD_CLOEXEC`. The hook "will not allocate, so it is safe to call from this hook". It **does not close other file descriptors**.
- **Rust std** `CommandExt::pre_exec`: the closure runs "in the context of the child process after a `fork`", where malloc and mutexes "are not guaranteed to work". std also notes that `CLOEXEC` is "set by default on all file descriptors opened by the standard library".

**Sources**: [libvirt vircommand.c](https://github.com/libvirt/libvirt/blob/master/src/util/vircommand.c); [command-fds lib.rs](https://github.com/google/command-fds/blob/main/src/lib.rs); [Rust std CommandExt](https://doc.rust-lang.org/std/os/unix/process/trait.CommandExt.html). Accessed 2026-09-23.
**Confidence**: High.
**Bears on**: D-2 and D-3.
**Analysis (interpretation, labelled)**:
- **What prevents the leak.** A queue descriptor opened `O_CLOEXEC` in the parent and `dup2`'d to fd 3 only inside the forked child cannot leak into a concurrently forked sibling: the parent's copy stays CLOEXEC, and only the child's fd 3 has CLOEXEC cleared. D-2 opens with `O_CLOEXEC`, and D-3 uses a `dup2` mapping. **This matches libvirt's model** (open CLOEXEC, clear it in the child only).
- **Where D-2 diverges.** D-2 asserts "No other descriptor above 2 is inherited". command-fds does not enforce that. It holds only if every other descriptor in the `overdrive serve` process is CLOEXEC: std and tokio descriptors are, but descriptors created by raw syscalls or FFI libraries are not guaranteed to be. libvirt closes every other descriptor in the child (`virCommandMassClose`), and Firecracker's jailer closes all non-stdio descriptors. D-3 as written has no such mass-close. The E2 native check (only CH holds `iff:<tap>`; a concurrently launched second VM holds none) observes the property but does not enforce it.

#### Finding 1.5: `command-fds`: maintenance, license, adoption, and use in Rust VMM tooling
**Evidence** (crates.io API and GitHub):
- **Identity.** Repository `github.com/google/command-fds` (Google-owned). License `Apache-2.0`.
- **Maintenance.** Newest version `0.3.3`, published 2026-04-10. The repository showed 107 commits.
- **Adoption.** 5,106,419 total downloads (2,124,989 recent). 41 crates.io reverse dependencies, including `zed-util`, `brush-core`, `gel-tokio`, and `cageforge-linux`.
- **Tokio support.** The optional `tokio` feature exists (`tokio = ["dep:tokio"]`).
- **Rust VMM use.** AOSP's `virtmgr` is not on crates.io but uses `command_fds::CommandFdExt` to pass a TAP fd to crosvm. The Firecracker jailer, also Rust, uses no mapping crate: it closes all non-stdio descriptors, because Firecracker opens the TAP itself.

**Sources**: [crates.io command-fds](https://crates.io/crates/command-fds) (API `/api/v1/crates/command-fds` and `/reverse_dependencies`); [GitHub google/command-fds](https://github.com/google/command-fds); [AOSP virtmgr crosvm.rs](https://android.googlesource.com/platform/packages/modules/Virtualization/+/refs/heads/main/android/virtmgr/src/crosvm.rs). Accessed 2026-09-23.
**Confidence**: High for the metadata. The repository page showed 52 GitHub stars.
**Bears on**: D-3. The design's "license … Apache-2.0; this is to be confirmed" is **confirmed** by crates.io metadata. "Support for `tokio::process::Command` through its `tokio` feature": the feature exists; that it targets `tokio::process::Command` specifically was not read from source (**unverified detail**). The same Google team's Android VMM launcher using it for exactly a TAP-fd handoff is direct precedent.

### RQ2 — Persistent TAP lifecycle and flags (D-2, D-4, D-5)

#### Finding 2.1: Single-queue attach semantics, sticky feature flags, carrier on attach, and last-detach behaviour (kernel `drivers/net/tun.c`)
**Evidence** (verbatim from `torvalds/linux` `master`):
- **Second attach.** `tun_attach`: `err = -EBUSY; if (!(tun->flags & IFF_MULTI_QUEUE) && tun->numqueues == 1) goto out;`
- **Multiqueue mismatch.** `tun_set_iff`, existing device: `if (!!(ifr->ifr_flags & IFF_MULTI_QUEUE) != !!(tun->flags & IFF_MULTI_QUEUE)) return -EINVAL;`
- **Sticky flags.** After a successful attach: `WRITE_ONCE(tun->flags, (tun->flags & ~TUN_FEATURES) | (ifr->ifr_flags & TUN_FEATURES));` with `#define TUN_FEATURES (IFF_NO_PI | IFF_ONE_QUEUE | IFF_VNET_HDR | IFF_MULTI_QUEUE | IFF_NAPI | IFF_NAPI_FRAGS | IFF_BACKPRESSURE)`. So each single-queue attacher overwrites `IFF_VNET_HDR`.
- **Carrier.** `tun_set_iff`: `if (ifr->ifr_flags & IFF_NO_CARRIER) netif_carrier_off(tun->dev); else netif_carrier_on(tun->dev);`. A `TUNSETCARRIER` ioctl also exists (`tun_net_change_carrier(tun->dev, (bool)carrier)`).
- **Last detach.** `__tun_detach`: when `tun->numqueues == 0 && tun->numdisabled == 0`, it calls `netif_carrier_off(tun->dev)`, and `unregister_netdevice` only `if (!(tun->flags & IFF_PERSIST) …)`. Nothing there changes administrative (`IFF_UP`) state.
- **Owner ioctls.** `TUNSETPERSIST` toggles `IFF_PERSIST`. `TUNSETOWNER` and `TUNSETGROUP` set `tun->owner` and `tun->group`, rejecting invalid ids with `-EINVAL`.
- **Vnet-header size.** `TUNSETVNETHDRSZ` is delegated to `tun_vnet_ioctl(&tun->vnet_hdr_sz, &tun->flags, cmd, argp)`, so the size is per device, like the flag.

**Sources**: [linux drivers/net/tun.c](https://github.com/torvalds/linux/blob/master/drivers/net/tun.c) (primary); [kernel TUN/TAP doc](https://docs.kernel.org/networking/tuntap.html) (multiqueue: "multiple file descriptors (queues)", `IFF_MULTI_QUEUE`, `TUNSETQUEUE`). Accessed 2026-09-23. Kernel-version differences are out of scope.
**Cross-reference**: The repository's increment-y native run observed each of these: `EBUSY` on a second attach, `IFF_VNET_HDR` tracking the last attacher, `LOWER_UP` on reattaching an admin-up TAP, and NO-CARRIER with admin `UP` retained after VMM exit.
**Confidence**: High (primary source plus independent native observation).
**Bears on**: D-2 ("exactly `IFF_TAP|IFF_NO_PI|IFF_VNET_HDR`, never `IFF_MULTI_QUEUE`"; `EBUSY` → `Attach`; read-back `0x5802`), and D-2/D-5 ("TAP down at every attach", the teardown step "TAP down and read-back").
**Analysis**: D-2's attach contract is consistent with kernel semantics. Two kernel facts bear on it:
- **Carrier alternatives.** The carrier-on-at-attach hazard behind "down at every attach" can also be controlled with `IFF_NO_CARRIER` at attach, or with `TUNSETCARRIER`. The DESIGN does not mention either. This is noted as an alternative the kernel offers, not as a recommendation.
- **Vnet-header size.** The spike observed CH's `TUNSETVNETHDRSZ(12)` persisting after exit. Because the size is per device, a future attacher that does not set it inherits the prior size.

#### Finding 2.2: An ownerless persistent TAP is attachable by any uid that can open `/dev/net/tun`. This CONTRADICTS ADR-0130 / D-4's stated premise.
**Evidence** (verbatim, `drivers/net/tun.c`):
```c
static inline bool tun_not_capable(struct tun_struct *tun)
{
	const struct cred *cred = current_cred();
	struct net *net = dev_net(tun->dev);

	return ((uid_valid(tun->owner) && !uid_eq(cred->euid, tun->owner)) ||
		(gid_valid(tun->group) && !in_egroup_p(tun->group))) &&
		!ns_capable(net->user_ns, CAP_NET_ADMIN);
}
```
- A new device is initialised with `tun->owner = INVALID_UID; tun->group = INVALID_GID;`.
- Attaching to an existing device is refused only `if (tun_not_capable(tun)) return -EPERM;`, followed by the LSM hook `security_tun_dev_open`. With both owner and group invalid, the first operand is false, so `tun_not_capable` returns **false** for every caller: the attach is permitted without `CAP_NET_ADMIN`.
- Only *creating* a device checks `ns_capable(net->user_ns, CAP_NET_ADMIN)` unconditionally.
- The existing-device lookup is `__dev_get_by_name(net, ifr->ifr_name)`, so it is scoped to the caller's network namespace.

**Corroboration from practice**:
- Firecracker's CI creates TAPs with `ip tuntap add mode tap name {name}`, with no `user`/`group`, inside a netns (`tests/host_tools/network.py`).
- It runs Firecracker under the jailer as `uid=1234, gid=1234` (`tests/framework/jailer.py`). Firecracker then opens the existing TAP by name without `CAP_NET_ADMIN`.
- If an ownerless TAP required `CAP_NET_ADMIN`, that configuration could not work.

**Device-node permission**: systemd's default udev rule is `KERNEL=="tun", MODE="0666", OPTIONS+="static_node=net/tun"` (`rules.d/50-udev-default.rules.in`), so on a stock systemd host an unprivileged uid can open `/dev/net/tun`. Whether the Overdrive appliance image keeps that mode was not checked (**unverified**).
**Conflicting wording**: The kernel TUN/TAP document says "CAP_NET_ADMIN is required for creating network devices or for connecting to network devices which aren't owned by the user in question." Read literally, that implies an ownerless device needs `CAP_NET_ADMIN`. The code above is the authoritative behaviour; see *Conflicting Information*.
**Sources**: [linux tun.c](https://github.com/torvalds/linux/blob/master/drivers/net/tun.c); [Firecracker tests/host_tools/network.py](https://github.com/firecracker-microvm/firecracker/blob/main/tests/host_tools/network.py); [Firecracker tests/framework/jailer.py](https://github.com/firecracker-microvm/firecracker/blob/main/tests/framework/jailer.py); [kernel TUN/TAP doc](https://docs.kernel.org/networking/tuntap.html). Accessed 2026-09-23.
**Confidence**: High (primary source, corroborated by an independent production project's CI).
**Bears on**: **D-4 / ADR-0130 — divergence.** The ADR states: "With no owner, the kernel permits a queue attach only to a caller holding `CAP_NET_ADMIN`, so the root launcher is the only attacher" and "The kernel closes cross-guest queue attaches independently of the VMM's confinement." Kernel source contradicts both statements.
- With no owner and no group, any uid, including the VMM uid 4200, can attach to an unattached TAP it can name. Every #295 guest TAP is in the host netns, so every VMM can name every TAP.
- What remains: DAC on `/dev/net/tun`, LSM `security_tun_dev_open`, and CH's own Landlock and seccomp.
- The evidence-lane prediction E4 ("An attach attempt as uid 4200 gets `EPERM`") is therefore predicted by source to **fail**. No native run has tested it; ADR-0130 itself says "no spike has tested it".
- Kernel semantics give an owner or group that no VMM process holds (for example uid 0) the "only privileged callers attach" property. That is stated as a kernel fact, not a recommendation.

#### Finding 2.3: How established systems choose between an owner uid, a pre-opened fd, or both
| System | Who opens the TAP queue | Owner/group set? | Persistent? | Isolation of the TAP from other guests |
|---|---|---|---|---|
| libvirt (system QEMU) | root `libvirtd` opens it and passes the fd (`virCommandPassFD`) | **No** `TUNSETOWNER`/`TUNSETGROUP` call in `virNetDevTapCreate` | Only with `VIR_NETDEV_TAP_CREATE_PERSIST` | The QEMU child holds the fd; the TAP is not persistent by default, so it dies with its last fd. (sVirt/LSM labelling is not verified here.) |
| Kata Containers + CH | the runtime opens it and sends it via `SCM_RIGHTS` | not verified | not verified | per-sandbox netns (general Kata design; not re-verified here) |
| AVF virtmgr + crosvm | virtmgr opens it and passes it (`command-fds`, `tap-fd=`) | not verified | not verified | not verified |
| crosvm (standalone book) | crosvm opens by name (`tap-name=`) | **Yes**: `ip tuntap add mode tap user $USER vnet_hdr crosvm_tap`. The book adds that crosvm lacks `CAP_NET_ADMIN` under its sandbox, so "hotplug only accepts a persistent TAP device owned by the user running crosvm" | Yes | owner uid |
| Firecracker | Firecracker opens by name after the jailer drops privileges | **No** (CI) | Created by `ip tuntap`, so persistent | **per-VM netns** (`--netns`); `__dev_get_by_name` is per-netns, so other VMs cannot name the TAP |

**Sources**: as in Findings 1.1 and 2.2, plus [crosvm book: network](https://crosvm.dev/book/devices/net.html). Accessed 2026-09-23.
**Confidence**: Medium overall (several cells unverified).
**Bears on**: D-4, D-2.
**Analysis**: No surveyed system relies on "ownerless ⇒ root-only". Each system gets TAP isolation in one of three ways:
- by owner uid (crosvm);
- by non-persistence plus fd custody (libvirt);
- by per-VM netns (Firecracker).

#295 places every TAP in the host netns and makes it persistent (ADR-0114). None of the three mechanisms applies to an ownerless TAP in that topology.

### RQ3 — Network down until policy is enforced (D-5, gate ordering)

#### Finding 3.1: Container orchestration sets networking and enforcement up *before* the workload process starts. "Ready" means the endpoint's datapath and policy are programmed.
**Evidence**:
- **Kubernetes kubelet** `SyncPod` steps: "4. Create sandbox if necessary. 5. Invoke OnPodSandboxReady … 8. Create init containers. 9. Create normal containers." Sandbox creation, the CRI `RunPodSandbox` step that runs the CNI ADD, precedes every workload container.
- **CNI spec**: "The container runtime must create a new network namespace for the container before invoking any plugins." The spec does *not* explicitly require ADD to complete before the container runs; Kubernetes gets that ordering from the kubelet sequence above.
- **Cilium CNI ADD** (`plugins/cilium-cni/cmd/cmd.go`): "// Specify that endpoint must be regenerated synchronously. See GH-4409. `ep.SyncBuildEndpoint = true`". A failed `EndpointCreate` returns `unable to create endpoint: %w`, which fails the ADD.
- **Cilium endpoint states**: `regenerating`: "The endpoint's networking configuration is being (re)generated. This includes programming eBPF for that endpoint." `ready`: "…successfully (re)generated." Before labels are known the endpoint carries `reserved:init`. With no policy selecting it, "if the policy enforcement mode is `never` or `default`, all ingress (resp. egress) traffic is allowed … Otherwise, all ingress (resp. egress) traffic is dropped."

**Sources**: [kubelet kuberuntime_manager.go](https://github.com/kubernetes/kubernetes/blob/master/pkg/kubelet/kuberuntime/kuberuntime_manager.go); [CNI SPEC.md](https://github.com/containernetworking/cni/blob/main/SPEC.md); [Cilium cilium-cni cmd.go](https://github.com/cilium/cilium/blob/main/plugins/cilium-cni/cmd/cmd.go); [Cilium Endpoint Lifecycle](https://docs.cilium.io/en/stable/security/policy/lifecycle/). Accessed 2026-09-23.
**Confidence**: High.
**Bears on**: D-5 and the lifecycle gate ordering.

#### Finding 3.2: Service meshes gate pod start on redirection being in place, and fail closed when it cannot be established.
**Evidence**:
- **Istio ambient CNI plugin** (`cni/pkg/plugin/plugin.go`) pushes the add event to the node agent over its UDS (`PushCNIEvent(...)`). On failure it returns `"istio-cni cmdAdd failed to contact node Istio CNI agent: %s"`, which fails pod creation. The node agent "enters the pod's network namespace and establishes network redirection rules", including mangle `TPROXY --on-port 15008 … --tproxy-mark 0x111/0xfff`. It then tells ztunnel to open that pod's listen ports.
- **Istio sidecar mode**: "an `istio-validation` init container is added as part of the sidecar injection, which detects if traffic redirection is set up correctly, and blocks the pod starting up if not". The DaemonSet can then delete, label, or repair such pods.
- **Linkerd** (2.20 docs): the proxy runs as a native sidecar "starting before the application's initContainers". With native sidecars disabled, init containers get no network because "its packets will be caught by iptables and the linkerd-proxy will not yet be available". Linkerd's network-validator init container was **not found** in the fetched page (gap).

**Sources**: [Istio plugin.go](https://github.com/istio/istio/blob/master/cni/pkg/plugin/plugin.go); [Istio ambient traffic redirection](https://istio.io/latest/docs/ambient/architecture/traffic-redirection/); [Istio CNI setup, race condition & mitigation](https://istio.io/latest/docs/setup/additional-setup/cni/); [Linkerd CNI](https://linkerd.io/2-edge/features/cni/). Accessed 2026-09-23.
**Confidence**: Medium-High. Istio is verified from source and docs. One fetched summary characterised ambient as "fail-open", but source contradicts that for the ADD path; the source is used.
**Bears on**: D-5.

#### Finding 3.3: VMM network bring-up attaches the NIC before boot. None of the surveyed systems holds a host-side link admin-down across guest boot as its enforcement gate.
**Evidence**:
- Kata attaches the TAP to CH through `vm.add-net` during `bootVM()`, before the VM runs (Finding 1.1).
- libvirt brings the TAP online at creation based on the `VIR_NETDEV_TAP_CREATE_IFUP` flag (`virNetDevSetOnline(*ifname, !!(flags & VIR_NETDEV_TAP_CREATE_IFUP))`).
- Firecracker documentation brings the TAP up at host setup (`sudo ip link set tap0 up`, network-setup.md).

**Sources**: as in Findings 1.1 and 2.3; [Firecracker network-setup.md](https://github.com/firecracker-microvm/firecracker/blob/main/docs/network-setup.md). Accessed 2026-09-23.
**Confidence**: Medium. A negative result across the systems read; not an exhaustive survey.
**Bears on**: D-5.
**Analysis (interpretation, labelled)**:
- **Principle — matches.** "No workload traffic before enforcement is live; ready means the enforcement program is installed and read back" matches Cilium's `SyncBuildEndpoint`, Istio ambient's fail-closed ADD, and Istio's validation init container. D-5's gate (activate only after the exact `mtls.intercept.install.success` event, before EXEC) is the same shape of promise.
- **Mechanism — no precedent found.** The prior art orders *enforcement before workload start*. D-5 orders *guest boot → enforcement → link up → workload command release*, keeping the host TAP admin-down in between.
- The kernel facts that make admin-down an effective gate are in RQ2: carrier and administrative state are separate; CH's fd path never sets `IFF_UP`. This mechanism is supported by the repository's own native evidence (the spikes and `c4d36190`/`f1a15668`), not by external precedent.

### RQ4 — Linearizing capacity admission (D-6, D-7, D-8)

#### Finding 4.1: Kubernetes has an optimistic scheduler whose placement is advisory, and an authoritative node-local admission step.
**Evidence**:
- **Scheduler.** The scheduler cache doc reads: "AssumePod assumes a pod scheduled and aggregates the pod's information into its node". Its state machine is `Initial → Assumed → Added`, with `Forget` as "an undo operation for AssumePod". Pod events are not guaranteed: "We don't have guaranteed delivery of all events".
- **Kubelet admission.** On the node, `predicateAdmitHandler.Admit()` rebuilds a `NodeInfo` from `attrs.OtherPods` and runs the scheduler's `AdmissionCheck`. It rejects with `OutOfcpu`, `OutOfmemory`, `OutOfephemeral-storage`, or `OutOfpods`.
- The fetched source does not document *why* the kubelet re-checks (**unverified rationale**). The code shows that it does.

**Sources**: [scheduler cache interface.go](https://github.com/kubernetes/kubernetes/blob/master/pkg/scheduler/backend/cache/interface.go); [kubelet lifecycle/predicate.go](https://github.com/kubernetes/kubernetes/blob/master/pkg/kubelet/lifecycle/predicate.go). Accessed 2026-09-23.
**Confidence**: High (primary source) for the mechanism.
**Bears on**: D-8 (placement advisory, authority at the node-local gate) and D-6.

#### Finding 4.2: Nomad runs schedulers optimistically in parallel and linearizes at the leader's plan applier, which re-verifies fit against current state.
**Evidence**:
- **Optimistic scheduling.** "Multiple schedulers are running in parallel without locking or reservations, making Nomad optimistically concurrent. As a result, schedulers might overlap work on the same node and cause resource over-subscription."
- **Serialized plans.** "The plan queue allows the leader node to protect against this and do partial or complete rejections of a plan." The plan result lets the scheduler "terminate or explore alternate plans".
- **The fit check.** In `plan_apply.go`, `evaluateNodePlan` fetches `snap.AllocsByNodeTerminal(ws, nodeID, false)` (non-terminal allocs). It removes the plan's `NodeUpdate` and `NodePreemptions`, adds `NodeAllocation`, and calls `structs.AllocsFit(node, proposed, nil, true)`.
- **Pipelining.** Plans are pipelined: "Multiple plans (roughly 6-8) can be in-flight in the Raft pipeline", over an optimistic state view.

**Sources**: [Nomad scheduling concepts](https://developer.hashicorp.com/nomad/docs/concepts/scheduling/scheduling); [nomad/plan_apply.go](https://github.com/hashicorp/nomad/blob/main/nomad/plan_apply.go). Accessed 2026-09-23.
**Confidence**: High.
**Bears on**: D-6 and D-8. Both match the pattern: advisory placement, plus one serialized authority that can refuse, plus a retry.

#### Finding 4.3: In Kubernetes/AWS, IP address allocation is *not* the authoritative capacity gate. A pod-count proxy (`maxPods`) is, and IP exhaustion surfaces after placement.
**Evidence**:
- **The count.** The EKS default `maxPods` is "equivalent to the formula `(number of ENIs × (IPs per ENI − 1)) + 2`". Its precedence can be overridden by the managed-node-group caps (110/250) or by kubelet config.
- **Exhaustion.** VPC CNI troubleshooting: "If a subnet runs out of IP addresses, ipamD will not able to get secondary IP addresses. When this happens, pods assigned to this node may not able to get an IP and get stuck in **ContainerCreating**."

**Sources**: [EKS: choosing instance type / How maxPods is determined](https://docs.aws.amazon.com/eks/latest/userguide/choosing-instance-type.html); [amazon-vpc-cni-k8s troubleshooting.md](https://github.com/aws/amazon-vpc-cni-k8s/blob/master/docs/troubleshooting.md). Accessed 2026-09-23.
**Calico/Cilium IPAM exhaustion**: not researched in depth (budget). **Unverified** beyond the general statement that CNI ADD fails on IPAM exhaustion (Cilium's ADD error path in Finding 3.1 returns an error on endpoint-create failure).
**Confidence**: Medium (AWS only, two AWS documents).
**Bears on**: D-6.
**Analysis (interpretation)**:
- **What D-6 fuses.** D-6 makes `GuestAddressPool::assign` the linearization point, with the cap counted as a *lease count* (admitted), separately from address capacity (`held` vs `capacity`). That fuses an admission counter with the address allocator under one lock.
- **Where the prior art puts authority.** Kubernetes keeps the count gate (`maxPods`) in kubelet admission and IPAM in the CNI, and the two can disagree (the ContainerCreating failures above).
- **Verdict.** No surveyed system uses the address allocator itself as the authoritative *count* gate. The underlying principle — one serialized node-local authority that may refuse what advisory placement allowed — matches Kubernetes kubelet admission and the Nomad plan applier. D-6's co-location of the two counters is a design choice without a direct precedent found. The prior art does not refute it.

### RQ5 — Do terminating/retiring workloads count toward capacity? (D-7)

#### Finding 5.1: The Kubernetes kubelet counts terminating (not-yet-terminated) pods in admission, and treats the one exception as a known bug.
**Evidence**:
- **`GetActivePods` doc.** "returns pods that have been admitted to the kubelet that are not fully terminated … WARNING: Currently this list does not include pods that have been force deleted but may still be terminating, which means resources assigned to those pods during admission may still be in use. See https://github.com/kubernetes/kubernetes/issues/104824".
- **PR #104577** ("kubelet: Admission must exclude completed pods and avoid races") added `IsPodKnownTerminated()`, which "returns true only if the pod is in a known terminated state (no running containers AND known to pod worker)". The PR states: "This commit does not fix the long standing bug that force deleted pods are omitted from admission checks, which must be fixed by having GetActivePods() also include pods 'still terminating'."
- **Termination includes sandbox teardown.** `killPodWithSyncResult` stops every sandbox (`StopPodSandbox`), which is where CNI teardown happens. It records a failure as `killSandboxResult.Fail(kubecontainer.ErrKillPodSandbox, …)`. The pod then stays terminating and is retried.

**Sources**: [kubelet_pods.go](https://github.com/kubernetes/kubernetes/blob/master/pkg/kubelet/kubelet_pods.go); [PR #104577](https://github.com/kubernetes/kubernetes/pull/104577); [kuberuntime_manager.go](https://github.com/kubernetes/kubernetes/blob/master/pkg/kubelet/kuberuntime/kuberuntime_manager.go). Accessed 2026-09-23.
**Confidence**: High (source plus a maintainer-authored PR).

#### Finding 5.2: Nomad does *not* count allocations whose desired status is stop or evict when fitting new plans, even while they still run. A user-visible port-collision issue is on record.
**Evidence**:
- `TerminalStatus()` "returns if the desired or actual status is terminal": `return a.ServerTerminalStatus() || a.ClientTerminalStatus()`.
- `ServerTerminalStatus()` is true for `AllocDesiredStatusStop, AllocDesiredStatusEvict`.
- The state store's alloc `node` index is a `memdb.ConditionalIndex` on `alloc.TerminalStatus()`. The plan applier reads `AllocsByNodeTerminal(ws, nodeID, false)`, so a desired-stop alloc still running on the client is excluded from `AllocsFit`.
- GH #1225, "Nomad stop exits before allocations have exited", records a port collision when a new run starts while the old container is still running.

**Sources**: [nomad/structs/alloc.go](https://github.com/hashicorp/nomad/blob/main/nomad/structs/alloc.go); [nomad/state/schema.go](https://github.com/hashicorp/nomad/blob/main/nomad/state/schema.go); [nomad/plan_apply.go](https://github.com/hashicorp/nomad/blob/main/nomad/plan_apply.go); [nomad GH #1225](https://github.com/hashicorp/nomad/issues/1225). Accessed 2026-09-23.
**Confidence**: High for the mechanism. Medium for the linkage to #1225: the issue was seen in search results only, and its body was not read.

#### Finding 5.3: Replacement at the cap: surge vs recreate, and the upstream Kubernetes move toward waiting for full termination
**Evidence**:
- **Deployment `Recreate`**: "All existing Pods are killed before new ones are created". `RollingUpdate`'s `maxSurge` permits exceeding the desired count.
- **KEP-3939 (Jobs).** "Currently, Jobs start replacement Pods as soon as previously created Pods are terminating (have a `deletionTimestamp`) … This KEP proposes a new field … only once the existing pods are fully terminated." `podReplacementPolicy: Failed` means "Wait until a previously created Pod is fully terminated before creating a replacement Pod".
- **KEP-3973 (Deployments).** Terminating pods not counted leads to "Unnecessary autoscaling of nodes in tight environments" and "pods are fighting over resources". It adds `.status.terminatingReplicas`.

**Sources**: [Kubernetes Deployment docs](https://kubernetes.io/docs/concepts/workloads/controllers/deployment/); [KEP-3939](https://github.com/kubernetes/enhancements/blob/master/keps/sig-apps/3939-allow-replacement-when-fully-terminated/README.md); [KEP-3973](https://github.com/kubernetes/enhancements/blob/master/keps/sig-apps/3973-consider-terminating-pods-deployment/README.md). Accessed 2026-09-23.
**Confidence**: High.

**Analysis for D-7 (interpretation, labelled)**:
- **Prior art conflicts.** Kubernetes counts terminating workloads until they are fully terminated, including sandbox/network teardown, and calls the exception a bug. Nomad releases at desired-stop, before the task stops.
- **Where D-7 sits.** D-7 retires a lease only *after* VMM quiescence is proven (`driver.stop` Ok/NotFound). It then excludes Retiring leases from the 16,384 cap while still counting them in `held` against address capacity.
  - That is stricter than Nomad: compute is proven gone.
  - It is looser than the Kubernetes kubelet: a Retiring lease whose network or intercept cleanup is still pending or failing, with its TAP and nft members still in the kernel, no longer counts toward the cap. In Kubernetes, a pod whose `StopPodSandbox` fails remains terminating and counted.
- **Which one applies.** Whether D-7 matches or diverges depends on what resource the 16,384 cap protects. If it protects VMM/compute density, D-7 aligns with Kubernetes' "no running containers" notion. If it protects dataplane state (TAP count, TCX attachments, nft membership), Retiring residue still consumes it, and D-7 diverges from the Kubernetes approach.
- **Replace at the cap.** `replace` (the predecessor becomes Retiring atomically with the successor's admission) resembles the Job default `TerminatingOrFailed` (replace once the predecessor is terminating), not `podReplacementPolicy: Failed`. The difference is that D-6 requires the predecessor already Failed/Terminated, with its VMM stopped.

### RQ6 — Fallible cleanup that keeps ownership until it succeeds (D-10, D-11)

#### Finding 6.1: Kubernetes finalizers keep the object, and its ownership, until the cleanup controller succeeds
**Evidence**: When an object with finalizers is deleted, the API server sets `metadata.deletionTimestamp` and "Prevents the object from being removed until all items are removed from `metadata.finalizers`". The controller "attempts to satisfy the requirements" and removes each key once satisfied. If cleanup never succeeds, the object stays terminating. Once deletion is requested, "the object cannot be resurrected".
**Source**: [Kubernetes Finalizers](https://kubernetes.io/docs/concepts/overview/working-with-objects/finalizers/). Accessed 2026-09-23.
**Cross-reference**: The kubelet keeps a pod terminating (and counted, Finding 5.1) while `StopPodSandbox` fails.
**Confidence**: High.
**Bears on**: D-10 ("keep the guards and the drain … keep the Retiring record, and return `ElementRemoval`") and D-11. Both match the "retain ownership until cleanup succeeds; the identity is never resurrected" shape. D-7's "Retiring never returns to Admitted" matches "cannot be resurrected".

#### Finding 6.2: CNI DEL must be idempotent and tolerate missing state, and runtimes retry DEL. This is the "convergent removal" semantics.
**Evidence**:
- **CNI spec**: "Plugins MUST accept multiple `DEL` calls for the same (`CNI_CONTAINERID`, `CNI_IFNAME`) pair, and return success if the interface in question, or any modifications added, are missing." It also says: "Plugins should generally complete a `GC` action without error. If an error is encountered, a plugin should continue; removing as many resources as possible."
- **Cilium CNI DEL**: "Note: kubelet will retry the deletion for a long time. Therefore, only return an error for errors which are guaranteed to be recoverable". `DeleteEndpointNotFound` means "No need to retry". It returns the error only on `lib.ErrClientFailure`, when the agent is unreachable.

**Sources**: [CNI SPEC.md](https://github.com/containernetworking/cni/blob/main/SPEC.md); [Cilium cilium-cni cmd.go](https://github.com/cilium/cilium/blob/main/plugins/cilium-cni/cmd/cmd.go). Accessed 2026-09-23.
**Confidence**: High.
**Bears on**: D-10. The convergent `remove_allocation_elements`, where "Members already absent … are not an error", **matches** CNI's DEL rule and Cilium's not-found handling. D-10's retry-retaining `stop_alloc` matches "kubelet will retry the deletion".

#### Finding 6.3: Rust `Drop` cannot await. The language's own roadmap workaround is sync `Drop` plus a spawned task, which the repository's rules forbid for completion-bearing effects. Libraries expose explicit async close/flush methods.
**Evidence**:
- **Rust async fundamentals initiative, async drop** (status "💤", not implemented): "In these cases, however, it's usually enough to impl _synchronous_ Drop and spawn a task for the 'real' destructor."
- **tokio `fs::File`**: "A file will not be closed immediately when it goes out of scope if there are any IO operations that have not yet completed. To ensure that a file is closed immediately when it is dropped, you should call flush before dropping it". That is an explicit async operation before drop.
- **Tokio's graceful-shutdown guide** uses explicit signalling (`CancellationToken`) and explicit waiting (`TaskTracker::wait()`), not destructors.

**Sources**: [Rust async-fundamentals: async drop](https://rust-lang.github.io/async-fundamentals-initiative/roadmap/async_drop.html); [docs.rs tokio::fs::File](https://docs.rs/tokio/latest/tokio/fs/struct.File.html); [tokio.rs: Graceful Shutdown](https://tokio.rs/tokio/topics/shutdown). Accessed 2026-09-23.
**Confidence**: Medium-High. The initiative page is a roadmap note, not a language specification; no fetched source states verbatim that "Drop cannot be async".
**Bears on**: D-10.
**Analysis (interpretation)**:
- D-10 moves normal `2 + P` element release out of infallible `Drop` into an awaited port call that returns a typed error and retains ownership.
- That matches tokio's explicit-flush-before-drop pattern and the finalizer model.
- It does **not** adopt the Rust roadmap's "sync Drop + spawn" workaround. The repository's `rust.md` rule forbids that workaround for completion-bearing effects, and nothing in the prior art contradicts that rule.

#### Finding 6.4: Retrying cleanup of superseded instances without rewriting their state: finalizers and the kubelet's pod worker retry cleanup on non-current objects
**Evidence**: Findings 5.1, 6.1 and 6.2 together. The kubelet pod worker retries termination (`killSandboxResult.Fail(...)`, then retry) independently of the replacement pod the controller already created; the replacement is admitted separately. Finalizers retain a deleted object until its controller finishes.
**Confidence**: Medium (synthesis; no single source describes a "row-neutral reclaim action").
**Bears on**: D-11. A cleanup-retry owner that is separate from the current instance's lifecycle **matches** the Kubernetes split (the replica controller creates the replacement; the kubelet and finalizers retry the old instance's teardown). The specific shape — a reconciler-emitted action that writes no row, with backoff kept in the view — has no direct external precedent found. It is not contradicted either.

### RQ7 — Reconciling kernel dataplane state after abrupt agent restart (D-12)

#### Finding 7.1: Cilium restores live endpoints and diff-deletes stale BPF entries. Workloads keep running across an agent restart.
**Evidence** (local Cilium checkout, `daemon/cmd/endpoint_restore.go`, commit `e99150f8`):
- "restoreOldEndpoints performs the second step in restoring the endpoint structure, allocating their existing IPs out of the CIDR block and then inserting the endpoints into the endpoints list. It needs to be followed by a call to regenerateRestoredEndpoints() once the endpoint builder is ready. Endpoints which cannot be associated with a container workload are deleted."
- The code dumps the endpoint map (`existingEndpoints, err = r.lxcMap.DumpToMap()`) and deletes each restored endpoint's addresses from that dump. Every remaining non-host entry is then removed (`r.lxcMap.DeleteEntry(addr)` — "Removed outdated endpoint from endpoint map").
- Cilium documentation, `clean-cilium-bpf-state`: "ongoing connections may be briefly disrupted and loadbalancing decisions will be lost … All eBPF state will be reconstructed". Flush-and-rebuild is the exceptional path, not the default.

**Sources**: `/Users/marcus/git/cilium/cilium/daemon/cmd/endpoint_restore.go` (the user-supplied checkout named in the feature delta; upstream [cilium/cilium](https://github.com/cilium/cilium)); [Cilium Kubernetes configuration docs](https://docs.cilium.io/en/stable/network/kubernetes/configuration/). Accessed 2026-09-23.
**Confidence**: High for the mechanism.
**Unverified**: whether Cilium defers accepting new CNI ADDs until restoration completes. That ordering was not read.

#### Finding 7.2: kube-proxy nftables periodically does a full sync that flushes and rebuilds its own chains, and forces a full resync after any failure. Calico Felix re-checks its dataplane on fixed periods.
**Evidence**:
- **kube-proxy nftables** `syncProxyRules`: `doFullSync := proxier.needFullSync || (time.Since(proxier.lastFullSync) > proxyutil.FullSyncPeriod)`. A full sync calls `setupNFTables(tx)`, which recreates base chains and flushes chains (`tx.Flush(chain)`). On failure, `proxier.needFullSync = true`. Stale chains are flushed first and deleted later, once references are cleared.
- **Calico Felix**:
  - `IptablesRefreshInterval` (default 180 s) and `RouteRefreshInterval` (default 90 s) are "the period at which Felix re-checks … to ensure that no other process has accidentally broken Calico's rules".
  - `IpsetsRefreshInterval` (default 90 s) is "the period at which Felix re-checks all IP sets to look for discrepancies".
  - `NftablesRefreshInterval` defaults to 180 s.
  - `RemoveExternalRoutes` defaults to true.

**Sources**: [kube-proxy nftables proxier.go](https://github.com/kubernetes/kubernetes/blob/master/pkg/proxy/nftables/proxier.go); [calico felix config_params.go](https://github.com/projectcalico/calico/blob/master/felix/config/config_params.go); [calico felixconfig.go](https://github.com/projectcalico/calico/blob/master/api/pkg/apis/projectcalico/v3/felixconfig.go). Accessed 2026-09-23.
**Confidence**: High.
**Bears on**: D-12 (boot); D-13/D-15 (audit cadence; see RQ8).

**Analysis for D-12 (interpretation)**:
- **What D-12 is.** Its boot sequence runs VM reclamation with no adoption, a stale sweep, shared converge, member clear to ∅, program replacement, a full read-back, and only then admission (`open_after_boot`). That is a *flush-and-rebuild of owned state before admitting new work*.
- **Where it matches.** It matches kube-proxy's full-sync behaviour (flush owned chains, rebuild) and Cilium's rule that entries not associated with a live workload are deleted.
- **Where it diverges.** It departs from Cilium's *default* of hitlessly restoring live workloads. That departure follows from #295's accepted no-adoption rule (VMMs are reclaimed at boot), not from D-12 itself.
- **Clearing members is not a Cilium-style diff.** Once no workload survives, "clear members to ∅" is the degenerate diff: expected = ∅.
- **Ordering.** D-12 opens admission only after cleanup is read back. No surveyed source states an ordering that contradicts that; Cilium's ordering was not verified.

### RQ8 — Supervision, audit cadence, and fail-stop (D-13, D-14, D-15)

#### Finding 8.1: Bounded restart intensity with escalation is established (OTP, systemd). Unbounded retry with backoff is also established (Kubernetes, Cilium controllers).
**Evidence**:
- **Erlang/OTP**: "If more than `MaxR` number of restarts occur in the last `MaxT` seconds, the supervisor terminates all the child processes and then itself." The parent then restarts or terminates it. Defaults are `intensity` 1 and `period` 5. The guidance warns against a "very high period" and against identical intensities across levels, because "Total restarts equal the product of all supervisor intensities above a failing process".
- **systemd**: "Units which are started more than burst times within an interval time span are not permitted to start any more." `StartLimitAction=` adds an optional action and defaults to none. `systemctl reset-failed` flushes the counter.
- **Kubernetes**: "If the liveness probe fails, the kubelet kills the container, and the container is subjected to its restart policy". Restart backoff is exponential, capped at five minutes (CrashLoopBackOff). Retries are unbounded.
- **Cilium controllers** (`pkg/controller/controller.go`): "DoFunc is the function that will be run until it succeeds". On error, "this value is multiplied by the number of consecutive errors to provide a constant back off. The default is 1s". The backoff is optionally capped by `MaxRetryInterval`. There is no terminal fail-stop.

**Sources**: [Erlang OTP supervisor principles](https://www.erlang.org/doc/system/sup_princ.html); [systemd.unit(5) source XML](https://github.com/systemd/systemd/blob/main/man/systemd.unit.xml); [Kubernetes Pod Lifecycle](https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/); Cilium `pkg/controller/controller.go` (local checkout `e99150f8`; upstream [cilium/cilium](https://github.com/cilium/cilium)). Accessed 2026-09-23.
**Confidence**: High.
**Bears on**: D-13, D-14.
**Analysis**: D-13 (at most 20 attempts or 5 s of recovery, then a one-shot typed fail-stop that exits the process) has the **OTP/systemd shape**: bounded intensity, then escalate to the parent, here the process supervisor. It diverges from Cilium's controllers, which retry forever. Both families exist in mature systems, so the choice is not unprecedented. Escalating to process exit relies on an external restarter (for example systemd) to supply the "parent" level.

#### Finding 8.2: When an agent cannot confirm enforcement, mature dataplanes keep enforcing the last programmed state ("fail static") or let traffic fail. None found kills every workload on the node locally.
**Evidence**:
- **Cilium upgrade guide**: "Networking connectivity, policy enforcement and load balancing will remain functional in general" while the agent restarts. "Existing policy will remain effective but implementation of new policy rules will be postponed". User-space L7 proxies restart, and "results in a connectivity outage and causes the connection to reset".
- **Istio ambient**: the CNI ADD fails pod creation when ztunnel is unreachable (Finding 3.2; user report GH #53843 "partial add error: no ztunnel connection"). No workload is killed.
- **Istio sidecar-mode repair**: acts only on pods whose redirection is broken — "Delete pods", "Label pods", or "Repair pods" (Finding 3.2). The action is scoped to affected pods.
- **Kubernetes node controller**: when a node is NotReady or Unreachable, it applies `NoExecute` taints. Pods without a toleration are evicted after the default 300 s toleration (`DefaultTolerationSeconds`). Eviction is **node-wide but API-side** (reschedule elsewhere), delayed, and rate-limited when many nodes are unhealthy. It is not a local kill by the node agent.
- **Counter-evidence, a fail-open incident**: Istio GH #60882 reports that after a node reboot without drain, ambient pods "lose their traffic redirection rules to ztunnel" and their traffic "bypasses ztunnel". No maintainer response was recorded.

**Sources**: [Cilium upgrade guide](https://docs.cilium.io/en/stable/operations/upgrade/); [Istio GH #53843](https://github.com/istio/istio/issues/53843); [Istio CNI race mitigation](https://istio.io/latest/docs/setup/additional-setup/cni/); [Kubernetes Nodes](https://kubernetes.io/docs/concepts/architecture/nodes/); [Istio GH #60882](https://github.com/istio/istio/issues/60882). Accessed 2026-09-23.
**Confidence**: Medium-High. There are several independent systems, but a negative result ("no system kills all") is inherently incomplete.
**Bears on**: **D-14.**
**Analysis (interpretation, labelled)**:
- **Established:** a node-wide consequence of unconfirmed node health (Kubernetes taint eviction), and scoping remediation to affected workloads (Istio repair, liveness kills one container).
- **Not found:** killing *every* workload on a node **locally and immediately** when the agent cannot confirm quiescence (D-14: write `1` to `<workloads_slice>/cgroup.kill`, then fail-stop). The nearest precedent is Kubernetes' node-wide eviction, which is API-side, delayed by 300 s, and rate-limited.
- **The case for D-14's scope, and its limit.** The affected set in D-14 is arguably *all* workloads, because the failing control (TAP quiescence over a shared bridge and shared nft program) is node-wide. By Istio's "scope to affected" principle the node-wide scope is defensible, but no external system applies it as a local kill.
- **Opposite posture.** The fail-static dataplanes (Cilium, and Calico by the same design) keep enforcing kernel state when the agent is gone. That depends on the kernel state still being correct. D-14 addresses the case where it cannot be confirmed, which Cilium's posture does not claim to cover.

#### Finding 8.3: Periodic audit-and-repair of owned dataplane state against expected state is standard. The published cadences are tens of seconds to minutes.
**Evidence**:
- **Calico Felix**: `IptablesRefreshInterval` (180 s) and `RouteRefreshInterval` (90 s) are "the period at which Felix re-checks … to ensure that no other process has accidentally broken Calico's rules". `IpsetsRefreshInterval` (90 s) is "the period at which Felix re-checks all IP sets to look for discrepancies". `NftablesRefreshInterval` defaults to 180 s.
- **kube-proxy nftables**: a periodic full sync (`time.Since(proxier.lastFullSync) > proxyutil.FullSyncPeriod`) plus a forced full sync after any failure. The value of `FullSyncPeriod` was not read (**unverified value**).
- **Cilium**: per-endpoint and per-subsystem controllers with `RunInterval` plus retry.

**Sources**: Findings 7.2 and 8.1. Accessed 2026-09-23.
**Confidence**: High for the pattern; Medium for the cadence comparison.
**Bears on**: D-15 and D-13.
**Analysis**:
- **Pattern — matches.** D-15 (the worker audits the constant program *and* dynamic members against its registry, and repairs both through the same port) **matches** Felix's "re-check all IP sets to look for discrepancies" plus rules re-check, and kube-proxy's full sync.
- **Cadence — diverges.** D-13's `SHARED_NETWORK_AUDIT_PERIOD = 1 s` is **one to two orders of magnitude more frequent** than Felix's defaults. No surveyed system audits at 1 s by default. The DESIGN itself makes the audit bound and mutex hold time subject to native measurement (E18) at density.
- Felix's audits are drift correction. D-13's 1 s bound comes from ADR-0124's detection-latency requirement, a security-timing goal Felix does not claim. This is a divergence in cadence, not in pattern.

### RQ9 — Fail-closed mark-based interception (D-18)

#### Finding 9.1: The kernel TPROXY pattern is mark → policy rule → `local` route in a side table → a transparent socket. Delivery depends on both the mark rule and the socket assignment.
**Evidence** (kernel TPROXY doc): `iptables -t mangle -N DIVERT; iptables -t mangle -A PREROUTING -p tcp -m socket --transparent -j DIVERT; iptables -t mangle -A DIVERT -j MARK --set-mark 1`, then `ip rule add fwmark 1 lookup 100; ip route add local 0.0.0.0/0 dev lo table 100`. TPROXY takes `--tproxy-mark 0x1/0x1 --on-port 50080`. Sockets need `IP_TRANSPARENT` before `bind()`. The document does **not** say what happens to a marked packet with no matching transparent socket (gap).
- **Istio ambient** uses the same pattern inside the pod netns: mangle `TPROXY --on-port 15008 --on-ip 0.0.0.0 --tproxy-mark 0x111/0xfff`, with marks `0x539/0xfff` and `0x111/0xfff`.

**Sources**: [kernel TPROXY doc](https://docs.kernel.org/networking/tproxy.html); [Istio ambient traffic redirection](https://istio.io/latest/docs/ambient/architecture/traffic-redirection/). Accessed 2026-09-23.
**Confidence**: High.
**Bears on**: D-18. The #295 healthy path (TPROXY rewrites the mark to `0x1`; `fwmark 0x1` selects table 100's `local 0.0.0.0/0 dev lo`) is this canonical pattern.

#### Finding 9.2: Routing-policy order: the `local` table is consulted first, at priority 0, so an added fwmark blackhole rule cannot stop delivery to host-local addresses.
**Evidence** (`ip-rule(8)`):
- At startup the RPDB holds "Priority: 0 … match anything, Action: lookup routing table **local** (ID 255)". The local table holds "high priority control routes for local and broadcast addresses".
- Main (32766) and default (32767) follow.
- Rules are "scanned in order of decreasing priority (note that a lower number means higher priority)".
- Action types: "**blackhole** - the rule causes a silent drop the packet", "**unreachable**", "**prohibit**".
- Linux address ownership: "IP addresses are owned by the complete host on Linux, not by particular interfaces" (`ip-sysctl`, `arp_filter` default). A destination equal to *any* host address therefore matches a `local` route.

**Sources**: [ip-rule(8)](https://man7.org/linux/man-pages/man8/ip-rule.8.html); [kernel ip-sysctl](https://docs.kernel.org/networking/ip-sysctl.html). Accessed 2026-09-23.
**Confidence**: High.
**Bears on**: D-18. It confirms the R18-A rejection reason: a `fwmark 0x295a blackhole` rule added at any preference > 0 drops forwarded packets but not packets to host addresses, because pref 0 `lookup local` matches first.
**Unverified**: whether rule 0 can be deleted or moved (R18-C's premise). The kernel VRF document fetched did not contain the "move the local rule" recipe, and `ip-rule(8)` as fetched did not state deletability. R18-C is not the recommended option, so this does not affect R18-B.

#### Finding 9.3: nftables base-chain semantics support an independent drop table that fires after the intercept chain, whatever the intercept chain did.
**Evidence** (`nft(8)`, verbatim):
- "An **accept** verdict (including an implicit one via the base chain's policy) ends the evaluation of the current base chain."
- "A **drop** verdict (including an implicit one via the base chain's policy) immediately ends the evaluation of the whole ruleset."
- "For each hook, the attached chains are evaluated in order of their priorities. Chains with lower priority values are evaluated before those with higher ones."
- Standard ip-family priorities include `mangle -150` and `filter 0`.
- The prerouting hook runs before the routing decision.

**Source**: [nft(8) man page, netfilter.org](https://www.netfilter.org/projects/nftables/manpage.html). Accessed 2026-09-23.
**Confidence**: High for the verdict semantics (verbatim re-fetch). The prerouting-before-routing statement is a summarised paraphrase.
**Bears on**: D-18 (R18-B).
**Analysis**: A separate table whose prerouting base chain sits at priority 0 (after the mangle-priority intercept chain at −150) sees every packet the intercept chain accepted, and its drop is final. That is consistent with R18-B's claims:
- in healthy operation the guard matches nothing, because the intercept chain rewrites `0x295a` to `0x1` or drops;
- with the intercept table deleted, packets still carrying `0x295a` meet the guard before routing, so neither forwarding nor local delivery occurs.

This is inferred from documented semantics. It has not been observed natively; the DESIGN itself conditions R18 on E14.

#### Finding 9.4: The leak class is real in prior art: when interception rules vanish, traffic bypasses the proxy. An *independent* L3/L4 layer is Istio's documented defense-in-depth.
**Evidence**:
- **Istio GH #60882** (open, no maintainer response recorded): after node reboot without drain, ambient pods "lose their traffic redirection rules to ztunnel", and "application traffic from these pods bypasses ztunnel … and was blocked by Network Policies". The workaround was restarting istio-cni.
- **Istio security best practices**: "Istio policies can be layered with Kubernetes Network Policies. This enables a strong defense in depth strategy". Also: "the application may have the ability to remove redirection rules … the security boundary is that a client may not bypass another pod's sidecar."
- **Linkerd**: with native sidecars disabled, init-container packets "will be caught by iptables and the linkerd-proxy will not yet be available". Linkerd's model depends on the redirect rules being present.

**Sources**: [Istio GH #60882](https://github.com/istio/istio/issues/60882); [Istio security best practices](https://istio.io/latest/docs/ops/best-practices/security/); [Linkerd CNI](https://linkerd.io/2-edge/features/cni/). Accessed 2026-09-23.
**Confidence**: Medium-High. The bypass incident is one user report; the defense-in-depth recommendation is official.
**Bears on**: D-18.
**Analysis (interpretation)**:
- **Principle — established.** Independent enforcement layers, so that loss of the interception rules does not leave traffic unprotected. In #60882 the independent layer (NetworkPolicy) is what caught the bypass.
- **Exact mechanism — no precedent found.** A second nft table dropping packets that still carry the *interception classifier's* mark was not found in Cilium, Istio, or Linkerd source. That is not a finding that it is unsound; Findings 9.3 and 9.2 support its semantics.

#### Finding 9.5: Are the two leak paths D-18 claims real? Both are consistent with kernel semantics; neither is externally verified for the #295 topology.
**Evidence and analysis**:
- **Host-local delivery.** A TCP packet reaching the host IP stack with a host address as destination matches the pref-0 `local` rule (Finding 9.2). Addresses are host-owned regardless of interface (ip-sysctl). With no transparent-socket assignment (the IP program is gone), normal socket lookup applies, and a wildcard (`0.0.0.0`) listener on that port matches.
  - Supported by kernel semantics.
  - **Not natively verified.**
  - Whether other host controls (host firewall INPUT policy, `rp_filter`) would drop it was not researched.
- **Forwarding in cleartext.** With `net.ipv4.ip_forward=1` ("Forward Packets between interfaces"), a packet whose destination is not local is routed by the main table. For a peer guest address on the same bridge subnet, that means back out the bridge to the peer TAP.
  - Consistent with kernel forwarding semantics.
  - **Not natively verified.**
  - Depends on #295-specific facts not checked here: bridge guard rules, forward-chain policy, `rp_filter`, TCX on the peer TAP's egress path.
  - The DESIGN acknowledges this ("reasoned from source and not executed") and requires the E14 native RED.

**Sources**: [kernel ip-sysctl](https://docs.kernel.org/networking/ip-sysctl.html); [ip-rule(8)](https://man7.org/linux/man-pages/man8/ip-rule.8.html). Accessed 2026-09-23.
**Confidence**: Medium (inference from primary semantics, no observation).

### RQ10 — Injectable dataplane/DNS owners (D-16)

*Brief, per instruction: only enough to judge whether a required DNS port is conventional.*

#### Finding 10.1: Node-local DNS is an optional, separately deployed agent in Kubernetes. Cilium composes its DNS proxy as an agent component behind an explicit contract, and wires agent components through constructor-declared dependencies.
**Evidence**:
- **Kubernetes NodeLocal DNSCache** "improves Cluster DNS performance by running a DNS caching agent on cluster nodes as a DaemonSet". It runs CoreDNS in cache mode on a link-local address, with the recommendation "from the 'link-local' range '169.254.0.0/16'". It "will query the CoreDNS service for cache misses". It is optional: "You can disable this feature by removing the DaemonSet". The page does not say what happens to pod DNS when the cache pod is down (gap).
- **Cilium**: the DNS proxy runs in the agent by default. An alpha "standalone DNS proxy" talks to the agent over a "gRPC API contract between Standalone DNS Proxy and Cilium Agent" (`Documentation/sdpapi.rst`; flag `--enable-standalone-dns-proxy`).
- **Cilium hive**: "Cilium is using dependency injection (via `pkg/hive`) to wire up the initialization, starting and stopping of its components … Object constructors only need to declare their dependencies as function parameters" (`Documentation/contributing/development/hive.rst`).

**Sources**: [Kubernetes NodeLocal DNSCache](https://kubernetes.io/docs/tasks/administer-cluster/nodelocaldns/); Cilium `Documentation/sdpapi.rst`, `Documentation/cmdref/cilium-agent.md`, and `Documentation/contributing/development/hive.rst` (local checkout `e99150f8`; upstream [cilium/cilium](https://github.com/cilium/cilium)). Accessed 2026-09-23.
**Confidence**: Medium (brief by design).
**Bears on**: D-16.
**Analysis**:
- **Composition — matches.** Constructor-declared, non-optional dependencies are how Cilium wires agent components. That matches D-16's "required `ServerConfig` ports composed at the serve boundary", as opposed to `Option` overrides.
- **Whether DNS must be present — no convention.** In Kubernetes node-local DNS is optional. In #295, guest DNS is a required product component (ADR-0072), so making its port required follows from the product decision rather than from DNS-agent convention.
- **A DNS lifecycle trait.** Cilium's standalone-proxy contract shows that DNS behind an explicit component interface is not unusual.

## Per-Decision Table (D-1 … D-18)

Legend: **Matches**: prior art or primary source supports the decision's technical premise. **Diverges**: evidence contradicts the premise, or established practice does something materially different. **Unsupported**: no precedent found either way; the claim rests only on the repository's own evidence or reasoning. **N/A**: not a technical claim, or out of scope.

| Decision | What prior art / primary sources do | Verdict | Key evidence |
|---|---|---|---|
| **D-1** CH gets the NIC as one inherited TAP queue fd (`--net fd=`); the TAP stays down through READY/Running | libvirt/QEMU (`-netdev tap,fd=`), Kata→CH (`vm.add-net` + `SCM_RIGHTS`), AVF virtmgr→crosvm (`tap-fd=`) all pass parent-opened TAP fds. Firecracker opens by name inside a per-VM netns. CH's `from_tap_fds` `dup`s and never enables; only the named `open_tap` calls `tap.enable()`. | **Matches** (fd handoff). **Unsupported externally** (holding the TAP admin-down across guest boot as the gate; own native evidence only). | F1.1, F1.2, F3.3 |
| **D-2** The VMM adapter attaches, verifies, maps, and closes the per-launch queue; single-queue `IFF_TAP\|IFF_NO_PI\|IFF_VNET_HDR`, `O_CLOEXEC`; parent copy dropped with the `Command` | Firecracker's own open uses exactly `O_RDWR\|O_NONBLOCK\|O_CLOEXEC` and `IFF_TAP\|IFF_NO_PI\|IFF_VNET_HDR`. libvirt closes the parent copy "no later than Run/RunAsync/Free". command-fds' `OwnedFd` lives until the `Command` drops. `tun.c`: second single-queue attach → `EBUSY`; flags overwritten per attach. | **Matches**. **Diverges** on one sub-claim: "No other descriptor above 2 is inherited" is not enforced by command-fds (no mass close); libvirt and the Firecracker jailer close all other fds in the child. | F1.3, F1.4, F2.1 |
| **D-3** `command-fds` safe wrapper behind a dependency-review gate | Apache-2.0, `google/command-fds`, v0.3.3 (2026-04-10), 5.1M downloads, 41 reverse deps, optional `tokio` feature. AOSP virtmgr uses it to hand a TAP fd to crosvm. `dup2` in `pre_exec`, non-allocating; collisions handled with `F_DUPFD_CLOEXEC`. | **Matches** (direct Rust-VMM precedent). The license is confirmed. The tokio `Command` impl was seen on docs.rs, not read in source (medium). | F1.5, F1.4 |
| **D-4** Guest TAPs carry no owner grant; "only `CAP_NET_ADMIN` can attach" | Kernel `tun_not_capable`: with owner **and** group unset, *no* capability check applies to attaching an existing device. Firecracker CI attaches ownerless TAPs as uid 1234. Isolation in prior art comes from an owner uid (crosvm), fd custody plus non-persistence (libvirt), or per-VM netns (Firecracker). | **Diverges — premise contradicted by kernel source.** An ownerless host-netns TAP is attachable by the VMM uid when it is not attached; E4's predicted `EPERM` is predicted by source to fail. | F2.2, F2.3 |
| **D-5** Provision down; `activate` only after the exact intercept success event, before EXEC; serialized with quiescence | Cilium CNI ADD waits for synchronous endpoint regeneration. The Istio ambient ADD fails pod creation without the node agent. `istio-validation` blocks pod start. The kubelet creates the sandbox and network before any container. All of them order enforcement **before the workload starts**. | **Matches** (principle: no traffic before enforcement is live and read back). **Unsupported externally** (the admin-down-across-boot mechanism). The kernel also offers `IFF_NO_CARRIER`/`TUNSETCARRIER` (not mentioned in the DESIGN). | F3.1, F3.2, F3.3, F2.1 |
| **D-6** `GuestAddressPool::assign` is the sole admission linearization point; atomic `replace` handover | The Nomad plan applier (serialized, re-verifies `AllocsFit`, partial reject) and kubelet admission (re-checks `OutOfpods` against active pods) are single authoritative gates behind optimistic placement. In Kubernetes/AWS the count gate (`maxPods`) is separate from IPAM, and IP exhaustion surfaces after placement. | **Matches** (single serialized authority that may refuse). **Unsupported** (fusing the admission count into the address allocator's lock; no precedent found, not refuted). | F4.1, F4.2, F4.3 |
| **D-7** Only Admitted leases count toward 16,384; Retiring leases do not (they count toward `held`) | The Kubernetes kubelet counts terminating pods until fully terminated, including `StopPodSandbox`/CNI teardown, and calls the force-delete exception a bug (#104824). Nomad excludes desired-stop allocs from `AllocsFit` while they still run (port-collision report #1225). KEP-3939/3973 move Kubernetes toward waiting for full termination. | **Conflicting prior art.** D-7 sits between: retire happens after VMM quiescence (stricter than Nomad), but uncounted while network/intercept residue remains (looser than Kubernetes). **Diverges from Kubernetes** if the cap protects dataplane state rather than compute. | F5.1, F5.2, F5.3 |
| **D-8** Placement reads one consistent occupancy snapshot through a new read-port; advisory; restart gated excluding its own predecessor | The Kubernetes scheduler's assume cache (advisory, `ForgetPod` undo) plus kubelet admission (authoritative). Nomad optimistic schedulers plus plan applier rejection plus scheduler retry. | **Matches**. | F4.1, F4.2 |
| **D-9** Workload-local CPU/memory defect scoped out of #295 | The Kubernetes scheduler/kubelet and Nomad `AllocsFit` account node-wide usage, which corroborates that workload-local accounting is a defect. | **N/A** (scope decision). | F4.1, F4.2 |
| **D-10** Element release is one awaited, convergent, retry-retaining port operation | CNI: DEL "MUST … return success if the interface … or any modifications added, are missing". Cilium DEL treats not-found as success, and the "kubelet will retry the deletion for a long time". Finalizers retain objects until cleanup succeeds. Rust async drop is unimplemented; tokio documents explicit flush before drop. | **Matches**. | F6.1, F6.2, F6.3 |
| **D-11** Row-neutral `ReclaimAllocationNetwork` retries cleanup of superseded allocations | The kubelet pod worker and finalizers retry teardown of the old instance independently of the replacement. | **Matches** (principle). **Unsupported** (the exact row-neutral action shape; no direct precedent found). | F6.4, F5.1 |
| **D-12** Fresh-process boot converges dynamic members to ∅ before the program and before admission | kube-proxy full sync flushes and rebuilds owned chains, and a failure forces a full resync. Cilium dumps its endpoint map and deletes entries not tied to a live workload. Cilium's default is a hitless restore of live workloads. | **Matches** (given #295's accepted no-adoption rule). The divergence from Cilium's hitless restore comes from no-adoption, not D-12. Cilium's admission ordering is unverified. | F7.1, F7.2 |
| **D-13** Recovery = converge every failing owner → one full audit → restore quiesced TAPs only if clean; bounded 20 attempts / 5 s → fail-stop; 1 s audit | OTP: more than MaxR restarts in MaxT → terminate children and self, escalate. systemd StartLimit refuses further starts. Cilium controllers retry forever with linear backoff. Felix refresh defaults are 90–180 s. | **Matches** the OTP/systemd bounded-then-escalate shape. **Diverges** from Cilium's unbounded retry (both families exist). **Diverges in cadence**: 1 s vs 90–180 s (driven by ADR-0124, not by prior art). | F8.1, F8.3 |
| **D-14** Unconfirmed quiescence → kill every workload VMM (`cgroup.kill` on the workloads slice) → fail-stop | Cilium is fail-static (the datapath keeps enforcing while the agent is down). Istio fails pod creation or traffic without killing workloads. Istio repair deletes only pods with broken redirection. Kubernetes node NotReady leads to node-wide eviction that is API-side, delayed 300 s, and rate-limited. Liveness kills one container. | **Unsupported by precedent** for a node-wide *local immediate* kill. The nearest analogue (Kubernetes taint eviction) is node-wide but API-side, delayed, and rate-limited. The scope matches the "affected set" only because the failing control is node-wide. | F8.2 |
| **D-15** The worker audits the constant program and dynamic members against its registry; repairs through the same port | Felix re-checks rules and IP sets for "discrepancies" periodically. kube-proxy runs a periodic full sync. Cilium controllers run periodic `DoFunc`. | **Matches** (pattern). Cadence: see D-13. | F8.3, F7.2 |
| **D-16** `ServerConfig` requires intercept and guest-DNS ports; always composed | Cilium hive: constructors declare dependencies as parameters. The Cilium DNS proxy is in-agent or standalone behind a gRPC contract. Kubernetes NodeLocal DNSCache is optional. | **Matches** (required constructor-injected composition). A DNS port being *required* follows the product decision (ADR-0072), not DNS convention. | F10.1 |
| **D-17** Serve lifetime port as built | — | **N/A** (out of scope by instruction; user-ruled). | — |
| **D-18** An independent guard table drops TCP still marked `0x295a` after the intercept chain (R18-B) | Kernel TPROXY: mark → `ip rule fwmark` → `local` route table. `ip-rule(8)`: pref 0 `lookup local` is scanned first, so an added blackhole rule cannot stop host-local delivery (confirms the R18-A rejection). `nft(8)`: chains run in priority order; accept ends only the current base chain; drop ends the whole ruleset (supports R18-B). The leak class is real (Istio #60882: rules lost → traffic bypasses the proxy, caught only by NetworkPolicy). Istio officially recommends independent NetworkPolicy defense in depth. | **Matches** (principle, and the kernel/nft semantics of both the rejection and the chosen option). **Unsupported externally** (the exact guard-table-on-classifier-mark mechanism). Both leak paths are **consistent with kernel semantics but unverified** (native E14 RED required, as the DESIGN states). | F9.1–F9.5 |

## Source Analysis

| Source | Domain | Reputation | Type | Access date | Cross-verified |
|---|---|---|---|---|---|
| Linux `drivers/net/tun.c` | github.com (torvalds/linux) | High (primary kernel source) | primary source code | 2026-09-23 | Y (Firecracker CI, repo native spikes) |
| Kernel TUN/TAP, TPROXY, ip-sysctl docs | docs.kernel.org | High | official | 2026-09-23 | Y |
| `ip-rule(8)` | man7.org | High (iproute2 man page; domain not in trusted list, primary documentation) | official man page | 2026-09-23 | Y (kernel TPROXY doc) |
| `nft(8)` | netfilter.org | High (project's own man page; domain not in trusted list) | official man page | 2026-09-23 | Partial |
| Cloud Hypervisor `net.rs`, `open_tap.rs`, `config.rs`, macvtap doc | github.com | High (primary) | primary source code / docs | 2026-09-23 | Y (repo native spikes) |
| Firecracker `tap.rs`, `jailer.md`, `network-setup.md`, `getting-started.md`, tests | github.com | High (primary) | primary source code / docs | 2026-09-23 | Y |
| libvirt `virnetdevtap.c`, `vircommand.c` | github.com (libvirt mirror) | High (primary) | primary source code | 2026-09-23 | Y |
| QEMU invocation docs | qemu.org | High (project docs; domain not in trusted list) | official | 2026-09-23 | Y (libvirt, Kata) |
| Kata Containers `clh.go` | github.com | High (primary) | primary source code | 2026-09-23 | Partial |
| AOSP virtmgr `crosvm.rs` | android.googlesource.com | High (primary AOSP source; domain not in trusted list) | primary source code | 2026-09-23 | Y (crates.io reverse deps, command-fds source) |
| crosvm book | crosvm.dev | High (project docs; domain not in trusted list) | official | 2026-09-23 | N |
| `command-fds` crates.io API, docs.rs, GitHub source | crates.io, docs.rs, github.com | High | technical docs / primary | 2026-09-23 | Y |
| Rust std `CommandExt` | doc.rust-lang.org | High | official | 2026-09-23 | Y |
| Kubernetes kubelet, scheduler, kube-proxy source; PR #104577; KEPs 3939/3973 | github.com | High (primary) | primary source code | 2026-09-23 | Y |
| Kubernetes docs (finalizers, pod lifecycle, nodes, deployment, NodeLocal DNS, network plugins) | kubernetes.io | High | official | 2026-09-23 | Y |
| CNI SPEC.md | github.com (containernetworking) | High (specification) | official spec | 2026-09-23 | Y |
| Nomad `plan_apply.go`, `alloc.go`, `schema.go`; scheduling docs; GH #1225 | github.com, developer.hashicorp.com | High | primary / official | 2026-09-23 | Y |
| Cilium source (local checkout `e99150f8`, and `plugins/cilium-cni` on GitHub); docs.cilium.io | github.com, docs.cilium.io | High | primary / official | 2026-09-23 | Y |
| Calico `config_params.go`, `felixconfig.go` | github.com | High (primary) | primary source code | 2026-09-23 | N (single project) |
| Istio `plugin.go`; istio.io docs; GH #53843, #60882 | github.com, istio.io | High / Medium-High (issues) | primary / official / user reports | 2026-09-23 | Partial |
| Linkerd CNI docs | linkerd.io | High | official | 2026-09-23 | N |
| AWS EKS docs, amazon-vpc-cni-k8s troubleshooting | docs.aws.amazon.com, github.com | High | technical docs | 2026-09-23 | Y |
| Erlang OTP supervisor principles | erlang.org | High (project docs; domain not in trusted list) | official | 2026-09-23 | Y (systemd) |
| systemd `systemd.unit.xml`, `50-udev-default.rules.in` | github.com (systemd) | High (primary) | primary source | 2026-09-23 | Y |
| tokio docs (shutdown, `fs::File`); Rust async-fundamentals initiative | tokio.rs, docs.rs, rust-lang.github.io | High / Medium-High (initiative roadmap page) | official / technical | 2026-09-23 | Partial |

Reputation: all cited sources are primary source code, specifications, or official project documentation. Several official project documentation domains are outside the configured trusted list: qemu.org, man7.org, netfilter.org, erlang.org, crosvm.dev, android.googlesource.com, freedesktop/systemd via GitHub, and rust-lang.github.io. Each is the project's own primary documentation or source, and is marked as such. No medium-trust or excluded domains are relied on. Search-result blog hits (oneuptime.com, arthurchiao.art, backreference.org, gabriel.urdhr.fr) were **not** used as evidence. Average reputation is ≈ 0.97.

## Knowledge Gaps / Unverified Claims

1. **Kata's close point for its copy of the TAP fds.** No `.Close()` was found in `clh.go`. It may live in shared network code, which was not read. **Unverified.**
2. **Tokio `Command` support in `command-fds`.** Docs.rs indicates `tokio::process::Command` implements `CommandFdExt` behind the `tokio` feature; this was not read in source. **Medium confidence.**
3. **Rule-0 deletability (R18-C premise).** Neither `ip-rule(8)` as fetched nor the kernel VRF document stated whether the pref-0 local rule can be deleted or moved. **Unverified.** R18-C is not the recommended option.
4. **Both D-18 leak paths** (cleartext forwarding to a peer TAP; delivery to a host `0.0.0.0` listener). They are consistent with documented kernel semantics, but not observed. They depend on #295-specific host controls (bridge guard, forward policy, `rp_filter`, host INPUT policy) that were not researched. **Unverified** until native E14.
5. **Kernel TPROXY doc** does not state what happens to a marked packet with no matching transparent socket. **Gap.**
6. **Cilium's ordering of new CNI ADD admission relative to endpoint restoration** after an agent restart. **Unverified.**
7. **kube-proxy `FullSyncPeriod` value.** Not read. **Unverified.**
8. **Calico/Cilium IPAM exhaustion behaviour.** Not researched beyond Cilium's generic ADD error path. **Gap.**
9. **Linkerd network-validator init container.** Not found on the fetched Linkerd CNI page. **Gap.**
10. **Isolation model for Kata/AVF TAPs** (owner set? persistent? per-sandbox netns?). Not verified. **Gap** (Finding 2.3 table cells).
11. **The kubelet's reason for re-checking predicates at admission.** The mechanism is verified; the documented rationale was not found in source comments. **Gap.**
12. **Nomad GH #1225 body.** Seen only in search results; linking it to the `TerminalStatus` semantics is Medium confidence.
13. **Overdrive appliance `/dev/net/tun` mode.** The stock systemd udev rule is `0666`; the appliance image was not checked. **Unverified.**
14. **CH source was read on `main`, not the v53.0 tag.** The repository spike cites v53.0 line links for the same functions.
15. **Whether any system kills all node workloads locally on unconfirmed isolation.** None was found among Cilium, Istio, Linkerd, Kubernetes, or Calico. A negative result is not proof of absence.

## Conflicting Information

### Conflict 1: Who may attach to an ownerless persistent TAP
- **Position A (documentation):** "CAP_NET_ADMIN is required for creating network devices or for connecting to network devices which aren't owned by the user in question." Source: [kernel TUN/TAP doc](https://docs.kernel.org/networking/tuntap.html), reputation 1.0. Read literally, an ownerless device "isn't owned by the user", so `CAP_NET_ADMIN` would be required. ADR-0130 adopts this reading.
- **Position B (code):** `tun_not_capable` requires `CAP_NET_ADMIN` only when `uid_valid(tun->owner)` and the caller is not the owner, or when `gid_valid(tun->group)` and the caller is not in the group. With neither set, any caller may attach. Source: [tun.c](https://github.com/torvalds/linux/blob/master/drivers/net/tun.c), reputation 1.0. Corroborated by Firecracker CI, where jailed uid 1234 attaches ownerless TAPs.
- **Assessment:** Position B is authoritative. The code is the behaviour, and an independent production project's CI depends on it. The documentation's sentence describes the owned-device case and is silent on the ownerless case.

### Conflict 2: Do terminating workloads count toward node capacity?
- **Position A (Kubernetes kubelet):** yes, until fully terminated, including sandbox teardown. Excluding force-deleted-but-terminating pods is flagged as a bug (#104824). Sources: `kubelet_pods.go` and PR #104577, reputation 1.0.
- **Position B (Nomad plan applier):** no. Allocations with desired status stop or evict are terminal for fit even while running. Sources: `alloc.go`, `schema.go`, `plan_apply.go`, reputation 1.0.
- **Assessment:** Both are authoritative for their own systems. Mature orchestrators genuinely differ. Upstream Kubernetes is moving toward counting terminating workloads (KEP-3939, KEP-3973). D-7 sits between the two positions (Finding 5.3 analysis).

### Conflict 3: Istio ambient startup posture (fetched-summary artefact)
- One fetched summary of the Istio ambient redirection page characterised ambient as a "fail-open model".
- Istio's CNI plugin source returns `istio-cni cmdAdd failed to contact node Istio CNI agent`, which fails pod creation, and user report GH #53843 shows exactly that failure.
- **Assessment:** The source code is authoritative for the ADD path. The fail-open characterisation is rejected as a summarisation artefact. GH #60882 shows a separate, real fail-open incident *after* node reboot, when rules were lost on existing pods. That concerns rule loss, not ADD.

## Full Citations

[1] Linux kernel. "drivers/net/tun.c". GitHub torvalds/linux (master). https://github.com/torvalds/linux/blob/master/drivers/net/tun.c. Accessed 2026-09-23.
[2] Linux kernel docs. "Universal TUN/TAP device driver". https://docs.kernel.org/networking/tuntap.html. Accessed 2026-09-23.
[3] Linux kernel docs. "Transparent proxy support". https://docs.kernel.org/networking/tproxy.html. Accessed 2026-09-23.
[4] Linux kernel docs. "IP Sysctl". https://docs.kernel.org/networking/ip-sysctl.html. Accessed 2026-09-23.
[5] iproute2. "ip-rule(8)". https://man7.org/linux/man-pages/man8/ip-rule.8.html. Accessed 2026-09-23.
[6] netfilter project. "nft(8)". https://www.netfilter.org/projects/nftables/manpage.html. Accessed 2026-09-23.
[7] Cloud Hypervisor. "virtio-devices/src/net.rs". https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/virtio-devices/src/net.rs. Accessed 2026-09-23.
[8] Cloud Hypervisor. "net_util/src/open_tap.rs". https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/net_util/src/open_tap.rs. Accessed 2026-09-23.
[9] Cloud Hypervisor. "vmm/src/config.rs". https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/vmm/src/config.rs. Accessed 2026-09-23.
[10] Cloud Hypervisor. "docs/macvtap-bridge.md". https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/macvtap-bridge.md. Accessed 2026-09-23.
[11] Firecracker. "src/vmm/src/devices/virtio/net/tap.rs". https://github.com/firecracker-microvm/firecracker/blob/main/src/vmm/src/devices/virtio/net/tap.rs. Accessed 2026-09-23.
[12] Firecracker. "docs/jailer.md". https://github.com/firecracker-microvm/firecracker/blob/main/docs/jailer.md. Accessed 2026-09-23.
[13] Firecracker. "docs/network-setup.md" and "docs/getting-started.md". https://github.com/firecracker-microvm/firecracker/tree/main/docs. Accessed 2026-09-23.
[14] Firecracker. "tests/host_tools/network.py" and "tests/framework/jailer.py". https://github.com/firecracker-microvm/firecracker/tree/main/tests. Accessed 2026-09-23.
[15] libvirt. "src/util/virnetdevtap.c". https://github.com/libvirt/libvirt/blob/master/src/util/virnetdevtap.c. Accessed 2026-09-23.
[16] libvirt. "src/util/vircommand.c". https://github.com/libvirt/libvirt/blob/master/src/util/vircommand.c. Accessed 2026-09-23.
[17] QEMU. "Invocation — -netdev tap". https://www.qemu.org/docs/master/system/invocation.html. Accessed 2026-09-23.
[18] Kata Containers. "src/runtime/virtcontainers/clh.go". https://github.com/kata-containers/kata-containers/blob/main/src/runtime/virtcontainers/clh.go. Accessed 2026-09-23.
[19] AOSP. "packages/modules/Virtualization/android/virtmgr/src/crosvm.rs". https://android.googlesource.com/platform/packages/modules/Virtualization/+/refs/heads/main/android/virtmgr/src/crosvm.rs. Accessed 2026-09-23.
[20] crosvm. "Book of crosvm — Network". https://crosvm.dev/book/devices/net.html. Accessed 2026-09-23.
[21] Google. "command-fds" (crates.io API, reverse dependencies, docs.rs, src/lib.rs). https://crates.io/crates/command-fds ; https://docs.rs/command-fds ; https://github.com/google/command-fds. Accessed 2026-09-23.
[22] Rust project. "std::os::unix::process::CommandExt". https://doc.rust-lang.org/std/os/unix/process/trait.CommandExt.html. Accessed 2026-09-23.
[23] Kubernetes. "pkg/kubelet/kuberuntime/kuberuntime_manager.go"; "pkg/kubelet/kubelet_pods.go"; "pkg/kubelet/lifecycle/predicate.go"; "pkg/scheduler/backend/cache/interface.go"; "pkg/proxy/nftables/proxier.go". https://github.com/kubernetes/kubernetes. Accessed 2026-09-23.
[24] Kubernetes. PR #104577 "kubelet: Admission must exclude completed pods and avoid races". https://github.com/kubernetes/kubernetes/pull/104577. Accessed 2026-09-23.
[25] Kubernetes enhancements. KEP-3939 and KEP-3973. https://github.com/kubernetes/enhancements/tree/master/keps/sig-apps. Accessed 2026-09-23.
[26] Kubernetes docs. Finalizers; Pod Lifecycle; Nodes; Deployments; NodeLocal DNSCache; Network Plugins. https://kubernetes.io/docs/. Accessed 2026-09-23.
[27] CNI. "SPEC.md". https://github.com/containernetworking/cni/blob/main/SPEC.md. Accessed 2026-09-23.
[28] HashiCorp Nomad. "nomad/plan_apply.go", "nomad/structs/alloc.go", "nomad/state/schema.go"; GH #1225. https://github.com/hashicorp/nomad. Accessed 2026-09-23.
[29] HashiCorp. "Nomad scheduling concepts". https://developer.hashicorp.com/nomad/docs/concepts/scheduling/scheduling. Accessed 2026-09-23.
[30] Cilium. "plugins/cilium-cni/cmd/cmd.go". https://github.com/cilium/cilium/blob/main/plugins/cilium-cni/cmd/cmd.go. Accessed 2026-09-23.
[31] Cilium. "daemon/cmd/endpoint_restore.go", "pkg/controller/controller.go", "Documentation/sdpapi.rst", "Documentation/contributing/development/hive.rst" (local checkout `/Users/marcus/git/cilium/cilium` at `e99150f8`). Upstream https://github.com/cilium/cilium. Accessed 2026-09-23.
[32] Cilium docs. Endpoint Lifecycle; Kubernetes configuration; Upgrade guide. https://docs.cilium.io/en/stable/. Accessed 2026-09-23.
[33] Project Calico. "felix/config/config_params.go"; "api/pkg/apis/projectcalico/v3/felixconfig.go". https://github.com/projectcalico/calico. Accessed 2026-09-23.
[34] Istio. "cni/pkg/plugin/plugin.go". https://github.com/istio/istio/blob/master/cni/pkg/plugin/plugin.go. Accessed 2026-09-23.
[35] Istio docs. Ambient traffic redirection; Install CNI (race condition & mitigation); Security best practices. https://istio.io/latest/docs/. Accessed 2026-09-23.
[36] Istio. GH #53843 and GH #60882. https://github.com/istio/istio/issues/53843 ; https://github.com/istio/istio/issues/60882. Accessed 2026-09-23.
[37] Linkerd. "CNI plugin". https://linkerd.io/2-edge/features/cni/. Accessed 2026-09-23.
[38] AWS. "Choose an optimal Amazon EC2 node instance type — How maxPods is determined". https://docs.aws.amazon.com/eks/latest/userguide/choosing-instance-type.html. Accessed 2026-09-23.
[39] AWS. "amazon-vpc-cni-k8s docs/troubleshooting.md". https://github.com/aws/amazon-vpc-cni-k8s/blob/master/docs/troubleshooting.md. Accessed 2026-09-23.
[40] Erlang/OTP. "Supervisor Behaviour". https://www.erlang.org/doc/system/sup_princ.html. Accessed 2026-09-23.
[41] systemd. "man/systemd.unit.xml"; "rules.d/50-udev-default.rules.in". https://github.com/systemd/systemd. Accessed 2026-09-23.
[42] Tokio. "Graceful Shutdown". https://tokio.rs/tokio/topics/shutdown ; "tokio::fs::File". https://docs.rs/tokio/latest/tokio/fs/struct.File.html. Accessed 2026-09-23.
[43] Rust async fundamentals initiative. "Async drop". https://rust-lang.github.io/async-fundamentals-initiative/roadmap/async_drop.html. Accessed 2026-09-23.

## Research Metadata

- **Scope.** RQ1–RQ10 were researched. D-17 and kernel-version differences were excluded by instruction.
- **Sources.** About 75 sources were examined across fetches, searches, and local reads. 43 citation groups (about 70 distinct URLs or files) are cited.
- **Method.** Primary source code (kernel, CH, Firecracker, libvirt, Kata, AOSP, Kubernetes, Nomad, Cilium, Calico, Istio) was preferred over documentation, and documentation over secondary writing. Local reads were limited to the design artifacts and the user-supplied Cilium checkout.
- **Confidence distribution across major findings.** High ≈ 65%, Medium/Medium-High ≈ 32%, Low ≈ 3% (the Kata fd-close point).
- **Tool notes.**
  - The freedesktop.org man page returned 403, so the systemd source XML on GitHub was used instead.
  - pkg.go.dev withheld Nomad docs (license notice), so upstream Nomad source was used instead.
  - The Cilium lifecycle doc URL moved; the current URL was used.
  - One Nomad `structs.go` fetch truncated; `alloc.go` and `schema.go` were used instead.
- **Output.** `docs/research/networking/netns-density-295-replacement-design-prior-art-comprehensive-research.md`. No design artifact was edited, no issue was created, and nothing was committed.

---

## Addendum 2026-09-24 — verification of recalled facts

**Date**: 2026-09-24 | **Researcher**: nw-researcher (Nova) | **Depth**: detailed | **Scope**: seven technical facts that the revised replacement DESIGN (feature-delta § "Correctness-Recovery Replacement DESIGN", revision of 2026-09-24) states without a fetched source. Primary source code is authoritative. Kernel-version differences are out of scope by user ruling; the version read is named for each quote. No design artifact was edited, nothing was committed, and no GitHub action was taken.

**Summary.** Five of the seven recalled facts hold as stated (A1, A3, A6, A7, and the mechanism part of A4). A2 holds but needs two more preconditions. The Cloud Hypervisor Landlock hope (A5) is refuted. The facts that change what the DESIGN may rely on:
- **A5.** CH v53.0 grants `/dev/net/tun` `rw` whenever any `--net` device exists, so the ADR-0130 residual stands on open item 8's "if not" branch. CH's seccomp filter does omit `TUNSETOWNER`/`TUNSETGROUP`/`TUNSETPERSIST`/`TUNSETCARRIER`; thread coverage is unverified.
- **A2.** The `TIME_WAIT` side door is reachable in source through both `nft_tproxy` and `xt_TPROXY`, and R19's order does not affect it. E14 (e) must also make the guest complete its close and send a SYN with a newer sequence number or timestamp.
- **A4.** A vanished scope proves the VMM can no longer run. In the source read it does not prove that the VMM's descriptors are already closed.
- **A1.** ADR-0130's list of holder actions leaves out `TUNSETGROUP`.
- **A7.** `command-fds` 0.3.3's tokio support is confirmed from source. Its `nix` `dup2_raw` does not check errors.

### A1 — TUN ioctls on an attached queue fd (ADR-0130 / L8)

**Verdict: VERIFIED.** `TUNSETOWNER`, `TUNSETGROUP`, `TUNSETPERSIST`, and `TUNSETCARRIER` are not gated by any capability or owner check once the descriptor is attached. The only precondition is that the descriptor is attached to a device. A queue holder running as uid 4200 without `CAP_NET_ADMIN` can issue all four on its own TAP.

**Source read**: `drivers/net/tun.c`, Linux tag `v6.18`. <https://github.com/torvalds/linux/blob/v6.18/drivers/net/tun.c> (fetched as raw text). Accessed 2026-09-24.

**Evidence (verbatim).**
- The gate before the `switch` is attachment, not capability. `tun_get` returns the device only when this file is attached (`tun = rcu_dereference(tfile->tun); if (tun) dev_hold(tun->dev);`). `__tun_chr_ioctl` then does:
  ```c
  	rtnl_lock();

  	tun = tun_get(tfile);
  	...
  	ret = -EBADFD;
  	if (!tun)
  		goto unlock;

  	netif_info(tun, drv, tun->dev, "tun_chr_ioctl cmd %u\n", cmd);

  	net = dev_net(tun->dev);
  	ret = 0;
  	switch (cmd) {
  ```
- The four arms contain no check:
  ```c
  	case TUNSETPERSIST:
  		if (arg && !(tun->flags & IFF_PERSIST)) {
  			tun->flags |= IFF_PERSIST;
  			__module_get(THIS_MODULE);
  			do_notify = true;
  		}
  		if (!arg && (tun->flags & IFF_PERSIST)) {
  			tun->flags &= ~IFF_PERSIST;
  			module_put(THIS_MODULE);
  			do_notify = true;
  		}
  		...
  	case TUNSETOWNER:
  		owner = make_kuid(current_user_ns(), arg);
  		if (!uid_valid(owner)) {
  			ret = -EINVAL;
  			break;
  		}
  		tun->owner = owner;
  		do_notify = true;
  		...
  	case TUNSETGROUP:
  		group = make_kgid(current_user_ns(), arg);
  		if (!gid_valid(group)) {
  			ret = -EINVAL;
  			break;
  		}
  		tun->group = group;
  		...
  	case TUNSETCARRIER:
  		ret = -EFAULT;
  		if (copy_from_user(&carrier, argp, sizeof(carrier)))
  			goto unlock;
  		ret = tun_net_change_carrier(tun->dev, (bool)carrier);
  		break;
  ```
- `tun_net_change_carrier` refuses carrier-on only when no queue is attached (`if (!tun->numqueues) return -EPERM;`). The holder's own attached queue satisfies that, so the holder can raise and lower carrier. The function touches carrier only.
- In `__tun_chr_ioctl` the only capability checks are for `SIOCGSKNS` (before the switch) and `TUNGETDEVNETNS` (`ret = -EPERM; if (!ns_capable(net->user_ns, CAP_NET_ADMIN)) goto unlock;`). The owner check `tun_not_capable(tun)` is called only from `tun_set_iff`, when attaching (`if (tun_not_capable(tun)) return -EPERM; err = security_tun_dev_open(tun->security);`).
- The full list of switch arms is `TUNGETIFF`, `TUNSETNOCSUM`, `TUNSETPERSIST`, `TUNSETOWNER`, `TUNSETGROUP`, `TUNSETLINK`, `TUNSETDEBUG`, `TUNSETOFFLOAD`, `TUNSETTXFILTER`, `SIOCGIFHWADDR`, `SIOCSIFHWADDR`, `TUNGETSNDBUF`, `TUNSETSNDBUF`, `TUNATTACHFILTER`, `TUNDETACHFILTER`, `TUNGETFILTER`, `TUNSETSTEERINGEBPF`, `TUNSETFILTEREBPF`, `TUNSETCARRIER`, `TUNGETDEVNETNS`, and `default` (`tun_vnet_ioctl`). None sets `IFF_UP`. This supports ADR-0130's "`TUNSETCARRIER` toggles carrier only".

**Refinements to ADR-0130's wording (facts, not recommendations).**
- `TUNSETOWNER` accepts any uid valid in the caller's user namespace (`make_kuid(current_user_ns(), arg)`). A holder can hand the TAP to its own uid 4200, as the ADR says, and equally to any other uid.
- `TUNSETGROUP` is equally unchecked. ADR-0130 lists three effects (owner, persist, carrier). A group grant is a fourth route to the same result as `TUNSETOWNER`: `tun_not_capable` admits a caller that matches the group even when the owner does not match. The DESIGN says the audit reads back the owner uid and persistence; whether it also reads back the group was not checked here.
- The same unchecked set also includes `SIOCSIFHWADDR` (`ret = dev_set_mac_address_user(tun->dev, …)`, the host-side MAC) and `TUNSETLINK`. `TUNSETLINK` changes `dev->type`, but only while the interface is down (`if (tun->dev->flags & IFF_UP) … ret = -EBUSY;`). Before activation the #295 TAP is down, so the holder can use `TUNSETLINK` during that window. These two are outside the four asked about and are recorded only as observations.

**LSM caveat.** A generic `security_file_ioctl` hook runs on every `ioctl(2)`, so an LSM policy (SELinux, AppArmor) could still refuse these commands. Landlock's `LANDLOCK_ACCESS_FS_IOCTL_DEV` cannot. The kernel Landlock documentation says that right, like `LANDLOCK_ACCESS_FS_TRUNCATE`, is checked when the file is opened, so a descriptor opened before the ruleset (the inherited queue) is unaffected (see A5). Cloud Hypervisor's seccomp filter is the remaining in-VMM control; A5 records what was and was not read.

**Cross-check.** The WebFetch listing of every `capable(CAP_NET_ADMIN)` and `tun_not_capable(` call in `tun.c` placed each one in `tun_set_iff` or in the two `__tun_chr_ioctl` sites above. That listing left out the definition of `tun_not_capable` itself, so it is not exhaustive and serves only as a secondary check. The primary evidence is the verbatim switch arms.
**Confidence**: High. Primary source; the arms are unambiguous.
**Affects**: ADR-0130 § "The protection is not independent of the queue holder's confinement" and feature-delta D-295-R4 / L8. The DESIGN's assumption holds. The ADR's list of holder actions leaves out `TUNSETGROUP`.

### A2 — TPROXY and TIME_WAIT (E14 case e / ADR-0140)

**Verdict: VERIFIED as reachable in source, with two conditions the DESIGN does not list.** A guest SYN that reuses the 4-tuple of a transparent `TIME_WAIT` socket is assigned to that `TIME_WAIT` socket when no listener exists at the TPROXY target. TCP input then looks up a listener on the packet's own destination address and port, falling back to `INADDR_ANY`, and hands the SYN to it. `nft_tproxy` and `xt_TPROXY` behave identically in this case. The case also does not depend on R19's order: TPROXY *succeeds* on the `TIME_WAIT` socket, so the mark is set under both the old and the R19 order.

**Sources read** (Linux tag `v6.18`, fetched as raw text, accessed 2026-09-24). The helper is at `net/ipv4/netfilter/nf_tproxy_ipv4.c`, not `net/netfilter/`.
- [nf_tproxy_ipv4.c](https://github.com/torvalds/linux/blob/v6.18/net/ipv4/netfilter/nf_tproxy_ipv4.c)
- [nft_tproxy.c](https://github.com/torvalds/linux/blob/v6.18/net/netfilter/nft_tproxy.c)
- [xt_TPROXY.c](https://github.com/torvalds/linux/blob/v6.18/net/netfilter/xt_TPROXY.c)
- [include/net/netfilter/nf_tproxy.h](https://github.com/torvalds/linux/blob/v6.18/include/net/netfilter/nf_tproxy.h)
- [include/net/tcp.h](https://github.com/torvalds/linux/blob/v6.18/include/net/tcp.h)
- [net/ipv4/inet_timewait_sock.c](https://github.com/torvalds/linux/blob/v6.18/net/ipv4/inet_timewait_sock.c)
- [net/core/sock.c](https://github.com/torvalds/linux/blob/v6.18/net/core/sock.c)
- [net/ipv4/tcp_ipv4.c](https://github.com/torvalds/linux/blob/v6.18/net/ipv4/tcp_ipv4.c)
- [net/ipv4/tcp_minisocks.c](https://github.com/torvalds/linux/blob/v6.18/net/ipv4/tcp_minisocks.c)
- [net/ipv4/inet_hashtables.c](https://github.com/torvalds/linux/blob/v6.18/net/ipv4/inet_hashtables.c)

**The path, step by step (verbatim excerpts).**
1. **Established lookup first, and it can return a `TIME_WAIT` socket.** In `nft_tproxy_eval_v4`:
   ```c
   	sk = nf_tproxy_get_sock_v4(nft_net(pkt), skb, iph->protocol,
   				   iph->saddr, iph->daddr,
   				   hp->source, hp->dest,
   				   skb->dev, NF_TPROXY_LOOKUP_ESTABLISHED);
   	...
   	/* UDP has no TCP_TIME_WAIT state, so we never enter here */
   	if (sk && sk->sk_state == TCP_TIME_WAIT) {
   		/* reopening a TIME_WAIT connection needs special handling */
   		sk = nf_tproxy_handle_time_wait4(nft_net(pkt), skb, taddr, tport, sk);
   	} else if (!sk) {
   ```
   The established lookup is keyed on the packet's own 4-tuple (`inet_lookup_established(net, saddr, sport, daddr, dport, in->ifindex)`). The leg-F socket that produced the `TIME_WAIT` entry was accepted with the guest's original destination as its local address, so a SYN that reuses that 4-tuple finds it.
2. **Replacement by a listener only if one exists at the TPROXY target.**
   ```c
   	if (hp->syn && !hp->rst && !hp->ack && !hp->fin) {
   		/* SYN to a TIME_WAIT socket, we'd rather redirect it
   		 * to a listener socket if there's one */
   		struct sock *sk2;

   		sk2 = nf_tproxy_get_sock_v4(net, skb, iph->protocol,
   					    iph->saddr, laddr ? laddr : iph->daddr,
   					    hp->source, lport ? lport : hp->dest,
   					    skb->dev, NF_TPROXY_LOOKUP_LISTENER);
   		if (sk2) {
   			nf_tproxy_twsk_deschedule_put(inet_twsk(sk));
   			sk = sk2;
   		}
   	}

   	return sk;
   ```
   With leg F absent (no socket on `127.0.0.1:<leg-F port>`), `sk2` is `NULL` and the `TIME_WAIT` socket is returned. **Edge (observation):** the listener lookup falls back to `INADDR_ANY` (step 5 shows the lookup function), and the lookup comment says "we return listeners even if bound to 0.0.0.0". So if a *non*-transparent listener holds the leg-F port, including one bound to `0.0.0.0`, that listener replaces the `TIME_WAIT` socket. It then fails the transparency check, and the rule breaks or drops rather than delivering.
3. **The transparency check passes for a `TIME_WAIT` socket that came from a transparent connection.**
   ```c
   	if (sk && nf_tproxy_sk_is_transparent(sk))
   		nf_tproxy_assign_sock(skb, sk);
   	else
   		regs->verdict.code = NFT_BREAK;
   ```
   `nf_tproxy_sk_is_transparent` calls `inet_sk_transparent`, which is defined in `include/net/tcp.h` (not `inet_sock.h`):
   ```c
   static inline bool inet_sk_transparent(const struct sock *sk)
   {
   	switch (sk->sk_state) {
   	case TCP_TIME_WAIT:
   		return inet_twsk(sk)->tw_transparent;
   	...
   	return inet_test_bit(TRANSPARENT, sk);
   }
   ```
   `inet_twsk_alloc` copies the bit from the closing socket: `tw->tw_transparent  = inet_test_bit(TRANSPARENT, sk);`. The accepted socket carries its listener's `TRANSPARENT` bit because `sock_copy` copies every byte of the listener outside the `sk_dontcopy_begin`…`sk_dontcopy_end` window, up to `prot->obj_size` (`memcpy(nsk, osk, offsetof(struct sock, sk_dontcopy_begin)); unsafe_memcpy(&nsk->sk_dontcopy_end, …, prot->obj_size - offsetof(struct sock, sk_dontcopy_end), …)`). *Interpretation:* `inet_flags` lives in `struct inet_sock`, beyond `struct sock`, so it is inside the copied range. No later clearing of `TRANSPARENT` in the accept path was looked for, so that one link is reasoned rather than quoted.
4. **Assignment, then local delivery to the `TIME_WAIT` socket.** `nf_tproxy_assign_sock` does `skb_orphan(skb); skb->sk = sk; skb->destructor = sock_edemux;`. `tcp_v4_rcv` takes the pre-assigned socket through `__inet_lookup_skb` and branches: `if (sk->sk_state == TCP_TIME_WAIT) goto do_time_wait;`. *Interpretation:* `__inet_lookup_skb` returning the `skb->sk` that TPROXY set, rather than doing a fresh lookup, is the documented steal-socket behaviour; the `inet_steal_sock` body was not reproduced here.
5. **`TCP_TW_SYN` looks up a listener on the packet's own destination and hands the SYN to it.**
   ```c
   	case TCP_TW_SYN: {
   		struct sock *sk2 = inet_lookup_listener(net, skb, __tcp_hdrlen(th),
   							iph->saddr, th->source,
   							iph->daddr, th->dest,
   							inet_iif(skb),
   							sdif);
   		if (sk2) {
   			inet_twsk_deschedule_put(inet_twsk(sk));
   			sk = sk2;
   			tcp_v4_restore_cb(skb);
   			refcounted = false;
   			__this_cpu_write(tcp_tw_isn, isn);
   			goto process;
   		}
   ```
   The listener lookup falls back to the wildcard address:
   ```c
   	/* Lookup lhash2 with INADDR_ANY */
   	hash2 = ipv4_portaddr_hash(net, htonl(INADDR_ANY), hnum);
   	ilb2 = inet_lhash2_bucket(hashinfo, hash2);

   	result = inet_lhash2_lookup(net, ilb2, skb, doff,
   				    saddr, sport, htonl(INADDR_ANY), hnum,
   				    dif, sdif);
   ```
   Nothing in this branch tests the listener's transparency or whether the destination address is a host address. So a host listener bound to `0.0.0.0` on the original destination port receives the SYN and completes the handshake (`goto process`).

**Two conditions the DESIGN's list of four omits (from `tcp_timewait_state_process`).**
- **The SYN must be acceptable as a reopen.** `TCP_TW_SYN` is returned only for:
  ```c
  	if (th->syn && !th->rst && !th->ack && !paws_reject &&
  	    (after(TCP_SKB_CB(skb)->seq, rcv_nxt) ||
  	     (tmp_opt.saw_tstamp &&
  	      (s32)(READ_ONCE(tcptw->tw_ts_recent) - tmp_opt.rcv_tsval) < 0))) {
  ```
  That means a sequence number after the old `rcv_nxt`, or a newer timestamp, and no PAWS rejection. A malicious guest controls its ISN and timestamps, so this condition is attacker-satisfiable. It is still a precondition that E14 (e)'s probe must meet: a naive reconnect with an older ISN gets an ACK (`TCP_TW_ACK`), not a listener.
- **The `TIME_WAIT` entry must be in the true `TIME_WAIT` substate, not `FIN_WAIT2`.** In `FIN_WAIT2` substate: `if (th->syn && !before(TCP_SKB_CB(skb)->seq, rcv_nxt)) return TCP_TW_RST;`. So the guest must have completed its own close (sent its FIN) before the probe SYN. The DESIGN's "host side closed first" is necessary but not sufficient.

**`xt_TPROXY` in the same case.** `tproxy_tg4` performs the same established lookup and the same `nf_tproxy_handle_time_wait4` call. The `TIME_WAIT` socket passes the same transparency check, and the target then marks and accepts:
```c
	if (sk && nf_tproxy_sk_is_transparent(sk)) {
		/* This should be in a separate target, but we don't do multiple
		   targets on the same rule yet */
		skb->mark = (skb->mark & ~mark_mask) ^ mark_value;
		nf_tproxy_assign_sock(skb, sk);
		return NF_ACCEPT;
	}
```
*Interpretation:* for case (e), `xt_TPROXY`'s "drop on failure" gives no protection, because this is not a failure: TPROXY succeeds on the `TIME_WAIT` socket. Local delivery then follows the same `tcp_v4_rcv` path.

**Independence from R19 (interpretation from the code above).** Under the current order (mark, then `tproxy`), the mark is already set and `tproxy` succeeds, so the result is `accept`. Under R19 (`tproxy`, then mark, then `accept`), `tproxy` succeeds and the mark is set. Either way the packet is marked `0x1`, routed locally, and reaches step 5. R19 neither opens nor closes case (e). This matches the DESIGN's decision to judge (e) on its own RED.

**Confidence**: High for steps 1, 2, 3 (`tw_transparent`), and 5, and for both extra conditions (verbatim primary source). Medium for the accepted-socket inheritance link in step 3 and the steal-socket link in step 4 (reasoned from quoted code, not quoted end to end). Not executed natively.
**Affects**: feature-delta "The TIME_WAIT side door (review finding L3)", E14 case (e), and ADR-0140. The DESIGN's claims hold as stated. E14 (e)'s reproduction procedure needs the two extra conditions: the guest completes its own close, and the probe SYN uses a sequence number above the old `rcv_nxt` (or a newer timestamp).

### A3 — `nft_tproxy` / `xt_TPROXY` failure verdicts and mark persistence (ADR-0140 / M1)

**Verdict: VERIFIED (all three parts).**

**Sources read** (Linux tag `v6.18`, accessed 2026-09-24):
- `net/netfilter/nft_tproxy.c`, `net/netfilter/xt_TPROXY.c` (URLs as in A2).
- [net/netfilter/nf_tables_core.c](https://github.com/torvalds/linux/blob/v6.18/net/netfilter/nf_tables_core.c).
- [net/netfilter/nft_meta.c](https://github.com/torvalds/linux/blob/v6.18/net/netfilter/nft_meta.c).

**Evidence (verbatim).**
- **`nft_tproxy` returns `NFT_BREAK` when no transparent socket is found.** The final statement of `nft_tproxy_eval_v4` is:
  ```c
  	if (sk && nf_tproxy_sk_is_transparent(sk))
  		nf_tproxy_assign_sock(skb, sk);
  	else
  		regs->verdict.code = NFT_BREAK;
  ```
  It also breaks for a non-TCP/UDP transport, a missing header, and a family mismatch in `nft_tproxy_eval`. On success it sets **no** verdict, so evaluation continues to the rule's next expression. This is why R19's explicit `accept` after the mark is needed.
- **`xt_TPROXY` returns `NF_DROP`.** `tproxy_tg4` ends with `return NF_DROP;` after the transparent-socket branch (quoted in A2). It also drops when the transport header is missing (`if (hp == NULL) return NF_DROP;`). It sets the mark only on success.
- **A mark set earlier in the rule survives `NFT_BREAK`.** `meta mark set` writes the packet immediately:
  ```c
  	switch (meta->key) {
  	case NFT_META_MARK:
  		skb->mark = value;
  		break;
  ```
  The rule loop in `nft_do_chain` stops evaluating the rule's remaining expressions and moves to the next rule, with no rollback of effects already applied to the `skb`:
  ```c
  			if (regs.verdict.code != NFT_CONTINUE)
  				break;
  		}

  		switch (regs.verdict.code) {
  		case NFT_BREAK:
  			regs.verdict.code = NFT_CONTINUE;
  			nft_trace_copy_nftrace(pkt, &info);
  			continue;
  ```
  So `meta mark set 0x1` placed *before* `tproxy` leaves `skb->mark == 0x1` after a `NFT_BREAK`. The corollary also holds: with R19's order, `meta mark set` placed *after* `tproxy` never runs on `NFT_BREAK`, so the packet keeps `0x295a`.

**Confidence**: High (primary source, three files).
**Affects**: ADR-0140 / M1 and feature-delta "Hazard 2". The DESIGN's statements ("`regs->verdict.code = NFT_BREAK`, which ends the rule without a verdict, and the mark set earlier in the same rule survives"; "`xt_TPROXY` … with no transparent socket it returns `NF_DROP`") are accurate. A2 qualifies ADR-0140's reach: the `TIME_WAIT` case is a TPROXY *success*.

### A4 — cgroup v2 kill and removal (H1)

**Verdict: VERIFIED for both mechanisms. The inference is sound for "no scope process can run again". It is not sound for "every resource those processes held is released".**
- `rmdir` is refused with `EBUSY` while the cgroup shows any task.
- `cgroup.kill` only sends `SIGKILL`. It does not wait for exit.
- "Scope directory absent (`ENOENT`)" proves that no process that was in the scope can execute user code again, *provided no process was migrated out of the scope first*.
- In the source read, the directory can disappear while a dying VMM task is still inside `do_exit`, before its memory and file table are torn down. So the directory's absence does not by itself prove that the VMM's descriptors, including its TAP queue, are already closed.

**Source read.**
- Local checkout `/Users/marcus/git/linux`, `Makefile` `VERSION = 7` `PATCHLEVEL = 2` `SUBLEVEL = 0`: `kernel/cgroup/cgroup.c`, `kernel/exit.c`, `Documentation/admin-guide/cgroup-v2.rst`. Upstream: <https://github.com/torvalds/linux/blob/master/kernel/cgroup/cgroup.c>, <https://github.com/torvalds/linux/blob/master/kernel/exit.c>, <https://docs.kernel.org/admin-guide/cgroup-v2.html>.
- v6.18's `cgroup.c` could not be fetched whole: WebFetch truncated the file. By user ruling, kernel-version differences are out of scope. The version read is 7.2.

**Evidence (verbatim, 7.2 tree).**
- **`rmdir` refusal.** `cgroup_destroy_locked` ("destroy @cgrp (called on rmdir)"):
  ```c
  	css_task_iter_start(&cgrp->self, 0, &it);
  	task = css_task_iter_next(&it);
  	css_task_iter_end(&it);
  	if (task)
  		return -EBUSY;

  	/*
  	 * Make sure there's no live children.  ...
  	 */
  	if (css_has_online_children(&cgrp->self))
  		return -EBUSY;
  ```
  Its header comment states the contract: "Userspace: rmdir must succeed when cgroup.procs and friends are empty", and "A task hidden from cgroup.procs (past exit_signals() with signal->live cleared) can still schedule, allocate, and consume resources until its final context switch."
- **Which tasks count.** The iterator hides exiting tasks by default:
  ```c
  	/*
  	 * Hide tasks that are exiting but not yet removed by default. Keep
  	 * zombie leaders with live threads visible. Usages that need to walk
  	 * every existing task can opt out via CSS_TASK_ITER_WITH_DEAD.
  	 */
  	if (!(it->flags & CSS_TASK_ITER_WITH_DEAD) &&
  	    (task->flags & PF_EXITING) && !atomic_read(&task->signal->live))
  		goto repeat;
  ```
- **Where that point falls in `do_exit` (`kernel/exit.c`).** Line numbers are from the 7.2 tree:
  - 950: `exit_signals(tsk);  /* sets PF_EXITING */`
  - 955: `group_dead = atomic_dec_and_test(&tsk->signal->live);`
  - 996: `exit_mm();`
  - 1003: `exit_files(tsk);`
  - 1008: `exit_task_work(tsk);`
  - 1012: `cgroup_task_exit(tsk);`
  - 1020: `exit_notify(tsk, group_dead);`

  The task is unlinked from its css_set only in `do_cgroup_task_dead` (`css_set_move_task(tsk, cset, NULL, false); cset->nr_tasks--;`), reached from `cgroup_task_dead()`, which runs "from finish_task_switch()". *Interpretation:* once the last thread of a VMM process passes line 955, every thread is `PF_EXITING` with `signal->live == 0`, so the process is hidden and `rmdir` can succeed. Lines 996–1008 (address-space teardown, closing the file table, running the deferred `fput` work) may still be pending at that moment.
- **Documentation.** `cgroup-v2.rst`: "A cgroup which doesn't have any children or live processes can be destroyed by removing the directory. Note that a cgroup which doesn't have any children and is associated only with zombie processes is considered empty and can be removed". This is the sentence the DESIGN cites, and it matches.
- **`cgroup.kill` is asynchronous.** It only signals:
  ```c
  	css_task_iter_start(&cgrp->self, CSS_TASK_ITER_PROCS | CSS_TASK_ITER_THREADED, &it);
  	while ((task = css_task_iter_next(&it))) {
  		/* Ignore kernel threads here. */
  		if (task->flags & PF_KTHREAD)
  			continue;

  		/* Skip tasks that are already dying. */
  		if (__fatal_signal_pending(task))
  			continue;

  		send_sig(SIGKILL, task, 0);
  	}
  ```
  `cgroup_kill` applies this to every live descendant (`cgroup_for_each_live_descendant_pre(dsct, css, cgrp) __cgroup_kill(dsct);`), and `cgroup_kill_write` returns `nbytes` right after. Two further details:
  - `cgroup_kill_write` returns `-ENOENT` when the cgroup is no longer live (`cgrp = cgroup_kn_lock_live(of->kn, false); if (!cgrp) return -ENOENT;`), and `-EOPNOTSUPP` on a threaded cgroup.
  - A fork that races the kill is handled in `cgroup_post_fork`: `/* Cgroup has to be killed so take down child immediately. */ if (unlikely(kill)) do_send_sig_info(SIGKILL, SEND_SIG_NOINFO, child, PIDTYPE_TGID);`. The `kill` flag comes from `kill_seq`; that derivation was not quoted.

  The documentation agrees: "all processes located in the affected cgroup tree will be killed via SIGKILL. Killing a cgroup tree will deal with concurrent forks appropriately and is protected against migrations." Completion is observed through `cgroup.events` `populated` ("This can be used, for example, to start a clean-up operation after all processes of a given sub-hierarchy have exited").

**Soundness of "`ENOENT` ⇒ the VMM is gone" (interpretation, labelled).**
1. **Sound for "cannot act".** An `rmdir` that succeeded means no task in the scope was visible to the iterator. Every such task either never existed or is `PF_EXITING` with a zero live count, so it is inside `do_exit` and cannot run user code again. The DESIGN's "whoever removed the scope, its VMM had exited first" holds in that sense.
2. **Not sound for "resources released", in the version read.** The DESIGN's quiescence argument also needs the VMM's TAP queue to be closed. The file-table teardown and the deferred `fput` work (`exit.c` lines 1003 and 1008) can run after the directory is gone. The window is bounded by the task's own exit path, but the directory's absence does not order after it. The completion signals that do order after teardown are these:
   - `populated 0` in `cgroup.events`. `css_set_move_task` calls `css_set_update_populated(from_cset, false)` when the last task leaves (`list_del_init(&task->cg_list); if (!css_set_populated(from_cset)) css_set_update_populated(from_cset, false);`). It runs from `do_cgroup_task_dead`, after the task's final context switch, which is after `exit_files`.
   - The parent's `wait`/`waitid`. *Reasoned:* `exit_notify` (`exit.c` line 1020) comes after `exit_files`.

   *Whether this matters depends on what the DESIGN does next with `ENOENT`.* If the next step reuses or re-attaches the TAP, or asserts that no process holds it, it can race the dying VMM's `exit_files`. A reuse is refused with `EBUSY` by `tun_attach` while the old queue is still attached (research F2.1), so the failure is loud rather than a leak, but it is a failure.
3. **Migration caveat.** `rmdir` succeeds just as well if the VMM was *moved* out of the scope. The inference therefore also assumes that no actor migrates VMM processes out. *Reasoned, not verified here:* an unprivileged uid 4200 cannot write root-owned `cgroup.procs` files without delegation, so a compromised VMM cannot move itself out.

**Confidence**: High for `EBUSY`, for `cgroup.kill` being signal-only, and for the documentation text (primary source plus official documentation). Medium for the exit-ordering interpretation (source lines quoted, composed by reasoning, not executed). The migration caveat is reasoned.
**Affects**: feature-delta "Why an absent scope proves the VMM gone (review finding H1)" and `VmKillCapability::kill_allocation` ("An absent scope is `Ok`: the VMM has already exited"). The DESIGN's claim holds for "exited / cannot act". It is too strong if it is also read as "its descriptors are closed".

### A5 — Cloud Hypervisor Landlock on the `fd=` path (ADR-0130 residual)

**Verdict: REFUTED. Cloud Hypervisor's own Landlock ruleset *allows* `/dev/net/tun` read-write whenever any `--net` device is configured, whether the device uses `fd=` or `tap=`.** Under CH's Landlock alone, a compromised VMM can open `/dev/net/tun`. Whether it can then attach a queue is decided by the kernel (`tun_not_capable`; with owner uid 0 it cannot, unless the owner was changed per A1) and by CH's seccomp filter (below).

**Source read**: Cloud Hypervisor tag **`v53.0`** (the tag exists; raw fetch succeeded), cross-checked on `main`. Accessed 2026-09-24.
- `vmm/src/vm_config.rs` holds the rule generation, not `vmm/src/config.rs`: <https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v53.0/vmm/src/vm_config.rs>.
- <https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v53.0/vmm/src/landlock.rs>
- <https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v53.0/vmm/src/seccomp_filters.rs>
- <https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v53.0/virtio-devices/src/seccomp_filters.rs>
- <https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/vmm/src/vm_config.rs>

**Evidence (verbatim).**
- **The rule is conditioned on the presence of net config only.** In `VmConfig::apply_landlock`:
  ```rust
      if self.net.is_some() {
          landlock.add_rule_with_access(Path::new("/dev/net/tun"), "rw")?;
      }

      if let Some(landlock_rules) = &self.landlock_rules {
          for landlock_rule in landlock_rules.iter() {
              landlock_rule.apply_landlock(&mut landlock)?;
          }
      }

      landlock.restrict_self()?;
  ```
  There is no `impl ApplyLandlock for NetConfig` in `vm_config.rs` (disks, memory zones, rng, fs, vhost-user, pmem, consoles, devices, user devices, vdpa, vsock, payload, and TPM each have one), so nothing distinguishes `fd=` from `tap=`. The same three lines are on `main`.
- **`"rw"` grants the read and write access sets of ABI v3.** `static ABI: ABI = ABI::V3;`, with `'r' => access |= AccessFs::from_read(ABI)` and `'w' => access |= AccessFs::from_write(ABI)`. The ruleset handles `AccessFs::from_all(ABI)` with `CompatLevel::HardRequirement`.
- **User rules cannot remove the grant.** `--landlock-rules` (`self.landlock_rules`) is added to the *same* ruleset, and Landlock rules within one ruleset only add access. *Interpretation:* only a separate ruleset stacked by the launcher before `exec` could deny `/dev/net/tun`. Whether Overdrive's launcher applies one was not checked; the DESIGN describes only CH `--landlock` rules (TAP sysfs `r`, run directory `rw`).

**Related facts that bear on the ADR-0130 residual (observations, not asked).**
- **Landlock does not restrict ioctls on the inherited queue.**
  - First, CH uses ABI v3. `LANDLOCK_ACCESS_FS_IOCTL_DEV` exists only from ABI v5 (kernel doc: "Starting with the Landlock ABI version 5, it is possible to restrict the use of ioctl(2) on character and block devices"), so CH's ruleset does not handle it at all.
  - Second, even at ABI v5 the right is bound at open time: "When opening a file, the availability of the `LANDLOCK_ACCESS_FS_TRUNCATE` and `LANDLOCK_ACCESS_FS_IOCTL_DEV` rights is associated with the newly created file descriptor", and "it only applies to *newly opened* device files. This means specifically that pre-existing file descriptors like stdin, stdout and stderr are unaffected."
  - Source: `Documentation/userspace-api/landlock.rst` (local 7.2 tree; upstream <https://docs.kernel.org/userspace-api/landlock.html>). The inherited queue descriptor is opened by the parent before `restrict_self`.
- **CH's seccomp filter does not permit the A1 ioctls.**
  - `vmm/src/seccomp_filters.rs` allows exactly five TUN ioctls, all in `create_vmm_ioctl_seccomp_rule_common()`: `and![Cond::new(1, ArgLen::Dword, Eq, TUNGETFEATURES as _)?]`, and likewise for `TUNGETIFF`, `TUNSETIFF`, `TUNSETOFFLOAD`, and `TUNSETVNETHDRSZ`.
  - `virtio-devices/src/seccomp_filters.rs` allows only `TUNSETOFFLOAD`, in `create_virtio_net_ctl_ioctl_seccomp_rule()`. The virtio-net data-thread rules have no `SYS_ioctl` except under `sev_snp`, but allow `SYS_openat` through `virtio_thread_common()`.
  - `TUNSETOWNER`, `TUNSETGROUP`, `TUNSETPERSIST`, and `TUNSETCARRIER` appear in neither file (per the fetched listings).
  - The VMM thread *does* allow `TUNSETIFF`. Combined with the Landlock grant above, a compromised VMM thread can open `/dev/net/tun` and try an attach by name. The kernel then refuses it for an owner-0 TAP (A1; research F2.2).
- **Limits of the seccomp observation (unverified).**
  - The two listings were produced by WebFetch summarisation of whole files, not a line-by-line read.
  - Overdrive's `--seccomp` setting for CH was not checked.
  - Seccomp is per-thread. Whether every CH thread, including the main thread, runs under a filter was not verified. A code-execution attacker in a shared address space could pivot to an unfiltered thread if one exists.
  - So this is a mitigation *observed in CH's filter source*, not a guarantee the DESIGN can rely on without a native check.

**Confidence**: High for the Landlock verdict (verbatim on the `v53.0` tag and on `main`). Medium for the seccomp observations (summarised listings; thread coverage and runtime flags unverified).
**Affects**: ADR-0130's residual ("depends on whether Cloud Hypervisor's own Landlock ruleset denies `/dev/net/tun` on the `fd=` path") and feature-delta open item 8. The ruleset does not deny it, so the residual "stands as stated in the security table" (item 8's "if not" branch). CH's seccomp filter omits `TUNSETOWNER`/`TUNSETGROUP`/`TUNSETPERSIST`/`TUNSETCARRIER`, which may narrow A1's holder actions in practice. That rests on unverified thread coverage and runtime flags.

### A6 — `close_range(CLOSE_RANGE_CLOEXEC)` in `pre_exec` (ADR-0129)

**Verdict: VERIFIED in substance, with one wording caveat.**
- **Semantics and flags.** `close_range(first, last, CLOSE_RANGE_CLOEXEC)` sets close-on-exec on every descriptor slot from `first` to `last` inclusive, without closing anything.
- **Failure modes.** For the DESIGN's exact call, `close_range(4, u32::MAX, CLOSE_RANGE_CLOEXEC)`, the kernel has no reachable error path on a kernel that implements the flag.
- **Safety between fork and exec.** It is a single system call that neither allocates in user space nor takes a user-space lock. glibc itself issues it as a raw syscall in its post-clone `posix_spawn` child.
- **Caveat.** It is *not* on the POSIX / `signal-safety(7)` async-signal-safe list, because it is not a POSIX function. The DESIGN's `SAFETY` wording ("calls only the async-signal-safe `close_range`") is accurate in substance but not a formal list membership.

**Sources.**
- `close_range(2)`: <https://man7.org/linux/man-pages/man2/close_range.2.html>
- `fork(2)`: <https://man7.org/linux/man-pages/man2/fork.2.html>
- `signal-safety(7)`: <https://man7.org/linux/man-pages/man7/signal-safety.7.html>
- Kernel `fs/file.c`, local 7.2 tree (upstream <https://github.com/torvalds/linux/blob/master/fs/file.c>)
- glibc `sysdeps/unix/sysv/linux/spawni.c`, `master`, via the GitHub mirror: <https://github.com/bminor/glibc/blob/master/sysdeps/unix/sysv/linux/spawni.c>

All accessed 2026-09-24.

**Evidence (verbatim).**
- **Semantics** (`close_range(2)`):
  - "The close_range() system call closes all open file descriptors from *first* to *last* (included)."
  - `CLOSE_RANGE_CLOEXEC`: "Set the close-on-exec flag on the specified file descriptors, rather than immediately closing them."
  - `CLOSE_RANGE_UNSHARE`: "Unshare the specified file descriptors from any other processes before closing them".
  - HISTORY: "FreeBSD. Linux 5.9, glibc 2.34." and "CLOSE_RANGE_CLOEXEC (since Linux 5.11)". STANDARDS: "None."
- **Kernel implementation** (`fs/file.c`, 7.2 tree):
  ```c
  SYSCALL_DEFINE3(close_range, unsigned int, fd, unsigned int, max_fd,
  		unsigned int, flags)
  {
  	...
  	if (flags & ~(CLOSE_RANGE_UNSHARE | CLOSE_RANGE_CLOEXEC))
  		return -EINVAL;

  	if (fd > max_fd)
  		return -EINVAL;

  	if ((flags & CLOSE_RANGE_UNSHARE) && atomic_read(&cur_fds->count) > 1) {
  		...
  		fds = dup_fd(cur_fds, punch_hole);
  		if (IS_ERR(fds))
  			return PTR_ERR(fds);
  		...
  	}

  	if (flags & CLOSE_RANGE_CLOEXEC)
  		__range_cloexec(cur_fds, fd, max_fd);
  	...
  	return 0;
  }
  ```
  `__range_cloexec` clamps to the table size and sets bits under the kernel's own spinlock: `max_fd = min(last_fd(fdt), max_fd); if (fd <= max_fd) bitmap_set(fdt->close_on_exec, fd, max_fd - fd + 1);`.
- **Failure modes.** `close_range(2)` ERRORS: `EINVAL` ("*flags* is not valid, or *first* is greater than *last*"), `EMFILE` ("The number of open file descriptors exceeds the limit specified in /proc/sys/fs/nr_open"), `ENOMEM` ("Insufficient kernel memory was available."). *From the source above:* `EMFILE` and `ENOMEM` can only come from `dup_fd` on the `CLOSE_RANGE_UNSHARE` path. With `CLOSE_RANGE_CLOEXEC` alone, `first = 4`, and `last = u32::MAX`, both `EINVAL` checks are false, so the call returns 0.
- **Failure modes outside the man page (interpretation).**
  - `ENOSYS` on a kernel without the syscall, and `EINVAL` for the flag on a kernel that has the syscall but not the flag. Both are kernel-version cases, out of scope by ruling.
  - Any error a seccomp filter on the `serve` process injects, for example a `SystemCallFilter=` in its unit. Whether one applies was not checked.
  - The DESIGN's handling (the hook returns the `io::Error`, which fails the spawn) covers all of these.
- **The child's descriptor table is private.** *Reasoned from Rust std:* a `pre_exec` closure forces the `fork` path, and the child gets its own copy of the table (`files->count == 1`). So the non-`UNSHARE` form affects only the child. The man page's own pre-exec example uses `close_range(3, ~0U, CLOSE_RANGE_UNSHARE); execve(....);` for the case of a *shared* table. It also notes that "Using CLOSE_RANGE_CLOEXEC avoids this: the descriptors can be marked before the seccomp(2) profile is set up."
- **Async-signal safety.**
  - `fork(2)`: "After a fork() in a multithreaded program, the child can safely call only async-signal-safe functions (see signal-safety(7)) until such time as it calls execve(2)."
  - `signal-safety(7)` lists `close`, `dup2`, and `fcntl`; `close_range` does not appear.
  - Practical precedent: glibc's Linux `spawni.c` child, which runs in a stricter context than a forked child (it shares memory with the parent), issues `int r = INLINE_SYSCALL_CALL (close_range, lowfd, ~0U, 0);`.
  - *Interpretation:* the call is a direct system call with no user-space allocation or locking, which is the property the list exists to guarantee. The DESIGN's claim holds in substance. A reviewer who reads "async-signal-safe" as a POSIX list membership would find `close_range` absent. `libc::syscall(SYS_close_range, …)` and the glibc wrapper are equivalent here; the wrapper only adds an `errno` write.

**Confidence**: High (man pages, kernel source, glibc source).
**Affects**: ADR-0129 and feature-delta "In-child close (D-295-R3)" and its `SAFETY` comment. Both hold. The "a failed call returns its `io::Error`" branch is unreachable on a supported kernel without an injected seccomp error.

### A7 — `command-fds` with tokio (ADR-0129)

**Verdict: VERIFIED, with one observation.**
- **Tokio support.** The `tokio` feature implements `CommandFdExt` for `tokio::process::Command`, and the implementation registers the same `map_fds` child hook through tokio's `pre_exec`.
- **Mechanism.** The mapping is a `pre_exec` closure that uses `dup2` (or `F_SETFD` with no flags when parent and child numbers coincide). Parent/child number collisions are first moved aside with `F_DUPFD_CLOEXEC`.
- **Hook ordering.** The hook runs in registration order relative to other `pre_exec` hooks, after the standard library has set up stdio and the working directory.
- **`FD_CLOEXEC`.** The mapping clears `FD_CLOEXEC` on the target descriptor.
- **Observation.** In the version read, a `dup2` failure is not reported as an error (see below).

**Sources.**
- `google/command-fds` `main`, whose `Cargo.toml` is `version = "0.3.3"`: `src/lib.rs`, `src/tokio.rs`, `Cargo.toml` (<https://github.com/google/command-fds>).
- docs.rs source view of the released 0.3.3 `src/tokio.rs` (<https://docs.rs/crate/command-fds/0.3.3/source/src/tokio.rs>), identical to `main`.
- `nix` 0.31.3 `src/unistd.rs`, from both the GitHub tag `v0.31.3` and the docs.rs source view (<https://github.com/nix-rust/nix/blob/v0.31.3/src/unistd.rs>).
- tokio 1.53.1 `process::Command` documentation (<https://docs.rs/tokio/latest/tokio/process/struct.Command.html>).
- Rust std `library/std/src/os/unix/process.rs` and `library/std/src/os/fd/owned.rs`, `master` (<https://github.com/rust-lang/rust>).
- `dup(2)` (<https://man7.org/linux/man-pages/man2/dup.2.html>).

All accessed 2026-09-24.

**Evidence (verbatim).**
- **Feature wiring.** `Cargo.toml`:
  ```toml
  tokio = { version = "1.52.3", optional = true, default-features = false, features = [
    "process",
  ] }
  ...
  [features]
  default = []
  tokio = ["dep:tokio"]
  ```
  `src/lib.rs`: `#[cfg(feature = "tokio")] pub mod tokio;`.
- **The tokio implementation** (`src/tokio.rs`; the same text in the 0.3.3 release on docs.rs):
  ```rust
  use tokio::process::Command;

  impl CommandFdExt for Command {
      fn fd_mappings(
          &mut self,
          mut mappings: Vec<FdMapping>,
      ) -> Result<&mut Self, FdMappingCollision> {
          let child_fds = validate_child_fds(&mappings)?;

          unsafe {
              self.pre_exec(move || map_fds(&mut mappings, &child_fds));
          }

          Ok(self)
      }
  ```
  The `std::process::Command` implementation is the same, with the comment "Safety: `map_fds` will not allocate, so it is safe to call from this hook."
- **The child hook** (`map_fds`, `src/lib.rs`, "This function must not do any allocation, as it is called from the pre_exec hook"):
  1. **Collision step.** "If any parent FDs conflict with child FDs, then first duplicate them to a temporary FD which is clear of either range": `let parent_fd = fcntl(&mapping.parent_fd, FcntlArg::F_DUPFD_CLOEXEC(first_safe_fd))?;`.
  2. **Same-number step.** When the child number equals the parent number: "Remove the FD_CLOEXEC flag, so the FD will be kept open when exec is called for the child." (`fcntl(&mapping.parent_fd, FcntlArg::F_SETFD(FdFlag::empty()))?;`).
  3. **Otherwise.** "This closes child_fd if it is already open as something else, and clears the FD_CLOEXEC flag on child_fd." (`let _ = dup2_raw(&mapping.parent_fd, mapping.child_fd)?.into_raw_fd();`).

  `dup(2)` confirms the flag behaviour: "The close-on-exec flag (FD_CLOEXEC; see fcntl(2)) for the duplicate descriptor is off". It also states that "If the file descriptor newfd was previously open, it is closed before being reused; the close is performed silently", and that "The steps of closing and reusing the file descriptor newfd are performed atomically." That confirms the DESIGN's review-finding-L1 reasoning, where the standard library's exec-error pipe numbered 3 is replaced by the queue.
- **Ordering.**
  - tokio `pre_exec`: "Schedules a closure to be run just before the `exec` function is invoked. … Multiple closures can be registered and they will be called in order of their registration."
  - Rust std `CommandExt::pre_exec`: "Multiple closures can be registered and they will be called in order of their registration", and "When this closure is run, aspects such as the stdio file descriptors and working directory have successfully been changed".
  - The mapping therefore runs after stdio is installed, and before any hook registered after it. The DESIGN registers the `close_range` hook second, so it runs after the `dup2`. Its range starts at 4, so fd 3 is unaffected in either order. `command-fds`' temporary duplicates are already `F_DUPFD_CLOEXEC`.
- **Parent-side lifetime.** The mappings, with their `OwnedFd`s, are moved into the `pre_exec` closure held by the `Command`. This confirms research F1.3 and the DESIGN's "the parent's copy lives exactly as long as the `Command` value".

**Observation: an unchecked `dup2` (fact plus labelled consequence).**
- `nix` 0.31.3, the version `command-fds` 0.3.3 depends on (`nix = { version = "0.31.3", features = ["fs"] }`), has this `dup2_raw` body, from both sources:
  ```rust
      let duplicated_fd = unsafe {
          libc::dup2(oldfd.as_fd().as_raw_fd(), newfd.into_raw_fd())
      };
      // SAFETY:
      //
      // This is unsafe if `newfd` is not a file descriptor that can be consumed
      Ok(unsafe {
          OwnedFd::from_raw_fd(duplicated_fd)
      })
  ```
  There is no `Errno::result`, so the `?` in `map_fds` cannot propagate a `dup2` failure.
- std's `OwnedFd::from_raw_fd` "Panics if the raw file descriptor has the value `-1`" (`ValidRawFd::new(fd).expect("fd != -1")`). *Consequence (reasoned):* a failed `dup2` panics inside the forked child instead of returning an `io::Error` to `spawn`. That departs from the DESIGN's model that hook failures surface as spawn errors.
- *Likelihood (reasoned):* `dup2(queue, 3)` in a freshly forked, single-threaded child has essentially no failure path. `EBADF` cannot occur because the source descriptor is open. The `EBUSY` open race needs a concurrent thread. `EMFILE` needs `RLIMIT_NOFILE ≤ 3`. How the standard library handles a panic in a `pre_exec` closure was not read. This is recorded as a fact about the dependency chain, not as a defect in the DESIGN.

**Confidence**: High for the tokio implementation, the mechanism, the ordering, and `FD_CLOEXEC` (primary source; two independent reads of the tokio file). High for the unchecked `dup2` (two independent reads of the `nix` source). The panic consequence is reasoned.
**Affects**: ADR-0129 and feature-delta "`CloudHypervisorVmm::create`" and "Command lifetime (F19)".
- This closes the earlier research gap 2 and the DESIGN's "DELIVER only confirms by compiling that the `tokio` feature implements `CommandFdExt` for `tokio::process::Command`; the research read that from docs.rs, not from source": it is now read from source.
- Registration-order semantics support "register exactly one further `pre_exec` hook" after the mapping.

### Addendum knowledge gaps

1. **v6.18 `kernel/cgroup/cgroup.c` was not read.** WebFetch truncated the file. A4 rests on the local 7.2 tree plus the version-agnostic `cgroup-v2.rst` sentence. Kernel-version differences are out of scope by ruling.
2. **Cloud Hypervisor seccomp coverage is incomplete.** The TUN-ioctl listings for `vmm/src/seccomp_filters.rs` and `virtio-devices/src/seccomp_filters.rs` came from WebFetch summaries of whole files. Whether every CH thread, including the main thread, is filtered, and Overdrive's `--seccomp` value, were not checked (A5).
3. **Accepted-socket `TRANSPARENT` inheritance** is reasoned from `sock_copy`. No search was made for a later clear in the accept path (A2, step 3).
4. **`inet_steal_sock` / `__inet_lookup_skb`.** Their bodies were not reproduced. That TCP input uses the TPROXY-assigned socket is reasoned (A2, step 4).
5. **Rust std's handling of a panic inside a `pre_exec` closure** was not read (A7 observation).
6. **Not checked in the repository:**
   - whether Overdrive's launcher stacks its own Landlock ruleset (A5);
   - whether the TAP audit reads back the group as well as the owner (A1);
   - the appliance's `/dev/net/tun` mode (existing gap 13).
7. **Nothing in this addendum was executed natively.** E14 (e) remains the deciding evidence for A2.

### Addendum sources

All accessed 2026-09-24. Kernel sources are tag `v6.18` unless marked "7.2", which means the local checkout `/Users/marcus/git/linux` (`Makefile` 7.2.0).

- **[A-1] Linux kernel, `drivers/net/tun.c` (v6.18).** <https://github.com/torvalds/linux/blob/v6.18/drivers/net/tun.c>
- **[A-2] Linux kernel, TPROXY and TCP input (v6.18).**
  - `net/ipv4/netfilter/nf_tproxy_ipv4.c`, `net/netfilter/nft_tproxy.c`, `net/netfilter/xt_TPROXY.c`, `include/net/netfilter/nf_tproxy.h`
  - `include/net/tcp.h`, `net/ipv4/inet_timewait_sock.c`, `net/core/sock.c`, `net/ipv4/tcp_ipv4.c`, `net/ipv4/tcp_minisocks.c`, `net/ipv4/inet_hashtables.c`
  - <https://github.com/torvalds/linux/tree/v6.18>
- **[A-3] Linux kernel, nf_tables (v6.18).** `net/netfilter/nf_tables_core.c`, `net/netfilter/nft_meta.c`. <https://github.com/torvalds/linux/tree/v6.18/net/netfilter>
- **[A-4] Linux kernel, cgroup, exit, and file code (7.2).**
  - `kernel/cgroup/cgroup.c`, `kernel/exit.c`, `fs/file.c`, `Documentation/admin-guide/cgroup-v2.rst`, `Documentation/userspace-api/landlock.rst`
  - Upstream <https://github.com/torvalds/linux>; <https://docs.kernel.org/admin-guide/cgroup-v2.html>; <https://docs.kernel.org/userspace-api/landlock.html>
- **[A-5] Cloud Hypervisor `v53.0` (and `main`).**
  - `vmm/src/vm_config.rs`, `vmm/src/landlock.rs`, `vmm/src/seccomp_filters.rs`, `virtio-devices/src/seccomp_filters.rs`
  - <https://github.com/cloud-hypervisor/cloud-hypervisor/tree/v53.0>
- **[A-6] Linux man pages.** `close_range(2)`, `fork(2)`, `dup(2)`, `signal-safety(7)`. <https://man7.org/linux/man-pages/>
- **[A-7] glibc `sysdeps/unix/sysv/linux/spawni.c` (`master`, GitHub mirror).** <https://github.com/bminor/glibc/blob/master/sysdeps/unix/sysv/linux/spawni.c>
- **[A-8] Google `command-fds`.**
  - `main` (`version = "0.3.3"`): `src/lib.rs`, `src/tokio.rs`, `Cargo.toml`. <https://github.com/google/command-fds>
  - docs.rs 0.3.3 source: <https://docs.rs/crate/command-fds/0.3.3/source/src/tokio.rs>
- **[A-9] `nix` 0.31.3 `src/unistd.rs`.** <https://github.com/nix-rust/nix/blob/v0.31.3/src/unistd.rs>; <https://docs.rs/nix/0.31.3/src/nix/unistd.rs.html>
- **[A-10] tokio 1.53.1 `tokio::process::Command`.** <https://docs.rs/tokio/latest/tokio/process/struct.Command.html>
- **[A-11] Rust std, `master`.** `library/std/src/os/unix/process.rs`, `library/std/src/os/fd/owned.rs`. <https://github.com/rust-lang/rust/tree/master/library/std/src/os>

All domains are on the trusted list, except the glibc GitHub mirror, which is on github.com and serves project source. Each fact rests on primary source code; man pages and kernel documentation corroborate.

### Addendum metadata

- **Method.** Primary source read verbatim for every verdict: kernel, Cloud Hypervisor, `command-fds`, `nix`, glibc, and Rust std.
- **Sources.** About 40 fetches and local reads were examined; 11 source groups are cited.
- **Tool notes.**
  - WebFetch enforces short quotes and truncates large files, so kernel code was fetched one function at a time.
  - The local Linux 7.2 checkout was used where fetches truncated (`cgroup.c`) and for `fs/file.c`, `exit.c`, and the documentation.
- **Scope.** No design artifact was edited, nothing was committed, and no GitHub action was taken.

### Addendum summary table

| # | Fact under test | Verdict | DESIGN item affected |
|---|---|---|---|
| A1 | `TUNSETOWNER`/`TUNSETGROUP`/`TUNSETPERSIST`/`TUNSETCARRIER` on an attached fd have no capability or owner check | **VERIFIED**; ADR-0130 omits `TUNSETGROUP`; `SIOCSIFHWADDR` and `TUNSETLINK` are likewise unchecked | ADR-0130 / L8 / D-295-R4 |
| A2 | A SYN reusing a transparent `TIME_WAIT` 4-tuple reaches a host `0.0.0.0` listener when no transparent listener exists | **VERIFIED in source** for both `nft_tproxy` and `xt_TPROXY`; independent of R19's order; two extra preconditions: the guest finished its close (true `TIME_WAIT`), and the SYN's sequence number or timestamp is newer | E14 (e) / ADR-0140 / L3 |
| A3 | `nft_tproxy` → `NFT_BREAK`; `xt_TPROXY` → `NF_DROP`; a `meta mark set` before `tproxy` survives `NFT_BREAK` | **VERIFIED** | ADR-0140 / M1 / D-295-R19 |
| A4 | `rmdir` gets `EBUSY` with live tasks; `cgroup.kill` is signal-only; `ENOENT` ⇒ the VMM is gone | **VERIFIED** (`EBUSY`, signal-only); inference sound for "cannot run again", **not** for "descriptors closed"; assumes no migration out | H1 / `VmKillCapability` |
| A5 | CH Landlock denies `/dev/net/tun` on the `fd=` path | **REFUTED**: allowed `rw` whenever `--net` is present (`v53.0`); CH seccomp omits the A1 ioctls (thread coverage unverified) | ADR-0130 residual / open item 8 |
| A6 | `close_range(4, u32::MAX, CLOSE_RANGE_CLOEXEC)` in `pre_exec`: semantics, safety, failures | **VERIFIED**; no reachable error for this call; not on the formal POSIX async-signal-safe list, but a bare syscall that glibc uses in its spawn child | ADR-0129 / D-295-R3 |
| A7 | `command-fds` `tokio` feature maps fds for `tokio::process::Command`, via `pre_exec` + `dup2`, and clears `FD_CLOEXEC` | **VERIFIED**; runs in registration order after stdio setup; nix 0.31.3's `dup2_raw` does not check errors, so a failed `dup2` panics in the child | ADR-0129 / F19 |

## Addendum 2 (2026-09-24) — host-side TAP MAC change and bridge FDB delivery

**Date**: 2026-09-24 | **Researcher**: nw-researcher (Nova) | **Depth**: detailed | **Scope**: final design review finding **R5-H1**. Each microVM holds its own TAP queue fd as uid 4200 without `CAP_NET_ADMIN`. The TAPs are ports of one node-local bridge. The design has an ingress-only TCX classifier per TAP and does no FDB handling. The question is whether the queue holder can set its TAP's host-side MAC to another guest's MAC and so receive that guest's host-to-guest plaintext. Primary source is authoritative. The kernel read is the local checkout `/Users/marcus/git/linux` (`Makefile` `VERSION = 7`, `PATCHLEVEL = 2`, `SUBLEVEL = 0`), with upstream paths on <https://github.com/torvalds/linux/blob/master/>. Line numbers are from that tree. Version differences are out of scope by user ruling. No design artifact was edited, nothing was committed, and no GitHub action was taken.

**Summary.**
- **R5-H1 holds in source (B1).** The fd holder's `SIOCSIFHWADDR` reaches `dev_set_mac_address_user` with no capability or owner check. The TAP's `IFF_LIVE_ADDR_CHANGE` lets it apply while the TAP is up. The bridge's `br_fdb_changeaddr` → `fdb_add_local` deletes any non-local entry for that MAC, including a user `static` pin, and installs `LOCAL|STATIC` on the holder's port. `br_dev_xmit` then sends host frames for that MAC out of that port.
  - The attacker must copy the victim's *guest* (virtio-net) MAC, not its TAP MAC, and must first learn it.
  - The redirect is sticky: the victim cannot re-learn over a `LOCAL` entry.
- **Cloud Hypervisor v53.0's seccomp does not block it (B2).** The VMM thread's filter allows `SIOCSIFHWADDR` on any fd, and the main thread appears unfiltered.
- **Prior art (B3).** Source-MAC anti-spoofing (libvirt nwfilter, Neutron ebtables, the bridge `locked` flag) targets guest-emitted frames and does not see the ioctl. Designs that choose delivery from a registered record rather than a learned FDB (Neutron's OVS firewall, Cilium, Kata, a routed Firecracker TAP) are not steered by a port's MAC.
- **Candidate controls (B4).**
  - A destination-MAC check at TAP egress stops the theft but not the victim's outage.
  - Detection sees the change at once, but "set down" leaves the stolen entry in place.
  - A `static` pin is overridden. A `permanent` pin survives in source, with side effects.
  - No kernel mechanism makes the MAC immutable. The generic chokepoints are a launcher-installed seccomp filter and a BPF-LSM `file_ioctl` hook.

### B1 — The mechanism, end to end in kernel source

**Verdict: VERIFIED, with one precondition the finding does not state.** An attached TAP queue holder with no capability can change the TAP's host-side MAC. The bridge accepts the change without a veto. It deletes any *non-local* FDB entry for the new MAC, whichever port that entry points at, and installs a `LOCAL|STATIC` entry on the holder's port. The bridge's own transmit path then sends host-originated unicast for that MAC out of the holder's port. The theft works only if the new MAC is the **victim guest's virtio-net MAC**. That is the MAC the host's traffic to the victim is addressed to. It is not the victim TAP's host-side MAC.

#### B1.1 The ioctl path on a tun fd has no capability or owner check

- **`SIOCSIFHWADDR` is forwarded to the netdev without a check.** In `drivers/net/tun.c`:
  - `__tun_chr_ioctl` copies an `ifreq` for socket-type ioctls (lines 3207–3210: `if (cmd == TUNSETIFF || cmd == TUNSETQUEUE || (_IOC_TYPE(cmd) == SOCK_IOC_TYPE && cmd != SIOCGSKNS)) { if (copy_from_user(&ifr, argp, ifreq_len))`).
  - The only gate before the `switch` is attachment (lines 3264–3266: `ret = -EBADFD; if (!tun) goto unlock;`). The only capability check in this prologue is for `SIOCGSKNS` (lines 3223–3225: `if (!ns_capable(net->user_ns, CAP_NET_ADMIN)) return -EPERM;`).
  - The arm (lines 3385–3394):
    ```c
    	case SIOCSIFHWADDR:
    		/* Set hw address */
    		if (tun->dev->addr_len > sizeof(ifr.ifr_hwaddr)) {
    			ret = -EINVAL;
    			break;
    		}
    		ret = dev_set_mac_address_user(tun->dev,
    					       (struct sockaddr_storage *)&ifr.ifr_hwaddr,
    					       NULL);
    		break;
    ```
    The line numbers the reviewer cited match exactly. The compat entry point forwards the same command (line 3502).
- **Contrast: the socket path requires `CAP_NET_ADMIN`.** `net/core/dev_ioctl.c`, lines 797 and 808–809: `case SIOCSIFHWADDR:` … `if (!ns_capable(net->user_ns, CAP_NET_ADMIN)) return -EPERM;`. Both paths reach the same `dev_set_mac_address_user` (tun.c line 3391; dev_ioctl.c line 567). *Interpretation:* the tun character device is a second route to the same netdev operation, and it skips the capability check the socket route applies.
- **The core setter has no check either.** `net/core/dev_api.c`, lines 88–101: `dev_set_mac_address_user` takes `dev_addr_sem`, then calls `netif_set_mac_address`. `net/core/dev.c`, lines 10035–10059:
  ```c
  	if (!ops->ndo_set_mac_address)
  		return -EOPNOTSUPP;
  	if (ss->ss_family != dev->type)
  		return -EINVAL;
  	if (!netif_device_present(dev))
  		return -ENODEV;
  	err = netif_pre_changeaddr_notify(dev, ss->__data, extack);
  	if (err)
  		return err;
  	if (memcmp(dev->dev_addr, ss->__data, dev->addr_len)) {
  		err = ops->ndo_set_mac_address(dev, ss);
  		if (err)
  			return err;
  	}
  	dev->addr_assign_type = NET_ADDR_SET;
  	call_netdevice_notifiers(NETDEV_CHANGEADDR, dev);
  ```
- **A running TAP accepts the change.** The TAP's `ndo_set_mac_address` is `eth_mac_addr` (tun.c line 1359). `tun_net_initialize` sets `dev->priv_flags |= IFF_LIVE_ADDR_CHANGE;` for `IFF_TAP` (line 1416). `net/ethernet/eth.c`, lines 276–279:
  ```c
  	if (!(dev->priv_flags & IFF_LIVE_ADDR_CHANGE) && netif_running(dev))
  		return -EBUSY;
  	if (!is_valid_ether_addr(addr->sa_data))
  		return -EADDRNOTAVAIL;
  ```
  The only content restriction is `is_valid_ether_addr`: unicast and non-zero. Another guest's virtio-net MAC passes.

#### B1.2 The bridge does not veto, and it moves the address to the holder's port

- **No veto.** `net/bridge/br.c`, lines 78–87. For a port, `NETDEV_PRE_CHANGEADDR` does nothing when the bridge's own MAC was set explicitly. Otherwise it forwards the pre-change notification to the bridge device's notifier chain:
  ```c
  	case NETDEV_PRE_CHANGEADDR:
  		if (br->dev->addr_assign_type == NET_ADDR_SET)
  			break;
  		prechaddr_info = ptr;
  		err = netif_pre_changeaddr_notify(br->dev,
  						  prechaddr_info->dev_addr,
  						  extack);
  ```
  *Interpretation:* the bridge itself rejects nothing here. A veto could only come from another notifier on the bridge device's chain, and none is described in the design.
- **`NETDEV_CHANGEADDR` rewrites the FDB unconditionally.** `br.c`, lines 89–98:
  ```c
  	case NETDEV_CHANGEADDR:
  		spin_lock_bh(&br->lock);
  		br_fdb_changeaddr(p, dev->dev_addr);
  		changed_addr = br_stp_recalculate_bridge_id(br);
  		spin_unlock_bh(&br->lock);
  ```
  No port flag is consulted, so `learning off`, `locked`, and `isolated` do not affect this path.
- **`br_fdb_changeaddr`** (`net/bridge/br_fdb.c`, lines 460–504) first deletes the port's old auto-created local entry. It matches `READ_ONCE(f->dst) == p && test_bit(BR_FDB_LOCAL, …) && !test_bit(BR_FDB_ADDED_BY_USER, …)` and calls `fdb_delete_local`. It then calls `fdb_add_local(br, p, newaddr, 0)`, and repeats per VLAN when the port has VLANs.
- **`fdb_add_local` replaces any non-local entry for the new MAC** (lines 430–458):
  ```c
  	fdb = br_fdb_find(br, addr, vid);
  	if (fdb) {
  		/* it is okay to have multiple ports with same
  		 * address, just use the first one.
  		 */
  		if (test_bit(BR_FDB_LOCAL, &fdb->flags))
  			return 0;
  		br_warn(br, "adding interface %s with same address as a received packet (addr:%pM, vlan:%u)\n",
  			source ? source->dev->name : br->dev->name, addr, vid);
  		fdb_delete(br, fdb, true);
  	}

  	fdb = fdb_create(br, source, addr, vid,
  			 BIT(BR_FDB_LOCAL) | BIT(BR_FDB_STATIC));
  ```
  - A dynamically learned entry for the victim's guest MAC on the victim's port is deleted, and a `LOCAL|STATIC` entry is created with `dst` = the holder's port.
  - The only survivor is an entry that is already `BR_FDB_LOCAL` ("just use the first one").
  - Which user-added entries are `LOCAL`: `fdb_add_entry` sets `BR_FDB_LOCAL` only for `NUD_PERMANENT` (lines 1207–1210). `NUD_NOARP`, which is iproute2's `static`, *clears* it (lines 1211–1214). So a user `static` entry pinning the victim's MAC to the victim's port is **not** local and is deleted like a learned one. B4.3 takes this up.
  - The kernel logs one `br_warn` line per replacement. That is a side-channel for detection, not a prevention.
- **The theft is sticky.** When the victim guest later transmits with its own MAC as source, `br_fdb_update` finds a `LOCAL` entry and refuses to move it (lines 985–988):
  ```c
  		if (unlikely(test_bit(BR_FDB_LOCAL, &fdb->flags))) {
  			if (net_ratelimit())
  				br_warn(br, "received packet on %s with own address as source address (addr:%pM, vlan:%u)\n",
  					source->dev->name, addr, vid);
  ```
  Normal learning cannot undo the redirect. It lasts until the holder's port changes MAC again or leaves the bridge (see B4.2).

#### B1.3 Where frames addressed to that MAC go

- **Host-originated frames are sent out of the holder's port.** `net/bridge/br_device.c` `br_dev_xmit`, lines 109–112:
  ```c
  	} else if ((dst = br_fdb_find_rcu(br, dest, vid)) != NULL) {
  		br_forward(READ_ONCE(dst->dst), skb, false, true);
  	} else {
  		br_flood(br, skb, BR_PKT_UNICAST, false, true, vid);
  	}
  ```
  The transmit path does not test `BR_FDB_LOCAL`. For a port-local entry, `dst->dst` is the holder's port. `br_forward` (`br_forward.c`, lines 144–173) delivers when `should_deliver` holds: the port is forwarding, VLAN egress is allowed, and the frame is not isolated-to-isolated. `__br_forward` then sets `skb->dev = to->dev` and runs `NF_HOOK(NFPROTO_BRIDGE, NF_BR_LOCAL_OUT, …, br_forward_finish)` (lines 92, 110, 115–117). `br_dev_queue_push_xmit` ends in `dev_queue_xmit(skb)` on the TAP (line 53). **The frame reaches the holder's queue fd.**
- **Frames from other ports are passed up to the host, not forwarded.** `net/bridge/br_input.c` `br_handle_frame_finish`, lines 218–222:
  ```c
  	if (dst) {
  		unsigned long now = jiffies;

  		if (test_bit(BR_FDB_LOCAL, &dst->flags))
  			return br_pass_frame_up(skb, false);
  ```
  So guest-to-guest L2 frames addressed to the stolen MAC go to the host stack instead of the victim. This is a denial of service, not theft. Host-originated traffic is the theft path, which matches R5-H1's list: leg-F replies, leg-C inbound delivery, and DNS replies.
- **The bridge's `isolated` flag does not apply to host-originated frames.** `br_dev_xmit` zeroes the control block (`memset(skb->cb, 0, sizeof(struct br_input_skb_cb));`, line 49), and `br_skb_isolated` requires `BR_INPUT_SKB_CB(skb)->src_port_isolated` (`br_private.h`, lines 910–915).

#### B1.4 Which MAC the victim's traffic targets

*Interpretation from the paths above, labelled.*
- The host builds a frame to the victim guest with the destination MAC from its neighbour entry for the guest's IP. The guest's virtio-net interface answers ARP/NDP with its own MAC, so the destination is the **guest's virtio-net MAC**. The victim TAP's host-side MAC is only the bridge-port address of the TAP netdev. No host frame is addressed to it in normal operation.
- An attacker who copies the victim **TAP's host-side** MAC achieves nothing. `fdb_add_local` finds an existing `LOCAL` entry and returns 0 ("just use the first one").
- An attacker who copies the victim **guest's** MAC replaces the learned non-local entry and takes the traffic.
- **The precondition R5-H1 does not state: the attacker must know the victim guest's MAC.** On a default bridge, guest ARP requests are broadcast and flooded to every port (`br_flood` `BR_PKT_BROADCAST`), including the attacker's TAP. A scheme that derives MACs deterministically from addresses would also reveal them. Whether the design floods guest broadcasts to other TAPs, or derives MACs predictably, was not checked in the repository.

#### B1.5 Adjacent observations (same source, not asked)

1. **Unknown-unicast flooding leaks host-to-guest frames without any MAC change.** When the victim's learned entry is absent, `br_dev_xmit` floods (line 112). That happens after ageing, before the first guest frame, or right after the attacker reverts its MAC and `fdb_delete_local` removes the stolen entry. `br_flood` delivers `BR_PKT_UNICAST` to every port with `BR_FLOOD_BIT` set (`br_forward.c`, lines 216–218), which includes other guests' TAPs.
   - *Interpretation:* on a default bridge with learning and no static entries, a compromised VMM can receive other guests' host-to-guest frames during those windows by doing nothing.
   - An ingress-only classifier does not see them, because they travel TAP-egress.
   - Whether the design sets `flood off` on TAP ports, or programs entries, was not checked.
2. **A holder can change the bridge device's own MAC when that MAC was never set explicitly.** `br_stp_recalculate_bridge_id` returns early only for `NET_ADDR_SET` (`br_stp_if.c`, line 269). Otherwise it adopts the numerically lowest port MAC (lines 272–277), and `br_stp_change_bridge_id` applies it with `eth_hw_addr_set(br->dev, addr)` (line 241).
   - *Interpretation:* this moves the guests' gateway MAC, which is a denial of service.
   - libvirt avoids the effect by giving TAPs a high first octet (B3.1).
   - Whether Overdrive's bridge MAC is `NET_ADDR_SET` was not checked.
3. **The change is announced on netlink.** `rtnetlink_event` emits `RTM_NEWLINK` for `NETDEV_CHANGEADDR` (`net/core/rtnetlink.c`, lines 7181, 7192). `fdb_add_local` and `fdb_delete` emit `RTM_NEWNEIGH` and `RTM_DELNEIGH` (`br_fdb.c`, lines 456 and 328). B4.2 uses this.

**Confidence**: High. Every link is quoted from primary source in one tree. Not executed natively.
**Affects**: R5-H1. The mechanism holds as the reviewer stated it, with the refinement that the attacker must copy the victim *guest's* MAC and learn it first. Two further facts sit beside it: the unknown-unicast flood (observation 1) and the bridge-MAC move (observation 2).

### B2 — Does Cloud Hypervisor's seccomp filter block `SIOCSIFHWADDR`?

**Verdict: REFUTED for the VMM thread. CH v53.0's VMM-thread filter explicitly allows `SIOCSIFHWADDR`, keyed only on the ioctl request number, so it allows the call on the TAP queue fd too.** The virtio-net worker and control threads do not allow it. The process's main thread is not filtered at all, per two fetched summaries. `--seccomp true` therefore does not cover every thread and does not stop this hazard.

**Sources read.** Cloud Hypervisor tag `v53.0`, fetched as raw text, accessed 2026-09-24. `vmm/src/seccomp_filters.rs` was fetched three times with different prompts, and the `SIOCSIFHWADDR` rule line is identical in all three. The other CH files were fetched once each in this addendum. `virtio-devices/src/seccomp_filters.rs` was also fetched in the earlier addendum (A5), with the same result.
- [vmm/src/seccomp_filters.rs](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v53.0/vmm/src/seccomp_filters.rs)
- [virtio-devices/src/seccomp_filters.rs](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v53.0/virtio-devices/src/seccomp_filters.rs)
- [net_util/src/tap.rs](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v53.0/net_util/src/tap.rs)
- [vmm/src/lib.rs](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v53.0/vmm/src/lib.rs)
- [cloud-hypervisor/src/main.rs](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/v53.0/cloud-hypervisor/src/main.rs)
- [seccomp(2)](https://man7.org/linux/man-pages/man2/seccomp.2.html)

**Evidence.**
- **The VMM thread allows `SIOCSIFHWADDR`.** In `create_vmm_ioctl_seccomp_rule_common` (the fullest of the three fetches):
  ```rust
          and![Cond::new(1, ArgLen::Dword, Eq, SIOCGIFFLAGS)?],
          and![Cond::new(1, ArgLen::Dword, Eq, SIOCGIFHWADDR)?],
          and![Cond::new(1, ArgLen::Dword, Eq, SIOCGIFMTU)?],
          and![Cond::new(1, ArgLen::Dword, Eq, SIOCGIFINDEX)?],
          and![Cond::new(1, ArgLen::Dword, Eq, SIOCSIFADDR)?],
          and![Cond::new(1, ArgLen::Dword, Eq, SIOCSIFFLAGS)?],
          and![Cond::new(1, ArgLen::Dword, Eq, SIOCSIFHWADDR)?],
          and![Cond::new(1, ArgLen::Dword, Eq, SIOCSIFMTU)?],
          and![Cond::new(1, ArgLen::Dword, Eq, SIOCSIFNETMASK)?],
  ```
  The KVM builder includes it: `let common_rules = create_vmm_ioctl_seccomp_rule_common(HypervisorType::Kvm)?; … arch_rules.extend(common_rules);`. `vmm_thread_rules` maps `(libc::SYS_ioctl, create_vmm_ioctl_seccomp_rule(hypervisor_type)?)`.
- **The condition tests argument 1 only.** `Cond::new(1, …)` compares the request number. A seccomp filter sees `struct seccomp_data { int nr; __u32 arch; __u64 instruction_pointer; __u64 args[6]; }` (seccomp(2)). The fd is `args[0]`, a bare integer, and this rule does not constrain it. *Interpretation:* the filter cannot tell the TAP queue fd from the AF_UNIX socket CH uses for its own MAC calls, so allowing the request number allows it on every fd.
- **Why CH allows it: its own MAC setter uses a separate socket.** `net_util/src/tap.rs` `set_mac_addr`: `let sock = create_unix_socket().map_err(Error::NetUtil)?;` … `unsafe { Self::ioctl_with_ref(&sock, libc::SIOCSIFHWADDR as c_ulong, &ifreq) }`. It returns early when the address already matches (`if self.get_mac_addr()? == addr { return Ok(()); }`). *Interpretation:* on the socket path the kernel demands `CAP_NET_ADMIN` (B1.1), so CH's legitimate use fails for uid 4200. The seccomp allowance exists for that path, and on the tun fd the kernel asks for nothing.
- **Virtio-net threads do not allow it** (`virtio-devices/src/seccomp_filters.rs`, per fetch):
  - `VirtioNet` allows `SYS_ioctl` only under `sev_snp`, for `MSHV_MODIFY_GPA_HOST_ACCESS()`.
  - `VirtioNetCtl` allows `TUNSETOFFLOAD`, plus the same `sev_snp` entry.
  - `virtio_thread_common` has no `SYS_ioctl`.
  - No `SIOC` constant appears anywhere in the file.
- **vCPU threads do not allow it.** `create_vcpu_ioctl_seccomp_rule` lists VFIO and vDPA requests and extends with hypervisor and iommufd rules. It does not include the common VMM rules.
- **Filters are per-thread, and the main thread has none.**
  - `vmm/src/lib.rs` applies filters inside each spawned thread: `get_seccomp_filter(seccomp_action, Thread::Vmm, Some(hypervisor_type))` then `apply_filter(&vmm_seccomp_filter)` in `start_vmm_thread`, and likewise `Thread::EventMonitor` and `Thread::SignalHandler`.
  - Both fetches of `lib.rs` and `main.rs` report no TSYNC and no filter on the calling (main) thread. `main.rs` passes `&seccomp_action` to `start_vmm_thread` and `start_event_monitor_thread` and only installs the `SIGSYS` handler itself.
  - seccomp(2) describes TSYNC as the mechanism that synchronises "all other threads of the calling process to the same seccomp filter tree". Without it, each thread keeps its own filter.
- **`--seccomp true` means `SeccompAction::Trap`.** `main.rs`: `.value_parser(["true", "false", "log", "errno"]).default_value("true")` and `"true" => SeccompAction::Trap`. The Overdrive tree renders `--seccomp` from `VmConfinement::seccomp_arg`, which returns `"true"` (`crates/overdrive-core/src/vm/config.rs`, lines 466–468). That is a fact about the current code, not a verification of the replacement design.

**The TAP host-side MAC vs the guest MAC in CH's configuration.** CH's `--net` has `mac=` for the guest virtio-net MAC and `host_mac=` for the TAP's host-side MAC. The current Overdrive renderer passes `tap=…,mac=…` (`crates/overdrive-host/src/vmm.rs`, lines 294–304), which is the guest MAC, and no `host_mac`. That matches B1.4: the two MACs are distinct, and the guest MAC is the one host traffic is addressed to.

**Confidence**: High that the VMM-thread rule allows `SIOCSIFHWADDR` keyed only on the request number (three consistent fetches, plus the seccomp data layout from the man page). Medium for the per-thread, no-TSYNC, main-thread-unfiltered finding (fetched summaries of `lib.rs` and `main.rs`, not a line-by-line read). Not executed natively.
**Affects**: R5-H1 and the earlier addendum's A5 seccomp observation. A5 listed CH's *TUN-namespace* ioctls. `SIOCSIFHWADDR` is a socket-namespace request number that the VMM thread also permits, so CH's seccomp is not a control against this hazard. Earlier gap 2 ("whether every CH thread, including the main thread, is filtered") is now answered at Medium confidence: the main thread is not.

### B3 — Prior art for stopping one VM from hijacking another's traffic on a shared switch

**Verdict: VERIFIED that the prior art divides into two classes, and that only one of them addresses this hazard.**
- **Source-MAC anti-spoofing on frames *from* the VM.** libvirt nwfilter, Neutron's ebtables rules, the bridge `locked` flag, and the cloud source/destination checks all belong here. They stop a guest from *emitting* frames under another MAC, and so from poisoning learning. They do not see a host-side `SIOCSIFHWADDR`, because that is an ioctl, not a frame, and it installs a `LOCAL` entry directly (B1.2).
- **Delivery keyed on a control-plane record rather than a learned FDB.** Cilium, Kata's TC-filter model, and Neutron's OVS firewall belong here. The output interface for a destination is chosen from a registered endpoint or port record, so a port device's own MAC does not steer delivery.
- **No primary source was found that addresses the host-side TAP MAC change on a Linux bridge by name.** libvirt's code does record the bridge behaviour the hazard depends on (B3.1).

#### B3.1 libvirt (nwfilter, and its TAP MAC choice)

- **What `no-mac-spoofing` enforces.** It filters frames *from* the VM by source MAC (`src/nwfilter/xml/no-mac-spoofing.xml`, libvirt `master`):
  ```xml
  <filter name='no-mac-spoofing' chain='mac' priority='-800'>
    <!-- return packets with VM's MAC address as source address -->
    <rule direction='out' action='return'>
      <mac srcmacaddr='$MAC'/>
    </rule>
    <!-- drop everything else -->
    <rule direction='out' action='drop'>
      <mac/>
    </rule>
  </filter>
  ```
  The libvirt documentation defines `direction` relative to the VM: 'in' is incoming traffic toward the VM, 'out' is outgoing traffic from the VM (<https://libvirt.org/formatnwfilter.html>, fetched summary).
- **What `clean-traffic` adds.** It composes `no-mac-spoofing`, `no-ip-spoofing`, `allow-incoming-ipv4`, `no-arp-spoofing`, `no-other-l2-traffic`, and `qemu-announce-self-rarp` (`clean-traffic.xml`, verbatim). Its only incoming-direction content is `allow-incoming-ipv4` and an ARP accept. No shipped filter read here checks the *destination* MAC of traffic toward the VM.
- **Where it runs.** The documentation says the `mac`, `stp`, `vlan`, `arp`, `rarp`, `ipv4`, and `ipv6` protocols are "implemented using ebtables", and that other IPv4 and IPv6 protocols use iptables and ip6tables. The rules attach at the VM's TAP bridge port.
- **libvirt's TAP MAC choice records the same bridge behaviour.** `src/util/virnetdevtap.c`, `virNetDevTapCreateInBridgePort`, libvirt `master`:
  ```c
  if (!(flags & VIR_NETDEV_TAP_CREATE_USE_MAC_FOR_BRIDGE)) {
      /* The tap device's MAC address cannot match the MAC address
       * used by the guest. This results in "received packet on
       * vnetX with own address as source address" error logs from
       * the kernel. Making the tap address as high as possible
       * discourages the bridge from using this tap's MAC as its own
       * (a Linux host bridge will take on the lowest numbered MAC
       * of all devices attached to it).
       */
      if (tapmac.addr[0] == 0xFE)
          tapmac.addr[0] = 0xFA;
      else
          tapmac.addr[0] = 0xFE;
  }
  ```
  The quoted kernel message is `br_fdb_update`'s warning on a `LOCAL` entry (B1.2). The "lowest numbered MAC" remark is `br_stp_recalculate_bridge_id` (B1.5 observation 2). So libvirt independently documents both consequences of a TAP holding a guest's MAC. *Interpretation:* libvirt sets the TAP MAC once at creation. It does not defend against a later change by the fd holder.
- **Would it stop the hazard?** No. nwfilter sees frames the VM sends. The hijack is an ioctl on the host-side netdev, and the stolen frames travel *toward* the attacker's TAP. *Interpretation:* nwfilter's rule grammar has `direction='in'` and a `dstmacaddr` attribute, so a custom 'in' rule could express a destination check. No shipped filter doing so was found.

#### B3.2 OpenStack Neutron

- **Linux-bridge agent: source-side ebtables rules on the TAP.** `neutron/plugins/ml2/drivers/linuxbridge/agent/arp_protect.py`, tag `22.0.0`:
  ```python
      if not _mac_vif_jump_present(vif, current_rules):
          ebtables(['-I', 'PREROUTING', '-i', vif, '-j', vif_chain])
      _delete_vif_mac_rules(vif, current_rules)
      for chunk in (mac_addresses[i:i + 500]
                    for i in range(0, len(mac_addresses), 500)):
          new_rule = ['-I', vif_chain, '-i', vif,
                      '--among-src', ','.join(sorted(chunk)), '-j', 'RETURN']
  ```
  The chain defaults to `DROP`. ARP protection is likewise attached with `ebtables(['-A', 'PREROUTING', '-i', vif, '-j', vif_chain, '-p', 'ARP'])` and allows only the port's `--arp-ip-src` addresses. Both match `-i <tap>`: frames *from* the VM.
- **The operator documentation states the scope.** From the `linuxbridge_agent.ini` sample, `prevent_arp_spoofing`: "This prevents the VMs attached to this agent from spoofing, it doesn't protect them from other devices which have the capability to spoof … For LinuxBridge, this requires ebtables." (<https://docs.openstack.org/newton/config-reference/networking/samples/linuxbridge_agent.ini.html>). The Liberty spec's aim is to "prevent the sending of spoofed ARP packets through" VM ports (<https://github.com/openstack/neutron-specs/blob/master/specs/liberty/arp-spoof-filtering-ebtables.rst>).
- **OVS firewall: delivery chosen by the port record's MAC.** `neutron/agent/linux/openvswitch_firewall/firewall.py`, tag `22.0.0`, `initialize_port_flows`:
  ```python
  self._add_flow(
      flow_group_id=port.ofport,
      table=ovs_consts.TRANSIENT_TABLE,
      priority=90,
      dl_dst=mac_addr,
      dl_vlan='0x%x' % port.vlan_tag,
      actions='set_field:{:d}->reg{:d},'
              'set_field:{:d}->reg{:d},'
              'strip_vlan,resubmit(,{:d})'.format(
                  port.ofport,
                  ovsfw_consts.REG_PORT,
  ```
  There is a matching `dl_dst=mac_addr` flow in `ACCEPT_OR_INGRESS_TABLE`, and source-side `dl_src=mac_addr` and `arp_spa` flows in `BASE_EGRESS_TABLE`. *Interpretation:* the destination port is set from the Neutron port's registered MAC. A frame is steered to a port only if its destination MAC is that port's registered MAC. This is the switch-level analogue of candidate (a).
  - Whether OVS itself reacts to a port netdev's `NETDEV_CHANGEADDR` was not read (unverified).
- **Would it stop the hazard?** The Linux-bridge ebtables rules would not. The OVS firewall's registered-MAC delivery would, in design, because the output port does not come from a device-MAC FDB. That rests on the unread OVS behaviour.

#### B3.3 Linux bridge port flags and FDB entry types

From `bridge(8)` (<https://man7.org/linux/man-pages/man8/bridge.8.html>), with the kernel behaviour from B1.
| Mechanism | `bridge(8)` text | What it enforces, and where (kernel) | Stops the host-side MAC change? |
|---|---|---|---|
| `learning off` | "Controls whether a given port will learn MAC addresses from received traffic or not." | Gates `br_fdb_update` on received frames only (`br_input.c`, lines 143–144). | **No.** `NETDEV_CHANGEADDR` calls `br_fdb_changeaddr` with no flag check (`br.c`, lines 89–91). |
| `locked` | "When locked, non-link-local frames received through the port are dropped unless an FDB entry with the MAC source address points to the port." | Ingress source check (`br_input.c`, lines 114–138). It drops when `fdb_src->dst != p` *or* the entry is `BR_FDB_LOCAL`. | **No.** Receive side only. After a hijack, the *victim's* frames on a locked port are also dropped, because the victim's MAC is now `LOCAL` elsewhere (lines 126–129). |
| `isolated` | "…able to communicate with non-isolated ports only." | `br_skb_isolated` needs `src_port_isolated`, which `br_dev_xmit` zeroes (B1.3). | **No.** It does not apply to host-originated frames. |
| `flood off` | "Controls whether unicast traffic for which there is no FDB entry will be flooded towards this given port." | `br_flood` skips the port for `BR_PKT_UNICAST` (`br_forward.c`, lines 216–218). | **No** for the hijack, which uses a known entry. **Yes** for the unknown-unicast leak (B1.5 observation 1). |
| `static` entry | "is a static (no arp) fdb entry" | `NUD_NOARP` clears `BR_FDB_LOCAL` (`br_fdb.c`, lines 1211–1214). | **No.** A non-local entry is deleted by `fdb_add_local` (lines 443–447). |
| `sticky` entry | "this entry will not change its port due to learning." | Only `br_fdb_update` honours it (line 1000). | **No.** `fdb_add_local` tests only `BR_FDB_LOCAL`. |
| `local`/`permanent` entry | "the bridge will not forward frames with this destination MAC address and VLAN ID, but terminate them locally." | Sets `BR_FDB_LOCAL` (lines 1207–1210). | **In source, yes**, with side effects. See B4.3 and Conflict C1. |

#### B3.4 Cilium

- **Delivery to a local endpoint rewrites both MACs from the endpoint record and redirects to the endpoint's ifindex.** `bpf/lib/local_delivery.h`, `main`:
  ```c
  	ret = ipv4_l3(ctx, l3_off, ep->node_mac.addr, ep->mac.addr, ip4);
  	if (ret != CTX_ACT_OK)
  		return ret;

  	return local_delivery(ctx, seclabel, magic, ep, direction, from_host,
  			      from_tunnel, cluster_id);
  ```
  In `local_delivery`, the endpoint-routes branch does `return redirect_ep(ctx, ep->ifindex, false, from_tunnel);`. Otherwise it tail-calls the endpoint's policy program (`return tail_call_policy(ctx, ep->lxc_id);`).
- **`ipv4_l3` writes the MACs.** `bpf/lib/l3.h`: `if (smac && eth_store_saddr(ctx, smac, 0) < 0) return DROP_WRITE_ERROR; if (dmac && eth_store_daddr(ctx, dmac, 0) < 0) return DROP_WRITE_ERROR;`.
- *Interpretation:* the destination interface and the destination MAC both come from `struct endpoint_info`, which the agent writes. There is no bridge FDB, so the host-side device's MAC cannot redirect another endpoint's traffic. The endpoint lookup key (destination IP) was not quoted.
- **Would it stop the hazard?** Yes, structurally: no MAC-learned forwarding table exists to poison.

#### B3.5 Kata Containers and Firecracker

- **Kata.** Kata connects each VM's TAP to the pod veth with TC redirects, not a bridge. `docs/design/architecture/networking.md`, `main`: "Kata Containers will create a tap device for the VM, `tap0_kata`, and setup a TC redirection filter to redirect traffic from `eth0`'s ingress to `tap0_kata`'s egress, and a second TC filter to redirect traffic from `tap0_kata`'s ingress to `eth0`'s egress." Also: "Kata Containers has deprecated support for bridge due to lacking performance relative to TC-filter and MACVTAP." *Interpretation:* delivery is a fixed ifindex-to-ifindex redirect, so a TAP MAC change steers nothing. Each VM's pair is private to its pod.
- **Firecracker.** Firecracker's guide gives each microVM its own TAP. The primary topology is NAT, and a bridge is an alternative (`docs/network-setup.md`, `main`: "Each microVM requires a host network interface (like `eth0`) and a Linux `tap` device (like `tap0`) used by Firecracker"; "Bridge-based, which exposes your microVM to the local network"; fetched summary). No security note about sharing a bridge was found.
  - Firecracker's shipped seccomp filter (`resources/seccomp/x86_64-unknown-linux-musl.json`, `main`, fetched summary) allows `TUNSETIFF`, `TUNSETOFFLOAD`, and `TUNSETVNETHDRSZ` on the VMM thread and `TUNSETOFFLOAD` on vCPU threads. It allows **no** `SIOC*` request, `SIOCSIFHWADDR` included, in any section. That is a contrast with CH (B2).
- **Would they stop the hazard?** Kata, yes: no FDB. Firecracker's routed topology, yes: no FDB. Its filter would also refuse the ioctl.

#### B3.6 AWS and GCP

These are official vendor documents. They are outside the prompt's trusted list and are cited as the vendors' own statements.
- **AWS.** "Each EC2 instance performs source/destination checks by default. This means that the instance must be the source or destination of any traffic it sends or receives." (<https://docs.aws.amazon.com/vpc/latest/userguide/work-with-nat-instances.html>).
- **GCP.** "By default, IP forwarding is disabled, and Google Cloud performs strict source address checking." (<https://docs.cloud.google.com/vpc/docs/using-routes>, fetched summary).
- *Interpretation:* both are IP-level checks enforced in the provider's fabric, outside the guest. Neither provider documents a customer-visible MAC-learning switch. Their internals are not public, so whether they address an analogue of this hazard is **UNVERIFIED**.

**Confidence**: High for libvirt, Neutron, `bridge(8)`, and Cilium (verbatim primary source). Medium for Kata, Firecracker, libvirt's documentation prose, and GCP (fetched summaries). The AWS/GCP internals are unverified.

### B4 — Candidate controls, judged against the source

This section records evidence only. It makes no design recommendation beyond noting what prior art does.

#### B4.1 (a) Structural: refuse host-to-TAP delivery unless the destination MAC is that TAP's registered guest MAC (or broadcast/multicast)

**Verdict: VERIFIED that both proposed hook points see the stolen frames on the attacker's TAP, with the destination MAC readable. It stops the theft but not the misdirection.**
- **TCX egress on each TAP.** The bridge hands the frame to the port with `skb->dev = to->dev` (`br_forward.c`, line 92) and ends in `dev_queue_xmit(skb)` after `skb_push(skb, ETH_HLEN)` (lines 35 and 53). `__dev_queue_xmit` then runs `nf_hook_egress` (the nftables `netdev`-family egress hook) and `sch_handle_egress`, which runs TCX egress, on that device (`net/core/dev.c`, lines 4851–4852 and 4860). The frame starts at the Ethernet header, so a TCX egress program on the attacker's TAP sees `h_dest` = the victim's guest MAC. It can compare that against the endpoint map's registered MAC for its own ifindex and drop.
- **nft `bridge` family.** Host-originated frames traverse `NF_BR_LOCAL_OUT` with `outdev` = the destination port (`__br_forward`, lines 110 and 115–117). Port-to-port frames traverse `NF_BR_FORWARD`. An `output`/`forward` rule keyed on `oifname` + `ether daddr` sees the same frame.
- **What it does not undo (interpretation from B1).** The FDB entry still says the victim's MAC is local on the attacker's port. The victim's host-to-guest frames are dropped at the attacker's TAP instead of being stolen, so the victim is still cut off until the entry is removed (B4.2). The same check also drops unknown-unicast floods addressed to other guests (B1.5 observation 1), because each non-target TAP sees a foreign destination MAC.
- **The ingress-only classifier in the current design does not cover this.** TCX ingress runs in `__netif_receive_skb_core` (`dev.c`, line 6115) on frames *from* the guest, before the bridge's `rx_handler` (line 6145). The stolen frames travel the TAP's egress.
- **Prior art.**
  - Neutron's OVS firewall selects the output port with `dl_dst=<registered port MAC>` (B3.2). That is the switch-level form of "deliver only if the destination is this port's registered MAC".
  - Cilium goes further. It writes the destination MAC *from* the endpoint record and redirects to the endpoint's ifindex (B3.4), so the destination is chosen rather than checked.
  - libvirt's nwfilter grammar could express an 'in'-direction `dstmacaddr` rule, but no shipped filter does (B3.1).
  - No prior art was found that adds a destination check *alongside* a learning Linux bridge specifically against a port MAC change.

#### B4.2 (b) Detection: audit the TAP's host-side MAC, then condemn and set the TAP down

**Verdict: VERIFIED that the change is observable immediately. Also VERIFIED from source that setting the TAP down does NOT remove the stolen entry. And there is an unavoidable leak window between the change and detection.**
- **Event-driven observation exists.** `NETDEV_CHANGEADDR` produces `RTM_NEWLINK` (`rtnetlink.c`, lines 7181 and 7192). `fdb_add_local` produces `RTM_NEWNEIGH` for the new `LOCAL` entry (`br_fdb.c`, line 456). `fdb_delete` produces `RTM_DELNEIGH` for the displaced entry (line 328). The kernel also logs `br_warn(... "adding interface %s with same address as a received packet ...")` (line 445). *Interpretation:* a netlink subscriber sees the change as it happens, whereas a periodic audit can miss a change that is reverted between samples. Neither closes the window in which frames were already delivered to the attacker.
- **Setting the TAP down leaves the stolen entry in place.** `NETDEV_DOWN` on a port runs `br_stp_disable_port` (`br.c`, lines 108–111). That calls `br_fdb_delete_by_port(br, p, 0, 0)` (`br_stp_if.c`, line 117), which skips static entries when `do_all` is 0:
  ```c
  		if (!do_all)
  			if (test_bit(BR_FDB_STATIC, &f->flags) ||
  			    (test_bit(BR_FDB_ADDED_BY_EXT_LEARN, &f->flags) &&
  			     !test_bit(BR_FDB_OFFLOADED, &f->flags)) ||
  			    (vid && f->key.vlan_id != vid))
  				continue;
  ```
  (`br_fdb.c`, lines 885–890). The stolen entry is `LOCAL|STATIC` (line 451), so it survives. Host frames for the victim then reach `br_forward`, and `should_deliver` fails because the port is no longer forwarding (`br_forward.c`, lines 27–28 and 162–172), so they are dropped. *Interpretation:* after "set down", the theft stops but the victim stays cut off.
- **What does clear it.**
  - Removing the port from the bridge: `br_del_if` → `br_fdb_delete_by_port(br, p, 0, 1)` (`br_if.c`, line 358), which calls `fdb_delete_local`. Device unregister reaches the same function (`br.c`, lines 126–127).
  - Reverting the port's MAC: `br_fdb_changeaddr`'s delete loop (`br_fdb.c`, lines 472–477).

  In both cases `fdb_delete_local` deletes the entry unless another port or the bridge has that address (lines 348–369). The victim's MAC then has no entry until the victim guest transmits again. In that interval host frames to it are flooded to every flood-enabled port (B1.5 observation 1).
- **Prior art.** No primary source was found in which libvirt, Neutron, Kata, Firecracker, or Cilium audits a TAP's host-side MAC and condemns on a change.

#### B4.3 Pinning with a static FDB entry or bridge-port configuration

**Verdict: REFUTED for `static`, `sticky`, and every port flag. `permanent` pinning holds in source, with side effects, and is UNVERIFIED at runtime.**
- **`static` and `sticky` entries are deleted.** `fdb_add_local` keeps an existing entry only if it is `BR_FDB_LOCAL` (`br_fdb.c`, lines 443–444). iproute2's `static` is `NUD_NOARP`, which clears `LOCAL` (lines 1211–1214). `sticky` is consulted only by learning (line 1000). The attacker's `SIOCSIFHWADDR` therefore deletes a `static` or `static sticky` pin of the victim's MAC (line 447) and installs its own.
- **Port flags do not apply.** `learning`, `locked`, `isolated`, and `flood` do not gate `br_fdb_changeaddr` (B3.3 table).
- **A `permanent` (local) pin of each guest's MAC on its own port survives in source.** `NUD_PERMANENT` sets `BR_FDB_LOCAL` (lines 1207–1210), and the attacker's `fdb_add_local` then returns 0 (line 444). `br_fdb_changeaddr` deletes only entries whose `dst` is the *attacker's* port (line 473). Host-originated frames still go out of the victim's port, because `br_dev_xmit` forwards to `dst->dst` without testing `LOCAL` (B1.3). Side effects follow from the same source (interpretation):
  1. Guest-to-guest L2 frames to that MAC are passed up to the host instead of being forwarded (`br_input.c`, lines 221–222).
  2. Every frame the guest sends triggers the rate-limited "received packet on %s with own address as source address" warning (`br_fdb.c`, lines 985–988). This is the log libvirt's TAP-MAC choice exists to avoid (B3.1).
  3. It is incompatible with `locked`, because a locked port drops frames whose source entry is `LOCAL` (`br_input.c`, lines 126–129).
  4. `bridge(8)` describes `permanent` as "terminate them locally", which the transmit path does not honour for host-originated frames (Conflict C1). Relying on it therefore depends on unspecified behaviour.
- **Prior art.** None was found that uses local/permanent FDB entries for guest MACs. libvirt deliberately keeps the TAP MAC different from the guest MAC for the reasons in side effects 2 and 4.

#### B4.4 Does any kernel mechanism stop an unprivileged fd holder from changing the netdev MAC?

**Verdict: none in the tun/netdev/bridge path (VERIFIED by source). The only generic chokepoints are the LSM ioctl hook and seccomp.**
- **Nothing in the path.** B1.1 lists every check on the path: attachment only, then `ss_family`, device presence, the vetoable `NETDEV_PRE_CHANGEADDR`, `IFF_LIVE_ADDR_CHANGE`, and `is_valid_ether_addr`. User space cannot clear `IFF_LIVE_ADDR_CHANGE`, which tun sets for every TAP (tun.c, line 1416). *Contrast:* the bridge does veto one port change, `NETDEV_PRE_TYPE_CHANGE` (`br.c`, lines 136–138: `/* Forbid underlying device to change its type. */ return NOTIFY_BAD;`), and has no equivalent for addresses.
- **LSM hook.** Every `ioctl(2)` calls `security_file_ioctl(fd_file(f), cmd, arg)` before `do_vfs_ioctl` (`fs/ioctl.c`, lines 591–595). The compat entry calls `security_file_ioctl_compat` (line 647). `file_ioctl` is an LSM hook (`include/linux/lsm_hook_defs.h`, line 197), and BPF LSM programs "allow runtime instrumentation of the LSM hooks by privileged users to implement system-wide MAC (Mandatory Access Control)" (`Documentation/bpf/prog_lsm.rst`, lines 8–10). *Interpretation:* a BPF-LSM program on `file_ioctl` and `file_ioctl_compat` could refuse `SIOCSIFHWADDR` on the tun character device for the confined uid. Unlike Landlock (A5), the hook fires on every call, so it covers the inherited descriptor. No prior art doing exactly this was found.
- **seccomp installed by the launcher.** seccomp(2): "If fork(2) or clone(2) is allowed by the filter, any child processes will be constrained to the same system call filters as the parent. If execve(2) is allowed, the existing filters will be preserved across a call to execve(2)." When several filters exist, the result is "the first-seen action value of highest precedence".
  - *Interpretation:* a filter installed before `exec` that returns an error for `ioctl` with `args[1] == SIOCSIFHWADDR` would bind every CH thread, the main thread included, which CH's own per-thread filters do not (B2). CH's own filter cannot relax it.
  - CH's `set_mac_addr` issues `SIOCSIFHWADDR` only when changing the address (B2). Whether the design ever configures `host_mac` was not checked.
  - Prior art: Firecracker's shipped filter allows no `SIOC*` request (B3.5).

**Confidence**: High for B4.1–B4.4's kernel facts (verbatim, one tree). The runtime behaviour of the `permanent` pin and of each hook placement is not executed natively. The effectiveness judgements are labelled interpretations.

### Addendum 2 knowledge gaps

1. **Nothing was executed natively.** Every B1 and B4 conclusion is a source reading. A native reproduction would decide R5-H1: set a TAP's host MAC via its queue fd to another guest's MAC as uid 4200, then observe `bridge fdb show` and where host-to-guest frames land.
2. **CH thread coverage** rests on fetched summaries of `vmm/src/lib.rs` and `cloud-hypervisor/src/main.rs`, not a line-by-line read (B2).
3. **How attackers learn guest MACs.** Whether the design floods guest ARP broadcasts to other TAPs, or derives MACs predictably, was not checked in the repository (B1.4).
4. **Design facts not checked in the repository.** Whether the bridge MAC is `NET_ADDR_SET` (B1.5 observation 2), whether TAP ports use `flood off` (observation 1), and whether `host_mac` is ever configured (B4.4).
5. **OVS behaviour on a port netdev's MAC change** was not read (B3.2).
6. **AWS/GCP fabric internals** are not public. Only their IP-level source/destination-check statements were found (B3.6).
7. **No primary source names this exact hazard.** Searches of libvirt, Neutron, Firecracker, Kata, and Cilium sources and documentation found no discussion of a VMM changing its TAP's host-side MAC to redirect a shared bridge's traffic. libvirt's `virnetdevtap.c` comment is the closest: it documents the kernel behaviour, not the attack.
8. **QEMU's analogous exposure is not researched.** libvirt passes TAP fds to QEMU in the same way. Whether QEMU's `-sandbox` or sVirt/SELinux ioctl policy refuses `SIOCSIFHWADDR` was not researched.

### Addendum 2 conflicting information

#### Conflict C1: Does a local FDB entry "terminate locally" or transmit out of its port?

- **Position A.** `bridge(8)`, for `local`/`permanent`: "the bridge will not forward frames with this destination MAC address and VLAN ID, but terminate them locally." (<https://man7.org/linux/man-pages/man8/bridge.8.html>)
- **Position B.** For **host-originated** frames, `br_dev_xmit` forwards to the entry's port without testing `BR_FDB_LOCAL` (`br_device.c`, lines 109–110). Only frames **received from a port** are terminated locally (`br_input.c`, lines 221–222).
- **Assessment.** The kernel source is authoritative, and the man page describes only the receive path. The hijack in R5-H1 depends on Position B, so its truth does not depend on the man page's wording.

### Addendum 2 sources

All accessed 2026-09-24. "7.2" means the local checkout `/Users/marcus/git/linux` (`Makefile` 7.2.0); upstream is <https://github.com/torvalds/linux>.
- **[B-1] Linux kernel 7.2.**
  - `drivers/net/tun.c`, `net/core/dev_api.c`, `net/core/dev.c`, `net/core/dev_ioctl.c`, `net/core/rtnetlink.c`, `net/ethernet/eth.c`
  - `net/bridge/br.c`, `net/bridge/br_fdb.c`, `net/bridge/br_device.c`, `net/bridge/br_forward.c`, `net/bridge/br_input.c`, `net/bridge/br_stp_if.c`, `net/bridge/br_if.c`, `net/bridge/br_private.h`
  - `fs/ioctl.c`, `include/linux/lsm_hook_defs.h`, `Documentation/bpf/prog_lsm.rst`
- **[B-2] Cloud Hypervisor `v53.0`.**
  - `vmm/src/seccomp_filters.rs`, `virtio-devices/src/seccomp_filters.rs`, `net_util/src/tap.rs`, `vmm/src/lib.rs`, `cloud-hypervisor/src/main.rs`
  - <https://github.com/cloud-hypervisor/cloud-hypervisor/tree/v53.0>
- **[B-3] Linux man pages.** `seccomp(2)`, `bridge(8)`. <https://man7.org/linux/man-pages/>
- **[B-4] libvirt `master`.**
  - `src/nwfilter/xml/no-mac-spoofing.xml`, `src/nwfilter/xml/clean-traffic.xml`, `src/util/virnetdevtap.c`: <https://github.com/libvirt/libvirt>
  - <https://libvirt.org/formatnwfilter.html>
- **[B-5] OpenStack Neutron tag `22.0.0`.**
  - `neutron/plugins/ml2/drivers/linuxbridge/agent/arp_protect.py`, `neutron/agent/linux/openvswitch_firewall/firewall.py`: <https://github.com/openstack/neutron/tree/22.0.0>
  - `linuxbridge_agent.ini` sample: <https://docs.openstack.org/newton/config-reference/networking/samples/linuxbridge_agent.ini.html>
  - Liberty spec `arp-spoof-filtering-ebtables.rst`: <https://github.com/openstack/neutron-specs>
- **[B-6] Cilium `main`.** `bpf/lib/local_delivery.h`, `bpf/lib/l3.h`. <https://github.com/cilium/cilium>
- **[B-7] Kata Containers `main`.** `docs/design/architecture/networking.md`. <https://github.com/kata-containers/kata-containers>
- **[B-8] Firecracker `main`.** `docs/network-setup.md`, `resources/seccomp/x86_64-unknown-linux-musl.json`. <https://github.com/firecracker-microvm/firecracker>
- **[B-9] AWS VPC User Guide**, "Disable source/destination checks": <https://docs.aws.amazon.com/vpc/latest/userguide/work-with-nat-instances.html>. **GCP VPC**, "Use routes": <https://docs.cloud.google.com/vpc/docs/using-routes>. Official vendor documentation, outside the prompt's trusted list.
- **[B-10] Overdrive repository (current tree, for context only).** `crates/overdrive-core/src/vm/config.rs` (`seccomp_arg`), `crates/overdrive-host/src/vmm.rs` (`cloud_hypervisor_network_arg`).

### Addendum 2 metadata

- **Method.** Kernel source was read locally, verbatim, with line numbers. Cloud Hypervisor, libvirt, Neutron, Cilium, Kata, and Firecracker sources were fetched as raw text. The load-bearing CH seccomp rule was fetched twice with independent prompts. About 35 reads and fetches were examined and 10 source groups are cited.
- **Tool notes.** Two Neutron stable-branch URLs returned 404, so tag `22.0.0` was used. The Cilium local-delivery code has moved from `l3.h` to `local_delivery.h` on `main`.
- **Scope.** No design artifact was edited, nothing was committed, and no GitHub action was taken.

### Addendum 2 summary table

| # | Question | Verdict |
|---|---|---|
| B1 | Holder's `SIOCSIFHWADDR` on the TAP queue fd → bridge `LOCAL` entry on its port → host frames for that MAC transmitted to it | **VERIFIED** end to end in 7.2 source. Precondition: the attacker copies the victim *guest's* MAC and must first learn it. Displaces learned *and* user-`static` entries. Sticky against re-learning. Adjacent: unknown-unicast flood leak; bridge-MAC move if the bridge MAC is not `NET_ADDR_SET`. |
| B2 | CH v53.0 seccomp blocks `SIOCSIFHWADDR` on the threads that hold the queue fd | **REFUTED.** The VMM-thread rule allows it, keyed on the request number only (any fd). Virtio-net and vCPU threads do not allow it. The main thread is unfiltered (Medium). `--seccomp true` does not cover every thread. |
| B3 | Prior art | **VERIFIED** classification. Source-MAC anti-spoofing (libvirt nwfilter, Neutron ebtables, bridge `locked`, AWS/GCP src/dst checks) does not see the ioctl. Registered-record delivery (Neutron OVS firewall `dl_dst`, Cilium `ep->ifindex`/`ep->mac`, Kata TC redirect, Firecracker routed TAP) is not steered by a port's MAC. |

**Candidate control → prior-art precedent → does it stop the hazard?**

| Candidate control | Prior-art precedent | Stops the hazard? (source-based; not executed) |
|---|---|---|
| (a) TCX egress on each TAP: drop unicast whose destination MAC ≠ that TAP's registered guest MAC | Neutron OVS firewall selects the output port by `dl_dst=<registered MAC>`; Cilium writes `ep->mac` and redirects to `ep->ifindex` | **Stops the theft.** The stolen frame is visible at the attacker's TAP egress (`dev.c`, line 4860). Also stops the unknown-unicast flood leak. **Does not stop the misdirection:** the victim stays cut off while the stolen entry exists. |
| (a′) nft `bridge` output/forward rule on `oifname` + `ether daddr` | libvirt nwfilter / Neutron use ebtables in the bridge family, but for *source* MACs only | **Same as (a)**, at `NF_BR_LOCAL_OUT` / `NF_BR_FORWARD` (`br_forward.c`, lines 98, 110, 115–117). |
| (b) Detect the host-side MAC change, then condemn and set the TAP down | None found | **Partial.** Observable at once via `RTM_NEWLINK`/`RTM_NEWNEIGH`, but frames leak until detection. "Set down" **keeps** the stolen `LOCAL\|STATIC` entry (`br_fdb.c`, lines 885–890), so the victim stays cut off until the port leaves the bridge or its MAC is reverted, and then floods until the victim re-learns. |
| Pin the victim MAC with a `static` / `static sticky` FDB entry | Common operator practice; no anti-hijack precedent found | **No.** It is non-local and deleted by `fdb_add_local` (`br_fdb.c`, lines 443–447). |
| Pin the victim MAC with a `permanent` (local) FDB entry on its own port | None found. libvirt deliberately avoids TAP MAC == guest MAC | **In source, yes.** Side effects: guest-to-guest frames terminate on the host; a rate-limited warning per guest frame; incompatible with `locked`; relies on transmit behaviour the man page contradicts (C1). **UNVERIFIED** at runtime. |
| Bridge port flags `learning off` / `locked` / `isolated` | libvirt, Neutron, and `bridge(8)` use them against *guest* spoofing | **No.** `br_fdb_changeaddr` ignores them. `isolated` does not apply to host-originated frames. |
| `flood off` on TAP ports | None found for this purpose | **No** for the hijack. **Yes** for the unknown-unicast leak. |
| Guest-source MAC anti-spoofing (the existing ingress TCX, nwfilter-style) | libvirt `no-mac-spoofing`, Neutron `neutronMAC-` chains, AWS/GCP checks | **No.** It addresses the learning-poisoning variant, not the ioctl. |
| CH's built-in seccomp (`--seccomp true`) | Firecracker's filter omits all `SIOC*` | **No.** CH's VMM thread allows `SIOCSIFHWADDR`, and the main thread is unfiltered. |
| Launcher-installed pre-`exec` seccomp denying `ioctl(…, SIOCSIFHWADDR, …)` | Firecracker's filter (by omission) | **In source, yes**, for every thread, since filters are inherited and stacked (seccomp(2)). **UNVERIFIED** natively. |
| BPF-LSM `file_ioctl` / `file_ioctl_compat` denying `SIOCSIFHWADDR` on the tun device | None found for this exact ioctl | **In source, yes.** The hook runs on every `ioctl(2)` (`fs/ioctl.c`, lines 591 and 647) and covers inherited fds. **UNVERIFIED** natively. |
| Any kernel flag or capability that makes a TAP's MAC immutable to the fd holder | None exists | **No such mechanism** in tun, core, or the bridge. The bridge vetoes type changes but not address changes (`br.c`, lines 136–138). |
