# Research: Exec Health Probes for Cloud Hypervisor VM Services in Overdrive

**Date**: 2026-09-06
**Researcher**: nw-researcher (Nova)
**Status**: Complete
**Intended use**: Evidence for GH #257 DISCUSS/DESIGN
**Confidence**: High on current implementation and the mechanism choice; medium
on the precise cancellation/concurrency contract pending a metal spike
**Sources**: 17 primary/official external sources plus repository code, ADRs,
issues, and executed spikes

## Executive Summary

Exec probes inside an Overdrive Cloud Hypervisor guest are possible. They are
not possible through the **currently implemented host-only `CgroupExecProber`**,
because a host process cannot be turned into a process under the guest kernel
by writing its PID to a host cgroup. That distinction corrects the premise in
GH #257 without pretending the missing mechanism is already built.

Overdrive already has most of the substrate: a per-allocation vsock device, a
guest-initiated and metal-proven host connection, a platform-owned guest PID 1
that already executes the workload command, a routed guest address delivered
by GH #222, and a role-aware ProbeRunner. The missing part is a persistent
in-guest command supervisor and a driver-aware route from a probe tick to it.

The best fit is to evolve `overdrive-init` into that persistent supervisor and
add a **small, dedicated probe RPC channel on a separate vsock port**, initiated
by the guest and backed by another per-allocation Unix listener. Do not turn
the current one-shot Beacon lifecycle language into an ad-hoc multiplexed RPC,
and do not add QEMU Guest Agent, SSH, or Kata's full container-management agent.
The initial RPC should execute the already-declared argv directly (no implicit
shell), return a correlated termination result, carry an end-to-end deadline,
kill and reap only the guest probe process tree on timeout, bound requests and
responses, and never replay an ambiguous request after connection loss.

HTTP and TCP probes should stay host-originated and target the VM's existing
guest `workload_addr` over the routed TAP. They do not require an agent. The
Cloud Hypervisor management API itself has no guest-process execution endpoint;
every credible general VM exec precedent uses an in-guest component.

This is a recommendation for DESIGN, not a public API prescription. The exact
Rust trait, type, enum, message framing, protocol version, and lifecycle-owner
signatures are not accepted yet and must not be invented during DELIVER.

## Scope and Research Questions

This document asks how Overdrive should treat Exec health probes when a Service workload runs in a Cloud Hypervisor VM. It distinguishes four evidence classes throughout: **observed current implementation**, **accepted design**, **executed spike evidence**, and **unimplemented possibility**. An absent mechanism is not treated as proof that a mechanism is impossible.

Questions:

1. What path do VM workloads, guest boot, vsock beacons, service probes, cgroups, `overdrive serve`, and `overdrive deploy` take today?
2. What semantics must an Exec probe preserve?
3. Which established guest-control mechanisms can actually execute a process inside a guest, and with what security, lifecycle, output, concurrency, and failure model?
4. Which option best fits Overdrive: reject Exec probes, extend the current guest component, add a dedicated minimal probe RPC, or use a credible agentless/network alternative?
5. What must DESIGN decide, and what must be proven in a bounded spike first?

## Research Methodology

**Ordering**: local production code, accepted ADRs/artifacts, spike evidence, and GitHub issues (with comments) are inspected and written first; external web research follows.
**Search strategy**: targeted symbol/caller-path inspection with `rg`; issue retrieval with `gh issue view --comments`; then official kernel, Cloud Hypervisor, Kubernetes, KubeVirt, Kata Containers, QEMU, and Firecracker documentation/source.
**Source selection**: primary project source and artifacts for Overdrive; official documentation/source and standards for external claims; peer-reviewed systems work where relevant.
**Quality standard**: one authoritative source is sufficient for a narrow protocol/API fact; consequential architectural claims are cross-referenced where independent sources exist. Each external source records URL, domain, access date, reputation score, and verification status.

## Part I — Overdrive Implementation and Architecture Baseline

This baseline was completed before the external mechanism search. “Today” below
means the production tree inspected on 2026-09-06, not a claim about what Cloud
Hypervisor or a future Overdrive guest component can support.

### 1. Submission and admission path

`overdrive deploy` reads TOML into the core workload parser and projects the
parsed model into the submit wire type. A Job projection handles either an
`[exec]` or `[vm]` driver, but both Service projection paths currently assume
`[exec]`. This is reinforced at two earlier boundaries:

- `SectionPresence::validated` rejects `[service]` plus `[vm]` as
  `VmNotAllowed` before driver construction.
- `ServiceV2::from_submit` independently rejects a wire-level VM input, so a
  caller cannot bypass the TOML parser.

The rejection is therefore an implemented admission rule, not a consequence
of an attempted VM probe failing. It also rejects the entire VM Service, rather
than selecting behavior per probe mechanic. Some diagnostic prose still says
the tap/guest network is missing; that premise is stale after GH #222.

`overdrive serve` composes a driver registry containing the Exec driver and,
when Cloud Hypervisor discovery and probing succeeds, the VM driver. It also
constructs one node-wide `ProbeRunner`, but threads that runner only into
`ExecDriver::with_probe_runner`. `VmDriver` has no corresponding probe-runner
lifecycle hook today. The accepted driver-registry design deliberately left VM
Services deferred to GH #257; it did not prove they were impossible.

