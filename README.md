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
23. [Image Factory](#23-image-factory)

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
- `cr-sqlite` + Corrosion — SQLite CRDT replication with SWIM gossip over QUIC, production-proven at Fly.io's global scale for continent-spanning routing state that Raft cannot express

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

**9. Strong consistency where it matters, gossip where it scales.**
Cluster state divides cleanly along a consistency boundary. *Intent* — job specs, policies, certificates, scheduler allocation decisions — requires linearizability and flows through per-region Raft. *Observation* — live allocation status, service endpoints, node health, resource profiles — tolerates seconds of staleness and flows through Corrosion: CR-SQLite tables gossiped over SWIM. Raft scales to a quorum; Corrosion scales to continents. Each tool is used where its guarantees fit, and never where they don't.

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
│  │  IntentStore │  │  Reconcilers   │  │  Built-in CA        │ │
│  │  single:redb │  │  (Rust traits  │  │  (SPIFFE/X.509)     │ │
│  │  ha: raft+redb  │   / WASM ext.) │  │                     │ │
│  │  (per region)│  │                │  │                     │ │
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
│      ObservationStore  (Corrosion — CR-SQLite + SWIM/QUIC)      │
│      alloc status · service backends · node health · regions    │
│      — local SQLite on every node · gossiped within / across    │
│        regions · subsumes plain membership gossip               │
├─────────────────────────────────────────────────────────────────┤
│             Object Storage (Garage, S3-compatible)              │
└─────────────────────────────────────────────────────────────────┘

Single binary — role declared at bootstrap:
  role = "control-plane"        dedicated control plane member
  role = "worker"               dedicated worker node
  role = "control-plane+worker" both (single node or 3-node HA)
  node.gateway.enabled = true   activates ingress subsystem
  cluster.region = "eu-west-1"  regional Raft + Corrosion peer
```

---

## 4. Control Plane

### The Intent / Observation Split

Helios splits cluster state along a fundamental consistency boundary, reflecting design principle 9:

|                | **Intent**                                        | **Observation**                                         |
|----------------|---------------------------------------------------|---------------------------------------------------------|
| Examples       | Job specs, policies, certificates, scheduler allocation decisions, compiled policy verdicts | Live allocation status, service backend IPs, node health, resource profiles |
| Consistency    | Linearizable                                      | Eventually consistent (seconds)                         |
| Backend        | Raft (openraft + redb), per region                | Corrosion (CR-SQLite + SWIM/QUIC), global               |
| Writer         | Control plane leader within region                | Every node writes its own rows                          |
| Reader         | Control plane reconcilers                         | Every node agent, scheduler, gateway, dataplane         |
| Scale ceiling  | 3–5 node quorum, one region                       | Thousands of nodes, many regions                        |
| Partition behavior | Minority region unavailable for writes        | Reads always succeed locally; writes catch up on heal   |

The split isolates two classes of bug. A Raft partition does not stall service routing — the dataplane reads observation, which stays live. A Corrosion backfill does not corrupt job specs — intent sits in a separate store with separate writers. Nothing in the codebase can cross the boundary accidentally: `IntentStore` and `ObservationStore` are distinct traits on distinct types.

This is the split Fly.io arrived at after years of trying to use Consul-style consensus for everything. Helios adopts it from day one, not after the incident.

### IntentStore — Authoritative Control Plane State

Helios abstracts intent storage behind a single trait, with the implementation chosen by deployment mode. A single-node setup carries none of the overhead of a distributed consensus system — complexity scales with the deployment, not with the platform.

```rust
trait IntentStore: Send + Sync {
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

**Per-region quorum.** In multi-region deployments each region runs its own Raft cluster. A region's intent store is authoritative for jobs and policies declared in that region. Cross-region visibility is provided by the observation store, not by stretching Raft across the WAN — see *Multi-Region Federation* below.

**Migration: single → HA**

Teams that start on a single node and grow to HA do not need external tooling or manual data migration. Both store implementations share the same snapshot format, and the migration is built into the platform CLI:

```
helios cluster upgrade --mode ha --peers node-2,node-3

1. LocalStore exports full state snapshot
2. RaftStore bootstraps from snapshot on all three nodes
3. Leader election completes
4. Cluster continues — zero downtime, no data loss
```

The snapshot interface is part of the `IntentStore` contract:

```rust
trait IntentStore: Send + Sync {
    // ... core operations ...

    /// Export full state for migration or backup
    async fn export_snapshot(&self) -> Result<StateSnapshot>;

    /// Bootstrap from an existing snapshot (used by RaftStore
    /// when initialising a new HA cluster from a single-node export)
    async fn bootstrap_from(&self, snapshot: StateSnapshot) -> Result<()>;
}
```

`export_snapshot` serialises the full key-value state of `LocalStore` into a portable `StateSnapshot`. `RaftStore::bootstrap_from` replays that snapshot as the initial Raft log entry on each peer before the cluster starts — no peer sees an empty state, no reconciliation loop runs against a blank slate. The snapshot format is also used for regular Raft snapshots in HA mode and for disaster recovery backups written to Garage, so the same code path is exercised continuously in production rather than only at migration time.

All authoritative intent — job definitions, node registrations (static identity, not runtime health), allocation decisions (the intent, not the live status), network policies, certificates — passes through whichever IntentStore implementation is active. The rest of the control plane is unaware of which is running.

```
Control plane footprint by mode:
  Single:  ~30MB RAM  — redb direct, no Raft overhead
  HA:      ~80MB RAM  — openraft + redb, 3-node quorum
  (vs ~1GB for Kubernetes control plane in either topology)
```

### ObservationStore — Live Cluster Map

Intent defines what the cluster *should* do. Observation defines what it *is doing right now*. Observation is what every node agent must read continuously to hydrate BPF maps; what the scheduler consults to bin-pack against real utilization; what the gateway resolves `spiffe://helios.local/job/payments` against to find a live backend set.

Pushing this from a single Raft leader via gRPC streams does not scale. Above a few hundred nodes it is a fan-out bottleneck; across regions, Raft's quorum-latency floor makes it impossible. Fly.io learned this the hard way with Consul; the answer they built — Corrosion — is open source, pure Rust, and production-proven across a continent-spanning fleet. Helios adopts it as the observation substrate:

```rust
trait ObservationStore: Send + Sync {
    async fn read(&self, sql: &str, params: &[Value]) -> Result<Rows>;
    async fn write(&self, sql: &str, params: &[Value]) -> Result<()>;
    async fn subscribe(&self, sql: &str) -> Result<RowStream>;
}
```

Under the hood each node runs a Corrosion peer backed by **cr-sqlite** — a SQLite extension that converts tagged tables into CRDTs with last-write-wins semantics under logical timestamps. Every write is logged in `crsql_changes` and gossiped to a random peer subset over QUIC within seconds. Every node ends up with a complete, ACID-queryable local SQLite of the cluster's observation state.

The core schema:

```sql
CREATE TABLE alloc_status (
    alloc_id       BLOB PRIMARY KEY,
    job_id         TEXT,
    node_id        TEXT,
    state          TEXT,          -- pending | running | draining | terminated
    svid_hash      BLOB,
    resources      BLOB,          -- current cgroup/vCPU/memory
    region         TEXT,
    updated_at     INTEGER        -- logical timestamp
);

CREATE TABLE service_backends (
    service_id     TEXT,
    alloc_id       BLOB,
    ip             BLOB,
    port           INTEGER,
    weight         INTEGER,
    health         INTEGER,
    PRIMARY KEY (service_id, alloc_id)
);

CREATE TABLE node_health (
    node_id        TEXT PRIMARY KEY,
    region         TEXT,
    capacity       BLOB,
    last_heartbeat INTEGER
);

CREATE TABLE policy_verdicts (
    scope_id       TEXT,
    key            BLOB,
    verdict        BLOB,
    compiled_at    INTEGER,
    PRIMARY KEY (scope_id, key)
);

SELECT crsql_as_crr('alloc_status');
SELECT crsql_as_crr('service_backends');
SELECT crsql_as_crr('node_health');
SELECT crsql_as_crr('policy_verdicts');
```

**Who writes.**
Every node writes its own rows (owner-writer model). Allocation status is written by the node that runs the allocation. Node health is written by the node itself. Compiled policy verdicts are written by the regional control plane leader after Regorus/WASM evaluation — the source policy is intent (Raft), but the evaluated output that nodes materialise into BPF maps is observation (Corrosion).

**Who reads.**
Every subsystem reads locally, with no gRPC round trip. The node agent subscribes to `service_backends`, `alloc_status`, and `policy_verdicts` and materialises BPF maps on change. The scheduler reads `node_health` to bin-pack. The gateway reads `service_backends` to resolve routes. The LLM observability agent correlates `alloc_status` transitions against telemetry.

**What it replaces.**
The earlier bare "Gossip / Health (SWIM)" component is gone. Corrosion is SWIM membership plus state propagation in one system — you do not run both. The gRPC push path from control plane to node agent for dataplane maps is also retired: the control plane writes verdicts into `policy_verdicts`, gossip carries them, node agents react to subscription events.

### Consistency Guardrails

CR-SQLite sacrifices strong ordering for availability. Helios enforces the boundary between intent and observation with compile-time discipline and runtime safeguards drawn directly from Fly.io's published post-mortems:

- **Type-level separation.** `IntentStore` and `ObservationStore` are distinct traits on distinct types. Nothing in the codebase can persist a job spec into Corrosion or an allocation heartbeat into Raft — the compiler rejects it. There is no shared `put(key, value)` surface that lets the wrong call go to the wrong place.
- **Identity-scoped writes.** A Corrosion peer only accepts writes whose CRDT site ID matches a live node SVID signed by the platform CA. A compromised node cannot forge rows on behalf of another node, and a decommissioned node's site ID is purged from the trust bundle.
- **Additive-only schema migrations.** Nullable column additions in CR-SQLite trigger cluster-wide backfill storms — Fly's most painful Corrosion incident. Helios schema migrations are strictly additive, versioned in the intent store, and gated through a two-phase rollout: new table first, readers cut over, old table drained, old table dropped. No `ALTER TABLE ADD COLUMN NULL` across the live fleet.
- **Full rows over field diffs.** Learning from Fly's post-mortem on partial updates, node agents republish the complete row for an allocation on every state transition rather than diffing fields. Late or reordered gossip converges deterministically under LWW; diff-merge logic does not.
- **Event-loop watchdogs.** Every subscription has a stall detector. A Corrosion peer whose event loop has not advanced within N seconds is killed and restarted before it can propagate stuck state — the bug class that contagion-deadlocked Fly's proxy fleet is a named DST scenario, not a hypothetical.
- **Per-region blast radius.** The global Corrosion topology is not a single flat cluster. Regional clusters gossip internally; a thin global membership cluster maps regions to coordinates. A runaway write in one region does not fan out globally in the same tick.

### Multi-Region Federation

The intent/observation split makes geographic federation a straightforward extension rather than a new architecture:

```
┌─ region: us-east-1 ────────────┐   ┌─ region: eu-west-1 ─────────────┐
│                                │   │                                 │
│  IntentStore (Raft, 3 nodes)   │   │  IntentStore (Raft, 3 nodes)    │
│    jobs, policies, certs       │   │    jobs, policies, certs        │
│    scoped to this region       │   │    scoped to this region        │
│                                │   │                                 │
│  ObservationStore (Corrosion) ◄┼───┼► ObservationStore (Corrosion)   │
│    alloc_status, service_*,    │   │    alloc_status, service_*,     │
│    node_health                 │   │    node_health                  │
│                                │   │                                 │
│  Node agents · Gateway · eBPF  │   │  Node agents · Gateway · eBPF   │
└────────────────────────────────┘   └─────────────────────────────────┘
                  ▲                                      ▲
                  └──── global Corrosion membership ─────┘
                        (region metadata only; no jobs)
```

Each region is operationally autonomous. Control plane decisions are made by the regional Raft cluster; they do not wait on consensus across an ocean. Routing, service discovery, and health converge globally through Corrosion at gossip latency (seconds), which is well below the rate at which routing decisions need to react.

Under a region-to-region partition each region continues to operate on locally-committed intent, serve locally-running workloads, and write to its local observation store. When the partition heals the Corrosion tables converge via LWW. Intent does not need to converge — it was never shared.

Cross-region service discovery works because every node reads `service_backends` locally, and the gossip tables are populated by every region. A gateway in Tokyo resolving `job/payments` sees backends in `us-east-1` and `eu-west-1` in its local SQLite, and the dataplane's XDP programs load-balance by whatever weighting the regional policy engines have compiled into `policy_verdicts`.

```toml
[cluster]
mode    = "ha"
region  = "eu-west-1"
peers   = ["node-2:7001", "node-3:7001"]

[cluster.observation]
corrosion_peers         = ["obs-1:8787", "obs-2:8787", "obs-3:8787"]
global_bootstrap        = ["global.helios.local:8787"]
rejoin_timeout_seconds  = 60
```

The same binary, the same role mechanic, one new line of configuration. Federation is not a separate product.

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

When a node runs both roles, control plane processes run in dedicated cgroups with kernel-enforced resource reservations. A misbehaving workload cannot starve the IntentStore, the Corrosion peer, or the scheduler regardless of how aggressively it consumes CPU or memory:

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

The root CA key lives in the IntentStore, encrypted at rest. In HA mode it is Raft-replicated across all control plane nodes within a region — CA material is deliberately never written to the eventually-consistent ObservationStore. Each node receives an intermediate CA certificate at bootstrap, signed by the root. The node agent issues short-lived leaf certificates (SVIDs, 1-hour TTL) for each workload it runs, using its intermediate.

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

- Registering with the regional control plane and writing live state (`alloc_status`, `node_health`, `service_backends`) into its local Corrosion ObservationStore
- Loading and managing eBPF programs via aya-rs
- Subscribing to Corrosion tables and materialising BPF maps (`SERVICE_MAP`, `IDENTITY_MAP`, `POLICY_MAP`, `FS_POLICY_MAP`) on row change — there is no gRPC push path for dataplane state
- Requesting and distributing workload SVIDs from the built-in CA
- Running workloads via the appropriate driver
- Collecting telemetry from the eBPF ringbuf and forwarding to DuckLake
- Responding to reconciler actions (start, stop, migrate, resize) — control-flow RPCs still arrive via gRPC streaming from the regional control plane

The agent is event-driven throughout. BPF ringbuf events push telemetry without polling. Observation changes arrive as SQLite subscription events from the local Corrosion peer. Intent-level reconciler instructions arrive via gRPC streaming. There are no periodic polling loops in the critical path.

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

### Persistent MicroVMs — Long-Lived Stateful Workloads

Not every workload is ephemeral. AI coding agents, CI runners, interactive development environments, Jupyter notebooks, and long-running data processing workers share a shape that neither stateless microVMs nor WASM functions serve well: they need a persistent filesystem, a stable addressable endpoint, and the ability to sleep and resume without losing state.

Helios handles this by extending the `microvm` driver with a `persistent` flag rather than introducing a new workload type. The Cloud Hypervisor substrate, the SPIFFE identity, the eBPF dataplane, the gateway, the credential proxy, and the WASM sidecars are already first-class — persistence is the missing ingredient.

```toml
[job]
name = "agent-claude-code"
driver = "microvm"

[job.microvm]
persistent = true
persistent_rootfs_size   = "100GB"
snapshot_on_idle_seconds = 30
expose                   = true   # auto-registers a gateway route
```

When `persistent = true`:

1. **Persistent rootfs bound to workload identity.** The rootfs is object-backed (Garage) with an NVMe hot-tier cache. The storage model follows the JuiceFS approach — content-addressed immutable chunks in object storage, metadata in fast local storage kept durable via streaming replication. The authoritative state lives in Garage; nodes cache chunks. The workload can migrate between nodes without moving a volume, and restores hydrate metadata only, not the full filesystem.
2. **Checkpoint/restore via Cloud Hypervisor.** The driver exposes `snapshot()` and `restore()` as control-plane actions that delegate to Cloud Hypervisor's existing snapshot/restore API with `userfaultfd` lazy memory paging — restore is pay-as-you-access, not load-everything-upfront. Disk snapshots are metadata-only against the chunk store (chunks are already immutable). Memory uses Cloud Hypervisor's native mechanism.
3. **Idle eviction with checkpoint (scale-to-zero).** After `snapshot_on_idle_seconds` of no traffic, the workload reconciler checkpoints the VM, tears down the running process, and records the handle in the ObservationStore. An inbound request via the gateway triggers a restore. The restore path is sub-second: metadata-only on disk plus `userfaultfd`-lazy on memory.
4. **VMGenID wired into the guest.** Live-migrated or snapshot-restored VMs face entropy-reuse hazards — the kernel's RNG can produce identical output on both sides of a snapshot. Cloud Hypervisor exposes a VMGenID device; the node agent updates the generation counter on every restore, and the guest kernel reseeds.

### Composition, Not a New Workload Type

A persistent microVM that also wants a stable public URL, credential sandboxing, and structural prompt-injection defense composes this from existing primitives:

- **Gateway route (§11)** — automatically registered for persistent microVMs declaring `expose = true`, providing a per-workload URL routed through the in-process gateway.
- **Credential proxy sidecar (§8, §9)** — automatically attached for persistent workloads that declare external credentials, ensuring the workload never holds real secrets regardless of what it runs.
- **Content-inspector sidecar (§9)** — optional; relevant for workloads processing untrusted content (AI agents reading the web, CI runners fetching third-party packages).

The principle: persistent microVMs are not a separate concept. They are the `microvm` driver with `persistent = true`, and the other capabilities stateful workloads typically need are already first-class in Helios and compose via the job spec. The platform does not ship pre-baked workload images (Python, Node, Claude Code) — that is a product decision, not a platform primitive.

### Use Cases

| Use case | Example | Why persistent microVM |
|---|---|---|
| AI coding agents | Claude Code, Cursor, Devin-style long sessions | State accumulates across turns; fast resume matters |
| CI runners | Self-hosted Buildkite / GitLab runners | Warm state (layer cache, artifacts) |
| Interactive dev environments | Codespaces-style, remote Jupyter | User filesystem persists; instant resume expected |
| Long-running data workers | Pipeline orchestrators with recovery state | State is the workload |
| Customer-code sandboxes | SaaS offering arbitrary user code execution | Per-tenant isolation + persistent scratch |

This is a natural consequence of the existing microVM driver, object storage, gateway, and identity model — not a separate product inside Helios.

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

All eBPF programs are readers of BPF maps. The node agent is the only writer. Policy is evaluated once, in the regional control plane, then propagated to every node as observation — compiled verdicts in Corrosion, read locally, materialised into BPF maps:

```
Intent (Raft, per region)
    │
Control Plane (Regorus / WASM evaluates policy → compiled verdicts)
    │ writes into ObservationStore.policy_verdicts
    ▼
Corrosion (CR-SQLite, SWIM-gossiped to every node over QUIC)
    │ SQLite subscription event on local peer
    ▼
Node Agent (writes BPF maps via aya-rs)
    │
    ▼
BPF Maps: POLICY_MAP · SERVICE_MAP · IDENTITY_MAP · FS_POLICY_MAP
    │ read-only
    ▼
XDP · TC · sockops · BPF LSM programs
```

Policy *source* lives in the IntentStore — auditable, versioned, strongly consistent. Policy *verdicts* flow through the ObservationStore — eventually consistent within seconds, readable locally on every node without a gRPC round trip. Service backend changes take the same path: a node brings up a new allocation, writes its row into `service_backends`, and within gossip-propagation time every node's XDP program is load-balancing across the new backend set.

Policy evaluation (Rego/Regorus) is never in the hot path. Per-connection enforcement is an O(1) BPF map lookup measured in nanoseconds.

### Comparison to Fly.io's Dataplane Model

Fly.io operates the largest production deployment of the Intent / Observation pattern Helios adopts in §4, and its dataplane is the most frequently cited reference for "Rust-native orchestrator with a userspace proxy." The comparison clarifies what Helios shares, what it diverges on, and why.

**Shared.** Both platforms use **Corrosion** (CR-SQLite + SWIM via Foca + QUIC, Apache-2.0) as the global eventually-consistent service catalog. Helios's ObservationStore is the same open-source component Fly runs in production — not a reimplementation. The regional-Raft + global-CRDT topology in §3.5 matches the operational shape Fly arrived at after years of trying to stretch Consul-style consensus across the WAN.

**Divergent — by design.** Fly routes east-west traffic over a **WireGuard mesh** backhaul with a userspace proxy (`fly-proxy`) handling TLS termination, load balancing, service catalog lookups, and request replay. Helios splits these concerns across two dataplanes:

| Concern | Fly.io | Helios |
|---|---|---|
| Packet-level load balancing | userspace `fly-proxy` | XDP SERVICE_MAP (in-kernel, nanosecond path) |
| Inter-node encryption | WireGuard peer-keyed tunnels | sockops + kTLS with per-workload SPIFFE SVIDs |
| Service identity | region + app + machine ID (`*.internal`, `*.flycast`) | cryptographic SPIFFE ID in every TLS session |
| L7 routing and TLS termination | `fly-proxy` userspace | Gateway subsystem (§11) — userspace |
| Request replay (`fly-replay` / `helios-replay`) | `fly-proxy` | Gateway subsystem + XDP loop counter (§11) |

The consequence is an end-to-end identity path in Helios: every east-west packet carries a per-workload SVID enforced at the socket layer, where Fly's WireGuard mesh enforces peer-level (machine-to-machine) trust and delegates per-workload authorization to the proxy layer. The trade-off is principled — Helios accepts the complexity of two dataplanes (XDP for the fast path, Gateway for L7) in exchange for nanosecond-scale LB and identity-bearing encryption without a per-hop userspace proxy.

This divergence is the answer to why Helios, given that it adopts Corrosion directly, is not simply a Fly rebuild: the service-catalog layer is shared; the dataplane is reimagined on primitives (stable eBPF, kTLS, SPIFFE) that did not exist when `fly-proxy` was designed.

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
    tls:         Arc<TlsManager>,    // internal CA + embedded ACME, unified rotation
}
```

Route updates are in-process state mutations. TLS certificate rotation — for both internal-trust SVIDs and public-trust ACME-issued certs — is handled by the same identity manager. Telemetry writes to the same DuckLake pipeline. Everything is coherent because it is the same binary.

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

- TLS 1.3 termination via rustls; certs issued by either the built-in CA (internal trust) or an embedded ACMEv2 client (public trust), with unified rotation through `IdentityMgr`
- HTTP/1.1, HTTP/2, gRPC, gRPC-Web, WebSocket
- Declarative route configuration pushed from control plane
- Composable middleware pipeline: rate limiting, JWT auth, CORS, circuit breaking, egress inspection
- In-process BPF map access for routing table updates — route changes are atomic, no restart

### Public-Trust Certificates

The built-in CA issues certs in the Helios trust domain — used for SVIDs, node intermediates, and the gateway's east-west mTLS. Generic internet clients (browsers, third-party SDKs, mobile apps) do not trust the Helios root, so public north-south ingress needs **publicly-trusted certs**.

Helios embeds [`instant-acme`](https://docs.rs/instant-acme) — a pure-Rust, rustls-native ACMEv2 client (RFC 8555) — directly in the gateway. Certs from Let's Encrypt or any ACMEv2-compliant CA feed into the same `IdentityMgr` that handles SVID rotation. Two trust lanes, one manager:

| Lane | Issuer | Clients | Use |
|---|---|---|---|
| Internal (east-west) | Built-in CA (§4) | Helios workloads, node agents, gateway east-west | Service mesh mTLS, SVIDs |
| Public (north-south) | ACMEv2 via `instant-acme` | Browsers, third-party clients | Gateway ingress on `https_port` |

Both lanes share `IdentityMgr` for storage and rotation, rustls as the TLS terminator, and the same reconciler-driven watchdog for certs approaching expiry.

```toml
[node.gateway.acme]
enabled       = true
directory_url = "https://acme-v02.api.letsencrypt.org/directory"
contact_email = "ops@example.com"
challenge     = "dns-01"    # "http-01" | "dns-01" | "tls-alpn-01"
dns_provider  = "route53"   # required when challenge = "dns-01"
```

Challenge support:
- **HTTP-01** — the gateway serves `/.well-known/acme-challenge/` on port 80 in-process; no external state
- **DNS-01** — required for wildcard certs (e.g. `*.workloads.example.com` covering per-workload URLs under one cert, §6 *Persistent MicroVMs*); pluggable DNS provider interface
- **TLS-ALPN-01** — gateway-local, port 443 only

Route configuration selects the cert source per host:

```toml
[[routes]]
host    = "api.example.com"
path    = "/payments/*"
backend = "job/payments"
tls     = "acme"            # "acme" | "internal" | "operator"
```

Storage boundary: operator-uploaded certs and ACME account keys live in the **IntentStore** (authoritative, linearizable, Raft-replicated in HA). Issued cert leaves and private keys live in the `IdentityMgr` cache alongside SVIDs — rotation is driven by the same reconciler that rotates workload identity.

`instant-acme` is maintained by the author set behind `rustls`, `rcgen`, `quinn`, and `hickory-dns` — the exact libraries Helios already depends on. It defaults to `aws-lc-rs` + `hyper-rustls` with `ring` as an alternative, offers an optional `rcgen` feature for CSR/keypair generation, and ships with explicit `RetryPolicy`, pluggable `HttpClient`, ACME Profiles, and ACME Renewal Information (ARI) support. The architectural consequence matters: **`IdentityMgr` uses one `rcgen`-based cert-generation path for both internal SVIDs and public-trust ACME certs** — no second TLS stack, no OpenSSL dependency pulled in transitively. Design principle 7 (*Rust throughout*) is preserved at full strength, not merely under the critical-path caveat.

### Route Configuration

Routes are declared as top-level platform resources and pushed to gateway nodes by the control plane — not embedded in job specs:

```toml
[[routes]]
host = "api.example.com"
path = "/payments/*"
backend = "job/payments"
tls    = "acme"
timeout_ms = 5000

[routes.middleware]
rate_limit = { rps = 1000, burst = 100 }
auth = { mode = "jwt", issuer = "https://auth.example.com" }
```

### External Traffic Path

```
External client (TLS via rustls; public-trust cert from ACME or operator upload)
    │
Gateway subsystem (hyper, in-process route engine, middleware)
    │ mTLS (SPIFFE identity, built-in CA, same IdentityMgr as all workloads)
    ▼
XDP dataplane (in-process BPF map lookup, DNAT)
    │
Backend service
```

The public TLS boundary terminates at the gateway. Inside the gateway, traffic is re-wrapped in mTLS using the built-in CA — every east-west hop carries cryptographic workload identity, exactly as if the request had originated inside the cluster. Two trust lanes meet at the gateway; from that point onward, everything is Helios-native identity.

### Declarative Request Replay

Applications frequently need to redirect an individual request to a different region, instance, or job — a write against a read-only regional replica belongs at the primary; a sticky session belongs on the canary allocation; a tenant-sharded request belongs on the shard that owns the tenant. Static route tables cannot express this; the choice depends on request content that only the application can inspect.

Helios exposes an application-driven replay primitive via a response header:

```
helios-replay: region=eu-west-1
helios-replay: instance=<alloc_id>
helios-replay: job=payments-primary
```

When a backend returns this header, the gateway reads it **before** streaming the body to the client, consults `service_backends` in the local ObservationStore for a backend matching the target, and re-issues the originally-buffered request via the XDP fast path to the new destination. The client sees a single response from the eventual backend. The original request body is held in a bounded buffer (≤1 MB) during the replay; requests whose body exceeds the buffer cannot be replayed and the header is honored on best effort.

Loop prevention is enforced in-kernel. A `helios-replay-count` header is incremented on every replay hop and a BPF map on the XDP fast path drops any replay whose counter exceeds a configurable ceiling (default 3). Loops that would otherwise consume multiple round-trips before a userspace check are extinguished at line rate.

Typical patterns:

- **Primary-region writes.** A read replica receiving a write request responds with `helios-replay: region=<primary>`; the gateway replays to the primary region's job. This composes with §3.5 Multi-Region Federation — each region reads its local ObservationStore for the primary's backend set.
- **Canary pinning.** A sticky session is pinned across canary promotion with `helios-replay: instance=<canary-alloc>` until promotion completes. Rollback remains a single SERVICE_MAP atomic update (§15) — once the canary allocation stops emitting the header, traffic follows the weighted backend set normally.
- **Tenant sharding.** A request whose tenant hash maps to a shard the local instance does not own is redirected with `helios-replay: instance=<shard-owner-alloc>`. The shard map itself is application state; the platform only carries the redirect primitive.

### Region Preference Hints

For cases where the routing preference is known at the *client* rather than the backend, Helios recognises two request headers:

- `helios-prefer-region: <region>` — bias backend selection toward the named region; fall back to other regions if unavailable.
- `helios-force-region: <region>` — require the named region; return 502 if no healthy backend exists there.

These hints are evaluated in the XDP fast path rather than at the userspace gateway. `service_backends` rows are keyed on `(service_id, region)`; the XDP program selects the matching subset before weighted load balancing. Happy-path cost is an additional BPF map lookup — no userspace hop, no TLS handshake overhead.

### Private Service VIPs and Auto-Wake

East-west traffic inside a Helios cluster addresses services by SPIFFE ID (`spiffe://helios.local/job/payments`) resolved via the local ObservationStore. For workloads that cannot carry SPIFFE identity natively (third-party SDKs, legacy clients, WASM runtimes without Helios-aware networking), Helios also exposes a stable per-service IPv6 VIP:

```
<job>.svc.helios.local  →  fdc2:<cluster>:<region>:<job-hash>::<N>
```

The VIP is allocated from a Helios-reserved ULA prefix (`fdc2::/16`). XDP SERVICE_MAP routes VIP traffic to the current backend set from `service_backends`, and the standard sockops layer wraps the connection in SPIFFE mTLS — the caller sees a plain IPv6 socket, the dataplane still enforces identity-bound encryption.

When no backend is in the `running` state — all allocations are `suspended` or `stopped` — XDP returns `XDP_PASS` to the node's local gateway subsystem. The gateway issues a resume via the proxy-triggered resume path (§14) and replays the buffered request once a backend becomes healthy. The VIP is therefore the natural target for scale-to-zero services: clients address a stable name; the platform brings the backend up transparently on first request.

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

### Scale-to-Zero for VM Workloads

Live hotplug right-sizing keeps *running* workloads matched to their actual demand. For workloads that sit idle between requests — interactive dev environments, cron-like batch runners, per-tenant sandboxes, review-app previews — the correct resource envelope between requests is zero.

Helios extends the `alloc_status` lifecycle with a `suspended` state and exposes scale-to-zero as a driver action across all VM-class workloads:

```
pending → running ⇄ suspended → terminated
```

When the idle-eviction reconciler marks an allocation for suspension:

1. Cloud Hypervisor's native snapshot API checkpoints VM memory to the object-backed rootfs chunk store (§6 *Persistent MicroVMs*). Disk state is already content-addressed and requires no additional write.
2. The node agent updates `alloc_status.state` to `suspended` and retains the allocation handle.
3. VM process memory is released; billing stops counting CPU/RAM against the allocation.

Resume is the inverse: Cloud Hypervisor `restore()` with `userfaultfd` lazy memory paging — pages materialise on access, not upfront. A VMGenID counter update on restore reseeds the guest kernel RNG to prevent entropy-reuse hazards across snapshot forks (§6 *Persistent MicroVMs*).

This composes with the WASM scale-to-zero pool (§16) — the mechanism differs per driver (Cloud Hypervisor snapshot/restore vs Wasmtime instantiation) but the control-plane contract is identical: `suspended` is a first-class allocation state, and the resume trigger is the gateway or the reconciler, not the workload itself. Process-driver workloads opt out — processes cannot be checkpointed safely without userspace cooperation; they remain running or terminate.

### Proxy-Triggered Resume

Scale-to-zero is only useful if something wakes the workload on demand. Helios wires this through the gateway and XDP fast path:

```
Request arrives at Gateway (or Private Service VIP, §11)
    │
XDP SERVICE_MAP lookup
    │
    ├── backend is `running`     → forward normally (nanosecond path)
    │
    └── all backends `suspended` → XDP_PASS to local gateway subsystem
            │
            ▼
        Gateway buffers request (≤1 MB)
            │
            ├── backend on same node  → in-process resume call
            └── backend elsewhere     → write alloc_status.requested_state = 'running'
                                         into ObservationStore; owner node agent
                                         observes the change via SQL subscription
            │
            ▼
        Node agent issues vm.resume via Cloud Hypervisor API
            │ (tens of ms for CH restore; ~1 ms for WASM instantiation)
            ▼
        alloc_status.state → 'running'; service_backends row re-weights
            │
            ▼
        Gateway replays buffered request via XDP → now-running backend
```

The resume path is identical in shape to the declarative replay primitive (§11) — request is held, destination is resolved, request is re-issued via the same XDP fast path. The only new state is the `suspended → running` transition, which is a single ObservationStore row update that the owning node's agent subscribes to directly.

Requests whose body exceeds the 1 MB buffer cannot be held across a cold resume. For these, the gateway responds immediately with 503 and a `Retry-After` hint derived from the expected restore latency — the client retry arrives once the backend is up.

### Deterministic Scale Rules

The predictive scaler above identifies patterns and proposes cron-based resource schedules — effective for traffic whose shape is learnable over days or weeks. For workloads driven by *current* signal — queue depth, inflight requests, CPU utilisation above a threshold — Helios also supports rule-based scale-out expressed in Rego:

```rego
# Scale worker pool by queue depth
scale_target {
    input.service == "ingestion-worker"
    desired := min(50, input.queue_depth / 2)
    desired > input.current_replicas
}
```

Rules evaluate against ObservationStore metrics on a fixed cadence (default 15 s). Output writes the desired replica count into the IntentStore; the job-lifecycle reconciler picks it up through the normal convergence path. Rule-based and LLM-based scalers are complementary — rules cover deterministic, short-horizon signals; the LLM covers pattern-based, long-horizon predictions. A job can use either or both.

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

Different data shapes require different storage primitives. Helios uses purpose-fit storage at each layer, with a hard boundary between *intent* (linearizable) and *observation* (eventually consistent) as established in §4:

### Control Plane Intent — IntentStore (mode-dependent)

Hot authoritative metadata: job specs, policies, certificates, scheduler allocation decisions. Requires linearizability. The active implementation depends on deployment mode:

- **Single mode** — redb direct. ACID transactions, no Raft overhead, ~30MB RAM. Right-sized for a single server without paying for distributed consensus that provides no benefit.
- **HA mode** — openraft + redb. Linearizable via the Raft log, replicated across 3 or 5 control plane nodes, ~80MB RAM. Per-region: a multi-region deployment runs one IntentStore per region.

Both implementations are pure Rust, embedded, and require no separate process. The rest of the platform is unaware of which is active. Migration from single to HA is non-destructive — both share the same snapshot format.

### Live Cluster Map — ObservationStore (Corrosion)

Live operational state: allocation status, service backend endpoints, node health, compiled policy verdicts, resource profiles. Strong consistency is unnecessary here and actively harmful — it cannot scale geographically, and the cost of Raft latency on the hot dataplane hydration path is unjustified when seconds of staleness is acceptable.

Helios uses **Corrosion** (Fly.io, AGPL/Rust) backed by **cr-sqlite** (Vlcn, MIT). Each node runs a Corrosion peer with a local SQLite file. CR-SQLite converts tagged tables into CRDTs with last-write-wins semantics under logical timestamps; peers gossip row changes over QUIC via a SWIM membership protocol.

```
Per-node footprint:
  SQLite file     ~50–500MB  (full cluster observation state)
  Corrosion peer  ~15MB RAM  (QUIC endpoint + gossip engine)
  Read path       local SQL  (no RPC, no network)
  Write path      local SQL + gossip fan-out
```

The store is global in multi-region deployments, with the regional blast-radius limits described in §4. The full Intent / Observation rationale, the schema, and the consistency guardrails live in §4 — this section lists ObservationStore as a storage layer; §4 describes how it is used.

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

Each reconciler gets a private libSQL database for stateful memory across reconciliation cycles — restart tracking, placement history, resource sample accumulation. Reconciler DB writes are strictly private; cluster mutations always route through a typed store — the IntentStore for intent, the ObservationStore for observation — never through the reconciler's private DB. The three consistency models (private libSQL, linearizable Raft, eventually-consistent CR-SQLite) never mix.

---

## 18. Reconciler Model

### Design

Helios' reconciliation model is inspired by Kubernetes' control loop but with three key differences: reconcilers are strongly typed Rust trait objects (not Go processes with cluster-admin privileges), they have access to a private persistent store for stateful reasoning, and the platform ships a durable-workflow primitive alongside the reconciler for multi-step operations that cannot be cleanly expressed as diff-based convergence.

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

The `reconcile` function is pure over `(desired, actual, db) → actions`. Neither the trigger reason nor wall-clock time are inputs. This is the property that makes reconcilers testable in the simulation harness (§21) and tractable for formal verification (below).

### Triggering Model — Hybrid by Design

Every mature production orchestrator — Kubernetes, Nomad, KCP, Crossplane — converges on level-triggered reconciliation because it is the only pattern that survives missed events, crashes, and stale caches. Pure event-sourced orchestrators do not exist in production; the straw-man is always a hybrid in practice. Helios follows the same consensus with Nomad's concrete shape:

- **Edge-triggered at ingress.** External state changes (job submission, node heartbeat failure, policy update, cert approaching expiry) produce a typed `Evaluation` enqueued through Raft.
- **Level-triggered inside the reconciler.** Each `Evaluation` causes the responsible reconciler to recompute `desired vs actual → Vec<Action>` against the authoritative IntentStore. Missed or duplicated events do not lose state — the next evaluation sees the full current delta.

### Evaluation Broker — Storm-Proof Ingress

A naïve edge-triggered ingress amplifies correlated failures. Nomad documents the canonical failure mode: 500 flapping nodes × 20 allocations × 100 system jobs = 60,000 evaluations in a single heartbeat window. Without mitigation, this saturates Raft and the reconciler fleet — HashiCorp retrofitted a cancelable-eval-set after production incidents produced literal millions of evaluations.

Helios ships the mitigation natively rather than retrofitting after an incident:

- Evaluations are keyed by `(reconciler, target_resource)`. A second evaluation for the same key while one is pending moves the prior evaluation into a **cancelable set** processed by a reaper in bulk.
- Because reconciliation is idempotent, collapsing N pending evaluations for the same target into one is semantically free — the surviving evaluation sees the fully-converged delta anyway.
- Back-pressure is measured in evaluations-per-second per reconciler; sustained over-budget shedding raises a platform alert rather than silently degrading.

### Extension Model

First-party reconcilers are Rust trait objects — maximum performance, full type safety. Third-party reconcilers are WASM modules loaded at runtime — sandboxed, hot-reloadable, language-agnostic. The interface is identical; the execution backend differs.

Input and output types are fully serializable from day one, making the WASM migration path trivial.

This replaces the Kubernetes operator model, where extensions ship as Go binaries running with cluster-admin privileges — the single largest source of cluster-destabilizing incidents in production Kubernetes. A misbehaving WASM reconciler cannot escape its sandbox, cannot mutate state without going through Raft, and can be evicted or hot-reloaded without restarting a pod. WASM as the control-plane extensibility substrate is now industry consensus (Helm 4, Cosmonic Control, wasmCloud) — Helios is early, not fringe.

### Built-in Reconcilers

- Job lifecycle (start, stop, migrate, restart)
- Certificate rotation
- Resource right-sizing
- Rolling deployment strategies
- Canary promotion/rollback
- Node drain and replacement
- WASM function scaling
- Chaos engineering (deliberate fault injection for reliability testing)
- Workflow execution (see below)

### Workflow Reconciler — Durable Execution for Multi-Step Operations

Some orchestration operations are fundamentally sequential: "roll certificate through DNS propagation, wait for validation, swap trust anchor, verify all nodes accepted, retire old cert." These do not fit cleanly into diff-based convergence because the correct next action depends on the *history* and the *timing* of prior steps, not just the current delta. Encoding them as reconciler memory works but reproduces what Temporal and Restate call *durable execution* — poorly.

Helios therefore treats durable workflows as a first-class primitive, implemented *as* a built-in reconciler whose desired state is a workflow definition and whose memory is a replayable event journal:

```rust
trait Workflow {
    async fn run(&self, ctx: &WorkflowCtx) -> WorkflowResult;
}
```

- Each `await` point is a durable checkpoint written to libSQL. A crashed workflow resumes on any control plane node by replaying the journal.
- Cross-workflow coordination uses typed signals, not ad-hoc StateStore writes.
- Used internally for certificate lifecycle, multi-stage deployments, cross-region migrations, and human-in-the-loop staged rollouts.

The reconciler primitive handles "converge cluster toward spec." The workflow primitive handles "execute this defined sequence to completion with crash-safe resume." They compose: a deployment workflow emits actions that are realized by the job-lifecycle reconciler.

### Three-Layer State Taxonomy

Helios draws a hard boundary between three state layers, each with different consistency guarantees. The reconciler and workflow primitives read and write these layers with explicit rules:

| Layer | Primitive reads | Primitive writes | Store | Guarantee |
|---|---|---|---|---|
| Intent — what should be | yes | only via Raft actions | IntentStore (redb / openraft+redb) | Linearizable |
| Observation — what is | yes | never directly | ObservationStore (Corrosion / CRDT) | Eventually consistent |
| Memory — what happened | yes | yes, private | libSQL per primitive | Private to the primitive |

This boundary is load-bearing. Authoritative schedule decisions must go through Raft — CRDT state is correct for globe-spanning observation data but wrong for "this workload is definitely scheduled here." Private libSQL gives each primitive persistent memory for backoff counters, placement history, resource samples, and workflow journals without inflating the authoritative store. §4 and §17 specify the stores in detail; §18 specifies which primitive reads and writes each layer.

### Formal Verification Path

The typed Rust trait interface is deliberately aligned with the target shape of recent verified-controller research (USENIX OSDI '24 *Anvil*, built on Verus). The *Eventually Stable Reconciliation* property — progress (converges toward desired state) and stability (remains at desired state absent external change) — is specifiable for each built-in reconciler as a temporal-logic formula over the `reconcile` function's pre/post-state.

First-party reconcilers ship with ESR specifications. WASM extensions declare ESR preconditions that the runtime enforces at load time. This is the largest future-proofing investment available to the platform: no existing production orchestrator has controllers with mechanically checked liveness properties, and the ecosystem around reconciler verification has reached academic maturity faster than around any alternative primitive.

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
| Cluster state fan-out | etcd watch via kube-apiserver (central bottleneck) | Corrosion gossip: local SQLite on every node |
| Multi-region | Raft stretched across WAN, or federation plane | Per-region Raft + global CRDT gossip (Fly-proven) |

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

// Observation — Corrosion cannot run in simulation (real QUIC stack, real SWIM timers)
trait ObservationStore: Send + Sync {
    async fn read(&self, sql: &str, params: &[Value]) -> Result<Rows>;
    async fn write(&self, sql: &str, params: &[Value]) -> Result<()>;
    async fn subscribe(&self, sql: &str) -> Result<RowStream>;
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
| `IntentStore` | `LocalStore` / `RaftStore` | `LocalStore` (already pure Rust, no kernel) |
| `ObservationStore` | `CorrosionStore` (cr-sqlite + SWIM/QUIC) | `SimObservationStore` (in-memory LWW, injectable gossip delay, injectable partition) |

`SimDataplane` tracks policy and service state in memory and generates synthetic flow events — enough to test that the control plane correctly drives the dataplane without involving a real kernel. `SimDriver` can be configured to fail on start, crash after N operations, or consume exactly specified resources, making scheduler and reconciler logic fully testable without spawning real VMs. `SimObservationStore` implements the CR-SQLite LWW merge semantics in memory with a controllable gossip delay and partition matrix — the contagion-deadlock failure mode that hit Fly.io's fleet is a named scenario in the DST fault catalogue.

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
                clock:       Arc::new(SimClock::new()),
                transport:   Arc::new(SimTransport::new()),
                entropy:     Arc::new(SeededEntropy::new(42)),
                dataplane:   Arc::new(SimDataplane::new()),
                driver:      Arc::new(SimDriver::new()),
                intent:      IntentMode::Ha,
                observation: Arc::new(SimObservationStore::new(
                    GossipProfile::realistic(), // injectable delay + partition
                )),
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
assert_always!("intent never crosses into observation",
    corrosion.tables().all(|t| !t.contains_intent_class())
);
assert_always!("observation never crosses into intent",
    raft.keys().all(|k| !k.starts_with("alloc-status/"))
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

Observation: Corrosion gossip stalled, LWW clock skew across peers,
             peer event-loop deadlock (Fly-style contagion scenario),
             region-to-region partition with independent writes,
             schema-migration backfill storm

Workloads:   driver fails to start, workload OOM, restart loop,
             workload consumes all node CPU

Control plane: leader crash during job submission, leader crash
               during cert rotation, reconciler panic,
               policy evaluation timeout
```

This catalogue also drives the chaos engineering reconciler in production — the same failure modes exercised in simulation are injected deliberately in live clusters to validate self-healing behavior.

### The Store Abstractions Are Already Correct

Both `IntentStore` (with `export_snapshot` / `bootstrap_from`) and `ObservationStore` (with `read` / `write` / `subscribe`) are the right shapes for DST. Simulation tests use `LocalStore` + `SimObservationStore` with a `SimClock` — single-node, no Raft complexity, no real QUIC, fully deterministic. `RaftStore` is added only to tests that specifically exercise consensus behavior; the real `CorrosionStore` is exercised by cross-region tests that need the actual SWIM/LWW semantics. The four store modes (single-region intent, HA intent, sim observation, real Corrosion observation) are independently composable and each is exercised continuously rather than only at boundary events.

### Antithesis

For exhaustive state-space exploration beyond what turmoil covers, Helios is designed to be compatible with Antithesis — a deterministic hypervisor that runs regular software in a fully reproducible environment. Antithesis has a native Rust SDK. The property assertions defined for turmoil tests map directly to Antithesis assertions, making the two approaches complementary: turmoil for fast in-process tests during development, Antithesis for deep exploration against the real binary in CI.

---

## 22. Roadmap

### Phase 1 — Foundation (Months 1–3)
- Core data model (Job, Node, Allocation, Policy)
- Control plane API (tonic/gRPC — internal node-agent + CLI transport)
- IntentStore abstraction: LocalStore (redb direct) for single mode
- ObservationStore abstraction: single-process in-memory implementation (lays the trait boundary early, swapped for Corrosion in Phase 2)
- Injectable Clock, Transport, Entropy, Dataplane, Driver, ObservationStore traits
- turmoil simulation harness + SimDriver / SimDataplane / SimClock / SimObservationStore
- Process driver
- Basic scheduler (first-fit)
- CLI (`helios job submit`, `helios node list`, `helios alloc status`)
- Image Factory MVP: `meta-helios` Yocto layer, `helios-image-factory` Rust service (schematic store, artifact cache, HTTP download frontend)

### Phase 2 — Networking and Observation (Months 3–6)
- aya-rs eBPF scaffolding
- XDP routing and service load balancing
- TC egress control
- RaftStore (openraft + redb) for HA mode + single → HA migration
- **CorrosionStore — production ObservationStore backed by Corrosion + cr-sqlite**
- **Corrosion schema: `alloc_status`, `service_backends`, `node_health`, `policy_verdicts`**
- BPF map hydration via Corrosion subscriptions (retires the gRPC push path for dataplane state)
- Node-identity-scoped write authorisation on Corrosion peers
- Additive-only schema migration tooling (avoids the Fly backfill-storm failure mode)

### Phase 3 — Identity and Security (Months 6–9)
- Built-in CA (rcgen + rustls)
- SPIFFE SVID issuance and rotation
- sockops mTLS + kTLS installation
- BPF LSM programs
- Regorus policy evaluation (intent), verdict compilation into ObservationStore

### Phase 4 — Additional Drivers (Months 9–12)
- Cloud Hypervisor microVM and VM driver (replaces Firecracker + QEMU)
- virtiofsd lifecycle management and cross-workload volume sharing
- WASM serverless driver (Wasmtime)
- WASM sidecar runtime + TC eBPF interception generalisation
- Built-in sidecars: credential-proxy, content-inspector, rate-limiter, request-logger
- Sidecar SDK (Rust + TypeScript)
- Gateway (hyper + rustls)
- Embedded ACMEv2 client via `instant-acme` (rustls-native, `rcgen`-integrated) — public-trust certs for the gateway (HTTP-01, DNS-01, TLS-ALPN-01), rotation unified with SVIDs in `IdentityMgr` on a single cert-generation path
- DuckLake telemetry pipeline (catalog: libSQL, storage: Garage Parquet)

### Phase 5 — Intelligence (Months 12–18)
- LLM observability agent (rig-rs)
- Self-healing tier 3 (LLM reasoning)
- Right-sizing reconciler (writes resource profiles into ObservationStore)
- Incident memory (libSQL)
- Predictive scaling
- Persistent microVMs (step 1): Cloud Hypervisor snapshot/restore exposed in the `microvm` driver with `userfaultfd` lazy memory paging; VMGenID wired into the guest on restore
- Persistent microVMs (step 2): object-backed rootfs (chunked over Garage) with NVMe hot-tier cache
- Persistent microVMs (step 3): gateway auto-route (`expose = true`) + credential-proxy sidecar defaults
- Persistent microVMs (step 4): idle-eviction reconciler with checkpoint (`snapshot_on_idle_seconds`) — scale-to-zero for long-lived stateful workloads

### Phase 6 — Federation and Ecosystem (Months 18+)
- WASM Component Model SDK (Rust, TypeScript, Go)
- OTel export adapter
- Unikernel drivers (Nanos, Unikraft with virtiofs)
- QEMU opt-in driver (exotic hardware emulation only)
- **Multi-region federation: per-region IntentStore (Raft) + global ObservationStore (Corrosion)**
- **Regional Corrosion clusters + thin global membership cluster (regionalized blast radius from day one — the lesson Fly learned mid-incident)**
- **Region-aware scheduler + gateway (reads `node_health.region` from local SQLite)**
- Cross-region partition tolerance: each region continues to operate on locally-committed intent under partition; observation converges via LWW on heal
- Image Factory: OCI registry frontend, PXE boot, dm-verity + TPM attestation, Secure Boot signing

---

## 23. Image Factory

Helios nodes run an immutable, purpose-built OS — no shell, no package manager, no SSH. This is not a constraint to work around; it is a deliberate security choice. Every component on the node is explicitly declared, compiled with hardening flags, and verified at boot. The Image Factory is the system that makes this tractable: it manages how node OS images are built, customized, versioned, and distributed.

### The Problem

Provisioning a Kubernetes node means installing a general-purpose Linux distribution and then running configuration management over it. The attack surface is whatever the distro ships. Security is whatever the configuration management enforced, subject to drift.

Helios takes the opposite approach: the OS is minimal by construction, not by configuration. The Image Factory is how operators get from "I need a node image" to a bit-for-bit reproducible artifact they can verify and trust.

### Design

The Image Factory is two things:

1. **`meta-helios`** — a Yocto layer that produces the node OS. It defines every package, every kernel config flag, every compiler flag. The output is a ~50 MB image: systemd, the Helios binary, Cloud Hypervisor, Wasmtime, and nothing else.

2. **`helios-image-factory`** — a Rust service that wraps the Yocto build system behind an HTTP API, manages content-addressable image IDs, and caches artifacts in an OCI-compatible store.

The factory service is thin. The heavy lifting — OS assembly, kernel compilation, SBOM generation — happens in Yocto. The service coordinates, caches, and serves.

### Why Yocto

Helios has non-trivial kernel requirements that cannot be satisfied by OCI layer assembly or a stock distribution:

- `CONFIG_BPF_LSM=y` — required for BPF LSM MAC (kernel 5.7+)
- `CONFIG_TLS=y` — required for kTLS and sockops mTLS
- `CONFIG_KVM=y` / `CONFIG_VHOST_VSOCK=y` — required for Cloud Hypervisor
- `CONFIG_BPF_SYSCALL=y` — required for aya-rs eBPF programs

Yocto's `defconfig` + `security.cfg` fragment model gives precise, auditable control over every kernel option. Every installed package is an explicit BitBake recipe. `inherit create-spdx` produces a machine-readable SPDX SBOM for every build. This aligns directly with the "own your primitives" principle — there is no hidden package manager, no transitive dependency that snuck in through an Alpine apk.

Build times are 60–90 minutes cold, ~5 minutes with a warm S3 sstate cache. For a factory service, this is acceptable: all official `(schematic_id, helios_version, arch)` tuples are pre-built at release time and served from cache. Operators waiting for a custom build are the exception.

### Schematics

A **schematic** is a TOML document whose SHA-256 hash is the image ID. Identical schematics always produce the same ID. The empty schematic — a base Helios node with all defaults — has a fixed well-known ID.

```toml
[node]
role = "worker"   # "control-plane" | "worker" | "control-plane+worker"

[drivers]
process   = true
microvm   = true    # Cloud Hypervisor
unikernel = false   # Unikraft (optional, increases image size)
wasm      = true    # Wasmtime

[kernel]
extra_args = ["intel_iommu=on", "iommu=pt"]

[extensions]
official = ["nvidia-gpu"]   # resolved to versioned recipes by factory

[security]
bpf_lsm = true   # locked true in production; configurable for dev only
ktls    = true
```

The `role` field in the schematic maps directly to the `[node] role` declaration in the Helios binary — the same binary handles all roles, and the schematic makes that explicit at image build time.

### Profiles

A **profile** combines a schematic with a Helios version, architecture, and output type:

```
Profile = (schematic_id, helios_version, arch, output_type)

output_type:
  raw.wic.gz    bare metal disk image (GPT: EFI + rootfs + verity)
  rootfs.ext4   VM rootfs for Cloud Hypervisor or PXE
  vmlinuz       bare kernel
  initramfs.xz  bare initramfs
  oci           OCI image for in-place upgrades via registry pull
```

Every profile tuple maps to exactly one artifact. The factory stores artifacts content-addressed in an OCI-compatible registry:

```
registry.helios.io/images/helios-node/{schematic_id}/{helios_version}/{arch}/
  raw.wic.gz
  rootfs.ext4
  vmlinuz
  initramfs.xz
  sbom.spdx.json
```

### API

```
POST   /v1/schematics                              → { id: SchematicId }
GET    /v1/schematics/{id}                         → schematic TOML

GET    /v1/versions                                → [ "0.1.0", ... ]

GET    /v1/image/{id}/{version}/{arch}/raw.wic.gz  → streamed or 202 + poll
GET    /v1/image/{id}/{version}/{arch}/rootfs.ext4
GET    /v1/image/{id}/{version}/{arch}/vmlinuz
GET    /v1/image/{id}/{version}/{arch}/initramfs.xz
GET    /v1/image/{id}/{version}/{arch}/sbom.spdx.json

GET    /v1/builds/{build_id}                       → { status, progress }

# OCI Distribution Spec v2 — standard registry interface for upgrades
GET    /v2/{name}/manifests/{reference}
GET    /v2/{name}/blobs/{digest}
```

Requests for cached artifacts are served immediately. Cache misses trigger an async Yocto build — the response is `202 Accepted` with a build ID to poll.

### `meta-helios` Layer

The Yocto layer is a direct evolution of the `meta-opencapsule` pattern, with three additions:

1. **Helios binary** via `inherit cargo_bin` + `meta-rust-bin` (prebuilt toolchain, single Cargo workspace)
2. **Workload driver binaries** — Cloud Hypervisor (`cloud-hypervisor`), Wasmtime runtime (`wasmtime`), optional Unikraft tools
3. **Kernel config fragments** — BPF LSM, kTLS, KVM, vhost-vsock additions to the base security config

```
meta-helios/
  conf/machine/
    helios-node-x86_64.conf      # bzImage, EFI_PROVIDER=grub-efi, wic+ext4
    helios-node-aarch64.conf
  recipes-core/images/
    helios-node-image.bb         # inherits core-image, helios-hardening, create-spdx
  recipes-helios/helios/
    helios_git.bb                # inherit cargo_bin; single binary, all roles
  recipes-drivers/
    cloud-hypervisor/            # microVM driver
    wasmtime/                    # WASM driver
    unikraft/                    # optional unikernel driver
  recipes-kernel/linux/
    linux-yocto_%.bbappend       # kernel 6.x, defconfig + security.cfg
  classes/
    helios-hardening.bbclass     # RELRO/NOW, stack protector, -D_FORTIFY_SOURCE=2
  wic/
    helios-node.wks              # GPT: EFI + rootfs (+ verity hash partition, Phase 2)
```

The image has no shells, no package manager, no SSH server, no getty. Post-processing strips debug tooling. The only user-facing entry point is the Helios binary managed by systemd.

### Node Upgrade Path

Upgrades are handled by the OCI registry frontend. A node running Helios can pull a new image as an OCI artifact, verify its digest against the schematic ID, write it to the inactive partition, and reboot into the new image — the same pattern as Talos upgrades, without requiring an external upgrade tool. This is Phase 2; Phase 1 upgrades are re-provisioning from a new image.

---

## Conclusion

Helios is not a Kubernetes improvement. It is a clean-slate design that leverages a set of primitives — stable eBPF APIs, Rust systems libraries, WASM runtimes, kernel TLS — that simply did not exist at production quality when Kubernetes was designed.

The result is a platform that is structurally more efficient, more secure, and more observable than any existing orchestrator, while supporting a broader range of workload types under a unified operational model.

The core insight is that eBPF is not a feature to add to an orchestrator. It is the right foundation for one. When the dataplane, the security model, the telemetry pipeline, and the service mesh all emerge from the same kernel primitive with the same workload identity attached, the platform is coherent in a way that bolted-on approaches cannot match.

---

*Helios is open source under the AGPL-3.0 license.*
*Contributions, feedback, and discussion welcome.*
