# Research: Flat-Cluster Membership, Discovery, Admission and Voter-Set Management over a WireGuard Mesh

**Date**: 2026-09-25 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium-High overall (High on mechanisms; Medium on scale claims and on the maturity of VR reconfiguration) | **Sources**: 72 cited (avg reputation ≈ 0.88)

> Scope: problem cluster B (who is in the cluster, how nodes find each other, who votes) for a TARGET design. This is not an evaluation of the current codebase. Sibling documents cover consensus/log dissemination (`docs/research/architecture/flat-cluster-consensus-and-log-dissemination-comprehensive-research.md`) and a third cluster. Prior findings on Serf/memberlist/Foca/Lifeguard (`docs/research/gossip-protocols/serf-internals-and-overdrive-fit.md`) and Corrosion scale (`docs/research/scalability/corrosion-crsqlite-production-scale-evidence.md`) are cited and extended, not repeated.

## Executive Summary

The proposed flow ("configure one WireGuard endpoint → announce → a peer returns the full list → gossip → a leader emerges → a small, automatically chosen voter set commits intent, and everyone else is a learner") has close production precedent. **Elasticsearch 7+** is the single nearest match: seed hosts, mutual exchange of known *master-eligible* peers, election among eligible nodes, an automatically maintained odd-sized voting configuration, all roles on by default, and a TLA+-specified safety core from the Paxos/Raft/VR family. **Consul Autopilot** supplies the voter-management rules: learner-first promotion after a stabilization window, dead-server cleanup gated by a quorum floor, and redundancy zones. **Service Fabric** states the design's central principle explicitly: "decouple failure *detection* from failure *decision*", with an arbitration log. On this evidence, the working hypothesis "gossip carries facts, the log carries decisions" is directionally right.

The research also finds four places where the current direction is under-specified.