### 2. VM boot and control path

`VmDriver::provision_vmm` derives the run directory and workload cgroup, binds
the per-allocation Unix listener used as the vsock backend, and constructs a
`VmConfig`. `CloudHypervisorVmm` starts Cloud Hypervisor with:

- `--vsock cid=3,socket=<allocation run-dir socket>`;
- `--net tap=<platform tap>,mac=<derived MAC>` for networked allocations; and
- `ip netns exec <allocation netns>` before the existing confinement wrapper.

The Unix API socket, vsock socket, and console log remain per-allocation
filesystem objects. The VMM host process is put into the allocation cgroup;
that cgroup does not place a later process inside the guest's PID, mount, or
network namespaces.

The guest's PID 1 is `overdrive-init`. Its implemented lifecycle is intentionally
single-shot:

1. initialize modules and static guest networking;
2. initiate an `AF_VSOCK` connection to host CID 2, port 1234;
3. send `READY`;
4. receive exactly one `EXEC {argv}`;
5. run that operator command synchronously with `std::process::Command::status`;
6. send `EXIT <code>`;
7. read `SHUTDOWN` or EOF and power off.

This is direct evidence that Overdrive already executes one host-selected
command *inside* the VM. It is equally direct evidence that the component is
not presently a probe agent: while a long-running Service command is alive,
PID 1 is blocked in `status()` and does not read, schedule, identify, cancel, or
report additional command executions.

The published Beacon language is a lifecycle language, not an RPC protocol. It
has `READY`, one JSON-encoded `EXEC` argv, `EXIT`, and `SHUTDOWN`; it has no
request identifier, repeated-command state, per-command result, output frame,
deadline, cancellation, concurrency, or version negotiation. `VmDriver`
accepts the guest-initiated Unix stream, waits for `READY`, retains a serialized
write half, and defers the one workload `EXEC` until transparent-mTLS install
has completed. Its read side later expects the one workload `EXIT`.

Consequently, “the host cannot fork/exec inside a VM” is imprecise. The host
kernel cannot make a normal host `fork`/`exec` become a process in a separate
guest kernel. But the host can request an in-guest component to create a guest
process, and Overdrive already does so once for the workload command. The open
question is which persistent, bounded guest-control mechanism should service
probe requests.

### 3. Network path already delivered by GH #222

Accepted ADR-0088/0089 and the delivered GH #222 path give each VM allocation a
deterministic routed guest `/30`, platform TAP, guest MAC, host return route,
and VMM placement in the workload netns. `workload_addr` for a VM is the guest
address. Guest network initialization completes before `READY`; host intercept
installation completes before the deferred workload `EXEC` is flushed.

This removes the old “no guest network” premise. It does not automatically make
current probes target the guest. `ProbeRunner::probe_tick` converts an omitted
or wildcard TCP/HTTP host to host `127.0.0.1` and otherwise dials the descriptor
host from the host process. Its input is allocation ID plus probe descriptors;
it receives no per-allocation guest target. GH #257 therefore needs a design
for projecting VM probe targets to the already-recorded guest
`workload_addr`, plus the thin native-metal proof that a host-originated SYN
reaches the guest. No new TAP topology is indicated by the current evidence.

### 4. Existing probe semantics and lifecycle

The accepted ProbeRunner design creates one task per `(allocation, role,
probe-index)`. Startup, readiness, and liveness roles have independent
supervision tokens. Each tick applies the descriptor's initial delay, interval,
and timeout, invokes one of three adapters, and writes an LWW `ProbeResultRow`.
Allocation termination cancels all roles; becoming Stable cancels only startup.

Current adapter behavior is:

| Mechanic | Execution location | Success | Timeout/output behavior |
|---|---|---|---|
| TCP | Host process network context | TCP connect plus handshake completes | Bounded by probe timeout |
| HTTP | Host process network context | HTTP GET returns 2xx; 3xx is failure | Response body discarded; bounded by timeout |
| Exec | New host child | Process exit status is success | stdout/stderr discarded; on timeout the adapter invokes `cgroup.kill`, then kills/reaps the child |

The current `ExecProber` contract takes argv, an absolute workload cgroup path,
and a timeout. `CgroupExecProber` spawns a **host** process, then writes its PID
to that host cgroup. Cgroup membership provides accounting and kill scope; it
does not cross into a guest kernel. For a VM, the allocation cgroup contains
the VMM host process, so reusing the current timeout path would risk killing the
VMM through `cgroup.kill`. This code is currently unreachable for VM Services
because admission rejects them.

Cancellation also needs precision in any future design. A supervisor token is
cooperative. If a probe call is already in progress, the current loop does not
select cancellation concurrently with that call; the adapter's timeout bounds
it. A guest RPC therefore needs an end-to-end deadline and a guest-side process
termination/reap contract, rather than assuming that dropping the host future
stops a guest process.

### 5. Accepted design and executed spike evidence

- ADR-0054 fixes the ProbeRunner's per-allocation/per-role ownership and LWW
  result model for the implemented Exec-Service path.
