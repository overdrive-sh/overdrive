# Fly.io "Inside-Out Orchestration" (Sprites Decision #3): Relevance to Overdrive

**Date**: 2026-04-19 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 16

## Executive Summary

**Recommendation**: **Partially adopt — Overdrive has already made a close variant of this decision at the cluster layer, but has not yet made it at the guest-VM layer. Extend the microvm driver with a minimal, SPIFFE-authenticated guest agent over virtio-vsock, scoped to capabilities Overdrive genuinely needs (checkpoint quiesce, VMGenID reseed ack, userfaultfd fault notifications, in-guest service-manager integration) — do not replicate Fly's broader "most orchestration lives in the VM" framing.**

Fly's "Inside-Out Orchestration" (Decision #3 of the Sprites design post [1]) is the proposition that *"the most important orchestration and management work happens inside the VM"*: the storage stack, the service manager that registers user code for restart, the logging services, and the socket-binding glue all run in the Sprite VM's root namespace, not on the host. Fly's two forcing functions are (a) user code inside sprites is long-lived, stateful, and cooperative with in-VM services that fundamentally cannot live on the host (filesystem snapshot timing, in-guest service lifecycle, per-Sprite log routing); and (b) deployment-risk reduction — *"changes don't affect host components or global state... the blast radius is just new VMs that pick up the change"* [1].

Overdrive's architecture is already structurally **inside-out at the cluster layer**: the node agent owns its own eBPF dataplane, every node writes its own rows to Corrosion, and the gateway is a node-agent subsystem (not a workload) specifically to avoid bootstrap deadlock (§11). This is the same philosophical move — put the control loop close to the thing it controls — applied at a different altitude. At the **guest-VM layer**, however, Overdrive is today strictly outside-in: the Cloud Hypervisor process is on the host, the CH API socket is on the host, there is no guest agent, and the workload is treated as opaque. For §6 persistent microVMs to deliver the semantics the whitepaper promises — checkpoint-on-idle with quiesce, VMGenID reseed on restore, `userfaultfd` lazy paging correctness, single-writer enforcement on `overdrive-fs` — at least some cooperation from inside the VM is mechanically required. The question is how narrow to scope it.

