# ADR-0352 — A VMM is a member of its allocation's scope before it executes

## Status

**Proposed** 2026-11-23 (decision RB-9 of the node-local persistent volume feature, GH #348; the contract is the user's instruction of 2026-11-23,
review finding F1; the mechanism awaits the user's approval). Not implemented. The interface contract is in
[`feature-delta.md`](../../feature/node-local-persistent-volume/feature-delta.md) § "A VMM is born inside its scope". Depends on
[ADR-0082](adr-0082-vmm-port-trait-and-vmconfig-anti-corruption-value.md) (`VmConfig.cgroup_scope`) and the audited launch hook of
[ADR-0129](adr-0129-safe-descriptor-mapping-for-vmm-launch.md) and [ADR-0143](adr-0143-vmm-launch-seccomp-filter-denies-tap-mutating-ioctls.md).

## Context

The lease gate (ADR-0156, second amendment) and boot reclamation read a VM's scope to decide whether the VMM still holds its vsock device and CID:
an empty scope is removed and its offset released or not reserved. That is sound only if a process that holds the device is always a member of the
scope. Today the driver creates the scope, `Vmm::create` spawns the VMM, and only then the driver writes the VMM's pid to the scope's
`cgroup.procs`. The spawn does no cgroup enrolment, and `VmConfig.cgroup_scope` is not read by it. A `serve` crash between the spawn and the write
leaves a VMM, holding a vsock device, outside an empty scope; boot reads `populated 0`, removes the scope and reserves nothing, and a later launch
fails `EADDRINUSE`. The stop path's gate has the same hole, and writing only the first pid leaves every thread and child created before the write
outside the scope.

## Decision

**`Vmm::create` returns a VMM whose process was a member of the allocation's scope before it executed the first program of the launch chain.** Every
process and thread of the VMM tree is then a member from its first instruction, so no process that holds the device is ever outside the scope.

The mechanism: the forked child writes `0` to the scope's `cgroup.procs` as the first step of the single audited launch hook, with the descriptor
opened by the parent before the fork. The driver's later write of the pid is deleted.

## Alternatives considered

- **The driver writes the pid after the spawn (today).** Rejected: the crash window above, and a VMM running with no resource limit and no
  membership until the write.
- **`clone3` with `CLONE_INTO_CGROUP`.** Equivalent in effect: the child's css_set is built before it runs
  (`cgroup_css_set_fork`, `kernel/cgroup/cgroup.c:6769-6852 @ v7.0`; accepted for `clone3` only, `kernel/fork.c:2840`, `:2899 @ v7.0`). Rejected as
  the mechanism: neither `std::process::Command` nor `tokio::process::Command` can pass the flag, so it needs a raw `clone3`, an `exec` and its own
  child supervision, and a second `unsafe` surface beside the one audited hook, for no property the write form lacks. The write form has a window
  between the fork and the write in which the child is not yet a member, and the child holds no vsock device in it: the launcher passes no vsock
  descriptor, the hypervisor opens `/dev/vhost-vsock` itself after `exec` (ADR-0146), and a child whose write fails never reaches `exec`.
- **Read the VMM's membership at boot and move stragglers into the scope.** Rejected: a repair path for a state the design can make unreachable, and
  it cannot find a VMM that has already exited its way past the reader.

## Consequences

- The port contract holds for either mechanism, so a falsified premise of the write form switches the adapter to `CLONE_INTO_CGROUP` without a change
  to what DISTILL pins; no spike is needed first (feature delta, § "Is it contract-shaping validation?").
- `memory.max` and the other limits apply from the VMM's first instruction, because the driver writes them before the spawn.
- `CloudHypervisorVmm` needs the cgroup root, and the driver's `place_pid_in_scope` call, the cgroup manager's method and its test are deleted.
- The audited launch hook gains one step, before the descriptor close, `no_new_privs` and the seccomp load.
