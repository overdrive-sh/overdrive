# ADR-0143 — A seccomp filter installed at VMM launch denies every Cloud Hypervisor thread the TAP-mutating ioctls

## Status

**Accepted (2026-09-24).** GH #295 correctness-recovery replacement DESIGN,
decision D-295-R22. Approved by the user on 2026-09-24, who asked for the
compromised-VMM TAP-ioctl hazards to be prevented at their source rather than
accepted as residuals. The approval rests on the native evidence of spike
increment-aa (`docs/feature/netns-density-295/spike/findings-tap-ioctl-seccomp.md`,
verdict WORKS: native x86_64 metal, kernel `7.0.0-29`, Cloud Hypervisor v53.0 at
commit `9ed824d6d08df3e96f7d5f50795d9449ac99f431`). Depends on ADR-0127 and
ADR-0128 (the inherited TAP queue), ADR-0129 (the one audited launch hook that
installs the filter), and ADR-0130 (the uid-0 TAP owner and the audit read-back
set). It complements ADR-0142 (delivery by registered guest MAC). The user
ruled on 2026-09-24 that #295 ships the filter for x86_64 only. Proving and
enabling it on aarch64 is
[GH #302](https://github.com/overdrive-sh/overdrive/issues/302). The exact
program, request-value derivation, probe, and evidence live only in the #295
feature delta.

## Context

Each Cloud Hypervisor process holds its own TAP's queue descriptor as uid 4200
without `CAP_NET_ADMIN` (ADR-0130). Before the `__tun_chr_ioctl` switch the
kernel checks only that the descriptor is attached (`drivers/net/tun.c`), so a
compromised VMM can issue every own-queue ioctl against its TAP. Several reach
beyond the holder's own guest or state the network owner owns:

- `SIOCSIFHWADDR` sets the TAP's host-side MAC. Set to another guest's MAC, it
  poisons the shared bridge's forwarding database (reproduced natively in spike
  increment-z). ADR-0142's egress classifier keeps the redirected frames from
  the holder, but the victim receives no host unicast until the holder's TAP is
  torn down.
- `TUNSETOWNER` hands the TAP to uid 4200, so after the holder exits a second
  uid-4200 process could attach it by name.
- `TUNSETDEBUG` sets the TAP's `msg_enable`, after which the kernel writes an
  unratelimited host kernel-log line for every ioctl on the queue and every
  frame toward the guest.
- `TUNSETPERSIST`, `TUNSETGROUP`, `TUNSETCARRIER`, and `TUNSETLINK` mutate TAP
  state the shared guest-network owner owns. The tun TX filter, the classic and
  eBPF filter and steering controls, and `TUNSETQUEUE` are unused by Cloud
  Hypervisor on this launch shape.

Cloud Hypervisor's own `--seccomp` does not close these. Its filters are
per-thread, the process's main thread carries none, and the VMM thread's filter
allows `SIOCSIFHWADDR` keyed only on the request number (research
`docs/research/networking/netns-density-295-replacement-design-prior-art-comprehensive-research.md`,
Addendum 2 B2; the increment-aa control run reproduced the unfiltered leader).
Firecracker's shipped filter allows no `SIOC*` request at all (Addendum 2
B3.5).

The kernel keeps a process's seccomp filters across `fork`, `clone`, and
`execve`, and evaluates every stacked filter, taking the most restrictive
action (seccomp(2)). A filter loaded in the single-threaded launcher child
before its first exec therefore binds every thread Cloud Hypervisor later
creates, and Cloud Hypervisor's own filters cannot relax it.

On the `fd=` path, Cloud Hypervisor v53's net-device creation and activation
issue exactly `TUNGETIFF`, `TUNSETIFF` (accepting `EEXIST` on the
already-attached queue), `TUNSETVNETHDRSZ`, `SIOCGIFMTU`, `SIOCSIFMTU` (only
when `mtu=` is configured), and `TUNSETOFFLOAD` (increment-aa source audit at
the tag commit). None of the requests above is among them.

Increment-aa measured the filter on native metal. Installed in the child after
`PR_SET_NO_NEW_PRIVS` and before the first exec:

- it survived the `prlimit` → `setpriv` → Cloud Hypervisor exec chain;
- all 11 Cloud Hypervisor threads, the leader included (0 filters in the
  control), gained exactly one filter;
- Cloud Hypervisor's own `--seccomp true` filters stacked on top without
  relaxing it;
- Cloud Hypervisor reached guest READY and passed ICMP in both directions, with
  no frame captured before activation;
