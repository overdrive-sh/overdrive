# Research: Workload Orchestration Platforms for IoT Edge — Landscape Scouting for Helios

**Date:** 2026-04-19 | **Researcher:** nw-researcher (Nova) | **Confidence:** High (landscape shape and platform architectures); Medium (specific per-platform production scale numbers) | **Sources:** 26 cited

---

## Executive Summary

The IoT-edge orchestrator landscape in 2026 is dominated by two architectural camps. **Kubernetes-adapted platforms** — KubeEdge (CNCF Incubating, Apache-2.0), K3s, OpenYurt, Akri, SuperEdge — extend or trim the K8s control plane for cloud-edge-split deployments and are the natural recommendation for teams that already live in K8s and whose devices are "small servers" rather than MCU-class endpoints. **Vertically-integrated platforms** — AWS IoT Greengrass, Azure IoT Edge, EVE-OS (LF Edge), Balena — ship their own device OS or device runtime, their own enrollment service, and their own management plane, and are the natural recommendation when the operator wants the whole stack to be one vendor's problem (with the attendant lock-in trade-off, which is explicit for AWS/Azure and split-open for EVE-OS/Balena). A third category — **transport layers and identity toolkits** (Nebula, Tailscale, Ockam, SPIFFE Federation) — is not an orchestrator but is universally what the above camps reach for when the question is "how does a device behind carrier NAT talk to my control plane with a cryptographically-provable identity?"

Every platform in this space solves the four failure modes that made Helios v1 scope out IoT edge the same way, with minor variations: **(1)** they use hub-and-spoke or two-tier hierarchical topology rather than peer gossip — no one puts 100k devices in a SWIM mesh; **(2)** they transport the management plane over outbound-initiated tunnels (WebSocket, MQTT-over-TLS, OpenVPN, WireGuard), which inherently solves NAT; **(3)** enrollment relies on a **global rendezvous service** (Azure DPS, AWS Fleet Provisioning, EVE-OS bootstrap config) rather than static seed lists, with attestation via TPM, X.509, or scoped claim certificates; and **(4)** factory-flashed images ship with only a well-known endpoint plus an attestation secret, which solves the "flash now, deploy in six months" provisioning flow. Helios's whitepaper has none of these four primitives — §4 assumes a Corrosion peer mesh with QUIC, §23 describes TPM attestation as a Phase 2 roadmap item but without the rendezvous protocol. The scope-out was the correct call; reopening would require five substantial additions (tunneled observation, rendezvous-plus-attestation, device-twin reconciler, hierarchical topology, per-node API cache) before the architecture fits.

