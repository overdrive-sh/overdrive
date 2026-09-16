# Research: Linux cgroups for an explanatory Overdrive blog post

**Date**: 2026-09-14 | **Researcher**: nw-researcher (Nova; Terra routing) | **Confidence**: High, with a cleanup-completion caveat | **Sources**: 10

## Executive Summary

A Linux control group (cgroup) is a kernel mechanism for arranging host
processes in a hierarchy and applying resource-controller policy to that
hierarchy. In cgroup v2 there is one unified hierarchy. Every host process is a
member of exactly one cgroup in it; writing a PID to `cgroup.procs` moves that
host process. Directories in cgroupfs represent groups, while files such as
`cpu.weight`, `memory.max`, and `cgroup.kill` are the control surface. [Linux
kernel cgroup v2 documentation](https://docs.kernel.org/7.1/admin-guide/cgroup-v2.html)

The useful mental model is **a named set of host processes plus a resource budget
and lifecycle handle**. `cpu.weight` is a relative share among active sibling
groups, not a dedicated CPU reservation or a hard ceiling. `memory.max` is the
memory controller's hard-limit mechanism, but the kernel documents temporary
overage and cgroup-local OOM behavior when it cannot reclaim enough memory.
`cgroup.kill` is a forceful tree kill (`SIGKILL`), not graceful shutdown.

Overdrive's VM path gives each allocation a cgroup-v2 scope at
`overdrive.slice/workloads.slice/<alloc_id>.scope`. `VmDriver` creates it,
passes the CPU-milli input to the cgroup manager for relative-weight derivation,
derives a reserve-padded cgroup memory maximum from the guest-memory input, and
writes `cpu.weight` and `memory.max`. It supplies the same scope to Cloud
Hypervisor's `VmConfig` and places the started Cloud Hypervisor host PID in
`cgroup.procs`. The guest runs its own kernel: guest process PIDs are not host
cgroup members. Scope-creation and VMM-PID-placement errors reject VM start;
resource-file write failures log a warning and continue. VM teardown performs
VM-specific termination and calls cgroup cleanup, but does not prove that scope
removal completed. Cgroup-BPF remains a general host-process policy selector;
this research does not use it to explain VM guest networking.

## Research Methodology

**Search strategy**: The Linux model comes from the kernel cgroup-v2
documentation, Linux man-pages, systemd resource-control documentation, and
kernel BPF documentation. The product example comes from the current
working-tree production composition and VM-driver source at commit
`46a602f08bf35f18e55e2b344939d64063f45a3a`.

**Verification**: Major Linux behavior has an independent source where one
exists. Kernel ABI details retain an explicit primary-source exception.
Overdrive behavior is a primary-source exception: the current source is the
authority for the current production path.

**Source selection**: Sources are official kernel documentation, Linux
man-pages, upstream systemd documentation, and first-party source. Unverified
blog sources were excluded. Average source reputation is 0.90/1.00.

## Findings

### Finding 1: A cgroup groups host processes for resource and lifecycle control

**Evidence**: The kernel defines cgroups as a hierarchical way to organize
processes and distribute system resources. In v2, every process belongs to one
cgroup; child processes inherit their parent's membership and a process can later
move. [Control Group v2](https://docs.kernel.org/7.1/admin-guide/cgroup-v2.html)

**Verification**: [cgroups(7)](https://man7.org/linux/man-pages/man7/cgroups.7.html)
describes hierarchical process groups with monitored and limited resource use.
[systemd.resource-control(5)](https://man7.org/linux/man-pages/man5/systemd.resource-control.5.html)
describes the same hierarchical groups for resource management.

**Claim boundary**: Describe a cgroup as a Linux kernel grouping primitive, not
as a container or VM. Containers and VMs can combine cgroups with other
mechanisms.

### Finding 2: The cgroup-v2 tree selects the policy for a host process

**Evidence**: Cgroup v2 has one hierarchy. A cgroup directory exposes a
read/write `cgroup.procs` file; writing a PID migrates that process. A parent
enables controllers through `cgroup.subtree_control`, making controller files
available in children. [Control Group v2](https://docs.kernel.org/7.1/admin-guide/cgroup-v2.html)

**Verification**: [cgroups(7)](https://man7.org/linux/man-pages/man7/cgroups.7.html)
independently documents PID writes to `cgroup.procs`, one membership per
hierarchy, and controller-file creation in children.

**Claim boundary**: Writing a PID does not start a process or create a sandbox.
It selects the resource and lifecycle policy that applies to that host process.
An Overdrive VM scope contains the Cloud Hypervisor host PID, not guest process
PIDs.

### Finding 3: CPU weight and memory maximum control different things

**Evidence**: `cpu.weight` ranges from 1 through 10,000 (default 100) and
divides a parent's CPU time among active child cgroups in proportion to their
weights. The model is work-conserving. `memory.max` is a read/write hard memory
limit; if use reaches it and cannot be reduced, the OOM killer runs in that
cgroup, although use can temporarily exceed the value in some circumstances.
[Control Group v2](https://docs.kernel.org/7.1/admin-guide/cgroup-v2.html)

**Verification**: [cgroups(7)](https://man7.org/linux/man-pages/man7/cgroups.7.html)
documents CPU and memory controllers. [systemd.resource-control(5)](https://man7.org/linux/man-pages/man5/systemd.resource-control.5.html)
maps `MemoryMax=` to `memory.max` and describes cgroup-local OOM.

**Claim boundary**: Call `cpu.weight` a relative CPU share under sibling
contention, never a core reservation, quota, or guaranteed percentage. Call
`memory.max` a hard-limit mechanism with reclaim and cgroup-local OOM
consequences; do not promise no temporary overage or a deterministic OOM victim.

### Finding 4: `cgroup.kill` forcefully kills a cgroup tree

**Evidence**: `cgroup.kill` is a write-only v2 file accepting `1`; writing it
kills processes in the target cgroup and all descendants with `SIGKILL`.
[Control Group v2](https://docs.kernel.org/7.1/admin-guide/cgroup-v2.html)

**Verification**: `VmDriver` cleanup calls `CgroupManager::cgroup_kill` and
`remove_workload_scope` after VM-specific termination. [VM cleanup source](https://github.com/overdrive-sh/overdrive/blob/46a602f08bf35f18e55e2b344939d64063f45a3a/crates/overdrive-worker/src/vm_driver.rs#L1424-L1432),
[VM stop source](https://github.com/overdrive-sh/overdrive/blob/46a602f08bf35f18e55e2b344939d64063f45a3a/crates/overdrive-worker/src/vm_driver.rs#L1770-L1820)

**Claim boundary**: `cgroup.kill` is not graceful shutdown. For Overdrive VMs,
say that teardown terminates the VM and calls cgroup cleanup; do not claim that
the scope is absent when stop returns.

### Finding 5: Cgroup-BPF can select host socket policy, not a private network

**Evidence**: Kernel documentation and its source documentation map
`BPF_PROG_TYPE_CGROUP_SOCK_ADDR` to the cgroup socket-address hooks
`cgroup/connect4`, `cgroup/sendmsg4`, and `cgroup/recvmsg4`.
[Program Types and ELF Sections](https://docs.kernel.org/7.0/bpf/libbpf/program_types.html),
[BPF_PROG_TYPE_CGROUP_SOCK_ADDR mappings](https://github.com/torvalds/linux/blob/master/Documentation/bpf/libbpf/program_types.rst)

**Verification**: [cgroups(7)](https://man7.org/linux/man-pages/man7/cgroups.7.html)
documents cgroup-based network tagging and cgroup-v2 eBPF attachment.

**Claim boundary**: Cgroup membership can select BPF policy for a host
process's socket operations. It does not supply a private interface, routing
table, firewall, port space, or protocol stack. Do not present cgroup-BPF as the
networking model for an Overdrive VM guest.

### Finding 6: Namespaces, identity, and virtual machines are separate mechanisms

**Evidence**: A namespace isolates a process's view of a global resource. A
network namespace isolates devices, IP stacks, routes, firewall rules, and port
numbers. A user namespace isolates user and group IDs and capabilities.
[namespaces(7)](https://man7.org/linux/man-pages/man7/namespaces.7.html),
[network_namespaces(7)](https://man7.org/linux/man-pages/man7/network_namespaces.7.html),
[user_namespaces(7)](https://man7.org/linux/man-pages/man7/user_namespaces.7.html)

**Verification**: The [KVM API](https://docs.kernel.org/virt/kvm/api.html)
describes a separate virtual-machine facility.

**Claim boundary**: Cgroups answer “which host processes share this budget and
lifecycle control?” Namespaces answer “which view of a resource can those
processes see?” User namespaces and normal user/group permission mechanisms
govern identity, capabilities, and authorization. KVM provides VM execution.
Do not say that a cgroup creates a network namespace, a user/group identity
boundary, or a guest operating system.

### Finding 7: Overdrive applies cgroups to the Cloud Hypervisor host process

**Evidence**: The control plane runs cgroup preflight and workload-slice
controller bootstrap before composing its driver registry. It then discovers
and probes Cloud Hypervisor before inserting `VmDriver`. If Cloud Hypervisor is
absent, the registry has no `Vm` entry and `[vm]` deployment is rejected at
admission. [Production composition](https://github.com/overdrive-sh/overdrive/blob/46a602f08bf35f18e55e2b344939d64063f45a3a/crates/overdrive-control-plane/src/lib.rs#L1670-L1745)

`VmDriver` derives `CgroupPath::for_alloc`, creates the allocation scope, and
derives `MemoryPlan` from `[resources].memory_bytes`. It passes
`[resources].cpu_milli` to `CgroupManager`, which derives
`cpu.weight = clamp(cpu_milli / 10, 1, 10000)`, and it writes the reserve-padded
`MemoryPlan::cgroup_max_bytes()` as `memory.max`; a write failure logs a warning
and continues. It puts the same scope in `VmConfig`, starts the VMM, then writes
the Cloud Hypervisor PID to `cgroup.procs`. [Scope and resource setup](https://github.com/overdrive-sh/overdrive/blob/46a602f08bf35f18e55e2b344939d64063f45a3a/crates/overdrive-worker/src/vm_driver.rs#L1186-L1280),
[CPU-weight derivation and writes](https://github.com/overdrive-sh/overdrive/blob/46a602f08bf35f18e55e2b344939d64063f45a3a/crates/overdrive-worker/src/cgroup_manager.rs#L153-L158),
[VMM configuration](https://github.com/overdrive-sh/overdrive/blob/46a602f08bf35f18e55e2b344939d64063f45a3a/crates/overdrive-worker/src/vm_driver.rs#L1311-L1327),
[VMM PID placement](https://github.com/overdrive-sh/overdrive/blob/46a602f08bf35f18e55e2b344939d64063f45a3a/crates/overdrive-worker/src/vm_driver.rs#L1480-L1525)

**Verification**: [The public microVM concept](/docs/concepts/microvms)
states that a `[vm]` workload runs under Cloud Hypervisor with its own kernel.
The source establishes the host-side process placement; the public concept
explains why guest process PIDs must not be described as host cgroup members.

**Claim boundary**: `[resources]` supplies a CPU-milli input and a guest-memory
input. `CgroupManager` derives the relative CPU weight; it is not numerically
equal to `cpu_milli`. The VM driver derives a reserve-padded cgroup maximum, so
the blog must not say it writes declared guest RAM directly. Scope creation and
VMM-PID placement failures reject VM start. Accepted VM start does not prove
that a requested resource-file write succeeded.

### Finding 8: VM cleanup calls do not prove scope absence

**Evidence**: `VmDriver::stop` performs its VM-specific termination path, then
calls `cleanup_driver_artifacts`. That helper calls `cgroup_kill`,
`remove_workload_scope`, run-directory removal, and rootfs cleanup while
discarding the individual cleanup results. Its source comment states that
completion means the calls returned, not that each artifact's absence was
verified. [VM cleanup source](https://github.com/overdrive-sh/overdrive/blob/46a602f08bf35f18e55e2b344939d64063f45a3a/crates/overdrive-worker/src/vm_driver.rs#L1424-L1432),
[VM stop source](https://github.com/overdrive-sh/overdrive/blob/46a602f08bf35f18e55e2b344939d64063f45a3a/crates/overdrive-worker/src/vm_driver.rs#L1770-L1820)

**Claim boundary**: Use “calls cgroup cleanup” or “attempts cgroup cleanup,”
not “waits for the scope to be removed” or “guarantees the scope is gone.”

## Recommended Blog Narrative and Claim Boundaries

1. Start with host-process grouping, not containers: a cgroup is a kernel-level
   resource and lifecycle boundary.
2. Show the v2 tree and explain that a PID write selects the policy for a host
   process.
3. Distinguish `cpu.weight`, `memory.max`, and `cgroup.kill` without promising
   reserved CPU, exact-memory immutability, or graceful shutdown.
4. Explain cgroup-BPF only as host-process socket-policy selection, not VM guest
   networking.
5. State the separate roles of namespaces, user/group identity, and KVM.
6. Use `VmDriver` as the concrete example: its allocation scope contains Cloud
   Hypervisor's host PID; `[resources]` supplies CPU-milli and guest-memory
   inputs; `CgroupManager` derives `cpu.weight`, and `memory.max` receives the
   derived reserve-padded value.

### Statements the blog should avoid

- “A CPU weight reserves a CPU/core” or “guarantees CPU percentage.”
- “`memory.max` makes an allocation unable ever to exceed that byte value” or
  “selects a deterministic OOM victim.”
- “Cgroups are containers/VMs” or “a cgroup creates networking isolation.”
- “Cloud Hypervisor's cgroup contains guest process PIDs.”
- “Overdrive writes `cpu_milli` unchanged to `cpu.weight`.”
- “Overdrive writes the declared guest RAM directly to `memory.max`.”
- “Successful Overdrive VM start proves resource limits are applied.”
- “Overdrive VM stop guarantees the scope has already been removed.”
- “Cgroup-BPF describes how Overdrive VM guest networking works.”

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified | Verification status |
|---|---|---:|---|---|---|---|
| [Control Group v2](https://docs.kernel.org/7.1/admin-guide/cgroup-v2.html) | docs.kernel.org | 1.0 High | Official kernel documentation | 2026-09-14 | Yes | Primary for v2 hierarchy, controllers, memory, kill, and cgroup namespaces |
| [Program Types and ELF Sections](https://docs.kernel.org/7.0/bpf/libbpf/program_types.html) | docs.kernel.org | 1.0 High | Official kernel documentation | 2026-09-14 | Yes | Primary for cgroup-BPF attach types |
| [BPF_PROG_TYPE_CGROUP_SOCK_ADDR mappings](https://github.com/torvalds/linux/blob/master/Documentation/bpf/libbpf/program_types.rst) | github.com/torvalds/linux | 1.0 High | Linux kernel source documentation | 2026-09-14 | Yes | Primary for cgroup socket-address hook mappings |
| [KVM API](https://docs.kernel.org/virt/kvm/api.html) | docs.kernel.org | 1.0 High | Official kernel documentation | 2026-09-14 | Yes | Primary for the separate VM facility |
| [cgroups(7)](https://man7.org/linux/man-pages/man7/cgroups.7.html) | man7.org | 0.8 Medium-High | Linux man-pages project | 2026-09-14 | Yes | Independent cgroup cross-check |
| [network_namespaces(7)](https://man7.org/linux/man-pages/man7/network_namespaces.7.html) | man7.org | 0.8 Medium-High | Linux man-pages project | 2026-09-14 | Yes | Network-namespace boundary cross-check |
| [user_namespaces(7)](https://man7.org/linux/man-pages/man7/user_namespaces.7.html) | man7.org | 0.8 Medium-High | Linux man-pages project | 2026-09-14 | Yes | User/group-ID and capability boundary cross-check |
| [namespaces(7)](https://man7.org/linux/man-pages/man7/namespaces.7.html) | man7.org | 0.8 Medium-High | Linux man-pages project | 2026-09-14 | Yes | General namespace model cross-check |
| [systemd.resource-control(5)](https://man7.org/linux/man-pages/man5/systemd.resource-control.5.html) | man7.org | 0.8 Medium-High | Upstream systemd manual mirrored by Linux man-pages | 2026-09-14 | Yes | Resource-controller cross-check |
| [Overdrive source at audited commit](https://github.com/overdrive-sh/overdrive/tree/46a602f08bf35f18e55e2b344939d64063f45a3a) | github.com | 1.0 High | First-party source | 2026-09-14 | Internally | Sole authority for production composition and `VmDriver` behavior |

Reputation: High: 5 (50%) | Medium-High: 5 (50%) | Average: 0.90/1.00.

## Knowledge Gaps

### Gap 1: Completed scope removal after VM stop

**Issue**: The source proves that VM-specific termination and cleanup calls
occur in order. It does not establish an operator-visible guarantee that the
cgroup directory is absent when `VmDriver::stop` returns. **Recommendation**:
keep that claim out of the blog.

### Gap 2: Universal controller availability

**Issue**: Production bootstraps the controller hierarchy and `VmDriver`
attempts the resource writes, but this research did not run every target-host
configuration. **Recommendation**: retain warn-and-continue language; do not
claim universal enforcement for every accepted VM.

### Gap 3: VM guest networking

**Issue**: Kernel documentation supports multiple cgroup-BPF hooks, but it does
not make host cgroup membership an explanation of a VM guest's networking.
**Recommendation**: keep the cgroup-BPF paragraph general and separate from the
VM example.

## Review History

- The general Linux research completed two approved review iterations.
- This correction replaces the stale product-path claims with the current
  `VmDriver` source chain. The Linux findings remain unchanged in substance; the
  product example, cleanup caveat, and claim boundaries now require independent
  re-review.

## Full Citations

1. Linux Kernel Documentation. “Control Group v2.” https://docs.kernel.org/7.1/admin-guide/cgroup-v2.html. Accessed 2026-09-14.
2. Linux Kernel Documentation. “Program Types and ELF Sections.” https://docs.kernel.org/7.0/bpf/libbpf/program_types.html. Accessed 2026-09-14.
3. Linux kernel source documentation. “BPF_PROG_TYPE_CGROUP_SOCK_ADDR mappings.” https://github.com/torvalds/linux/blob/master/Documentation/bpf/libbpf/program_types.rst. Accessed 2026-09-14.
4. Linux Kernel Documentation. “The Definitive KVM API Documentation.” https://docs.kernel.org/virt/kvm/api.html. Accessed 2026-09-14.
5. Linux man-pages project. “cgroups(7).” https://man7.org/linux/man-pages/man7/cgroups.7.html. Accessed 2026-09-14.
6. Linux man-pages project. “network_namespaces(7).” https://man7.org/linux/man-pages/man7/network_namespaces.7.html. Accessed 2026-09-14.
7. Linux man-pages project. “user_namespaces(7).” https://man7.org/linux/man-pages/man7/user_namespaces.7.html. Accessed 2026-09-14.
8. Linux man-pages project. “namespaces(7).” https://man7.org/linux/man-pages/man7/namespaces.7.html. Accessed 2026-09-14.
9. systemd. “systemd.resource-control(5).” https://man7.org/linux/man-pages/man5/systemd.resource-control.5.html. Accessed 2026-09-14.
10. Overdrive contributors. “Current source audit, commit `46a602f08bf35f18e55e2b344939d64063f45a3a`.” https://github.com/overdrive-sh/overdrive/tree/46a602f08bf35f18e55e2b344939d64063f45a3a. Accessed 2026-09-14.

## Research Metadata

Examined: 10 cited sources plus current production composition, `VmDriver`,
`MemoryPlan`, and the public microVM concept. Confidence: High for Linux and VM
start-path facts; medium for the intentionally limited scope-removal completion
claim. Output: `docs/research/cgroups-for-explanation-doc.md`.