- in a helper process under the same filter, holding an attached queue, each
  of the 13 denied requests returned `EPERM` from its main thread and from
  threads created after installation. The unfiltered control returned no
  `EPERM`.

That evidence is for x86_64 only. Cloud Hypervisor does not run in the Lima
VM, and no aarch64 host with KVM is available. An aarch64 program therefore
cannot be proven against Cloud Hypervisor on native hardware.

A deny-list whose default is allow can be bypassed through another syscall ABI.
An i386 compat syscall on x86_64 carries a different audit architecture. An x32
syscall carries the same `AUDIT_ARCH_X86_64` (`arch/x86/include/asm/syscall.h`)
with a syscall number that has `__X32_SYSCALL_BIT` set. The x32 `ioctl` is
entry 514, `compat_sys_ioctl`, which reaches `__tun_chr_ioctl` through
`tun_chr_compat_ioctl` (`drivers/net/tun.c`). seccomp(2) names this bypass for
deny-lists keyed only on the native syscall number. No other route reaches the
tun ioctl handler. io_uring cannot: `tun_fops` defines no `uring_cmd` and
io_uring has no generic ioctl opcode. The socket-path `SIOCSIFHWADDR` needs
`CAP_NET_ADMIN`.

## Decision

Every Cloud Hypervisor launch installs a seccomp filter in the forked launcher
child, before that child's first exec. The filter returns `EPERM` for these
`ioctl` requests and allows every other syscall:

- `SIOCSIFHWADDR`
- `TUNSETOWNER`
- `TUNSETGROUP`
- `TUNSETPERSIST`
- `TUNSETCARRIER`
- `TUNSETDEBUG`
- `TUNSETLINK`
- `TUNSETTXFILTER`
- `TUNATTACHFILTER`
- `TUNDETACHFILTER`
- `TUNSETSTEERINGEBPF`
- `TUNSETFILTEREBPF`
- `TUNSETQUEUE`

It matches the low 32 bits of the request argument, which is the kernel's
`unsigned int cmd`, on any descriptor. Where a Cloud Hypervisor thread's own
filter already traps a denied request, that stricter action applies instead of
`EPERM`; either way the request never reaches the tun ioctl handler. It fails
closed on another syscall ABI: a syscall carrying a foreign audit architecture
(an i386 compat syscall), or an x32 syscall, kills the process.

The filter persists across the `prlimit` → `setpriv` → Cloud Hypervisor exec
chain, and every Cloud Hypervisor thread inherits it. No launch proceeds
without it.

The filter exists for the 64-bit x86_64 target only. Every other target,
aarch64 and the x32 target included, has no program, and no microVM starts on
it:

- The VMM adapter's startup probe fails. Under the existing composition rule
  for a failed probe (ADR-0083 §D3c, capability absence), the node composes no
  microVM driver, and every microVM start is refused.
- The VMM adapter's `create` also refuses before any effect, so no path
  launches Cloud Hypervisor unfiltered.

