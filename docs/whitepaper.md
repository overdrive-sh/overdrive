# Helios: A Next-Generation Workload Orchestration Platform

**Version 0.12 — Draft**

---

## Abstract

Helios is an open-source workload orchestration platform built entirely in Rust, designed to replace Kubernetes, Nomad, and Talos for teams that demand simplicity, security, and efficiency without compromise. It unifies virtual machines, processes, unikernels, and serverless WASM functions under a single control plane, with a native eBPF dataplane, built-in mutual TLS, kernel-level mandatory access control, and LLM-driven self-healing observability — all without external dependencies like etcd, Envoy, SPIRE, or a CNI plugin.

The foundational thesis: the primitives required to build a genuinely better orchestration platform — stable eBPF APIs, production-ready Rust systems libraries, WASM runtimes, and kTLS offload — only reached maturity in the last two years. Helios makes different architectural choices than Kubernetes, not better ones for 2014, but definitively better ones for 2026.

---

## Table of Contents

1. [Motivation](#1-motivation)
2. [Design Principles](#2-design-principles)
3. [Architecture Overview](#3-architecture-overview)
4. [Control Plane](#4-control-plane)
5. [Node Agent](#5-node-agent)
6. [Workload Drivers](#6-workload-drivers)
7. [eBPF Dataplane](#7-ebpf-dataplane)
8. [Identity and mTLS](#8-identity-and-mtls)
9. [WASM Sidecars](#9-wasm-sidecars)
10. [Policy Engine](#10-policy-engine)
11. [Gateway](#11-gateway)
12. [Observability and Self-Healing](#12-observability-and-self-healing)
13. [Dual Policy Engine](#13-dual-policy-engine)
14. [Right-Sizing](#14-right-sizing)
15. [Zero Downtime Deployments](#15-zero-downtime-deployments)
16. [Serverless WASM Functions](#16-serverless-wasm-functions)
17. [Storage Architecture](#17-storage-architecture)
18. [Reconciler Model](#18-reconciler-model)
19. [Security Model](#19-security-model)
20. [Efficiency Comparison](#20-efficiency-comparison)
21. [Deterministic Simulation Testing](#21-deterministic-simulation-testing)
22. [Roadmap](#22-roadmap)

---

## 1. Motivation

### The State of Orchestration

Kubernetes dominates container orchestration. It is operationally well-understood, has an enormous ecosystem, and is supported by every major cloud provider. It is also a product of its time: designed in 2013, open-sourced in 2014, built on architectural assumptions that made sense a decade ago and are now significant liabilities.

- **etcd** is a separately operated distributed database that must be kept healthy for the cluster to function
- **kube-proxy** implements service routing via iptables — O(n) rule scan per packet, degrading linearly with cluster size
- **Sidecars** are the only viable service mesh model, adding a full proxy process per workload consuming CPU and memory
- **CNI plugins** are shell-executed on every pod start, introducing latency and operational complexity
- **CRDs and operators** are the extension model — Go binaries that run with cluster-admin privileges and frequently destabilize production clusters
- **Only containers** are a first-class workload type — VMs, processes, and unikernels are second-class citizens at best

Nomad is simpler and supports multiple workload types, but lacks Kubernetes' security depth, service mesh, and extensibility model. Talos provides an excellent immutable OS foundation but is tightly coupled to Kubernetes.

None of these platforms were designed with eBPF, kTLS, WASM, or modern Rust in mind — because those primitives did not exist at production quality when they were built.

### Why Now

Several foundational technologies reached production maturity simultaneously between 2023 and 2025:

- `aya-rs` — Rust-native eBPF program development, stable and production-used
- `openraft` — Pure Rust Raft consensus library (HA mode)
- `wasmtime` — Production-stable WASM runtime with WASI support
- `rustls` with kTLS offload — Kernel TLS with hardware crypto offload
- `redb` — Pure Rust embedded database (single and HA modes)
- BPF LSM — Kernel 5.7+, stable, enables custom MAC without SELinux complexity

Helios is the platform that becomes possible when all of these exist simultaneously.

---

## 2. Design Principles

**1. Own your primitives.**
No etcd. No Envoy. No SPIRE. No CNI. Every critical subsystem is built into the platform or is a standard Rust library. External process dependencies are liabilities.

**2. eBPF is the dataplane.**
All network policy enforcement, load balancing, service routing, flow telemetry, and mTLS happens at the kernel level via eBPF. No userspace proxies in the data path.

**3. Security is structural, not configurable.**
mTLS between all workloads is not an option — it is the default and cannot be disabled. Every packet carries cryptographic workload identity. Policy is enforced in the kernel, not by application cooperation.

**4. All workload types are first class.**
Virtual machines, processes, unikernels, containers, and WASM functions share one control plane, one identity model, one policy system, and one dataplane. Not one model bolted onto another.

**5. Observability is native, not retrofitted.**
eBPF gives the platform kernel-level visibility into every workload with full identity context from day one. The LLM observability layer operates on this data, not on logs scraped after the fact.

**6. The platform learns.**
Self-healing and right-sizing are not static rules. Historical incident memory, resource profiles, and LLM reasoning compound over time. The platform becomes more reliable and more efficient with operational age.

**7. Rust throughout.**
Memory safety, performance, and a maturing ecosystem that now covers every required primitive. No FFI to Go or C++ in the critical path.

**8. One binary, any topology.**
The control plane and node agent are compiled into a single binary. Role is declared at bootstrap, not at build time. A single-node development cluster and a hundred-node production cluster run the same binary with different configuration. There is no separate installation, no separate upgrade path, no separate operational model.

---

## 3. Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                        CLI / API                                │
│                  (gRPC + REST, tonic)                           │
├─────────────────────────────────────────────────────────────────┤
│          Control Plane  (co-located with node agent             │
│          on same bare metal, or dedicated — role config)        │
│                                                                 │
│  ┌──────────────┐  ┌────────────────┐  ┌─────────────────────┐ │
│  │  StateStore  │  │  Reconcilers   │  │  Built-in CA        │ │
│  │  single:redb │  │  (Rust traits  │  │  (SPIFFE/X.509)     │ │
│  │  ha: raft+redb  │   / WASM ext.) │  │                     │ │
│  └──────────────┘  └────────────────┘  └─────────────────────┘ │
│  ┌──────────────┐  ┌────────────────┐  ┌─────────────────────┐ │
│  │  Scheduler   │  │  Regorus +     │  │  DuckLake           │ │
│  │  (bin-pack)  │  │  WASM policies │  │  (telemetry, hot)   │ │
│  └──────────────┘  └────────────────┘  └─────────────────────┘ │
├─────────────────────────────────────────────────────────────────┤
│                       Node Agent                                │
│                                                                 │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │  aya-rs eBPF Dataplane                                   │   │
│  │  XDP (routing/LB) · TC (egress) · sockops (mTLS)        │   │
│  │  BPF LSM (MAC) · kprobes (telemetry)                    │   │
│  └──────────────────────────────────────────────────────────┘   │
│  ┌──────────┐ ┌──────────┐ ┌────────────┐ ┌────────────────┐   │
│  │ Process  │ │ MicroVM  │ │ Unikernel  │ │ WASM           │   │
│  │ Driver   │ │ (Cloud   │ │ (Cloud HV  │ │ Driver         │   │
│  │          │ │  HV)     │ │ + Unikraft)│ │ (Wasmtime)     │   │
│  └──────────┘ └──────────┘ └────────────┘ └────────────────┘   │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │  Gateway Subsystem  (optional, node.gateway.enabled)     │   │
│  │  hyper · rustls · route engine · middleware pipeline     │   │
│  └──────────────────────────────────────────────────────────┘   │
├─────────────────────────────────────────────────────────────────┤
│             Gossip / Health (SWIM)                              │
├─────────────────────────────────────────────────────────────────┤
│             Object Storage (Garage, S3-compatible)              │
└─────────────────────────────────────────────────────────────────┘

Single binary — role declared at bootstrap:
  role = "control-plane"        dedicated control plane member
  role = "worker"               dedicated worker node
  role = "control-plane+worker" both (single node or 3-node HA)
  node.gateway.enabled = true   activates ingress subsystem
```

---

## 4. Control Plane

### State Store

Helios abstracts control plane storage behind a single trait, with the implementation chosen by deployment mode. This means a single-node setup carries none of the overhead of a distributed consensus system — complexity scales with the deployment, not with the platform.

```rust
trait StateStore: Send + Sync {
    async fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;
    async fn put(&self, key: &[u8], value: &[u8]) -> Result<()>;
    async fn delete(&self, key: &[u8]) -> Result<()>;
    async fn watch(&self, prefix: &[u8]) -> Result<WatchStream>;
    async fn txn(&self, ops: Vec<TxnOp>) -> Result<TxnResult>;
}
```

Two implementations, one interface, selected at bootstrap:

```toml
[cluster]
mode = "single"   # LocalStore — redb direct, no Raft
# or
mode = "ha"       # RaftStore — openraft + redb, 3 or 5 nodes
peers = ["node-2:7001", "node-3:7001"]
```

**Single mode — `LocalStore` (redb direct)**

On a single node, Raft provides zero fault tolerance benefit while adding log serialization, fsync overhead, leader election machinery, and snapshot compaction on every write. `LocalStore` bypasses all of it — writes go directly to a redb ACID transaction:

```
Write path:  API request → redb transaction → done
Read path:   query → redb read → done
Footprint:   ~30MB RAM, single redb file, no background tasks
```

**HA mode — `RaftStore` (openraft + redb)**

For multi-node clusters, `RaftStore` wraps the same redb backend behind openraft consensus. All writes go through the Raft log before committing, providing linearizable reads and writes across a 3- or 5-node control plane:

```
Write path:  API request → Raft propose → quorum commit → redb → done
Read path:   linearizable read → Raft read index → redb → done
Footprint:   ~80MB RAM, redb + Raft log, background replication tasks
```

**Migration: single → HA**

Teams that start on a single node and grow to HA do not need external tooling or manual data migration. Both store implementations share the same snapshot format, and the migration is built into the platform CLI:

```
helios cluster upgrade --mode ha --peers node-2,node-3

1. LocalStore exports full state snapshot
2. RaftStore bootstraps from snapshot on all three nodes
3. Leader election completes
4. Cluster continues — zero downtime, no data loss
```

The snapshot interface is part of the `StateStore` contract:

```rust
trait StateStore: Send + Sync {
    // ... core operations ...

    /// Export full state for migration or backup
    async fn export_snapshot(&self) -> Result<StateSnapshot>;

    /// Bootstrap from an existing snapshot (used by RaftStore
    /// when initialising a new HA cluster from a single-node export)
    async fn bootstrap_from(&self, snapshot: StateSnapshot) -> Result<()>;
}
```

`export_snapshot` serialises the full key-value state of `LocalStore` into a portable `StateSnapshot`. `RaftStore::bootstrap_from` replays that snapshot as the initial Raft log entry on each peer before the cluster starts — no peer sees an empty state, no reconciliation loop runs against a blank slate. The snapshot format is also used for regular Raft snapshots in HA mode and for disaster recovery backups written to Garage, so the same code path is exercised continuously in production rather than only at migration time.

All authoritative cluster state — job definitions, node registrations, allocations, network policies, certificates — passes through whichever store is active. The rest of the control plane is unaware of which implementation is running.

```
Control plane footprint by mode:
  Single:  ~30MB RAM  — redb direct, no Raft overhead
  HA:      ~80MB RAM  — openraft + redb, 3-node quorum
  (vs ~1GB for Kubernetes control plane in either topology)
```

### Control Plane and Worker on the Same Node

Like Talos, Helios supports running the control plane and node agent on the same bare metal server. A node declares its role at bootstrap — dedicated worker, dedicated control plane member, or both:

```toml
[node]
role = "control-plane+worker"   # or "control-plane" or "worker"
```

Because the control plane and node agent are compiled into a single binary, co-location is a configuration choice, not an architectural compromise. The same binary activates different subsystems depending on the declared role.

```
Single node (development / edge):
  One binary, one server
  LocalStore (redb direct) + node agent
  Full platform capabilities, zero distributed systems overhead

Three-node HA cluster (typical production):
  All three nodes run control-plane+worker
  RaftStore — quorum requires 2/3 nodes healthy
  Workloads schedulable on all three nodes
  No dedicated control plane nodes wasting capacity

Five-node mixed cluster (larger deployments):
  3 nodes: control-plane+worker (RaftStore)
  N nodes: worker only
  Raft quorum isolated from workload scheduling pressure
```

### Workload Isolation on Co-located Nodes

When a node runs both roles, control plane processes run in dedicated cgroups with kernel-enforced resource reservations. A misbehaving workload cannot starve the state store or scheduler regardless of how aggressively it consumes CPU or memory:

```
/helios.slice/
  control-plane.slice/    ← reserved budget, never preempted
    raft.service
    scheduler.service
    ca.service
  workloads.slice/        ← remaining node capacity
    job-payments.scope
    job-frontend.scope
```

The scheduler respects a default taint on control plane nodes, preventing arbitrary workload placement unless explicitly tolerated:

```toml
[node.scheduling]
taint = "control-plane:NoSchedule"
```

Operators running three-node all-in-one clusters typically tolerate this taint cluster-wide. Larger deployments keep it as a guard against accidental overcommit on control plane members.

### Core Data Model

```
Job        — desired workload specification (driver, resources, constraints)
Node       — registered worker node with capabilities and labels
Allocation — binding of a job to a node, lifecycle state machine
Policy     — Rego-based network and security rules
Certificate — issued SVID, TTL, rotation schedule
```

### Built-in Certificate Authority

Helios embeds a full X.509 certificate authority directly in the control plane. There is no SPIRE server, no cert-manager, no Vault integration required for basic operation.

The root CA key lives in the state store, encrypted at rest. In HA mode it is Raft-replicated across all control plane nodes. Each node receives an intermediate CA certificate at bootstrap, signed by the root. The node agent issues short-lived leaf certificates (SVIDs, 1-hour TTL) for each workload it runs, using its intermediate.

SPIFFE IDs are used as the identity format:

```
spiffe://helios.local/job/payments/alloc/a1b2c3
```

Short TTLs eliminate the need for CRL or OCSP. Expiry is the revocation mechanism. The reconciler loop handles rotation automatically.

### Scheduler

The scheduler is a bin-packing allocator that assigns jobs to nodes based on declared resource requirements, node labels, affinity rules, and constraints. Scheduling decisions are state machine transitions written through the active store — linearizable in HA mode, ACID-transactional in single mode. No global lock, no single-threaded bottleneck.

Resource profiles maintained by the right-sizing subsystem feed real utilization data back to the scheduler over time, progressively improving placement density.

---

## 5. Node Agent

The node agent is a single Rust binary that runs on every worker node. It is responsible for:

- Registering with the control plane and maintaining heartbeat
- Loading and managing eBPF programs via aya-rs
- Receiving and applying BPF map updates from the control plane
- Requesting and distributing workload SVIDs from the built-in CA
- Running workloads via the appropriate driver
- Collecting telemetry from the eBPF ringbuf and forwarding to DuckLake
- Responding to reconciler actions (start, stop, migrate, resize)

The agent is event-driven throughout. BPF ringbuf events push telemetry without polling. Control plane instructions arrive via gRPC streaming. There are no periodic polling loops in the critical path.

---

## 6. Workload Drivers

Helios treats every workload type as a first-class citizen through a unified driver interface:

```rust
trait Driver: Send + Sync {
    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle>;
    async fn stop(&self, handle: &AllocationHandle) -> Result<()>;
    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationStatus>;
    async fn resize(&self, handle: &AllocationHandle, resources: &Resources) -> Result<()>;
}
```

| Driver | Backend | Use Case |
|---|---|---|
| `process` | tokio::process + cgroups v2 | Native binaries, daemons |
| `microvm` | Cloud Hypervisor | Fast-boot (~200ms), strong isolation |
| `vm` | Cloud Hypervisor | Full OS, hotplug, virtiofs, AArch64 |
| `unikernel` | Cloud Hypervisor + Unikraft | Extreme density, virtiofs-capable |
| `wasm` | Wasmtime | Serverless functions, plugins |

All drivers share the same identity model, the same eBPF dataplane, the same policy system, and the same telemetry pipeline. A network policy that governs a process workload governs a VM workload identically.

### Cloud Hypervisor as the Unified VMM

Helios uses **Cloud Hypervisor** as its sole VMM, handling both microvm and full VM workloads. This replaces the two-VMM model (Firecracker for microvms, QEMU for full VMs) that most platforms adopt. Cloud Hypervisor is written in Rust, maintains a minimal attack surface, and supports the full capability set required across all VM-class workloads.

| Capability | Firecracker | Cloud Hypervisor | QEMU |
|---|---|---|---|
| Fast boot (~200ms) | ✅ | ✅ | ❌ |
| Full VM (arbitrary OS) | ❌ | ✅ | ✅ |
| virtiofs filesystem sharing | ❌ | ✅ | ✅ |
| CPU / memory hotplug | ❌ | ✅ | ✅ |
| AArch64 | ❌ | ✅ | ✅ |
| Written in Rust | ✅ | ✅ | ❌ |
| No central daemon | ✅ | ✅ | ❌ |

QEMU is retained only as an explicit opt-in for workloads with exotic hardware emulation requirements. It is not part of the default node agent deployment.

### One Process Per VM

Cloud Hypervisor follows the same process model as Firecracker — one process per VM, no central daemon. The `cloud-hypervisor` binary is the VMM, not a CLI to a background service. The node agent spawns and manages these processes directly:

```rust
impl Driver for CloudHypervisorDriver {
    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle> {
        // Start virtiofsd if the job declares filesystem mounts
        let virtiofs = if spec.has_mounts() {
            Some(self.start_virtiofsd(&spec).await?)
        } else {
            None
        };

        // Spawn the VMM process — this process IS the hypervisor
        let proc = Command::new(&self.binary_path)
            .args(self.build_args(&spec, &virtiofs))
            .spawn()?;

        Ok(AllocationHandle { proc, virtiofs, .. })
    }

    async fn resize(&self, handle: &AllocationHandle, resources: &Resources) -> Result<()> {
        // Live CPU and memory hotplug — no VM restart required
        // Directly enables the right-sizing subsystem for VM workloads
        self.api_call(handle, "vm.resize", &ResizeConfig {
            desired_vcpus: resources.cpu_cores,
            desired_ram: resources.memory_bytes,
        }).await
    }
}
```

### virtiofs and Cross-Workload Volume Sharing

Cloud Hypervisor's virtiofs support (backed by a `virtiofsd` daemon per VM) enables shared filesystem volumes between workload types — a capability Firecracker permanently forecloses:

```
VM workload writes to /shared-volume  (virtiofs mount)
Process workload reads /shared-volume (bind mount)
    → same volume, same data, different workload types
    → lifecycle managed by the storage reconciler
```

Unikraft added virtiofs support to mainline (`lib/ukfs-virtiofs`, December 2025), meaning unikernel workloads participate in the same volume model.

### Live VM Right-Sizing

Cloud Hypervisor's CPU and memory hotplug integrates directly with the right-sizing subsystem. Where process workloads are right-sized via live cgroup adjustment, VM workloads are right-sized via hotplug — no restart, no workload disruption:

```
eBPF detects VM memory pressure approaching limit
    │
Tier 1: node agent issues vm.resize via CH API
    │   (memory hotplug, guest kernel sees new pages)
    │
No VM restart. No workload interruption.
```

This is not possible with Firecracker, which has no hotplug support. The right-sizing story is now uniform across all workload types.

---

## 7. eBPF Dataplane

The eBPF dataplane is the core of Helios' network and security architecture. All programs are written in Rust using aya-rs and loaded by the node agent at startup.

### XDP — Fast Path Packet Processing

XDP programs attach at the NIC driver level, before the Linux kernel networking stack, enabling:

- **Service load balancing** — O(1) BPF map lookup for VIP-to-backend resolution, replacing kube-proxy entirely
- **Network policy enforcement** — identity-based allow/deny at line rate
- **DDoS mitigation** — drop attack traffic before it consumes kernel resources
- **Zero-copy packet redirection** — via AF_XDP for telemetry fast path

XDP operates in native mode on supported NICs (virtio, mlx5, i40e) and falls back to generic mode for compatibility. The BPF maps driving XDP decisions are updated atomically by the node agent — rolling deployments are BPF map updates, not proxy reconfigurations.

### TC — Egress Control

Traffic Control (TC) programs operate on the egress and ingress paths and implement:

- Egress interception for workloads with sidecars declared — redirects traffic through the node agent's sidecar handler chain (credential proxy, content inspection, user WASM modules)
- Per-workload traffic shaping and rate limiting
- Flow export to the telemetry ringbuf

### sockops — Kernel mTLS

BPF socket operations programs intercept the socket lifecycle to implement transparent kernel mTLS:

1. A new connection is initiated by any workload
2. The sockops program intercepts `BPF_SOCK_OPS_TCP_CONNECT_CB`
3. The node agent performs the TLS 1.3 handshake via rustls, presenting the workload's SVID
4. Session keys are installed into kTLS — the kernel record layer takes over
5. All subsequent encrypt/decrypt happens in-kernel, with optional NIC offload

The application is completely unaware. This works identically for process workloads, VMs, unikernels, and WASM functions — there is no sidecar injection required or possible.

### BPF LSM — Mandatory Access Control

BPF LSM programs (Linux 5.7+) attach to kernel security hooks to enforce per-workload mandatory access controls:

- `file_open` — restrict filesystem access to declared mount paths
- `socket_create` — prevent raw socket creation (blocks mTLS bypass attempts)
- `socket_connect` — enforce egress port policy
- `task_setuid` — prevent privilege escalation
- `bprm_check_security` — enforce binary execution allowlists

LSM enforcement is in-kernel and cannot be bypassed regardless of what code runs inside a workload. A compromised process, VM, or WASM function hits the same kernel walls as a well-behaved one.

### BPF Map Architecture

All eBPF programs are readers of BPF maps. The node agent is the only writer. The control plane pushes policy decisions via gRPC; the agent writes them into maps; eBPF programs enforce them:

```
Control Plane (Regorus evaluates policy)
    │ gRPC push
    ▼
Node Agent (writes BPF maps via aya-rs)
    │
    ▼
BPF Maps: POLICY_MAP · SERVICE_MAP · IDENTITY_MAP · FS_POLICY_MAP
    │ read-only
    ▼
XDP · TC · sockops · BPF LSM programs
```

Policy evaluation (Rego/Regorus) is never in the hot path. Per-connection enforcement is an O(1) BPF map lookup measured in nanoseconds.

---

## 8. Identity and mTLS

### SPIFFE Identity Model

Every workload allocation receives a SPIFFE ID encoding its platform identity:

```
spiffe://helios.local/job/payments/alloc/a1b2c3
```

This identity is backed by a short-lived X.509 SVID issued by the node agent using its intermediate CA certificate. The full chain is:

```
Helios Root CA (control plane, Raft-backed)
    └── Node Intermediate CA (per node, issued at bootstrap)
            └── Workload SVID (per allocation, 1hr TTL)
```

SVIDs are issued at workload start, rotated automatically before expiry, and revoked when the workload stops. The reconciler loop manages rotation — it is just another reconciler action.

### Kernel mTLS Operation

mTLS between workloads is transparent and universal:

```
Workload A calls connect() to Workload B
    │
sockops intercepts
node agent fetches SVID for A, trust bundle for cluster
rustls performs TLS 1.3 handshake (A presents SVID, verifies B's SVID)
session keys installed into kTLS
node agent exits data path
    │
kTLS handles all encrypt/decrypt in-kernel
optional NIC offload for crypto operations
```

IP addresses are routing hints, not security boundaries. Policy is expressed in terms of SPIFFE identities. A workload can move nodes, scale to 100 instances, and receive a new IP — the policy remains correct because it references `job/payments`, not `10.0.1.45`.

### Credential Proxy

The credential proxy is a built-in sidecar — the first instance of a general pattern described fully in section 9. For workloads that interact with external services, it ensures real API keys and tokens never enter the workload sandbox. The platform generates dummy credentials, intercepts outbound requests via TC eBPF, swaps dummy credentials for real ones, and enforces domain allowlists. A compromised workload that exfiltrates its credentials has nothing useful.

### Credential Proxy for AI Agents

The credential proxy is particularly valuable for AI agent workloads — any workload that processes untrusted content (documents, emails, web pages, API responses) and has real capabilities (shell access, API credentials, internet connectivity).

An agent that can call external APIs and has access to real credentials is dangerous if it starts following instructions embedded in the content it processes — a technique known as prompt injection. No amount of system prompt hardening reliably prevents this. The structural defense is to ensure the agent never holds credentials worth stealing and cannot reach destinations it has not been explicitly authorised to reach.

```toml
[job]
name = "ai-research-agent"
driver = "wasm"

[[job.sidecars]]
name = "credential-proxy"
module = "builtin:credential-proxy"
hooks = ["egress"]
config.allowed_domains = ["api.anthropic.com", "gmail.com"]
config.credentials = { ANTHROPIC_KEY = { secret = "anthropic-prod" } }
```

The agent receives dummy credentials. The proxy holds the real ones, verifies every outbound request against the domain allowlist, and checks that the token being presented matches the one issued for this specific workload — preventing an injected instruction from authenticating to an allowed domain using the attacker's own token. LSM blocks raw socket creation so the agent cannot bypass the proxy regardless of what instructions it follows.

The principle: security properties are enforced by infrastructure, not by the model's judgment. A compromised agent hits the same walls as a well-behaved one.

---

## 9. WASM Sidecars

The credential proxy is a specific instance of a general pattern. Any workload may need request interception, transformation, or enforcement logic that sits between it and the network — without modifying the workload itself. Helios formalises this as the WASM sidecar model.

### The Pattern

A sidecar is a WASM module that intercepts ingress and/or egress traffic for a workload, processes each request through a handler, and returns an action:

```rust
trait Sidecar: Send + Sync {
    async fn on_egress(&self, req: Request, ctx: &SidecarContext) -> SidecarAction;
    async fn on_ingress(&self, req: Request, ctx: &SidecarContext) -> SidecarAction;
    async fn on_start(&self, ctx: &SidecarContext) -> Result<()>;
    async fn on_stop(&self, ctx: &SidecarContext) -> Result<()>;
}

enum SidecarAction {
    Pass(Request),           // forward unmodified
    Modify(Request),         // forward modified
    Block(StatusCode, Body), // return error to caller
    Redirect(SocketAddr),    // forward to different backend
}

struct SidecarContext {
    workload_identity: SpiffeId,
    workload_policy:   Policy,
    db:                Db,         // same libSQL interface as reconcilers
    secrets:           SecretStore,
}
```

### How Traffic Is Intercepted

The TC eBPF layer already handles interception. When a workload has sidecars declared, its cgroup ID is added to a `SIDECAR_MAP`. The TC egress/ingress programs redirect matching traffic to the node agent's sidecar handler, which runs the WASM chain and forwards the result. No separate process, no TCP stack traversal, no iptables rules:

```
Workload → connect() / send()
    │
TC eBPF: is this alloc in SIDECAR_MAP?
    │ yes
    ▼
Node agent sidecar handler (in-process)
    │ run sidecar chain in order
    │ each module: Pass / Modify / Block / Redirect
    ▼
XDP dataplane → destination (or blocked)
```

### Job Spec Interface

Sidecars are declared as an ordered chain. Each is applied in sequence — a block from any sidecar short-circuits the rest:

```toml
[job]
name = "ai-research-agent"
driver = "wasm"

# Platform built-in — credential management
[[job.sidecars]]
name = "credential-proxy"
module = "builtin:credential-proxy"
hooks = ["egress"]
config.allowed_domains = ["api.anthropic.com", "gmail.com"]
config.credentials = { ANTHROPIC_KEY = { secret = "anthropic-prod" } }

# User WASM module — AWS SigV4 request signing
[[job.sidecars]]
name = "aws-sigv4"
module = "sha256:abc123"
hooks = ["egress"]

# User WASM module — prompt injection detection on incoming content
[[job.sidecars]]
name = "content-inspector"
module = "sha256:def456"
hooks = ["ingress"]
config.mode = "block"

# User WASM module — structured audit log
[[job.sidecars]]
name = "audit-logger"
module = "sha256:ghi789"
hooks = ["egress", "ingress"]
```

### Built-in vs User Sidecars

Built-in sidecars are Rust native trait objects — zero WASM overhead for common cases. User sidecars are WASM modules — sandboxed, hot-reloadable, language-agnostic, content-addressed in Garage:

| Module | Type | Purpose |
|---|---|---|
| `builtin:credential-proxy` | Rust | Credential swap, domain allowlist |
| `builtin:content-inspector` | Rust | Prompt injection detection via LLM (rig-rs) |
| `builtin:rate-limiter` | Rust | Per-workload rate limiting |
| `builtin:request-logger` | Rust | Structured audit log → DuckLake |
| `sha256:...` | WASM | User-defined, any language |

### Sidecar SDK

```rust
// Rust WASM sidecar — AWS SigV4 signing
use helios_sidecar_sdk::{Request, SidecarAction, SidecarContext};

#[helios_sidecar::egress]
async fn on_egress(req: Request, ctx: SidecarContext) -> SidecarAction {
    let key = ctx.secrets().get("AWS_SECRET_KEY").await?;
    let signed = sign_sigv4(req, &key, "us-east-1", "execute-api")?;
    SidecarAction::Modify(signed)
}
```

```typescript
// TypeScript WASM sidecar — JWT validation
export async function onIngress(
    req: Request,
    ctx: SidecarContext
): Promise<SidecarAction> {
    const token = req.headers.get("Authorization")?.replace("Bearer ", "");
    if (!token || !await ctx.verifyJwt(token)) {
        return SidecarAction.block(401, "Unauthorized");
    }
    return SidecarAction.pass(req);
}
```

### What Users Can Build

Because `SidecarContext` provides workload identity, DB access, and a secret store, sidecars can implement anything that belongs in the request path:

| Sidecar | What It Does |
|---|---|
| AWS SigV4 signing | Sign outbound requests with scoped AWS credentials |
| JWT validation | Validate inbound tokens, inject identity headers |
| Request deduplication | DB-backed idempotency key checking |
| Semantic caching | Cache LLM responses by embedding similarity |
| PII scrubbing | Strip sensitive fields before forwarding |
| Protocol translation | REST → gRPC, JSON → protobuf |
| Chaos injection | Random failures for resilience testing |

The chaos sidecar is worth highlighting — attaching it to a downstream dependency lets workloads be tested for resilience without modifying infrastructure or the workload itself. The same failure modes used in DST simulation are injectable in production via a sidecar.

### Relationship to the Credential Proxy

The credential proxy does not change — it becomes `builtin:credential-proxy`, a first-party sidecar implemented as a native Rust trait object. The TC eBPF interception mechanism it uses becomes the general interception layer for all sidecars. The content inspection feature described in the WASM functions section becomes `builtin:content-inspector`, reusable by any workload type rather than specific to WASM functions.

### WASM Sidecars vs WASM Policies

Both are WASM modules that influence what a workload can do. The distinction is precise and matters for correct usage.

**Policy answers: should this be allowed?**
**Sidecar answers: what should happen to this request?**

Policy is evaluated before anything happens — at scheduling time, at admission, at connection establishment. It is a gate with a binary outcome: allow or deny. The result is compiled into a BPF map and enforced at the kernel level on every subsequent connection at zero per-request cost.

Sidecar is evaluated during the request — in the data path, on every message. It is a transformer with a rich outcome: pass, modify, block, or redirect. It runs on every single request for the lifetime of the workload.

```
Job submitted → Policy: "can this workload reach stripe.com?"
                → Allow (compiled to BPF map, enforced in kernel)
                → Never evaluated again per connection

Workload running, request outbound → Sidecar: "sign this request with SigV4"
                                   → Modify (signs and forwards)
                                   → Runs on every request
```

The execution model reflects this:

| | WASM Policy | WASM Sidecar |
|---|---|---|
| When | Control plane reconciliation | Per request, in data path |
| Frequency | Once per policy change | Every request |
| Latency budget | ~ms acceptable | ~μs required |
| Output | Verdict → BPF map | Transformed request / action |
| Hot path | No | Yes |

**Choosing between them:**

```
Decision is static for the workload's lifetime?  → Policy
Requires inspecting / transforming request content? → Sidecar

"Can this workload reach gmail.com?"                → Policy
"Is this request using the right token?"            → Sidecar
"Can this job run on this node?"                    → Policy
"Does this response contain prompt injection?"      → Sidecar
"Can frontend talk to database?"                    → Policy
"Sign this request with SigV4"                      → Sidecar
```

They compose naturally. A policy deny at the kernel level never reaches the sidecar chain — there is no point running request transformation logic on a connection XDP already dropped. Policy sets the outer boundary; sidecars handle the inner behaviour on traffic that passes.

---

## 10. Policy Engine

### Regorus

Helios embeds **Regorus** — Microsoft's Rust-native Rego evaluation engine — directly in the control plane for policy evaluation. Rego is the language used by Open Policy Agent and is widely understood by platform and security engineers.

Regorus handles:
- Admission control (can this job be submitted?)
- RBAC (who can perform what operations?)
- Network policy (which jobs can communicate?)
- Scheduling constraints (where can this job run?)
- Audit rules (does this job spec comply with security policy?)

Regorus is **not** in the hot path. Policy is evaluated during control plane reconciliation — when jobs start, stop, or policies change — and compiled into BPF maps. Per-connection enforcement is a BPF map lookup.

### Policy Layers

```
Regorus (control plane)    — evaluates Rego policy → verdict decisions
BPF maps (node agent)      — stores compiled verdicts
XDP / LSM (kernel)         — enforces verdicts per packet / syscall
```

Policy changes propagate within a sub-second window: Regorus re-evaluates, node agents receive updated maps via gRPC, kernel programs enforce new policy.

### Example Policy

```rego
# Network policy — no frontend direct database access
deny_connection {
    input.src.job == "frontend"
    input.dst.job == "database"
}

# Scheduling policy — payments job requires PCI-compliant nodes
require_label {
    input.job.name == "payments"
    not input.node.labels["pci-compliant"] == "true"
}
```

---

## 11. Gateway

Helios includes a native HTTP/gRPC gateway built in Rust using `hyper` and `rustls`. There is no Envoy dependency.

The gateway is a built-in subsystem of the node agent, not a platform job. This distinction matters: a job depends on the scheduler, can be evicted, and requires the cluster to be healthy before it can run. The gateway needs to be available before any of that — it is infrastructure, not a workload. Making it a job would create a bootstrap deadlock and contradict the single-binary design principle.

Gateway nodes are designated by configuration, not by scheduling:

```toml
[node]
role = "control-plane+worker"

[node.gateway]
enabled = true
http_port  = 80
https_port = 443
```

The node agent activates the gateway subsystem at startup on nodes where it is enabled — no scheduling step, no chicken-and-egg dependency on the control plane being healthy:

```rust
struct NodeAgent {
    ebpf:          EbpfDataplane,      // always active
    drivers:       DriverRegistry,     // always active
    identity:      IdentityManager,    // always active
    gateway:       Option<Gateway>,    // active if node.gateway.enabled
    control_plane: Option<ControlPlane>, // active if role includes control-plane
}
```

Because the gateway runs in the same process as the node agent, it has direct access to internal state with no IPC overhead:

```rust
struct Gateway {
    // Direct in-process access — no gRPC, no IPC
    route_table: Arc<RouteTable>,    // shared with XDP dataplane
    identity:    Arc<IdentityMgr>,   // shared with sockops layer
    telemetry:   Arc<TelemetrySink>, // shared with eBPF ringbuf consumer
    tls:         Arc<TlsManager>,    // built-in CA, auto-rotation
}
```

Route updates are in-process state mutations. TLS certificate rotation is handled by the same identity manager that handles workload SVIDs. Telemetry writes to the same DuckLake pipeline. Everything is coherent because it is the same binary.

### Node Topologies

```
Single node (development / edge):
  role = "control-plane+worker", gateway.enabled = true
  One binary, one server, full ingress capability

Edge HA cluster:
  Node 1: control-plane+worker, gateway.enabled = true   ← ingress
  Node 2: control-plane+worker
  Node 3: control-plane+worker

Production (dedicated ingress tier):
  Node 1-2: worker, gateway.enabled = true               ← ingress tier
  Node 3-5: control-plane+worker
  Node 6-N: worker
```

### Capabilities

- TLS 1.3 termination via rustls and the built-in CA (automatic rotation)
- HTTP/1.1, HTTP/2, gRPC, gRPC-Web, WebSocket
- Declarative route configuration pushed from control plane
- Composable middleware pipeline: rate limiting, JWT auth, CORS, circuit breaking, egress inspection
- In-process BPF map access for routing table updates — route changes are atomic, no restart

### Route Configuration

Routes are declared as top-level platform resources and pushed to gateway nodes by the control plane — not embedded in job specs:

```toml
[[routes]]
host = "api.example.com"
path = "/payments/*"
backend = "job/payments"
timeout_ms = 5000

[routes.middleware]
rate_limit = { rps = 1000, burst = 100 }
auth = { mode = "jwt", issuer = "https://auth.example.com" }
```

### External Traffic Path

```
External client (TLS via rustls, built-in CA)
    │
Gateway subsystem (hyper, in-process route engine, middleware)
    │ mTLS (SPIFFE identity, same identity manager as all workloads)
    ▼
XDP dataplane (in-process BPF map lookup, DNAT)
    │
Backend service
```

Every request carries cryptographic workload identity end-to-end. The gateway is the first hop in the same identity-aware dataplane, with no architectural boundary between it and the rest of the node agent.

---

## 12. Observability and Self-Healing

### Native Telemetry

The eBPF layer produces structured, identity-tagged telemetry from the kernel for every workload without application instrumentation:

```
FlowEvent {
    timestamp, duration_ns,
    src_identity (full SPIFFE ID, job name, alloc ID, node),
    dst_identity,
    verdict, policy_rule_matched,
    tcp_retransmits, kernel_latency_ns,
    tls_version, certificate_ttl_remaining,
    bytes, connections_active
}

ResourceEvent {
    alloc_id, job_name,
    cpu_cycles, cpu_throttled_ns, runqueue_latency_ns,
    rss, page_faults_major, memory_pressure,
    disk_read_bytes, disk_write_bytes, io_wait_ns
}
```

Because every event carries the full workload identity — not a raw IP address — the LLM observability layer reasons about the cluster in business terms: `payments talking to database`, not `10.0.1.45:5432`.

### Storage

### Storage

Telemetry lives in **DuckLake** — an integrated data lake and catalog format using embedded libSQL as the catalog and Parquet files in Garage as storage. All control plane nodes write to and read from the same DuckLake instance with ACID guarantees. There is no hot/cold split to manage and no export pipeline — DuckLake handles retention, compaction, and Parquet lifecycle automatically. The LLM agent issues standard SQL queries against a single endpoint that spans the full history, with time travel available for historical correlation.

### LLM Agent (rig-rs)

Helios embeds an LLM agent via `rig-rs` (Rust-native LLM orchestration) that has tool access to the full telemetry store, cluster state, and control plane API:

```
Tools:
  query_flows(sql)          → flow event history
  get_job_status(job_id)    → current allocation state
  get_policy_decisions()    → recent Regorus evaluations
  get_node_metrics()        → resource utilization
  get_incident_history()    → past incidents and resolutions
  propose_action(action)    → submit action through approval gate
```

### Tiered Self-Healing

Self-healing operates at three tiers, each appropriate to its response time requirements:

**Tier 1 — Reflexive (milliseconds, eBPF)**
- Dead backend detected → BPF map updated, traffic rerouted immediately
- Memory pressure approaching OOM → cgroup limit expanded before OOM kill
- SYN flood → XDP drop at NIC before kernel TCP stack

**Tier 2 — Reactive (seconds, reconciler)**
- Crashed allocation → reschedule on healthy node
- Node unhealthy → drain and migrate workloads
- Replica count below desired → scale up

**Tier 3 — Reasoning (seconds to minutes, LLM)**
- Failures that don't match predefined patterns
- Correlation across cert events, resource metrics, historical incidents
- Root cause analysis with proposed remediation
- Graduated approval gate based on action risk level

### Incident Memory

Every incident, its diagnosis, actions taken, and outcome are stored in **libSQL** (embedded SQLite). The LLM agent retrieves similar past incidents before reasoning about new anomalies using embedding-based similarity search. The platform's diagnostic accuracy improves with operational age.

### OpenTelemetry Compatibility

Helios' internal telemetry model is richer than the OTel data model and is not built on OTel primitives. However, Helios emits OTLP for interoperability with external backends (Datadog, Grafana, Jaeger, Honeycomb). The OTel Collector is available as a pre-configured platform job. OTel is an export format, not a foundation.

---

## 13. Dual Policy Engine

### The Problem With a Single Policy Model

Rego (via Regorus) is the right language for declarative, auditable policies — network rules, RBAC, admission control. It is readable by compliance teams, statically analyzable, and fast to evaluate. But it has hard limits: no persistent state, no imperative logic, no ability to reason across historical data. Complex scheduling heuristics, anomaly-based policies, and business-rule enforcement quickly exceed what Rego can express cleanly.

WASM policies solve this — but at the cost of the property that makes Rego valuable in the first place: auditability. A Rego policy is human-readable and statically analyzable. A WASM binary is opaque.

The answer is not to choose. Both engines coexist, selected per policy based on what the policy requires.

### Two Engines, One Interface

All policies — regardless of engine — return a `Verdict` through the same interface:

```rust
enum Verdict {
    Allow,
    Deny(String),   // reason string for audit log
    Defer,          // pass to next policy in chain
}

trait Policy: Send + Sync {
    fn evaluate(&self, input: &PolicyInput) -> Verdict;
}
```

The control plane evaluates a policy chain. Each policy can allow, deny, or defer to the next. The engine backing each policy is an implementation detail:

```toml
[[job.policies]]
name = "network-egress"
engine = "rego"
source = "policies/egress.rego"

[[job.policies]]
name = "placement-history"
engine = "wasm"
module = "sha256:abc123"
```

### Rego / Regorus — Auditable Policies

Regorus is the right engine for policies that compliance and security teams need to read and reason about:

| Policy Type | Why Rego |
|---|---|
| Network allow/deny | Declarative, auditable, line-by-line reviewable |
| RBAC | Standard pattern, tooling exists (conftest, opa check) |
| Admission control | Compliance teams must be able to verify |
| Job spec validation | Static analysis catches errors before deployment |

Rego policies can be statically analyzed — tools can prove properties about them without executing them. For regulated industries (finance, government, healthcare), this is not optional. "Here is our Rego policy" is auditable. "Here is our WASM binary" is not.

### WASM Policies — Stateful and Expressive

WASM policies use the same execution model and sandbox as WASM reconcilers. They have access to the same libSQL private DB, the same host function interface, and the same content-addressed storage in Garage. This enables policy logic that is simply impossible in Rego:

```rust
// Placement policy with historical OOM memory
#[policy]
fn schedule_allow(input: &ScheduleInput, db: &Db) -> Verdict {
    let oom_count: u32 = db.query("
        SELECT count(*) FROM events
        WHERE job_id = ? AND node_class = ? AND event = 'oom'
        AND timestamp > ? - 604800
    ", [input.job_id, input.node_class, now()])?;

    if oom_count > 2 {
        return Verdict::Deny(
            "repeated OOM on this node class in last 7 days".into()
        );
    }
    Verdict::Allow
}

// Security policy: deny connections from jobs with recent breach events
#[policy]
fn connection_allow(input: &PolicyInput, db: &Db) -> Verdict {
    let recent_breach: u32 = db.query("
        SELECT count(*) FROM security_events
        WHERE job_id = ? AND event = 'breach'
        AND timestamp > ? - 86400
    ", [input.src.job_id, now()])?;

    if recent_breach > 0 {
        return Verdict::Deny("source job has recent security breach".into());
    }
    Verdict::Allow
}
```

| Policy Type | Why WASM |
|---|---|
| Stateful scheduling | Needs DB access for placement history |
| Anomaly-based rules | Complex logic, historical correlation |
| Custom business rules | User-defined, arbitrary expressiveness |
| ML-based policy | Can embed inference logic directly |

### LLM-Generated Policies

The combination of WASM policies and the LLM observability agent enables a new operational pattern: natural language policy authoring.

```
Operator: "payments service should never communicate outside
           the EU after a security incident is logged"

LLM agent:
  1. Generates WASM policy module implementing this rule
  2. Stores module in Garage (content-addressed, immutable)
  3. Submits policy proposal to control plane
  4. Operator reviews generated source and approves
  5. Policy becomes active
```

The LLM writes the policy. The human reviews the source. The platform enforces the compiled WASM. Rego's declarative constraints would limit what the LLM could express — WASM removes that ceiling while keeping the human approval gate intact.

### Engine Selection Guide

```
Need compliance team to read it?            → Rego
Need static analysis / formal proofs?       → Rego
Simple allow/deny on current state?         → Rego
Need DB access / historical reasoning?      → WASM policy
Complex imperative logic?                   → WASM policy
User-defined or LLM-generated?             → WASM policy
Embedding inference logic?                  → WASM policy

Needs to inspect / transform request body? → Sidecar (not a policy)
Needs to run on every request?             → Sidecar (not a policy)
```

When in doubt, start with Rego. The auditability is worth the constraint. Graduate to WASM policy when Rego's limits become the bottleneck. If the logic needs to run in the request data path rather than at admission or connection establishment, it is a sidecar — see section 9.

### Performance Characteristics

Rego evaluation via Regorus: microseconds per evaluation, never in the hot path.

WASM policy evaluation: warm instances are fast (~microseconds for simple logic). The instance pool pattern from the WASM function driver applies here too — policies are pre-instantiated and reused. The first evaluation after deployment pays instantiation cost; subsequent evaluations do not.

Neither engine is in the packet forwarding hot path. Policy is evaluated during control plane reconciliation and compiled into BPF maps. Per-connection enforcement remains an O(1) BPF map lookup regardless of policy engine complexity.

---

## 14. Right-Sizing

### The Problem

In practice, most production clusters run at 20-40% actual resource utilization against allocated limits. Teams provision conservatively because over-provisioning causes wasted cost, while under-provisioning causes OOM kills and performance degradation.

Kubernetes' Vertical Pod Autoscaler requires pod restarts to resize and polls metrics at coarse intervals. It cannot prevent OOM kills — it can only react to them.

### Helios Approach

Helios observes actual resource consumption at the kernel level via eBPF kprobes and cgroup v2 BPF programs — continuously, without instrumentation, with full workload identity. This enables:

**Live cgroup resizing** — the node agent can expand a cgroup memory limit before an OOM kill occurs, without restarting the workload. This works for process, VM, and unikernel workloads identically.

**Resource profiles** — the reconciler accumulates p95 CPU and memory utilization per job, per hour-of-week, over a rolling 30-day window stored in libSQL. Right-sizing recommendations carry a confidence score based on sample count.

**Predictive scaling** — the LLM agent identifies time-based patterns (daily batch spikes, weekly traffic patterns) and proposes cron-based resource schedules. Resources are pre-expanded before spikes hit, not after.

**Bin-packing feedback** — right-sized resource profiles feed back to the scheduler. As jobs are right-sized, the scheduler can place more workloads per node, compounding the efficiency gain.

### Expected Outcome

Teams consistently running at 70% utilization instead of 30% — achievable with continuous right-sizing — do not merely save 57% on compute. They reduce node count, which reduces control plane overhead, network overhead, and operational burden. The efficiency gains compound.

---

## 15. Zero Downtime Deployments

Because Helios' load balancing is implemented as BPF map entries rather than proxy configuration, deployment strategies are BPF map update sequences. No proxy restart, no connection drop window, no configuration propagation delay.

### Rolling Deployment

```
1. Start new allocation (v2) alongside existing (v1)
2. Health check passes → add to SERVICE_MAP backend list (atomic)
3. Drain old allocation: stop new connections, await in-flight completion
4. Remove old allocation from SERVICE_MAP (atomic)
5. Terminate old allocation
6. Repeat for each replica
```

In-flight connection tracking uses sockops BPF maps — the agent knows exactly when it is safe to terminate.

### Canary and Blue/Green

Both are implemented as WASM or native reconcilers that drive BPF map weight updates:

- **Canary**: weighted backends (e.g., 95% v1, 5% v2), LLM agent monitors error rate and latency, promotes or rolls back automatically
- **Blue/green**: full parallel fleet, single atomic BPF map swap for cutover, old fleet retained as instant rollback target

### LLM-Supervised Promotion

The self-healing LLM agent watches deployment metrics automatically. For canary deployments, it compares error rates, latency distributions, memory utilization, and flow anomalies between versions and makes promotion or rollback decisions based on configurable SLO thresholds.

---

## 16. Serverless WASM Functions

### Cold Start

WASM is the only workload type where cold start is genuinely negligible:

| Workload Type | Cold Start |
|---|---|
| Container | 500ms – 2s |
| Firecracker microVM | ~125ms |
| WASM (instantiation only) | 1 – 5ms |

WASM modules are compiled to native code by Wasmtime at deployment time and cached on nodes. Subsequent invocations pay only instantiation cost.

### Instance Pool

The node agent maintains a pool of warm WASM instances per function. The LLM observability layer predicts demand from traffic patterns and adjusts the warm pool size proactively. Scale-to-zero drains the pool; scale-from-zero costs one instantiation (~1ms).

### Invocation Triggers

- **HTTP** — gateway routes requests directly to the WASM driver
- **Event** — platform event bus, jobs emit events that functions subscribe to
- **Schedule** — cron expressions managed by the reconciler

### Security

WASM functions receive tighter sandboxing than other workload types:

- **WASI capabilities** — filesystem, network, and environment access are explicitly granted in the job spec
- **Wasmtime fuel** — computational budget prevents infinite loops and CPU starvation
- **BPF LSM** — the Wasmtime process runs in a cgroup; LSM programs enforce syscall policy on the runtime itself
- **WASM sidecars** — the full sidecar chain (section 9) applies to WASM functions identically to any other workload type; `builtin:credential-proxy` and `builtin:content-inspector` are particularly relevant for functions processing untrusted content

WASM functions processing untrusted content (documents, web pages, API responses) cannot exfiltrate data regardless of what instructions that content contains. LSM blocks raw socket creation. TC redirects all egress through the sidecar chain. Infrastructure enforces security; the model's judgment is not required.

### Function SDK

```rust
#[helios_fn::handler]
async fn handle(req: Request, ctx: Context) -> Response {
    // ctx.identity()          → SPIFFE ID for this invocation
    // ctx.secret("API_KEY")   → credential fetched via proxy
    // ctx.emit_event(...)     → platform event bus
    // ctx.http()              → HTTP client routed via credential proxy

    let body: serde_json::Value = req.json()?;
    Response::json(&process(body, &ctx).await?)
}
```

Language-agnostic via the WASM Component Model. TypeScript, Go, Python, and Rust functions share the same platform primitives.

---

## 17. Storage Architecture

Different data shapes require different storage primitives. Helios uses purpose-fit storage at each layer:

### Control Plane State — StateStore (mode-dependent)

Hot metadata: jobs, nodes, allocations, policies, certificates. The active implementation depends on deployment mode:

- **Single mode** — redb direct. ACID transactions, no Raft overhead, ~30MB RAM. Right-sized for a single server without paying for distributed consensus that provides no benefit.
- **HA mode** — openraft + redb. Linearizable via the Raft log, replicated across 3 or 5 control plane nodes, ~80MB RAM.

Both implementations are pure Rust, embedded, and require no separate process. The rest of the platform is unaware of which is active. Migration from single to HA is non-destructive — both share the same snapshot format.

### Garage — Object Storage

Garage is a Rust-native S3-compatible object store designed for small clusters. In single mode it runs on the same node. In HA mode it replicates across nodes. It stores:

- WASM function modules (content-addressed by SHA-256 — immutable, auditable)
- VM and unikernel images
- Telemetry Parquet files (written and managed by DuckLake)
- State store snapshots (disaster recovery)

In single mode, Garage can be replaced with local filesystem storage — the same content-addressed interface applies, just without replication.

### Telemetry — DuckLake

eBPF flow events and resource metrics are append-only columnar data. Helios uses **DuckLake** — an integrated data lake and catalog format from the DuckDB team — as the unified telemetry store.

DuckLake separates catalog metadata (table schemas, snapshot history, file statistics) from data storage (Parquet files). In Helios:

- **Catalog** — a libSQL (SQLite) file embedded on the control plane node. Zero additional processes.
- **Storage** — Parquet files in Garage (S3-compatible). All telemetry data lives alongside other platform artifacts.

```
eBPF events → DuckLake
                │
                ├── catalog: libSQL (embedded, metadata only)
                └── data:    Parquet files in Garage (S3)
```

This replaces the previous hot/cold split (DuckDB for 7 days, manual export to Garage) with a single unified endpoint. There is no export pipeline to operate and no query routing logic to maintain.

**Multi-node writes.** In HA mode, all control plane nodes write telemetry to the same DuckLake instance with ACID transactional guarantees. Every node sees the full cluster telemetry. The LLM observability agent running on any control plane node queries the complete dataset:

```sql
-- LLM agent tool call — full history, all nodes, one endpoint
SELECT job_name, percentile_cont(0.99) WITHIN GROUP (ORDER BY duration_ns) as p99
FROM telemetry.flows
WHERE timestamp > now() - interval '1 hour'
  AND policy_rule_matched IS NOT NULL
GROUP BY job_name
ORDER BY p99 DESC
```

**Time travel.** DuckLake's snapshot model enables the LLM agent to correlate current anomalies against historical states without manual Parquet file management:

```sql
-- What were flow patterns at the time of the last incident?
SELECT * FROM telemetry.flows
AT (TIMESTAMP => '2026-04-01 14:32:00')
WHERE job_name = 'payments';
```

**Retention** is managed by DuckLake's snapshot expiry — no separate archival job required. Old snapshots are expired automatically; Garage storage is reclaimed via DuckLake's vacuum operation.

DuckLake is MIT-licensed and ships as a DuckDB extension — no new runtime dependency is introduced since DuckDB is already embedded in the control plane.

### Incident Memory — libSQL (embedded)

Historical incidents, resource profiles, LLM reasoning chains. Not on the critical path — eventual consistency acceptable. SQL interface is natural for LLM agent tool calls. Optional sync to Turso for cross-node incident sharing.

### Object Storage — Garage

Garage is a Rust-native S3-compatible object store designed for small clusters. It stores:

- WASM function modules (content-addressed by SHA-256 — immutable, auditable)
- VM and unikernel images
- Telemetry Parquet files (written by DuckLake, queried via DuckLake catalog)
- State store snapshots (disaster recovery)

In single mode, Garage can be replaced with local filesystem storage — the same content-addressed interface applies, just without replication.

### Reconciler Memory — libSQL (per-reconciler)

Each reconciler gets a private libSQL database for stateful memory across reconciliation cycles — restart tracking, placement history, resource sample accumulation. Reconciler DB writes are strictly private; all cluster state mutations go through the active StateStore. The consistency model never mixes.

---

## 18. Reconciler Model

### Design

Helios' reconciliation model is inspired by Kubernetes' control loop but with two key differences: reconcilers are strongly typed Rust trait objects (not Go processes with cluster-admin privileges), and they have access to a private persistent store for stateful reasoning.

```rust
trait Reconciler: Send + Sync {
    fn reconcile(
        &self,
        desired: &State,
        actual: &State,
        db: &Db,          // private libSQL — reconciler memory
    ) -> Vec<Action>;     // all mutations through Raft, never direct
}
```

### Extension Model

First-party reconcilers are Rust trait objects — maximum performance, full type safety. Third-party reconcilers are WASM modules loaded at runtime — sandboxed, hot-reloadable, language-agnostic. The interface is identical; the execution backend differs.

Input and output types are fully serializable from day one, making the WASM migration path trivial.

### Built-in Reconcilers

- Job lifecycle (start, stop, migrate, restart)
- Certificate rotation
- Resource right-sizing
- Rolling deployment strategies
- Canary promotion/rollback
- Node drain and replacement
- WASM function scaling
- Chaos engineering (deliberate fault injection for reliability testing)

---

## 19. Security Model

### Defense in Depth

Helios enforces security at four independent layers. A compromise of any one layer does not defeat the others:

```
Layer 1: WASM sandbox / VM isolation     — workload execution boundary
Layer 2: BPF LSM                         — kernel syscall policy
Layer 3: kTLS + SPIFFE mTLS              — network identity and encryption
Layer 4: XDP network policy              — packet-level enforcement
```

### No Trust in Workload Cooperation

Security properties are enforced by infrastructure, not by application behavior. A workload cannot:

- Bypass mTLS by opening raw sockets (BPF LSM blocks raw socket creation)
- Exfiltrate data to unauthorized domains (TC eBPF + credential proxy)
- Escalate privileges (BPF LSM task_setuid hook)
- Execute unauthorized binaries (BPF LSM bprm_check hook)
- Access unauthorized filesystem paths (BPF LSM file_open hook)

### Multi-Level Security

Job specs declare their security profile explicitly:

```toml
[job.security]
fs_paths = ["/data/payments", "/tmp"]
allowed_ports = [8080, 8443]
allowed_binaries = ["payments-server"]
no_raw_sockets = true
no_privilege_escalation = true
egress.mode = "intercepted"
egress.allowed_domains = ["api.stripe.com"]
```

The control plane compiles this into BPF maps. LSM and XDP programs enforce it. The security profile is as reliable as the kernel.

---

## 20. Efficiency Comparison

### Structural Advantages

| Component | Kubernetes | Helios |
|---|---|---|
| Service routing | iptables O(n) | XDP BPF O(1) |
| mTLS | Envoy sidecar (~0.5 vCPU each) | kTLS in-kernel (~0 overhead) |
| Control plane RAM | ~1GB | ~100MB |
| Network policy eval | Per-packet iptables | BPF map lookup |
| Node join | 2-5 minutes | <10 seconds |
| Workload types | Containers only | All (unified Cloud Hypervisor VMM) |
| Observability | Scraped logs | Kernel-native, structured |

### Utilization

The most significant efficiency gain is workload density. Kubernetes clusters typically run at 20-40% actual utilization against allocated limits. Helios' continuous right-sizing targets 60-80% utilization through live cgroup adjustment and predictive resource profiles.

Running at 70% utilization instead of 30% on the same hardware does not merely halve the node count. It reduces control plane overhead, network overhead, and operational cost in proportion — the gains compound.

### Estimated Performance Metrics

These are directional estimates based on analogous measurements from eBPF-based networking projects:

| Metric | Kubernetes | Helios | Estimated Gain |
|---|---|---|---|
| Network latency p99 | 2–10ms | 0.5–2ms | ~5x |
| mTLS CPU overhead | ~0.5 vCPU/sidecar | ~0 | ~100x |
| Control plane RAM | ~1GB | ~100MB | ~10x |
| Workload density | ~30% utilization | ~70% utilization | ~2.3x |
| Scheduling latency | 1–10s | <100ms | ~50x |
| Rolling deploy time | Minutes | Seconds | ~10x |

---

## 21. Deterministic Simulation Testing

Deterministic simulation testing (DST) is an approach to finding and reliably reproducing complex bugs in distributed systems — concurrency issues, timing races, partition behavior — that are effectively invisible to conventional tests. It was pioneered at FoundationDB and has since been adopted by TigerBeetle, WarpStream, RisingWave, and other serious distributed infrastructure projects.

The core requirement: every source of nondeterminism must be injectable. This is almost impossible to retrofit onto an existing system. Helios is designed with DST as a first-class constraint from day one.

### Sources of Nondeterminism

Every nondeterministic boundary in Helios is abstracted behind a trait:

```rust
// Time — no Instant::now() in production code
trait Clock: Send + Sync {
    fn now(&self) -> Instant;
    fn unix_now(&self) -> Duration;
    async fn sleep(&self, duration: Duration);
}

// Network — no direct TcpStream usage
trait Transport: Send + Sync {
    async fn connect(&self, addr: SocketAddr) -> Result<Connection>;
    async fn listen(&self, addr: SocketAddr) -> Result<Listener>;
}

// Randomness — no rand::random() in production code
trait Entropy: Send + Sync {
    fn u64(&self) -> u64;
    fn fill(&self, buf: &mut [u8]);
}

// Dataplane — eBPF cannot run in simulation
trait Dataplane: Send + Sync {
    async fn update_policy(&self, key: PolicyKey, verdict: Verdict) -> Result<()>;
    async fn update_service(&self, vip: Ipv4Addr, backends: BackendList) -> Result<()>;
    async fn get_flow_events(&self) -> Result<Vec<FlowEvent>>;
}

// Drivers — no real VMs or processes in simulation
trait Driver: Send + Sync {
    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle>;
    async fn stop(&self, handle: &AllocationHandle) -> Result<()>;
    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationStatus>;
    async fn resize(&self, handle: &AllocationHandle, resources: &Resources) -> Result<()>;
}
```

Each trait has two implementations: a real implementation for production, and a simulation implementation for testing. The compiler enforces the boundary — `rand` and `std::time` are direct dependencies only in platform wiring crates, never in core logic crates.

### Simulation Implementations

| Trait | Production | Simulation |
|---|---|---|
| `Clock` | `SystemClock` (wall clock) | `SimClock` (turmoil, controllable) |
| `Transport` | `TcpTransport` | `SimTransport` (turmoil, partitionable) |
| `Entropy` | `OsEntropy` (OS RNG) | `SeededEntropy` (StdRng, reproducible) |
| `Dataplane` | `EbpfDataplane` (aya-rs) | `SimDataplane` (in-memory HashMap) |
| `Driver` | `CloudHypervisorDriver` etc. | `SimDriver` (configurable failure modes) |
| `StateStore` | `LocalStore` / `RaftStore` | `LocalStore` (already pure Rust, no kernel) |

`SimDataplane` tracks policy and service state in memory and generates synthetic flow events — enough to test that the control plane correctly drives the dataplane without involving a real kernel. `SimDriver` can be configured to fail on start, crash after N operations, or consume exactly specified resources, making scheduler and reconciler logic fully testable without spawning real VMs.

### The Simulation Harness

Helios uses **turmoil** — a Rust DST framework that provides deterministic async simulation with controllable time, network, and multi-host environments — as the test harness foundation:

```rust
#[test]
fn test_leader_election_after_partition() {
    let mut sim = turmoil::Builder::new()
        .tick_duration(Duration::from_millis(1))
        .build();

    for i in 0..3 {
        sim.host(format!("node-{i}"), || async move {
            HeliosNode::new(SimConfig {
                clock:     Arc::new(SimClock::new()),
                transport: Arc::new(SimTransport::new()),
                entropy:   Arc::new(SeededEntropy::new(42)),
                dataplane: Arc::new(SimDataplane::new()),
                driver:    Arc::new(SimDriver::new()),
                store:     StoreMode::Ha,
            }).run().await
        });
    }

    sim.run(|| async {
        sim.advance(Duration::from_secs(5)).await;
        assert_invariant!(cluster_has_one_leader());

        // Partition node-0 from the rest
        sim.partition("node-0", &["node-1", "node-2"]);
        sim.advance(Duration::from_secs(10)).await;

        assert_invariant!(cluster_has_one_leader());
        assert_invariant!(leader_is_not("node-0"));

        // Heal partition — node-0 must rejoin and converge
        sim.repair("node-0", &["node-1", "node-2"]);
        sim.advance(Duration::from_secs(10)).await;

        assert_invariant!(all_nodes_agree_on_state());
    })
}
```

This test runs in milliseconds, is perfectly reproducible from the same seed, and exercises a failure mode that would take minutes to manifest in a real cluster and might never reproduce consistently.

### Properties

DST is paired with property-based testing. Helios tests three categories of invariant:

**Safety — nothing bad ever happens:**
```rust
assert_always!("single leader",
    cluster.nodes().filter(|n| n.is_leader()).count() <= 1
);
assert_always!("no double scheduling",
    allocations.iter().all(|a| a.node_count() == 1)
);
assert_always!("policy consistency",
    bpf_maps.reflects(regorus_decisions())
);
assert_always!("mTLS enforced",
    flows.iter().all(|f| f.tls_version.is_some())
);
```

**Liveness — good things eventually happen:**
```rust
assert_eventually!("job scheduled after submission",
    submitted_jobs.iter().all(|j| j.has_allocation())
);
assert_eventually!("leader elected",
    cluster.has_leader()
);
assert_eventually!("expiring certs rotated",
    expiring_svids.iter().all(|s| s.is_rotated())
);
```

**Convergence — reconcilers reach desired state:**
```rust
assert_eventually!("desired == actual",
    desired_state == actual_state
);
```

### Fault Injection Catalogue

The simulation harness exercises the following fault classes against every release:

```
Network:     partition (minority / majority), packet loss, reordering,
             duplication, latency injection, complete failure

Nodes:       clean crash + restart, crash mid-write, clock skew,
             slow node (CPU starvation)

Storage:     redb write failure, disk full, corrupt snapshot

Workloads:   driver fails to start, workload OOM, restart loop,
             workload consumes all node CPU

Control plane: leader crash during job submission, leader crash
               during cert rotation, reconciler panic,
               policy evaluation timeout
```

This catalogue also drives the chaos engineering reconciler in production — the same failure modes exercised in simulation are injected deliberately in live clusters to validate self-healing behavior.

### The StateStore Abstraction Is Already Correct

The `StateStore` trait with `export_snapshot` / `bootstrap_from` is the right shape for DST. Simulation tests use `LocalStore` with a `SimClock` — single-node, no Raft complexity, fully deterministic. `RaftStore` is added only to tests that specifically exercise consensus behavior. The two store implementations are independently testable, and the snapshot format shared between them is exercised continuously rather than only at migration time.

### Antithesis

For exhaustive state-space exploration beyond what turmoil covers, Helios is designed to be compatible with Antithesis — a deterministic hypervisor that runs regular software in a fully reproducible environment. Antithesis has a native Rust SDK. The property assertions defined for turmoil tests map directly to Antithesis assertions, making the two approaches complementary: turmoil for fast in-process tests during development, Antithesis for deep exploration against the real binary in CI.

---

## 22. Roadmap

### Phase 1 — Foundation (Months 1–3)
- Core data model (Job, Node, Allocation, Policy)
- Control plane API (tonic/gRPC — internal node-agent + CLI transport)
- StateStore abstraction: LocalStore (redb direct) for single mode
- Injectable Clock, Transport, Entropy, Dataplane, Driver traits
- turmoil simulation harness + SimDriver / SimDataplane / SimClock
- Process driver
- Basic scheduler (first-fit)
- CLI (`helios job submit`, `helios node list`, `helios alloc status`)

### Phase 2 — Networking (Months 3–6)
- aya-rs eBPF scaffolding
- XDP routing and service load balancing
- TC egress control
- BPF map management in node agent
- RaftStore (openraft + redb) for HA mode + single → HA migration

### Phase 3 — Identity and Security (Months 6–9)
- Built-in CA (rcgen + rustls)
- SPIFFE SVID issuance and rotation
- sockops mTLS + kTLS installation
- BPF LSM programs
- Regorus policy evaluation

### Phase 4 — Additional Drivers (Months 9–12)
- Cloud Hypervisor microVM and VM driver (replaces Firecracker + QEMU)
- virtiofsd lifecycle management and cross-workload volume sharing
- WASM serverless driver (Wasmtime)
- WASM sidecar runtime + TC eBPF interception generalisation
- Built-in sidecars: credential-proxy, content-inspector, rate-limiter, request-logger
- Sidecar SDK (Rust + TypeScript)
- Gateway (hyper + rustls)
- DuckLake telemetry pipeline (catalog: libSQL, storage: Garage Parquet)

### Phase 5 — Intelligence (Months 12–18)
- LLM observability agent (rig-rs)
- Self-healing tier 3 (LLM reasoning)
- Right-sizing reconciler
- Incident memory (libSQL)
- Predictive scaling

### Phase 6 — Ecosystem (Months 18+)
- WASM Component Model SDK (Rust, TypeScript, Go)
- OTel export adapter
- Unikernel drivers (Nanos, Unikraft with virtiofs)
- QEMU opt-in driver (exotic hardware emulation only)
- Multi-region federation

---

## Conclusion

Helios is not a Kubernetes improvement. It is a clean-slate design that leverages a set of primitives — stable eBPF APIs, Rust systems libraries, WASM runtimes, kernel TLS — that simply did not exist at production quality when Kubernetes was designed.

The result is a platform that is structurally more efficient, more secure, and more observable than any existing orchestrator, while supporting a broader range of workload types under a unified operational model.

The core insight is that eBPF is not a feature to add to an orchestrator. It is the right foundation for one. When the dataplane, the security model, the telemetry pipeline, and the service mesh all emerge from the same kernel primitive with the same workload identity attached, the platform is coherent in a way that bolted-on approaches cannot match.

---

*Helios is open source under the AGPL-3.0 license.*
*Contributions, feedback, and discussion welcome.*
