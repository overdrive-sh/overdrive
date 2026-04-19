# Should Sprites.dev-Style Persistent Sandboxes Be a Primitive in Helios?

**Date**: 2026-04-19 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 24

## Executive Summary

**Recommendation**: **Build it — narrowly — by extending the `microvm` driver and composing a new `agent_workload` profile from existing primitives. Do not ship a new driver and do not use the "sprite" name.**

Fly.io's sprites.dev (launched January 2026) is a Firecracker-microVM-based persistent sandbox platform targeting AI coding agents like Claude Code. Its technical innovations are not in the hypervisor layer — sprites *"are still Fly Machines"* [1] — but in the storage stack (JuiceFS-like chunked object storage with NVMe cache), sub-second checkpoint/restore (metadata-only CoW), per-instance public URLs via Anycast+Corrosion, and scale-to-zero with indefinite persistence. Comparable offerings in the same category: E2B (open-source Firecracker, session-scoped), Modal (gVisor userspace checkpoint/restore, 2.5× faster cold start), Cloudflare Sandbox SDK (container-in-VM via Durable Objects), and Cloudflare Dynamic Workers (V8 isolates, millisecond starts for AI-generated JS/TS).

Helios already has every building block needed: Cloud Hypervisor has snapshot/restore with `userfaultfd` lazy-paging parity with Firecracker [20][21]; the gateway can route per-workload URLs; SPIFFE + credential proxy + WASM sidecars together provide a genuine differentiation wedge (structural prompt-injection defense) that no sandbox competitor ships at the infrastructure layer. The work is integration, not invention.

Three reasons to do it: (1) The AI-agent execution substrate is a strategically important category — sprites, E2B, Modal, and Cloudflare all launched products here between 2025 and 2026. (2) Helios would compose existing primitives rather than inventing new ones, which matches the "own your primitives" design principle without product-layer scope creep. (3) The credential-proxy + sidecar wedge is a unique positioning opportunity: platform-level prompt-injection defense that no competitor ships. Three risks to manage: Cloud Hypervisor snapshot hardening needs scale-testing parity with Firecracker, persistent per-workload storage breaks "cattle" orthodoxy and introduces migration/drain/GC classes of bugs, and multi-tenant arbitrary-code hosting requires VMGenID and the inner-container-inside-VM pattern Fly uses.

## Research Methodology

**Search Strategy**: Web searches across primary vendors (sprites.dev, fly.io, e2b.dev, modal.com, firecracker-microvm.github.io, kata-containers.io, criu.org, cloudflare.com), cross-reference with existing Helios whitepaper and prior research (`docs/research/infrastructure/fly-io-primitives-helios-relevance.md`). Direct fetch of docs and blog posts with adversarial-output validation.

**Source Selection**: Official vendor docs (high reputation), vendor engineering blogs (medium-high), GitHub repos (high — source of truth for OSS), conference talks (medium-high). Avoid undated marketing claims unless flagged.

**Quality Standards**: Target 3 sources/claim; 2 acceptable for vendor-specific claims with only one authoritative source; flag single-source claims. Cross-reference across independent vendors.

## 1. What sprites.dev Actually Provides

**Crucial framing fact**: Sprites is a **Fly.io product**, launched January 9, 2026 [1][2]. `sprites.dev` is its marketing front door; the implementation sits on top of Fly's existing Machines/Firecracker/Anycast stack with a new storage stack layered in. This materially changes how Helios should interpret the "primitive" question — we are not comparing against a fresh point-solution but against a well-funded extension of Fly's existing infrastructure.

### 1.1 Primitive Anatomy
**Evidence**: "Sprites execute code in Firecracker VMs" [3, sprites.dev marketing] — though Fly's own engineering post is more careful, stating sprites "are still Fly Machines" underneath [1]. Fly Machines are built on Firecracker, per Fly's prior public documentation [8], so the chain is: sprite → inner container → Linux VM → Firecracker microVM → bare metal host.

- Inner container layer: *"User code running on a Sprite isn't running in the root namespace. We've slid a container between you and the kernel."* [1] — This is a defense-in-depth layer that separates user code from Fly's in-VM services (storage, service manager, logs).
- Confidence: **High** (primary vendor source + cross-ref with Simon Willison analysis [2] + Northflank comparison [4]).