A kernel that refuses the filter fails the startup probe in the same way, and
fails the spawn of any launch. Proving the filter on aarch64 and enabling
aarch64 launches is [GH #302](https://github.com/overdrive-sh/overdrive/issues/302).

The filter is a VMM confinement layer beside Cloud Hypervisor's own
`--seccomp`, the uid drop, the rlimits, and Landlock. It moves no TAP state:
the shared guest-network owner still owns every TAP, and the launcher still
owns only its per-launch queue.

## Alternatives considered

### Keep delivery control and detection only, and accept the residuals

Rejected by the user on 2026-09-24. ADR-0142's egress classifier and ADR-0130's
audit read-back act only after the mutation. The victim of an FDB poisoning
loses host-to-guest delivery until the holder's teardown. A holder can re-grant
its TAP to uid 4200 with `TUNSETOWNER`. A holder can flood the host log with
`TUNSETDEBUG`. The filter prevents all three.

### A BPF-LSM `file_ioctl` hook on the tun device

Rejected. It is a node-global mandatory-access-control policy over every
process's ioctls, a far larger surface and a different confinement boundary.
It would prevent nothing the launch filter does not.

### Rely on Cloud Hypervisor's own `--seccomp`

Rejected on evidence. The filters are per-thread, and the main thread has none.
The VMM thread allows `SIOCSIFHWADDR` for any descriptor (Addendum 2 B2; the
increment-aa control).

### Deny only on the queue's descriptor number

Rejected. A seccomp filter sees the descriptor number, not the file it names.
Cloud Hypervisor may duplicate the queue to another number, so a deny keyed on
descriptor 3 is bypassable. Matching the request number on every descriptor is
safe for this launch shape, where Cloud Hypervisor issues none of the denied
requests.

### An allow-list of every syscall Cloud Hypervisor issues

Rejected. Cloud Hypervisor's own per-thread filters are already allow-lists.
A process-wide allow-list would duplicate them and tie every Cloud Hypervisor
upgrade to a full syscall audit, while the hazard is a closed set of ioctl
requests on one syscall.

### Ship an aarch64 program built from source reading

Rejected by the user on 2026-09-24. The audit architecture, the syscall number,
and the absence of a compat route could be derived from kernel headers, but
Cloud Hypervisor's compatibility with the filter on aarch64 would stay unproven.
No aarch64 host with KVM is available, and Cloud Hypervisor does not run in
the Lima VM. An unproven filter is not shipped on a confinement boundary.
[GH #302](https://github.com/overdrive-sh/overdrive/issues/302) tracks proving
it and enabling aarch64 launches.

### Launch without the filter on a target that has no program

Rejected. It would hand a TAP queue to a VMM that this decision says cannot
mutate its TAP, and the FDB poisoning, the `TUNSETOWNER` re-grant, and the
`TUNSETDEBUG` host-log flood would return on that target.

### Kill the process on a denied request instead of returning `EPERM`

Rejected. `EPERM` is the natively proven action, and it already refuses the
request. A denied request fails the call, which Cloud Hypervisor reports as its
own error, rather than ending the VM with an unexplained `SIGSYS`. The kill
action is reserved for the alternative-ABI routes, which no legitimate Cloud
Hypervisor call takes.

## Consequences

Positive:

- The mutations are prevented at their source. A VMM can no longer poison the
  bridge forwarding database, so the ADR-0142 victim outage no longer arises
  from it. It can no longer re-grant its TAP with `TUNSETOWNER`, or flood the
  host log with `TUNSETDEBUG`. It can no longer change the TAP's persistence,
  group, carrier, or link type, install tun filters or steering, or attach
  another queue.
- The filter binds every Cloud Hypervisor thread, the unfiltered main thread
  included, which Cloud Hypervisor's own filters do not.
- ADR-0142's egress classifier and ADR-0130's read-back stay as independent
  layers. The classifier still closes the unknown-unicast flood leak, which
  needs no ioctl. The read-back of host-side MAC, owner, persistence, and debug
  mask detects a change made outside the VMM or through a gap in the filter.
- No new crate is added. The program is a classic-BPF program built over the
  `libc` types the workspace already locks. The mechanism is chosen, with its
  evidence, in the feature delta.

Negative:

- The filter is coupled to the launch shape. It matches request numbers on every
  descriptor, so a `host_mac=`, named-TAP, or multiqueue launch would need a
  denied request and is incompatible with it. Any change to the Cloud Hypervisor
  version the appliance ships, to the `--net` launch shape, or to the Cloud
  Hypervisor net-device paths the platform uses repeats the source audit and
  the native boot-and-traffic case before it lands. An unaudited version that
  does issue a denied request fails the operation rather than bypassing the
  filter.
- The filter is coupled to the syscall ABI. The program exists for 64-bit
  x86_64 only, with request values and audit architecture taken from that
  target's headers. The probe's program is natively proven, and the production
  program's x32 prologue carries a native evidence obligation.
- No microVM starts on any other target, aarch64 and the x32 target included.
  Such a node composes no microVM driver, so every microVM start on it is
  refused. Proving the filter on aarch64 and enabling aarch64 launches is
  [GH #302](https://github.com/overdrive-sh/overdrive/issues/302).
- The single audited launch hook (ADR-0129) gains two raw syscalls,
  `prctl(PR_SET_NO_NEW_PRIVS)` and `seccomp(SECCOMP_SET_MODE_FILTER)`, beside
  `close_range`. It remains `overdrive-host`'s only production `unsafe`
  function.
- `no_new_privs` is set before the first exec, so `prlimit` and `setpriv` also
  run with it. `setpriv` already sets it for Cloud Hypervisor, and the proven
  chain ran this way.
- The VMM adapter's startup probe gains one stage. A host whose kernel refuses
  the filter, or whose target has no program, fails that probe and composes no
  microVM driver (ADR-0083 §D3c).
- It is a deny-list. It closes the listed TAP-mutation set, not Cloud
  Hypervisor's whole syscall surface, which remains the job of Cloud
  Hypervisor's own filters and Landlock.