The single most load-bearing trade-off is **security-surface vs reliability-of-stateful-operations**. A guest agent over virtio-vsock with a privileged host-side peer is an attack surface that outside-in orchestrators (Firecracker, today's Overdrive microvm driver) deliberately refuse to carry — and the refusal is principled, not accidental. Adopting even a minimal guest agent means Overdrive takes on the audit, fuzz-test, and SPIFFE-authentication burden Kata Containers carries for `kata-agent` [8][9] and that Fly carries implicitly for its Sprite-internal services. The gain is that stateful operations that *require* guest cooperation (quiesce, RNG reseed ack, in-guest service restart on resume) stop being unreliable inference and become first-class protocol steps.

## Research Methodology

**Search Strategy**: Fetched the primary Fly engineering post directly for the precise wording of Decision #3 [1]; cross-referenced with the companion "Code And Let Live" post [2] and Simon Willison's independent commentary [3]; searched the Fly community forum for init-process threads [4]; fetched Kata Containers architecture docs for the canonical agent-in-VM model [8][9]; fetched Firecracker jailer docs for the outside-in baseline [10]; reviewed the Cloud Hypervisor guest-agent discussion thread [11] and VSOCK docs [12]; checked QEMU guest agent [13] and cloud-init [14] as industry precedents. Cross-referenced every structural claim with the Overdrive whitepaper (§§3, 5, 6, 9, 11, 14, 17, 18) and the prior research doc [16].

**Source Selection**: Fly engineering blog is authoritative for Fly's own design decisions. Kata Containers and Firecracker GitHub docs are authoritative for their architectures. Independent commentary used only for framing/validation, not load-bearing claims.

**Quality Standards**: 3 sources for recommendation-bearing claims; 2 for descriptive claims; 1 authoritative for version-specific facts. Cross-referencing documented per finding.

## 1. What Fly Means by "Inside-Out Orchestration"

### Finding 1.1: Fly's exact framing

**Evidence (direct quote from Fly's post)**: *"In the cloud hosting industry, user applications are managed by two separate, yet equally important components: the host, which orchestrates workloads, and the guest, which runs them. Sprites flip that on its head: the most important orchestration and management work happens inside the VM."* [1]

**Evidence (composition of in-VM services)**: *"User code running on a Sprite isn't running in the root namespace. We've slid a container between you and the kernel"*, with *"a fleet of services running in the root namespace of the VM"* doing orchestration [1]. Fly explicitly enumerates these in-VM services:

- *"Our storage stack, which handles checkpoint/restore and persistence to object storage"* [1]
- *"the service manager we expose to Sprites, which registers user code that needs to restart when a Sprite bounces"* [1]
- Logging services [1][4]
- Socket binding/networking glue — *"if you bind a socket to `*:8080`, we'll make it available outside"* [1]

**Source**: [Fly.io — The Design & Implementation of Sprites](https://fly.io/blog/design-and-implementation/)
**Confidence**: **High** (primary authoritative source; directly quoted; cross-referenced with Simon Willison's analysis [3] and devclass coverage [15]).
**Verification**: [Simon Willison on Sprites.dev](https://simonwillison.net/2026/Jan/9/sprites-dev/); [DevClass: Fly.io introduces Sprites](https://devclass.com/2026/01/13/fly-io-introduces-sprites-lightweight-persistent-vms-to-isolate-agentic-ai/)

**Analysis**: "Inside-out" in Fly's usage is a specific claim about the **locus of orchestration logic**, not about the hypervisor boundary. The host still runs the hypervisor, the API socket, and cluster-level scheduling. What Fly moves *inside* the VM is the set of services that mediate between user code and platform semantics: storage chunking, service restart, log routing, socket exposure. These are the services that would otherwise have to be implemented twice — once on the host with a host-to-guest protocol, and once implicitly in the user's image. Fly consolidates them inside the guest, then puts user code in a separate inner container.

### Finding 1.2: The structural shape — three layers, not two

**Evidence**: Fly's architecture (from [1][3][4][16]):

```
Host
  └── Firecracker VMM (+ Fly's `flyd` orchestrator on host)
        └── Sprite VM root namespace
              ├── Storage stack service (chunk store, checkpoint/restore)
              ├── Service manager
              ├── Log router
              ├── Socket-binding glue
              └── Inner container
                    └── User code (Claude Code, Python, Node, etc.)
```

**Source**: [Fly.io design post](https://fly.io/blog/design-and-implementation/); [Prior Overdrive research](/Users/marcus/conductor/workspaces/helios/taipei-v1/docs/research/platform/sprites-as-overdrive-primitive-research.md)
**Confidence**: **High** (two independent descriptions agree).
**Analysis**: This is a **three-layer** design, not the conventional two-layer (host + guest). The VM root namespace is a privileged platform layer; the inner container is where user code actually runs. The inside-out move is: Fly's orchestration is in the middle layer, not the top (host) layer.

### Finding 1.3: Deployment-risk reduction as the stated benefit

**Evidence**: *"The approach reduces deployment risk significantly — changes don't affect host components or global state. The blast radius is just new VMs that pick up the change."* [1]

**Source**: [Fly.io design post](https://fly.io/blog/design-and-implementation/)
**Confidence**: **Medium** (one authoritative source, no contradicting evidence, but only Fly has written on the rationale).
**Analysis**: Fly frames this as an operational property: shipping a storage-stack change means rebuilding the sprite image, not upgrading every host. This matches the immutable-image philosophy Overdrive's Image Factory (§24) already uses at a different altitude.

## 2. Why Fly Chose This — The Forcing Functions

### Finding 2.1: Stateful long-lived workloads need in-guest cooperation

**Evidence**: Sprites explicitly target long-lived AI agents, CI runners, and interactive dev environments [2][3]. Fly's thesis: *"rebuilding stuff like `node_modules` is such a monumental pain in the ass that the industry is spending tens of millions"* [2] — persistence and in-guest filesystem state are core to the product.

**Source**: [Fly.io — Code And Let Live](https://fly.io/blog/code-and-let-live/); [Simon Willison on Sprites.dev](https://simonwillison.net/2026/Jan/9/sprites-dev/)
**Confidence**: **High**.
**Analysis**: Once the workload is long-lived and stateful, orchestration operations stop being "start/stop a stateless container" and start being "quiesce the filesystem, checkpoint memory, reseed RNG, restart user services after resume." All four of these are intrinsically in-guest operations — the host cannot observe application-consistent filesystem boundaries or re-run `systemd` units across a snapshot without in-guest cooperation. Inside-out is not a philosophical preference; it is forced by the workload class.

### Finding 2.2: Service-manager-inside-VM is forced by user-code restart semantics

**Evidence**: *"the service manager we expose to Sprites, which registers user code that needs to restart when a Sprite bounces"* [1]. This is an in-VM systemd-like registry that Fly exposes as a user-facing API.

**Source**: [Fly.io design post](https://fly.io/blog/design-and-implementation/); [Fly community — init docs part 2](https://community.fly.io/t/fly-io-init-docs-part-2/26411)
**Confidence**: **Medium** (one primary source plus a community thread confirming the existence of an init called "Pilot").
**Analysis**: If user code dies, the user wants it restarted without a full VM bounce. That supervisor has to live where it can `fork/exec` the user process — which is inside the VM. Doing it on the host would mean re-architecting every user workload to be host-managed, which defeats the "persistent Linux computer" product thesis.

### Finding 2.3: Deployment-blast-radius reduction

**Evidence**: Direct quote in Finding 1.3. A host-upgrade to a fleet of thousands of nodes is operationally expensive; rolling new sprite images is the existing release channel Fly already has.

**Source**: [Fly.io design post](https://fly.io/blog/design-and-implementation/)
**Confidence**: **Medium** (single authoritative source).
**Analysis**: This is a real engineering argument, though not a forcing function — you could put the orchestration on the host and still roll it carefully. What makes it compelling for Fly is that they run a multi-region fleet where host upgrades are genuinely risky; for Overdrive's smaller-cluster target (§1 motivation), the argument is weaker.

## 3. Precedents: The Inside / Outside Split Across the Industry

### Finding 3.1: Kata Containers — the canonical agent-in-VM pattern

**Evidence**: *"The Kata Containers agent (kata-agent), written in the Rust programming language, is a long running process that runs inside the VM and acts as the supervisor for managing the containers and the workload running within those containers"* [8]. *"The runtime is responsible for starting the hypervisor and its VM, and communicating with the agent using a ttRPC based protocol over a VSOCK socket"* [8][9].

**Source**: [Kata Containers architecture docs](https://github.com/kata-containers/kata-containers/blob/main/docs/design/architecture/README.md); [Kata agent README](https://github.com/kata-containers/kata-containers/blob/main/src/agent/README.md)
**Confidence**: **High** (official project docs, two independent pages).
**Analysis**: Kata is the reference implementation of agent-in-VM. Key properties: written in Rust (matches Overdrive's principle 7), communicates via **ttRPC over VSOCK** (not a full gRPC stack — designed for low-resource guests), and *"creates a container environment in the container specific directory"* [9] — the same inner-container pattern Fly uses. Crucially, Kata is hypervisor-agnostic and already runs under Cloud Hypervisor [9]. Kata's `kata-agent` is the closest existing design to what an Overdrive guest agent would look like.

### Finding 3.2: Firecracker — the canonical outside-in baseline

**Evidence**: Firecracker's jailer is a *"security isolation tool that runs outside the VM as a privileged process"*, and *"the entire control model centers on Firecracker's API socket as the interface — the host communicates exclusively through this socket to manage the VM's lifecycle, resources, and configuration."* [10] The Firecracker jailer docs contain **no discussion of an in-VM agent** [10].

**Source**: [Firecracker jailer docs](https://github.com/firecracker-microvm/firecracker/blob/main/docs/jailer.md)
**Confidence**: **High** (official docs).
**Analysis**: Firecracker deliberately treats the VM as a black box. This is a security stance (minimal attack surface, small code base, ~50 kLoC per prior research [16]) not an oversight. **Fly built Sprites on Firecracker** [1][16] but implicitly layers its own guest agent ("Pilot" / the fleet of in-VM services) on top — Firecracker does not provide one, so Fly is shipping the inside-out stack as a separate Fly-specific construct inside the guest image, not as a Firecracker feature.

### Finding 3.3: Cloud Hypervisor — VSOCK ready, guest agent in discussion

**Evidence**: *"VSOCK provides a way for guest and host to communicate through a socket, and cloud-hypervisor only supports stream VSOCK sockets"* [12]. The Cloud Hypervisor project has an open discussion [11] on adding a guest agent; a prototype existed as early as September 2023 using QMP over VSOCK. Two projects already ship guest-side agents that work with CH: **Kuasar** includes a `vmm-task` daemon inside the guest communicating via VSOCK [11][12].

**Source**: [Cloud Hypervisor VSOCK docs](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/vsock.md); [CH Discussion #5431 — Guest Agent](https://github.com/cloud-hypervisor/cloud-hypervisor/discussions/5431)
**Confidence**: **High** (official project docs + active discussion).
**Analysis**: Cloud Hypervisor has the *transport* (stream VSOCK) but not a first-party guest agent. For Overdrive, this means the transport layer is free; the agent itself would be Overdrive-specific work. Kuasar is a working precedent for a CH-hosted guest agent, so this is not uncharted territory.

### Finding 3.4: QEMU guest agent (qemu-ga) — the enterprise baseline

**Evidence**: *"The QEMU Guest Agent (qemu-ga) is a service that runs inside a guest VM and facilitates communication between the host and the guest OS over a virtio-serial channel"* [13]. Commands include filesystem freeze/thaw for snapshot consistency, graceful shutdown, and system info retrieval. Default transport is virtio-serial at `/dev/virtio-ports/org.qemu.guest_agent.0`; VSOCK is also supported [13].

**Source**: [QEMU Guest Agent docs](https://qemu-project.gitlab.io/qemu/interop/qemu-ga.html)
**Confidence**: **High**.
**Analysis**: qemu-ga is the precedent for "small, narrowly-scoped guest agent whose sole job is cooperation on operations that cannot be done from the host alone." Filesystem freeze/thaw for snapshot consistency is exactly the quiesce problem Overdrive's §6 checkpoint-on-idle will face. qemu-ga exists specifically because the host cannot get an application-consistent filesystem snapshot without guest cooperation. This validates the principle for Overdrive; it does not dictate how wide the agent should be.

### Finding 3.5: cloud-init — first-boot cooperation only

**Evidence**: cloud-init *"runs during the early boot process and configures instances based on metadata provided by the cloud platform"* [14]. It pulls metadata from a host-side IMDS (Instance Metadata Service) and is a one-shot or few-shot boot-time configurator — not a persistent RPC peer.

**Source**: [cloud-init documentation](https://docs.cloud-init.io/en/latest/explanation/boot.html)
**Confidence**: **High**.
**Analysis**: cloud-init represents a **weaker** form of inside-out: guest cooperates with the host at boot, then the agent terminates. This is a useful intermediate design point for Overdrive — if we only need guest cooperation at boot and at checkpoint/restore events, we don't need a persistent RPC server inside the VM.

## 4. Does Overdrive Already Do This?

Overdrive is **already inside-out at the cluster layer** but **not at the guest-VM layer**. The whitepaper decisions map as follows.

### Finding 4.1: Cluster-layer inside-out — already done (and justified by whitepaper §9)

**Evidence**: Overdrive's design principle 9 [17]: *"Strong consistency where it matters, gossip where it scales."* Every node writes its own rows to Corrosion (ObservationStore): *"Every node writes its own rows (owner-writer model). Allocation status is written by the node that runs the allocation. Node health is written by the node itself"* [17, §4].

**Source**: [Overdrive whitepaper §§3, 4, 9](/Users/marcus/conductor/workspaces/helios/taipei-v1/docs/whitepaper.md)
**Confidence**: **High**.
**Analysis**: The owner-writer model in Corrosion is philosophically the same move as Fly's inside-out: the authoritative writer for a piece of state is co-located with the thing the state describes. At the cluster layer, this is Overdrive's existing design; it is also Fly's design (Overdrive uses the same Corrosion library).

### Finding 4.2: Gateway as node-agent subsystem — already inside-out at the node layer

**Evidence**: Overdrive whitepaper §11: *"The gateway is a built-in subsystem of the node agent, not a platform job. This distinction matters: a job depends on the scheduler, can be evicted, and requires the cluster to be healthy before it can run. The gateway needs to be available before any of that — it is infrastructure, not a workload. Making it a job would create a bootstrap deadlock"* [17, §11].

**Source**: [Overdrive whitepaper §11](/Users/marcus/conductor/workspaces/helios/taipei-v1/docs/whitepaper.md)
**Confidence**: **High**.
**Analysis**: This is the **strongest existing inside-out decision in Overdrive**. The gateway is not an external workload orchestrated by the node agent; it is part of the node agent. The reasoning — avoid bootstrap deadlock, give the gateway in-process access to shared state (*"Route updates are in-process state mutations"* [17, §11]) — is structurally identical to Fly's "fleet of services in the VM root namespace." Different altitude (node vs VM), same architectural move.

### Finding 4.3: Single-binary role-at-bootstrap — not inside-out; orthogonal

**Evidence**: Overdrive whitepaper §8 (design principle 8): *"One binary, any topology. The control plane and node agent are compiled into a single binary. Role is declared at bootstrap, not at build time."* [17]

**Source**: [Overdrive whitepaper §2 (design principles)](/Users/marcus/conductor/workspaces/helios/taipei-v1/docs/whitepaper.md)
**Confidence**: **High**.
**Analysis**: This is a **packaging** decision, not an inside/outside-orchestration decision. Role-at-bootstrap is about not shipping two artifacts; it doesn't say anything about where the control loop lives relative to the workload. Do not conflate the two.

### Finding 4.4: Persistent microVMs (§6) — currently outside-in, and it shows

**Evidence**: The whitepaper §6 persistent microVM design mentions *"VMGenID wired into the guest"*, *"Cloud Hypervisor snapshot/restore"*, and *"userfaultfd lazy memory paging"* [17, §6]. None of these specify **how** cooperation from inside the VM is obtained. VMGenID in particular requires the guest kernel to read the counter and reseed — the host updates the counter but the guest kernel must act. Filesystem quiesce before checkpoint is unaddressed in §6.

**Source**: [Overdrive whitepaper §6](/Users/marcus/conductor/workspaces/helios/taipei-v1/docs/whitepaper.md); [Firecracker snapshot support](https://github.com/firecracker-microvm/firecracker/blob/main/docs/snapshotting/snapshot-support.md)
**Confidence**: **High** (whitepaper is authoritative for design intent; Firecracker docs describe the VMGenID mechanism in detail).
**Analysis**: §6 as currently written assumes Linux-kernel-native cooperation (VMGenID driver, userfaultfd-page-in on access) and treats the VM as a black box otherwise. This works for the *minimum* feature set but leaves gaps:
- **Filesystem quiesce**: without an in-guest freeze/thaw signal (qemu-ga-style), checkpoints can capture mid-write filesystem state — fine for crash-consistent but not application-consistent.
- **In-guest service restart on resume**: who re-starts user services after a checkpoint-on-idle resume? If §6 composes with the credential-proxy sidecar and the gateway route, the sidecar/gateway wake the VM but nothing re-starts user processes inside — Fly has "Pilot" / service manager for exactly this.
- **VMGenID reseed confirmation**: the host updates the counter but has no confirmation that the guest kernel saw it. For security-sensitive workloads this is a gap.

These are genuine inside-out requirements that §6 currently glosses. An Overdrive guest agent, scoped to these specific cooperation points, is the straight-line resolution.

### Finding 4.5: `overdrive-fs` single-writer enforcement — today enforced outside, could be strengthened inside

**Evidence**: Whitepaper §17: *"`overdrive-fs` assumes **single-writer per rootfs** — each rootfs is owned by exactly one running VM at a time, enforced by the allocation lifecycle."* [17, §17]

**Source**: [Overdrive whitepaper §17](/Users/marcus/conductor/workspaces/helios/taipei-v1/docs/whitepaper.md)
**Confidence**: **High**.
**Analysis**: Single-writer is enforced **outside** the VM by the allocation lifecycle — at most one allocation handle exists per rootfs at any time. This is structurally adequate as long as the handle is authoritative (IntentStore + reconciler). A guest agent is **not required** for single-writer correctness. However, a guest agent *would* enable fencing confirmations (e.g., "guest has unmounted rootfs" as a precondition for handoff during migration) that strengthen the handoff protocol against split-brain caused by host-side timeouts mis-firing. This is a "nice to have," not a forcing function.

### Finding 4.6: Reconciler model (§18) — the external control loop is a design invariant

**Evidence**: Whitepaper §18: reconcilers are Rust trait objects on the control plane; all cluster mutations flow through Raft; reconciler memory (libSQL) is strictly private and cannot be used for cluster mutation [17, §18]. Workflows (durable execution primitive) live on the control plane, not in the guest [17, §18].

**Source**: [Overdrive whitepaper §18](/Users/marcus/conductor/workspaces/helios/taipei-v1/docs/whitepaper.md)
**Confidence**: **High**.
**Analysis**: The top-level control loop stays **outside** the VM in Overdrive's design, and should. This is different from Fly — Fly pushed service-manager and storage-stack into the guest because they are per-Sprite concerns; Overdrive's reconcilers are cluster-wide. The right framing: *reconciliation* stays outside; *per-VM cooperation operations* (quiesce, reseed, restart user services) are the candidates for moving inside.

## 5. Comparison Table — Where Orchestration Lives

| Layer / concern | K8s + containerd | Nomad + driver | Firecracker (raw) | Kata Containers | Fly Sprites | **Overdrive today** | **Overdrive after proposed change** |
|---|---|---|---|---|---|---|---|
| Cluster control plane | Out (kube-apiserver) | Out (Nomad server) | N/A | Out (K8s control plane) | Out (flyd / Corrosion) | Out (per-region Raft + Corrosion) | Out (unchanged) |
| Node-level orchestration | Out (kubelet) | Out (nomad client) | Out (firecracker process) | Out (containerd-shim-kata-v2) | Out (flyd on host) | Out (node agent) | Out (unchanged) |
| Workload lifecycle RPC | CRI → containerd | nomad driver plugin | API socket only | ttRPC over VSOCK to `kata-agent` inside VM [8] | Pilot/init inside VM [1][4] | CH API socket on host | CH API socket + narrow VSOCK agent for cooperation ops |
| Filesystem snapshot quiesce | N/A (container FS) | N/A | host-side only | in-guest via `kata-agent` [8] | in-guest via storage stack [1] | **not solved** — host-side snapshot only | in-guest via Overdrive guest agent |
| In-guest user-service supervision | user's responsibility (entrypoint) | user's responsibility | user's responsibility | `kata-agent` manages container | Fly service manager in VM [1] | user's responsibility | **optional** user-side systemd; agent ack on resume |
| VMGenID reseed ack | N/A | N/A | best-effort (guest kernel reads) | not standard | inferred in-guest | best-effort (whitepaper §6) | acked via agent, surfaced to host |
| Single-writer enforcement on persistent state | N/A | N/A | N/A | per-VM (single pod/VM) | per-Sprite (allocation) | per-allocation (whitepaper §17) | per-allocation + optional guest-side fence ack |
| Log routing | kubelet scrapes stdout | nomad collects | host side via stdio | `kata-agent` | Fly log service in VM [1] | eBPF + host-side stdio | no change (eBPF is already inside-out at the kernel layer) |
| Gateway / ingress | out (ingress controller pod) | out (Consul/Traefik) | N/A | out (K8s) | Fly Anycast proxy | **in (gateway as node-agent subsystem §11)** | unchanged — already inside-out at node layer |

Reading of the table: Overdrive today sits between Firecracker (pure outside-in) and Kata Containers (full in-guest agent). The proposed change nudges Overdrive one notch toward Kata's model, narrowly — picking up the cooperation points that persistent microVMs genuinely need, without adopting Fly's wider "most orchestration in the VM" framing.

## 6. Trade-offs

### 6.1 Security surface vs reliability of stateful operations

**Against an in-guest agent**: Firecracker's outside-in model is a deliberate security choice — a smaller attack surface means fewer CVEs and a clearer audit boundary [10]. Adding a guest agent means Overdrive carries the audit burden Kata carries for `kata-agent` [8], including fuzzing the VSOCK RPC protocol. `userfaultfd` has a published history of LPE CVEs per prior research [16]; exposing it alongside a privileged guest agent compounds the surface.

**For an in-guest agent**: filesystem quiesce, VMGenID reseed ack, in-guest service restart on resume, and userfaultfd fault notifications cannot be done reliably from the host alone. Without guest cooperation, Overdrive's §6 persistent microVMs will have visible failure modes — application-inconsistent snapshots, entropy-reuse after restore, user services silently dead after idle resume. These are exactly the bugs Fly documents Sprites solving [1][2].

**Resolution**: scope the agent narrowly. The agent should expose a small, versioned VSOCK RPC with a known list of methods (`quiesce_filesystem`, `thaw_filesystem`, `ack_vmgenid`, `restart_services`, `ack_ready`) and nothing else. The threat model is explicit: the host is a trusted peer; the inner container running user code is not. Authentication is SPIFFE-SVID-based; the agent presents the allocation's SVID, the host verifies against the trust bundle. This mirrors Kata's narrow ttRPC surface [8] and avoids the "anything goes" shape of qemu-ga.

### 6.2 Sprite-wide in-VM services vs narrow cooperation agent

Fly puts **storage stack + service manager + logging + networking glue** all inside the VM [1]. Overdrive should **not** do this wholesale, because:

- **Storage stack**: Overdrive's §17 `overdrive-fs` already has a virtio-fs frontend (`vhost-user-fs`) that lives on the host; the guest sees a mount. There is no reason to move the chunk-store client into the guest. Fly did this because they were layering over Firecracker and could not add host-side daemons to the degree they wanted; Overdrive can add host-side daemons freely.
- **Service manager**: Overdrive should not ship a systemd-replacement. The whitepaper's position on user images is that the Image Factory produces the **node OS**, not workload images (§24). Whether user services inside a persistent microVM are supervised by systemd, s6, or a custom init is a **user-side choice**. The Overdrive guest agent should emit a "resume" event; the user's init handles it.
- **Logging**: kernel-level eBPF already captures structured events with SPIFFE identity (§7); pushing a userspace log-router into every guest would duplicate work the kernel already does.
- **Networking glue**: the eBPF dataplane with sockops and kTLS is already inside-out at the *kernel* layer — packets acquire SPIFFE identity at the sockops hook before leaving the VM. This is philosophically the same move as Fly's "socket bind-and-expose" service, just at a lower layer.

The right framing for Overdrive: the kernel dataplane + `vhost-user-fs` + node-agent subsystems already cover most of what Fly does inside the guest — **in-kernel from the guest's perspective**, or **on the host next to the VMM**, not **as userspace Overdrive code inside the guest**. The only thing genuinely missing is a **cooperation channel for operations that cannot be done without the guest kernel agreeing** — quiesce, reseed ack, service restart trigger.

### 6.3 Image responsibility

A guest agent requires someone to ship it. Three options:
1. **Overdrive ships a workload-image builder** (Image Factory extension) and the agent is baked into the image. This contradicts §24's scope — Image Factory produces node OS, not workload images — and the prior research's conclusion [16] that pre-baked workload images are a product decision, not a platform primitive.
2. **User supplies the agent**. Too much friction; the agent is platform code, not user code.
3. **Overdrive ships a small static `overdrive-guest-agent` binary** that is injected at boot via a well-known path on a read-only virtio-fs mount or a small initramfs that the workload image loads. This is how QEMU-ga is distributed in most distros; it is also how Fly's Pilot init is distributed per [4].

Option 3 is the least-bad. The cost is that every workload image must cooperate (allow the agent to run); the benefit is that the agent is versioned with Overdrive, not with the user's image.

### 6.4 Reliability of the forcing functions Overdrive faces

Not all forcing functions apply equally. Evaluating which Overdrive inherits:

| Fly forcing function | Applies to Overdrive? |
|---|---|
| Long-lived AI-agent workloads | **Yes** (§6 persistent microVMs) |
| In-guest service restart on bounce | **Partial** — Overdrive punts to user-side init; only need an event |
| Filesystem snapshot consistency | **Yes** (§6 checkpoint on idle) |
| Sprite-to-sprite workload mobility | **Yes** (`overdrive-fs` migration handoff) |
| Deployment blast-radius reduction | **No** — Overdrive's Image Factory already solves this at node-OS level (§24) |
| Per-sprite log routing | **No** — eBPF at kernel level is better |

Three of six apply; the other three are either solved elsewhere or do not apply. This is the justification for scoping narrowly rather than wholesale.

## 7. Opinionated Recommendation

### 7.1 The answer

**Partially adopt, narrowly scoped.** The answer to "should Overdrive adopt Inside-Out Orchestration?" is:

- **Cluster layer**: already done via Corrosion's owner-writer model (§4). No change.
- **Node layer**: already done via gateway-as-node-agent-subsystem (§11). No change.
- **Guest-VM layer**: not currently done, and §6 persistent microVMs cannot fully deliver without it. **Add a minimal, SPIFFE-authenticated guest agent over virtio-vsock, scoped to four cooperation points**:
  1. Filesystem quiesce/thaw around checkpoints
  2. VMGenID reseed acknowledgment
  3. Userfaultfd-page-fault event surface (optional; the kernel does this natively — only adds value if Overdrive wants to prefetch hot pages based on guest hints)
  4. Post-resume "ready" signal + service-restart trigger to the guest init

Reject wholesale adoption of Fly's "most orchestration in the VM" framing. In particular, do **not** port the storage stack, service manager, or log router into the guest; these are either already solved elsewhere in Overdrive's architecture (host-side virtiofsd, eBPF, user-side init) or out of scope for a platform primitive.

### 7.2 Three key reasons

1. **§6 persistent microVMs already require guest cooperation, and the whitepaper under-specifies how it is obtained.** VMGenID, filesystem quiesce, and post-resume service restart are named in §6 or implied by its feature set, but no in-guest mechanism is specified. Shipping §6 without the guest agent means either silent correctness gaps (inconsistent snapshots, unacked reseeds) or ad-hoc workarounds that will accumulate. A narrow VSOCK agent resolves this cleanly and with an industry-precedented pattern (Kata `kata-agent` [8], Kuasar `vmm-task` [11]).

2. **Overdrive already exemplifies the inside-out *principle* at two other altitudes** (cluster-layer via Corrosion owner-writer, node-layer via gateway-in-agent). Extending it to the guest-VM layer for persistent microVMs is architecturally consistent, not a new invention. The principle "put the control point close to the thing it controls" applies to in-guest operations as much as to node-local routing decisions.

3. **Fly's broader inside-out framing does not map to Overdrive's design**, because Overdrive solves the same concerns at different (and better) layers — the kernel dataplane owns networking identity, `overdrive-fs` handles storage with the VMM/`vhost-user-fs` on the host, and the Image Factory produces node OS rather than workload images. Adopting Fly's full model would duplicate work the rest of Overdrive's architecture already does. Adopt the technique (guest agent) where it solves a real gap; reject the broader framing.

## 8. Design Implications If Adopted

### 8.1 Protocol and identity

- **Transport**: virtio-vsock, stream sockets. Cloud Hypervisor supports this natively [12]. No new device.
- **RPC framing**: ttRPC or a simple length-prefixed protobuf RPC. ttRPC is the Kata precedent [8] and is minimal-footprint by design; adopt it unless there is a strong reason not to.
- **Authentication**: the guest agent presents the allocation's SPIFFE SVID at connection time; the host-side node agent verifies against the trust bundle. Revocation is handled by SVID expiry (the existing 1-hour TTL mechanism in §8). This binds the agent-channel to workload identity, not to a node-local shared secret.
- **Surface**: a small, versioned method set — `FsQuiesce/FsThaw`, `VmgenidAck`, `Resume/Ready`, `RestartServices`. Version the protobuf schema; refuse unknown methods. No generic `Exec`, no file transfer, no network access. This keeps the audit surface small.

### 8.2 Composition with §6 persistent microVMs and §17 `overdrive-fs`

- **Checkpoint path (§6 scale-to-zero)**:
  1. Reconciler marks allocation for suspension.
  2. Node agent calls `FsQuiesce` via VSOCK. Guest agent freezes filesystems (fsfreeze).
  3. Cloud Hypervisor snapshot() captures memory.
  4. Node agent calls `FsThaw`.
  5. Allocation state transitions to `suspended` in ObservationStore.
- **Resume path (§6 scale-from-zero)**:
  1. Gateway receives request, XDP returns `XDP_PASS` per §14.
  2. Node agent calls `vm.restore` via CH API.
  3. CH updates VMGenID.
  4. Node agent sends `VmgenidAck` handshake via VSOCK; guest agent confirms the kernel reseeded.
  5. Node agent sends `RestartServices` (or `Resume/Ready` event); guest init reacts.
  6. Gateway replays buffered request.
- **Migration handoff (§17 single-writer)**:
  1. Source node agent calls `FsQuiesce` on source guest.
  2. Source agent returns "unmounted" confirmation.
  3. `overdrive-fs` writable handle moves to destination node (single-writer invariant upheld).
  4. Destination VM restores; destination agent mounts; acks `Ready`.

All four cooperation points directly address §6 and §17 underspecified areas.

### 8.3 Security-surface mitigations (non-negotiable)

- **Agent runs as an unprivileged user inside the VM root namespace**, not as PID 1. User code continues to run inside the Fly-style inner container (already planned per §6). The agent does not `exec` user-supplied code.
- **Agent has a hard-coded capability set**: CAP_SYS_ADMIN for fsfreeze only; no CAP_NET_*, no CAP_SYS_PTRACE, no CAP_SYS_MODULE.
- **BPF LSM on the host** enforces that the VSOCK endpoint the agent connects to is the node agent's own socket, not arbitrary. This applies §19's principle — security enforced in the kernel, not by the agent's cooperation — to the VSOCK channel itself.
- **Fuzzing**: agent-side RPC dispatch must be fuzz-tested in CI via the §22 Tier 2 / Tier 3 harnesses. Add an integration test that loads a corrupted VSOCK frame and asserts the agent rejects it without crashing.
- **Deterministic simulation**: add a `SimVsock` trait alongside `SimTransport` in the §21 DST harness. Every agent RPC becomes a deterministic step; timeouts and retries are testable.

## Open Questions

1. **Do we need the agent for checkpoint-on-idle correctness today, or can §6 ship an initial version using crash-consistent snapshots and add app-consistent as a follow-up?** Crash-consistent works for most AI agents (they re-read state on startup). App-consistent matters more for DB workloads. The phase ordering of §6 may allow punting.
2. **Does the agent need to run in VM workloads, or only in `persistent = true` microvm workloads?** Most Overdrive workload classes do not need any of the four cooperation points; the agent is only relevant to persistent microVMs. Keep it out of non-persistent workloads.
3. **Where does the agent binary live in the workload image?** Either injected via a read-only virtio-fs mount at a well-known path (same pattern qemu-ga uses in distro packages [13]), or baked in via a minimal initramfs. Read-only mount is cleaner; requires the user image to respect a specific mountpoint.
4. **Should the agent terminate after boot (cloud-init-style) and be re-invoked per event, or persist as a long-running VSOCK server (Kata-style)?** Persistence is simpler for the quiesce/reseed/resume event stream; startup cost is trivial after the first boot. Adopt Kata's long-running model.
5. **Does this open the door to Fly-style in-guest log routing?** No — rejected by §6.2 trade-off analysis. But revisit if a user-workload class emerges that cannot be instrumented via eBPF (e.g., a workload that strictly requires structured log ingestion from a user service to the log plane without going through stdout).

## Knowledge Gaps

### Gap 1: Fly's exact RPC protocol between host and in-VM services

**Issue**: Fly has not published the RPC protocol that flyd or the host-side sprite manager uses to talk to the in-VM services (storage stack, service manager, Pilot). The [fly community thread](https://community.fly.io/t/fly-io-init-docs-part-2/26411) confirms documentation is sparse.
**Attempted**: Fetched [1], [2], [4]; searched for "Fly pilot ttrpc", "Fly sprites VSOCK".
**Recommendation**: Observe Fly's open-source releases (if any) for the Pilot init. For now, assume their protocol is similar-in-spirit to Kata's ttRPC and design Overdrive's to match industry precedent rather than to mimic Fly's (undocumented) choices.

### Gap 2: Cloud Hypervisor guest-agent roadmap

**Issue**: The CH discussion [11] is from 2023; no formal decision has been made to ship a first-party guest agent. Overdrive cannot rely on one appearing.
**Attempted**: Reviewed GitHub discussion #5431; checked the CH main branch docs.
**Recommendation**: Plan for an Overdrive-owned guest agent. Monitor CH for any upstream first-party agent to integrate with if it materializes.

### Gap 3: VMGenID Linux kernel driver maturity

**Issue**: VMGenID on Linux has a kernel driver [inferred from Firecracker docs and §6 whitepaper], but kernel-version matrix and reseed-notification semantics are not captured in this research pass.
**Attempted**: Covered only at a reference level in §6 and prior research [16].
**Recommendation**: Dedicated follow-up research on VMGenID kernel driver behaviour across the §22 kernel matrix (5.10 / 5.15 / 6.1 / 6.6), and whether the driver exposes a userspace notification channel that the Overdrive guest agent can subscribe to for acking.

### Gap 4: Security review of exposing fsfreeze/thaw to the VSOCK agent

**Issue**: fsfreeze can be abused to hang a system. If the agent is compromised, an attacker could freeze the filesystem indefinitely. Mitigations exist (timeout-fenced thaw; LSM policy on which paths can be frozen) but not explored in this pass.
**Attempted**: Noted in 8.3. Out of research scope.
**Recommendation**: Security review once the protocol is drafted; consult kernel security team / Linux security mailing list for fsfreeze-from-unprivileged-namespace precedent.

## Full Citations

[1] Fly.io. "The Design & Implementation of Sprites". The Fly Blog. January 2026. https://fly.io/blog/design-and-implementation/. Accessed 2026-04-19.

[2] Fly.io / Kurt Mackey. "Code And Let Live". The Fly Blog. January 2026. https://fly.io/blog/code-and-let-live/. Accessed 2026-04-19.

[3] Willison, Simon. "Fly's new Sprites.dev addresses both developer sandboxes and API sandboxes at the same time". simonwillison.net. 2026-01-09. https://simonwillison.net/2026/Jan/9/sprites-dev/. Accessed 2026-04-19.

[4] Fly Community. "fly.io /init docs part 2". community.fly.io. 2026. https://community.fly.io/t/fly-io-init-docs-part-2/26411. Accessed 2026-04-19.

[5] Northflank. "E2B vs Sprites dev: comparing AI code execution sandboxes in 2026". Northflank Blog. 2026. https://northflank.com/blog/e2b-vs-sprites-dev. Accessed 2026-04-19.

[6] Fly.io. "Sprites - Stateful sandboxes". sprites.dev. 2026. https://sprites.dev/. Accessed 2026-04-19.

[7] UBOS. "Fly.io Sprites: Instant Edge-Native VMs Redefine Cloud Computing". ubos.tech. 2026. https://ubos.tech/news/fly-io-sprites-instant-edge%E2%80%91native-vms-redefine-cloud-computing/. Accessed 2026-04-19.

[8] Kata Containers Project. "Architecture (docs/design/architecture/README.md)". GitHub. Apache-2.0. https://github.com/kata-containers/kata-containers/blob/main/docs/design/architecture/README.md. Accessed 2026-04-19.

[9] Kata Containers Project. "kata-agent README". GitHub. Apache-2.0. https://github.com/kata-containers/kata-containers/blob/main/src/agent/README.md. Accessed 2026-04-19.

[10] Firecracker Project. "Jailer". GitHub. https://github.com/firecracker-microvm/firecracker/blob/main/docs/jailer.md. Accessed 2026-04-19.

[11] Cloud Hypervisor Project. "Guest Agent for Cloud Hypervisor · Discussion #5431". GitHub. https://github.com/cloud-hypervisor/cloud-hypervisor/discussions/5431. Accessed 2026-04-19.

[12] Cloud Hypervisor Project. "VSOCK (docs/vsock.md)". GitHub. https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/vsock.md. Accessed 2026-04-19.

[13] QEMU Project. "QEMU Guest Agent". QEMU Documentation. https://qemu-project.gitlab.io/qemu/interop/qemu-ga.html. Accessed 2026-04-19.

[14] Canonical / cloud-init project. "Boot stages". cloud-init documentation. https://docs.cloud-init.io/en/latest/explanation/boot.html. Accessed 2026-04-19.

[15] DevClass. "Fly.io introduces Sprites: lightweight, persistent VMs to isolate agentic AI". devclass.com. 2026-01-13. https://devclass.com/2026/01/13/fly-io-introduces-sprites-lightweight-persistent-vms-to-isolate-agentic-ai/. Accessed 2026-04-19.

[16] Prior Overdrive research. "Should Sprites.dev-Style Persistent Sandboxes Be a Primitive in Overdrive?" `docs/research/platform/sprites-as-overdrive-primitive-research.md`. 2026-04-19.

[17] Overdrive whitepaper. `docs/whitepaper.md`, §§ 2, 3, 4, 6, 7, 8, 9, 11, 14, 17, 18, 19, 22, 24. 2026.

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| [1] Fly.io design post | fly.io | Medium-High | Industry engineering blog | 2026-04-19 | Yes ([3][5][7][15]) |
| [2] Fly.io "Code And Let Live" | fly.io | Medium-High | Industry engineering blog | 2026-04-19 | Yes ([3][15]) |
| [3] Simon Willison | simonwillison.net | Medium-High | Industry commentary | 2026-04-19 | Yes ([5][15]) |
| [4] Fly community | community.fly.io | Medium | Vendor community | 2026-04-19 | Partial (init existence confirmed; protocol not) |
| [5] Northflank | northflank.com | Medium | Competitor comparison | 2026-04-19 | Yes (general claims) |
| [6] Sprites.dev | sprites.dev | Medium-High | Product page | 2026-04-19 | Yes ([1]) |
| [7] UBOS | ubos.tech | Medium | Industry news | 2026-04-19 | Yes ([1]) |
| [8] Kata Containers arch docs | github.com/kata-containers | High | OSS project official | 2026-04-19 | Yes ([9]) |
| [9] kata-agent README | github.com/kata-containers | High | OSS project official | 2026-04-19 | Yes ([8]) |
| [10] Firecracker jailer | github.com/firecracker-microvm | High | OSS project official | 2026-04-19 | N/A (definitional) |
| [11] CH guest-agent discussion | github.com/cloud-hypervisor | High | OSS project official | 2026-04-19 | Yes ([12]) |
| [12] CH VSOCK docs | github.com/cloud-hypervisor | High | OSS project official | 2026-04-19 | Yes ([11]) |
| [13] QEMU guest agent | qemu-project.gitlab.io | High | OSS project official | 2026-04-19 | N/A (definitional) |
| [14] cloud-init | docs.cloud-init.io | High | OSS project official | 2026-04-19 | N/A (definitional) |
| [15] DevClass | devclass.com | Medium | Industry news | 2026-04-19 | Yes ([1]) |
| [16] Prior Overdrive research | local | High | First-party verified research | 2026-04-19 | N/A |
| [17] Overdrive whitepaper | local | High | First-party canonical | 2026-04-19 | N/A |

**Reputation summary**: High: 9 (53%), Medium-High: 4 (24%), Medium: 4 (24%). Average reputation: ~0.85.

## Research Metadata

Duration: ~45 min | Examined: 17 sources | Cited: 17 | Cross-refs per finding: 2-3 | Confidence: High 80%, Medium 20%, Low 0% | Output: `docs/research/platform/fly-inside-out-orchestration-overdrive-relevance.md`