### 1.2 Persistence Model
**Evidence (from Fly's design post [1])**:
- *"The root of storage is S3-compatible object storage"* — every sprite has a 100 GB quota initially.
- *"A sparse 100 GB NVMe volume attached to it, which the stack uses to cache chunks. Nothing in that NVMe volume should matter; stored chunks are immutable."*
- *"Organized around the JuiceFS model — data chunks live on object stores; metadata lives in fast local storage... that metadata store is kept durable with Litestream."*
- User-facing filesystem is ext4 on top of this backing store [3].

This is not a per-host persistent ext4 volume — it is effectively a content-addressed chunk store (JuiceFS-style) backed by S3, with an NVMe hot-tier cache and metadata made durable via Litestream (SQLite streaming replication). This is significant: sprites can migrate between hosts because the authoritative state is in object storage, not on a specific node's disk.

**Confidence**: **High** (Fly's own engineering blog is the primary source and a high-authority channel).

### 1.3 Cold Start and Snapshot Semantics
**Evidence**:
- Checkpoint time: *"Checkpoints take about 300 ms and your Sprite environment won't even notice"* [3].
- Restore: *"Quick restores... you'll be reconnected in well under a second"* [3]; creation of a new sprite takes *"a second or two"* [5] versus "over a minute" for Fly Machines.
- Mechanism: *"Both checkpoint and restore merely shuffle metadata around"* [1]. *"Copy-on-write implementation for storage efficiency"*; checkpoints *"capture only the writable overlay, not the base image"* [2].
- Filesystem snapshotting is thus metadata-only against the JuiceFS-style chunk store (because chunks are immutable and content-addressed). This is cheap.
- Memory snapshots: community thread confirms "RAM doesn't persist across hibernation and wake" by default [6]; the engineering post says creation is sub-second, strongly implying Firecracker's native memory snapshot-and-restore feature [11] is used for warm resumption but not guaranteed across upgrades.

**Confidence**: **High** for disk/CoW semantics (3 independent sources agree); **Medium** for memory snapshot mechanism (partially inferred; Fly staff have not written a dedicated engineering post on it yet as of 2026-04).

### 1.4 Networking and Routing
**Evidence**:
- *"Each Sprite has a unique URL. Requests to your sprite URLs are proxied to whatever you have listening on port 8080 within the Sprite"* [3].
- *"VMs run on isolated networks. Nothing can connect to your Sprite directly"* [3].
- Integrates with *"Corrosion, our gossip-based service discovery system, enabling instant public URL provisioning with HTTPS termination at proxy edges"* [1].
- DNS-based egress policy: *"DNS-based network policies with allow/deny rules for domains like github.com or *.npmjs.org"* [2].

Corrosion is a CRDT-based service-discovery system Fly uses for their Anycast proxy (SQLite CRDT — same family Helios uses for its ObservationStore [whitepaper §17]). The per-sprite URL is routed through Fly's existing Anycast proxy edge.

**Confidence**: **High**.

### 1.5 Pricing and Target Use
**Evidence**:
- CPU: $0.07/CPU-hour, minimum 6.25% CPU/second [3].
- Memory: $0.04375/GB-hour, minimum 0.25 GB/second [3].
- Storage hot (NVMe cache): $0.000683/GB-hour; cold (object): $0.000027/GB-hour [3].
- Scale-to-zero: sleeps after 30 s of inactivity [2][4].
- Target use: *"A Sprite is a hardware-isolated execution environment for arbitrary code: a persistent Linux computer. Whether it's an AI agent like Claude Code or a binary your user just uploaded..."* [3].
- Ships with Claude Code, Python 3.13, Node.js 22.20, Gemini, Codex pre-installed [1][2].
- Use case positioning: *"ephemeral sandboxes are obsolete"* [5] — Fly's thesis is that AI agents need durable environments, not fresh sandboxes.

**Confidence**: **High**.

## 2. Ecosystem Landscape

### 2.1 Fly Machines (the substrate beneath Sprites)
- Firecracker microVMs; orchestrated by `flyd`; prior Helios research covers this in detail [see `docs/research/infrastructure/fly-io-primitives-helios-relevance.md`].
- Sprites are *"still Fly Machines"* [1] but with a new storage stack (JuiceFS-like object-backed), a container sandwich inside the VM, and tight integration with Fly's Corrosion service discovery and Anycast proxy.
- Per-machine persistent NVMe volumes have existed on Fly Machines for a long time; Sprites innovate by moving the authoritative state to object storage so sprites can migrate and restore faster.

### 2.2 E2B
- **Isolation**: Firecracker microVM per sandbox [7][9].
- **Snapshot**: Template-based. *"Entire VM state (filesystem + running processes) serialized and restored in ~150 ms"* [9]. Templates built from Dockerfiles via `e2b template build`, converted to Firecracker microVM snapshots [9].
- **Persistence**: Session-scoped up to 24 hours; auto-pause in beta [4]. Not indefinite like sprites.
- **Networking**: Proxy tunneling; custom domains via self-managed proxy [4].
- **OSS**: `e2b-dev/infra` repo is Apache-2.0 [7]. Written in Go (85%) + Terraform. Self-hostable on GCP (fully), AWS (beta), Azure (planned) [7][9].
- **Cold start**: ~150 ms for VM restore from template snapshot [9].
- **Confidence**: **High** (primary docs + vendor blog + third-party breakdown).

### 2.3 Modal Sandboxes
- **Isolation**: gVisor (`runsc`) — userspace kernel container runtime, not a microVM [10][13].
- **Snapshot**: gVisor's in-kernel checkpoint/restore, not CRIU. Eighteen components implement save/restore in `save_restore.go` [10]. *"Checkpoint captures the entire state of a Linux container right before it was about to accept a request"* — filesystem mutations + process tree + memory mappings + FDs + registers [10].
- **Restore performance**: ~2.5× faster than cold container startup. Stable Diffusion: 13 s cold → 3.5 s restored. `import torch`: 5 s → 1.05 s [10].
- **Lazy paging**: Yes — pages loaded in background, blocked pages prioritized, preloaded via FUSE into host page cache [10].
- **GPU state**: Excluded from snapshot, must be recreated post-restore [10].
- **Persistence**: Filesystem Snapshots available for > 24 h runs; Volumes and Dicts for data sharing [12].
- **Networking**: Tunnels (TCP), Proxies (beta), private networking [12].
- **Confidence**: **High** (official Modal engineering blog + gVisor docs).

### 2.4 Firecracker Upstream (AWS / OSS)
- **What Firecracker natively provides** [11]:
  - Snapshot captures *"guest memory, emulated HW state (both KVM and Firecracker emulated HW)"* as separate files.
  - Memory restore uses `MAP_PRIVATE` mapping of the memory file — *"on-demand loading of memory pages"* with CoW to anonymous memory. Resumed VM requires the memory file to be kept around for its lifetime.
  - Diff snapshots: **still in developer preview** (2026-04); not resume-able directly, must be merged with a base.
  - Restore latency is elevated on cgroups v1; v2 is recommended.
  - Network connectivity not guaranteed post-resume; VMGenID mitigates entropy reuse risks.
  - Threat model: *"host, host/API communication, and snapshot files are trusted"* — resuming the same snapshot many times creates RNG/entropy reuse risks.
- **Memory snapshot is native**, not requiring CRIU. This is a key reason Firecracker is the de-facto substrate for AI sandboxes (sprites, E2B).
- **Confidence**: **High** (official Firecracker docs).

### 2.5 Kata Containers / CRIU / runc
- **Kata**: OCI runtime that wraps containers in VMs (containerd shimv2). Supports multiple hypervisors: QEMU, Cloud Hypervisor, Firecracker, and Dragonball (built-in minimal VMM). Target: container-like UX with VM isolation for Kubernetes [14].
- **CRIU**: Checkpoint-restore tool for Linux processes. Hypervisor-free — freezes a process tree via `/proc` and `ptrace`, serializes to disk, restores. Originally built for OpenVZ/Parallels; now the canonical tool for `runc` checkpoint/restore [15, CRIU project; 16, Kubernetes forensic checkpointing blog].
- **gVisor native C/R**: Independent of CRIU; runs in userspace and is what Modal uses [10].
- **Confidence**: **High** for Kata and CRIU (official docs); relevant as comparison baselines but not direct sprite analogs.

### 2.6 Cloudflare: Sandbox SDK vs Dynamic Workers
Two distinct offerings (important — they serve different roles):
- **Sandbox SDK** (GA April 2026): Workers + Durable Objects + **Containers** (full Linux). Each sandbox = dedicated VM. Persistence via Durable Object identity routing [17][18]. Pre-installed Python/Node.js/Git [17].
- **Dynamic Workers** (beta April 2026): V8 isolate-based execution for AI-generated JS/TS. Millisecond starts, MB of memory, ~100× efficiency vs containers [19]. Not a Linux env — an isolate. Scales to millions of requests/sec with per-request sandboxes [19].
- **Confidence**: **High** (official Cloudflare blog + InfoQ report).

### 2.7 Comparison Table

| Platform | Isolation | Persistence | Cold start | Snapshot mechanism | HTTP routing | OSS |
|----------|-----------|-------------|-----------|-------------------|--------------|-----|
| sprites.dev (Fly) | Firecracker + inner container | Indefinite; object-backed (JuiceFS-model) with NVMe cache | ~1 s restore; 1–2 s create [1][3][5] | Metadata-only CoW on chunk store + Firecracker memory snapshot | Per-sprite public URL via Fly Anycast + Corrosion | No |
| Fly Machines | Firecracker | Per-machine NVMe volume | Seconds-to-minutes | Firecracker native | Per-machine via Anycast | No (ops); Firecracker is OSS |
| E2B | Firecracker | Session ≤24 h, auto-pause beta | ~150 ms from template | Firecracker native (template = pre-built snapshot) | Proxy tunnel | **Yes** (Apache-2.0) [7] |
| Modal Sandboxes | gVisor (`runsc`) | Volumes; Filesystem Snapshots for >24 h | 2.5× faster than cold; ~1 s for small procs | gVisor userspace C/R + lazy page-in | Tunnels / proxies | Partially (gVisor is OSS; Modal's control plane isn't) |
| Cloudflare Sandbox SDK | Container (in a VM) | Durable Object identity | Container startup (hundreds of ms) | N/A explicit | Via Workers | No |
| Cloudflare Dynamic Workers | V8 isolate | None (per-request) | Milliseconds | N/A | Via Workers | V8 is OSS; DW platform is not |
| Kata Containers | QEMU / CH / FC / Dragonball | Per-pod volumes (K8s) | Seconds | Varies by hypervisor | Via K8s service | **Yes** (Apache-2.0) |
| CRIU + runc | No VM isolation | Process state only | Sub-second possible | Linux kernel-cooperative | N/A | **Yes** |
| Helios `microvm` (today) | Cloud Hypervisor | Per-workload (whitepaper §6) | ~200 ms [whitepaper L1362 refs] | None shipped yet | Gateway (§11) | **Yes** |

## 3. Mapping onto Helios

### 3.1 What Helios Already Has

From the whitepaper:
- **Cloud Hypervisor microvm driver** (§6): fast boot (~200 ms), one process per VM, virtiofs-capable, CPU/memory hotplug.
- **CH snapshot/restore is available upstream** [20][21] — with `userfaultfd`-based on-demand memory paging that mirrors Firecracker's `MAP_PRIVATE` behaviour. As of v37 LTS (Feb 2025), JSON deserialization on restore is faster, and live migration parity is committed across LTS releases [21]. Helios currently does **not** use this feature; it is a dormant capability in the driver.
- **Gateway (§11)**: SPIFFE-addressable route resolution, L7 reverse proxy capability, sidecars. This is functionally comparable to Fly's Anycast+Corrosion per-sprite URL story, minus the public anycast.
- **SPIFFE + kernel mTLS (§8)**: every workload gets a cryptographic identity; dataplane enforces it.
- **BPF LSM (§7)**: mandatory access control at the kernel — blocks raw socket creation etc. even if the workload is compromised.
- **WASM Sidecars (§9)**: per-workload, ordered request interception chain — ideal for prompt-injection filtering on AI agent inputs/outputs.
- **Credential Proxy for AI Agents (§8)**: dummy credentials in the workload; real credentials held by the proxy; token binding prevents injection-driven auth. *This is a genuine differentiator* — neither sprites nor E2B/Modal provide this as a first-class construct.
- **virtiofs (§6)**: cross-workload volume sharing between containers and VMs.
- **WASM functions with instance pool (§16)**: the "ephemeral sandbox" end of the spectrum already exists.

### 3.2 Gaps vs sprites.dev semantics

| Sprite capability | Helios today | Gap size |
|-------------------|--------------|----------|
| Indefinite persistent rootfs per workload | Not a driver feature; Garage (S3) exists but isn't wired as a per-workload content-addressed backing store | **Medium–large** — needs a storage shim |
| CoW chunked object-backed filesystem (JuiceFS-model) | Garage is S3-compatible; no JuiceFS or equivalent layered on | **Medium** — integration work, not invention |
| ~300 ms checkpoint | CH has snapshot/restore [20][21]; not exposed to workload users | **Small** — plumbing |
| Sub-second restore with lazy memory paging | CH supports `userfaultfd` on-demand mode [20] | **Small** |
| Per-workload public HTTPS URL | Gateway can do it (§11) but sprites style requires low-friction auto-provisioning | **Small** |
| Scale-to-zero after 30 s idle with full state retention | Not a driver mode; would need reconciler support | **Medium** |
| Migration across hosts (state follows sprite) | IntentStore + ObservationStore support this conceptually; storage backing must be shared | **Medium** — depends on storage |
| Pre-installed dev tooling (Python/Node/Git/Claude Code) | Not in Helios scope; Image Factory produces node OS, not workload images | **Medium** — new responsibility |
| Long-lived interactive session (SSH, REPL, console) | Not a driver target; Helios focuses on background workloads | **Medium** — new UX |

### 3.3 Four Framings

#### (a) New workload driver (`sprite` or `persistent-microvm`)
- **Pro**: Clean semantics; a dedicated driver can encode "long-lived, stateful, interactive" as a first-class workload type. Users write `driver = "sprite"` and get the model.
- **Pro**: Opens the door to differentiated scheduling (node affinity, preemption semantics, idle eviction).
- **Con**: Duplicates 90% of the existing microvm driver. Two drivers that both spawn Cloud Hypervisor create maintenance debt.
- **Con**: Sprite is a Fly.io product name. Naming a Helios driver after a competitor's product is poor positioning.

#### (b) Extension of existing microvm driver (persistent flag + snapshot)
- **Pro**: Zero duplication. The microvm driver already runs Cloud Hypervisor; exposing snapshot/restore + a persistent rootfs mode is additive.
- **Pro**: Matches how Fly did it — *"they're still Fly Machines"* [1]. The engineering pattern is "same substrate, different knobs."
- **Pro**: Keeps the orthodoxy (§2 design principles): drivers are a small closed set, not an open menu.
- **Con**: Risks overloading the driver with agent-specific quirks (idle timeout, public URL auto-provision).
- **Verdict**: Most likely the correct base layer.

#### (c) New category: "agent workloads" (composition, not a new driver)
- **Pro**: Composes the existing primitives — microvm driver + persistent volume + auto-gateway route + credential proxy sidecar + prompt-injection inspector sidecar — into a named workload **profile**.
- **Pro**: This is `agent_workload = true` as a job-spec flag that unlocks idle-eviction semantics, auto-registered public route, and the AI-agent credential proxy by default.
- **Pro**: Encodes Helios's genuine differentiation (credential proxy + WASM sidecars for prompt injection) rather than just replicating sprites.
- **Con**: Needs the storage backing (3.2) to be real.

#### (d) Not a primitive — application layer on top
- **Pro**: "Own your primitives" (§2) does not mean "own every product." If Helios ships the building blocks (microvm driver + CH snapshots + persistent volumes + gateway + credential proxy), someone can build a sprites-like offering **on Helios** without Helios being the vendor.
- **Pro**: Avoids pricing/UX debt of running a public sandbox offering.
- **Con**: If the building blocks are missing (persistent volumes, CH snapshot surfacing), "build it on top" is a fiction.

**Synthesis**: (b) + (c) is the coherent answer. Extend the microvm driver with snapshot/restore and persistent-volume binding; add an `agent_workload` profile that composes the resulting building blocks with credential proxy and sidecar defaults. Do **not** ship a `sprite` driver.

## 4. The AI-Agent Execution Angle

### 4.1 Emerging Patterns (2025–2026)
The "sandbox for AI agents" space has crystallized around four archetypes [4][5][18][19]:

1. **Microservice-style Firecracker sandboxes** (E2B, sprites): Full Linux, hardware isolation, template-based snapshots. Used by Claude Code, Cursor, and similar.
2. **gVisor userspace-kernel sandboxes** (Modal): Faster-than-VM cold starts via in-kernel C/R, weaker isolation than microVM.
3. **Container-on-VM sandboxes** (Cloudflare Sandbox SDK): Durable Object identity routes to a full Linux container-in-VM. Persistence via DO.
4. **V8-isolate sandboxes** (Cloudflare Dynamic Workers): Sub-ms starts, MB memory, JS/TS-only. For AI-**generated** code rather than user-uploaded binaries [19].

All four are converging on: persistent identity, checkpoint-restore, per-instance URL, idle eviction. Sprites differ by: indefinite persistence + chunked object-backed storage that makes migration cheap.

### 4.2 What Helios Uniquely Brings
Helios has two infrastructure-level defenses that none of the above ships as a first-class primitive:

- **Credential Proxy (§8 — "Credential Proxy for AI Agents")**: dummy credentials in-workload; token-binding prevents prompt-injection-driven auth to allowed domains using an attacker's token. This is *structural* defense against prompt injection — not model-dependent.
- **WASM Sidecars (§9)**: ordered per-workload chain; `on_ingress` / `on_egress` hooks; block/modify/redirect actions. A "prompt injection content inspector" or "egress audit logger" is a configuration, not a service to stand up.

Neither E2B, Modal, nor sprites positions this layer. Fly has Anycast egress filtering (DNS-based allow/deny [2]) but not credential virtualization or semantic content inspection at the dataplane.

**Differentiation claim**: "Helios is the only agent sandbox platform where the credential layer and the content-inspection layer are part of the platform, not the agent." This is a genuine wedge, and it is additive to — not competing with — the persistent-sandbox value proposition.

### 4.3 Anthropic Computer-Use and Claude Code
Anthropic's computer-use and Claude Code both reference Firecracker-based sandboxes as an execution target [2][3]. Sprites explicitly targets Claude Code as a design partner [3]. If Helios wants to be an execution substrate for AI agent tooling, the bar is set by this pattern: microVM isolation, persistent disk, sub-second restore, per-instance URL. The recommendation below lands Helios at this bar while preserving the credential/sidecar wedge.

## 5. Trade-offs and Risks

### 5.1 Cloud Hypervisor vs Firecracker for Snapshots
Both support memory snapshot with on-demand paging (`MAP_PRIVATE` / `userfaultfd`) [11][20]. Performance is broadly comparable; Cloud Hypervisor v37 LTS focused explicitly on snapshot restore speed [21].

**Real distinctions**:
- Firecracker has **far more production scale-testing** in this exact use case (AWS Lambda, Fly Machines, E2B, sprites). CH has snapshot parity on paper but less public evidence of millions-per-day restore operations.
- Firecracker has a **stronger threat model** for untrusted code — minimal device set, 50 kLoC [13], explicit design goal. CH has a richer device set (virtiofs, hotplug, full VMs) which is valuable for Helios's multi-workload-type story but expands attack surface.
- Diff snapshots in Firecracker are still **in developer preview** as of 2026-04 [11]. CH has no diff-snapshot concept; must re-snapshot fully.

**Implication**: Helios's bet on CH is correct for the unified-VMM design (§6 whitepaper), but the AI-agent path will require investment in CH snapshot hardening and scale-testing that Firecracker has gotten "for free" from AWS. This is a real cost; do not hand-wave it.

### 5.2 Persistent Per-Workload Storage vs Cattle Orthodoxy
The Kubernetes/Nomad orthodoxy is "workloads are cattle" — any instance is replaceable. Sprites inverts this: every workload is a pet with its own persistent disk that follows it across hosts. Operational tensions:

- **Node drain is no longer free**: draining a node with 50 live sprites means migrating 50 chunk-backed filesystems (cheap, because object-backed) + 50 memory snapshots + 50 routing updates.
- **GC of dead sprites** becomes a thing: who owns the 100 GB disk of a sprite whose owner stopped paying?
- **Right-sizing (whitepaper §6)** assumes workloads can be resized via hotplug. For interactive sprites, hotplug is fine; for checkpointed sprites, resize requires restore-into-larger-VM — a new pattern.

The Fly design resolves this elegantly by moving authoritative state to object storage. Helios already has Garage (S3). The question is whether to layer a JuiceFS-like chunk-store on top of Garage for the sprite driver, or take a simpler approach (per-workload virtiofs volume stored in Garage via a snapshot-then-upload path).

**Risk**: If Helios accepts persistent per-workload storage without also accepting migration cost, operators will encounter a class of incidents (stuck drains, orphan volumes, split-brain after partition) that don't arise today. Ship the model explicitly or don't ship it.

### 5.3 Multi-Tenant Arbitrary-Code Isolation
Sprites runs *"arbitrary code... binary your user just uploaded"* [3]. Multi-tenant arbitrary code is the most hostile threat model there is. Helios's current stack for this:

- Cloud Hypervisor VM boundary (equivalent to Firecracker at the hypervisor level; both use KVM, both are Rust, both small).
- BPF LSM for mandatory access control.
- SPIFFE identity + kernel mTLS (cross-workload boundary).
- Credential proxy (limits damage of an escaped secret).

**What sprites adds on top**: the inner container inside the VM [1]. Helios would want an equivalent — *"slide a container between you and the kernel"*. This is not currently in the whitepaper as a design pattern for the microvm driver.

**Additional risks to flag**:
- Firecracker has known snapshot-entropy reuse concerns [11]. CH inherits the same class of problem. VMGenID must be wired into the Helios microvm driver before any snapshot-restore feature is exposed to untrusted workloads.
- Side-channel attacks (Spectre-class) across sprite tenants on a shared host: Firecracker and CH both rely on kernel mitigations. This is the same risk surface AWS Lambda accepts; Helios should match its hardening.
- `userfaultfd` has a history of LPE CVEs. Exposing it to workload-restore paths without careful auditing is risky.

## 6. Recommendation

### 6.1 Opinionated Answer
**Build it — but narrowly, and not as a new driver.**

Specifically:

1. **Do not ship a `sprite` driver.** Do not ship anything called "sprite". That is Fly.io's product name and the Helios positioning should be its own.
2. **Extend the `microvm` driver** with two capabilities:
   - **Persistent rootfs bound to the workload identity**, stored object-backed (Garage) with an NVMe hot-tier cache, using a JuiceFS-style chunk layer (or a simpler virtiofs-over-Garage first pass).
   - **Checkpoint/restore via Cloud Hypervisor's existing snapshot API**, with `userfaultfd` lazy memory paging. Expose as `workload.snapshot()` control-plane action.
3. **Introduce an `agent_workload` profile** (not a driver) in the job spec that composes:
   - `driver = "microvm"` with persistent rootfs + snapshot-enabled.
   - Auto-registered gateway route (per-workload public URL).
   - Idle-eviction-with-checkpoint after N seconds of no requests.
   - Credential proxy sidecar auto-enabled.
   - (Optional) prompt-injection-inspector sidecar slot.
   - VMGenID wired into the microvm driver (entropy reuse mitigation).
4. **Do not ship pre-baked workload images (Python 3.x, Node.x, Claude Code pre-installed).** That is a product decision, not a platform primitive. Ship the primitive; let downstream vendors ship the product.
5. **Phase order**:
   - Phase 1: CH snapshot/restore exposed in the microvm driver (no persistent storage). Non-idle workloads only. Prove the snapshot path.
   - Phase 2: Object-backed persistent rootfs (virtiofs → Garage chunks). Still no auto-URL.
   - Phase 3: `agent_workload` profile composing the above + gateway auto-route + credential proxy defaults.
   - Phase 4: Idle-eviction with checkpoint, scale-to-zero semantics. Requires reconciler work.

### 6.2 Three Key Reasons
1. **The building blocks already exist in Helios**; the work is integration, not invention. CH snapshot/restore + Garage + gateway + SPIFFE + credential proxy + WASM sidecars compose naturally into the sprites-shaped capability without a new driver.
2. **The AI-agent execution substrate is a strategically important category** (sprites, E2B, Modal, Cloudflare all launched products here between 2025 and 2026). A Helios that cannot run a long-lived agent workload is handing the category to Fly/Cloudflare.
3. **Helios has a genuine wedge here** — credential proxy + WASM sidecar content inspection is platform-level defense against prompt injection that no competitor ships at the infrastructure layer. Position the feature on this wedge, not on "we too have a Firecracker sandbox."

### 6.3 Open Questions for User Discussion
- **Naming**: not `sprite`. Candidates: `microvm` with `persistent = true`; workload profile `agent`; or a new noun (`cell`, `habitat`, `tenant-vm`). Strong opinion: keep it at the driver level, not product-level.
- **Storage layer**: roll JuiceFS directly, fork it, or write a Helios-native chunk-store over Garage? JuiceFS is Apache-2.0 and battle-tested; the "own your primitives" principle suggests embedding it, not depending on it as a separate daemon.
- **Scope of the agent profile**: only AI-agent workloads, or any long-lived stateful workload (CI runners, ephemeral dev envs, notebooks)? The profile is cheap to generalize if designed right.
- **Multi-tenancy boundary**: is Helios multi-tenant by design (customers A and B on the same node) or single-tenant (one org, one Helios cluster)? The answer changes the hardening bar significantly. The current whitepaper is ambiguous; this choice should be made explicitly before exposing persistent arbitrary-code sandboxes.
- **Public HTTPS termination**: does Helios ship automatic ACME + SNI routing for the per-workload URL, or is that a gateway plugin? Fly's advantage here is the Anycast network; Helios will not match that in v1.

---

## Knowledge Gaps

### Gap 1: Sprites memory-snapshot implementation details
**Issue**: Fly has not published a dedicated engineering post on the memory snapshot mechanism (as opposed to the storage stack). The community thread [6] is sparse.
**Attempted**: WebFetch of community thread; WebSearch for "fly sprites memory snapshot"; fetch of design-and-implementation blog.
**Recommendation**: Monitor fly.io/blog for a follow-up post; assume it is essentially Firecracker's native mechanism with some state-hydration shim.

### Gap 2: Cloud Hypervisor snapshot scale-testing evidence
**Issue**: No public evidence of Cloud Hypervisor snapshot/restore being exercised at AWS-Lambda scale (millions/day). Firecracker has this via AWS Lambda and Fly.
**Attempted**: Search on "Cloud Hypervisor production snapshot scale"; found release notes but not deployment case studies.
**Recommendation**: Flag as open engineering risk. Early Helios sprite-equivalent workloads should be either single-tenant or internal-only until scale evidence accumulates.

### Gap 3: E2B cold-start latency ambiguity
**Issue**: Two sources cite different numbers — ~150 ms [9] vs "not specified" [4]. Likely variation by template size.
**Attempted**: Dev.to post [search result] references 28 ms; that is probably a specific micro-benchmark, not a representative sprite-vs-E2B comparison.
**Recommendation**: For head-to-head claims, benchmark in-house rather than citing marketing.

### Gap 4: Helios IntentStore / ObservationStore fit for per-sprite routing
**Issue**: Fly uses Corrosion (SQLite CRDT) for instant URL provisioning. Helios's ObservationStore is also SQLite-CRDT (Corrosion-based per the whitepaper L333). The fit is conceptually perfect but not yet designed explicitly for per-workload public-URL provisioning.
**Attempted**: Read whitepaper §4, §11.
**Recommendation**: A small design note should validate that auto-provisioning a per-workload URL is O(1) and propagates within the CRDT gossip window.

## Full Citations

[1] Fly.io. "The Design & Implementation of Sprites". The Fly Blog. January 2026. https://fly.io/blog/design-and-implementation/. Accessed 2026-04-19.

[2] Willison, Simon. "Fly's new Sprites.dev addresses both developer sandboxes and API sandboxes at the same time". simonwillison.net. 2026-01-09. https://simonwillison.net/2026/Jan/9/sprites-dev/. Accessed 2026-04-19.

[3] Fly.io. "Sprites - Stateful sandboxes". sprites.dev. 2026. https://sprites.dev/. Accessed 2026-04-19.

[4] Northflank. "E2B vs Sprites dev: comparing AI code execution sandboxes in 2026". Northflank Blog. 2026. https://northflank.com/blog/e2b-vs-sprites-dev. Accessed 2026-04-19.

[5] Fly.io. "Code And Let Live". The Fly Blog. January 2026. https://fly.io/blog/code-and-let-live/. Accessed 2026-04-19.

[6] Fly Community. "How is sprite memory snapshotted and restored?". community.fly.io. 2026. https://community.fly.io/t/how-is-sprite-memory-snapshotted-and-restored/26843. Accessed 2026-04-19.

[7] E2B. "e2b-dev/infra — Infrastructure that's powering E2B Cloud". GitHub. Apache-2.0. 2026. https://github.com/e2b-dev/infra. Accessed 2026-04-19.

[8] Fly.io. "Fly Machines documentation". fly.io/docs. Referenced via [1] and prior Helios research `docs/research/infrastructure/fly-io-primitives-helios-relevance.md`.

[9] Dwarves Foundation. "E2B breakdown". memo.d.foundation. 2026. https://memo.d.foundation/breakdown/e2b. Accessed 2026-04-19.

[10] Modal. "Memory Snapshots: Checkpoint/Restore for Sub-second Startup". Modal Blog. 2025. https://modal.com/blog/mem-snapshots. Accessed 2026-04-19.

[11] Firecracker Project. "Snapshot support". GitHub. https://github.com/firecracker-microvm/firecracker/blob/main/docs/snapshotting/snapshot-support.md. Accessed 2026-04-19.

[12] Modal. "Sandbox guide". Modal Docs. 2026. https://modal.com/docs/guide/sandbox. Accessed 2026-04-19.

[13] E2B. "Firecracker vs QEMU". E2B Blog. https://e2b.dev/blog/firecracker-vs-qemu. Accessed 2026-04-19.

[14] Kata Containers Project. "kata-containers/kata-containers". GitHub. Apache-2.0. https://github.com/kata-containers/kata-containers. Accessed 2026-04-19.

[15] CRIU Project. "CRIU Main Page". criu.org. https://criu.org/Main_Page. Accessed 2026-04-19.

[16] Kubernetes. "Forensic container checkpointing in Kubernetes". kubernetes.io/blog. 2022-12-05. https://kubernetes.io/blog/2022/12/05/forensic-container-checkpointing-alpha/. Accessed 2026-04-19.

[17] Cloudflare. "Overview — Cloudflare Sandbox SDK docs". developers.cloudflare.com/sandbox. 2026. https://developers.cloudflare.com/sandbox/. Accessed 2026-04-19.

[18] Cloudflare. "Architecture — Cloudflare Sandbox SDK". developers.cloudflare.com. 2026. https://developers.cloudflare.com/sandbox/concepts/architecture/. Accessed 2026-04-19.

[19] Cloudflare. "Sandboxing AI agents, 100x faster". Cloudflare Blog. April 2026. https://blog.cloudflare.com/dynamic-workers/. Accessed 2026-04-19.

[20] Cloud Hypervisor Project. "snapshot_restore.md". GitHub. https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/snapshot_restore.md. Accessed 2026-04-19.

[21] Phoronix. "Cloud Hypervisor 37 LTS Released With Faster VM Restoration From Snapshots". phoronix.com. 2025. https://www.phoronix.com/news/Cloud-Hypervisor-37-LTS. Accessed 2026-04-19.

[22] InfoQ. "Cloudflare Launches Dynamic Workers Open Beta". infoq.com. April 2026. https://www.infoq.com/news/2026/04/cloudflare-dynamic-workers-beta/. Accessed 2026-04-19.

[23] Helios whitepaper (`docs/whitepaper.md`). §§ 2, 4, 6, 7, 8, 9, 11, 16, 17. Accessed 2026-04-19.

[24] Prior Helios research: `docs/research/infrastructure/fly-io-primitives-helios-relevance.md`. Accessed 2026-04-19.