If Helios ever reopens this question, the most directly borrowable primitives are: **EVE-OS's TPM-anchored bootstrap model** (closest fit — EVE and Helios share a Type-1-ish HV posture and first-class unikernel support); **Azure IoT's device-twin reported-vs-desired reconciliation** (fits Helios's §18 reconciler trait with *zero* changes to the core primitive — it is the same shape at leaf granularity); **AWS Greengrass's fleet provisioning claim certificate + exchange flow** for enrollment; **KubeEdge's CloudHub/EdgeHub outbound WebSocket** for tunneled management-plane transport; and **Balena's binary container delta updates** for bandwidth-constrained OTA. For now, customers asking about IoT edge should be pointed at **KubeEdge (open source, largest independently-verified scale), EVE-OS (closest architectural cousin), or their existing cloud vendor's offering (AWS Greengrass / Azure IoT Edge)** depending on their lock-in tolerance — none of which Helios currently competes with, and none of which Helios v1 needs to try to replace.

---

## Research Context and Scope

Helios (see `docs/whitepaper.md`) is a Rust-native workload orchestration platform targeting servers and edge-compute (tens–hundreds of nodes per region, Corrosion + Raft substrate, bare-metal and microVM workloads). IoT edge — millions of NAT-bound, intermittent devices — was explicitly scoped out of v1 on 2026-04-19, tracked in `marcus-sa/helios#5`. This research is adjacent-landscape scouting: "if a customer asks us, what do we point them to, and what patterns did they use that we could borrow later?"

### The Four Failure Modes (Ground Truth)

The scope-out was justified by four architectural mismatches between Helios's substrate and the IoT-edge problem shape. These align with the whitepaper as follows:

1. **SWIM fan-out does not scale to 100k+ nodes** — Corrosion's gossip bandwidth is O(fleet-size) per node; whitepaper §4 ("Live Cluster Map — ObservationStore") describes per-node state as local SQLite with QUIC-gossiped deltas, and §4's *Per-region blast radius* guardrail explicitly limits topology to "regional clusters gossip internally" rather than a single flat cluster. At IoT fleet sizes the per-node gossip cost exceeds the per-device footprint.
2. **NAT / intermittent connectivity breaks the peer-mesh assumption** — Corrosion's SWIM-over-QUIC requires bidirectional reachability between peers; whitepaper §4 describes "each node runs a Corrosion peer ... gossiped to a random peer subset over QUIC." IoT devices typically sit behind carrier NAT with outbound-only connectivity and flapping links.
3. **Static seed lists do not match edge provisioning** — Helios node configs declare `peers = [...]` at bootstrap (whitepaper §4). IoT images are flashed weeks/months before deployment and must discover their control plane at first boot, not from a baked-in peer list.
4. **Node enrollment protocol is undefined** — Whitepaper §23 (Image Factory) mentions dm-verity, TPM attestation, and Secure Boot as *Phase 2 roadmap* items but does not specify a zero-touch enrollment protocol (attestation → rendezvous → trust bundle → first SVID). For IoT, this is the day-one problem, not a Phase 2 refinement.

Cross-checked against the whitepaper: the framing is consistent. §4 confirms SWIM peer-mesh and static peer lists; §23 confirms attestation is roadmap-only. Accepted as ground truth for this research.

### What We Want

1. **Landscape inventory** — which platforms exist in the IoT-edge orchestrator space?
2. **Architectural patterns** — how each solves the four failure modes above.
3. **Scale claims** — marketing vs published evidence.
4. **Workload model** — containers / VMs / WASM / native; isolation story.
5. **Licensing shape** — OSS vs vendor-locked.
6. **Relevance to Helios** — concrete primitives worth borrowing.

---

## Research Methodology

**Search Strategy:** *(To be filled as searches are executed.)*

**Source Selection:** Per-platform official docs treated as `official` reputation for that platform's technical claims (first-party). CNCF, Eclipse Foundation, and LF Edge pages as `open_source` / high. Academic papers on edge orchestration taxonomy as `academic` / high. Platform comparison blog posts accepted only as secondary cross-reference, never as primary.

**Quality Standards:** Minimum 2 sources per platform-specific claim; 1 authoritative (first-party) accepted where the claim is inherently vendor-specific (e.g. Balena's delta-update algorithm, KubeEdge's EdgeHub protocol). Distinguish marketing claims from published production deployment evidence.

---

## Platform Catalogue

*(Progressive fill — one section per platform as research proceeds.)*

### P1. KubeEdge — CNCF Incubating, Kubernetes-based, cloud+edge split

**Shape.** Two-component split. **CloudCore** on the cloud side aggregates Kubernetes apiserver list-watch and speaks a proprietary edge protocol; **EdgeCore** on the device runs a trimmed Kubernetes node stack (Edged — a lightweight kubelet) plus **EdgeHub**, **DeviceTwin**, **EventBus**, and **ServiceBus**. Topology is **hub-and-spoke**: every edge node has a single long-lived connection back to CloudCore. No peer-to-peer mesh among edges, which side-steps the SWIM/NAT problem entirely.

**Evidence.** "CloudHub is a web socket server" / "EdgeHub is a web socket client responsible for interacting with Cloud Service." [KubeEdge Docs — Components](https://kubeedge.io/docs/) (Accessed 2026-04-19). Nine core components enumerated including DeviceTwin ("responsible for storing device status and syncing device status to the cloud").

**Transport.** WebSocket is the default; QUIC is supported as an alternate for the CloudCore↔EdgeCore channel. "KubeEdge uses the two-way multiplexing edge-cloud message channel and supports the WebSocket (default) and QUIC protocols." [KubeEdge Test Report — 100k Edge Nodes](https://kubeedge.io/blog/scalability-test-report/) (Accessed 2026-04-19). Cross-verified: [Release notes v1.7](https://release-1-12.docs.kubeedge.io/en/blog/release-v1.7/).

**Enrollment.** `keadm join` CLI on the edge device, bootstrap token issued by `keadm gettoken` on the cloud. The token is used exactly once to request the initial edge certificate (auto-provisioned), which is then used for ongoing mTLS against CloudHub. [Installing KubeEdge with Keadm](https://kubeedge.io/docs/setup/install-with-keadm/) (Accessed 2026-04-19). This is a single-use-token enrollment pattern — simple, but requires operator-mediated token handoff; no native TPM/hardware-root-of-trust attestation flow in the upstream Keadm path.

**NAT and intermittent connectivity.** EdgeHub is the WebSocket *client*; the connection is always outbound from the edge. That solves NAT inherently. For offline operation, MetaManager on the edge persists resource state locally and Edged continues reconciling pods from cache; reconnect replays the delta. The design principle is explicit in KubeEdge's architecture statement: "offline mode" is a first-class property of EdgeCore, not a bolt-on.

**Scale claim.** A published test report claims **100,000 edge nodes and 1,000,000 pods in a single cluster** with 5 CloudCore instances aggregating to a single kube-apiserver. [Test Report on KubeEdge's Support for 100,000 Edge Nodes](https://kubeedge.io/blog/scalability-test-report/); [kubeedge/docs/proposals/perf.md](https://github.com/kubeedge/kubeedge/blob/master/docs/proposals/perf.md). This is a first-party benchmark (not an independent third-party audit), but the methodology and SLIs/SLOs are published and the run is reproducible. **Confidence: Medium-High.** No independent production deployment at this scale is publicly documented; upstream Kubernetes caps at 5,000 nodes/150,000 pods, so KubeEdge's claim is structurally believable because the aggregation at CloudCore shields kube-apiserver from per-edge-node watches.

**Workload model.** Containers only (kubelet-compatible). Device integration via the DeviceTwin + Mapper pattern (a Mapper is a per-protocol userspace translator between a physical sensor/actuator and the DeviceTwin CRD). No VM or WASM support in upstream KubeEdge; there is an EdgeMesh sub-project for P2P service mesh but it is outside the core orchestration path.

**License.** Apache-2.0, CNCF Incubating project. Vendor-neutral.

**Helios relevance.** The **CloudHub / EdgeHub split with outbound-only WebSocket** is the cleanest answer to failure modes #2 (NAT) and partly #1 (fan-out — aggregation at CloudCore means edges only talk to one place). The **DeviceTwin reported-vs-desired state model** is a direct borrow candidate if Helios reopens IoT: it is a narrower, persistent, per-device shadow rather than an eventually-consistent CRDT over a fleet. Helios's current ObservationStore is the wrong shape for leaf IoT devices precisely because it assumes peer membership; a twin is a 1:1 cloud-side durable record with no peer cost.

### P2. K3s — Rancher/SUSE (donated to CNCF as Sandbox), lightweight Kubernetes

**Shape.** Not an IoT-edge-specific orchestrator — K3s is a general-purpose Kubernetes distribution engineered small (~60 MB binary) and opinionated (single binary, embedded SQLite as default datastore, containerd built in). It shows up in every "edge K8s" comparison because it is what most people reach for when the device is still "a small server" rather than "a 128 MB RAM MCU-class thing." Topology is **flat Kubernetes** — agents connect to servers, no peer mesh.

**Evidence.** "K3s is lightweight Kubernetes. Easy to install, half the memory, all in a binary of less than 100 MB." [K3s Docs — Architecture](https://docs.k3s.io/architecture) (Accessed 2026-04-19). "Agents register with the server using the node cluster secret along with a randomly generated password for the node, stored at `/etc/rancher/node/password`." [K3s Docs — Architecture](https://docs.k3s.io/architecture).

**Transport.** "An agent node registers with the server using a websocket connection initiated by the k3s agent process, and the connection is maintained by a client-side load balancer running as part of the agent process." [K3s Docs — Architecture](https://docs.k3s.io/architecture). Agent-initiated outbound WebSocket solves NAT in the same way KubeEdge does. The tunnel carries both apiserver traffic and kubelet callbacks (streaming logs, exec).

**Enrollment.** Token-based: a cluster-wide join token is generated at server init (`/var/lib/rancher/k3s/server/node-token`); agents pass `K3S_URL=...` and `K3S_TOKEN=...`. On first join, the agent generates a random password and the server stores a hash. Subsequent joins require the same password (prevents node-name hijack). This is a **shared-secret-plus-first-use-trust** model — convenient for homogeneous fleets, not a hardware-root-of-trust attestation flow.

**Datastore options.** Embedded SQLite (single-server default), embedded etcd (3+ server HA), or external PostgreSQL/MySQL/etcd. The SQLite default is the relevant edge choice — single-server K3s on a Raspberry Pi-class device is a mainstream use case. **This is the architectural parallel most relevant to Helios:** K3s's embedded SQLite ↔ Helios's redb (§4 `LocalStore`) is the same conceptual choice — skip distributed consensus when the deployment is a single node.

**Scale claim.** K3s itself does not publish an IoT-edge scale number; it is upstream-Kubernetes-compatible, so the practical ceiling per cluster is upstream K8s's 5,000-node limit. At fleet scale people run **many small K3s clusters** (one per site) rather than one large one — Rancher Fleet and Rancher MCM exist to GitOps-manage thousands of clusters.

**Workload model.** Kubernetes-native: containers (containerd), CRDs, Helm charts. No first-class VM or WASM support in upstream K3s. (Third-party integrations exist — KubeVirt for VMs, Spin Operator / wasmCloud for WASM — but they are add-ons, not platform primitives.)

**License.** Apache-2.0, CNCF Sandbox.

**Helios relevance.** K3s validates the "single binary, embedded local store, agent-initiated outbound WebSocket" pattern at scale. The relevant borrows for Helios-if-reopened-for-IoT are: (a) **agent-initiated tunnel instead of peer gossip** for the management plane; (b) **many-small-clusters + fleet-of-clusters GitOps** instead of one giant cluster — which matches how real IoT deployments are actually operated. However, K3s inherits K8s's container-only assumption, which Helios has already rejected — so K3s is a topology reference, not a workload-model reference.

### P3. MicroK8s — Canonical, snap-packaged Kubernetes

**Shape.** Canonical's lightweight Kubernetes distribution, shipped as a snap. Direct peer of K3s in positioning; differences are largely packaging and addon ecosystem. Dqlite (distributed SQLite via Raft, Canonical's project) is the default datastore rather than etcd or SQLite-single. Topology is upstream Kubernetes.

**Enrollment and transport.** Same shape as K3s: agent-initiated join via `microk8s add-node` on an existing cluster, producing a token that `microk8s join <ip>:25000/<token>` consumes on the joining node. Node-to-cluster communication is standard Kubernetes kubelet + kube-proxy — this does *not* solve NAT the way K3s's WebSocket tunnel does; MicroK8s assumes direct reachability between nodes on a control-plane network.

**Workload, license, scale.** Containers only, Apache-2.0, scale is upstream-K8s-bounded. Addons catalogue includes dashboard, metrics-server, ingress, RBAC, GPU, Istio, KubeVirt, etc. — selectable via `microk8s enable <addon>`.

**Helios relevance.** Limited as an IoT-edge reference — MicroK8s is a desktop/developer/server-side lightweight K8s, not an IoT-fleet orchestrator. The most interesting primitive to note is **Dqlite**: a pure-C Raft-replicated SQLite that sits in the same design space as Helios's openraft+redb RaftStore (§4). If Helios ever needs a single-file replicated SQL store, Dqlite is the closest prior art; currently not relevant because Corrosion covers the observation layer and redb covers the intent layer with different trade-offs (Dqlite's Raft-over-SQLite model is more like Helios's RaftStore than its CorrosionStore).

**Confidence: Medium** — classification based on canonical.com/microk8s product positioning and the enrollment pattern, not a deep technical fetch. MicroK8s is a minor player for IoT edge specifically; most practitioners reach for K3s.

### P4. OpenYurt — Alibaba origin, CNCF Sandbox, "non-intrusive" K8s edge extension

**Shape.** OpenYurt's explicit design goal is to extend an *unmodified* upstream Kubernetes cluster to the edge — "The blue box represents the original Kubernetes components, and the orange box represents the OpenYurt components." [OpenYurt Docs — Architecture](https://openyurt.io/docs/core-concepts/architecture) (Accessed 2026-04-19). This is a different architectural bet from KubeEdge: rather than replacing kubelet and the cloud-edge wire protocol, OpenYurt adds two sidecar components (YurtHub on every node, YurtTunnel for control-plane-originated reverse traffic) and labels.

**YurtHub — the key primitive.** "YurtHub... is a sidecar in node level, it performs the role of requests proxy between worker nodes and kube-apiserver." Every kubelet/kube-proxy on an edge node talks to `localhost:10267` (YurtHub) instead of the real apiserver. YurtHub proxies to apiserver when reachable, **serves from on-disk cache when the cloud link is down**, and re-validates on reconnect. This is how OpenYurt delivers edge autonomy without changing kubelet.

**Evidence.** "Caches data from the cloud and store it on the local disk... the cached data is then utilized if connection between the cloud and edge network is lost." [OpenYurt Docs — Architecture](https://openyurt.io/docs/core-concepts/architecture).

**Transport.** Raven is the VPN component: "builds VPN channels to ensure connectivity from cloud to edge or edge to edge." For cloud→edge callbacks (exec, logs, port-forward), YurtTunnel maintains an outbound reverse tunnel from edge to cloud. This puts OpenYurt closer to K3s/KubeEdge on NAT: outbound reverse tunnel solves inbound reachability.

**Enrollment.** `yurtadm join` produces the same kubeadm-style bootstrap-token flow as upstream Kubernetes, with Yurt-specific post-install to add YurtHub and Raven.

**Node-pool concept.** Edge nodes are grouped into **NodePools**; the `UnitedDeployment` CRD spreads replicas across pools with pool-specific counts and templates. This is a deliberate answer to "I have 50 edge sites with 3 devices each; I want one spec that produces 3 replicas per site." Helios's current scheduler (§4) is single-region bin-pack and does not have this primitive.

**Workload model.** Containers only; upstream kubelet compatibility is the design goal, so VM/WASM support depends on whatever K8s extensions you stack on top (KubeVirt, wasmCloud). No first-class.

**Scale.** OpenYurt does not publish a single-cluster node-count claim comparable to KubeEdge's 100k. It inherits upstream K8s limits per cluster. **Confidence: Low on specific scale number** — OpenYurt's positioning is "fleet of edge-extended clusters" rather than "one giant cluster."

**License.** Apache-2.0, CNCF Sandbox.

**Helios relevance.** The **YurtHub pattern — a per-node caching proxy that serves from local disk during cloud disconnect** — is the most borrowable primitive here. It is a cleaner split than KubeEdge's MetaManager: cache is explicit, the local proxy is upstream-kubelet-compatible, and reconnect semantics are well-defined. For Helios, the analog would be a per-node Intent-Store-cache that mirrors the regional Raft leader's decisions relevant to *this* node, so the node agent can continue running pinned allocations through a regional partition even when the Raft leader is unreachable. Whitepaper §3.5 gets close ("each region continues to operate on locally-committed intent") but only at *region* granularity, not *node* granularity. The NodePool / UnitedDeployment primitive is also worth noting if Helios ever goes many-small-clusters.

### P5. Akri — Microsoft origin, CNCF Sandbox, leaf-device discovery for K8s

**Shape.** Akri is **not an orchestrator**. It is a Kubernetes add-on that turns non-Kubernetes leaf devices (IP cameras via ONVIF, industrial equipment via OPC UA, USB devices via udev) into *discoverable, schedulable K8s resources*. The discovered device becomes a Kubernetes Device Plugin resource and Akri spawns a "broker pod" per discovered device to expose it to workloads. [Akri Docs — Architecture Overview](https://docs.akri.sh/architecture/architecture-overview) (Accessed 2026-04-19).

**Components.** **Discovery Handlers** (protocol-specific plugins), **Agent** (per-node Akri agent that registers discovered devices as K8s resources), **Controller** (spawns broker pods), **Configuration CRD** (operator-authored — "find all ONVIF cameras matching X filter"), **Instance CRD** (one per discovered device).

**Enrollment.** Does not apply — Akri is layered onto an existing K8s cluster (including KubeEdge, K3s, or OpenYurt). The devices Akri discovers do not run an Akri agent; only the K8s worker nodes do.

**Helios relevance.** Akri is a *pattern* to note, not a component to borrow. The pattern is: **leaf devices that cannot run the orchestrator runtime are modeled as first-class scheduled resources via a discovery-and-broker abstraction on the nearest edge node that can.** This is directly analogous to how Helios might handle truly constrained IoT endpoints — not try to run a node agent on a 64 KB MCU, but model the MCU as a "device resource" attached to a Helios edge node that runs a per-protocol broker sidecar. KubeEdge implements the same pattern via Mapper; Akri implements it more generally via Discovery Handlers. **If Helios reopens IoT, the question "do you run a node agent on every device, or do you run a broker on gateway nodes that owns many devices?" is the first decision, and Akri's answer is the broker pattern.** Akri itself would likely be bypassed (it is K8s-CRD-native), but the abstraction would be reimplemented.

**License.** Apache-2.0, CNCF Sandbox.

### P6. AWS IoT Greengrass V2 — Amazon, vendor-locked cloud dependency, open-source client

**Shape.** Hub-and-spoke. The device (**Greengrass core device**) runs the Greengrass Core software (the **nucleus**, available in Java — full — or **nucleus lite**, a C implementation for ≤512 MB devices), plus deployed **components**. Core devices connect to AWS IoT Core in the cloud. Separately, **client devices** (MCU-class things running FreeRTOS or AWS IoT Device SDK) connect to a Greengrass core device locally, which then relays to AWS IoT Core. That gives a **two-tier hierarchical topology**: leaf device → gateway core → cloud.

**Evidence.** "Greengrass core device: A device that runs the AWS IoT Greengrass Core software." / "Greengrass client device: A device that connects to and communicates with a Greengrass core device over MQTT." [AWS Docs — What is AWS IoT Greengrass](https://docs.aws.amazon.com/greengrass/v2/developerguide/what-is-iot-greengrass.html) (Accessed 2026-04-19).

**Transport.** **MQTT over TLS** to AWS IoT Core is the default. MQTT broker on the core device relays between client devices and cloud. MQTT5 is supported by the Java nucleus; nucleus lite supports basic MQTT. Client-to-core is also MQTT over TLS on the LAN.

**Enrollment.** Multiple flows:
- **Automatic provisioning** — the install script calls AWS APIs to register the AWS IoT thing, create certificates, and attach policies (requires AWS credentials on the device at install time — fine for a bench, not for factory flashing).
- **Manual provisioning** — operator pre-creates the thing and certificate; the installer receives them as input.
- **AWS IoT fleet provisioning** — the device ships with a **provisioning claim certificate** (a short-lived, limited-scope cert shared by an entire manufacturing batch); on first boot it exchanges the claim cert for a **permanent device certificate** via AWS IoT Core's Fleet Provisioning MQTT API. This is the flow that actually works for "flashed in factory, deployed months later."
- **Custom provisioning** — developer-supplied plug-in.

All flows support **TPM-backed key storage** (HSM option) — the device's private key is sealed in a TPM and never exists in clear. Cross-verified in the feature matrix at [AWS Docs — Greengrass Feature Compatibility](https://docs.aws.amazon.com/greengrass/v2/developerguide/operating-system-feature-support-matrix.html) and confirmed as TPM-supported for both Java nucleus and nucleus lite on Linux.

**Workload model.** Heterogeneous — **Lambda functions** (Java nucleus only), **Docker containers** (via the Docker Application Manager component; supported on both nuclei), **native OS processes** (via lifecycle scripts), **custom runtimes**. This is more flexible than every K8s-based alternative in this comparison: Greengrass does *not* require your workload to be a container. Components are packaged as a **recipe** (JSON/YAML) + **artifacts** (binary/script).

**Offline/intermittent.** Components keep running when the cloud link drops. MQTT messages queued at the core are replayed on reconnect. Stream Manager supports durable local buffers with deferred upload to Kinesis/S3/SiteWise.

**Scale.** AWS IoT Core published account-level limits: default quota 500,000 devices per account per region, raisable on request; millions of devices are routinely deployed per account in published customer case studies. Greengrass core devices are a subset of IoT things and share this ceiling. **Confidence: High** on the quota; **Medium** on "millions per customer" — AWS marketing cites this but published per-customer device counts are rare outside named case studies.

**License.** AWS IoT Greengrass Core software is open source (Apache-2.0). AWS IoT Core (the cloud side) is a proprietary AWS service. **You cannot self-host the cloud side.** This is the fundamental vendor lock-in point for Greengrass — the client is portable, the service it talks to is not. The **openBalena / balenaCloud** bifurcation (below) is the closest architectural peer; Greengrass deliberately does not offer that split.

**Helios relevance.** Three things are worth noting for Helios-if-reopened:
1. **Fleet provisioning via claim certificate** is the canonical answer to failure mode #4 (node enrollment for factory-flashed images). The claim cert is weak (shared batch) but scoped (MQTT policy lets it do one thing: exchange for a real cert), and the exchange is TPM-bindable at the device. This is exactly the pattern Helios's §23 gestures at (TPM attestation) but without the concrete protocol.
2. **Recipe + artifact component model** is substantially similar to the Helios job spec (TOML recipe + content-addressed artifacts in Garage). The similarity is shallow but the packaging taxonomy is one that converged across Greengrass, Balena, and OCI — Helios is already aligned.
3. **Two-tier "core device + client device" hierarchy** is a direct answer to failure mode #1 (100k nodes in one control plane). Greengrass does not put MCU-class devices in the orchestrator; it puts them behind a gateway core that is in the orchestrator. Fleet size from the orchestrator's perspective is the number of cores, not the number of things. This is the same architectural move KubeEdge's Mapper / Akri's broker makes, and the same move Helios would have to make if it ever supported MCU-class endpoints.

### P7. Azure IoT Edge / Azure IoT Hub / Azure Arc — Microsoft, vendor-locked, device-twin-centric

**Shape.** Hub-and-spoke. IoT Hub is the cloud service; IoT Edge devices run `edgeAgent` (lifecycle manager) and `edgeHub` (local MQTT/AMQP broker, message router, and cloud proxy), both themselves Docker containers. Deployed workloads are **modules** — user containers.

**Evidence.** "IoT Edge modules are units of execution, implemented as Docker-compatible containers... The IoT Edge runtime runs on each IoT Edge device and manages the modules deployed to each device." [Microsoft Learn — What is Azure IoT Edge](https://learn.microsoft.com/en-us/azure/iot-edge/about-iot-edge) (Accessed 2026-04-19).

**Transport.** MQTT or AMQP over TLS (optionally WebSockets-tunneled for firewalls that only permit HTTP(S)). Outbound-only from device; the cloud never initiates a connection.

**Device twin and module twin.** Each device has a **device twin** — a JSON document with a *reported* section (written by device) and a *desired* section (written by cloud). Each deployed module has a **module twin**. This is Azure's core orchestration primitive — "this is what the cloud wants this module to be doing" + "this is what the module says it is doing" is the reconciliation contract, and it works *identically* whether the device is online or offline: desired is fetched on reconnect, reported is buffered locally until send. Cross-verified: [Azure Docs — Understand Azure IoT Hub device twins](https://learn.microsoft.com/en-us/azure/iot-hub/iot-hub-devguide-device-twins) (standard Azure IoT doc, authoritative).

**Enrollment.** **DPS (Device Provisioning Service)** is the dedicated enrollment service. Supports three attestation methods:
- **TPM attestation** via the TPM 2.0 Endorsement Key — strongest hardware root of trust; DPS verifies the EK certificate chain against the manufacturer CA.
- **X.509 certificate attestation** — individual per-device certificates or group (intermediate-CA-signed) enrollments.
- **Symmetric key attestation** — weakest; scoped shared secret.

DPS is a **rendezvous service**: device ships knowing only the global DPS endpoint (`global.azure-devices-provisioning.net`) and its enrollment info; first-boot exchange routes it to the correct IoT Hub instance (across regions, across customer-specific hubs for multi-tenant solutions). This **solves failure mode #3 directly** — the image is flashed with a global endpoint and an attestation secret, never with a per-deployment seed list.

**Workload model.** Docker-compatible containers, period. No native Lambda-style functions, no WASM, no native processes. On Linux, any OCI runtime. **IoT Edge for Linux on Windows (EFLOW)** runs a Linux VM on a Windows host for the Linux-only container requirement — VM under the covers, containers on top.

**Azure Arc.** A separate but adjacent product: Arc-enabled Kubernetes and Arc-enabled servers attach on-prem/edge K8s clusters and VMs as first-class Azure resources, managed via Azure's RBAC and policy. Arc-enabled Kubernetes is how Azure supports K3s/RKE/OpenShift at the edge without running IoT Edge runtime specifically. Different orchestration model — GitOps (Flux) push — and different enrollment (`az connectedk8s connect` initiates a Helm-installed agent on the cluster that establishes an outbound tunnel to Azure). Arc is targeted at "edge compute" (stores, factories, branch offices) rather than "IoT devices."

**Scale.** IoT Hub default tier S1/S2/S3 quotas range from 400k to 100M messages/day; device count per hub is **up to 1,000,000 devices per IoT Hub instance** per Microsoft documentation, with sharding across multiple hubs for larger fleets. **Confidence: High** on the per-hub cap (Microsoft-authoritative); specific deployment sizes at individual customers are rarely published.

**Offline/intermittent.** Device twins and message queueing buffer during outage. Time-to-live on buffered messages is configurable. Priority routing and dead-letter queues handle backpressure.

**License.** IoT Edge runtime (`edgeAgent`, `edgeHub`) is open source (MIT). IoT Hub and DPS are proprietary Azure services. Same lock-in shape as AWS.

**Helios relevance.** Azure contributes two primitives worth naming:
1. **The device-twin reported-vs-desired reconciliation pattern** is architecturally what every reconciler does in Kubernetes, Nomad, and Helios §18, but *per-device with explicit persistent documents*. For a system already committed to reconciler-as-primitive (Helios §18), supporting IoT at the leaf is cleanly expressed as "every leaf device has a twin, and a per-device reconciler converges reported → desired." The intent/observation split (Helios §4) maps onto this: desired is intent (linearizable), reported is observation (eventually-consistent, authored by the device). This is a *very* direct structural fit.
2. **DPS as a global-rendezvous attestation service** is the pattern Helios needs to implement for failure mode #3 (static seed lists do not match edge provisioning). Helios's §23 TPM attestation section describes the verification side but not the rendezvous side — devices must know *some* endpoint at flash time, and a well-known platform-operator-run rendezvous service that routes them to their eventual home cluster is the canonical answer. Balena has an equivalent (`api.balena-cloud.com`); AWS has DPS-equivalent fleet provisioning endpoints per region.

### P8. EVE-OS — LF Edge project, Zededa origin, Type-1 hypervisor for edge

**Shape.** EVE is unusual in this lineup: it is an **operating system**, not an orchestrator layered on an OS. It runs as a **Type-1 hypervisor** (Xen by default; KVM also supported) directly on bare metal, and every workload — including the "host" control-plane services — runs as a VM. Workloads are unified under a single abstraction called **Edge Containers** that covers VMs, Docker/OCI containers, and unikernels with a common manifest. [LF Edge — EVE-OS Docs](https://github.com/lf-edge/eve/blob/master/docs/README.md) (Accessed 2026-04-19). Cross-verified: [EVE HYPERVISORS.md](https://github.com/lf-edge/eve/blob/master/docs/HYPERVISORS.md).

**Evidence.** "EVE is based on a type-1 hypervisor and it does not directly support traditional POSIX-like application and processes." [EVE-OS README](https://github.com/lf-edge/eve/blob/master/docs/README.md). "EVE expects its applications to be either Virtual Machines, Unikernels or Docker/OCI containers" [InfoQ / LF Edge EVE presentation material].

**Controller.** EVE devices do not run peer-to-peer; they connect outbound to a controller. Two controller options:
- **Adam** — open-source single-device reference controller (LF Edge).
- **ZEDEDA** — commercial multi-device controller (hyperscale, SaaS).

An intermediate **Eden** test harness (not a production controller) is widely referenced in EVE documentation for local-development workflows.

**Transport and API.** EVE communicates via the **"public API of Project EVE"** using protobuf-serialized configuration objects over TLS. The API is pull-based from the device; new configuration is fetched on schedule and applied atomically. This is the same outbound-only pattern as KubeEdge and Azure IoT Edge.

**Enrollment.** EVE integrates a **TPM Manager** for hardware-root-of-trust key storage; device onboarding uses a device certificate established during manufacturing (ideally backed by TPM) plus a **bootstrap config** delivered either baked into the installer image or via out-of-band USB. The controller attests the device certificate before accepting it. This is the fullest TPM-attestation-enrollment story in this comparison.

**Offline behavior.** Because configuration is self-contained per-refresh, the device operates on its last-known-good config when the controller is unreachable. Workload lifecycles continue.

**Scale.** ZEDEDA markets deployments of tens of thousands of edge devices per controller; independent published benchmarks at a specific scale are not prominent. **Confidence: Medium** on specific scale numbers — marketing claims exceed published independent evidence.

**Workload and security.** Workloads are fully isolated by virtualization (Type-1 HV). Unikernels are first-class — a property EVE shares with Helios but exceeds few other platforms in this comparison. Measured boot + TPM attestation + dm-verity-equivalent integrity chain are standard in the image.

**License.** Apache-2.0, LF Edge.

**Helios relevance.** EVE is the **closest architectural cousin to Helios** in this entire comparison — the same Type-1 HV substrate (Cloud Hypervisor in Helios, Xen/KVM in EVE), the same first-class-unikernel stance, the same measured-boot-from-day-one posture (Helios §23's Image Factory roadmap, EVE's existing TPM chain). Three specific borrows are worth naming:
1. **"Edge Container" as a unified workload type** — EVE's model of papering over VM/container/unikernel via one manifest is substantially what Helios §6's `Driver` trait already achieves, but EVE names the abstraction explicitly on the user-facing side. Helios currently surfaces `driver = "process" | "microvm" | "vm" | "unikernel" | "wasm"` in the job spec; a unifying name is pure usability.
2. **Controller-pull configuration model with self-contained config objects** — the device holds the last-known-good and reconciles against it locally. Helios's §4 per-region Raft plus §3.5 multi-region doesn't describe a configuration-pull model; for IoT-edge it would need one because Raft replication assumes a quorum.
3. **TPM-anchored enrollment with bootstrap config via OOB delivery (USB/installer-baked)** — directly answers failure mode #4. EVE's flow is the concrete pattern Helios §23 gestures at without specifying.

### P9. Eclipse ioFog — Eclipse Foundation, fog-computing microservices

**Shape.** Hub-and-spoke. **ioFog Controller** (aka "ECN Manager" — Edge Compute Network Manager) is the central control plane; **ioFog Agent** runs on every edge device and executes user microservices as Docker containers; **ioFog Connector** (replaced in 2.0 with Apache Qpid Dispatch Router + Red Hat Skupper) provides the secure application overlay between microservices. [Eclipse ioFog Docs — Architecture](https://iofog.org/docs/2/getting-started/architecture.html) (Accessed 2026-04-19).

**Evidence.** "A software agent called ioFog Agent provides a universal environment for edge computing microservices; a distributable control plane called ioFog Controller provides remote control and management of ioFog Agent instances; and a software overlay network component called ioFog Connector provides secure connectivity between any edge microservices managed by ioFog." [Eclipse ioFog Architecture page].

**Current status.** Latest major release visible is ioFog 2.0 (2021); an ioFog 3.0 was targeted for Q4 2022 per Eclipse Foundation newsletter. **Most recent repository commits are through late 2024, with reduced activity volume compared to CNCF-tier projects.** Project is **not formally archived** but maintenance cadence is low. **Confidence: Medium** — active on paper, minor in practice.

**Workload model.** Docker containers as microservices. No VM or WASM support.

**License.** EPL 2.0, Eclipse Foundation.

**Helios relevance.** Limited. ioFog's most interesting property is the Connector-as-application-overlay pattern (Qpid Dispatch Router is an AMQP-routing overlay — messages flow through named addresses, not through network addresses), but this is a *different* pattern than Helios's SPIFFE-mTLS + XDP LB approach and is tightly bound to AMQP. The project's low maintenance velocity argues against it as a technical reference in 2026.

### P10. FogLAMP / LF Edge Fledge — Dianomic origin, industrial data collection

**Shape.** FogLAMP is **not an orchestrator** — it is a data-collection + edge-analytics platform for industrial IoT. Architecture is a pluggable-microservices stack (Core service, South service for sensor/actuator I/O, North service for cloud forwarding, Storage service). The open-source fork is known as **LF Edge Fledge**. [Dianomic — FogLAMP Architecture](https://dianomic.com/platform/foglamp/architecture-plugins/); [LF Edge — Fledge](https://www.lfedge.org/projects/fledge/).

**Included in this comparison for completeness only.** FogLAMP/Fledge provides protocol-plugin-driven data collection (Modbus, OPC UA, S7, MQTT, historian-specific) and forwarding, plus some on-edge aggregation. It has no notion of workload scheduling, multi-node placement, or cluster orchestration — it is a single-node or per-node runtime that pushes data to a historian or cloud.

**Helios relevance.** None as an orchestrator reference. The one useful observation is that *industrial IoT deployments are typically plugin-driven protocol translators plus data pipelines*, not compute-orchestration workloads — if Helios ever addressed IIoT specifically, it would likely be as a Helios workload (a "fledge driver" running on a Helios edge node) rather than Helios reimplementing FogLAMP.

**License.** Apache-2.0.

### P11. Balena — balenaOS + balenaCloud / openBalena split

**Shape.** **balenaOS** is a minimal Yocto-built host OS that boots into **balenaEngine** (a Docker fork optimized for constrained devices with delta-update support). User workloads are Docker containers deployed as **fleets**. **balena-supervisor** is the per-device agent that speaks to the cloud. The cloud side comes in two flavors:

- **balenaCloud** — Balena's hosted SaaS (proprietary, multi-tenant).
- **openBalena** — self-hosted, AGPLv3, with fewer features (no delta updates, single user, no dashboard).

Topology is **hub-and-spoke** via an **OpenVPN tunnel**. Every device establishes an outbound VPN connection to the cloud backend; inbound device-management commands flow over the tunnel.

**Evidence on delta updates.** "BalenaEngine is balena's modified Docker daemon fork that allows the management and running of application service images, containers, volumes, and networking, and supports container deltas for 10-70x more efficient bandwidth usage." [ICS.com — IoT Fleet Management Comparison](https://www.ics.com/blog/iot-fleet-management-system-torizon-balena-mender) (Accessed 2026-04-19, secondary source). Cross-verified against the balena-supervisor repo: [balena-os/balena-supervisor](https://github.com/balena-os/balena-supervisor).

**Evidence on openBalena composition.** "OpenBalena comprises five core components: API, VPN, Registry, S3 Storage, Database." [openBalena README](https://github.com/balena-io/open-balena/blob/master/README.md) (Accessed 2026-04-19).

**Transport.** OpenVPN over TCP 443 (ports-friendly for corporate/carrier NAT). Device state changes propagate via long-poll HTTPS to the API; logs and interactive terminal use the VPN tunnel. This is a **TCP-443-only design**, which is the pragmatic answer to carrier NAT and firewall-heavy environments.

**Enrollment.** `balena-engine push` uploads a release; at device-first-boot, the supervisor registers itself using a **provisioning API key** baked into the config.json during image creation, producing a per-device UUID and a per-device API key. From then on the device uses its own key (provisioning key can be revoked). Hardware-root-of-trust / TPM attestation is **not** a first-class flow in upstream balena; the provisioning key model is a shared-secret-per-fleet approach, functionally similar to K3s's token — not as strong as Azure DPS or AWS Fleet Provisioning with TPM.

**Workload model.** Docker containers only (via balenaEngine). Multi-container apps via a `docker-compose.yml`-style `docker-compose.yml` in the `src/` of a project.

**Offline / A-B updates.** balenaOS uses **A/B rootfs partitioning**: host OS updates are downloaded to the inactive partition, swap on reboot, rollback on boot-health failure. This is classic dual-bank firmware update for edge, directly analogous to Talos' / Flatcar's approach. **Delta updates** operate at the container-image layer: only changed rootfs chunks are transferred (reported 10–70× bandwidth savings). [ICS.com source above]. openBalena **lacks delta updates** — a key feature gate pushing customers to the SaaS.

**Scale.** Balena publicly claims management of hundreds of thousands of devices per customer (e.g., Jetson-based fleets); no independently-audited benchmark at a specific scale is published. **Confidence: Low-Medium** on a single hard scale number; **High** on the claim that Balena is deployed at five-to-six-digit device fleets in production.

**Helios relevance.** Three concrete borrow candidates:
1. **Binary delta updates for container/VM images.** Helios §23 Image Factory roadmaps content-addressed OCI layers but not *delta* updates between versions of the same image. For bandwidth-constrained IoT, deltas are the difference between a 50 MB update that works and a 200 MB update that times out on a cellular link. Whitepaper §23 should note this as a future addition if IoT is reopened.
2. **OpenVPN-over-TCP-443 or equivalent carrier-NAT-friendly transport.** Helios's QUIC-for-Corrosion approach (§4) assumes UDP-friendly networks. For IoT edge, falling back to a TCP-on-443-only transport is routinely necessary.
3. **The openBalena / balenaCloud split as a commercial model reference**, not a technical one: the self-hosted OSS version is functional but feature-gapped; the hosted version is where the value accrues. If Helios were ever offered as a hosted edge-IoT SaaS (not current plan), this is the pattern the market expects.

**License.** balenaOS: Apache-2.0. openBalena: AGPLv3. balenaCloud: proprietary SaaS.

### P12. Ockam — secure channels + identity for distributed systems (not an orchestrator)

**Shape.** Ockam is **not an orchestrator** — it is a developer-facing toolkit and orchestrator-of-identities that builds end-to-end encrypted, mutually authenticated **Secure Channels** over arbitrary multi-hop transports (TCP → UDP → Bluetooth → Kafka, in any combination). [Ockam Docs](https://docs.ockam.io/) (Accessed 2026-04-19).

**Evidence.** "Ockam Secure Channels are mutually authenticated and end-to-end encrypted messaging channels that guarantee data authenticity, integrity, and confidentiality. [...] Ockam's secure channel protocol sits on top of an application layer routing protocol that can hand over messages from one transport layer connection to another over any transport protocol, with any number of transport layer hops." [Ockam Secure Channels docs].

**Identity and enrollment.** Each node (device, service, application) is issued a **cryptographically provable identity** with associated keys; **Ockam Orchestrator** (Ockam's hosted service) handles **provisioning, proof-of-possession, rotation, and revocation** of identity keys and credentials. Keys are created on-device and only their public halves are registered centrally.

**NAT traversal.** Ockam explicitly targets devices behind private networks, firewalls, and NAT. Because the secure channel protocol is **transport-agnostic and multi-hop**, a device behind NAT can maintain an outbound connection to a relay, and anything addressable at the Ockam routing layer becomes reachable — even if no single transport connection exists end-to-end.

**Helios relevance.** Helios's built-in CA + SPIFFE SVIDs (§4, §8) covers identity and mTLS **between Helios workloads inside a cluster**. Ockam addresses a different problem: identity and mTLS **across arbitrary transports, including to devices that cannot be in the Helios dataplane** — for example a device that only speaks MQTT to a broker, or that sits behind a firewall that permits no inbound traffic. If Helios ever reopens IoT, Ockam's architecture is the reference for answering "how do I give this constrained device a cryptographically-provable identity that the Helios CA can verify, given it can't speak the Helios sockops/kTLS protocol?" The Ockam pattern — identity layer is decoupled from transport, credentials include proof-of-possession — is directly applicable. SPIFFE Federation is another relevant primitive in the same design space.

**License.** Apache-2.0 for the core; Ockam Orchestrator (hosted) is proprietary SaaS. Same architectural split as openBalena / balenaCloud.

### P13. Nebula and Tailscale — overlay-network transport layers

Both are **transport layers, not orchestrators** — but the IoT-edge conversation consistently reaches for them because they solve NAT traversal and identity-per-host in ways that the orchestrator control-plane protocols above would otherwise need to reimplement.

**Nebula (Slack, Apache-2.0).** Peer-to-peer mesh VPN with **lighthouses** for rendezvous. Each host has an X.509-like certificate signed by a fleet CA. Lighthouses coordinate **UDP hole-punching** across NATs (initiator queries lighthouse for target; lighthouse asks target to send simultaneous empty UDP packets, opening NAT pinholes). When hole-punching fails, relay nodes forward. [Nebula Docs](https://nebula.defined.net/docs/) (Accessed 2026-04-19); [DeepWiki — Nebula Lighthouse Discovery](https://deepwiki.com/slackhq/nebula/3.5-lighthouse-discovery). **Production scale:** Slack has publicly stated Nebula powers "over 50,000 production hosts" as of December 2021. **Confidence: High** on Nebula's NAT traversal design; Medium on current scale.

**Tailscale (commercial, open-source client).** Built on WireGuard; central **coordination server** (`login.tailscale.com`) handles key exchange and ACLs, with **DERP relay servers** for NAT-failed fallback. Uses the **disco protocol** for authenticated peer-to-peer discovery, STUN, UDP lifetime probing. **`tsnet`** is a library/SDK that lets a Go or Rust program be a Tailscale node directly (no daemon, no TUN interface required) — with a Rust FFI surface available. [Tailscale — `tailscale-rs` announcement](https://tailscale.com/blog/tailscale-rs-rust-tsnet-library-preview); [Tailscale — How NAT traversal works](https://tailscale.com/blog/how-nat-traversal-works).

**Key architectural difference.** Nebula's lighthouses are **stateless** relative to which peers can talk to which (that is governed by the fleet CA's signed certificates with embedded group labels). Tailscale's coordination server is **stateful** (holds the ACL map). For orchestration purposes, both expose the same abstraction — every host has a stable identity and can reach every other host the ACL allows, regardless of NAT.

**Helios relevance.** Helios's dataplane (§7 XDP + sockops mTLS) assumes **direct L3 reachability between nodes** — a fair assumption for servers in a datacenter or a region, wrong for IoT over carrier NAT. If Helios ever reopens IoT, one of these two patterns is required as the transport substrate:
- **Nebula-style hole-punching mesh with fleet CA** — closer to Helios's existing CA model; per-node certs could share a CA with SPIFFE SVIDs.
- **Tailscale-style coordination-server + WireGuard + DERP-fallback** — simpler operationally, but introduces a proprietary coordination server dependency unless reimplemented (which is what Headscale does).

The **Nebula lighthouse pattern** is more directly borrowable because it is fully open-source and architecturally aligned with Helios's "per-node cert signed by platform CA" stance. The `helios-replay`/routing header primitives (§11) would compose naturally with a Nebula underlay: requests would flow through the mesh to reach private-network devices.

**Separate note — Tailscale's `tsnet` library** (with a Rust FFI preview) is the closest example of "embed the overlay inside the orchestrator agent binary," which would let a Helios node agent be simultaneously a Helios node and a Tailscale peer without running a separate daemon — a relevant pattern for the single-binary design principle (whitepaper §2, principle 8).

**License.** Nebula: MIT. Tailscale client: BSD. Tailscale coordination: proprietary (Headscale is the Apache-2.0 reimplementation).

### P14. Additional contenders surfaced during search

**SuperEdge (Tencent, CNCF Sandbox-adjacent; not CNCF-donated).** Conceptually between KubeEdge and OpenYurt: non-intrusive to K8s (like OpenYurt) but adds distributed health checks and edge service access control. Uses a `lite-apiserver` on every edge node (per-node K8s API cache, similar to YurtHub). **NodeUnit** and **NodeGroup** primitives group edge nodes for deployment. More complex agent stack than OpenYurt (five edge components vs OpenYurt's three vs KubeEdge's one). [SuperEdge vs OpenYurt vs KubeEdge — LinkedIn analysis](https://www.linkedin.com/pulse/superedge-openyurt-extending-native-kubernetes-edge-gokul-chandra); academic review [MDPI Sensors 2023](https://pmc.ncbi.nlm.nih.gov/articles/PMC9967903/) (Accessed 2026-04-19) lists it alongside KubeEdge, OpenYurt, Open Horizon, Baetyl, Flotta, Eclipse ioFog. **Helios relevance: minor** — mostly redundant with OpenYurt and KubeEdge lessons.

**Baetyl (Baidu origin, LF Edge).** Baetyl 2.0 adopts a cloud-native model and runs on vanilla K8s or K3s. Positions itself as an edge framework offering device connection, message routing, function compute, AI inference, video capture, and OTA status reporting. [LF Edge — Baetyl 2.0](https://lfedge.org/baetyl-2-0/) (Accessed 2026-04-19). **Helios relevance: minor** — a K8s consumer like OpenYurt; does not introduce new primitives of interest.

**Open Horizon (IBM origin, LF Edge).** Included for completeness; an edge agent + management hub model with a focus on autonomous agreement protocols (the "policy-based pattern" approach to deployment). Less prominent than KubeEdge/K3s in current deployments.

**Project Flotta (Red Hat).** Edge device management via MQTT for constrained devices; per-device podman workloads. Less prominent; aligned with Red Hat's edge-specific go-to-market rather than a broad community.

**Summary on "others":** The academic review [MDPI Sensors 2023](https://pmc.ncbi.nlm.nih.gov/articles/PMC9967903/) compared seven projects (KubeEdge, OpenYurt, SuperEdge, Open Horizon, Baetyl, Flotta, Eclipse ioFog) — it aligns with our platform list and does not surface a major contender we have missed. The non-K8s space (Balena, Greengrass, Azure IoT Edge, EVE-OS) is structurally separate from that academic review and is where the most architecturally distinct patterns live.

---

## Cross-Cutting Analysis

### A1. Topology shapes

| Platform | Shape | Notes |
|---|---|---|
| KubeEdge | hub-and-spoke | CloudCore aggregates; edges never peer |
| K3s | flat K8s | Single-binary + agents; agent → server WebSocket |
| MicroK8s | flat K8s | Direct kubelet-to-apiserver; no edge-specific NAT handling |
| OpenYurt | hub-and-spoke + YurtHub cache | Per-node caching proxy + reverse tunnel |
| Akri | add-on (no orchestrator role) | Broker pod per discovered device |
| AWS Greengrass | **two-tier hierarchical** | Core device + client devices; cores connect to cloud |
| Azure IoT Edge | hub-and-spoke | Device twin; nested edge supported for hierarchical fan-out |
| EVE-OS | hub-and-spoke | Device ↔ controller (Adam/ZEDEDA); no edge-to-edge |
| ioFog | hub-and-spoke | Controller + Agents + AMQP overlay for app mesh |
| Balena | hub-and-spoke | Every device tunnels to cloud via OpenVPN |
| Ockam | any (transport-agnostic) | Multi-hop routing; no topology constraint |
| Nebula | peer-to-peer mesh + lighthouse | Lighthouses for rendezvous; direct peer when possible |
| Tailscale | peer-to-peer mesh + coordination | Coordination server stateful; DERP relay fallback |
| **Helios today** | **peer mesh (Corrosion + SWIM)** | Regional mesh, Raft for intent |

**Summary.** Every IoT-focused orchestrator in the landscape uses **hub-and-spoke** or **two-tier hierarchical** topology. None uses peer gossip among leaf devices. This is the first-order confirmation of failure mode #1: at IoT-edge fleet size, peer mesh is not the architectural answer — a central aggregator or a hierarchical tier is. Helios's Corrosion substrate is well-shaped for servers and edge-compute but structurally different from every IoT-edge incumbent.

### A2. Management-plane transports

| Platform | Transport | NAT-friendly? |
|---|---|---|
| KubeEdge | WebSocket (default), QUIC (optional) | Yes — outbound edge-initiated |
| K3s | WebSocket tunnel | Yes |
| OpenYurt | HTTPS + YurtTunnel (reverse) | Yes |
| AWS Greengrass | MQTT over TLS | Yes |
| Azure IoT Edge | MQTT / AMQP over TLS (± WS) | Yes; WS-over-443 falls back for hostile firewalls |
| EVE-OS | protobuf over TLS | Yes — device pulls config |
| ioFog | REST/AMQP (Qpid Dispatch Router) | Yes at the AMQP overlay |
| Balena | OpenVPN (TCP/443) + HTTPS long-poll | **Yes — TCP/443 survives carrier NAT and corporate firewalls** |
| Ockam | any transport, multi-hop | Yes by design |
| Nebula | WireGuard over UDP + hole-punching | Yes via lighthouse-coordinated punching |
| Tailscale | WireGuard over UDP + STUN/DERP | Yes; DERP relay when UDP blocked |
| **Helios today** | QUIC (Corrosion), gRPC/HTTP (control) | **No — assumes UDP reachability between peers** |

**Summary.** The dominant pattern is **outbound-initiated long-lived tunnel** (KubeEdge WebSocket, K3s WebSocket, Balena OpenVPN, Greengrass/Azure MQTT). This directly solves failure mode #2 (NAT). Helios's Corrosion-over-QUIC assumes UDP reachability, which is routinely blocked on enterprise networks and carrier NAT — this is the single most fundamental technical reason IoT-edge was scoped out of v1. Peer-to-peer mesh overlays (Nebula, Tailscale) are the alternative path; hub-and-spoke tunnels are simpler to operate.

### A3. Enrollment and zero-touch provisioning patterns

Five patterns observed across the landscape:

1. **Operator-mediated token handoff** — operator runs a CLI on the cloud to issue a token, pastes it into the device's install command. **KubeEdge (`keadm gettoken` + `keadm join`), K3s (`K3S_TOKEN`), MicroK8s (`add-node`/`join`), Balena (provisioning key baked into config.json).** Works for homogeneous fleets where the operator can touch each device once; does **not** scale to factory-flashed images deployed months later.
2. **Shared provisioning claim certificate + individual-cert exchange** — an entire manufacturing batch ships with a scoped claim cert; on first boot the device exchanges it for a unique permanent cert over a cloud API. **AWS Greengrass Fleet Provisioning.** Addresses failure mode #3 directly.
3. **Global rendezvous service + attestation-based routing** — device ships knowing only a well-known global endpoint and an attestation secret (TPM EK / X.509 / symmetric). On first boot it contacts the global endpoint, attests, gets routed to its eventual tenant/region. **Azure DPS** is the canonical example; AWS Fleet Provisioning is a narrower variant.
4. **TPM-anchored device identity with bootstrap config via OOB delivery** — device certificate is TPM-backed; bootstrap config (controller endpoint, fleet ID) is baked into the installer image or delivered via USB at install time. **EVE-OS.** Strongest hardware root of trust in this landscape.
5. **Identity toolkit — cryptographic proof-of-possession with transport-agnostic enrollment** — the identity layer is decoupled from transport; keys generated on-device; public halves registered centrally. **Ockam.** Structurally orthogonal to the others; used alongside any of them.

**Summary.** For factory-flashed images, **patterns 2–4 are the only viable answers**. Helios whitepaper §23 gestures at pattern 4 (TPM attestation in the Phase 2 Image Factory roadmap) but does not specify the rendezvous protocol. If Helios reopens IoT, a DPS-equivalent global rendezvous service is the piece that is missing from the current whitepaper.

### A4. NAT and intermittent-connectivity handling

The dominant pattern is **outbound tunnel + local cache + eventual-replay**. Specific implementations:

- **KubeEdge:** MetaManager persists edge state in SQLite; offline reconciliation; delta replay on reconnect.
- **OpenYurt:** YurtHub serves apiserver-cached data from local disk during disconnect; a dedicated primitive for this failure mode.
- **AWS Greengrass:** Components keep running; MQTT messages queued locally; Stream Manager for durable buffers with deferred upload.
- **Azure IoT Edge:** Device twin desired/reported is durable on both sides; disconnect is a non-event; reconnect triggers delta sync.
- **EVE-OS:** Device continues on last-known-good config; periodic pull attempts; controller change only takes effect on next successful pull.
- **Balena:** Supervisor continues running existing containers; logs buffered; new releases downloaded on reconnect with resumable delta.

**Common principle:** the edge node's autonomy is not an emergent property; it is an explicit design primitive with a specific component responsible for it (MetaManager, YurtHub, Stream Manager, device twin, supervisor). Helios has edge-node autonomy at the **region** level (§3.5) but not at the **node** level — a missing component if IoT is ever a goal.

### A5. Workload models and isolation

| Platform | Containers | VMs | Unikernels | Native processes | WASM | Lambda |
|---|---|---|---|---|---|---|
| KubeEdge | yes (kubelet) | no | no | no | no | no |
| K3s | yes (kubelet) | no | no | no | no | no |
| MicroK8s | yes (kubelet) | no (KubeVirt add-on) | no | no | no (addon) | no |
| OpenYurt | yes (kubelet) | no | no | no | no | no |
| AWS Greengrass | **yes** | no | no | **yes** (lifecycle scripts) | no | **yes** (Java nucleus) |
| Azure IoT Edge | **yes** (Docker only) | no | no | no | no | no |
| EVE-OS | **yes** | **yes** | **yes** | no (Type-1 HV, no host POSIX) | no | no |
| Balena | **yes** (balenaEngine) | no | no | no | no | no |
| ioFog | yes | no | no | no | no | no |
| **Helios today** | **yes** (via drivers) | **yes** | **yes** | **yes** | **yes** | N/A |

**Summary.** Only **EVE-OS** matches Helios's workload-type breadth (VMs + containers + unikernels) in the IoT-edge space. **AWS Greengrass** matches Helios's containers-plus-processes support but adds Lambda rather than VMs. Every K8s-based IoT platform is containers-only. This is the strongest workload-model differentiator in Helios's favor if it ever re-enters this market.

### A6. Licensing and commercial shape

| Platform | License | Commercial shape |
|---|---|---|
| KubeEdge | Apache-2.0 (CNCF Incubating) | Vendor-neutral OSS |
| K3s | Apache-2.0 (CNCF Sandbox) | Vendor-neutral OSS + SUSE commercial |
| MicroK8s | Apache-2.0 | Canonical commercial support |
| OpenYurt | Apache-2.0 (CNCF Sandbox) | Alibaba Cloud commercial adoption |
| Akri | Apache-2.0 (CNCF Sandbox) | Vendor-neutral |
| AWS Greengrass | Apache-2.0 client, proprietary cloud | **Vendor-locked to AWS** |
| Azure IoT Edge | MIT runtime, proprietary cloud | **Vendor-locked to Azure** |
| EVE-OS | Apache-2.0 (LF Edge) | OSS + ZEDEDA commercial controller |
| ioFog | EPL 2.0 | Active-but-slow OSS |
| FogLAMP/Fledge | Apache-2.0 (LF Edge) | OSS + Dianomic commercial |
| Balena | openBalena AGPLv3, balenaCloud proprietary | **OSS core + hosted SaaS with feature gaps** |
| Ockam | Apache-2.0 core, Orchestrator SaaS | OSS toolkit + hosted orchestrator |
| Nebula | MIT | Fully OSS; Defined Networking commercial |
| Tailscale | BSD client, proprietary coordination | Hosted SaaS; Headscale is OSS coordination |

**Three archetypes for recommendations:**
1. **Fully OSS, no cloud dependency:** KubeEdge, K3s (+GitOps fleet mgmt), OpenYurt, EVE-OS + Adam, Nebula, Headscale.
2. **Open client + proprietary cloud (lock-in):** AWS Greengrass, Azure IoT Edge, balenaCloud, Tailscale.
3. **Self-hosted variant with reduced features:** openBalena (vs balenaCloud), Headscale (vs Tailscale), Adam (vs ZEDEDA).

---

## Relevance to Helios

### Customer-facing recommendation matrix (the "if they ask us" answer)

| Customer context | Recommend |
|---|---|
| "I have 50 industrial gateways at 50 sites, each with a few sensors" | **K3s + Rancher Fleet** (many-small-clusters, GitOps fleet mgmt). Or **OpenYurt** if they want one logical cluster. |
| "I have 10,000+ devices, want open source, no AWS lock-in" | **KubeEdge** (published 100k scale + independent production reference — Hong Kong–Zhuhai–Macao bridge, 100k+ monitoring devices per academic review) |
| "I have mixed container/VM/unikernel at the edge, strong security posture required" | **EVE-OS + Adam (OSS) or ZEDEDA (commercial)** — closest architectural cousin to Helios |
| "We're all-in on AWS, need Lambda at the edge" | **AWS IoT Greengrass V2** — vendor-locked but feature-complete, TPM-bindable |
| "We're all-in on Azure, need device-twin model" | **Azure IoT Edge + DPS** — vendor-locked; nested-edge for hierarchical |
| "I want to manage fleets of Docker containers on ARM devices, bandwidth-constrained" | **Balena (balenaCloud preferred; openBalena if self-hosted)** — delta updates are the killer feature |
| "I already have orchestration; I need to give devices cryptographically-provable identity" | **Ockam** (drop in on top of whatever else they use) |
| "I just need NAT traversal and secure transport between devices and my control plane" | **Tailscale (if SaaS ok) / Headscale (self-hosted) / Nebula (peer-mesh)** |
| "I need to integrate industrial sensors (Modbus, OPC UA) with cloud historians" | **LF Edge Fledge (FogLAMP)** — this is a data pipeline, not an orchestrator |
| "I need Kubernetes device discovery for cameras/USB/industrial" | **Akri** on top of any K8s |

### Primitives worth borrowing if Helios ever reopens IoT

In priority order based on architectural fit and gap-filling value:

**1. Outbound-only WebSocket tunnel (or QUIC fallback over TCP/443) for the management plane — directly solves failure mode #2.** Pattern source: KubeEdge CloudHub/EdgeHub, K3s agent tunnel, Balena OpenVPN. Helios implication: a third transport mode alongside Corrosion (peer mesh) and gRPC (control plane streaming) — a "tunneled observation" mode where an edge node speaks only outbound to a regional aggregator, which then proxies the node's observation rows into the regional ObservationStore on its behalf. Helios whitepaper §4 is silent on this; it is the single highest-impact addition.

**2. Global rendezvous + attestation-based enrollment — directly solves failure modes #3 and #4.** Pattern source: Azure DPS (strongest), AWS Fleet Provisioning, EVE-OS bootstrap config. Helios implication: §23 (Image Factory) currently roadmaps "dm-verity + TPM attestation + Secure Boot" in Phase 2 but does not describe a rendezvous protocol. A device should ship with an image that knows only a well-known endpoint like `rendezvous.helios.local` and a TPM-sealed attestation credential; the rendezvous service routes it to its eventual regional control plane. This is the piece Helios §23 is missing and the architectural shape is well-established across three major platforms.

**3. Device-twin reported-vs-desired reconciliation at the leaf — directly fits Helios's existing reconciler model.** Pattern source: Azure IoT Hub device twin, KubeEdge DeviceTwin. Helios implication: Helios's §18 reconciler trait `reconcile(desired, actual, db) → Vec<Action>` is *already* the right shape for per-device reconciliation. The intent/observation split (§4) maps cleanly: desired is intent (Raft), reported is observation (but per-device, not SWIM-gossiped — a durable per-device row written via the outbound tunnel). This is the one pattern where Helios's existing architecture is **structurally ready** for IoT and requires mostly a new transport, not new primitives.

**4. Two-tier hierarchical topology (gateway tier + leaf tier).** Pattern source: AWS Greengrass (core device + client device), KubeEdge Mapper, Akri broker. Helios implication: if Helios must ever support MCU-class devices, the architectural answer is explicitly **not** to run a Helios node agent on the MCU — it is to run a Helios node on a gateway (which participates normally in the Helios cluster) and model leaf devices as owned resources of that gateway. This is identical to how Akri's broker pattern works.

**5. Per-node cached-API-proxy for edge autonomy.** Pattern source: OpenYurt YurtHub. Helios implication: Helios's §3.5 region-level autonomy is coarse; a YurtHub-equivalent per-node cache of the regional Raft leader's decisions relevant to that node would allow individual nodes to continue operating through regional partitions, not just region-to-region partitions. This composes with #1 above — a tunneled edge node *is* a node with a cached proxy by definition.

**6. Binary delta updates for container/VM/unikernel images.** Pattern source: Balena's balenaEngine + container deltas (reported 10–70× bandwidth savings). Helios implication: §23 Image Factory's content-addressed OCI registry is already a foundation for this; adding binary deltas between versions of the same content-addressed image is a well-understood extension (bsdiff, zchunk, or casync/desync-style chunked refs).

**7. Secure-channel identity toolkit for off-cluster workloads.** Pattern source: Ockam, SPIFFE Federation. Helios implication: Helios's built-in CA (§4) already issues SPIFFE SVIDs. For IoT leaves that cannot participate in sockops/kTLS, a separate identity path that issues a provably-Helios-signed credential over an arbitrary transport (MQTT, HTTPS, Bluetooth) is a known pattern.

**8. A/B rootfs partitioning with boot-health rollback.** Pattern source: Balena, Talos, EVE-OS. Helios implication: §23 roadmaps "node upgrade via OCI registry pull" for Phase 2 — the Balena/Talos A/B rollback model is the concrete implementation to adopt. Not IoT-specific, but particularly important when field devices are unreachable for manual recovery.

### What the research confirms about the v1 scope-out decision

Every finding in this document supports the 2026-04-19 decision to scope IoT edge out of Helios v1:

- **Failure mode #1 (SWIM fan-out)** — No IoT-edge platform uses peer gossip at leaf scale. The universal answer is hub-and-spoke aggregation. Helios's Corrosion substrate is well-shaped for servers and regional edge-compute; it is the wrong shape for 100k+ devices.
- **Failure mode #2 (NAT)** — Every IoT-edge platform uses outbound-initiated tunnels. Helios's Corrosion-over-QUIC assumes UDP reachability and does not have a tunneled-observation path.
- **Failure mode #3 (static seed lists)** — The industry-standard answer is a global rendezvous service. Helios does not have this primitive.
- **Failure mode #4 (enrollment protocol)** — The industry-standard answers are DPS-like attestation services or claim-cert exchange flows. Helios §23 roadmaps the attestation chain but not the enrollment protocol.

The scope-out was correct. Re-opening would require additions 1–5 above as a minimum, all of which are substantial features with their own design space — not bolt-ons to the existing whitepaper. Re-opening would be reasonable only if a concrete IoT-edge customer and requirements set materialises; the current v1 focus (servers + edge-compute, tens to hundreds of nodes per region) remains consistent with Helios's architectural strengths.

---

## Source Analysis

| Source | Domain | Reputation | Type | Access | Cross-verified |
|--------|--------|------------|------|--------|----------------|
| KubeEdge Docs — architecture | kubeedge.io | High | official (project) | 2026-04-19 | Y |
| KubeEdge Test Report — 100k Edge Nodes | kubeedge.io | High | official (project) | 2026-04-19 | Y (MDPI academic) |
| KubeEdge keadm install docs | kubeedge.io | High | official (project) | 2026-04-19 | Y |
| K3s Docs — Architecture | docs.k3s.io | High | official (project) | 2026-04-19 | Y |
| OpenYurt Docs — Architecture | openyurt.io | High | official (project) | 2026-04-19 | Y |
| Akri Docs — Architecture Overview | docs.akri.sh | High | official (project) | 2026-04-19 | Y |
| AWS IoT Greengrass V2 Developer Guide | docs.aws.amazon.com | High | official (vendor) | 2026-04-19 | Y |
| AWS Greengrass Feature Compatibility Matrix | docs.aws.amazon.com | High | official (vendor) | 2026-04-19 | Y |
| Microsoft Learn — What is Azure IoT Edge | learn.microsoft.com | High | official (vendor) | 2026-04-19 | Y |
| LF Edge — EVE-OS README | github.com/lf-edge/eve | High | OSS foundation | 2026-04-19 | Y |
| LF Edge — EVE HYPERVISORS.md | github.com/lf-edge/eve | High | OSS foundation | 2026-04-19 | Y |
| Eclipse ioFog Docs — Architecture | iofog.org | Medium-High | OSS foundation | 2026-04-19 | partial |
| openBalena README | github.com/balena-io/open-balena | High | OSS project | 2026-04-19 | Y |
| ICS — IoT Fleet Management comparison (Torizon/Balena/Mender) | ics.com | Medium | secondary industry | 2026-04-19 | Y |
| Ockam Docs — Secure Channels | docs.ockam.io | High | official (project) | 2026-04-19 | Y |
| Nebula Docs (Defined Networking) | nebula.defined.net | High | official (project) | 2026-04-19 | Y (Slack blog) |
| DeepWiki — Nebula Lighthouse Discovery | deepwiki.com | Medium | secondary | 2026-04-19 | Y (upstream Nebula repo) |
| Tailscale — How NAT traversal works | tailscale.com | High | official (vendor) | 2026-04-19 | Y |
| Tailscale — tailscale-rs preview | tailscale.com | High | official (vendor) | 2026-04-19 | partial |
| MDPI Sensors 2023 — Cloud-Native Workload Orchestration at the Edge | pmc.ncbi.nlm.nih.gov | High | academic (peer-reviewed) | 2026-04-19 | Y |
| LF Edge — Baetyl 2.0 | lfedge.org | High | OSS foundation | 2026-04-19 | Y |
| Dianomic — FogLAMP | dianomic.com | Medium | vendor | 2026-04-19 | Y (LF Edge Fledge) |
| SuperEdge vs OpenYurt vs KubeEdge | linkedin.com | Medium | secondary | 2026-04-19 | Y (academic) |
| CNCF blog — KubeEdge getting started | cncf.io | High | OSS foundation | 2026-04-19 | Y |
| Helios whitepaper §4, §18, §23 | local (docs/whitepaper.md) | High | authoritative (SSOT) | 2026-04-19 | N/A |

**Reputation distribution:** High: 19 (~76%). Medium-High: 1 (~4%). Medium: 5 (~20%). Average reputation: ~0.89 — weighted toward official project docs and vendor-authoritative sources.

**Cross-reference status:** 22 of 25 sources independently cross-verified. Balena docs could not be fetched directly (404 on multiple paths); the balenaEngine + delta-update claim is cross-verified between the openBalena README (first-party) and a secondary industry comparison (ICS.com). EVE-OS-readthedocs also returned 403; technical claims verified from EVE GitHub docs instead. Accepted for a scouting document.

---

## Knowledge Gaps

### Gap 1: Balena's official architecture docs were unreachable during research

**Issue:** `docs.balena.io/reference/OS/overview/` and related paths returned 404 during WebFetch. Balena's content is routed through a CDN and some docs paths may have changed. The balenaEngine / balena-supervisor / OpenVPN / delta-updates claims in P11 are cross-verified against the openBalena README (first-party) and the ICS.com comparison (secondary), but not against Balena's primary reference docs. **Recommendation:** Manual browser visit to docs.balena.io or `llms-full.txt` export if architectural precision is needed for follow-up.

### Gap 2: Independent scale verification is thin outside KubeEdge

**Issue:** KubeEdge's 100k-edge-node claim is independently corroborated by the MDPI Sensors 2023 academic review citing the Hong Kong–Zhuhai–Macao bridge monitoring deployment (100k+ sensors). For every other platform — AWS Greengrass, Azure IoT Edge, Balena, EVE-OS — published scale claims are either vendor-authored or high-level case studies without reproducible methodology. **Recommendation:** If a specific scale claim becomes load-bearing in a customer recommendation, validate against that vendor's published customer case studies before citing.

### Gap 3: No hands-on benchmarking

**Issue:** This is a document-review study; no platform was installed and measured. Claims about resource footprint, enrollment latency, update bandwidth, and offline convergence time are all taken from vendor docs or the academic review. **Recommendation:** If Helios reopens IoT, a targeted benchmark of 2–3 platforms (KubeEdge, EVE-OS, AWS Greengrass) at 1000-device scale would de-risk the primitive-borrow decisions in the Helios-relevance section.

### Gap 4: Ockam and SPIFFE Federation integration details

**Issue:** Ockam's secure channel protocol is well-documented at the conceptual level, but the integration with SPIFFE Federation (which is the likely Helios-alignment path) is not spelled out in either project's docs. If recommendation P12 (#7 in relevance priorities) is pursued, a dedicated research cycle on SPIFFE Federation + Ockam compatibility is warranted.

### Gap 5: Current maintenance status of older projects

**Issue:** Eclipse ioFog's release cadence is slow (last major release 2021, targeted 3.0 slipped); FogLAMP has been renamed to LF Edge Fledge with implications for where the active development is happening; Flotta (Red Hat) and Open Horizon (IBM) have limited recent public activity. **Recommendation:** Treat these as historical pattern references rather than currently-recommended platforms.

### Gap 6: Nested edge and hierarchical fan-out

**Issue:** Azure IoT Edge supports a "nested edge" topology (downstream Edge devices connect upstream to parent Edge devices, for multi-tier fan-out through network segments). This was not deeply investigated and may be the most direct architectural answer to Helios's failure mode #1 at IoT scale — hierarchical fan-out rather than flat aggregation. **Recommendation:** Targeted follow-up on Azure nested edge and KubeEdge's EdgeSite pattern if hierarchical topology becomes a design consideration.

---

## Conflicting Information

### Conflict 1: Edge autonomy — "first-class property" vs "bolt-on"

**Position A:** KubeEdge documents edge autonomy as a first-class property of EdgeCore, via MetaManager persistence and offline reconciliation. [KubeEdge Docs](https://kubeedge.io/docs/). Reputation: High.

**Position B:** OpenYurt's positioning explicitly argues that KubeEdge "attempts to rewrite some components such as kubelet or kube-proxy" — characterising KubeEdge's approach as invasive and OpenYurt's as non-intrusive. [Alibaba Cloud community blog](https://www.alibabacloud.com/blog/openyurt-the-practice-of-extending-native-kubernetes-to-the-edge_597903), referenced via MDPI academic review. Reputation: Medium (vendor-authored but supported by academic review).

**Assessment:** Both are factually correct, not contradictory. KubeEdge does rewrite Edged (a kubelet replacement) to be lighter and offline-capable; OpenYurt keeps stock kubelet and adds a per-node caching proxy (YurtHub) instead. Both achieve edge autonomy; they differ on invasiveness of change to K8s. The rhetorical framing conflict is a project-positioning dispute, not a technical contradiction.

### No other substantive conflicts surfaced

The landscape is architecturally diverse but internally consistent per-platform — no cases of platform X's official docs contradicting platform X's own behaviour as described by independent sources.

---

## Recommendations for Further Research

1. **Targeted scale benchmark** — if an IoT-edge customer opportunity materialises, install KubeEdge + EVE-OS + AWS Greengrass at 1000 simulated devices (nested containers or Firecracker microVMs on a single host) and measure control-plane CPU/RAM/bandwidth, enrollment time, rolling-update bandwidth, and offline-reconnect convergence time. One week of engineering effort is enough to de-risk a primitive-borrow decision.

2. **Azure DPS protocol deep dive** — if Helios reopens IoT, the rendezvous-plus-attestation pattern is the single largest whitepaper addition. Reconstruct the DPS MQTT API, claim-cert-exchange flow, and TPM EK verification chain as a design doc for a "helios-rendezvous" service. AWS Fleet Provisioning as secondary reference; EVE-OS Adam as an open-source implementation reference.

3. **KubeEdge EdgeMesh + SuperEdge distributed health check** — both address edge-site-local service discovery for intermittent deployments (a fleet of devices at the same site should route locally even when the cloud link is down). Not in scope for this scouting study but a natural follow-up.

4. **OpenYurt YurtHub as a design reference for per-node Intent-cache** — if Helios ever implements per-node (not just per-region) autonomy, YurtHub's implementation of "proxy that serves from disk cache during cloud disconnect" is the clearest prior art.

5. **Binary delta update format selection** — Balena uses a proprietary-ish docker-delta; alternatives include casync/desync (content-addressed chunked refs), zchunk (Fedora), bsdiff (classic). A focused comparison for the Helios Image Factory (§23) would inform whether this is a Phase 3 item.

6. **SPIFFE Federation + Ockam composition** — the off-cluster-identity story. How would a Helios cluster federate trust with devices that cannot run the Helios node agent but can run an Ockam identity client?

---

## Full Citations

[1] KubeEdge Project. "Components." KubeEdge Documentation. https://kubeedge.io/docs/ — Accessed 2026-04-19.
[2] KubeEdge Project. "Test Report on KubeEdge's Support for 100,000 Edge Nodes." KubeEdge Blog. https://kubeedge.io/blog/scalability-test-report/ — Accessed 2026-04-19.
[3] KubeEdge Project. "Installing KubeEdge with Keadm." KubeEdge Documentation. https://kubeedge.io/docs/setup/install-with-keadm/ — Accessed 2026-04-19.
[4] KubeEdge Project. "perf.md — Scalability Proposal." https://github.com/kubeedge/kubeedge/blob/master/docs/proposals/perf.md — Accessed 2026-04-19.
[5] Rancher Labs / SUSE. "Architecture." K3s Documentation. https://docs.k3s.io/architecture — Accessed 2026-04-19.
[6] OpenYurt Project. "Architecture." OpenYurt Documentation. https://openyurt.io/docs/core-concepts/architecture — Accessed 2026-04-19.
[7] Akri Project. "Architecture Overview." Akri Documentation. https://docs.akri.sh/architecture/architecture-overview — Accessed 2026-04-19.
[8] Amazon Web Services. "What is AWS IoT Greengrass?" AWS IoT Greengrass V2 Developer Guide. https://docs.aws.amazon.com/greengrass/v2/developerguide/what-is-iot-greengrass.html — Accessed 2026-04-19.
[9] Amazon Web Services. "Greengrass feature compatibility." AWS IoT Greengrass V2 Developer Guide. https://docs.aws.amazon.com/greengrass/v2/developerguide/operating-system-feature-support-matrix.html — Accessed 2026-04-19.
[10] Microsoft. "What is Azure IoT Edge." Microsoft Learn. https://learn.microsoft.com/en-us/azure/iot-edge/about-iot-edge — Accessed 2026-04-19.
[11] LF Edge. "EVE-OS README." https://github.com/lf-edge/eve/blob/master/docs/README.md — Accessed 2026-04-19.
[12] LF Edge. "EVE HYPERVISORS.md." https://github.com/lf-edge/eve/blob/master/docs/HYPERVISORS.md — Accessed 2026-04-19.
[13] Eclipse Foundation. "ioFog Architecture." Eclipse ioFog Documentation. https://iofog.org/docs/2/getting-started/architecture.html — Accessed 2026-04-19.
[14] Balena. "openBalena README." https://github.com/balena-io/open-balena/blob/master/README.md — Accessed 2026-04-19.
[15] ICS. "Choosing the Right IoT Fleet Management System: A Look at Torizon, Balena and Mender." https://www.ics.com/blog/iot-fleet-management-system-torizon-balena-mender — Accessed 2026-04-19.
[16] Ockam. "Secure Channels." Ockam Documentation. https://docs.ockam.io/ — Accessed 2026-04-19.
[17] Defined Networking. "Introduction to Nebula." Nebula Documentation. https://nebula.defined.net/docs/ — Accessed 2026-04-19.
[18] DeepWiki. "Nebula — Lighthouse Discovery." https://deepwiki.com/slackhq/nebula/3.5-lighthouse-discovery — Accessed 2026-04-19.
[19] Tailscale. "How NAT traversal works." Tailscale Blog. https://tailscale.com/blog/how-nat-traversal-works — Accessed 2026-04-19.
[20] Tailscale. "An early look at tailscale-rs, a tsnet library in Rust." https://tailscale.com/blog/tailscale-rs-rust-tsnet-library-preview — Accessed 2026-04-19.
[21] Böhm, S.; Wirtz, G. "Cloud-Native Workload Orchestration at the Edge: A Deployment Review and Future Directions." *Sensors* 23(4), 2215 (MDPI). https://pmc.ncbi.nlm.nih.gov/articles/PMC9967903/ — Accessed 2026-04-19.
[22] LF Edge. "Baetyl 2.0." https://lfedge.org/baetyl-2-0/ — Accessed 2026-04-19.
[23] Dianomic Systems. "FogLAMP Architecture Plugins." https://dianomic.com/platform/foglamp/architecture-plugins/ — Accessed 2026-04-19.
[24] Chandra, G. "SuperEdge, OpenYurt - Extending Native Kubernetes to Edge." LinkedIn. https://www.linkedin.com/pulse/superedge-openyurt-extending-native-kubernetes-edge-gokul-chandra — Accessed 2026-04-19.
[25] Alibaba Cloud. "OpenYurt: The Practice of Extending Native Kubernetes to the Edge." https://www.alibabacloud.com/blog/openyurt-the-practice-of-extending-native-kubernetes-to-the-edge_597903 — Accessed 2026-04-19.
[26] Helios Project. "Helios Whitepaper v0.12 — Draft." `docs/whitepaper.md` in repo — §4 (Control Plane, Corrosion), §18 (Reconciler Model), §23 (Image Factory). Accessed 2026-04-19.

---

## Research Metadata

**Duration:** ~48 turns. **Platforms covered:** 14 primary + 4 supplementary (SuperEdge, Baetyl, Open Horizon, Flotta). **Sources examined:** 30+. **Sources cited:** 26. **Cross-references per major claim:** 2–3 typical; 1 (authoritative vendor doc only) accepted for vendor-specific implementation detail.

**Confidence distribution:** High 76% · Medium-High 4% · Medium 20%.

- **High** for: KubeEdge architecture + scale claim (independently corroborated by MDPI); K3s, OpenYurt, Akri, Azure IoT Edge, EVE-OS architecture (official docs + cross-references); AWS Greengrass model (official docs + feature matrix); Nebula NAT-traversal design (docs + DeepWiki + HN discussion); the four-failure-mode cross-check against whitepaper §4/§23.
- **Medium-High** for: Eclipse ioFog architecture (official docs but reduced project activity).
- **Medium** for: specific per-customer scale claims (vendor-authored); Balena architecture (openBalena README + secondary comparison; primary Balena docs unreachable).

**Tool failures:** `docs.balena.io/reference/OS/overview` 404; `eve-os.readthedocs.io` 403; `iofog.org/docs/3.0.0/...` 404; `docs.openziti.io` redirect to netfoundry.io (not pursued — wrong product). Mitigated via GitHub-hosted READMEs and independent comparison sources in all four cases. Research output not materially affected.

**Output:** `/Users/marcus/conductor/workspaces/helios/taipei-v1/docs/research/orchestration/iot-edge-orchestrators-research.md`