1. **WireGuard cannot be the first contact.** It silently drops handshakes from unknown keys. Every surveyed system (kubeadm, k3s, Talos, Consul, SPIRE) bootstraps over a separate authenticated channel, where the joiner *pins the cluster's CA hash* and presents a *short-lived, ideally single-use* credential. Talos CVE-2022-36103 shows the credential must not be able to mint more privilege than the admission record grants.
2. **Unauthenticated peer exchange is the eclipse/Sybil surface** (Bitcoin, libp2p). Exchanged records should be self-certifying and admission-backed, ENR- or Nebula-certificate style, so a list from *any* peer can be trusted.
3. **"Leader decides NodeDown from gossip" acts on second-hand, single-accuser evidence.** Gray failure (HotOS'17), Panorama (OSDI'18) and Rapid (ATC'18) show that decisions need multi-observer aggregation with hysteresis and should batch correlated failures. Kubernetes and Nomad add post-election grace periods and zone-aware rate limits.
4. **Authority-bearing facts** (keys, admission, declared roles, failure-domain labels) belong on the log even though they are "facts". The refined rule: *only the log creates authority; gossip transports signed, owner-written observations and may relay committed entries.*

The top-ranked unexplored alternatives are:
- **Rapid-style multi-observer cut detection** feeding the log (Rapid's logically centralized mode *is* a voters/learners split for membership).
- **Self-certifying CA-backed node records** as the unit of peer exchange.
- **Service Fabric-style lease monitoring with arbitration and self-fencing.**

Two material risks sit partly outside this scope:
- No production system was found running *thousands of full-log learners* off one leader: etcd deliberately caps learners, and Census-style tree dissemination is the plausible answer.
- TigerBeetle, the flagship VSR implementation, documents online **reconfiguration as unimplemented**, so voter-set changes under VR rest on the VR Revisited paper and the `viewstamp` crate rather than production experience.

## Research Methodology

**Search strategy**: primary sources first — papers (USENIX, ACM, arXiv, NDSS), specifications (EIPs, libp2p/devp2p specs, CometBFT spec), official product documentation, and source code or advisories on GitHub. Secondary sources (the morning paper, Jepsen) were used only where a primary source was unreachable or where the secondary source is itself the primary evidence (Jepsen's test results). PDFs that WebFetch could not parse were read page-by-page from the saved binary (Rapid, WireGuard, Gray Failure, Raft, the Service Fabric slides).
**Source selection**: types were academic, official, technical documentation and industry. The reputation floor was medium-high, with one medium-tier secondary summary, flagged. Where a project's authoritative source lies outside the trusted list (elastic.co, tailscale.com, nebula.defined.net, eips.ethereum.org, cockroachlabs.com, docs.restate.dev, aphyr.com), it is marked *out-of-list primary* and scored 0.8.
**Quality standards**: target 3 sources per major claim, with a minimum of 1 authoritative source; confidence is downgraded where only one source was available. Vendor self-reports (Tailscale throughput, Slack's Nebula host count, Rapid's own benchmarks) are flagged as such. SEO or vendor blogs on WireGuard scaling were found and **deliberately excluded**.
**Prior repo research reused (not repeated)**: [serf-internals-and-overdrive-fit.md](../gossip-protocols/serf-internals-and-overdrive-fit.md) (SWIM, Lifeguard, Foca, chitchat, Corrosion) and, by reference, [corrosion-crsqlite-production-scale-evidence.md](../scalability/corrosion-crsqlite-production-scale-evidence.md) (observation plane). The consensus protocol, log dissemination and the `viewstamp` crate are left to the sibling document [flat-cluster-consensus-and-log-dissemination-comprehensive-research.md](flat-cluster-consensus-and-log-dissemination-comprehensive-research.md).

## Target design under examination (restated)

- **Topology**: flat, with no separate single-node or HA modes. It runs from 1 node to thousands, possibly across regions, and grows by joining a node.
- **Per-node components** (gateway, web, control-plane, worker, telemetry, wasm, storage) are all enabled by default. Each component's port is satisfied by a local or a remote adapter.
- **Discovery**:
  - A new machine is given one existing machine's WireGuard endpoint.
  - It announces itself.
  - A peer answers with the list of all machines.
  - Gossip then handles discovery.
  - A leader is selected.
- **Intent** (specs, placements, membership/admission, declared roles, the voter set, NodeDown decisions) is ordered by a consensus log. The leading candidate is VR (VR Revisited, TigerBeetle VSR, the `viewstamp` crate with SingleChange reconfiguration and learners).
- **Voters**: 3–5, auto-selected from `control-plane` nodes across failure domains, with hysteresis. All other nodes are full-log learners. Roles are committed at join. NodeDown is a leader decision based on gossip failure detection.
- **Observation** (heartbeats, allocation status, health) is high-volume, per-node-owned, available under partition, and kept off the log.
- **Trust**: a single operator, trusted nodes, CFT. Admission must stop a machine that merely reaches a WireGuard endpoint.
- **Hypothesis under test**: "gossip carries facts, the log carries decisions."

## B1. Peer discovery via a seed peer and peer exchange

The user's proposal (seed endpoint → announce → a peer answers with the full list → gossip keeps it fresh) has close production analogues. The key question for a permissioned cluster is not *whether* peer exchange works (it does, everywhere) but **what makes an exchanged record trustworthy** and **whether the membership list it produces is the authoritative one**.

### Finding B1.1: Elasticsearch's discovery is the closest production match to the proposal: seed → mutual exchange of known peers → election among eligible nodes

**Evidence**: "This process starts with a list of *seed* addresses from one or more seed hosts providers … it shares with the remote node a list of all of its known master-eligible peers and the remote node responds with *its* peers in turn. The node then probes all the new nodes that it just discovered, requests their peers, and so on." And: "If the node is master-eligible then it continues this discovery process until it has either discovered an elected master node or else it has discovered enough masterless master-eligible nodes to complete an election."
**Source**: [Elastic — Discovery: seed hosts providers](https://www.elastic.co/guide/en/elasticsearch/reference/current/modules-discovery-hosts-providers.html) — Accessed 2026-09-25 (out-of-list primary, medium-high)
**Confidence**: High (for Elasticsearch behaviour)
**Verification**: [Elastic — A new era for cluster coordination in Elasticsearch](https://www.elastic.co/blog/a-new-era-for-cluster-coordination-in-elasticsearch) (same vendor, independent document: confirms 7.0 redesign, VR/Raft-family safety core, TLA+ specs); Rapid's `JOIN(HOST:PORT, SEEDS, …)` API (Suresh et al., USENIX ATC'18, §3) shows the same seed-list bootstrap shape in an academic design.
**Analysis**: Two properties of Elasticsearch's version matter. (1) Peer exchange only exchanges **master-eligible** (voter-eligible) peers, not every node; non-eligible nodes simply find *a* master and then receive the full, authoritative cluster state from it. (2) Discovery is only a means of finding enough eligible nodes to form a quorum; the membership list that matters is the **cluster state committed by the elected master**, not the gossip view. This is the same split the target design proposes, with one difference: Elasticsearch has no gossip layer at all after discovery. The master publishes cluster state, and followers are checked by master-driven "follower checks" (see B4).

### Finding B1.2: Rapid treats "join via seed" as a *membership change agreed by consensus*, not a gossip fact, and its logically centralized mode maps directly onto a "voters + learners" split

**Evidence**: "Processes use the membership service by using the Rapid library and invoking a call JOIN(HOST:PORT, SEEDS, VIEW-CHANGE-CALLBACK) … SEEDS is an initial set of process addresses known to everyone and used to contact for bootstrapping." "New processes join by contacting a list of K temporary observers obtained from a seed process … The temporary observers generate independent alerts about joiners." The logically centralized design: "a set of auxiliary nodes S records the membership changes for a cluster C … Nodes in the current configuration C continue monitoring each other according to the k-ring topology … Instead of gossiping these alerts to all nodes in C, they report it only to all nodes in S … Nodes in S apply the CD protocol … they execute the VC protocol only among themselves … Nodes in C learn about changes in the membership through notifications from S (or by probing nodes in S periodically)." Resiliency "is now bound to that of S (F = S/2 − 1)".
**Source**: [Suresh, Malkhi, Gopalan, Porto Carreiro, Lokhandwala — "Stable and Consistent Membership at Scale with Rapid", USENIX ATC 2018, §3, §4.1, §5](https://www.usenix.org/system/files/conference/atc18/atc18-suresh.pdf) — Accessed 2026-09-25 (academic, high)
**Confidence**: High (for the design); Low (for production maturity; see B1.3)
**Verification**: [USENIX ATC'18 presentation page](https://www.usenix.org/conference/atc18/presentation/suresh) (abstract confirms "a leaderless consensus protocol that converts multi-process cut detections into a view-change decision … works both in fully decentralized as well as logically centralized modes"); reference implementation [lalithsuresh/rapid](https://github.com/lalithsuresh/rapid).
**Analysis**: Rapid-C (the paper's name for the centralized variant, evaluated with a 3-node ensemble managing N processes) *is* the target design's voter/learner split applied to membership: every node monitors K others, alerts go to the small voter group, the voter group decides the cut and commits it, and everyone else learns the result. The difference from the proposal is that Rapid **aggregates multi-observer alerts into a single multi-node cut** before committing. The proposal as written has "the leader decides from gossip". Rapid shows the decision input can be made high-fidelity (B4) without changing who commits it.

### Finding B1.3: Rapid has strong academic results but thin production evidence

**Evidence**: Bootstrapping 2000-node clusters, "Rapid improves bootstrap latencies by 2-2.32x over Memberlist, and by 3.23-5.8x over ZooKeeper"; "Memberlist's convergence times are thereby as high as 95s on average when N = 2000" (non-seed processes rely on periodic push-pull every ~30 s). The paper also reports Akka Cluster "did not stabilize for clusters beyond 500 processes". The reference implementation is Java, ~2.4k LoC, "open-sourced under an Apache 2 license"; the GitHub repository shows 140 stars and a 0.8.0 Maven artifact, and presents itself as research software.
**Source**: [Rapid paper §6–§7](https://www.usenix.org/system/files/conference/atc18/atc18-suresh.pdf); [lalithsuresh/rapid](https://github.com/lalithsuresh/rapid) — Accessed 2026-09-25
**Confidence**: Medium: the performance numbers are from the authors' own evaluation (single source). The adoption evidence is an absence finding: a search for Rust ports and production adoption returned only SWIM crates.
**Verification**: The search `Rapid membership protocol … Rust OR production adoption` surfaced no Rust implementation and no production case study beyond the paper's two integrations (a transactional data platform and service discovery).
**Analysis**: Rapid is an **idea to borrow** (multi-observer, multi-node cut detection with H/L watermarks) rather than a library to adopt. For a Rust orchestrator it would be a from-scratch implementation.

### Finding B1.4: Self-certifying signed node records (Ethereum ENR, libp2p signed peer records) are the established answer to "can I trust an address I learned from a third party?"

**Evidence**: ENR (EIP-778): node records consist of a "cryptographic signature of record contents," a 64-bit sequence number, and sorted key/value pairs; nodes "increase the sequence number whenever the record changes and republish the record"; "The maximum encoded size of a node record is 300 bytes." In discv5 a receiver "must first validate it by checking the record's signature" during the WHOAREYOU handshake. libp2p signed peer records exist because "'ambient' discovery methods like DHT traversal depend on potentially untrustworthy third parties to relay address information", and `seq` is "a monotonically-increasing sequence counter to order PeerRecords in time".
**Source**: [EIP-778 Ethereum Node Records](https://eips.ethereum.org/EIPS/eip-778) (out-of-list primary); [ethereum/devp2p discv5-theory](https://github.com/ethereum/devp2p/blob/master/discv5/discv5-theory.md) (github.com, medium-high); [libp2p RFC 0003 routing records](https://github.com/libp2p/specs/blob/master/RFC/0003-routing-records.md) (github.com, medium-high) — Accessed 2026-09-25
**Confidence**: High
**Verification**: Three independent specifications (Ethereum EIP, Ethereum devp2p, libp2p) converge on the same shape: a signed record, a monotone sequence number, and a small size.
**Analysis**: In the target design the WireGuard public key already *is* a node identity. If each node publishes a small record (WG pubkey, endpoints, declared roles, failure-domain labels, seq) that is **signed by the node key AND carries an admission credential signed by the cluster CA** (B3), then peer exchange ("here is the list of everyone") becomes safe to accept from any peer, not only from the seed. Without the signature, peer exchange lets any member (or anyone who compromised one) poison the list. With it, the worst a malicious relayer can do is withhold or replay stale records, and `seq` resolves staleness.

### Finding B1.5: Open peer exchange without authenticated records is the documented eclipse-attack surface

**Evidence**: Heilman et al. show "an adversary controlling a sufficient number of IP addresses to monopolize all connections to and from a victim bitcoin node" can eclipse it, enabling "N-confirmation double spending, selfish mining, and adversarial forks". libp2p rendezvous acknowledges vulnerability to Sybil attacks (adversaries "generating numerous identities to spam namespaces") and lists mitigation as "TBD".
**Source**: [Heilman, Kendler, Zohar, Goldberg — "Eclipse Attacks on Bitcoin's Peer-to-Peer Network", USENIX Security 2015](https://www.usenix.org/conference/usenixsecurity15/technical-sessions/presentation/heilman) (academic, high); [libp2p rendezvous spec](https://github.com/libp2p/specs/blob/master/rendezvous/README.md) — Accessed 2026-09-25
**Confidence**: High
**Verification**: The discv5 theory document likewise only addresses DoS mitigation and "doesn't quantify security assumptions" for Sybil/eclipse resistance. Open-network discovery protocols treat Sybil resistance as out of scope or unsolved.
**Analysis**: Bitcoin, discv5 and libp2p are designed for **permissionless** networks, where Sybil resistance must come from resource cost or heuristics (bucket diversity, IP-prefix limits). A permissioned cluster should not import those heuristics; it should make Sybils impossible by requiring a CA-signed admission credential in every record (B3). The *mechanics* (addr/getaddr style exchange, DHT lookups) carry over; the *trust model* does not. For a cluster of at most thousands of nodes, a full list is also small enough (thousands × ~300 B ≈ 1 MB) that a DHT's O(log N) routing buys nothing. A full membership table replicated everywhere is simpler.

### Finding B1.6: Rust-native peer-sampling/broadcast options exist (HyParView + PlumTree in iroh-gossip; Scuttlebutt + phi-accrual in chitchat) but neither provides consistent membership

**Evidence**: iroh-gossip implements epidemic broadcast trees based on "HyParView" and "PlumTree"; peers join a topic swarm through "bootstrap peers"; dual-licensed Apache-2.0/MIT. Chitchat uses "scuttlebutt reconciliation … a anti-entropy gossip algorithm", "phi-accrual detection to dynamically compute a threshold", and states "liveness is a local concept. Every single node computes its own vision of the liveness of all other nodes"; "a node can only edit its own node state."
**Source**: [n0-computer/iroh-gossip](https://github.com/n0-computer/iroh-gossip); [quickwit-oss/chitchat](https://github.com/quickwit-oss/chitchat) — Accessed 2026-09-25 (github.com, medium-high)
**Confidence**: Medium-High
**Verification**: Prior repo research on Foca/chitchat ([serf-internals-and-overdrive-fit.md](../gossip-protocols/serf-internals-and-overdrive-fit.md), Findings 3.1, 4.1) confirms the Rust gossip set is Foca (SWIM+Inf+Susp, used by Corrosion), chitchat and a few small SWIM crates. Rapid's related-work section classifies all gossip-based membership (Cassandra, Akka, ScyllaDB, Serf, Redis Cluster, Orleans, Ringpop, Dynomite) as having "weak consistency guarantees".
**Analysis**: HyParView (partial views of log N active peers + a larger passive view) is the one gossip design here that **does not require every node to track every other node**, which matters only if the cluster goes well beyond thousands of nodes. PlumTree is relevant to the *log dissemination* problem (sibling document A2) more than to membership. Chitchat's "each node owns its own key space, liveness is local" model matches the target design's **observation** plane exactly: per-node-owned, available under partition, not authoritative.



## B2. WireGuard mesh formation and key/endpoint distribution at scale

Every production WireGuard mesh surveyed separates a **low-bandwidth control channel that distributes keys and endpoints** from the **mesh data plane**. They differ in who runs that control channel (a hosted server, lighthouses, gossip, or an encrypted blob store) and in what grants the right to be in the mesh.

### Finding B2.1: WireGuard deliberately leaves key distribution to a higher layer; a peer is only a public key plus allowed IPs, and endpoints roam

**Evidence**: "WireGuard's attitude toward key distribution is that this is the wrong layer to address that particular problem, and so the interface is simple enough that any key distribution solution can be used with it." "In WireGuard, peers are identified strictly by their public key, a 32-byte Curve25519 point. This means that there is a simple association mapping between public keys and a set of allowed IP addresses." Each peer "may *optionally* pre-specify a known external IP address and UDP port of that peer's endpoint" (§2.1, Endpoints & Roaming).
**Source**: [Donenfeld — "WireGuard: Next Generation Kernel Network Tunnel", NDSS 2017 (whitepaper), §1–§2](https://www.wireguard.com/papers/wireguard.pdf) — Accessed 2026-09-25 (out-of-list primary; peer-reviewed at NDSS, high)
**Confidence**: High
**Verification**: [Tailscale — How Tailscale works](https://tailscale.com/blog/how-tailscale-works) (the coordination server is "a shared drop box for public keys"); [Talos KubeSpan docs](https://docs.siderolabs.com/talos/v1.11/networking/kubespan) (public keys "published through the cluster discovery"); [wesher README](https://github.com/costela/wesher) (public keys "broadcast across the cluster").
**Analysis**: For the target design this means "configure a new machine with one existing machine's WireGuard endpoint" is **not enough on its own**: the existing machine must already have the newcomer's public key in its peer table, or the handshake is silently dropped (WireGuard answers nothing to unknown keys). The first contact therefore needs either (a) a pre-authorised key (the operator adds the pubkey out of band), (b) a separate bootstrap channel that is not WireGuard (for example TLS on a join port, as kubeadm/k3s/Talos do), or (c) a shared "join" peer entry. This is the concrete form of the B3 chicken-and-egg problem.

### Finding B2.2: The Linux kernel implementation has no practical peer-count ceiling at "thousands", but operational costs scale with the table

**Evidence**: `MAX_PEERS_PER_DEVICE = 1U << 20` (1,048,576). Timers: `REKEY_TIMEOUT = 5`, `REKEY_AFTER_TIME = 120`, `REJECT_AFTER_TIME = 180`, `KEEPALIVE_TIMEOUT = 10`; `MAX_QUEUED_INCOMING_HANDSHAKES = 4096`.
**Source**: [Linux kernel `drivers/net/wireguard/messages.h`](https://raw.githubusercontent.com/torvalds/linux/master/drivers/net/wireguard/messages.h) (github.com mirror of kernel.org source, high) — Accessed 2026-09-25
**Confidence**: High for the hard limit. Low for operational scaling claims (see Knowledge Gaps).
**Verification**: Tailscale's engine exposes `SetPeerConfigFunc`/`SyncDevicePeer`, where "lazily-created peers always see current state" and per-peer sync "performs O(1) work … avoiding a full [Engine.Reconfig]" ([pkg.go.dev tailscale.com/wgengine](https://pkg.go.dev/tailscale.com/wgengine), out-of-list primary). A full-table reconfiguration per membership change is the cost production systems work around. The only sources found for "1000 peers takes seconds to dump" and "handshake trickle from stale peers" were vendor/SEO blogs, which were **not** used as evidence.
**Analysis**: At 1k–10k nodes the binding constraints are (1) **full mesh = O(N) peers per node and O(N²) edges** in the cluster, (2) reconfiguring the device on every membership delta, and (3) handshakes to peers that are dead or unreachable. Tailscale's answer is **on-demand (lazy) peering**: keep the full peer *list* in memory, and install a peer in the kernel/device only when traffic needs it. For an orchestrator whose control-plane traffic mostly goes node→voters, and whose workload traffic goes only to peers that host a workload's backends, lazy peering is a strong fit. Its cost is first-packet latency (a handshake on first use).

### Finding B2.3: Talos KubeSpan + the Sidero Discovery Service: an encrypted "affiliate" blob store where the service cannot read membership data

**Evidence**: "Discovery data is encrypted/decrypted by the clients – the cluster members. The discovery service does not have the encryption key." "Data is encrypted with AES-GCM encryption and endpoint data is separately encrypted with AES in ECB mode." "The cluster ID is used as a key to select the affiliates." The service "knows the client version, cluster ID, the number of affiliates, some encrypted data for each affiliate, and a list of encrypted endpoints". README: "Node information is expired (if not updated) after 30 minutes"; "anyone can run their own instance". KubeSpan: "the Discovery Service communicates the apparent IP address of all peers to all other peers"; when a peer is down "Talos will be cycling through the available endpoints until it finds the one which works."
**Source**: [Sidero — Talos discovery docs (v1.11)](https://docs.siderolabs.com/talos/v1.11/configure-your-talos-cluster/system-configuration/discovery); [siderolabs/discovery-service](https://github.com/siderolabs/discovery-service); [Talos KubeSpan docs](https://docs.siderolabs.com/talos/v1.11/networking/kubespan) — Accessed 2026-09-25 (siderolabs.com / talos.dev: trusted open-source list, high; github.com medium-high)
**Confidence**: High
**Verification**: Three Sidero documents agree (vendor-authored, so not fully independent). The encrypted-rendezvous pattern is independently echoed by Tailscale's "shared drop box" and innernet's coordination server.
**Analysis**: KubeSpan is the nearest prior art to "WireGuard mesh for a cluster OS". Its discovery service is a **rendezvous point with no trust**: it hands out endpoint candidates, while trust comes from the cluster secret that encrypts affiliate data (and ultimately from the machine config, B3). Note the ECB-mode endpoint encryption, which leaks equality of endpoints and is a weakness to avoid if copied. The target design's "seed peer answers with the list" plays the same role as the discovery service, but in-cluster, with no external dependency.

### Finding B2.4: Nebula: lighthouses for discovery, CA-signed host certificates that carry name, IP and groups, and production scale in the tens of thousands

**Evidence**: "Discovery nodes (aka lighthouses) allow individual peers to find each other and optionally use UDP hole punching"; "Nebula uses certificates to assert a node's IP address, name, and membership within user-defined groups"; host certificates cannot be altered by hosts ("doing so will invalidate it"); the CA key is "the most sensitive file"; the default CA validity is one year and signed certificates expire "one second before expiration of the CA". The README says Nebula "is also able to connect tens of thousands of computers." Slack's announcement reported Nebula powering its "global overlay network of over 50,000 production hosts".
**Source**: [slackhq/nebula README](https://github.com/slackhq/nebula); [Nebula docs — Quick start](https://nebula.defined.net/docs/guides/quick-start/) (out-of-list primary); [Slack Engineering — Introducing Nebula](https://slack.engineering/introducing-nebula-the-open-source-global-overlay-network-from-slack/) (out-of-list primary; the 50,000 figure dates from a Dec 2021 update) — Accessed 2026-09-25
**Confidence**: High (mechanism). Medium (scale figure: vendor self-report, dated).
**Verification**: The Nebula releases page lists a v2 ASN.1 certificate format ([slackhq/nebula releases](https://github.com/slackhq/nebula/releases)).
**Analysis**: Nebula is **not WireGuard** (it runs its own Noise-based protocol), but its **identity and admission model** transfers directly. A host certificate binding (public key, overlay IP, groups) and signed by an offline or cluster CA is exactly the "admission credential" B1.4 recommends attaching to every node record. Groups map naturally onto *declared roles*. Nebula's lighthouses are just discovery hints, and any node can be one, which matches the proposal's "any peer can answer with the list."

### Finding B2.5: Coordination-server meshes (Tailscale, Headscale, innernet) centralise the key/endpoint drop box; the data plane stays a mesh, with relays for hard NATs

**Evidence**: Tailscale: "The control plane is hub and spoke, but … it carries virtually no traffic. … The data plane is a mesh"; nodes "download a list of public keys and addresses in its domain". DERP relays forward traffic when direct paths fail ("There is never a way for a DERP server to decrypt your traffic"). Peer Relays (public beta, 29 Oct 2025) are customer-run relays achieving throughputs "often multiple orders of magnitude higher than Tailscale's managed DERP fleet"; direct connection success is "over 90% of the time". NAT traversal needs a side channel that "can have a few seconds of latency, and only needs to deliver a few thousand bytes"; for endpoint-dependent (hard) NATs, "to hit a 99.9% chance of success, we need each side to send 170,000 probes". Headscale scopes itself to "a *single* Tailscale network … suitable for a personal use, or a small open-source organisation". innernet: "Every `innernet` network needs a coordination server"; peers join via "an invitation file"; it "should be considered experimental software".
**Source**: [Tailscale — How Tailscale works](https://tailscale.com/blog/how-tailscale-works); [Tailscale — Peer Relays beta](https://tailscale.com/blog/peer-relays-beta); [Tailscale — How NAT traversal works](https://tailscale.com/blog/how-nat-traversal-works) (out-of-list primary); [juanfont/headscale](https://github.com/juanfont/headscale); [tonarino/innernet](https://github.com/tonarino/innernet) — Accessed 2026-09-25
**Confidence**: High (architecture). Medium (Tailscale performance numbers: vendor self-report).
**Verification**: Nebula (B2.4) and KubeSpan (B2.3) implement the same control/data split independently.
**Analysis**: A cluster of servers that the operator owns usually has routable or at least UDP-lenient endpoints, so hard-NAT traversal matters mainly for edge nodes and home labs. The design lesson is that **the side channel for candidates can be the cluster's own membership state**: once a node is admitted, its endpoint candidates are part of its node record (B1.4). A relay is needed only for nodes that cannot be reached directly. Relays could be ordinary cluster nodes (as with Tailscale Peer Relays), advertised through a `relay`/`gateway` role.

### Finding B2.6: Gossip-formed WireGuard meshes (wesher) show the trust failure of a shared symmetric key

**Evidence**: wesher uses memberlist gossip; "The control-plane cluster communication is secured with a pre-shared AES-256 key"; the docs warn "this effectively downgrades some of the security benefits from wireguard"; limitations: overlay IPs by "consistent hashing based on the peer's hostname" (collision risk), split-brain ambiguity between failed and deliberately removed nodes, and a cluster key that "currently never rotates".
**Source**: [costela/wesher README](https://github.com/costela/wesher) — Accessed 2026-09-25 (github.com, medium-high)
**Confidence**: Medium-High (single primary source, self-documented limitations)
**Verification**: Consistent with the Serf/memberlist encryption model described in [serf-internals-and-overdrive-fit.md](../gossip-protocols/serf-internals-and-overdrive-fit.md) Finding 2.1 (symmetric AEAD with key rotation RPC).
**Analysis**: wesher is the target design's discovery idea (seed join + gossip distributes WireGuard keys) *without* a log. Its self-reported limitations are the ones the log exists to fix: IP allocation needs a single authority, "removed vs failed" needs a committed decision, and a shared symmetric key cannot revoke one node. This is useful negative evidence for "gossip alone".



## B3. Admission and Sybil resistance for a permissioned cluster

In a permissioned CFT cluster, Sybil resistance reduces to one question: **what credential, verified by whom, turns "a machine that can reach a WireGuard endpoint" into "a member"?** Every surveyed system answers with the same two-sided handshake: the joiner **pins the cluster's identity** (CA hash) so it is not talking to an impostor, and the cluster **verifies a secret or attestation** from the joiner. Then it issues a longer-lived, per-node, revocable credential.

### Finding B3.1: The join-token pattern is two-sided: the token authenticates the joiner, and a CA hash pins the cluster (kubeadm, k3s)

**Evidence**: Kubernetes bootstrap tokens match `[a-z0-9]{6}\.[a-z0-9]{16}`; the token ID is "public information used for token reference" and the secret is "shared only with trusted parties", used for bearer authentication and as the HMAC key for a JWS-signed `cluster-info` ConfigMap "in the `kube-public` namespace" that "enables discovery before TLS trust is established". Tokens carry an optional RFC3339 `expiration` and "Expired tokens are rejected during authentication". k3s: "The secure token format contains the following parts: `<prefix><cluster CA hash>::<credentials>`", and "the joining node performs the following steps to validate the identity of the server it has connected to, before transmitting credentials." `k3s token rotate` requires that "all servers and any agents that originally joined with the old token must be restarted with the new token."
**Source**: [Kubernetes — Authenticating with Bootstrap Tokens](https://kubernetes.io/docs/reference/access-authn-authz/bootstrap-tokens/) (official, high); [k3s — `k3s token`](https://docs.k3s.io/cli/token) (k3s.io trusted list, high) — Accessed 2026-09-25
**Confidence**: High
**Verification**: Consul `auto_config` uses a JWT `intro_token` validated by servers against `jwt_validation_pub_keys` and `claim_assertions`, after which "the client agent receives … the ACL token, TLS certificates, and gossip encryption key" ([HashiCorp — Consul auto_config](https://developer.hashicorp.com/consul/docs/reference/agent/configuration-file/auto-config), official, high). That is a third independent implementation of "short-lived intro credential → long-lived per-node credentials".
**Analysis**: Mapped onto the target design, a join string such as `ovd1:<cluster-CA-hash>:<wg-endpoint>:<token-id>.<token-secret>` gives the new machine everything it needs. It can pin the cluster CA (it will not accept a peer list or a cert from an impostor that answered on that endpoint), it knows where to go, and it holds a bounded, expiring, single-cluster credential. k3s's **shared, long-lived, rotate-requires-restart** server token is the anti-pattern. kubeadm's per-join, expiring, individually revocable tokens are the pattern to copy.

### Finding B3.2: SPIRE node attestation generalises the token into pluggable evidence: single-use join tokens, TPM DevID with proof of residency, or cloud instance identity

**Evidence**: "During node attestation, the agent and server together verify the identity of the node". Attestor types include AWS EC2 Instance Identity Documents, Azure MSI, GCE Instance Identity Tokens, Kubernetes service-account tokens, hardware ("a private key stored on a Hardware Security Module or Trusted Platform Module"), join tokens and X.509. "A join token is a pre-shared key between a SPIRE Server and Agent", and "join tokens expire immediately after use." After attestation "The server sends back an SVID for the agent node" plus optional node selectors. TPM DevID issues a proof-of-possession challenge (the DevID key chains to a trusted CA) and a proof-of-residency challenge (the key "was generated within and resides in a TPM", validated against manufacturer endorsement CAs).
**Source**: [SPIFFE — SPIRE concepts](https://spiffe.io/docs/latest/spire-about/spire-concepts/) (spiffe.io trusted list, high); [SPIRE — tpm_devid server node attestor](https://github.com/spiffe/spire/blob/main/doc/plugin_server_nodeattestor_tpm_devid.md) (github.com, medium-high) — Accessed 2026-09-25
**Confidence**: High
**Verification**: Kubernetes bootstrap tokens (B3.1) and Consul auto_config are independent token-based schemes. Nebula's CA-signed host certificate (B2.4) is the X.509-style equivalent.
**Analysis**: Treat **admission evidence as a pluggable port** (token | TPM DevID | cloud IID | pre-issued X.509), with the single-use join token as the default. The attestation output (selectors: cloud account, TPM fingerprint, etc.) is exactly the kind of fact that should go into the committed admission record, so later policy (for example "voters only on TPM-attested hardware") can reference it.

### Finding B3.3: The join credential must not be able to mint more privilege than a node needs: a real CVE in a join-token CSR signer

**Evidence**: Talos CVE-2022-36103 (GHSA-7hgc-php5-77qq, published 13 Sep 2022, CVSS 7.9): "A malicious workload could then use the join token to construct a Talos CSR … Due to improper validation while signing a worker node CSR, a Talos control plane node might issue a Talos certificate which allows full access to the Talos API". The join token lived in worker machine config and could be read by workloads via hostPath mounts or metadata-server exposure. Fixed in v1.2.2.
**Source**: [siderolabs/talos security advisory GHSA-7hgc-php5-77qq](https://github.com/siderolabs/talos/security/advisories/GHSA-7hgc-php5-77qq) (github.com, medium-high; primary vendor advisory) — Accessed 2026-09-25
**Confidence**: High
**Verification**: The fix commit "never sign client certificate requests in trustd" ([siderolabs/talos@9eaf33f](https://github.com/siderolabs/talos/commit/9eaf33f3f274e746ca1b442c0a1a0dae0cec088f)); the Talos control-plane docs describe worker nodes obtaining certificates from `trustd` on control-plane nodes, protected by the join token ([Sidero — Control Plane](https://docs.siderolabs.com/talos/v1.13/learn-more/control-plane)).
**Analysis**: Two lessons for the target design, where a microVM workload runs on the same node as the join credential. (1) The long-lived join secret should not persist on a node after admission. Single-use tokens (SPIRE) or tokens deleted after first use close this. (2) The admission service (whatever signs the node certificate) must **derive the certificate's contents (roles, SANs, key usages) from the committed admission record, never from the CSR**. The CSR supplies only the public key. This is also why declared roles must be log-committed rather than gossiped (B7): a role in a certificate is a capability.

### Finding B3.4: The first node's bootstrap is always an explicit, one-time act, never automatic discovery (Elasticsearch, CockroachDB, Consul)

**Evidence**: Elasticsearch: administrators must specify `cluster.initial_master_nodes` for the first multi-node formation; this is "only required the very first time the cluster forms", after which nodes persist and reuse the configuration. CockroachDB: `cockroach init` performs "a one-time initialization of a new multi-node cluster"; nodes started with `--join` are "waiting to be initialized as a new cluster". Consul: in `bootstrap` mode "It is important that no more than one server **per** datacenter be running in this mode" (multiple can self-elect and split-brain); `bootstrap_expect` "must agree with other servers in the cluster".
**Source**: [Elastic — A new era for cluster coordination](https://www.elastic.co/blog/a-new-era-for-cluster-coordination-in-elasticsearch) (out-of-list primary); [CockroachDB — cockroach init](https://docs.cockroachlabs.com/docs/stable/cockroach-init) (out-of-list primary); [HashiCorp — Consul bootstrap parameters](https://developer.hashicorp.com/consul/docs/reference/agent/configuration-file/bootstrap) (official, high) — Accessed 2026-09-25
**Confidence**: High
**Verification**: Three independent vendors converge. Rapid's bootstrap is also explicit: "The seed aggregates alerts until it bootstraps a cluster large enough to support a Paxos quorum (minimum of three processes)" ([Rapid §7](https://www.usenix.org/system/files/conference/atc18/atc18-suresh.pdf)).
**Analysis**: The target design's "every node is HA by default, grow by joining" suggests a two-state bootstrap. (1) **`init` on the first machine** mints the cluster CA and cluster ID, commits a genesis log entry (cluster ID, CA public key, first node's admission, voter set = {self}), and prints the join string. This is the only moment trust is created from nothing. (2) **Every later join is a log-committed admission**, starting with the second node. A 1-voter cluster is legal and "HA-shaped"; it just tolerates zero failures until more `control-plane` nodes join (B5). Avoid Consul-style `bootstrap_expect`, where several machines each decide they might be the first: it is the documented split-brain hazard, and with a single operator a single-node genesis is unnecessary to avoid.

### Finding B3.5: Where the admission decision lives: every surveyed system records it durably in the strongly consistent store, and issues a revocable per-node identity

**Evidence**: SPIRE: after attestation "The server sends back an SVID for the agent node" (the attested node becomes a server-side record from which later selectors derive). Consul auto_config returns per-client TLS certificates and an ACL token issued by the servers. Talos worker nodes obtain their certificate from `trustd` on control-plane nodes. Kubernetes bootstrap tokens are Secrets in etcd with expiry cleaned by the `tokencleaner` controller.
**Source**: [SPIRE concepts](https://spiffe.io/docs/latest/spire-about/spire-concepts/); [Consul auto_config](https://developer.hashicorp.com/consul/docs/reference/agent/configuration-file/auto-config); [Kubernetes bootstrap tokens](https://kubernetes.io/docs/reference/access-authn-authz/bootstrap-tokens/); [Sidero — Control Plane](https://docs.siderolabs.com/talos/v1.13/learn-more/control-plane) — Accessed 2026-09-25
**Confidence**: Medium-High. The pattern is consistent across the four. The exact persistence schema of each was not examined.
**Verification**: The negative case is wesher (B2.6), which has no durable admission record, only a shared key, and names the resulting "failed vs deliberately removed" ambiguity as a limitation.
**Analysis**: For the target design the admission record belongs **on the intent log**: `Admit{node_id, wg_pubkey, declared_roles, failure_domain_labels, attestation_selectors, admitted_by_token_id, cert_serial}`. The token is consumed in the same entry, which makes single-use atomic, and `Revoke{node_id}` is its inverse. Gossip then carries only *signed* node records whose admission can be checked against the log (B1.4). A node whose record is not backed by a committed `Admit` is ignored, whatever gossip says. This keeps the Sybil check local and cheap on every node.



## B4. Failure detection that feeds a committed "node down" decision

The surveyed systems use three structurally different liveness pipelines. Which one feeds the leader's committed decision matters more than which detector algorithm is used:

1. **Gossip failure detection (SWIM/Lifeguard, phi-accrual over gossip) → leader decides.** This is the target design's current proposal, and the Consul/Nomad-server model.
2. **Direct heartbeats to the leader or control plane → leader decides.** Used by Nomad clients, Kubernetes kubelet leases, and Elasticsearch follower checks. Borg is a pull variant, per its EuroSys 2015 paper (not fetched in this session).
3. **Multi-observer alerts → consensus decides a *cut*** (Rapid).

### Finding B4.1: SWIM + Lifeguard is the established gossip detector; its hardening targets false positives caused by the *accuser's* own slowness

**Evidence**: Already established in repo research: Lifeguard adds Local Health Multiplier (a degraded node's accusations become less authoritative), Dogpile (the refutation window shrinks as independent confirmations arrive) and Buddy System (suspicion is sent directly to the suspect). HashiCorp's motivation: "slow message processing caused by CPU exhaustion, network delay, or loss, could lead to incorrectly declaring members as faulty." Foca (used by Corrosion) implements SWIM+Inf+Susp and, per documentation, not Lifeguard.
**Source**: [serf-internals-and-overdrive-fit.md](../gossip-protocols/serf-internals-and-overdrive-fit.md) Findings 1.3, 3.2, citing [Dadgar et al., Lifeguard, arXiv:1707.00788](https://arxiv.org/abs/1707.00788) (academic, high) — Accessed 2026-09-25 (repo document dated 2026-04-19)
**Confidence**: High (inherited, not re-verified this session beyond the arXiv citation)
**Verification**: The Rapid paper's Figure 1 shows Memberlist (SWIM+Lifeguard) is more stable than Akka Cluster under 80% packet loss on 1% of processes but "unstable over a longer period of time", with "extended periods of inconsistencies in the membership view" ([Rapid §2](https://www.usenix.org/system/files/conference/atc18/atc18-suresh.pdf)).
**Analysis**: Lifeguard's Dogpile is itself a crude form of "wait for multiple independent observers". If the target design keeps SWIM, Lifeguard is the minimum bar. See B4.4 for the stronger form.

### Finding B4.2: Phi-accrual turns a binary verdict into a continuous suspicion level, decoupling detection from the action threshold

**Evidence**: Hayashibara et al. (SRDS 2004): "Instead of providing information of a binary nature (trust vs. suspect), accrual failure detectors output a suspicion level on a continuous scale"; the principal merit is "a nearly complete decoupling between application requirements and the monitoring of the environment." φ = −log10(1 − F(t_since_last)), with F estimated from historical inter-arrival times. Chitchat uses "phi-accrual detection to dynamically compute a threshold" and notes "liveness is a local concept."
**Source**: [Hayashibara, Défago, Yared, Katayama — "The φ Accrual Failure Detector", SRDS 2004 (ACM DL entry)](https://dl.acm.org/doi/10.5555/1032662.1034350) (academic, high); [quickwit-oss/chitchat](https://github.com/quickwit-oss/chitchat) — Accessed 2026-09-25
**Confidence**: High
**Verification**: [Akka PhiAccrualFailureDetector API docs](https://doc.akka.io/japi/akka/snapshot/akka/remote/PhiAccrualFailureDetector.html) (independent implementation, medium-high); Cassandra and Akka Cluster both use it, per Rapid §2's list of gossip systems.
**Analysis**: The decoupling is the relevant property. Observers report φ (or a quantised suspicion), and the *leader* applies the action threshold with hysteresis. That lets one detector drive several graded decisions (for example: stop placing new work at φ≥5; commit `NodeDown` and reschedule at φ≥12 sustained for T) instead of one binary cliff.

### Finding B4.3: Gray failure means a node's liveness signal and its usefulness diverge; restarting or evicting on a coarse signal can make things worse

**Evidence**: "a key feature of gray failure is *differential observability*: that the system's failure detectors may not notice problems even when applications are afflicted by them." Example: "if a system's request-handling module is stuck but its heartbeat module is not, then an error-handling module relying on heartbeats will perceive the system as healthy while a client seeking service will perceive it as failed." Under "Recovery that kills, rather than heals", an Azure storage manager kept routing writes to a degraded server, which crashed and rebooted repeatedly until it was taken out of service, which "reduced the total available storage … causing more servers to degrade … a catastrophic cascading failure." Panorama (OSDI'18) turns components into "logical observers" that report errors from the requester's perspective, and detected all "15 real-world gray failures that we reproduced in less than 7 s", where existing approaches detected only one within 300 s.
**Source**: [Huang et al., "Gray Failure: The Achilles' Heel of Cloud-Scale Systems", HotOS'17 (author PDF, Microsoft Research)](https://www.microsoft.com/en-us/research/wp-content/uploads/2017/06/paper-1.pdf) (academic, ACM-published, high; [ACM DL 10.1145/3102980.3103005](https://dl.acm.org/doi/10.1145/3102980.3103005)); [Huang et al., "Capturing and Enhancing In Situ System Observability for Failure Detection" (Panorama), OSDI'18](https://www.usenix.org/conference/osdi18/presentation/huang) (academic, high) — Accessed 2026-09-25
**Confidence**: High
**Verification**: The Rapid paper's motivation independently cites Cassandra and Consul incidents where "failure recovery workflows being triggered ad infinitum have led to Amazon EC2 outages" and "killer bugs" ([Rapid §1](https://www.usenix.org/system/files/conference/atc18/atc18-suresh.pdf)).
**Analysis**: This is the strongest argument against "leader commits NodeDown from a gossip ping signal". A SWIM ping answered by the node agent says nothing about whether the node's microVMs, disks or eBPF dataplane work. The target design already has an **observation** plane full of *requester-side* evidence (allocation health, probe failures, backend reachability). Panorama's lesson: feed peer-observed errors into the down/degraded decision, not only heartbeat liveness. The "recovery that kills" lesson: a committed NodeDown triggers **rescheduling**, an expensive recovery workflow. It must be rate-limited and must not cascade (B4.5).

### Finding B4.4: Rapid's multi-observer, multi-node cut detection (with H/L watermarks) is the published mechanism for stable, flap-free removal decisions

**Evidence**: "Every process p (a subject) is monitored by K observer processes. If L-of-K correct observers cannot communicate with a subject, then the subject is considered observably unresponsive." A process "is in a *stable report mode* if |tally(s)| ≥ H … *unstable report mode* if tally(s) is in between L and H. If there are fewer than L distinct observer alerts about s, we consider it noise." Aggregation rule: "delay proposing a configuration change until there is at least one process in stable report mode and there is no process in unstable report mode." Alerts are "irrevocable". Liveness is maintained by "implicit detections and reinforcements" (after a timeout in unstable mode, observers echo existing REMOVEs). Monitoring is an expander graph of K pseudo-random rings; "every process join or removal results only in 2·K monitoring edges being added or removed." Evaluated with {K,H,L} = {10,9,3}.
**Source**: [Rapid, USENIX ATC'18, §3–§4.2, §7](https://www.usenix.org/system/files/conference/atc18/atc18-suresh.pdf) — Accessed 2026-09-25 (academic, high)
**Confidence**: High (mechanism); Medium (generalisation beyond the authors' evaluation)
**Verification**: Lifeguard's Dogpile (B4.1) and Kubernetes' zone-aware eviction (B4.5) are independent, weaker forms of "don't act on a single accuser; look at the group". Elasticsearch's "considers a node to be faulty only after a number of consecutive checks have failed" is the single-observer, temporal version ([Elastic — Cluster fault detection](https://www.elastic.co/guide/en/elasticsearch/reference/current/cluster-fault-detection.html)).
**Analysis**: For the target design this separates cleanly into *what gossip carries* and *what the log commits*. Gossip (or direct observer→voter reports, as in Rapid-C) carries **edge alerts** `(observer, subject, reachable?)`, which are facts. The leader runs the H/L cut aggregation and commits **one** `MembershipCut{down:[…], up:[…]}` entry, which is the decision. Two benefits: (a) correlated failures (a rack or a zone) are committed as one cut, not N serial decisions that each trigger rescheduling; (b) a single node with a broken NIC cannot get healthy peers declared down, because it contributes at most one of K alerts per subject. The deterministic K-ring topology also suits DST: given the membership set, every node computes the same observer assignment.

### Finding B4.5: Production control planes add hysteresis and blast-radius limits *after* detection: rate limits, zone awareness, grace periods, TTL scaling

**Evidence**: Kubernetes: `--node-monitor-grace-period` (default 40 s) before a node is marked `Unknown`; evictions rate-limited by `--node-eviction-rate` (0.1 nodes/s); when a zone exceeds `--unhealthy-zone-threshold` (0.55) the rate drops to `--secondary-node-eviction-rate` (0.01/s); `--large-cluster-size-threshold` 50. Nodes heartbeat via NodeStatus (default 10 s) and cheaper Lease objects. Nomad: "Nomad adjusts the rate at which Clients heartbeat based on cluster size. The goal is to try to keep the resource cost of processing heartbeats constant regardless of cluster size", with TTL ≈ `<number of Clients> / <max_heartbeats_per_second>` plus `heartbeat_grace`, and `failover_heartbeat_ttl` is "The time by which all Clients must heartbeat after a Server leader election." Elasticsearch: the master removes nodes that cannot apply a cluster-state update within a default of 2 minutes (lagging-node removal).
**Source**: [Kubernetes — Nodes](https://kubernetes.io/docs/concepts/architecture/nodes/) (official, high); [HashiCorp — Nomad server configuration](https://developer.hashicorp.com/nomad/docs/configuration/server) (official, high); [Elastic — Cluster fault detection](https://www.elastic.co/guide/en/elasticsearch/reference/current/cluster-fault-detection.html) (out-of-list primary) — Accessed 2026-09-25
**Confidence**: High
**Verification**: Three independent orchestrator implementations converge on grace period + rate limit. Kubernetes and Nomad both **pause or slow mass action when the failure looks correlated**, and the gray-failure paper supplies the academic rationale (B4.3).
**Analysis**: Three points follow. (1) `failover_heartbeat_ttl` addresses an easily-missed hazard: **a newly elected leader has no liveness history** and must not declare everyone down. The target design's leader-decides model needs an explicit post-election grace period. (2) The "zone is unhealthy → act slower" rule is the operational form of "the observer may be the one partitioned". It sits naturally next to Rapid's cut detection. (3) Nomad's constant-cost heartbeat scaling shows that **direct heartbeats to the leader scale to thousands of nodes** when the TTL stretches with N. This is a real alternative to SWIM for liveness at the target size, at the cost of concentrating load on the leader. The committed decision is then the leader's own observation, not second-hand gossip.

### Finding B4.6: Single-leader "follower checks" are the simplest safe pipeline, and Elasticsearch runs exactly the proposed shape without gossip

**Evidence**: "The elected master periodically checks each of the nodes in the cluster to ensure that they are still connected and healthy. Each node in the cluster also periodically checks the health of the elected master." "Elasticsearch allows these checks to occasionally fail or timeout without taking any action. It considers a node to be faulty only after a number of consecutive checks have failed." A disconnect "is treated as an immediate failure. The master bypasses the timeout and retry setting values and attempts to remove the node from the cluster."
**Source**: [Elastic — Cluster fault detection](https://www.elastic.co/guide/en/elasticsearch/reference/current/cluster-fault-detection.html) — Accessed 2026-09-25 (out-of-list primary, medium-high)
**Confidence**: Medium-High (single vendor source; the design is also corroborated by the vendor's Zen2 blog)
**Verification**: The Kubernetes node controller (leader-elected controller reading kubelet leases) and Nomad (clients heartbeat to the server leader) use the same "leader observes directly" shape (B4.5).
**Analysis**: Elasticsearch clusters are typically hundreds of nodes, not thousands, and its master also holds all cluster state. For thousands of nodes the leader-probes-all model costs O(N) probes per interval on one machine, which Nomad mitigates by stretching the TTL. Rapid-C (B1.2) distributes the probing (K-ring) but centralises the *decision*, which is arguably the best of both.



## B5. Automatic voter-set management from an eligible pool

The target design's working model (3–5 voters chosen automatically from `control-plane` nodes, spread across failure domains, with hysteresis, and everyone else a learner) has direct production precedent. Consul Autopilot and Elasticsearch implement almost exactly this; etcd, CockroachDB and TiKV supply the promotion-safety and placement rules.

### Finding B5.1: Consul Autopilot is the closest production analogue: non-voter until stable, dead-server cleanup gated by a quorum floor, redundancy zones (Enterprise)

**Evidence**: New servers "join as non-voting members initially. They must remain healthy for the duration specified by `ServerStabilizationTime` before becoming voters" (default 10 s). `CleanupDeadServers` "Enables periodic dead server removal from the Raft peer set" (default true), avoiding the 72-hour reap. `MinQuorum` is the "Minimum number of healthy voting servers required to maintain quorum", and Autopilot refuses to remove servers below it. Redundancy zones **(Enterprise)**: non-voting "read replicas … promote to voter status if a voting server fails". Upgrade migrations **(Enterprise)**: new-version servers are added and "old servers demote to non-voters". The open-source library `hashicorp/raft-autopilot` (extracted from Consul) defines a `Promoter` interface (`CalculatePromotionsAndDemotions`, `FilterFailedServerRemovals`, `IsPotentialVoter`) and a `StablePromoter` that "will promote healthy servers to voting status. It will never change the leader ID nor will it perform demotions"; `MinQuorum` "sets the minimum number of servers required in a cluster before autopilot can prune dead servers"; state tracks `FailureTolerance`.
**Source**: [HashiCorp — Consul Autopilot](https://developer.hashicorp.com/consul/docs/manage/scale/autopilot) (official, high); [pkg.go.dev — hashicorp/raft-autopilot](https://pkg.go.dev/github.com/hashicorp/raft-autopilot) (out-of-list primary for HashiCorp's own library, medium-high) — Accessed 2026-09-25
**Confidence**: High
**Verification**: Elasticsearch's automatic voting configuration (B5.3) is an independent vendor implementing the same policy. The library shows the policy is a **pluggable promoter** over a shared health model.
**Analysis**: Autopilot supplies four reusable rules. (1) **Every eligible node enters as a learner and is promoted only after a stabilization window** (this is the hysteresis). (2) **Removal of a dead voter is gated by a quorum floor** (`MinQuorum`): never shrink the voter set below the declared minimum automatically; that would silently trade fault tolerance for availability. (3) **Promotion policy is a pure function** `(current state, health) → (promotions, demotions)`, which suits the target design's deterministic, log-driven voter selection and DST. (4) Redundancy zones express "one voter per failure domain, spares ready to promote". The difference from the target design: Consul's decisions are taken by the Raft leader and applied as Raft configuration changes, but the *eligibility* inputs (server health) come from Serf gossip plus Raft stats. The target design commits roles on the log, which removes one source of nondeterminism.

### Finding B5.2: Learners exist to make adding a member safe: catch up first, promote only when the log matches, and the learner never promotes itself

**Evidence**: Adding a member traditionally risks overloading the leader and triggering elections, and "member add" is a two-step process where invalid URLs could make "the cluster … lose quorum permanently". A learner is "a non-voting member" that "receives all data from leader". "Only after its log has caught up to leader's can learner be promoted to a voting member"; the server rejects premature promotion. "etcd limits the total number of learners that a cluster can have" and "Learner never promotes itself"; learners reject client reads/writes and cannot receive leadership transfer.
**Source**: [etcd — Learner design (v3.6)](https://etcd.io/docs/v3.6/learning/design-learner/) (etcd.io trusted list, high) — Accessed 2026-09-25
**Confidence**: High
**Verification**: Consul's `ServerStabilizationTime` non-voter phase (B5.1); CockroachDB's non-voting replicas, which "don't participate in voting but improve read availability" ([CockroachDB — Replication controls](https://docs.cockroachlabs.com/docs/stable/configure-replication-zones), out-of-list primary).
**Analysis**: etcd **caps** the learner count to protect the leader. The target design inverts this: *every* non-voter (possibly thousands) is a learner holding the full log. That is a different operating point, where learners are the **read and dissemination fan-out**, not a transient staging state. Whether one leader can stream the log to thousands of learners belongs to sibling research A2 (dissemination), and is the main scaling risk of "everyone is a learner". What carries over unchanged is the promotion rule: **promotion requires caught-up and healthy, is decided by the leader, and is committed**, never self-initiated.

### Finding B5.3: Elasticsearch automatically resizes the voting configuration to an odd size and auto-shrinks on departures, with explicit exclusions for planned removals

**Evidence**: "After a node joins or leaves the cluster, Elasticsearch reacts by automatically making corresponding changes to the voting configuration"; "If there is an even number, Elasticsearch leaves one of them out of the voting configuration to ensure that it has an odd size." With `cluster.auto_shrink_voting_configuration: true` (default), "Elasticsearch remains capable of processing cluster state updates as long as all but one of its master-eligible nodes are healthy" (≥3 nodes). If false, "you must remove departed nodes from the voting configuration manually. Use the voting exclusions API". The 7.0 redesign removed `minimum_master_nodes` because it had to be updated "correctly as the cluster scales dynamically" and was "very easy to forget".
**Source**: [Elastic — Voting configurations](https://www.elastic.co/guide/en/elasticsearch/reference/current/modules-discovery-voting.html); [Elastic — A new era for cluster coordination](https://www.elastic.co/blog/a-new-era-for-cluster-coordination-in-elasticsearch) (out-of-list primary, medium-high) — Accessed 2026-09-25
**Confidence**: High (for Elasticsearch behaviour)
**Verification**: Consul Autopilot (B5.1) is an independent vendor with the same "automatic, quorum-aware" stance. Elastic publishes TLA+ specifications with claimed "one-to-one correspondence between the formal model and the production code".
**Analysis**: Elasticsearch's auto-shrink shows a policy choice the target design must make explicitly. **Auto-shrink** (drop dead voters so the remaining ones keep quorum) raises availability but silently lowers fault tolerance. **Consul MinQuorum** refuses to go below a floor. For a single-operator orchestrator the defensible default is: *replace* a dead voter with a healthy eligible learner (keeping N), and never *shrink* below the declared target without an operator action or a visible degraded state. The odd-size rule is a cheap, deterministic heuristic worth copying. Note that Elasticsearch's master-eligible pool is usually small (3–5 dedicated nodes), so its rules were not designed for choosing 5 of 500 eligible nodes, where placement (B5.4) dominates.

### Finding B5.4: Placement across failure domains is the norm (locality labels → voter diversity), and "the log's replicas are the voters" makes voter placement a special case of replica placement

**Evidence**: CockroachDB spreads replicas using node `--locality` tiers, ordered "from most inclusive to least inclusive"; defaults are 3 replicas for table data and "5 replicas for the system database and critical ranges like meta and liveness"; `num_voters` and `voter_constraints` let voting replicas follow different placement rules than non-voting ones. TiKV PD: "PD scheduler uses the labels to optimize TiKV's failure tolerance capability"; "PD schedules replicas of the same `Region` to different data zones"; "If the data zone cannot recover within a period of time, PD removes the replica from this data zone." TigerBeetle recommends 6 replicas over 3 sites ("each site would then contain 2 replicas so that the loss of an entire site would not impair the availability"), with flexible quorums: half the cluster persists a prepare, but "for changing views … at least 4 replicas are needed."
**Source**: [CockroachDB — Replication controls](https://docs.cockroachlabs.com/docs/stable/configure-replication-zones) (out-of-list primary); [TiKV — Topology labels](https://tikv.org/docs/7.1/deploy/configure/topology/) (tikv.org trusted list, high); [TigerBeetle — Cluster recommendations](https://docs.tigerbeetle.com/operating/cluster/) and [ARCHITECTURE.md](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/ARCHITECTURE.md) (out-of-list primary / github.com) — Accessed 2026-09-25
**Confidence**: High
**Verification**: Three independent systems. Consul's redundancy zones (B5.1) express the same rule for control-plane voters.
**Analysis**: A concrete, deterministic voter-selection function consistent with this evidence: given the committed set of admitted, not-down, `control-plane`-role nodes with committed failure-domain labels (region > zone > rack), pick the target N (3 or 5) by **maximising distinct top-level domains, then the next tier**, breaking ties by a stable key (for example the committed admission index), and prefer keeping current voters (stickiness is hysteresis). Because every input is on the log, every node computes the same answer, and only the *leader* proposes the resulting `PromoteLearner`/`DemoteVoter` changes. CockroachDB's use of 5 replicas for its liveness/meta ranges is a hint that the cluster's own membership log deserves more redundancy than ordinary data.

### Finding B5.5: TigerBeetle has standbys in code, but VSR reconfiguration is documented as unimplemented; CometBFT applies validator-set changes with a fixed delay

**Evidence**: TigerBeetle's VSR internals document has a section "Protocol: Reconfiguration" that reads "TODO (Unimplemented)". The source contains standby support (`standbys_max`; replica indices in `replica_count + standbys_max`; `replica.standby()` used by the VOPR simulator). Search results summarise standbys as observing prepares and commits without participating in quorums. CometBFT: validator updates returned from `FinalizeBlock` at height H take effect at "Height `H+2`: The validator set change takes effect and `ValidatorsHash` is updated"; power 0 removes a validator ("set voting power to 0 to remove").
**Source**: [tigerbeetle docs/internals/vsr.md](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/vsr.md); [tigerbeetle src/vsr/superblock.zig](https://github.com/tigerbeetle/tigerbeetle/blob/main/src/vsr/superblock.zig); [cometbft spec/abci/abci++_methods.md](https://github.com/cometbft/cometbft/blob/main/spec/abci/abci%2B%2B_methods.md) (github.com, medium-high) — Accessed 2026-09-25
**Confidence**: Medium. TigerBeetle standby semantics were confirmed from code references and a search summary, not from prose documentation. The CometBFT H+2 rule is quoted from the spec.
**Verification**: The sibling document (A1/A2) covers the `viewstamp` crate's SingleChange reconfiguration (AddLearner/PromoteLearner/DemoteVoter/RemoveLearner). This document does not duplicate it.
**Analysis**: Two transferable ideas. (1) **TigerBeetle, the flagship VSR implementation, does not do online reconfiguration**, so the target design's reliance on VR reconfiguration rests on the Revisited paper and the `viewstamp` crate, not on TigerBeetle production experience. This is a maturity risk to flag (see Conflicting information). (2) CometBFT's **fixed-delay activation** (a set change decided at H applies at H+2) is a simple, deterministic way to make "who votes at log index i" computable by every node from the log alone, including learners replaying history. A voter change committed at index i taking effect at i+k, with at most one change in flight, is the SingleChange discipline in log-index form.



## B6. Leader election in a flat cluster

### Finding B6.1: Electing a leader from gossip is unsafe under partition. The documented failure is two simultaneous masters and silent loss of acknowledged writes.

**Evidence**: Jepsen's analysis of Elasticsearch's pre-7.0 Zen discovery found that "A node will happily support two leaders simultaneously". In a bridged (nontransitive) partition, "Both isolated nodes can see two thirds of the cluster (themselves and the common node), they believe they are eligible for leader election even when minimum_master_nodes is at least a majority". One partition isolating a primary lost over 90% of acknowledged writes ("Of 619 documents inserted, 538 returned successful, but only 54 … appeared in the final read"). Root cause: no monotonic, majority-voted terms.
**Source**: [Kingsbury — "Jepsen: Elasticsearch" (aphyr.com)](https://aphyr.com/posts/317-jepsen-elasticsearch). Out-of-list primary: an independent correctness-testing author whose findings are widely cited; medium-high. Accessed 2026-09-25.
**Confidence**: High
**Verification**: Elastic's own 7.0 redesign replaced Zen with a safety core it describes as "familiar" to users of "Paxos, Raft, Zab and Viewstamped Replication" and verified with TLA+ ([Elastic blog](https://www.elastic.co/blog/a-new-era-for-cluster-coordination-in-elasticsearch)). Rapid's related-work section classifies gossip membership as having "weak consistency guarantees" ([Rapid §2](https://www.usenix.org/system/files/conference/atc18/atc18-suresh.pdf)).
**Analysis**: A gossip view is a per-node opinion. Nodes on opposite sides of a partition, or on either side of a bridge node, hold different opinions and can each conclude "I can see a majority". Safe election needs **(a)** a *fixed, committed* voter set to count against, and **(b)** monotonic terms or views with at most one vote per term. In the target design both come from VR: the voter set is a log-committed configuration, and the view number is monotonic. Gossip may *wake* a view change ("I suspect the primary"). It must never *decide* one.

### Finding B6.2: VR uses deterministic round-robin primaries, Raft uses randomized timeouts, and Tendermint uses deterministic weighted round-robin with round timeouts

**Evidence**: VR Revisited: the primary is chosen round-robin by view number over replicas numbered by IP address, so "the replica with the smallest IP address is replica 1. The primary is chosen round-robin, starting with replica 1, as the system moves to new views." This was quoted from secondary summaries because the primary PDF host refused connections; see Knowledge Gaps. Raft: "election timeouts are chosen randomly from a fixed interval (e.g., 150–300ms). This spreads out the servers so that in most cases only a single server will time out". The Raft authors report: "Initially we planned to use a ranking system … We found that this approach created subtle issues around availability (a lower-ranked server might need to time out and become a candidate again if a higher-ranked server fails, but if it does so too soon, it can reset progress towards electing a leader) … Eventually we concluded that the randomized retry approach is more obvious and understandable." CometBFT requires proposer selection to satisfy "Determinism (R1): Given a validator set V, and two honest validators p and q, for each height h and round r" they select the same proposer, and fairness proportional to voting power. It uses weighted round-robin priorities, and new validators start at "−1.125 * P" to stop add/remove cycles from jumping the queue.
**Source**: [Liskov & Cowling — "Viewstamped Replication Revisited", MIT-CSAIL-TR-2012-021 (abstract page)](https://dspace.mit.edu/handle/1721.1/71763) (academic, high), with the round-robin rule via [the morning paper summary](https://blog.acolyer.org/2015/03/06/viewstamped-replication-revisited/) (medium); [Ongaro & Ousterhout — "In Search of an Understandable Consensus Algorithm", USENIX ATC'14 (extended PDF §5.2)](https://raft.github.io/raft.pdf) (academic; [USENIX page](https://www.usenix.org/conference/atc14/technical-sessions/presentation/ongaro)); [CometBFT proposer-selection spec](https://github.com/cometbft/cometbft/blob/main/spec/consensus/proposer-selection.md) (github.com, medium-high). Accessed 2026-09-25.
**Confidence**: High for Raft and CometBFT. Medium for VR's exact numbering rule, because the primary PDF could not be fetched.
**Verification**: TigerBeetle's architecture document describes view changes as "rotating the role of the primary to a different replica" ([TigerBeetle ARCHITECTURE.md](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/ARCHITECTURE.md)). The etcd raft library adds "Automatic stepping down when the leader loses quorum" and protection against disruptive rejoining nodes (PreVote/CheckQuorum), and calls itself "the most widely used Raft library in production" ([etcd-io/raft](https://github.com/etcd-io/raft)).
**Analysis**: Deterministic rotation has two properties the target design values. **Every node, learners included, can compute who the primary of view v is from the committed configuration alone**, which simplifies client routing and DST invariants (`primary(v) = voters[v mod n]`). Leader selection also becomes a pure function of the log. The cost is the one the Raft authors hit: if the next-in-rotation replica is also down, the cluster must time out again before progress, so worst-case failover grows with the number of consecutive dead voters. With a 3–5 voter set spread across failure domains this worst case is bounded and small. Tendermint's weighted variant is unnecessary here, since all voters are equal in CFT. Its "new members join at the back of the queue" rule is a useful anti-flap idea for voter churn.

### Finding B6.3: Deterministic simulation favours protocols whose non-determinism flows from injectable sources; both rotation and randomized timeouts qualify, but rotation shrinks the state space

**Evidence**: TigerBeetle's simulator "can run an entire cluster on a single thread, injecting various storage faults and infinitely speeding up time", and "unlike formal proofs and model checking, the simulation testing exercises a specific implementation." CometBFT makes determinism of proposer selection a stated protocol requirement (R1). Raft's randomized timeouts are drawn from an interval. In a DST harness, that randomness comes from the seeded entropy source.
**Source**: [TigerBeetle ARCHITECTURE.md](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/ARCHITECTURE.md); [CometBFT proposer-selection spec](https://github.com/cometbft/cometbft/blob/main/spec/consensus/proposer-selection.md); [Raft paper §5.2](https://raft.github.io/raft.pdf). Accessed 2026-09-25.
**Confidence**: Medium. The DST argument is analysis, supported by one practitioner source (TigerBeetle's VSR + VOPR).
**Verification**: Partial. TigerBeetle, which pairs deterministic view rotation with a whole-cluster deterministic simulator, is the single strongest example. No comparative study of DST effectiveness for Raft versus VR was found (Knowledge Gaps).
**Analysis** (interpretation): Both approaches are DST-compatible as long as timers and randomness are injected. Rotation shrinks the reachable state space, because "who will lead next" is not a random variable. That makes invariants like "at most one primary per view" and "the primary of view v is voters[v mod n]" checkable directly, and it makes a failing seed easier to read. The flat-cluster-specific hazard is **learners must never start view changes**. Thousands of learners timing out on a slow primary would be a self-inflicted storm. Only voters may run the view-change protocol. A learner that suspects the primary reports it to a voter or through gossip.

## B7. Node roles and component enablement prior art

### Finding B7.1: Several mature systems ship "all roles on by default, restrict per node as you grow", which is exactly the target UX

**Evidence**: Elasticsearch: when `node.roles` is unset, a node gets `master`, `data`, `data_content`, `data_hot`, `data_warm`, `data_cold`, `data_frozen`, `ingest`, `ml`, `remote_cluster_client` and `transform`. The docs advise: "As the cluster grows … consider separating dedicated master-eligible nodes from dedicated data nodes". A `voting_only` role also exists. Restate: nodes "can have identical configurations" with all roles enabled by default (`admin | http-ingress | log-server | metadata-server | worker`). Joining needs the shared `cluster-name` and `metadata-client.addresses` pointing at "at least one node that runs the metadata-server role". CockroachDB: "you can send SQL requests to any node", and all nodes are symmetric; replica roles are placed by the system (B5.4), not by the operator.
**Source**: [Elastic — Node settings](https://www.elastic.co/guide/en/elasticsearch/reference/current/modules-node.html) (out-of-list primary); [Restate — Clusters](https://docs.restate.dev/server/clusters) (out-of-list primary; cited only as node-role prior art, per scope); [CockroachDB — Architecture overview](https://docs.cockroachlabs.com/docs/stable/architecture/overview) (out-of-list primary). Accessed 2026-09-25.
**Confidence**: High
**Verification**: Three independent vendors.
**Analysis**: This pattern validates the target design's per-node component enablement with all components on by default. Across all three, **role eligibility and role activity are separate**. Elasticsearch has many `master`-eligible nodes, but the voting configuration is a chosen subset (B5.3). Restate runs `metadata-server` on every node by default, yet its metadata store is a bounded consensus group. The target design follows the same split: `control-plane` is *eligibility* to be a voter, and the voter set is the automatically chosen subset.

### Finding B7.2: The classic "server / client" split (Nomad, Consul, k3s, Kubernetes, Talos) makes the consensus role a join-time, explicit, operator-visible choice

**Evidence**: Nomad: "Servers are responsible for accepting jobs from users, managing clients, and computing task placements"; "Each region is expected to have either three or five servers"; clients "communicate using remote procedure calls (RPC) to register themselves, send heartbeats for liveness, wait for new allocations"; "Regions are loosely-coupled using a gossip protocol". k3s: server nodes run "control-plane and datastore components"; HA uses "Three or more server nodes with an integrated etcd datastore"; agents connect through a client-side load balancer holding "a dynamic list of server endpoints" and register with a join token plus a node password whose hash is stored as a Secret, which "protects node identity integrity during re-registration". Talos `machine.type` is `controlplane` or `worker`.
**Source**: [HashiCorp — Nomad architecture](https://developer.hashicorp.com/nomad/docs/concepts/architecture) (official, high); [k3s — Architecture](https://docs.k3s.io/architecture) (k3s.io trusted list, high); [Sidero — Talos v1alpha1 config reference](https://docs.siderolabs.com/talos/v1.10/reference/configuration/v1alpha1/config/) (siderolabs.com trusted list, high). Accessed 2026-09-25.
**Confidence**: High
**Verification**: Four independent systems (Nomad, k3s, Talos, and Consul's server/client model described in B5.1).
**Analysis**: These systems keep "just join a node" UX by giving the joiner **one** decision (server or agent) and making everything else automatic. The target design removes even that decision: every node joins eligible for everything, and voter status is computed. The risk is that with no explicit servers, an operator has no simple answer to "which machines must I keep alive?" Elasticsearch and Consul answer it by exposing the *current* voting configuration and fault tolerance (`FailureTolerance`, B5.1) as first-class status. Two lessons carry over. From k3s: a **per-node secret, hashed in the store and checked on re-registration** stops a second machine from re-joining under an admitted node's name. That maps onto binding `node_id` to the WireGuard public key in the admission record (B3.5). From Nomad: **gossip only between control-plane peers and across regions, with RPC heartbeats from clients** is a proven way to keep gossip traffic off thousands of workers. This is one of the alternatives in B4.

### Finding B7.3: Roles that gate authority must be committed state, not gossiped attributes

**Evidence**: Talos CVE-2022-36103 (B3.3): a join token allowed a worker to obtain a certificate with full API access "Due to improper validation while signing a worker node CSR". Rapid lets applications attach metadata such as `"role":"backend"` at join, carried with the consistent membership view (Rapid §6). Serf tags are "arbitrary key/value metadata per node, used as query filters and for role discovery" ([serf-internals-and-overdrive-fit.md](../gossip-protocols/serf-internals-and-overdrive-fit.md) Finding 2.1).
**Source**: [GHSA-7hgc-php5-77qq](https://github.com/siderolabs/talos/security/advisories/GHSA-7hgc-php5-77qq); [Rapid paper §6](https://www.usenix.org/system/files/conference/atc18/atc18-suresh.pdf). Accessed 2026-09-25.
**Confidence**: Medium-High. The principle follows from the evidence but is an interpretation.
**Verification**: Consul and Nomad take server status from Raft configuration, not Serf tags alone, when deciding quorum (B5.1).
**Analysis**: There are two kinds of role. **Authority-bearing roles** (`control-plane` eligibility, the voter set, any role that grants a certificate capability or places intent-bearing work) must be committed on the log, because voter selection must be deterministic (the user's premise) and because a role is a capability (B3.3). **Advisory attributes** (current load, a component's local health, "gateway currently serving") are observation and can be gossiped. This refines the user's rule: *declared* roles belong to the log, and *effective or health* status of a role belongs to gossip.



## Challenging the hypothesis: "gossip carries facts, the log carries decisions"

**Verdict (interpretation, grounded in the findings above):** the hypothesis is *directionally right* and has strong production precedent. But it is **under-specified in four ways**, and each one is a place where a naive implementation would fail.

**Where the evidence supports it.**
- Service Fabric's federation layer is built on exactly this split, stated as a design principle: "Decouples Failure *Detection* from Failure *Decision* (using Arbitrator)". The arbitrator keeps an "Arbitration Log" ("Log 1: Time T: Node B declared dead"), and "In Production: Multiple Arbitrators, Quorum Based approach" ([Kakivaya et al., Service Fabric, EuroSys 2018, slides 10–11](https://www.csd.uoc.gr/~hy559/fall21/ServiceFabricEuroSys2018.pdf); paper [ACM DL 10.1145/3190508.3190546](https://dl.acm.org/doi/10.1145/3190508.3190546)).
- Rapid-C sends observer alerts to a small ensemble that decides and records view changes (B1.2).
- Consul takes health from Serf, while Autopilot's voter changes are Raft configuration changes (B5.1).
- Elasticsearch has the master decide and publish cluster state (B1.1, B4.6).

Four independent production or academic designs converge on "observations in, committed decisions out".

**Refinement 1: not every fact may be gossiped. Authority-bearing facts belong on the log even though they are "facts".** A node's WireGuard public key, its admission, its declared roles and its failure-domain labels are facts *about* a node, yet each one grants a capability. The key admits traffic, the role determines voter eligibility, and the labels steer voter placement. Talos CVE-2022-36103 (B3.3) and wesher's shared-key limitations (B2.6) are what happens when capability-bearing facts are not tied to a committed admission. The line runs between **authority-bearing** facts (the log) and **advisory, high-volume, owner-written** facts (gossip), not between facts and decisions.

**Refinement 2: gossip is *second-hand* evidence, and a decision should rest on first-hand or multi-observer evidence.** In SWIM, a leader that reads "X is dead" from gossip is acting on one accuser's opinion, filtered through dissemination delay. Every surveyed system that commits "node down" either observes directly (Nomad heartbeats, Elasticsearch follower checks, Kubernetes leases; B4.5–B4.6) or requires agreement from several independent observers (Rapid's L-of-K, Service Fabric's arbitration on lease failure, Lifeguard's Dogpile; B4.1, B4.4). The hypothesis should read: *gossip transports observations; the decider aggregates independent observations with hysteresis before committing.*

**Refinement 3: some decisions must not be a single binary cliff.** Gray failure (B4.3) and Kubernetes' zone-aware eviction (B4.5) show that committed decisions should be **graded** (for example suspect → unschedulable → down) and **rate-limited**, and that correlated failures should be committed as **one cut** (Rapid) rather than N independent decisions. A "decision" is therefore sometimes a *policy output over a stream of facts*, not a single event.

**Refinement 4: gossip can legitimately carry *decisions* as a transport, just never as their source.** Once a decision is committed, thousands of learners must receive it. Census distributes committed epoch changes down a multicast tree built deterministically from the consistent membership ("Trees are constructed deterministically at each node using consistent membership information"; U5 below), and PlumTree/iroh-gossip provides epidemic broadcast trees (B1.6). Using a gossip or broadcast overlay to *disseminate* log entries, verified by index/hash against the log, is compatible with the hypothesis. Using gossip to *originate* a decision is not (B6.1).

**Restated hypothesis (proposed for the next design wave):** *"Only the log creates authority. Nodes gossip signed, self-owned observations, and may relay committed log entries. Every committed membership decision is computed by the leader from aggregated, multi-observer evidence under hysteresis, and is a deterministic function of log state plus that evidence."*

## Known candidates — latest state

| Candidate | Problem(s) | Latest state (as of 2026-09-25) | Maturity / production evidence | Key finding |
|---|---|---|---|---|
| SWIM + Lifeguard (memberlist) | B1, B4 | Stable; Lifeguard is the production hardening. Foca (Rust) documents SWIM+Inf+Susp without Lifeguard | Very high (Consul, Nomad, Serf) | B4.1; repo research Findings 1.3, 3.2 |
| chitchat (Scuttlebutt + phi-accrual) | B1, B4 (observation) | Maintained by Quickwit, MIT licence | Medium (Quickwit) | B1.6 |
| HyParView + PlumTree (iroh-gossip) | B1 (partial views), log dissemination | Active, Apache-2.0/MIT | Medium (iroh ecosystem) | B1.6 |
| Rapid | B1, B4 (consistent membership, cut detection) | Research artifact (Java, 0.8.0) | Low in production; strong in academia (ATC'18) | B1.2, B1.3, B4.4 |
| Ethereum discv5 / ENR | B1 (signed records) | Specified and deployed on the Ethereum network | High (permissionless setting) | B1.4, B1.5 |
| libp2p Kademlia / rendezvous / identify / signed peer records | B1 | Specified; rendezvous Sybil mitigation "TBD" | High (IPFS, Ethereum consensus clients) | B1.4, B1.5 |
| Bitcoin addr/getaddr | B1 | Eclipse attacks documented (USENIX Sec'15) | High, permissionless | B1.5 |
| Tailscale (+ Headscale) | B2 | Peer Relays public beta (Oct 2025); lazy per-peer engine config | Very high (Tailscale); Headscale scoped to a "single tailnet" | B2.2, B2.5 |
| Nebula | B2, B3 | v2 ASN.1 certificate format; lighthouses; relays | High (Slack: >50,000 hosts, per a Dec 2021 report) | B2.4 |
| Talos KubeSpan + Discovery Service | B2, B3 | Service registry default; Kubernetes registry deprecated for k8s 1.32+ | High (Talos) | B2.3, B3.3 |
| innernet / wesher | B2 | innernet "experimental"; wesher documents split-brain/static-key limits | Low | B2.5, B2.6 |
| kubeadm / k3s join tokens | B3 | Stable; k3s secure token pins CA hash | Very high | B3.1 |
| SPIFFE/SPIRE node attestation | B3 | Join token (single-use), TPM DevID, cloud IID, x509pop | High (CNCF graduated) | B3.2 |
| Consul auto_config (JWT intro token) | B3 | Stable | High | B3.1 |
| phi-accrual | B4 | Stable (Akka, Cassandra, chitchat) | High | B4.2 |
| Gray failure / Panorama | B4 | Academic; Panorama OSDI'18 | Research (Azure-derived incidents) | B4.3 |
| Kubernetes node lifecycle, Nomad heartbeats, Elasticsearch fault detection | B4 | Stable | Very high | B4.5, B4.6 |
| Consul Autopilot / raft-autopilot | B5 | OSS: stabilization, dead-server cleanup, MinQuorum. Enterprise: redundancy zones, upgrade migration | Very high | B5.1 |
| etcd learners | B5 | Stable since v3.4; promotion requires caught-up log; learner count capped | Very high | B5.2 |
| Elasticsearch voting configuration | B5, B6, B7 | Automatic, odd-size, auto-shrink default, TLA+-specified | Very high | B5.3 |
| CockroachDB / TiKV placement | B5 | Locality-tier diversity; `num_voters`/`voter_constraints`; PD label scheduling | Very high | B5.4 |
| CometBFT validator updates | B5, B6 | Changes at H take effect at H+2; deterministic weighted proposer rotation | High (Cosmos chains) | B5.5, B6.2 |
| TigerBeetle VSR standbys | B5, B6 | Standbys in code; **reconfiguration "TODO (Unimplemented)"** in internals doc | High for VSR core; **none for reconfiguration** | B5.5 |
| VR Revisited / Raft election | B6 | Stable literature; etcd-raft adds CheckQuorum/PreVote-style protections | Very high (Raft); VR through TigerBeetle | B6.2 |
| Nomad/Consul/k3s/Talos/Elasticsearch/Restate/CockroachDB roles | B7 | See B7.1–B7.2 | Very high | B7.1–B7.3 |

## Unexplored alternatives (ranked)

Ranking criterion: expected value to the target design. That means fit with CFT, a single operator and one flat topology from 1 to thousands of nodes; the size of the problem solved; and how mature the evidence is. Rank 1 is the most valuable to investigate next.

### U1. Multi-observer cut detection feeding the log (Rapid-C shape)

- **Solves**: B4 (flap-free "node down"), correlated failures committed as one decision, and B1 (joins agreed by the same mechanism).
- **Mechanism**: every node monitors K deterministic ring-neighbours computed from the committed membership. Observers send edge alerts (to voters, or by gossip). The leader tallies alerts per subject with H/L watermarks, proposes a cut only when no subject is in the unstable band, and commits one `MembershipCut` entry (B1.2, B4.4).
- **Trade-offs**: 2·K edge changes per membership change; must be implemented from scratch (reference code is Java); needs care for the "implicit detection / reinforcement" liveness rules; more moving parts than SWIM.
- **Evidence and maturity**: ATC'18 paper with a 2000-node evaluation and two application integrations. No known production adoption and no Rust implementation (B1.3).
- **Beats the current direction** on stability under one-way reachability and packet loss, on resistance to a single faulty accuser, and on batching correlated failures. It also suits DST because observer assignment is deterministic.
- **Loses** on maturity and on implementation cost.

### U2. Self-certifying, CA-backed node records as the unit of peer exchange (ENR × Nebula certificate)

- **Solves**: B1 trust ("can I believe a list from any peer?"), B2 key and endpoint distribution, and B3 Sybil resistance.
- **Mechanism**: each node publishes a small record (WG pubkey, endpoints, seq, declared-role digest) signed by its node key. The record carries, or references, a cluster-CA-signed certificate whose contents were derived from the committed `Admit` entry. Peers accept records from anyone once verified (B1.4, B2.4, B3.5).
- **Trade-offs**: needs CA key custody and rotation (Nebula's one-year CA expiry is the cautionary default), certificate revocation propagated through the log, and a fixed record size budget (ENR caps records at 300 B).
- **Evidence and maturity**: very high. ENR (Ethereum), libp2p signed peer records, and Nebula (>50,000 hosts at Slack).
- **Beats the current direction** (an unauthenticated "peer answers with the list") by making peer exchange safe from any member and removing the seed as a trust bottleneck.
- **Loses** nothing material. It adds CA machinery the design needs anyway for admission.

### U3. Service Fabric-style lease-based neighbourhood monitoring with arbitration

- **Solves**: B4 and B6 (consistent failure detection and leader election without gossip).
- **Mechanism**: nodes form a ring and hold *symmetric leases* with n successors and n predecessors. When a lease fails, the node asks a quorum of arbitrators, which record the decision in an arbitration log; the losing side must leave ("IF don't receive any reply within Tm, leave!").
- **Trade-offs**: ring maintenance and join/departure protocols are "intricate" (Microsoft's own word); proprietary design, with no open implementation.
- **Evidence and maturity**: very high. SF-Ring has been "used in production for more than 15 years", and platforms on it span ">100K machines" (Azure SQL DB, Cosmos DB), per the EuroSys'18 slides. [Microsoft Learn — Service Fabric architecture](https://learn.microsoft.com/en-us/azure/service-fabric/service-fabric-architecture) (technical_documentation, high) confirms "a leasing mechanism based on heart beating and arbitration" and "intricate join and departure protocols that only a single owner of a token exists at any time."
- **Beats the current direction** on a crucial safety property: **the accused node self-fences** when it cannot reach the arbitrators. That gives an orchestrator safe rescheduling, because a node that loses its lease stops its workloads before the leader reschedules them.
- **Loses** on implementation complexity. The ring is redundant if the log already provides consistent membership.

### U4. Direct heartbeats to the decider with cluster-size-scaled TTLs (Nomad) or leader follower checks (Elasticsearch), instead of SWIM

- **Solves**: B4, with first-hand evidence feeding the committed decision.
- **Mechanism**: nodes heartbeat to the leader (or to any voter, which forwards). The TTL stretches as N / `max_heartbeats_per_second`, and a `failover_heartbeat_ttl` protects a newly elected leader (B4.5, B4.6).
- **Trade-offs**: concentrates O(N) load on the leader; the detection time grows with N; a leader-side partition looks like mass failure (hence zone-aware rate limits).
- **Evidence and maturity**: very high (Nomad, Elasticsearch, Kubernetes node controller).
- **Beats the current direction** on simplicity and on evidence quality (no second-hand gossip), and it needs no gossip at all for liveness.
- **Loses** at the top of the size range (thousands of nodes across regions) and in availability of detection during leader failover. It fits best as the **combination** "SWIM/K-ring for observation, heartbeats for the commit decision", or as the small-cluster mode of U1.

### U5. Census-style epoch membership with deterministic dissemination trees and regions

- **Solves**: B1 and B4 at multi-region scale, plus dissemination of committed membership to thousands of learners (overlaps sibling A2).
- **Mechanism**: "The system divides time into epochs, and all nodes in the same epoch have identical views of system membership"; a leader collects joins and departures and broadcasts them at epoch boundaries through multicast trees "constructed deterministically at each node using consistent membership information". Regions group nodes by network coordinates, and a partial-knowledge mode lets nodes "know the full membership of their own region but only a few representative nodes from each other region". Parents monitor children and report absences to the region leader.
- **Evidence**: [Cowling, Ports, Liskov, Popa, Gaikwad — "Census: Location-Aware Membership Management for Large-Scale Distributed Systems", USENIX ATC 2009](https://www.usenix.org/legacy/event/usenix09/tech/full_papers/cowling/cowling_html/index.html) (academic, high). Overhead is "typically less than 1 KB/s", with feasibility shown at "over 100,000 nodes even in a high-churn environment". The evaluation method was not verified; see Knowledge Gaps.
- **Maturity**: academic. The authors include VR's co-author Liskov.
- **Beats the current direction** at >10k nodes and multi-region, because it batches membership changes into epochs and reuses consistent membership to build dissemination trees.
- **Loses** below that scale, where it is unnecessary complexity. Worth knowing as the *growth path*: batching membership decisions per epoch is compatible with a VR log.

### U6. On-demand (lazy) WireGuard peering

- **Solves**: B2 at thousands of nodes.
- **Mechanism**: keep the full, admitted peer set in memory, and install a kernel peer only when traffic needs it (Tailscale's `SetPeerConfigFunc`/`SyncDevicePeer`, O(1) per-peer sync; B2.2).
- **Trade-offs**: first-packet handshake latency; needs a hook on first packet to an unknown peer, for example via allowed-IPs routing to a catch-all.
- **Evidence and maturity**: high (Tailscale).
- **Beats** a full-mesh static configuration at any N above a few hundred, and removes O(N) handshake churn to dead peers.
- **Loses** only first-packet latency.

### U7. Eligible-only peer finding (Elasticsearch)

- **Solves**: B1 and B6 bootstrap.
- **Mechanism**: nodes exchange only *voter-eligible* peers until a leader is found, then receive authoritative state from the leader (B1.1).
- **Evidence and maturity**: very high.
- **Beats** "every node exchanges the full list with every other node" during the join storm, because a joiner needs only one reachable voter plus the log.
- **Loses** nothing. It is a strict simplification the target design can adopt.

### U8. Consensus-backed liveness leases (Kubernetes node Leases in etcd)

- **Solves**: B4, by putting liveness on the log.
- **Evidence**: Kubernetes supports "up to 5,000 nodes" and advises that "you can store Event objects in a separate dedicated etcd instance" ([Kubernetes — Considerations for large clusters](https://kubernetes.io/docs/setup/best-practices/cluster-large/), official, high). Nodes heartbeat via NodeStatus and cheaper Lease objects (B4.5).
- **Trade-offs**: at 5,000 nodes with 10 s renewals, roughly 500 consensus writes/s just for liveness. This is exactly the high-volume observation the target design keeps off the log.
- **Ranked last** because it contradicts the observation/intent split. Its one advantage is that "is the lease expired?" becomes a linearizable question. The design should get that property instead from U3-style self-fencing or U4-style leader-held heartbeats, not from log writes.

### Also examined, not ranked

- **Kademlia DHT and rendezvous** (B1.4/B1.5): O(log N) lookups solve a problem a ≤10k-node cluster does not have, since the full table is about 1 MB.
- **Fireflies** (Byzantine membership; cited in Rapid §2): out of scope for CFT.
- **TPM DevID attestation** (B3.2): not an alternative architecture but a pluggable evidence source for U2's admission. Recommended as an optional attestor, not a default.

## Comparison against the current direction

| Aspect | Current direction (as briefed) | Evidence-based assessment | Suggested adjustment (research recommendation) |
|---|---|---|---|
| Join via one existing WireGuard endpoint | New machine configured with one peer's WG endpoint | WireGuard drops handshakes from unknown keys (B2.1). Every surveyed system bootstraps over a separate authenticated channel with CA pinning (B3.1) | Join string = CA hash + endpoint + single-use token. Bootstrap RPC over TLS, or a pre-authorised WG key, before WG mesh membership |
| Peer answers with list of all machines | Blockchain-style peer exchange | Proven mechanism (Elasticsearch, Bitcoin, discv5), but unauthenticated exchange is the eclipse/Sybil surface (B1.5) | Exchange only signed, admission-backed records (U2). Joiners need only voter-eligible peers (U7) |
| Gossip handles discovery | Gossip spreads membership | Fine for *observations*. Gossip views are per-node opinions and must not define membership (B6.1) | Gossip transports signed records and observations. The committed `Admit`/`Revoke` set defines membership |
| One machine selected as leader | Leader selected after discovery | Must be VR view change among *committed voters* only; gossip-based election is unsafe (B6.1) | Voters only; deterministic `voters[v mod n]`; learners never start view changes (B6.3) |
| Voter set 3–5 from `control-plane` nodes, spread over failure domains, hysteresis | Automatic | Strong precedent: Consul Autopilot, Elasticsearch, CockroachDB/TiKV placement (B5.1–B5.4) | Pure selection function over committed state. Learner-first plus stabilization. Replace, don't shrink (floor). Odd size. One change in flight with delayed activation (B5.5) |
| All other nodes are full-log learners | Every non-voter holds the full log | etcd deliberately *caps* learners (B5.2); ZooKeeper showed herd effects at 2000 watchers (Rapid §7). No production system found running thousands of full-log learners off one leader | Treat as the main open risk. Dissemination via tree or relay (U5, sibling A2) rather than leader fan-out |
| Roles declared at join and committed | Log-committed | Supported: roles are capabilities (B3.3, B7.3) | Keep. Split authority-bearing roles (log) from effective/health status (gossip) |
| "Node down" = leader decision from gossip FD | Leader commits | Supported in shape (Service Fabric, Rapid-C). The *input* quality is the weak point (B4.3, B4.4) | Multi-observer cut (U1) or first-hand heartbeats (U4). Graded states, post-election grace, zone-aware rate limits (B4.5). Consider self-fencing (U3) |
| CFT with admission against rogue joiners | Permissioned | Standard token + CA pinning + attestation pattern (B3.1–B3.2) | Single-use tokens; certificate contents from log, never from CSR (B3.3); `init` genesis (B3.4) |

## Conflicting information

### Conflict 1: When voters die, shrink the voting set automatically, or refuse?
**Position A**: auto-shrink by default, so the cluster keeps processing "as long as all but one of its master-eligible nodes are healthy". Source: [Elastic — Voting configurations](https://www.elastic.co/guide/en/elasticsearch/reference/current/modules-discovery-voting.html). Reputation: medium-high (out-of-list primary).
**Position B**: refuse to remove voters below `MinQuorum`, the "Minimum number of healthy voting servers required to maintain quorum". Source: [Consul Autopilot](https://developer.hashicorp.com/consul/docs/manage/scale/autopilot). Reputation: high.
**Assessment**: this is not a factual conflict but a policy trade-off between availability and fault tolerance. Both are authoritative for their own systems. For a single-operator orchestrator, where the operator must be told when fault tolerance drops, Consul's floor plus automatic *replacement* from eligible learners is the safer default. Auto-shrink hides the loss.

### Conflict 2: Deterministic rotation or randomized election?
**Position A**: deterministic proposer selection is a protocol requirement ("Determinism (R1)", [CometBFT spec](https://github.com/cometbft/cometbft/blob/main/spec/consensus/proposer-selection.md), medium-high), and VR rotates primaries round-robin.
**Position B**: the Raft authors abandoned a ranking design because "a lower-ranked server might need to time out and become a candidate again if a higher-ranked server fails … Eventually we concluded that the randomized retry approach is more obvious" ([Raft §5.2](https://raft.github.io/raft.pdf), high).
**Assessment**: the Raft concern is about *availability* when consecutive ranked replicas are down, not safety. With 3–5 voters placed across failure domains, the extra timeouts are bounded. Rotation's DST and routing benefits (B6.3) favour it for the target design. The claim of a DST benefit is interpretation, not measured evidence.

### Conflict 3: Are gossip membership libraries "stable"?
**Position A**: memberlist with Lifeguard is production-hardened against false positives (HashiCorp; repo research Finding 1.3).
**Position B**: Rapid measured Memberlist as "unstable over a longer period of time" under 80% packet loss on 1% of processes, with "extended periods of inconsistencies" ([Rapid §2](https://www.usenix.org/system/files/conference/atc18/atc18-suresh.pdf), high).
**Assessment**: both can be true. Lifeguard reduces false positives from slow *accusers*. Rapid tests *partial connectivity of the subject*, which Lifeguard does not target. Note the author interest on each side: Rapid's authors are evaluating against a competitor, and HashiCorp is describing its own product. Medium confidence either way.

### Conflict 4: Can thousands of full-log learners hang off one VR leader?
**Position A (target design)**: every non-voter is a learner holding the full log.
**Position B**: "etcd limits the total number of learners that a cluster can have" to protect the leader ([etcd learner design](https://etcd.io/docs/v3.6/learning/design-learner/), high). ZooKeeper's watch herd inflated 2000-node bootstrap latency to about 165 s ([Rapid §7](https://www.usenix.org/system/files/conference/atc18/atc18-suresh.pdf)).
**Assessment**: unresolved by this research, and scoped to sibling A2. The evidence says *leader fan-out* to thousands does not scale. It does not say *tree or relay dissemination* of a committed log fails (Census, U5, suggests it can work).

### Conflict 5: TigerBeetle as maturity evidence for VR reconfiguration
**Position A**: VSR is production-grade (TigerBeetle).
**Position B**: TigerBeetle's own internals document lists reconfiguration as "TODO (Unimplemented)" ([vsr.md](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/vsr.md)).
**Assessment**: TigerBeetle validates VR's steady state and view change, not online membership change. Evidence for voter-set reconfiguration under VR has to come from VR Revisited §7 and the `viewstamp` crate (sibling A1), not from TigerBeetle.

## Knowledge gaps

### Gap 1: VR Revisited primary text unreachable
**Issue**: the exact primary-selection wording (round-robin over replicas numbered by IP) and the reconfiguration epoch rules were taken from secondary summaries. **Attempted**: pmg.csail.mit.edu PDF (ECONNREFUSED), the MIT DSpace bitstream (HTTP 405), and the DSpace abstract page (abstract only). **Recommendation**: read §4 and §7 of MIT-CSAIL-TR-2012-021 directly before the design relies on the rule.

### Gap 2: Operational WireGuard scaling at 1k–10k peers
**Issue**: the kernel hard limit is 2^20 peers per device, but no authoritative measurement of reconfiguration time, handshake load from stale peers, or memory at thousands of peers was found. Only vendor/SEO blogs made such claims, and they were excluded. **Attempted**: kernel source, the WireGuard whitepaper, and Tailscale engine docs. **Recommendation**: a Tier-3-style spike on the pinned kernel, measuring netlink set/dump time and handshake CPU at 1k/5k/10k peers, with and without lazy peering.

### Gap 3: Rapid in production, and in Rust
**Issue**: no production deployment or Rust implementation found. **Attempted**: web search, the GitHub repository, and the paper's own integration list. **Recommendation**: prototype the cut-detection aggregator as a pure function under DST before committing to it.

### Gap 4: TigerBeetle standby semantics
**Issue**: standby behaviour was confirmed only from code references and a search summary; there is no prose specification. **Recommendation**: read `src/vsr/replica.zig` standby paths if standbys are used as a design precedent.

### Gap 5: Census and Service Fabric evaluation details
**Issue**: Census's 100k-node claim was not checked for simulation versus deployment. Service Fabric evidence comes from EuroSys'18 presentation slides and Microsoft Learn; the full paper text was not read. **Recommendation**: read the Census evaluation section and the Service Fabric paper §4 (Federation) before relying on either.

### Gap 6: Talos machine-token documentation text
**Issue**: Talos's `machine.token` semantics were established from the CVE advisory and the control-plane docs; the configuration-reference text itself was not retrieved (404 or empty). **Impact**: low, since the advisory is primary evidence of the mechanism.

### Gap 7: Borg's polling-based liveness
**Issue**: Borg (EuroSys 2015) is referenced in the B4 taxonomy as a pull-based variant, but the paper text was not retrieved; only the abstract was ("clusters each with up to tens of thousands of machines"). **Recommendation**: read §3 of the Borg paper if the pull-based variant is considered.

### Gap 8: Comparative DST effectiveness of rotation versus randomized election
**Issue**: no study found. B6.3 is analysis. **Recommendation**: treat it as a hypothesis to test in the project's own DST harness.

### Gap 9: Mesh products not examined
**Issue**: NetBird, Netmaker and Cilium's WireGuard mode were not researched, for lack of turns. **Impact**: low. Their control/data split matches B2.5.

## Recommendation for the target design

These are research recommendations for the DESIGN wave, not decisions. Each names the evidence it rests on.

1. **Make genesis explicit and single-node.** An `init` on the first machine mints the cluster CA and cluster ID and commits a genesis entry: voter set = {self}, the first `Admit`. Do not use `bootstrap_expect`-style multi-machine genesis. (B3.4; Elasticsearch, CockroachDB, Consul.)

2. **Replace "configure one WireGuard endpoint" with a join string that pins the cluster and carries a single-use credential.** Shape: `CA-hash + endpoint(s) + token-id.secret` (k3s/kubeadm shape). The first contact goes over an authenticated bootstrap channel (for example TLS on a join port, verified against the CA hash), because WireGuard ignores unknown keys. The token is consumed atomically by the `Admit` log entry. (B2.1, B3.1, B3.2, B3.5.)

3. **Derive node certificate contents from the committed admission record, never from the CSR.** The CSR supplies only the public key. Remove the join credential from the node after admission. (B3.3; Talos CVE-2022-36103.)

4. **Gossip only signed, admission-backed node records**, containing WG pubkey, endpoint candidates, seq and a role digest, plus per-node observations. Accept peer lists from any member once records verify. Joiners need only one reachable voter-eligible peer and the log. (U2, U7; B1.4, B1.5.)

5. **Keep "the leader commits NodeDown", but upgrade its inputs and outputs**:
   - Aggregate *multi-observer* edge alerts with H/L watermarks, and commit correlated failures as one cut (U1).
   - Or use first-hand heartbeats with N-scaled TTLs (U4).
   - Commit graded states (suspect → unschedulable → down).
   - Apply a post-election grace period (`failover_heartbeat_ttl` analogue) and zone-aware rate limits (Kubernetes).
   - Evaluate self-fencing on lease loss (U3), so that rescheduling after NodeDown cannot double-run workloads.

   (B4.1–B4.6.)

6. **Voter management as a pure, log-driven function**:
   - Eligible pool = admitted, not-down, `control-plane`-role nodes.
   - Maximise failure-domain diversity, prefer incumbents (stickiness), keep the size odd, and target 3 or 5.
   - Every candidate enters as a learner and is promoted only when caught up and healthy for a stabilization window.
   - *Replace* dead voters from the pool; never shrink below the floor without a visible degraded state or operator action.
   - One change in flight, activated at index + k.

   (B5.1–B5.5; Consul Autopilot, etcd, Elasticsearch, CockroachDB/TiKV, CometBFT.)

7. **Only voters participate in view changes.** Use deterministic `voters[v mod n]` primaries. Learners report suspicion; they never initiate view changes. (B6.1–B6.3.)

8. **Adopt on-demand WireGuard peering.** Keep the full admitted peer set in memory and program kernel peers lazily. Offer relays through a role for nodes behind hard NATs. (U6; B2.2, B2.5.)

9. **Separate authority-bearing roles (log) from effective role status (gossip).** Expose current voters and failure tolerance as first-class operator status, so "which machines must I keep alive?" has an answer in a serverless-looking flat cluster. (B7.1–B7.3.)

10. **Spikes to run before locking the design** (in order):
    - (a) WireGuard at 1k/5k/10k peers on the pinned kernel (Gap 2).
    - (b) The Rapid cut-detection aggregator as a pure function under DST (Gap 3).
    - (c) With sibling A2: committed-log dissemination to thousands of learners via tree or relay, rather than leader fan-out (Conflict 4).

## Source Analysis

| Source group | Domain(s) | Reputation | Type | Access date | Cross-verified |
|---|---|---|---|---|---|
| Rapid (ATC'18), Eclipse attacks (USENIX Sec'15), Panorama (OSDI'18), Raft (ATC'14), Census (ATC'09) | usenix.org | High (1.0) | academic | 2026-09-25 | Y |
| Lifeguard | arxiv.org | High | academic | 2026-09-25 (via repo research) | Y |
| φ-accrual (SRDS'04), Gray Failure (HotOS'17), Service Fabric (EuroSys'18) | dl.acm.org (+ author/course PDF mirrors) | High | academic | 2026-09-25 | Y |
| VR Revisited (abstract) | dspace.mit.edu | High | academic | 2026-09-25 | Partial (body via secondary) |
| WireGuard (NDSS'17 whitepaper) | wireguard.com | High (peer-reviewed; out-of-list host) | academic | 2026-09-25 | Y |
| Linux `drivers/net/wireguard/messages.h` | github.com (kernel mirror) | High | official source | 2026-09-25 | Partial |
| Kubernetes (bootstrap tokens, nodes, large clusters) | kubernetes.io | High | official | 2026-09-25 | Y |
| Consul (auto_config, bootstrap, Autopilot), Nomad (server config, architecture) | developer.hashicorp.com | High | official | 2026-09-25 | Y |
| Service Fabric architecture | learn.microsoft.com | High | technical docs | 2026-09-25 | Y |
| etcd learner design | etcd.io | High | open source | 2026-09-25 | Y |
| k3s token, architecture | docs.k3s.io | High | open source | 2026-09-25 | Y |
| SPIRE concepts | spiffe.io | High | open source | 2026-09-25 | Y |
| Talos discovery, KubeSpan, control plane, config reference | docs.siderolabs.com | High | open source | 2026-09-25 | Y |
| TiKV topology labels | tikv.org | High | open source | 2026-09-25 | Y |
| Repos/specs: rapid, devp2p, libp2p specs, iroh-gossip, chitchat, discovery-service, wesher, headscale, innernet, nebula, spire, talos advisory/commit, cometbft specs, tigerbeetle docs/code, etcd-io/raft | github.com | Medium-High (0.8) | industry / source | 2026-09-25 | Mostly Y |
| Elasticsearch (discovery, voting, Zen2 blog, fault detection, node roles) | elastic.co | Medium-High (out-of-list primary) | official vendor | 2026-09-25 | Y (TLA+-backed; Jepsen) |
| EIP-778 | eips.ethereum.org | Medium-High (out-of-list primary) | specification | 2026-09-25 | Y |
| Tailscale (how it works, peer relays, NAT traversal), wgengine, raft-autopilot | tailscale.com, pkg.go.dev | Medium-High (out-of-list primary) | vendor docs | 2026-09-25 | Y |
| Nebula docs, Slack engineering | nebula.defined.net, slack.engineering | Medium-High (out-of-list primary) | vendor docs | 2026-09-25 | Y |
| CockroachDB (init, replication, architecture) | docs.cockroachlabs.com | Medium-High (out-of-list primary) | vendor docs | 2026-09-25 | Y |
| TigerBeetle cluster docs | docs.tigerbeetle.com | Medium-High (out-of-list primary) | vendor docs | 2026-09-25 | Y |
| Restate clusters (node-role prior art only) | docs.restate.dev | Medium-High (out-of-list primary) | vendor docs | 2026-09-25 | Y |
| Jepsen: Elasticsearch | aphyr.com | Medium-High (out-of-list primary; independent tester) | industry | 2026-09-25 | Y (Elastic's own redesign) |
| Akka PhiAccrualFailureDetector API | doc.akka.io | Medium-High | vendor docs | 2026-09-25 | Y |
| the morning paper (VR summary) | blog.acolyer.org | Medium (0.6) | secondary | 2026-09-25 | Flagged (primary unreachable) |

**Reputation**: High ≈ 30 (42%) | Medium-High ≈ 41 (57%) | Medium 1 (1%) | **Average ≈ 0.88**. Excluded: SEO/vendor blogs on WireGuard scaling (tech-insider.org, quickztna.com, tunnelpicks.net, meshwg.com, linuxgd.medium.com), oneuptime.com Talos blogs, and deepwiki.com AI-generated summaries. All were rejected as unverifiable or low-trust.

**Bias notes**: Rapid's evaluation is by its authors, against competitors. HashiCorp, Elastic, Tailscale, Slack/Defined, Sidero and TigerBeetle describe their own products. Wherever possible, independent corroboration (a different vendor or an academic source) is listed in each finding's Verification line.

## Full Citations

[1] Suresh, L., Malkhi, D., Gopalan, P., Porto Carreiro, I., Lokhandwala, Z. "Stable and Consistent Membership at Scale with Rapid". USENIX ATC 2018. https://www.usenix.org/system/files/conference/atc18/atc18-suresh.pdf (presentation: https://www.usenix.org/conference/atc18/presentation/suresh). Accessed 2026-09-25.
[2] lalithsuresh/rapid (reference implementation). https://github.com/lalithsuresh/rapid. Accessed 2026-09-25.
[3] Elastic. "Discovery — seed hosts providers". https://www.elastic.co/guide/en/elasticsearch/reference/current/modules-discovery-hosts-providers.html. Accessed 2026-09-25.
[4] Elastic. "Voting configurations". https://www.elastic.co/guide/en/elasticsearch/reference/current/modules-discovery-voting.html. Accessed 2026-09-25.
[5] Elastic. "A new era for cluster coordination in Elasticsearch". https://www.elastic.co/blog/a-new-era-for-cluster-coordination-in-elasticsearch. Accessed 2026-09-25.
[6] Elastic. "Cluster fault detection". https://www.elastic.co/guide/en/elasticsearch/reference/current/cluster-fault-detection.html. Accessed 2026-09-25.
[7] Elastic. "Node settings". https://www.elastic.co/guide/en/elasticsearch/reference/current/modules-node.html. Accessed 2026-09-25.
[8] Ethereum. "EIP-778: Ethereum Node Records". https://eips.ethereum.org/EIPS/eip-778. Accessed 2026-09-25.
[9] ethereum/devp2p. "Node Discovery Protocol v5 — Theory". https://github.com/ethereum/devp2p/blob/master/discv5/discv5-theory.md. Accessed 2026-09-25.
[10] libp2p. "Rendezvous Protocol". https://github.com/libp2p/specs/blob/master/rendezvous/README.md. Accessed 2026-09-25.
[11] libp2p. "RFC 0003 — Routing Records". https://github.com/libp2p/specs/blob/master/RFC/0003-routing-records.md. Accessed 2026-09-25.
[12] Heilman, E., Kendler, A., Zohar, A., Goldberg, S. "Eclipse Attacks on Bitcoin's Peer-to-Peer Network". USENIX Security 2015. https://www.usenix.org/conference/usenixsecurity15/technical-sessions/presentation/heilman. Accessed 2026-09-25.
[13] n0-computer. "iroh-gossip". https://github.com/n0-computer/iroh-gossip. Accessed 2026-09-25.
[14] quickwit-oss. "chitchat". https://github.com/quickwit-oss/chitchat. Accessed 2026-09-25.
[15] Donenfeld, J. A. "WireGuard: Next Generation Kernel Network Tunnel". NDSS 2017 (whitepaper). https://www.wireguard.com/papers/wireguard.pdf. Accessed 2026-09-25.
[16] Linux kernel. `drivers/net/wireguard/messages.h`. https://raw.githubusercontent.com/torvalds/linux/master/drivers/net/wireguard/messages.h. Accessed 2026-09-25.
[17] Tailscale. "wgengine" package docs. https://pkg.go.dev/tailscale.com/wgengine. Accessed 2026-09-25.
[18] Sidero Labs. "Discovery" (Talos v1.11). https://docs.siderolabs.com/talos/v1.11/configure-your-talos-cluster/system-configuration/discovery. Accessed 2026-09-25.
[19] Sidero Labs. "discovery-service". https://github.com/siderolabs/discovery-service. Accessed 2026-09-25.
[20] Sidero Labs. "KubeSpan" (Talos v1.11). https://docs.siderolabs.com/talos/v1.11/networking/kubespan. Accessed 2026-09-25.
[21] slackhq. "nebula". https://github.com/slackhq/nebula (releases: https://github.com/slackhq/nebula/releases). Accessed 2026-09-25.
[22] Defined Networking. "Nebula Quick Start". https://nebula.defined.net/docs/guides/quick-start/. Accessed 2026-09-25.
[23] Slack Engineering. "Introducing Nebula, the open source global overlay network from Slack". https://slack.engineering/introducing-nebula-the-open-source-global-overlay-network-from-slack/. Accessed 2026-09-25.
[24] Tailscale. "How Tailscale works". https://tailscale.com/blog/how-tailscale-works. Accessed 2026-09-25.
[25] Tailscale. "Tailscale Peer Relays (beta)". 2025-10-29. https://tailscale.com/blog/peer-relays-beta. Accessed 2026-09-25.
[26] Tailscale. "How NAT traversal works". https://tailscale.com/blog/how-nat-traversal-works. Accessed 2026-09-25.
[27] juanfont. "headscale". https://github.com/juanfont/headscale. Accessed 2026-09-25.
[28] tonarino. "innernet". https://github.com/tonarino/innernet. Accessed 2026-09-25.
[29] costela. "wesher". https://github.com/costela/wesher. Accessed 2026-09-25.
[30] Kubernetes. "Authenticating with Bootstrap Tokens". https://kubernetes.io/docs/reference/access-authn-authz/bootstrap-tokens/. Accessed 2026-09-25.
[31] k3s. "k3s token". https://docs.k3s.io/cli/token. Accessed 2026-09-25.
[32] HashiCorp. "Consul — auto_config parameters". https://developer.hashicorp.com/consul/docs/reference/agent/configuration-file/auto-config. Accessed 2026-09-25.
[33] SPIFFE. "SPIRE Concepts". https://spiffe.io/docs/latest/spire-about/spire-concepts/. Accessed 2026-09-25.
[34] SPIFFE. "Server plugin: NodeAttestor tpm_devid". https://github.com/spiffe/spire/blob/main/doc/plugin_server_nodeattestor_tpm_devid.md. Accessed 2026-09-25.
[35] Sidero Labs. "GHSA-7hgc-php5-77qq / CVE-2022-36103". 2022-09-13. https://github.com/siderolabs/talos/security/advisories/GHSA-7hgc-php5-77qq. Accessed 2026-09-25.
[36] Sidero Labs. "fix: never sign client certificate requests in trustd". https://github.com/siderolabs/talos/commit/9eaf33f3f274e746ca1b442c0a1a0dae0cec088f. Accessed 2026-09-25.
[37] Sidero Labs. "Control Plane" (Talos v1.13). https://docs.siderolabs.com/talos/v1.13/learn-more/control-plane. Accessed 2026-09-25.
[38] Cockroach Labs. "cockroach init". https://docs.cockroachlabs.com/docs/stable/cockroach-init. Accessed 2026-09-25.
[39] HashiCorp. "Consul — bootstrap parameters". https://developer.hashicorp.com/consul/docs/reference/agent/configuration-file/bootstrap. Accessed 2026-09-25.
[40] Dadgar, A., Phillips, J., Currey, J. "Lifeguard: Local Health Awareness for More Accurate Failure Detection". arXiv:1707.00788. https://arxiv.org/abs/1707.00788. (via repo research) Accessed 2026-09-25.
[41] Hayashibara, N., Défago, X., Yared, R., Katayama, T. "The φ Accrual Failure Detector". SRDS 2004. https://dl.acm.org/doi/10.5555/1032662.1034350. Accessed 2026-09-25.
[42] Akka. "PhiAccrualFailureDetector". https://doc.akka.io/japi/akka/snapshot/akka/remote/PhiAccrualFailureDetector.html. Accessed 2026-09-25.
[43] Huang, P., Guo, C., Zhou, L., Lorch, J. R., Dang, Y., Chintalapati, M., Yao, R. "Gray Failure: The Achilles' Heel of Cloud-Scale Systems". HotOS 2017. https://www.microsoft.com/en-us/research/wp-content/uploads/2017/06/paper-1.pdf ; https://dl.acm.org/doi/10.1145/3102980.3103005. Accessed 2026-09-25.
[44] Huang, P. et al. "Capturing and Enhancing In Situ System Observability for Failure Detection" (Panorama). OSDI 2018. https://www.usenix.org/conference/osdi18/presentation/huang. Accessed 2026-09-25.
[45] Kubernetes. "Nodes". https://kubernetes.io/docs/concepts/architecture/nodes/. Accessed 2026-09-25.
[46] HashiCorp. "Nomad — server block". https://developer.hashicorp.com/nomad/docs/configuration/server. Accessed 2026-09-25.
[47] HashiCorp. "Consul — Autopilot". https://developer.hashicorp.com/consul/docs/manage/scale/autopilot. Accessed 2026-09-25.
[48] HashiCorp. "raft-autopilot". https://pkg.go.dev/github.com/hashicorp/raft-autopilot. Accessed 2026-09-25.
[49] etcd. "Learner design" (v3.6). https://etcd.io/docs/v3.6/learning/design-learner/. Accessed 2026-09-25.
[50] Cockroach Labs. "Replication controls". https://docs.cockroachlabs.com/docs/stable/configure-replication-zones. Accessed 2026-09-25.
[51] TiKV. "Topology labels". https://tikv.org/docs/7.1/deploy/configure/topology/. Accessed 2026-09-25.
[52] TigerBeetle. "Cluster recommendations". https://docs.tigerbeetle.com/operating/cluster/. Accessed 2026-09-25.
[53] TigerBeetle. "ARCHITECTURE.md". https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/ARCHITECTURE.md. Accessed 2026-09-25.
[54] TigerBeetle. "docs/internals/vsr.md". https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/vsr.md. Accessed 2026-09-25.
[55] TigerBeetle. "src/vsr/superblock.zig". https://github.com/tigerbeetle/tigerbeetle/blob/main/src/vsr/superblock.zig. Accessed 2026-09-25.
[56] CometBFT. "ABCI++ methods". https://github.com/cometbft/cometbft/blob/main/spec/abci/abci%2B%2B_methods.md. Accessed 2026-09-25.
[57] CometBFT. "Proposer selection". https://github.com/cometbft/cometbft/blob/main/spec/consensus/proposer-selection.md. Accessed 2026-09-25.
[58] Kingsbury, K. "Jepsen: Elasticsearch". https://aphyr.com/posts/317-jepsen-elasticsearch. Accessed 2026-09-25.
[59] Liskov, B., Cowling, J. "Viewstamped Replication Revisited". MIT-CSAIL-TR-2012-021. https://dspace.mit.edu/handle/1721.1/71763. Accessed 2026-09-25.
[60] Colyer, A. "Viewstamped Replication Revisited" (the morning paper). https://blog.acolyer.org/2015/03/06/viewstamped-replication-revisited/. Accessed 2026-09-25. [secondary; primary PDF unreachable]
[61] Ongaro, D., Ousterhout, J. "In Search of an Understandable Consensus Algorithm (Extended Version)". USENIX ATC 2014. https://raft.github.io/raft.pdf ; https://www.usenix.org/conference/atc14/technical-sessions/presentation/ongaro. Accessed 2026-09-25.
[62] etcd-io. "raft". https://github.com/etcd-io/raft. Accessed 2026-09-25.
[63] Restate. "Clusters". https://docs.restate.dev/server/clusters. Accessed 2026-09-25. (node-role prior art only)
[64] k3s. "Architecture". https://docs.k3s.io/architecture. Accessed 2026-09-25.
[65] HashiCorp. "Nomad architecture". https://developer.hashicorp.com/nomad/docs/concepts/architecture. Accessed 2026-09-25.
[66] Cockroach Labs. "Architecture overview". https://docs.cockroachlabs.com/docs/stable/architecture/overview. Accessed 2026-09-25.
[67] Sidero Labs. "Talos v1alpha1 configuration reference" (v1.10). https://docs.siderolabs.com/talos/v1.10/reference/configuration/v1alpha1/config/. Accessed 2026-09-25.
[68] Kakivaya, G. et al. "Service Fabric: A Distributed Platform for Building Microservices in the Cloud". EuroSys 2018. https://dl.acm.org/doi/10.1145/3190508.3190546 (slides: https://www.csd.uoc.gr/~hy559/fall21/ServiceFabricEuroSys2018.pdf). Accessed 2026-09-25.
[69] Microsoft. "Architecture of Azure Service Fabric". https://learn.microsoft.com/en-us/azure/service-fabric/service-fabric-architecture. Accessed 2026-09-25.
[70] Cowling, J., Ports, D. R. K., Liskov, B., Popa, R. A., Gaikwad, A. "Census: Location-Aware Membership Management for Large-Scale Distributed Systems". USENIX ATC 2009. https://www.usenix.org/legacy/event/usenix09/tech/full_papers/cowling/cowling_html/index.html. Accessed 2026-09-25.
[71] Kubernetes. "Considerations for large clusters". https://kubernetes.io/docs/setup/best-practices/cluster-large/. Accessed 2026-09-25.
[72] Google. "Large-scale cluster management at Google with Borg" (abstract only). EuroSys 2015. https://research.google/pubs/large-scale-cluster-management-at-google-with-borg/. Accessed 2026-09-25.
[repo] nw-researcher. "HashiCorp Serf — Internals and Fit for Overdrive". docs/research/gossip-protocols/serf-internals-and-overdrive-fit.md. 2026-04-19.

## Research Metadata

- **Duration**: one session, about 50 turns.
- **Sources**: about 85 examined, 72 cited. Cross-referenced major claims: 24 of 27 findings carry at least one independent verification source.
- **Confidence distribution** (findings B1.1–B7.3): High ≈ 63%, Medium-High ≈ 26%, Medium ≈ 11%, Low 0%.
- **Tool failures**:
  - WebFetch could not parse PDFs (Rapid, WireGuard, Gray Failure, Raft, Service Fabric); these were read page-by-page from the saved binaries.
  - pmg.csail.mit.edu refused connections, and the DSpace bitstream returned 405 (VR Revisited body; Gap 1).
  - The Talos config reference returned no field text, and one URL 404'd.
  - ACM DL returned 403 for the Gray Failure page; the author PDF was used instead.
  - The TigerBeetle prose docs contain no standby text; code references were used instead (Gap 4).
- **Output**: `docs/research/architecture/flat-cluster-membership-discovery-and-admission-comprehensive-research.md`.
