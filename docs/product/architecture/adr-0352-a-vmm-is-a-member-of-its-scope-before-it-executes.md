# ADR-0352 — A VMM is a member of its allocation's scope before it executes

## Status

**Accepted** 2026-11-23 (decision RB-9 of the node-local persistent volume feature, GH #348; user ruling of 2026-11-23, review finding F1: the correct
mechanism, `clone3` with `CLONE_INTO_CGROUP`, over the smaller write form). Not implemented. The interface contract is in
[`feature-delta.md`](../../feature/node-local-persistent-volume/feature-delta.md) § "A VMM is born inside its scope". Depends on
[ADR-0082](adr-0082-vmm-port-trait-and-vmconfig-anti-corruption-value.md) (`VmConfig.cgroup_scope`) and the launch confinement of
[ADR-0129](adr-0129-safe-descriptor-mapping-for-vmm-launch.md) and [ADR-0143](adr-0143-vmm-launch-seccomp-filter-denies-tap-mutating-ioctls.md).

## Context

The lease gate (ADR-0156, second amendment) and boot reclamation read a VM's scope to decide whether the VMM still holds its vsock device and CID:
an empty scope is removed and its offset released or not reserved. That is sound only if a process that holds the device is always a member of the
scope. The VMM is a process tree created by the host adapter; the driver creates the scope before `Vmm::create` and writes the scope's limits
before it. A VMM that is outside the scope for any interval, however short, can be the one `serve` leaves behind when it crashes: boot reads
`populated 0`, removes the scope and reserves nothing, and a later launch fails `EADDRINUSE`.

## Decision

**`Vmm::create` creates the VMM process inside its allocation's scope, atomically.** The VMM is spawned with `clone3` and `CLONE_INTO_CGROUP`, the
`cgroup` argument being a descriptor of the scope's directory. The kernel builds the child's css_set for the scope before the child is made
runnable, so the first instruction of the first program of the launch chain runs inside the scope, and every process and thread of the VMM tree
descends from it. The memory and CPU limits the driver wrote before the spawn apply from that instruction. No process of the VMM is ever a member
of any other cgroup, and no step of the launch depends on a later write to `cgroup.procs`.

A refused placement (a scope that is gone, not writable by the caller, or not a valid target) fails `create` and creates no process.

The spawn is a dedicated host-adapter primitive, because neither `std::process::Command` nor `tokio::process::Command` can pass the flag. It owns
the whole launch: placement, standard I/O, the audited confinement steps (ADR-0129, ADR-0143) and `exec`. It returns a pidfd, which the adapter
uses to observe the process's exit and to deliver the kill.

## Alternatives considered

- **The child writes `0` to the scope's `cgroup.procs` as the first step of the launch hook.** Rejected by the user. The child exists, runnable and
  charged to `serve`'s cgroup, between the fork and the write: it is outside its scope for that interval, and the resource it consumes in it (page
  tables and the copied address space of the forked process, the hook's own work) is accounted to `serve`'s cgroup and not the allocation's. The
  guarantee that no process holding the device is outside the scope then rests on the interval being harmless, an argument about what the child does
  and does not hold in it, not a property the kernel gives.
- **The driver writes the VMM's pid to `cgroup.procs` after the spawn (the earlier behaviour).** Rejected: the same window, longer and with the VMM
  already running; the write places only the one pid, so every thread and child created before it stays outside the scope; and a `serve` crash
  between the spawn and the write leaves a vsock-holding VMM outside an empty scope.
- **Read the VMM's membership at boot and move stragglers into the scope.** Rejected: a repair path for a state the design can make unreachable, and
  it cannot find a VMM that has already exited past the reader.

## Consequences

- The port postcondition is the same as for any placement mechanism: on `Ok` the VMM was a member of `config.cgroup_scope` from its first
  instruction; on `Err` no process remains.
- A new `unsafe` surface replaces the audited `pre_exec` hook: one private module of `overdrive-host`, the crate's only production
  `#[allow(unsafe_code)]`. `std` and `tokio` child supervision are replaced by a pidfd for exit observation and kill, so the adapter keeps its
  bounded-reap behaviour (ADR-0348) on the new handle.
- The launch's confinement steps run in the clone child, and the probe that exercises them exercises this primitive.
- `CloudHypervisorVmm` needs the cgroup root, and the driver's `place_pid_in_scope` call, the cgroup manager's method and its test are deleted.
- The mechanism needs the kernel's `CLONE_INTO_CGROUP`, which the pinned appliance kernel provides (ADR-0068; `kernel/fork.c:2840`, `:2899 @ v7.0`).