- ADR-0059 fixes the current host Exec-probe cgroup placement and timeout-kill
  behavior. Its statement that entering the cgroup makes a process see the
  workload's mount/network namespace is not supported by the implementation:
  no namespace entry occurs. This is recorded as a conflict for DESIGN, not
  silently generalized to VMs.
- ADR-0082 gives `Vmm` responsibility for the hypervisor process only. Guest
  graceful control remains with `VmDriver` over Beacon. The trait exposes
  probe/create/terminate, not guest command execution.
- ADR-0083 establishes the driver registry and per-driver payload, explicitly
  retaining VM-Service rejection pending GH #257.
- ADR-0088/0089 establish the current routed TAP topology and the guest address
  carried as `workload_addr`.
- The Cloud Hypervisor v53.0 P2 metal spike demonstrated guest-to-host Beacon
  `READY`/`EXIT` through the Unix-backed vsock path. The P12 snapshot spike
  demonstrated that an established vsock connection cannot be treated as
  durable across restore and that successful guest send is not proof of host
  receipt; fresh reconnect recovered. Host-initiated fresh connection behavior
  was not qualified by those spikes.

### 6. Issue state and corrected readiness statement

GH #42 delivered the VM Job driver, GH #222 delivered guest networking and
transparent-intercept ordering, and GH #100 tracks a broader future guest
agent. GH #257 now owns VM Service admission, guest-targeted TCP/HTTP probes,
and the Exec-probe mechanism decision.

The accurate readiness statement is:

> Requirements identify the desired VM-Service outcome, but DESIGN has not yet
> selected or specified the in-guest Exec-probe control mechanism. The current
> host-only Exec adapter cannot provide guest execution; that is a limitation
> of the implemented adapter, not a proof that in-guest Exec probes are
> technically impossible.

## Part II — External Mechanisms and Prior Art

### 7. What “Exec probe” normally means

Kubernetes defines an Exec probe as a specified command executed inside the
container, successful only on exit status zero. It warns that every tick forks
processes and that poor timeout handling can accumulate processes and exhaust
resources. Its kubelet path passes the argv and timeout to the runtime's
`ExecSync`; TCP probes, in contrast, originate from the node and default to the
Pod IP. These are useful semantic precedents, not a requirement to import CRI.
([probe semantics](https://kubernetes.io/docs/concepts/workloads/pods/probes/),
[kubelet routing](https://github.com/kubernetes/kubernetes/blob/b2ec8b6fefac451a2dedafc4dd71f2f16c7a6abe/pkg/kubelet/prober/prober.go))

CRI's `ExecSyncRequest` carries container identity, an argv vector, and a
timeout “to stop the command.” The response carries stdout, stderr, and exit
code, and explicitly caps each output at 16 MiB because of prior security
failures. Overdrive does not need those streams for its boolean `ProbeOutcome`,
but the cap is strong evidence that an unbounded response is unsafe.
([CRI protocol](https://github.com/kubernetes/cri-api/blob/b47fc37312c54caef1a68c8b4ac8e07158331a82/pkg/apis/runtime/v1/api.proto#L1616-L1639))

Two semantic rules follow for this project:

1. argv is direct process execution; shell behavior exists only when the user
   explicitly supplies a shell in argv; and
2. a timeout is not merely “the caller stopped waiting.” It must terminate and
   reap the probe process or the platform creates the exact leak Kubernetes
   warns about.

**Confidence: high.** Kubernetes documentation, kubelet source, and the CRI
protocol independently expose the user semantic, caller path, and runtime
contract.

### 8. Cloud Hypervisor and virtio-vsock

Cloud Hypervisor supports stream vsock and exposes it through a Unix socket.
For a host-initiated connection, the host connects to the base Unix socket,
sends `CONNECT <guest-port>\n`, and then communicates after establishment. For
a guest-initiated connection, the host listens on `<base>_<port>` and the guest
connects to host CID 2. The latter exactly matches Overdrive's existing Beacon
direction. ([Cloud Hypervisor vsock](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/45972e6afc184ba8769c2023870874380288d9c9/docs/vsock.md))

The underlying Firecracker design documents the full host handshake, including
the required `OK <assigned-host-port>\n` acknowledgement, and the one-to-one
mapping between guest ports and host Unix sockets. Cloud Hypervisor states its
device is based on that implementation. This makes a separate guest-initiated
port a natural, low-risk extension, while a fresh host-initiated connection
needs an explicit Overdrive metal qualification before reliance.
([Firecracker vsock design](https://github.com/firecracker-microvm/firecracker/blob/7699746649826d1dfcdde626b3131bac08f28e0d/docs/vsock.md))

Cloud Hypervisor's documented management API manages the VM and its devices
(create, boot, resize, add/remove devices, shutdown, migration, and related
operations). It exposes no guest-process exec operation. The conclusion that
the CH API cannot itself implement an Exec probe is an inference from that
enumerated API, corroborated by CH's separate vsock documentation directing
guest communication through guest software.
([Cloud Hypervisor API](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/45972e6afc184ba8769c2023870874380288d9c9/docs/api.md))

**Confidence: high** for the transport mechanics (CH and its Firecracker
upstream agree); **medium-high** for choosing guest initiation, because the
second concurrent channel remains to be metal-qualified in Overdrive.

### 9. KubeVirt and QEMU Guest Agent

KubeVirt is the closest VM-specific precedent. Its Exec readiness/liveness
probe explicitly requires QEMU Guest Agent inside the VM; `virt-probe` forwards
the command and evaluates in-guest execution. If the agent is absent or not
ready, the probe fails. KubeVirt also offers `guest-ping`, but only one of ping
or exec can be selected. ([KubeVirt probe guide](https://kubevirt.io/user-guide/user_workloads/liveness_and_readiness_probes/))

QEMU Guest Agent's `guest-exec` returns a guest PID. `guest-exec-status` polls
that PID, reports normal exit code versus signal, optionally returns bounded/
truncated captured output after exit, and reaps the associated metadata. The
daemon has allow- and block-RPC lists because its broader surface includes
filesystem, credentials, shutdown, and system-management operations.
([QGA protocol](https://www.qemu.org/docs/master/interop/qemu-ga-ref.html),
[QGA daemon controls](https://www.qemu.org/docs/master/interop/qemu-ga.html))

This validates “in-guest agent + correlated process result” as an established
solution, but also supplies a counterexample. KubeVirt's current `GuestExec`
implementation times out its polling loop without signaling the guest PID; the
guest process may continue running. Overdrive should copy the architecture,
not that lifecycle defect.
([KubeVirt `GuestExec`](https://github.com/kubevirt/kubevirt/blob/007c02cac10a6bf5ead0313c7d048b8d1358279a/pkg/virt-launcher/virtwrap/agent/exec.go))

Installing QGA itself is a poor Overdrive fit. It adds a general-purpose daemon
and much broader privileged RPC surface merely to obtain one narrow operation,
while Overdrive already controls the guest image and has a Rust PID 1 that can
own the operation.

**Confidence: high** that KubeVirt/QGA provides an in-guest VM Exec precedent;
**medium-high** on the comparative fit judgment, which is project synthesis.

### 10. Kata Containers and Firecracker-containerd

Kata's architecture uses one long-running Rust guest agent as the supervisor
for workload containers. The host runtime communicates with it using ttRPC over
vsock; the same protocol carries process-management commands and standard I/O.
Its protocol separates `ExecProcess`, `WaitProcess` (wait and reap),
`SignalProcess`, and bounded stream operations, keyed by container and exec
identifiers. This is strong precedent for explicit identity, wait/reap, signal,
and stream lifecycle rather than a single uncorrelated line message.
([Kata architecture](https://github.com/kata-containers/kata-containers/blob/3e30c0afef54713528b3b275fe5f1561618a4292/docs/design/architecture/README.md),
[Kata agent protocol](https://github.com/kata-containers/kata-containers/blob/3e30c0afef54713528b3b275fe5f1561618a4292/src/libs/protocols/protos/agent.proto))

Kata also treats the guest workload as potentially malicious and vsock as the
control/stdio medium. Its threat model calls out that, for Firecracker and
Cloud Hypervisor, the host backend is a Unix-domain socket in VMM userspace.
That means Unix-file isolation and parser hardening are part of the boundary;
vsock is a transport, not an authorization policy.
([Kata threat model](https://github.com/kata-containers/documentation/blob/master/design/threat-model/threat-model.md))

Firecracker-containerd independently uses an in-guest agent over vsock to
create container processes. Firecracker's VMM API, like Cloud Hypervisor's,
does not make a host `exec` become a guest process.
([Firecracker-containerd agent](https://github.com/firecracker-microvm/firecracker-containerd/blob/main/docs/agent.md))

Kata's full protocol and OCI container supervisor are far larger than GH #257
needs. The useful lesson is the process lifecycle shape, not the dependency or
API size.

**Confidence: high** on Kata's agent/process lifecycle and vsock architecture;
**medium-high** on using only its lifecycle lessons in Overdrive.

### 11. Linux cgroups and namespaces

The kernel documents cgroups as a hierarchy for organizing, limiting, and
monitoring process resource use. Writing a PID to `cgroup.procs` migrates that
host process into a resource-control group. Namespace membership is distinct;
`setns` explicitly reassociates a thread with one selected namespace, and even
joining a cgroup namespace does not change cgroup membership.
([cgroup v2](https://docs.kernel.org/admin-guide/cgroup-v2.html),
[`setns(2)`](https://man7.org/linux/man-pages/man2/setns.2.html),
[`namespaces(7)`](https://man7.org/linux/man-pages/man7/namespaces.7.html))

This external evidence confirms the local-code reading: the current host
cgroup placement neither enters an Exec workload's mount/network namespaces
nor crosses a VM boundary. For a host container-like workload, namespace entry
would need a separate mechanism. For a VM, only code executing under the guest
kernel can satisfy true in-guest Exec semantics.

**Confidence: high.** Kernel documentation, man-pages, and the observed lack of
`setns` in Overdrive's adapter converge.

## Part III — Project-Specific Option Analysis

### 12. Decision criteria

Options are assessed against: semantic fidelity (runs in guest), fit with
existing components, protocol/lifecycle correctness, host security, guest
resource containment, implementation size, operability, and empirical risk.

| Option | Fidelity | Project fit | Principal problem | Disposition |
|---|---:|---:|---|---|
| Reject VM Exec probes at parse time | None | High, smallest change | Leaves declared Service parity incomplete; forces mechanic-specific limitation | Honest fallback only |
| Reuse current host `CgroupExecProber` | None | None | Runs on host; cgroup timeout could kill VMM | Reject |
| Add probe messages to current Beacon stream | High | Medium | Conflates one-shot lifecycle and concurrent RPC; both ends currently assume one command/result | Reject as default |
| Persistent probe RPC in `overdrive-init`, separate guest-initiated vsock port | High | High | Requires PID 1 supervision, bounded protocol, driver routing, and metal proof | Recommend |
| Add a separate minimal guest-agent daemon | High | Medium | Adds packaging, boot ordering, liveness, privilege, and upgrade owner beside existing PID 1 | Reserve only if PID 1 cannot safely supervise |
| Install QEMU Guest Agent | High | Low | Broad privileged API, QEMU/libvirt-oriented transport/integration, redundant guest component | Reject |
| Import Kata agent/ttRPC | High | Low | Full OCI/container management surface vastly exceeds need | Reject |
| SSH from host | High in principle | Low | Requires guest network, sshd, credentials/rotation, host-key policy, shell/account semantics | Reject |
| Replace Exec with HTTP/TCP/gRPC | Different semantics | High where app supports it | Cannot inspect arbitrary guest-local state or run existing argv | Recommend as operator preference, not equivalence |

### 13. Why a separate channel in the existing component

The lifecycle Beacon and probe RPC have different invariants. Beacon is a
strict boot/one-workload/exit/shutdown state machine whose byte language is
already published and whose ordering protects transparent-mTLS installation.
Probe execution is repeated, correlated, independently timed, potentially
concurrent, and only valid while a Service is running. Versioning Beacon to
carry both would increase the blast radius of every parser and state transition.

A second guest-initiated connection keeps those concerns separate while
reusing the same CH vsock device, run-directory isolation, and proven direction.
It also avoids depending on the unqualified fresh host→guest connection path.
The guest can establish and identify the channel before `READY`; the host can
retain it but must not dispatch service probes until the Running transition.

This does **not** imply a new guest daemon. `overdrive-init` is already the
platform-owned process launcher and PID 1/reaper. Evolving it to supervise the
long-running workload and a narrow control loop avoids two competing owners.
If a spike proves that keeping this logic in PID 1 materially harms reliable
reaping or shutdown, a separate minimal agent becomes a justified DESIGN
alternative—not an assumption.

### 14. Required protocol and process semantics

DESIGN should pin these behavioral requirements before naming public Rust API:

- **Scope:** the host may request only the argv already present in the accepted
  probe descriptor. There is no public interactive exec endpoint.
- **Execution:** direct argv execution in the guest; no implicit shell, stdin,
  working-directory invention, or environment override. Executable lookup
  semantics should match the existing `Command::new(argv[0])` behavior unless
  DESIGN deliberately changes the product contract.
- **Correlation:** every request and terminal response has a connection-scoped
  request ID. Unknown, duplicate, stale-session, and oversized frames fail
  closed.
- **Outcome:** distinguish exit zero, nonzero exit, signal, spawn error,
  deadline, agent unavailable, protocol failure, and overload internally; map
  them to the existing probe result model without expanding public surface
  accidentally.
- **Deadline:** host sends a relative timeout or absolute monotonic deadline;
  guest owns enforcement, terminates the probe's process group/tree only, and
  reaps it before reporting timeout. Host also bounds the RPC wait.
- **Disconnect:** never automatically replay an ambiguously delivered exec.
  Mark that tick unknown/failed according to accepted probe policy and let the
  next scheduled tick create a new request. Health commands should be designed
  idempotently, but the transport must not manufacture duplicates.
- **Output:** send no stdout/stderr in the first version because current
  `ProbeOutcome` discards it. Route child stdio to `/dev/null`. If diagnostics
  later require output, add a separately designed hard byte cap and truncation
  marker; CRI's security history rules out unbounded capture.
- **Concurrency:** permit only a small bounded number of active probes and
  return overload immediately. Probe roles can overlap, so global serialization
  risks making one hung readiness command consume a liveness deadline. The
  exact bound is a spike/DESIGN choice.
- **Lifecycle:** no probe before the Service Running transition; reject new
  work once stop begins; cancel/kill/reap active probes before guest shutdown;
  workload exit closes the session and drives the existing terminal path.
- **Recovery:** a new VM boot is a new session generation. Do not preserve or
  replay outstanding probe RPC across snapshot/restore or reconnect. The local
  P12 spike already disproves treating an established vsock stream as durable.

### 15. Routing HTTP/TCP probes

VM HTTP/TCP requires no guest agent. The driver-aware probe target should use
the allocation's guest `workload_addr` when the descriptor omits a concrete
host, while an explicit host retains its documented meaning. The dial remains
host-originated and follows the existing host route through the host veth,
allocation netns, and routed TAP to the guest. DESIGN must state where that
per-allocation target enters ProbeRunner without inventing a second source of
truth.

The implementation should not map a VM wildcard to host `127.0.0.1`; that tests
the worker/VMM namespace, not the guest service. Kubernetes' node-to-Pod-IP
probe behavior is a useful analogue.

### 16. Security and trust boundary

The recommended mechanism narrows rather than removes the trust questions:

- Host-side access to the CH vsock backend must remain under the allocation's
  confined run directory with strict ownership/mode. A global or guessable
  management socket is unacceptable.
- Treat every guest frame as attacker-controlled: length-prefix limits,
  bounded argv count/string length, strict decoding, request limits, no
  panics, no allocations based solely on peer sizes, and no returned bulk data.
- The protocol is host-command/guest-result only. A guest must never request
  host process execution or host filesystem access.
- Bind/listen before launching the workload. A workload in the same guest can
  still attack or impersonate a guest control client if it obtains sufficient
  privilege; vsock CID alone does not identify PID 1. Current Overdrive runs
  the workload directly under its minimal init and has no documented guest
  privilege boundary that makes result integrity against a hostile guest root
  possible. DESIGN must state whether health is advisory/self-reported or
  adversary-resistant.
- If adversary-resistant probe truth is required, the project first needs a
  guest isolation design (separate UID/capabilities, process/cgroup namespace,
  protected secret/attestation, and immutable agent), which is outside GH
  #257's current evidence. Do not claim vsock encryption/authentication solves
  a compromised guest kernel.

For the common health-check model—platform host trusted, workload potentially
buggy, health result advisory—the minimal channel is proportionate. A malicious
guest can already lie through HTTP or simply return exit zero; the critical
security property is that it cannot use the parser/control transport to escape
to the host.

## Recommendation

Proceed to DESIGN with this mechanism:

1. Enable `[vm]+[service]` as a driver combination and retain per-mechanic
   dispatch rather than a blanket VM-Service rejection.
2. Route TCP/HTTP probes from the host to the existing guest
   `workload_addr`; qualify the host-originated SYN on native metal.
3. Evolve `overdrive-init` into the persistent workload/probe supervisor.
4. Add a dedicated, guest-initiated, per-allocation vsock probe channel on a
   separate port, with a deliberately versioned bounded RPC language.
5. Route VM Exec ticks through that session; keep host Exec ticks on the
   current host adapter. Never send VM Exec through `CgroupExecProber`.
6. Enforce deadline and kill/reap inside the guest, with no initial output
   capture and no ambiguous replay.

If the bounded spike cannot demonstrate reliable guest process-tree cleanup or
session isolation, keep VM Exec rejected at parse time while still delivering
VM Services with HTTP/TCP probes. That is the safe fallback. The fallback must
say “unsupported by this release,” not “impossible.”

Do not define the Rust public API in this research artifact. The current
`ExecProber` signature is host-cgroup-specific, `Vmm` intentionally excludes
guest graceful control, and `VmDriver` lacks ProbeRunner wiring. Selecting the
new internal port/owner and its exact types is DESIGN work governed by the
repository's no-invented-surface rule.

## Bounded Spike Plan

Run one native-metal spike against the built Cloud Hypervisor v53.0 path. Keep
it throwaway; its output is evidence for DESIGN, not production architecture.

### Hypotheses

- H1: one CH vsock device supports Beacon on port 1234 and a simultaneous
  guest-initiated probe session on a second fixed port without cross-talk.
- H2: after `READY` and while a long-running workload child remains alive, PID
  1 can receive multiple request-ID/argv/deadline frames and return correlated
  terminal results.
- H3: a timed-out command that forks a descendant can be terminated and fully
  reaped without killing the workload or VMM.
- H4: three concurrent requests complete independently within a bounded
  concurrency limit; an extra request gets deterministic overload rather than
  unbounded queuing.
- H5: dropping the host socket during an active probe causes bounded guest
  cleanup; reconnect creates a new session and does not replay the old command.
- H6: a host-originated TCP SYN to `workload_addr:<listener>` reaches the guest
  over the delivered TAP topology.

### Fixtures and observations

Use the real appliance kernel/rootfs/init, real CH binary, real allocation
netns/TAP, and the same run-dir/vsock arguments as production. The guest probe
fixture should cover: exit 0, nonzero exit, signal termination, missing binary,
sleep beyond deadline, forked descendant beyond deadline, output flood, and
malformed/oversized frame. Observe guest PIDs before/after, host VMM survival,
workload survival, bounded wall time, exact response IDs, socket cleanup, and
console diagnostics. Include one hostile guest client attempting malformed
frames and session impersonation to establish the actual trust boundary.

### Exit criteria

Pass only if all process descendants are gone and reaped after timeout/
disconnect, the workload and VMM remain alive, memory/output is bounded,
Beacon ordering is unchanged, reconnect does not duplicate execution, and the
host→guest network probe succeeds. A failure in H3 or H5 blocks Exec enablement;
a failure in H6 blocks HTTP/TCP default routing. Record commands, CH version,
kernel config, and raw evidence so DESIGN can cite rather than reinterpret it.

## Conflicting Evidence

1. **GH #257 wording vs actual possibility.** “The host cannot directly
   fork/exec inside a VM” is true only for a host syscall crossing the guest
   kernel boundary. The issue text overreaches when read as “Exec probes cannot
   work at all”; existing `overdrive-init`, KubeVirt/QGA, Kata, and
   Firecracker-containerd demonstrate agent-mediated in-guest execution.
2. **ADR-0059 namespace claim vs Linux/current code.** Cgroup placement controls
   resources but does not enter mount/network namespaces. The current adapter
   writes `cgroup.procs` and performs no `setns`. The ADR is accepted for its
   implemented host-cgroup behavior, but its namespace rationale must not be
   carried into VM DESIGN.
3. **Parser diagnostic vs delivered networking.** `VmNotAllowed` guidance says
   guest TAP/network support is absent. ADR-0088/0089 and GH #222 delivered it;
   the remaining work is target projection and host-originated qualification.
4. **Beacon comments vs shutdown behavior.** Host `VmDriver` can write
   `SHUTDOWN`, but current guest init reads it only after the one workload child
   exits. A long-running VM Service therefore requires persistent supervision;
   treating the current writer as sufficient would be misleading.
5. **Established precedent vs correct cleanup.** KubeVirt proves QGA-backed VM
   Exec probes are practical, but its observed polling timeout does not signal
   the guest PID. Kubernetes explicitly warns about leaked probe processes.
   Precedent supports the mechanism class, not blind copying.

## Knowledge Gaps

- Exact guest process-tree primitive: process group, guest cgroup v2 subtree,
  pidfd, or another bounded mechanism. This is the key spike question.
- Whether the shipped guest kernel/rootfs currently enables and mounts the
  primitives required for descendant-safe kill/reap.
- The exact concurrency cap and overload-to-ProbeOutcome mapping.
- Whether explicit TCP/HTTP `host` values for a VM may address non-guest hosts,
  or must always be constrained to the VM address.
- The intended trust model for a workload that gains guest root: advisory
  health versus authenticated platform-agent truth.
- Protocol framing/versioning and exact Rust trait ownership. These are DESIGN
  gaps, not permission to invent an API during implementation.
- Snapshot/restore is tracked beyond GH #257; if later enabled, connection
  generation and reconnect behavior need a separate accepted recovery design.

## Source Analysis

All sources were accessed 2026-09-06. Scores follow the research skill's
0–10 reputation rubric; “verified” means the cited page/source directly
supports the narrow claim, not that Overdrive has reproduced it on metal.

| Source | Domain | Reputation | Type | Access date | Cross-verified |
|---|---|---:|---|---|---|
| Overdrive production tree and accepted ADRs | Project | 10/10 high | Primary code/design | 2026-09-06 | Yes, against issues/spikes |
| Overdrive GH #42/#100/#222/#257 with comments | GitHub/project | 9/10 high | Primary issue record | 2026-09-06 | Yes, against tree/ADRs |
| Overdrive P2/P12 records | Project | 9/10 high | Executed spike evidence | 2026-09-06 | Yes, scope checked against production |
| Cloud Hypervisor vsock/API, `45972e6` | GitHub/upstream | 9/10 high | Official technical docs | 2026-09-06 | Yes, Firecracker + local CH use |
| Firecracker vsock, `7699746` | GitHub/upstream | 9/10 high | Official technical docs | 2026-09-06 | Yes, Cloud Hypervisor docs |
| Linux cgroup v2 | docs.kernel.org | 10/10 high | Kernel documentation | 2026-09-06 | Yes, man-pages + local code |
| Linux `setns`/namespaces | man7.org | 9/10 high | Canonical API reference | 2026-09-06 | Yes, kernel cgroup docs |
| Kubernetes probes/source, `b2ec8b6` | kubernetes.io/GitHub | 9/10 high | Official docs/source | 2026-09-06 | Yes, CRI protocol |
| CRI API, `b47fc37` | GitHub/upstream | 10/10 high | Primary protocol | 2026-09-06 | Yes, kubelet source |
| KubeVirt guide/source, `007c02c` | kubevirt.io/GitHub | 9/10 high | Official docs/source | 2026-09-06 | Yes, QGA protocol |
| QEMU Guest Agent | qemu.org | 9/10 high | Official protocol/docs | 2026-09-06 | Yes, KubeVirt source |
| Kata architecture/protocol, `3e30c0a` | GitHub/upstream | 9/10 high | Official design/source | 2026-09-06 | Yes, threat model + CRI precedent |
| Kata threat model | GitHub/upstream | 8/10 medium-high | Security design | 2026-09-06 | Yes, current Kata architecture |
| Firecracker-containerd agent | GitHub/upstream | 8/10 medium-high | Official architecture | 2026-09-06 | Yes, Firecracker/Kata |

No community answer, vendor blog, or search-result snippet is used as decisive
evidence. Claims about Overdrive fit and the recommended architecture are
explicit synthesis from these sources. Reputation distribution is 12 high
(86%) and 2 medium-high (14%); average score is 9.1/10.

## Full Citations

### Overdrive primary evidence

- [`beacon.rs`](../../../crates/overdrive-core/src/vm/beacon.rs) — published
  lifecycle language and port.
- [`overdrive-init`](../../../crates/overdrive-init/src/main.rs) — guest boot,
  one command, exit, and shutdown lifecycle.
- [`vm_driver.rs`](../../../crates/overdrive-worker/src/vm_driver.rs) — vsock
  listener, READY race, deferred EXEC, exit drain, and stop ownership.
- [`prober.rs`](../../../crates/overdrive-core/src/traits/prober.rs) and
  [`probe_runner`](../../../crates/overdrive-worker/src/probe_runner/mod.rs) —
  adapter contracts, target selection, supervision, and result writes.
- [`exec_prober.rs`](../../../crates/overdrive-worker/src/probe_runner/exec_prober.rs)
  — current host process/cgroup timeout path.
- [`vmm.rs`](../../../crates/overdrive-host/src/vmm.rs) — CH `--vsock`, `--net`,
  and netns launch composition.
- [ADR-0054](../../product/architecture/adr-0054-probe-runner-subsystem.md),
  [ADR-0059](../../product/architecture/adr-0059-exec-probe-cgroup-placement.md),
  [ADR-0082](../../product/architecture/adr-0082-vmm-port-trait-and-vmconfig-anti-corruption-value.md),
  [ADR-0083](../../product/architecture/adr-0083-driver-registry-and-per-driver-allocation-payload.md),
  [ADR-0088](../../product/architecture/adr-0088-guest-stack-routed-tap-netns-topology-and-addressing.md), and
  [ADR-0089](../../product/architecture/adr-0089-tap-in-netns-provisioning-boundary-and-ch-net-attach.md).
- [GH #42](https://github.com/overdrive-sh/overdrive/issues/42),
  [GH #100](https://github.com/overdrive-sh/overdrive/issues/100),
  [GH #222](https://github.com/overdrive-sh/overdrive/issues/222), and
  [GH #257](https://github.com/overdrive-sh/overdrive/issues/257), including
  their comments as retrieved on the access date.

### External primary evidence

- [Cloud Hypervisor vsock](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/45972e6afc184ba8769c2023870874380288d9c9/docs/vsock.md)
  and [management API](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/45972e6afc184ba8769c2023870874380288d9c9/docs/api.md).
- [Firecracker virtio-vsock design](https://github.com/firecracker-microvm/firecracker/blob/7699746649826d1dfcdde626b3131bac08f28e0d/docs/vsock.md).
- [Linux cgroup v2](https://docs.kernel.org/admin-guide/cgroup-v2.html),
  [`setns(2)`](https://man7.org/linux/man-pages/man2/setns.2.html), and
  [`namespaces(7)`](https://man7.org/linux/man-pages/man7/namespaces.7.html).
- [Kubernetes probe documentation](https://kubernetes.io/docs/concepts/workloads/pods/probes/),
  [kubelet prober source](https://github.com/kubernetes/kubernetes/blob/b2ec8b6fefac451a2dedafc4dd71f2f16c7a6abe/pkg/kubelet/prober/prober.go), and
  [CRI `ExecSync`](https://github.com/kubernetes/cri-api/blob/b47fc37312c54caef1a68c8b4ac8e07158331a82/pkg/apis/runtime/v1/api.proto#L1616-L1639).
- [KubeVirt probe guide](https://kubevirt.io/user-guide/user_workloads/liveness_and_readiness_probes/)
  and [guest exec implementation](https://github.com/kubevirt/kubevirt/blob/007c02cac10a6bf5ead0313c7d048b8d1358279a/pkg/virt-launcher/virtwrap/agent/exec.go).
- [QEMU Guest Agent protocol](https://www.qemu.org/docs/master/interop/qemu-ga-ref.html)
  and [daemon RPC controls](https://www.qemu.org/docs/master/interop/qemu-ga.html).
- [Kata architecture](https://github.com/kata-containers/kata-containers/blob/3e30c0afef54713528b3b275fe5f1561618a4292/docs/design/architecture/README.md),
  [agent protocol](https://github.com/kata-containers/kata-containers/blob/3e30c0afef54713528b3b275fe5f1561618a4292/src/libs/protocols/protos/agent.proto),
  and [threat model](https://github.com/kata-containers/documentation/blob/master/design/threat-model/threat-model.md).
- [Firecracker-containerd guest agent](https://github.com/firecracker-microvm/firecracker-containerd/blob/main/docs/agent.md).

## Research Metadata

**Research mode**: deep, implementation-grounded comparative research
**Duration**: approximately 2 hours across local inspection, source retrieval,
cross-reference, and synthesis
**Examined**: 30+ local production/design/spike files, four issue threads, and
20+ external pages/source files
**Cited**: 14 external source groups plus Overdrive primary evidence
**Cross-references**: seven mechanism/semantic clusters
**Confidence distribution**: high 75%, medium-high 25%, low 0%
**Local inspection completed before web search**: yes
**External search started**: after Part I baseline was written
**Access date for all web sources**: 2026-09-06
**Technology pins checked**: Cloud Hypervisor main `45972e6` (2026-09-05),
Firecracker main `7699746` (2026-09-04), Kata main `3e30c0a` (2026-09-04),
KubeVirt main `007c02c` (2026-09-05), Kubernetes main `b2ec8b6`
(2026-09-04), CRI API master `b47fc37` (2026-09-04), QEMU master
`ff1d2d1` (2026-09-04).
**Artifact boundary**: research only; no production code, accepted design,
feature delta, issue body, or public API was changed.
**Recommended next wave**: DISCUSS the trust/output/concurrency questions, run
the bounded metal spike, then DESIGN the exact internal boundaries and protocol.
