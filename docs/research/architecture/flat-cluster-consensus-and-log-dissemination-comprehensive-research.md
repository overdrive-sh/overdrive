# Research: Flat-Cluster Consensus and Replicated-Log Dissemination for an Orchestrator Control Plane

**Date**: 2026-09-25 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium-High overall (High on prior-art patterns; Medium on `viewstamp` internals, which rest on documentation rather than code) | **Sources**: 70 citation entries (~85 distinct URLs), average reputation ≈ 0.90

**Scope**: Problem cluster A — ordering intent and distributing it (A1–A7), plus a state-of-repo report on the `viewstamp` crate. This document researches a *target* design; it does not evaluate the current codebase.

## Executive Summary

**Shape of the design.** The design's *shape* is among the best-supported patterns in production consensus:
- a small, automatically placed voter set;
- every other node a non-voting full-log replica;
- writes forwarded to the leader, and the scheduler at the leader;
- decisions on the log and observation off it.

Kafka KRaft (brokers observe the metadata log by pull), ZooKeeper (observers, including observers fed by followers), Spanner (read-only replicas plus witnesses) and, most closely, Cassandra's Transactional Cluster Metadata all converge on it. CEP-21 is a flat, gossip-native cluster that moved membership, ownership and schema *decisions* off gossip onto a Paxos log run by a small subset sized to the cluster, while keeping gossip for liveness. So the working hypothesis "gossip carries facts, the log carries decisions" survives, with two refinements:
- **Authority and transport are separate.** Facts are owner-authoritative and decisions are log-authoritative, but committed log entries can travel over gossip or relay trees, because a hash chain makes them verifiable from any relay.
- **Safety-gating facts need a leader-observed path.** Facts that gate safety decisions ("node is down", "learner caught up", "supports version v") also reach the leader through a lease or heartbeat path, as KRaft does with broker heartbeats and `FenceBrokerRecord`.

**Protocol.** VR/VSR and Raft are the same crash-fault protocol class. The protocol-level argument for VSR is storage faults. TigerBeetle's protocol-aware recovery survived Jepsen's corruption nemesis ("exceptional resilience"), while typical Raft deployments did not: Jepsen's 2025 NATS analysis saw 49.7% of acknowledged writes lost from minority-node corruption, and Antithesis (2026) reports bugs in every Raft implementation it examined, openraft included.

**Library.** The weaknesses are concentrated in the `viewstamp` *library*, precisely in the extensions the target depends on:
- **(1) Rolling upgrades do not exist.** Upgrades are flag-day and the wire handshake requires an exact version match. In a design where every node is a replica, that is a whole-cluster intent freeze, which is the most likely disqualifier.
- **(2) Reconfiguration is the least-proven path.** TigerBeetle, the reference implementation, never shipped reconfiguration, and `viewstamp`'s simulator does not yet cover reconfiguration or new storage failures (its issue #68).
- **(3) No dissemination beyond leader push.** Nothing is documented for learners, so the design has no answer yet for "thousands of learners".
- **(4) No linearizable-read path.** This turns out to matter less than expected: read-your-writes via VR §6.3.2 op-number tokens is clock-free, and linearizable needs are better served as leader-side commands.

The project is pre-0.1 with one visible maintainer. openraft is the mature fallback: joint consensus, ReadIndex, an opt-in lease read in 0.10, and 100+ contributors. It is weaker on storage faults and not Sans-I/O.

**Unexplored alternatives.** The most valuable alternatives are *patterns layered around* the consensus engine, not replacements for it:
- Cassandra TCM's "epoch in every message → catch up from any peer";
- pull-based, chained learner replication with hash-chain verification;
- range-negotiated wire versions plus a log-committed cluster-version gate, as in KIP-584/778, CockroachDB and TigerBeetle;
- Delos-style virtual consensus (seal-and-succeed log segments), which upgraded a production consensus protocol without downtime.

Leaderless protocols (EPaxos, Tempo, Accord), DAG BFT, Multi-Raft sharding and write-all local-read protocols (Hermes, CRAQ) were evaluated and lose for orchestration intent. The single-group write rate is not the constraint: Nomad scheduled about 1,500 containers/s across 6,000+ clients with 3 servers, and etcd sustains about 44–50k batched writes/s. The binding limits are state size, learner fan-out and WAN commit latency.

## Research Methodology

**Search Strategy**: Seeded by the prompt's candidate list (VR/VSR, `viewstamp`, TigerBeetle, openraft, EPaxos family, Flexible Paxos, DAG BFT, compartmentalization, shared logs, CometBFT, observers/learners, Plumtree, blockchain propagation, chain replication, lease protocols, upgrade schemes, protocol-aware recovery, Multi-Raft, WAN consensus). Actively extended beyond it: Cassandra CEP-21, Kafka KIPs 584/595/631/778/853, MongoDB pull-based consensus, OmniPaxos, LeaseGuard, Bodega, Consul Autopilot and anti-entropy, Spanner witnesses, Jepsen NATS, Antithesis Raft findings. Primary sources (papers, KIPs/CEPs, official docs, source repos) were fetched directly; secondary summaries were used only as corroboration.
**Source Selection**: Types: academic, official, technical_docs, industry | Reputation: medium-high minimum for load-bearing claims; out-of-list project primaries (jepsen.io, docs.tigerbeetle.com, docs.cockroachlabs.com, docs.cometbft.com, antithesis.com, docs.restate.dev, hashicorp.com) used only for their own project's behaviour and marked as such | Verification: independent cross-reference per claim; interpretation labelled explicitly
**Quality Standards**: Target 3 sources/claim (min 1 authoritative) | All major claims cross-referenced | Avg reputation ≈ 0.90
**Out of scope by instruction**: the current codebase (`crates/`) and `docs/whitepaper.md` were not read. Restate appears only as shared-log and node-role prior art; Temporal is not used.

**Relation to prior repo research**: `docs/research/gossip-protocols/serf-internals-and-overdrive-fit.md` (SWIM/Lifeguard/Foca) and `docs/research/scalability/corrosion-crsqlite-production-scale-evidence.md` (Corrosion fleet scale and failure modes) cover the *observation* half. This document does not repeat them; it covers the *intent* half and the point where the two meet.

## Target design under examination (restated)

- **Topology**: flat; no separate single-node vs HA mode; grows from one node to thousands, possibly across regions. Every node runs every component by default (gateway, web, control-plane, worker, telemetry, wasm, storage), with local or remote adapters per port.
- **Discovery**: a joining machine is given one peer's WireGuard endpoint and receives the peer list (peer exchange). Gossip handles discovery.
- **Intent** (workload specs, placement decisions, membership and admission, declared roles, the voter set, "node is down" decisions) is ordered by a replicated consensus log. The leading candidate is VR (VR Revisited / TigerBeetle VSR / `viewstamp`); the incumbent is Raft (openraft).
- **Working model**:
  - 3–5 voters auto-chosen from control-plane-eligible nodes across failure domains;
  - all other nodes are non-voting learners holding the full log;
  - reads are local and writes are forwarded to the leader;
  - the leader runs the scheduler and commits placement decisions;
  - workers learn assignments from their local replica.
- **Observation** (heartbeats, allocation status, health, service backends) is high-volume and per-node-owned, and stays off the log.
- **Trust model**: single operator, trusted nodes, crash faults only.
- **Hypothesis under test**: "gossip carries facts, the log carries decisions."

## A1. Consensus protocol for a flat, ever-growing cluster

### Finding A1.1: VR and Raft are the same protocol class. Their differences are election and recovery rules, so the choice between them rests on implementation properties

**Evidence**:
- Van Renesse, Schiper and Schneider compare Paxos, VR and Zab with refinement mappings: all three are "replication protocols that ensure high-availability in asynchronous environments with crash failures". They identify "key differences that have a significant impact on performance".
- Howard and Mortier find the key Paxos/Raft difference is that "Raft only allows servers with up-to-date logs to become leaders, whereas Paxos allows any server to be leader provided it then updates its log". Much of Raft's clarity "stems from effective presentation rather than fundamental algorithmic differences".
- A reading-group analysis notes that "Multi-Paxos and VR revisited recover the leader, while Raft ensures the new leader is up to date", and that "VR is the only major SMR protocol specified without the need for a durable infallible disk."

**Source**: [Van Renesse et al., "Vive la Différence", arXiv:1309.5671](https://arxiv.org/abs/1309.5671) (arxiv.org, high); [Howard & Mortier, "Paxos vs Raft", arXiv:2004.05074](https://arxiv.org/abs/2004.05074) (arxiv.org, high). Both accessed 2026-09-25.
**Confidence**: High
**Verification**: [Charapko, VR Revisited reading group](https://charap.co/reading-group-viewstamped-replication-revisited/) (academic blog, medium-high); [VR Revisited, MIT-CSAIL-TR-2012-021](https://dspace.mit.edu/handle/1721.1/71763) (MIT, high)
**Analysis**: "VR vs Raft" is not a decision about *protocol class*. Both are leader-based, majority-quorum, crash-fault state-machine replication. The decision is about *implementation*: Sans-I/O purity, the storage-fault model, simulation, reconfiguration, read paths, upgrades and maturity. The storage-fault argument favours VR/VSR (A5). The ecosystem-maturity argument favours openraft (Finding A1.3).

### Finding A1.2: TigerBeetle's VSR is production-hardened and Jepsen-validated, but it has no online membership reconfiguration. The target design's automatic voter churn therefore sits outside the reference implementation's proven envelope

**Evidence**: TigerBeetle's internals document lists "Protocol: Reconfiguration — TODO (Unimplemented)". It uses flexible quorums: "For `6` replicas, the replication quorum is only `3` ... at least `4` replicas are needed" for view changes. Jepsen (report dated 2025-06-06) concluded: "As of 0.16.30, TigerBeetle appeared to meet its promise of Strong Serializability. As of 0.16.45, TigerBeetle had addressed every issue we found, with the exception of indefinite retries." `viewstamp` names "TigerBeetle's src/vsr/replica.zig" as "the correctness reference for the protocol logic" and adds `SingleChange` reconfiguration on top.
**Source**: [TigerBeetle vsr.md](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/vsr.md) (github.com, medium-high); [Jepsen: TigerBeetle 0.16.11](https://jepsen.io/analyses/tigerbeetle-0.16.11) (out-of-list primary, medium-high). Both accessed 2026-09-25.
**Confidence**: High (for TigerBeetle's own state); Medium (for the inference about viewstamp)
**Verification**: [TigerBeetle ARCHITECTURE.md](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/ARCHITECTURE.md) (no reconfiguration content, fixed 6-replica recommendation); viewstamp search snippet naming replica.zig as reference ([docs.rs/viewstamp](https://docs.rs/viewstamp/latest/viewstamp/))
**Analysis**: The target design needs reconfiguration of the voter set to be routine and automatic: nodes join, fail, move failure domain and get promoted. In TigerBeetle, the codebase that gives VSR its credibility, that path does not exist. `viewstamp`'s `AddLearner/PromoteLearner/DemoteVoter/RemoveLearner` is therefore the *least* battle-tested part of the stack, and its own issue #68 says VOPR does not yet cover reconfiguration failures (Finding V3). By contrast, Raft membership change (single-server change, or joint consensus in etcd and openraft) has years of production use. This is the single strongest protocol-level argument against adopting `viewstamp` as-is.

### Finding A1.3: openraft is the mature incumbent: 0.9.x stable, many contributors and adopters, learners and joint consensus, ReadIndex reads, and an opt-in lease read in 0.10

**Evidence**:
- **Status**: the openraft README describes 0.10.0-alpha.35 on the main branch, while docs.rs serves 0.9.25 (dated 2026-07-28) as latest stable. "OpenRaft API is not stable yet. Before `1.0.0`, an upgrade may contain incompatible changes."
- **Features and adoption**: `add_learner()`, "Extended joint membership changes", `ensure_linearizable()`, and batching "reaching millions of writes/sec". About 2,100 stars, 3,077 commits, "100+ contributors" and "at least 13 known adopters", including Databend.
- **Reads**: v0.9.0 changelog: "add `Raft::ensure_linearizable()` to ensure linearizable read"; v0.8.4 added "leader lease" (used internally). A third-party tracking issue states that 0.10 adds `ReadPolicy::LeaseRead`, "which serves linearizable reads inside a quorum-acked time window without a per-read network round", with ReadIndex remaining the default.

**Source**: [databendlabs/openraft](https://github.com/databendlabs/openraft) (github.com, medium-high); [openraft change-log](https://github.com/databendlabs/openraft/blob/main/change-log.md); [docs.rs/openraft](https://docs.rs/openraft/latest/openraft/docs/faq/index.html) (docs.rs, high). All accessed 2026-09-25.
**Confidence**: Medium-High (LeaseRead-in-0.10 is corroborated only by a downstream issue, [vibesql #5389](https://github.com/rjwalters/vibesql/issues/5389), and search snippets; the docs.rs 0.10 page returned 404)
**Verification**: [drmingdrmer/consensus-essence: raft-read-index](https://github.com/drmingdrmer/consensus-essence/blob/main/src/list/raft-read-index/raft-read-index.md) (openraft author's notes: ReadIndex = `max(CommitIndex, NoopIndex)`)
**Analysis**: openraft is not Sans-I/O in the `viewstamp` sense: it owns an async runtime, while storage and network are traits. It also has no first-class storage-fault model. It does have what the target design lacks: proven membership change, a documented linearizable-read path, and a community (bus factor far above 1). Its own pre-1.0 caveat means neither candidate yet offers a stable wire/API contract for rolling upgrades (A4).

### Finding A1.4: Leaderless protocols (EPaxos, Atlas, Tempo, Accord) buy WAN latency and remove the leader bottleneck, at a high correctness and complexity cost. Only Accord is near broad production, and it depends on a separate ordered metadata log

**Evidence**:
- **EPaxos Revisited (NSDI'21)**: EPaxos "achieves optimal median commit latency in a WAN" but has "significantly worse tail latency ... up to 4x worse" under high-conflict workloads.
- **Ryabinin, Gotsman and Sutra (OPODIS'25)**: EPaxos "is very complex, ambiguously specified and suffers from nontrivial bugs". They present a corrected EPaxos*.
- **Tempo (EuroSys'21)**: it orders by timestamp stability, improving throughput "by 1.8-5.1x" and lowering tail latency "by an order of magnitude" versus prior leaderless protocols.
- **Accord (CEP-15)** targets "one wide-area round-trip under normal conditions". It ships in Cassandra 6.0, which is "currently published only as alpha releases".

**Source**: [Tollman, Park, Ousterhout, "EPaxos Revisited", NSDI'21](https://www.usenix.org/conference/nsdi21/presentation/tollman) (usenix.org, high); [Ryabinin et al., "Fixing and Simplifying Egalitarian Paxos", arXiv:2511.02743](https://arxiv.org/abs/2511.02743) (arxiv.org, high); [Enes et al., Tempo, arXiv:2104.01142](https://arxiv.org/abs/2104.01142) (arxiv.org, high); [CEP-15](https://cwiki.apache.org/confluence/display/CASSANDRA/CEP-15:+General+Purpose+Transactions) (apache.org, high). All accessed 2026-09-25.
**Confidence**: High (for protocol properties); Medium (Accord production status rests on vendor commentary, [Instaclustr](https://www.instaclustr.com/blog/apache-cassandra-6-accord-transactions-what-you-need-to-know/))
**Verification**: [Atlas, EuroSys'20](https://software.imdea.org/~gotsman/papers/atlas-eurosys20.pdf) (academic); [Enes et al. Tempo in ACM DL](https://dl.acm.org/doi/10.1145/3447786.3456236)
**Analysis** (interpretation): Leaderless protocols win when commands mostly *commute*. Orchestration intent does not commute well. Placement decisions contend for the same node capacity, and the design deliberately runs the scheduler at the leader, a single decision point that needs a total order over a global view. A leaderless log would still need a serialisation point for scheduling, which removes the benefit. Cassandra is adding leaderless Accord for *data* and, in the same generation, moving cluster *metadata* onto a single ordered Paxos log (CEP-21, see the Hypothesis section). A leaderless data path therefore does not remove the need for an ordered metadata log. The precise coupling of Accord to TCM epochs was not verified (see Knowledge Gap 8). Verdict: **loses** to the current direction for intent. A leaderless design could reappear only for per-region data that does commute.

### Finding A1.5: Flexible Paxos separates the replication quorum from the election quorum. VSR already uses this, and it is the cheapest way to tune the write-latency versus election-safety trade-off

**Evidence**: Howard, Malkhi and Spiegelman show that "each phase of Paxos may use non-intersecting quorums" and that "majority quorums are not necessary as intersection is required only across phases" (OPODIS 2016). TigerBeetle applies this: for 6 replicas the replication quorum is 3 and the view-change quorum is 4.
**Source**: [Howard, Malkhi, Spiegelman, "Flexible Paxos", arXiv:1608.06696](https://arxiv.org/abs/1608.06696) (arxiv.org, high), accessed 2026-09-25
**Confidence**: High
**Verification**: [DROPS / LIPIcs OPODIS 2016](https://drops.dagstuhl.de/entities/document/10.4230/LIPIcs.OPODIS.2016.25) (academic, high); [TigerBeetle vsr.md](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/vsr.md) (production use); [Jepsen TigerBeetle](https://jepsen.io/analyses/tigerbeetle-0.16.11) ("flexible quorums")
**Analysis**: For a multi-region voter set (A7), flexible quorums let commits wait on the *nearest* replication quorum while election quorums enlarge to preserve intersection. The cost is availability of elections: more replicas must be alive to change leader. `viewstamp`'s documentation does not mention flexible quorums. Whether it inherits TigerBeetle's 3/6 behaviour is a knowledge gap.

### Finding A1.6: DAG-based BFT protocols (Narwhal/Bullshark/Mysticeti/Shoal) show that *separating dissemination from ordering* scales throughput. That separation carries over to crash-fault settings; the DAG and BFT machinery does not

**Evidence**:
- Narwhal and Tusk (EuroSys'22) propose "separating the task of reliable transaction dissemination from transaction ordering". The DAG plus Tusk/Bullshark "sequences 100k+ transactions per second ... in a geo-replicated environment while keeping latency below 3s".
- Mysticeti (NDSS 2025) and Shoal (arXiv:2306.03058) continue the line with lower latency.
- The same separation appears in crash-fault shared logs. Corfu "had the idea to divorce ordering and replication", and Scalog "can totally order up to 52 million records per second".

**Source**: [Narwhal and Tusk, EuroSys'22 (ACM DL)](https://dl.acm.org/doi/10.1145/3492321.3519594) (acm.org, high); [Shoal, arXiv:2306.03058](https://arxiv.org/pdf/2306.03058) (arxiv.org, high); [Scalog, NSDI'20](https://www.usenix.org/conference/nsdi20/presentation/ding) (usenix.org, high). All accessed 2026-09-25.
**Confidence**: Medium-High
**Verification**: [Decentralized Thoughts, "DAG Meets BFT"](https://decentralizedthoughts.github.io/2022-06-28-DAG-meets-BFT/) (academic blog, medium-high)
**Analysis** (interpretation): The target does not need 100k-TPS intent throughput (A6), so a DAG protocol is over-engineered for it and brings Byzantine assumptions the trust model does not need. The *transferable principle* is the same one as Findings A2.2–A2.4: order small references on the log, and disseminate bulk payloads (workload specs, images) through a separate, content-addressed path. Verdict: DAG protocols **lose** as a consensus choice. Their decoupling principle **wins** as a design rule.

### Finding A1.7: Virtual or shared logs (Delos VirtualLog, Corfu/Scalog, Restate Bifrost) separate *reconfiguration* from *ordering*. Delos changed its consensus protocol in production without downtime

**Evidence**:
- **Delos (OSDI'20)**: "Consensus-based replicated systems are complex, monolithic, and difficult to upgrade once deployed." Delos "splits the logic of consensus into the VirtualLog, a generic and reusable reconfiguration layer; and pluggable ordering protocols called Loglets". Reported: "Delos reached production within 8 months, and 4 months later upgraded its consensus protocol without downtime for a 10X latency improvement", while processing "over 1.8 billion transactions per day".
- **Restate Bifrost** (cited strictly as shared-log prior art) uses "a segmented virtual log: the active segment receives appends; reconfiguration seals the active segment and atomically publishes a new segment as the head". The metadata store it depends on is "a built-in Raft-based metadata store ... hosted on all nodes running the `metadata-server` role".

**Source**: [Balakrishnan et al., "Virtual Consensus in Delos", OSDI'20](https://www.usenix.org/conference/osdi20/presentation/balakrishnan) (usenix.org, high); [Restate architecture](https://docs.restate.dev/references/architecture) (out-of-list primary, medium-high). Both accessed 2026-09-25.
**Confidence**: High (for Delos); Medium (for Bifrost details)
**Verification**: [Delos paper PDF](https://www.usenix.org/system/files/osdi20-balakrishnan.pdf); [Jack Vanlightly, "An Introduction to Virtual Consensus in Delos"](https://jack-vanlightly.com/blog/2025/2/5/an-introduction-to-virtual-consensus-in-delos) (practitioner, medium); [Restate issue #1830](https://github.com/restatedev/restate/issues/1830)
**Analysis**: This is the most important *unexplored* alternative for the target. It is not a replacement for VSR or Raft; it is a **layer above them**. With a small VirtualLog-style shim (an ordered chain of segments, each sealed and then succeeded by a new segment), the consensus engine becomes swappable. A flag-day-only engine could then be upgraded by *sealing* segment *n* on the old engine or version and starting segment *n+1* on the new one (A4). It also creates a natural seam for per-region segments (A6/A7). The cost is a second metadata tier: the segment chain must itself be stored somewhere consistent. Delos used ZooKeeper-backed metadata; Restate uses Raft metadata servers.

### Finding A1.8: Compartmentalization breaks the leader bottleneck without changing protocol class. Proxy leaders and read scaling address throughput, not dissemination

**Evidence**: Compartmentalization decouples "individual bottlenecks into distinct components" and scales them independently. It raised MultiPaxos throughput "by 6x on a write-only workload and 16x on a mixed read-write workload", with the claim that practitioners can "apply compartmentalization to their protocols incrementally without having to adopt a completely new protocol".
**Source**: [Whittaker et al., "Scaling Replicated State Machines with Compartmentalization", PVLDB 14(11) 2021](https://arxiv.org/abs/2012.15762) (arxiv.org/vldb.org, high), accessed 2026-09-25
**Confidence**: Medium-High (one academic source, peer reviewed)
**Verification**: [PVLDB PDF](https://vldb.org/pvldb/vol14/p2203-whittaker.pdf); [dblp record](https://dblp.org/rec/journals/pvldb/WhittakerACDGHH21.html)
**Analysis**: "Proxy leaders" is the formal version of the learner-relay idea in A2.2: the leader hands fan-out to stateless or semi-stateless relays. That makes compartmentalization a design vocabulary for the target rather than an alternative protocol.

### Finding A1.9: OmniPaxos is a Rust replicated-log library with no built-in I/O. It was designed for partial connectivity, a failure mode Raft-based systems have hit in production

**Evidence**:
- **OmniPaxos**: "an in-development replicated log library implemented in Rust", with "User-controlled network and storage implementations (no built-in I/O)", flexible quorums, snapshots and reconfiguration. The repository has 223 stars and 151 commits, is at version 0.2.3, and lists no production users. The EuroSys'23 paper claims progress "with one QC (quorum-connected) server, while other protocols require at least a majority", achieved by separating leader election from log replication, and "decoupling reconfiguration from log replication".
- **Cloudflare, 2020-11-02**: a partial switch failure "left etcd unavailable as it was unable to establish a stable leader", causing a 6h33m API and dashboard outage.

**Source**: [haraldng/omnipaxos](https://github.com/haraldng/omnipaxos) (github.com, medium-high); [Ng, Haridi, Carbone, EuroSys'23 (ACM DL)](https://dl.acm.org/doi/abs/10.1145/3552326.3587441) (acm.org, high); [Cloudflare, "A Byzantine failure in the real world"](https://blog.cloudflare.com/a-byzantine-failure-in-the-real-world/) (cloudflare.com, high). All accessed 2026-09-25.
**Confidence**: Medium-High
**Verification**: ["Examining Raft's behaviour during partial network failures", HAOC'21 (ACM DL)](https://dl.acm.org/doi/abs/10.1145/3447851.3458739) (acm.org, high); [clockworklabs/omnipaxos fork](https://github.com/clockworklabs/omnipaxos) (a company fork, usage undocumented)
**Analysis**: A WireGuard full mesh across regions is exactly where *partial* connectivity (A reaches B, B reaches C, A cannot reach C) is plausible. VR's round-robin view change and Raft (without PreVote and CheckQuorum) are both vulnerable to leader churn in that topology. OmniPaxos is a second Sans-I/O-style Rust candidate with this property built in. It is academic and pre-production, so it **loses** to openraft on maturity. It **beats** both viewstamp and openraft on the documented partial-connectivity liveness property. At a minimum, its quorum-connected election rule is worth porting as an idea.

### Finding A1.10: What carries over from CometBFT/HotStuff validator-set models to crash-fault tolerance: delayed, pipelined activation of membership changes, and commit certificates that full nodes can verify

**Evidence**: In CometBFT, validator "Updates returned after processing the block at height H will only take effect at block H+2". `validator_updates` "triggered by block H, affect validation for blocks H+1, H+2, and H+3", so full nodes must track them to validate subsequent blocks.
**Source**: [CometBFT ABCI++ application requirements (v0.38)](https://docs.cometbft.com/v0.38/spec/abci/abci++_app_requirements) (out-of-list primary, medium-high), accessed 2026-09-25
**Confidence**: Medium (single primary; the carry-over analysis is interpretation)
**Verification**: [CometBFT v0.38.0 CHANGELOG (github.com)](https://github.com/cometbft/cometbft/blob/v0.38.0/CHANGELOG.md); [Cosmos docs mirror](https://docs.cosmos.network/cometbft/latest/spec/abci/Requirements-for-the-Application)
**Analysis** (interpretation):
- **Carries over.** A fixed activation delay makes a voter-set change *a committed decision with a known effective op*. Every learner can then compute "who were the voters at op N" deterministically from the log, which is exactly what a learner needs to judge a relayed commit certificate (A2.5).
- **Does not carry over.** Signatures, stake weighting, 2f+1 quorums and accountable-safety slashing are all Byzantine machinery. Under the target's crash-fault model, a hash chain plus the voter set recorded in the log is enough.

## A2. Bounded voters, many learners, and disseminating committed entries

### Finding A2.1: A small voter set plus non-voting full replicas is the standard production answer. Four independent systems ship it, and none of them intends the leader to push to thousands of learners

**Evidence**:
- **ZooKeeper**: "Observers are non-voting members of an ensemble which only hear the results of votes, not the agreement protocol that leads up to them." The rationale: "As we add more voting members, the write performance drops."
- **etcd**: learners "Cannot ... Vote or count toward quorum" and cannot "Issue Read Index requests"; they can "Serve serializable reads". "etcd limits the total number of learners that a cluster can have" to avoid overloading the leader; v3.4 allows one.
- **Kafka KRaft (KIP-595)**: observers "cannot vote or become leaders but can still fetch from the leader"; they are "only responsible for discovering the leader and replicating the log".
- **TigerBeetle**: standbys are bounded by `standbys_max` and excluded from quorum (`replica_index >= replica_count`). The primary sends prepares "to every other replica and standby, in parallel".

**Source**: [ZooKeeper Observers](https://zookeeper.apache.org/doc/current/zookeeperObservers.html) (apache.org, high); [etcd learner design](https://etcd.io/docs/v3.6/learning/design-learner/) (etcd.io, high); [KIP-595](https://cwiki.apache.org/confluence/display/KAFKA/KIP-595%3A+A+Raft+Protocol+for+the+Metadata+Quorum) (apache.org, high); [TigerBeetle replica.zig](https://github.com/tigerbeetle/tigerbeetle/blob/main/src/vsr/replica.zig) (github.com, medium-high). All accessed 2026-09-25.
**Confidence**: High
**Verification**: Four independent codebases and organisations; [al8n/viewstamp README](https://github.com/al8n/viewstamp) adopts the same learner/standby concept.
**Analysis**: The target design's model (3–5 voters, everyone else a learner) is well trodden *in shape*. It is not well trodden *in scale*. etcd caps learners at one (v3.4), and TigerBeetle bounds standbys by a compile-time constant (`standbys_max`, value not verified). Only KRaft (every broker is an observer of the metadata log) and ZooKeeper observer masters (below) are designed for hundreds of non-voters. None of the four documents a deployment with thousands of full-log learners. Whether the working model holds past about 100 learners is therefore an open question, not something prior art establishes.

### Finding A2.2: Pull-based and chained replication removes the leader fan-out bottleneck *inside* the consensus protocol. ZooKeeper, MongoDB and KRaft each ship a form of it

**Evidence**:
- **KIP-595 on pull-based fetching**: it "makes it suited to large numbers of observer replicas since the leader only needs to track the status of replicas in the quorum."
- **ZooKeeper `observerMasterPort`**: it instructs "Observers to connect to peers (Leaders and Followers) ... and instruct[s] Followers to create an ObserverMaster thread to listen and serve on that port". The documented benefit is reduced leader load and "hundreds of Observers".
- **MongoDB (NSDI'21)**: it uses "a unique pull-based data synchronization model: a replica pulls new data from another replica". Unlike Raft, where "new data can only be pushed from the primary", transmission "can be initiated by any replica and can happen between any two replicas". MongoDB's docs note that secondaries "may automatically change their sync targets to secondary members based on changes in the ping time".

**Source**: [KIP-595](https://cwiki.apache.org/confluence/display/KAFKA/KIP-595%3A+A+Raft+Protocol+for+the+Metadata+Quorum); [ZooKeeper Observers](https://zookeeper.apache.org/doc/current/zookeeperObservers.html); [Zhou et al., "Fault-Tolerant Replication with Pull-Based Consensus in MongoDB", NSDI'21](https://www.usenix.org/conference/nsdi21/presentation/zhou) (usenix.org, high). All accessed 2026-09-25.
**Confidence**: High
**Verification**: Three independent production systems; [MongoDB sync-target docs](https://www.mongodb.com/docs/v8.0/tutorial/configure-replica-set-secondary-sync-target/) (out-of-list primary) corroborate chaining.
**Analysis**: This is the most directly applicable answer to "thousands of learners without a primary fan-out bottleneck". The leader feeds only the voters plus a handful of relay learners, and every other learner *pulls* committed entries from a nearby peer. Pull suits a learner because it knows its own `commit_number`/offset and can resume from any source. The MongoDB paper shows that combining pull with chaining is compatible with Raft-style safety. For voters, which count toward commit, the sync-source choice must not break the commit rule. For learners, which never count, the rule is trivially safe. `viewstamp` documents neither pull fetch nor learner-to-learner relay (Finding V2), so this would be new work on top of it.

### Finding A2.3: Epidemic broadcast trees (Plumtree over HyParView) give tree-efficient, self-healing dissemination, and there is a Sans-I/O Rust implementation. The protocol is best-effort, so it must be paired with log-position repair

**Evidence**: Plumtree "is a push-lazy-push broadcast protocol that combines the efficiency of tree-based multicast with the resilience of epidemic gossip". Payloads flow eagerly along a spanning tree, and lazy peers receive only `IHAVE` announcements. On a timeout, a node sends `GRAFT` to a lazy neighbour and promotes that link. HyParView is the companion membership/peer-sampling protocol (DSN 2007). `iroh-gossip` 0.101.0 (released 2026-09-07) implements HyParView + PlumTree with "a `proto` module functioning as an IO-free state machine" and an optional `net` module.
**Source**: [Leitão, Pereira, Rodrigues, "Epidemic Broadcast Trees", SRDS 2007](https://www.dpss.inesc-id.pt/~ler/reports/srds07.pdf) (academic, high); [iroh-gossip on docs.rs](https://docs.rs/iroh-gossip/latest/iroh_gossip/) (docs.rs, high). Both accessed 2026-09-25.
**Confidence**: Medium-High (protocol: high; production scale of iroh-gossip: not documented)
**Verification**: [Semantic Scholar record](https://www.semanticscholar.org/paper/Epidemic-Broadcast-Trees-Leitao-Pereira/528a0945acb242d4f36c361e10f9e612c0b631b9) for the paper; prior repo research on SWIM/Foca (`docs/research/gossip-protocols/serf-internals-and-overdrive-fit.md`) covers the membership side and is not repeated here.
**Analysis**: Plumtree is a good *push* accelerator for committed entries, bringing latency near the tree depth rather than gossip rounds. It guarantees neither order nor completeness under churn. The sound composition is: Plumtree pushes committed entries tagged `(op, checksum)`, and a learner that detects a gap *pulls* the missing range (Finding A2.2) from any peer. It then verifies the range against the hash chain (Finding A2.5). The log stays the single source of order and the tree is only a transport. Its Sans-I/O `proto` module fits the target's Sans-I/O discipline.

### Finding A2.4: Blockchain dissemination techniques target large blocks under packet loss. Erasure-coded trees (Solana Turbine) and compact blocks (Bitcoin BIP-152) transfer only partially to small intent entries

**Evidence**:
- **Turbine**: its "primary objective is reducing leader egress requirements by distributing retransmission responsibility". Layer *n+1* has `fanout ×` layer-*n* nodes, and the tree is regenerated per shred from "a seed derived from the slot leader id, slot, shred index, and shred type". Multihop loss "compounds ... exponentially", so FEC is used: `32:32` gives ">99% success" under 15% loss.
- **BIP-152**: compact blocks replace each transaction the peer probably has with "a fast per-peer 6-byte non-cryptographic hash". This relays a block in about 15 KB, a reduction of about 98%.

**Source**: [Solana Turbine doc](https://github.com/solana-labs/solana/blob/master/docs/src/consensus/turbine-block-propagation.md) (github.com, medium-high); [BIP-152](https://github.com/bitcoin/bips/blob/master/bip-0152.mediawiki) (github.com, medium-high). Both accessed 2026-09-25.
**Confidence**: Medium (two primary sources; transferability to orchestration is interpretation)
**Verification**: [Agave Turbine doc](https://docs.anza.xyz/consensus/turbine-block-propagation) (out-of-list primary, same content lineage); [Bitcoin Optech: compact block relay](https://bitcoinops.org/en/topics/compact-block-relay/)
**Analysis** (interpretation): Orchestration intent entries are small (KB-scale specs and placement decisions), so FEC-over-UDP buys little. Two ideas do transfer:
1. **Deterministic, per-entry-seeded relay trees.** Every node computes its parent and children from membership plus the entry id, with no coordination. This is a cheaper alternative to Plumtree's adaptive tree when membership is known from the log.
2. **Compact-block-style references.** If workload specs are content-addressed and gossiped or fetched separately, the log entry can carry only the hash. Order stays on the log while bulk bytes travel off-log.

### Finding A2.5: Hash-chained log entries let a learner accept an entry from *any* relay and verify it locally. TigerBeetle already treats "from disk" and "from a peer" identically

**Evidence**: In TigerBeetle, "The ground state of the system is an immutable, hash-chained, append-only log of prepares". The primary assigns each prepare "the next sequence number and add[s] a checksum 'pointer' to the previous log entry". "If you know the checksum, then, on receiving any data, you get strong guarantees that this is exactly the data you were looking for". Headers "are repaired backwards (from the head) by hash-chaining". "TigerBeetle doesn't make a distinction whether checksummed data comes from a local disk or another replica". `viewstamp` checkpoints are "a content-addressed block DAG".
**Source**: [TigerBeetle ARCHITECTURE.md](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/ARCHITECTURE.md) (github.com, medium-high), accessed 2026-09-25
**Confidence**: Medium-High
**Verification**: [TigerBeetle vsr.md](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/vsr.md) (repair via `request_headers`/`headers`); [al8n/viewstamp README](https://github.com/al8n/viewstamp) (content-addressed checkpoints)
**Analysis**: Under the target's crash-fault-only trust model, relayed-entry verification is about *corruption and misrouting*, not malice. A hash chain rooted in a commit certificate from the voters (the `commit_number` plus head checksum carried in a primary heartbeat) is enough. A learner that learns the head checksum from any voter can accept every earlier entry from any peer and detect a corrupted relay. Merkle proofs add value only for *partial* verification, for example when a learner fetches a single checkpoint block. The content-addressed checkpoint DAG already provides that.

### Finding A2.6: Chain replication and CRAQ are strong for throughput and reads, but they are replication schemes under an external configuration master. As a dissemination topology they carry per-hop latency and a single-failure stall hazard

**Evidence**:
- Chain replication (van Renesse & Schneider, OSDI'04) coordinates "clusters of fail-stop storage servers". The paper relies on a separate master service for chain membership.
- CRAQ (Terrace & Freedman, USENIX ATC'09) "is able to read from all nodes in the chain", distributing load and scaling "linearly with chain size without increasing consistency coordination".
- Jepsen found the chain-shaped hazard in practice. TigerBeetle's ring replication meant "the failure of any one of the next _f_ replicas in the ring will prevent commit entirely", and TigerBeetle added "bidirectional ring replication and dynamic topology adaptation" by 0.16.43. TigerBeetle PR #2635 added experimental star and closed-loop replication because under ring topology non-quorum replicas could fall "behind and enter[ing] state sync".

**Source**: [Chain Replication, OSDI'04](https://www.usenix.org/conference/osdi-04/chain-replication-supporting-high-throughput-and-availability) (usenix.org, high); [CRAQ, USENIX ATC'09](https://www.usenix.org/conference/usenix-09/object-storage-craq-high-throughput-chain-replication-read-mostly-workloads) (usenix.org, high); [Jepsen: TigerBeetle 0.16.11](https://jepsen.io/analyses/tigerbeetle-0.16.11) (out-of-list primary, medium-high); [TigerBeetle PR #2635](https://github.com/tigerbeetle/tigerbeetle/pull/2635). All accessed 2026-09-25.
**Confidence**: High
**Verification**: Four sources, two academic and two independent practitioners (Jepsen, TigerBeetle)
**Analysis**: The lesson for learner fan-out is to *avoid long, fixed relay chains*. A relay tree of depth 2–3 with a small fixed fan-out, pull-on-gap repair and parent re-selection bounds both latency and blast radius. The Turbine and ZooKeeper observer-master numbers imply that hundreds to thousands of nodes need only 2–3 hops.

## A3. Reads from any node

### Finding A3.1: VR Revisited specifies both read paths the target needs: primary leases for linearizable reads, and client-token reads at backups for read-your-writes. Only the second is clock-free

**Evidence**:
- **§6.3.1 Reads at the Primary**: "The primary process[es] reads unilaterally only if it holds valid leases from f other replicas, and a new view will start only after leases at f+1 participants in the view change protocol expire."
- **§6.3.2 Reads at Backups**: "Leases aren't needed if it is acceptable for the result of a read to be based on stale information." For causality: "When the client does a write, the primary returns the op-number that it assigned to that request, and the client stores this in last-request-number. When the client sends a read request it includes this number, and the replica responds only if it has executed operations at least this far."
- **§6.1 Witnesses**: "The group of 2f+1 replicas includes f+1 active replicas ... and f witnesses, which do not [store state]. ... Witnesses are needed for view changes and recovery."

**Source**: [Liskov & Cowling, "Viewstamped Replication Revisited", MIT-CSAIL-TR-2012-021](https://dspace.mit.edu/handle/1721.1/71763) (MIT, high). Text quoted from an HTML mirror, [ying-zhang.cn/dist/2012-vr.html](http://ying-zhang.cn/dist/2012-vr.html), because the MIT PDF endpoints refused automated fetches. Accessed 2026-09-25.
**Confidence**: High for the paper's text (primary source via mirror; the lease sentence is independently reproduced in search-indexed summaries)
**Verification**: [Charapko reading group](https://charap.co/reading-group-viewstamped-replication-revisited/) and [freeCodeCamp summary](https://www.freecodecamp.org/news/viewstamped-replication-revisited-a-summary-144ac94bd16f/) corroborate the surrounding protocol (client table, request numbers)
**Analysis**: §6.3.2 fits the target's "reads are local" rule almost exactly, with one change: the replica that "has executed operations at least this far" can be a *learner* as well as a backup. The token a client (or the operator CLI) carries is simply the op-number of its last write. This needs no clock and no extra protocol messages. It gives read-your-writes and monotonic reads per session, not linearizability. §6.3.1 leases give linearizability only *at the primary*, and they couple the view change to lease expiry, which is a protocol-core change (see A3.5).

### Finding A3.2: In Raft, linearizable reads away from the leader cost one round trip to the leader (ReadIndex). Leases are the zero-round-trip alternative, and correct lease implementations are recent and rare

**Evidence**:
- **ReadIndex steps**: confirm leadership with "a heartbeat ... to the quorum", then "wait[s] for the StateMachine to apply entries up to the ReadIndex". openraft sets ReadIndex to "`max(CommitIndex, NoopIndex)`".
- **etcd learners** cannot "Issue Read Index requests", only "serializable reads".
- **LeaseGuard (SIGMOD'26)**: "Prior lease protocols are vaguely specified and hurt availability, so most Raft systems implement them incorrectly or not at all." LeaseGuard "reduces the overhead of consistent reads from one to zero network roundtrips", and "the new leader instantly allows 99% of reads to succeed", whereas traditional leases "ban all reads on a new leader while it waits for a lease".

**Source**: [consensus-essence: raft-read-index](https://github.com/drmingdrmer/consensus-essence/blob/main/src/list/raft-read-index/raft-read-index.md) (github.com, medium-high); [etcd learner design](https://etcd.io/docs/v3.6/learning/design-learner/) (etcd.io, high); [Davis, Demirbas, Deng, "LeaseGuard: Raft Leases Done Right", arXiv:2512.15659](https://arxiv.org/abs/2512.15659) (arxiv.org, high; SIGMOD'26, [ACM DOI](https://doi.org/10.1145/3786663)). All accessed 2026-09-25.
**Confidence**: High
**Verification**: [openraft README](https://github.com/databendlabs/openraft) (`ensure_linearizable()`); [Howard & Mortier](https://arxiv.org/abs/2004.05074) (Raft/Paxos read discussion)
**Analysis**: For a learner, "linearizable local read" means: ask the leader for a read index (one round trip, possibly cross-region), then wait for the local apply index to reach it. The read is served locally but not *latency*-local. The LeaseGuard result is a warning for the target: a home-grown lease on top of `viewstamp` is exactly the "implemented incorrectly" risk.

### Finding A3.3: Linearizable reads *locally at non-leader replicas* are possible (Paxos Quorum Leases, Bodega roster leases, Hermes, CRAQ), but every such scheme makes writes wait on the read-serving replicas. None scales to thousands of lease holders

**Evidence**:
- **Paxos Quorum Leases (SoCC'14)**: "lease-granters must be a majority of nodes ... The lease-holders could be any number of nodes"; "Whenever the leader modifies some data, it promises to wait for acknowledgment from that data's lease-holders before it acknowledges the write". The published claim is that quorum leases reduce read latency "by two orders of magnitude in wide-area scenarios".
- **Bodega (2025)**: "the first consensus protocol that serves linearizable reads locally from any desired node, regardless of interfering writes", via all-to-all "roster leases", imposing "no special requirements on writes other than a responder-covering quorum".
- **Hermes (ASPLOS'20)**: membership-based protocols "require *all operational* nodes in the replica group to acknowledge each write (i.e., read-one/write-all protocols)". They depend on a "reliable membership (RM) ... typically based on Vertical Paxos ... guarded by leases". Hermes "targets intra-datacenter in-memory datastores with a replication degree typical of today's deployments (3-7 replicas)".
- **CRAQ**: can read at any chain node, but for a dirty object "the tail must be queried".

**Source**: [Moraru, Andersen, Kaminsky, "Paxos Quorum Leases", SoCC'14](https://dl.acm.org/doi/10.1145/2670979.2671001) (acm.org, high; mechanism quoted from [Davis's review](https://emptysqua.re/blog/review-paxos-quorum-leases/), medium-high); [Hu, Arpaci-Dusseau, Arpaci-Dusseau, "Bodega", arXiv:2509.07158](https://arxiv.org/abs/2509.07158) (arxiv.org, high); [Katsarakis et al., "Hermes", ASPLOS'20, arXiv:2001.09804](https://arxiv.org/abs/2001.09804) (arxiv.org, high); [CRAQ, USENIX ATC'09](https://www.usenix.org/conference/usenix-09/object-storage-craq-high-throughput-chain-replication-read-mostly-workloads) (usenix.org, high). All accessed 2026-09-25.
**Confidence**: High
**Verification**: Four independent research groups converge on the same trade-off (read-local ⇒ writes wait on the readers)
**Analysis**: These schemes make sense for a *bounded* set of read-serving replicas: for example one learner per region holding a quorum lease, so that regional linearizable reads avoid the WAN. They do not make sense for "every node reads linearizably from its own replica": thousands of lease holders would put thousands of acknowledgements on every intent write. Bodega is the newest and most general of the four, but it is a 2025 research prototype with no production evidence.

### Finding A3.4: Consistency each orchestrator read actually needs, and which mechanism fits it (analysis)

**Evidence**: This is synthesis from A3.1–A3.3, with no new source. For each orchestrator read class, the needed consistency and the cheapest sufficient mechanism:
- **Worker learns its assignments.** Needs monotonic, gap-free, eventually current reads. Mechanism: local learner apply index; ordering comes from the log.
- **Operator CLI reads its own write** (`deploy` then `status`). Needs read-your-writes. Mechanism: VR §6.3.2 token = op-number of the write, served at any replica once `applied ≥ token`.
- **Admission/CAS-shaped decisions** ("is this name free?", "is node X still a voter?"). Need linearizability. Mechanism: executed *at the leader as a committed command*, where the scheduler already runs.
- **Regional dashboards and UIs.** Need bounded staleness. Mechanism: local learner, exposing `applied_op` and last-heartbeat age.

**Source**: derived from [VR Revisited §6.3](http://ying-zhang.cn/dist/2012-vr.html), [etcd learner design](https://etcd.io/docs/v3.6/learning/design-learner/), [LeaseGuard](https://arxiv.org/abs/2512.15659)
**Confidence**: Medium (interpretation)
**Verification**: Consistent with etcd's split between serializable and linearizable reads, and with Kafka KRaft brokers serving from their local metadata image ([KIP-595](https://cwiki.apache.org/confluence/display/KAFKA/KIP-595%3A+A+Raft+Protocol+for+the+Metadata+Quorum))
**Analysis**: Under this classification the target design needs **no linearizable read at learners at all**. The token approach (clock-free, Sans-I/O-trivial) covers the operator. Linearizable decisions are commands, not reads, and naturally land on the leader. This lowers the priority of the "`viewstamp` has no lease/read-index" gap: it matters only for a linearizable *read API* offered to users, and ReadIndex-style forwarding covers that at one leader round trip.

### Finding A3.5: What each read mechanism requires of a Sans-I/O consensus library

**Evidence**: Mechanism requirements are drawn from the sources above:
- **VR primary leases** require that "a new view will start only after leases at f+1 participants ... expire". The *view-change* logic must consult lease state, so leases cannot be layered outside the core.
- **ReadIndex** requires a leadership-confirming quorum heartbeat and an apply-index wait.
- **The §6.3.2 token** requires only that replies expose the op-number and that replicas expose their executed op.
- **Quorum and roster leases** require that the *write* path waits on lease holders.

`viewstamp`'s core "owns no I/O, no clock, and no randomness source" (docs.rs snippet).
**Source**: [VR Revisited](http://ying-zhang.cn/dist/2012-vr.html); [docs.rs/viewstamp](https://docs.rs/viewstamp/latest/viewstamp/); [LeaseGuard](https://arxiv.org/abs/2512.15659). All accessed 2026-09-25.
**Confidence**: Medium-High
**Verification**: [Hermes](https://arxiv.org/abs/2001.09804) (leases guard membership, "loosely synchronized clocks")
**Analysis**: A clock-free Sans-I/O core can still support leases, provided the driver feeds *monotonic time* as an input event and the core is parameterised by a maximum clock-drift bound. The lease must also be wired into the view-change and election guard *inside* the core. So for `viewstamp`, ReadIndex and the token path can be added in the driver or as new messages, while a correct lease needs a core change. The DST simulator must then inject clock drift. VOPR-style simulators already control time, so this is feasible, but it is new surface area in the least-tested part of the stack.

## A4. Rolling upgrades of the consensus layer

### Finding A4.1: The upgrade story is the most likely disqualifier for `viewstamp` as-is. Its handshake rejects any non-identical wire version, and rolling upgrades are declared future work

**Evidence**: `viewstamp`: "Pre-1.0 the cluster upgrade story is flag-day: stop all replicas, upgrade all of them, restart — mixed-version clusters are rejected at connect time"; "Rolling upgrades require wire-version negotiation, which is future work". Messages from "any other version [are] rejected as `CodecError::UnknownVersion`", while durable formats accept a compatibility range. None of the open issues tracks rolling upgrades (see Finding V2).
**Source**: [al8n/viewstamp README](https://raw.githubusercontent.com/al8n/viewstamp/main/README.md); [docs.rs/viewstamp](https://docs.rs/viewstamp/latest/viewstamp/). Both accessed 2026-09-25.
**Confidence**: High
**Verification**: Two primary artefacts (README, rustdoc) agree
**Analysis**: In the target topology every node is a replica, as voter or learner. A flag-day upgrade therefore stops *every node in the cluster*, not just a 3–5 node control-plane tier. For an orchestrator whose workers keep running workloads, a flag-day upgrade of the log need not kill the data plane. It does freeze all intent changes and all new assignments for the duration. That is incompatible with "always-on".

### Finding A4.2: TigerBeetle's multiversion binaries coordinate upgrades through the replicated log, and this was still the most bug-dense area in Jepsen's analysis

**Evidence**:
- **Multiversion binaries**: "Each TigerBeetle binary includes the code not just for that particular version, but several previous versions." To upgrade, "one simply replaces the binary on disk. TigerBeetle loads the new binary, but continues running with the current version", then "will ... coordinate the actual upgrade when all replicas are ready and have the latest version available". There is "a short period of unavailability as the replicas restart ... on the order of 5 seconds". Versions cannot be skipped arbitrarily: each release names "the oldest release that can be upgraded from".
- **Coordination in code**: replicas advertise their available `releases` in pings, the primary tracks `upgrade_targets`, and it proposes an `upgrade_release` greater than the current release. That proposal is driven by `on_upgrade_timeout()` on the primary.
- **Jepsen upgrade bugs**:
  - #2745 checkpoint divergence on upgrade, "High (potential data loss)", which TigerBeetle "opted not to patch ... but updated the changelog";
  - #2758, a panic under rapid version changes, fixed 0.16.29;
  - #2763, "switch on corrupt value" on deprecated message types during upgrade, fixed 0.16.29.

**Source**: [TigerBeetle: Upgrading](https://docs.tigerbeetle.com/operating/upgrading/) (out-of-list primary, medium-high); [TigerBeetle replica.zig](https://github.com/tigerbeetle/tigerbeetle/blob/main/src/vsr/replica.zig) (github.com, medium-high); [Jepsen: TigerBeetle 0.16.11](https://jepsen.io/analyses/tigerbeetle-0.16.11) (out-of-list primary, medium-high). All accessed 2026-09-25.
**Confidence**: High
**Verification**: Three sources from two independent organisations (TigerBeetle, Jepsen); [TigerBeetle PR #2766 "Fix auto-upgrade stall race"](https://github.com/tigerbeetle/tigerbeetle/pull/2766)
**Analysis**: TigerBeetle proves the *pattern*: advertise capability, let the leader commit an upgrade op, and have replicas switch at that op. It also shows the pattern is subtle, with three of Jepsen's twelve findings in upgrade code. `viewstamp` inherits the protocol from TigerBeetle, but it does not inherit the upgrade machinery or its multi-year hardening. Multiversion binaries are also a Zig/static-binary technique (exec of an embedded older version). A Rust equivalent would more likely take the form of *N−1/N dual codecs* inside one binary.

### Finding A4.3: Three production control planes converge on one pattern: nodes advertise supported version ranges, a single authority validates, and a committed version gate activates new behaviour

**Evidence**:
- **Kafka (KIP-584, KIP-778)**: each broker advertises "The minimum supported version ... The maximum supported version" in `ApiVersionsResponse`. The controller allows upgrading to version Y only if "Y falls in the [min_version, max_version] range advertised by each live broker". In KRaft, `metadata.version` "is stored as a `FeatureLevelRecord` in the metadata log". Upgrade is "a rolling restart ... with the new binary" followed by "online finalization". Incompatible brokers remain "fenced", and controllers "terminate".
- **CockroachDB**: rolling upgrade "on one node at a time"; "Automatic finalization is enabled by default, and begins as soon as all nodes have rejoined the cluster using the new binary"; rollback is possible only before finalization. Version gates stop new features activating in mixed-version clusters.
- **etcd**: "operates with the protocol of the lowest common version". Downgrade is "a zero-downtime, rolling downgrade" and is limited to "one minor version at a time".

**Source**: [KIP-584](https://cwiki.apache.org/confluence/display/KAFKA/KIP-584%3A+Versioning+scheme+for+features) (apache.org, high); [KIP-778](https://cwiki.apache.org/confluence/display/KAFKA/KIP-778%3A+KRaft+to+KRaft+Upgrades) (apache.org, high); [CockroachDB: Upgrade](https://docs.cockroachlabs.com/docs/stable/upgrade-cockroach-version) (out-of-list primary, medium-high); [etcd v3.6 downgrade](https://etcd.io/docs/v3.6/downgrades/downgrade_3_6/) (etcd.io, high). All accessed 2026-09-25.
**Confidence**: High
**Verification**: Four independent systems; CockroachDB version-gate practice corroborated on [github.com/cockroachdb/cockroach issue #167366](https://github.com/cockroachdb/cockroach/issues/167366)
**Analysis**: This pattern maps directly onto the target design and onto a Sans-I/O core. The pieces:
1. The handshake exchanges `[min_wire, max_wire]`, and a connection uses the maximum common version, instead of requiring equality.
2. Every node reports its supported range. Voters report through the protocol; learners can report through gossip, since this is a *fact* about the node.
3. The leader commits a `ClusterVersion(v)` entry only when every voter supports `v`, and optionally when all learners seen in the last T seconds do.
4. Behaviour switches at the committed op.
5. Nodes that do not support the committed version are fenced from applying and must upgrade.

This is also a concrete instance of "gossip carries facts, the log carries decisions".

### Finding A4.4: Automated upgrade by *voter replacement* (HashiCorp Autopilot) avoids mixed-version quorums altogether. The same mechanism implements failure-domain-aware voter selection

**Evidence**: Consul Autopilot (Enterprise):
- **Automated upgrades**: "Operators can introduce new-version servers that initially join as non-voters. Once sufficient upgraded servers are present, a leadership transition occurs, demoting legacy servers to non-voter status."
- **Redundancy zones**: servers across failure domains, where "Read replicas automatically promote to voter status when voting members fail".
- **Health and stabilisation**: new servers wait a stabilisation period (default "10s"), and health requires leader contact within "200ms" and log lag within "250" entries.

**Source**: [Consul Autopilot](https://developer.hashicorp.com/consul/docs/manage/scale/autopilot) (developer.hashicorp.com, high), accessed 2026-09-25
**Confidence**: Medium-High (single official source; the feature is long-standing and documented similarly for Nomad)
**Verification**: Not independently verified. Nomad's autopilot equivalent is from the same vendor and was not fetched (see Knowledge Gap 9). The *pattern* of promoting caught-up non-voters is independently corroborated by [KIP-853](https://cwiki.apache.org/confluence/display/KAFKA/KIP-853%3A+KRaft+Controller+Membership+Changes) and the [etcd learner design](https://etcd.io/docs/v3.6/learning/design-learner/).
**Analysis**: For the target, where every node already holds the full log as a learner, *voter rotation* is cheap. An upgrade could proceed as: upgrade the learners first → promote upgraded learners into the voter set one at a time (SingleChange) → demote old-version voters. This needs only *pairwise* wire compatibility between the old and new versions for the duration, not an arbitrary mixed-version quorum. It relies on `PromoteLearner`/`DemoteVoter` being robust, which is the least-simulated area of `viewstamp` (Finding V3). Autopilot's redundancy zones are also the closest production precedent for the design's "3–5 voters chosen automatically across failure domains".

### Finding A4.5: The virtual-log approach upgrades the *consensus protocol itself* without downtime by sealing one log segment and starting the next on the new version

**Evidence**: Delos "reached production within 8 months, and 4 months later upgraded its consensus protocol without downtime". Its VirtualLog "chains multiple shared log instances (called Loglets) into a single shared log", and loglets can be "instances of the same ordering protocol with different parameters ... or ... entirely distinct log implementations". Restate Bifrost (shared-log prior art only) states that "reconfiguration seals the active segment and atomically publishes a new segment as the head".
**Source**: [Virtual Consensus in Delos, OSDI'20](https://www.usenix.org/conference/osdi20/presentation/balakrishnan) (usenix.org, high); [Restate architecture](https://docs.restate.dev/references/architecture) (out-of-list primary). Both accessed 2026-09-25.
**Confidence**: High (Delos)
**Verification**: [Vanlightly on Delos](https://jack-vanlightly.com/blog/2025/2/5/an-introduction-to-virtual-consensus-in-delos); [ACM DL entry](https://dl.acm.org/doi/abs/10.5555/3488766.3488801)
**Analysis**: This is the strongest mitigation available *if* `viewstamp` stays flag-day at the wire level. Each segment is a separate consensus group with its own wire version, and the upgrade is a *seal-and-succeed* reconfiguration. Old-version replicas finish the sealed segment, and new-version replicas start the successor. Learners replay across segments using the recorded segment metadata. The price is the extra metadata tier and one brief sealing pause per upgrade. That pause is comparable to TigerBeetle's roughly 5 seconds, not a cluster-wide outage.

## A5. Storage faults in consensus

### Finding A5.1: Conventional consensus implementations do not use their redundancy to survive a single storage fault, and protocol-aware recovery (PAR/CTRL) is the research remedy

**Evidence**:
- **Ganesan et al. (FAST'17)** tested Redis, ZooKeeper, Cassandra, Kafka, RethinkDB, MongoDB, LogCabin and CockroachDB and found that "a single file-system fault can cause catastrophic outcomes such as data loss, corruption, and unavailability" and that "modern distributed systems do not consistently use redundancy to recover from file-system faults".
- **Alagappan et al. (FAST'18)** introduce Protocol-Aware Recovery and "Corruption-Tolerant Replication (CTRL) ... a PAR mechanism specific to replicated state machine (RSM) systems". They show "the CTRL versions of two systems, LogCabin and ZooKeeper, safely recover from storage faults and provide high availability, while the unmodified versions can lose data or become unavailable", with "little performance overhead".

**Source**: [Ganesan et al., "Redundancy Does Not Imply Fault Tolerance", FAST'17](https://www.usenix.org/conference/fast17/technical-sessions/presentation/ganesan) (usenix.org, high); [Alagappan et al., "Protocol-Aware Recovery for Consensus-Based Storage", FAST'18](https://www.usenix.org/conference/fast18/presentation/alagappan) (usenix.org, high). Both accessed 2026-09-25.
**Confidence**: High
**Verification**: [Wisconsin tech report of FAST'17](https://research.cs.wisc.edu/wind/Publications/fast17-ganesan.pdf) (system list); [ACM TOS extended version](https://research.cs.wisc.edu/adsl/Publications/cords-tos17.pdf); [Charapko on VR](https://charap.co/reading-group-viewstamped-replication-revisited/) ("VR is the only major SMR protocol specified without the need for a durable infallible disk")
**Analysis**: The storage-fault argument is the strongest *protocol-level* reason to prefer VSR over a stock Raft library. PAR is not built into Raft's specification. A Raft library can adopt it, but openraft, etcd-raft and HashiCorp Raft do not advertise it (see Knowledge Gaps).

### Finding A5.2: TigerBeetle's VSR implements protocol-aware recovery, and Jepsen found it "exceptional" against minority and helical corruption. Two boundary conditions remain: majority WAL-head corruption and single-node full disk loss

**Evidence**:
- **TigerBeetle's design**: "Faulty storage can not be fully encapsulated by the storage interface and requires consensus cooperation to resolve." The view change uses a *nack quorum*: "If the new primary collects a _nack quorum_ of _blank_ headers for a particular possibly-uncommitted op, it truncates the log." Faulty WAL slots are tracked (`journal.faulty`), and a replica that cannot establish its head waits in `recovering_head` status.
- **Jepsen's corruption nemesis** "Flipped random bits", "Replaced file chunks to simulate misdirected writes" and "Restored snapshots of file chunks to simulate lost writes", including "helical" faults across nodes. Results: TigerBeetle showed "exceptional resilience to disk faults" and tolerated "the loss or corruption of all but one copy" in the grid. Limits: "Corrupting the WAL head on a majority of nodes 'permanently disable[d]' a TigerBeetle cluster"; #2681a/b were panics on superblock bitflips; and #2767, "No safe recovery path for single-node disk failure", was High severity, fixed 0.16.43.

**Source**: [TigerBeetle ARCHITECTURE.md](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/ARCHITECTURE.md); [TigerBeetle vsr.md](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/vsr.md); [TigerBeetle replica.zig](https://github.com/tigerbeetle/tigerbeetle/blob/main/src/vsr/replica.zig) (github.com, medium-high); [Jepsen: TigerBeetle 0.16.11](https://jepsen.io/analyses/tigerbeetle-0.16.11) (out-of-list primary, medium-high). All accessed 2026-09-25.
**Confidence**: High
**Verification**: Independent (Jepsen) plus primary (TigerBeetle); academic grounding in [FAST'18 PAR](https://www.usenix.org/conference/fast18/presentation/alagappan)
**Analysis**: This is the evidence behind `viewstamp`'s storage-fault claim. It is evidence about *TigerBeetle*, a different codebase in a different language whose recovery code has been hardened by Jepsen and by years of VOPR. `viewstamp` is a reimplementation. Its own issue #68 says its VOPR "excludes new storage ... failures", and no external audit (Jepsen or Antithesis) of `viewstamp` was found.

### Finding A5.3: Typical Raft implementations fail under storage faults and in protocol extensions. Recent Jepsen and Antithesis work finds bugs in every implementation examined, including openraft

**Evidence**:
- **Jepsen, NATS 2.12.1 (2025-12-08)**: JetStream "lost writes if data files were truncated or corrupted on a minority of nodes". In one run, "file corruption ... caused NATS to lose 679,153 acknowledged writes out of 1,367,069 total". Also, "By default, NATS calls `fsync` ... only once every two minutes, but acknowledges messages immediately", contrary to the Raft thesis requirement to flush before acknowledging.
- **Antithesis (2026-07-27)**: "we've found bugs in _every_ Raft implementation we've tested, including HashiCorp Raft, Aeron Cluster, OpenRaft, and MicroRaft". Detailed HashiCorp Raft bugs are in async heartbeats (a safety violation), leadership transfer (deadlock) and InstallSnapshot (livelock), and "Like `InstallSnapshot`, this feature was never formally verified in the TLA+ spec".
- **etcd with Antithesis (2025)**: 830 hours simulating 4.5 years reproduced storage-crash bugs such as "Revision decreasing caused by crash during compaction" (#17780) and "Inconsistent revision caused by crash during defrag" (#14685).

**Source**: [Jepsen: NATS 2.12.1](https://jepsen.io/analyses/nats-2.12.1) (out-of-list primary, medium-high); [Antithesis: Finding bugs in Raft implementations](https://antithesis.com/blog/2026/finding-bugs-in-raft-implementations/) (out-of-list primary, medium-high; vendor with commercial interest); [etcd blog: Autonomous testing with Antithesis](https://etcd.io/blog/2025/autonomus_testing_with_antithesis/) (etcd.io, high). All accessed 2026-09-25.
**Confidence**: High (three independent sources; the Antithesis post gives no openraft bug details)
**Verification**: [CNCF re-publication of the etcd post](https://www.cncf.io/blog/2025/09/25/autonomous-testing-of-etcds-robustness/) (cncf.io, high); [nats-server issue #7564](https://github.com/nats-io/nats-server/issues/7564)
**Analysis**: Two lessons. First, the *extensions* (snapshots, leadership transfer, membership change, leases) are where bugs concentrate. These are exactly the features the target design adds on top of either candidate: learners at scale, automatic voter churn, lease or read-index reads, and upgrades. Second, the choice of library matters less than having deterministic, fault-injecting simulation *of the composed system*. VOPR-style simulation is an advantage of `viewstamp` only to the extent that it covers those extensions, and today it does not (issue #68).

### Finding A5.4: Having thousands of learners changes the storage-fault economics: every learner holding hash-chained committed entries is a potential repair donor (analysis)

**Evidence**: `viewstamp`'s README notes that "a demoted voter lingers here, still a reachable repair donor, until its removal is garbage-collected". TigerBeetle "doesn't make a distinction whether checksummed data comes from a local disk or another replica". Jepsen's residual TigerBeetle failure is "Corrupting the WAL head on a majority of nodes".
**Source**: [al8n/viewstamp README](https://raw.githubusercontent.com/al8n/viewstamp/main/README.md); [TigerBeetle ARCHITECTURE.md](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/ARCHITECTURE.md); [Jepsen: TigerBeetle](https://jepsen.io/analyses/tigerbeetle-0.16.11). All accessed 2026-09-25.
**Confidence**: Low-Medium (interpretation; no system found that uses non-voting replicas as PAR donors)
**Verification**: Not independently verified — flagged as a research direction
**Analysis** (interpretation): Under PAR, *committed* entries can be repaired from any replica that holds them with a matching checksum. Learners hold every committed entry, so in principle a corrupted committed entry on a voter could be repaired from a learner. The hard, unsolvable-by-learners case is the one Jepsen found: the *uncommitted head* on a majority of voters, because learners never see uncommitted prepares. The target could therefore get "all but one copy" durability for committed intent almost for free. It cannot remove the majority-head window, which is intrinsic to the voter count.

## A6. Write scaling beyond one group

### Finding A6.1: A single batched consensus group already covers orchestration intent at thousands of nodes. The published limits bind on state size and fan-out, not write throughput

**Evidence**:
- **etcd v3.6 benchmarks**: "44,341" writes/s (100 connections, 1000 clients, to the leader) and "50,104" spread across members, on 8 vCPU / 16 GB machines. Performance is bounded by "network Round Trip Time (RTT) between members" and `fdatasync`. "etcd batches multiple requests together and submits them to Raft."
- **etcd limits**: default request limit "1.5 MiB" and storage quota "2 GiB", with "8 GiB ... a suggested maximum".
- **Kubernetes** supports "up to 5,000 nodes", "No more than 150,000 total pods", on one etcd cluster, and recommends that "you can store Event objects in a separate dedicated etcd instance".
- **Nomad C2M (2020-12-08)**: "3" servers in `us-east-1` scheduled 2,000,000 containers on "over 6,000" clients across "10 AWS regions" in about 22 minutes, at "nearly 1,500 containers per second", as "a single cluster topology".

**Source**: [etcd performance](https://etcd.io/docs/v3.6/op-guide/performance/) (etcd.io, high); [etcd limits](https://etcd.io/docs/v3.6/dev-guide/limit/) (etcd.io, high); [Kubernetes: Considerations for large clusters](https://kubernetes.io/docs/setup/best-practices/cluster-large/) (kubernetes.io, high); [HashiCorp: Nomad meets C2M](https://www.hashicorp.com/en/blog/hashicorp-nomad-meets-the-2-million-container-challenge) (out-of-list primary, medium-high). All accessed 2026-09-25.
**Confidence**: High
**Verification**: [hashicorp/c2m repository](https://github.com/hashicorp/c2m/blob/master/README.md) (github.com; its config shows `nomad_num_schedulers=6`, see Conflicting Information); [openraft README](https://github.com/databendlabs/openraft) ("millions of writes/sec" batched)
**Analysis** (interpretation):
- **Write rate.** Orchestration intent (specs, placement decisions, membership) is low-rate. Even Nomad's burst of about 1,500 placements/s is two orders of magnitude below a single etcd group's batched throughput. For the target, write TPS is not the reason to shard.
- **Real limits.** They are (a) **total state size** (etcd's 8 GiB guidance), which matters because *every learner* holds the full state; (b) **the fan-out cost of every write to every learner** (A2); and (c) **WAN commit latency** for regions far from the voters (A7).
- **Differences from Nomad.** Nomad's 6,000 clients are *not* Raft learners: they RPC to the servers. Nomad therefore proves single-group intent throughput, not single-group *full-replica* fan-out.
- **Kubernetes' event split.** Moving Events to their own etcd is production precedent for keeping high-volume, observation-like writes off the intent log.

### Finding A6.2: Multi-group sharding (Multi-Raft with a placement driver) is how CockroachDB and TiKV scale *data*. It brings per-group overhead that its own authors had to engineer around, plus a separate placement brain

**Evidence**:
- **TiKV**: "Multi-Raft only means we manage multiple Raft consensus groups on one node". Regions split (e.g. `[a, c)` → `[a, b)` + `[b, c)`) and merge, and TiKV polls "all Raft groups every 1000ms, batching ready states".
- **CockroachDB**: each node may participate in "hundreds of thousands of consensus groups". Heartbeat traffic grows with ranges, which led to "coalesced heartbeats (so that the number of nodes dictates the number of heartbeats ...)" and quiescence, where idle ranges "stop sending raft heartbeats".

**Source**: [TiKV deep dive: Multi-Raft](https://tikv.org/deep-dive/scalability/multi-raft/) (tikv.org, high); [CockroachDB: Scaling Raft](https://www.cockroachlabs.com/blog/scaling-raft/) (out-of-list primary, medium-high). Both accessed 2026-09-25.
**Confidence**: High
**Verification**: [CockroachDB quiesce-ranges RFC (github.com)](https://github.com/cockroachdb/cockroach/blob/master/docs/RFCS/20160824_quiesce_ranges.md); [CockroachDB replication-layer docs](https://docs.cockroachlabs.com/docs/stable/architecture/replication-layer)
**Analysis**: Multi-Raft addresses a problem the target does not have, namely large data volume and high write throughput. It would also break the target's key simplification that *one* log totally orders *all* intent, which the leader-hosted scheduler relies on. Cross-group decisions, such as placing a workload in region B based on capacity in region A, would need cross-group transactions or a coordinating root group. Verdict: **loses** for global intent. It is a candidate only for *region-local* intent that genuinely never crosses regions.

### Finding A6.3: Cassandra's move to a single ordered metadata log (CEP-21), sized by cluster size, shows how far one group stretches when it carries only *metadata*

**Evidence**: CEP-21's Cluster Metadata Service is "a subset of the nodes in the Cassandra cluster" that keeps "a totally ordered, immutable log of cluster state changes" (token ownership, schema, topology) using "**Paxos consensus**", with "a monotonically increasing **epoch**". Suggested CMS sizing: "1 member for clusters ≤2 nodes; 3 members for ≤5 nodes; 5 for ≤100; 10+ for larger clusters". The rollout guidance is to run "nodetool cms initialize" and then "nodetool cms reconfigure to add more members".
**Source**: [CEP-21: Transactional Cluster Metadata](https://cwiki.apache.org/confluence/display/CASSANDRA/CEP-21%3A+Transactional+Cluster+Metadata) (apache.org, high), accessed 2026-09-25
**Confidence**: Medium-High (design document plus release notes; production maturity in 6.0 is recent)
**Verification**: [apache/cassandra NEWS.txt](https://github.com/apache/cassandra/blob/trunk/NEWS.txt) (github.com, medium-high) describing CMS initialisation and reconfiguration
**Analysis**: Cassandra is a flat, peer-to-peer, gossip-native system at thousands of nodes. It chose *one* ordered metadata log with a small consensus subset that grows with the cluster, and it did **not** shard metadata. This is the closest structural precedent to the target design (see the Hypothesis section). It supports "one batched group" for intent, with automatic voter-count scaling.

## A7. Multi-region voter placement

### Finding A7.1: Surviving the loss of a region costs at least one inter-region round trip per write. Production systems expose this as an explicit survival-goal choice

**Evidence**:
- **CockroachDB, region survival**: "the database will remain fully available for reads and writes, even if an entire region becomes unavailable", which requires "at least 3 database regions". The replication factor rises "from 3 (the default) to 5", placed 2+2+1. "write latency will be increased by at least as much as the round-trip time to the nearest region. Read performance will be unaffected."
- **CockroachDB, zone survival** (the default) stays available if a zone fails, but may lose availability if a whole region fails.

**Source**: [CockroachDB: Multi-region survival goals](https://docs.cockroachlabs.com/docs/stable/multiregion-survival-goals) (out-of-list primary, medium-high), accessed 2026-09-25
**Confidence**: High (the physics is protocol-independent; corroborated by Spanner and ZooKeeper below)
**Verification**: [Spanner replication](https://docs.cloud.google.com/spanner/docs/replication) (Google Cloud, high); [ZooKeeper Observers WAN guidance](https://zookeeper.apache.org/doc/current/zookeeperObservers.html) (apache.org, high)
**Analysis**: The target design must choose between two postures:
- **(a) Zone survival**: 3–5 voters in one "home" region across its zones. Writes are fast. If the home region is lost, intent is *frozen*, though learners keep serving reads and workers keep running what they were assigned.
- **(b) Region survival**: 5 voters placed 2+2+1 across three regions. Every intent write pays about one inter-region round trip.

For an orchestrator, (a) degrades more gracefully than it sounds, because the data plane keeps working without the log. Whether this is acceptable is a product question the research cannot settle.

### Finding A7.2: Witness replicas and read-only replicas split the voter role: vote without full state, or hold full state without voting. The target's learners already match Spanner's read-only replicas; witnesses are the missing piece

**Evidence**:
- **Spanner witnesses** "vote whether to commit writes, but don't store a full copy of the data, can't become the leader, and can't serve reads". Read-only replicas "maintain a full copy ... but don't participate in voting", which allows "scaling read capacity without enlarging the write quorum".
- **VR §6.1**: "f+1 active replicas, which store the application state and execute operations, and f witnesses, which do not ... Witnesses are needed for view changes and recovery."
- **ZooKeeper** recommends running the voting ensemble in one datacenter "with Observers only in the second datacenter".

**Source**: [Spanner replication](https://docs.cloud.google.com/spanner/docs/replication) (cloud.google.com, high); [VR Revisited §6.1](http://ying-zhang.cn/dist/2012-vr.html) (MIT TR via mirror, high); [ZooKeeper Observers](https://zookeeper.apache.org/doc/current/zookeeperObservers.html) (apache.org, high). All accessed 2026-09-25.
**Confidence**: High
**Verification**: Three independent systems
**Analysis**: For posture (b), the fifth voter in the "+1" region can be a *witness* (a small, log-only voter) rather than a full voter, reducing the cost of region survival. `viewstamp` documents learners but not witnesses (Finding V2), and neither does openraft (see Knowledge Gaps). In VR, however, witnesses are part of the protocol's own literature (§6.1), which makes them a natural extension for a VR library.

### Finding A7.3: Multi-leader WAN protocols (WPaxos, EPaxos) win only when access has regional locality over partitioned objects. Global scheduling intent lacks that locality

**Evidence**: WPaxos "partitions the object-space among these multileaders", adapts "to the changing access locality through object stealing", and "appoints phase-2 acceptors to be close to their respective leaders" (flexible quorums), outperforming "both partitioned Paxos deployments and leaderless Paxos approaches" across five AWS regions. EPaxos has "significantly worse tail latency ... up to 4x worse" under conflicting WAN workloads (Finding A1.4).
**Source**: [Ailijiang, Charapko, Demirbas, Kosar, "WPaxos", arXiv:1703.08905](https://arxiv.org/abs/1703.08905) (arxiv.org, high); [EPaxos Revisited, NSDI'21](https://www.usenix.org/conference/nsdi21/presentation/tollman) (usenix.org, high). Both accessed 2026-09-25.
**Confidence**: Medium-High
**Verification**: [Flexible Paxos](https://arxiv.org/abs/1608.06696) (the quorum idea WPaxos builds on)
**Analysis** (interpretation): WPaxos-style *object ownership* could fit **region-pinned** workloads, where a region's leader owns the objects of workloads pinned to that region. It adds a second ordering authority and the cross-owner coordination that the single-leader scheduler avoids. It is a candidate for a *later* phase, if region-local intent latency becomes a measured problem. It is not a first choice.

### Finding A7.4: WAN voter sets need connectivity-aware elections and careful single-change reconfiguration. Kafka KRaft's KIP-853 is a recent, well-specified reference

**Evidence**:
- **KIP-853**: voters had to "shutdown all of the controllers nodes and manually make changes" before this KIP. It enforces "one voter change at a time" and requires that "Replicas must reach log-end offset before becoming voters". It persists the voter set in the log as a `VotersRecord`, and assigns each replica "a unique **replica directory ID**" so that a replaced disk is treated as a new replica.
- **ZooKeeper**: "high variance in latency between datacenters could lead to false positive failure detection".
- **Cloudflare**: a partial network failure left etcd "unable to establish a stable leader".

**Source**: [KIP-853](https://cwiki.apache.org/confluence/display/KAFKA/KIP-853%3A+KRaft+Controller+Membership+Changes) (apache.org, high); [ZooKeeper Observers](https://zookeeper.apache.org/doc/current/zookeeperObservers.html); [Cloudflare](https://blog.cloudflare.com/a-byzantine-failure-in-the-real-world/). All accessed 2026-09-25.
**Confidence**: High
**Verification**: [OmniPaxos EuroSys'23](https://dl.acm.org/doi/abs/10.1145/3552326.3587441) (quorum-connected election); [etcd learner design](https://etcd.io/docs/v3.6/learning/design-learner/) (catch-up-before-promote)
**Analysis**: KIP-853's *directory ID* point is directly relevant to storage faults and to VR recovery. A replica whose disk was wiped must not rejoin under its old identity with an empty log, because that is the classic amnesia hazard. `viewstamp`'s documented `SingleChange` verbs plus the "fresh durable-prefix proof" on promotion (Finding V2) point the same way. Whether `viewstamp` keys replica identity by incarnation is a knowledge gap. Partial-connectivity resilience (OmniPaxos, Finding A1.9) matters more across regions than within one.

## `viewstamp` crate — current state

### Finding V1: `viewstamp` is a pre-0.1 single-author Sans-I/O VSR library whose stated upgrade model is flag-day only

**Evidence**: The README describes it as "Pure-Rust Viewstamped Replication: a Sans-I/O consensus state machine, QUIC and TCP+TLS transports, real-I/O drivers, and a deterministic adversarial simulator." On maturity: "Pre-0.1 and unpublished; the wire and on-disk formats are unstable and upgrades are flag-day." On versioning: "Pre-1.0 the cluster upgrade story is flag-day: stop all replicas, upgrade all of them, restart — mixed-version clusters are rejected at connect time", and "Rolling upgrades require wire-version negotiation, which is future work." The docs.rs build (version `0.0.0`, published 2026-09-18) adds that messages use strict version equality: "any other version is rejected as `CodecError::UnknownVersion`", while durable formats accept a compatibility range.
**Source**: [al8n/viewstamp README](https://github.com/al8n/viewstamp) (github.com, medium-high), accessed 2026-09-25; [viewstamp on docs.rs](https://docs.rs/viewstamp/latest/viewstamp/) (docs.rs, high), accessed 2026-09-25
**Confidence**: High (for the claims about the crate itself; both are primary sources from the author)
**Verification**: The raw README ([raw.githubusercontent.com](https://raw.githubusercontent.com/al8n/viewstamp/main/README.md)) and the docs.rs page agree on the version, flag-day and handshake-fence statements.
**Analysis**: Wire versions must match exactly, while the durable format accepts a compatibility *range*. So the durable side is already built to evolve, and the missing piece is wire negotiation plus a log-coordinated activation step (see A4). Under the target design, a flag-day upgrade means a full stop of every voter *and every learner*, because learners speak the same wire protocol. In an always-on flat cluster that is an outage of the whole control plane. This is the most material gap of the crate for this design.

### Finding V2: Feature surface: SingleChange reconfiguration, learners, content-addressed incremental checkpoints and a storage-fault model; no documented lease or read-index read path

**Evidence**: README: "Dynamic reconfiguration — the membership mode is a compile-time choice" (`RestartOnly` or `SingleChange`); "AddLearner/PromoteLearner grows the voting set", "DemoteVoter/RemoveLearner shrinks it"; "A shrink that would lower crash tolerance is refused unless the caller names AcceptReducedFaultTolerance". Learners: "Non-voting learners / standbys — replicas that receive the log and stay current without counting toward quorum, so quorum math is unchanged." Checkpoints: "checkpoints are a content-addressed block DAG", and "a laggard state-syncs only the blocks that actually changed". Storage: "WAL corruption (torn writes, bit-rot, misdirected reads) is part of the fault model ... faults surface as data the protocol repairs from peers — never as panics." Threat model: "explicitly **not** Byzantine-fault-tolerant". Crates: `viewstamp-proto` (Sans-I/O core), `viewstamp-driver`, `viewstamp-compio`, `viewstamp-reactor`, `viewstamp-simulation` (VOPR). No throughput or latency figures are published. Neither the README, the docs.rs page nor the open-issue list mentions lease reads, read-index, linearizable reads at non-primaries, or learner-to-learner relay. The only relay described is that a backup "relays to the primary" (client request forwarding).
**Source**: [al8n/viewstamp README](https://raw.githubusercontent.com/al8n/viewstamp/main/README.md), accessed 2026-09-25
**Confidence**: Medium (single primary source; "absence" claims rest on README + issue list + docs.rs, not a code audit)
**Verification**: [docs.rs viewstamp](https://docs.rs/viewstamp/latest/viewstamp/) (no read/lease discussion); [open issues](https://github.com/al8n/viewstamp/issues) (none about reads, upgrades or relay)
**Analysis**: The reconfiguration verbs match the target design's working model, and the refusal to shrink fault tolerance without an explicit override is a useful safety property for an automatic voter picker. However, the design's "reads are local" rule and "thousands of learners" scale both rest on features the crate does not document: a linearizable read path, and a way to relay the log from learner to learner (see A2 and A3). Both would have to be built on top of the library or contributed to it.

### Finding V3: Activity is bursty and single-maintainer, and self-directed adversarial audits dominate the open issues

**Evidence**: The commit page shows 285 commits. The visible history runs from 2026-07-31 to 2026-09-02, with 21 commits on 2026-09-02 alone. Every visible commit is by one author (`al8n`). There is 1 star and 1 fork. All 12 listed open issues date from 2026-07-12 to 2026-07-19. Ten of them are by `al8n` and two are by `uqio`. They are framed as "Adversarial audit round 3–5" findings, for example "#71 release builds can commit undeliverable oversized reply", "#68 VOPR excludes new storage, reconfiguration, and client-lifecycle failures", and "#84 New-member bootstrap: an authenticated Joining endpoint mode". The contributors graph failed to load.
**Source**: [viewstamp commits](https://github.com/al8n/viewstamp/commits/main), [viewstamp issues](https://github.com/al8n/viewstamp/issues), accessed 2026-09-25
**Confidence**: Medium (the web view shows only the first page of commits; `uqio` could be a second identity or an automated auditor — not verified)
**Verification**: Repository metrics on the README page (285 commits, 1 star, 1 fork, 21 open issues, 7 PRs) are consistent with the commit view.
**Analysis**: Issue #68 states that the simulator does *not yet* exercise new storage, reconfiguration or client-lifecycle failures. Those are exactly the paths the target design leans on hardest: automatic voter changes and learner promotion. Issue #84 shows that authenticated admission of a new member is also still open, and it maps directly onto the design's WireGuard-endpoint join flow. The bus factor is 1. An independent comparison: `uvrr-core` (lua-lunet), a second Sans-I/O VRR core that adds unbounded reconfiguration via David Turner's leader-casting-vote technique, describes itself as "Pre-alpha ... the API is not stable" ([github.com/lua-lunet/uvrr-core](https://github.com/lua-lunet/uvrr-core), accessed 2026-09-25). So neither Rust VR crate is production-mature.

## Challenging the hypothesis: "gossip carries facts, the log carries decisions"

### Finding H.1: The strongest external evidence *supports* the direction of the hypothesis. Cassandra moved decisions off gossip onto an ordered log after years of gossip-carried metadata bugs

**Evidence**: CEP-21's motivation:
- "There is no strict order for changes propagated through gossip, there can be no universal 'reference' state at any given time".
- Bootstrap can violate replication factor, because "writes may begin before coordinators guarantee the bootstrapping replica exists".
- Operators "artificially throttle changes using RING_DELAY".
- Concurrent schema changes "can cause unrecoverable states".

Under TCM, "the role of gossip will be reduced so that critical changes to node state are no longer triggered by gossip (though tools like nodetool gossipinfo should continue to function as before)".
**Source**: [CEP-21](https://cwiki.apache.org/confluence/display/CASSANDRA/CEP-21%3A+Transactional+Cluster+Metadata) (apache.org, high), accessed 2026-09-25
**Confidence**: High
**Verification**: [apache/cassandra NEWS.txt](https://github.com/apache/cassandra/blob/trunk/NEWS.txt); [Kubernetes large-cluster guidance](https://kubernetes.io/docs/setup/best-practices/cluster-large/) (high-volume Events moved off the main etcd, the mirror-image move of facts off the log)
**Analysis**: This is a flat gossip cluster at scale deciding that *decisions* (membership, ownership, schema) must be totally ordered, while *liveness information* stays in gossip. It is direct precedent for the hypothesis's core split.

### Finding H.2: Putting per-node facts on the consensus log forces throttling that grows with cluster size, as Consul's anti-entropy shows

**Evidence**: "Agents forward information about services and their registered health checks to the leader node in the cluster, which replicates the authoritative global service catalog". Sync intervals scale with size: "1-128 nodes sync every 1 minute, while those with 129-256 nodes sync every 2 minutes", with "a staggered start time". Authority is inverted: "Consul treats the state of the agent as authoritative".
**Source**: [Consul: Consistency / anti-entropy](https://developer.hashicorp.com/consul/docs/concept/consistency) (developer.hashicorp.com, high), accessed 2026-09-25
**Confidence**: Medium-High (single official source; the scaling table is self-evidently a throttle)
**Verification**: [Serf/Consul gossip prior research in this repo](../gossip-protocols/serf-internals-and-overdrive-fit.md) (membership via gossip, catalog via Raft)
**Analysis**: Consul routes owner-authoritative *facts* through the Raft log and then has to slow their freshness as the cluster grows. The fact's owner stays authoritative anyway. This is the counterfactual cost the hypothesis avoids, so the evidence supports keeping observation off the log.

### Finding H.3: The hypothesis conflates *authority* with *transport*, and the evidence separates the two (analysis grounded in KRaft, Plumtree and hash chains)

**Evidence**:
- Kafka brokers consume the metadata log by pull (`FetchRequest`). Changes "need to be persisted to the __cluster_metadata log before we apply them on the other nodes" (KIP-631), but the *bytes* can come over any path.
- Plumtree can carry any payload (A2.3).
- Hash-chained entries are verifiable regardless of who relays them (A2.5).

**Source**: [KIP-631](https://cwiki.apache.org/confluence/display/KAFKA/KIP-631%3A+The+Quorum-based+Kafka+Controller) (apache.org, high); [Epidemic Broadcast Trees](https://www.dpss.inesc-id.pt/~ler/reports/srds07.pdf); [TigerBeetle ARCHITECTURE.md](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/ARCHITECTURE.md). All accessed 2026-09-25.
**Confidence**: Medium (interpretation over high-quality sources)
**Verification**: [ZooKeeper observer masters](https://zookeeper.apache.org/doc/current/zookeeperObservers.html) (log carried peer-to-peer, not only from the leader)
**Analysis**: A more precise statement of the hypothesis: *facts are owner-authoritative; decisions are log-authoritative; delivery is transport-agnostic*. Under that statement, gossip or a relay tree can legitimately carry *committed log entries* to learners, which is the answer to A2. Nothing is lost, because a learner applies an entry only after checking it against the committed hash chain.

### Finding H.4: Facts that *gate a safety decision* should reach the decider over a lease or heartbeat channel, and the decision should record its inputs. KRaft does exactly this for "node is down"

**Evidence**:
- **KIP-631**: brokers send heartbeats "(default 3-second intervals) to maintain their lease, which lasts 18 seconds by default". The heartbeat carries "the broker's current metadata offset". The controller records `FenceBrokerRecord`/`UnfenceBrokerRecord` in the log and "will not actually unfence the broker unless its metadata is reasonably current". Registration gets "an 'incarnation ID' (UUID)" and a broker epoch "based on the next available offset in the log".
- **KIP-584 applies the same split to versions**: supported ranges are facts, and the finalized version is a committed decision.

**Source**: [KIP-631](https://cwiki.apache.org/confluence/display/KAFKA/KIP-631%3A+The+Quorum-based+Kafka+Controller); [KIP-584](https://cwiki.apache.org/confluence/display/KAFKA/KIP-584%3A+Versioning+scheme+for+features). Both apache.org, high, accessed 2026-09-25.
**Confidence**: High (for KRaft's design); Medium (for the prescription)
**Verification**: [Consul Autopilot](https://developer.hashicorp.com/consul/docs/manage/scale/autopilot) (health = leader contact + log lag, decided at the leader); [etcd learner promotion](https://etcd.io/docs/v3.6/learning/design-learner/) (promotion gated on catch-up)
**Analysis**: Where the hypothesis needs sharpening:
1. **SWIM gossip is suspicion, not evidence.** Gossip-based failure detection is fine for *suspicion*. The committed "node is down" decision is safer when the leader holds a *lease* view of each node, fed by direct heartbeats or by gossip suspicion that the leader confirms. KRaft pairs a lease with a log record.
2. **Learner lag gates decisions.** Learner lag is a fact that gates decisions such as promote-to-voter or unfence-for-assignment, so it must be observable *by the leader*, not only through gossip.
3. **Admission and incarnation are decisions.** A node's admission and incarnation identity are *decisions* (log). Discovery and peer exchange are *facts* (gossip). This maps cleanly onto the target's WireGuard peer-exchange join flow.

So the hypothesis survives, with two refinements. Gossip is a *transport and suspicion* layer, not an authority. And safety-gating facts get a leader-observed lease path in addition to gossip.

## Known candidates — latest state

| Candidate | State as of 2026-09-25 | Strengths for the target | Weaknesses for the target | Key findings |
|---|---|---|---|---|
| **VR Revisited** (paper) | Stable 2012 specification | Clock-free recovery ("without ... durable infallible disk"); §6.3.2 read-your-writes tokens; §6.1 witnesses; §7 epoch reconfiguration | Specification only; reconfiguration via epochs and state transfer, with no production reference implementation of §7 found | A1.1, A3.1, A7.2 |
| **TigerBeetle VSR** | Production; Jepsen-validated (strong serializability as of 0.16.30; all but one issue fixed by 0.16.45); upgrades via multiversion binaries | Protocol-aware storage-fault recovery; flexible quorums (3/6 replicate, 4/6 view change); log-coordinated upgrades; standbys; hash-chained log; VOPR | **No online reconfiguration** ("TODO (Unimplemented)"); fixed-size clusters; Zig; ring topology needed experimental star/closed-loop fixes; upgrade code was Jepsen's bug hotspot | A1.2, A2.5, A2.6, A4.2, A5.2 |
| **`viewstamp`** (al8n) | Pre-0.1; docs.rs `0.0.0` (2026-09-18) while the README says "unpublished"; ~285 commits; 1 author visible; 1 star | Sans-I/O core (no I/O, clock or RNG); TigerBeetle-derived protocol; storage-fault model; VOPR; SingleChange reconfiguration with learners; content-addressed incremental checkpoints; mTLS transports | **Flag-day upgrades only** (exact-version wire handshake); no documented lease or read-index reads; no documented learner-to-learner relay; VOPR does not yet cover reconfiguration or new storage failures (#68); authenticated join still open (#84); bus factor 1; no external audit | V1–V3, A4.1 |
| **openraft** | 0.9.25 stable (2026-07-28); 0.10.0-alpha.35 on main; ~3,077 commits; 100+ contributors; 13+ adopters | Learners; joint-consensus membership change; `ensure_linearizable()` (ReadIndex); opt-in `LeaseRead` in 0.10 (partially verified); batching; large community | Not Sans-I/O in the `viewstamp` sense (async runtime); no advertised protocol-aware storage recovery; pre-1.0 "upgrade may contain incompatible changes"; Antithesis reports bugs found (details unpublished) | A1.3, A3.2, A5.3 |
| **Raft (protocol)** | Dominant in production (etcd, Consul, Nomad, KRaft, CockroachDB, TiKV) | Well-known membership change (KIP-853, etcd learners), ReadIndex, many reference points | Specification assumes a durable, uncorrupted disk; leases often "implemented incorrectly or not at all" (LeaseGuard); partial-connectivity liveness failures (Cloudflare 2020) | A1.1, A3.2, A5.3, A1.9 |

## Unexplored alternatives (ranked)

The ranking weighs relevance to the target design (flat topology, one log, thousands of learners, always-on, Rust, Sans-I/O) against production evidence. Each row states the problem the alternative solves, where it beats and loses to the current direction, and its maturity.

| Rank | Alternative | Solves | Beats current direction on | Loses on / cost | Maturity and evidence |
|---|---|---|---|---|---|
| 1 | **Cassandra Transactional Cluster Metadata (CEP-21) as a design template**: small consensus subset sized by cluster size; every inter-node message carries the metadata epoch; a lagging node catches up "by requesting the sequence of log entries with a greater epoch" from peers or the CMS | A1, A2, hypothesis | A complete, shipped answer to "flat gossip cluster + ordered decisions + automatic voter sizing + catch-up from any peer"; epoch-in-message gives free lag detection | Paxos-based (not VR); Java; CMS reconfiguration reuses Cassandra's bootstrap protocol; recent (6.0) | Accepted CEP, shipped in Cassandra's TCM ([CEP-21](https://cwiki.apache.org/confluence/display/CASSANDRA/CEP-21%3A+Transactional+Cluster+Metadata), [NEWS.txt](https://github.com/apache/cassandra/blob/trunk/NEWS.txt)) — H.1, A6.3 |
| 2 | **Pull-based / chained learner replication** (KRaft observers, ZooKeeper observer masters, MongoDB pull-based consensus) plus hash-chain verification | A2 | Removes leader fan-out; learners resume from any source; safe for non-voters by construction | Needs sync-source selection and relay-failure handling; not in `viewstamp` today | Production in three systems ([KIP-595](https://cwiki.apache.org/confluence/display/KAFKA/KIP-595%3A+A+Raft+Protocol+for+the+Metadata+Quorum), [ZooKeeper](https://zookeeper.apache.org/doc/current/zookeeperObservers.html), [NSDI'21](https://www.usenix.org/conference/nsdi21/presentation/zhou)) — A2.2 |
| 3 | **Range-negotiated wire versions plus a log-committed cluster-version gate** (KIP-584/778, CockroachDB finalization, TigerBeetle upgrade op) | A4 | Turns flag-day into rolling; decision in the log, facts (supported ranges) from nodes; unsupported nodes fenced | Requires N−1/N compatibility discipline and mixed-version simulation; upgrade code is historically bug-dense (Jepsen) | Production in Kafka, CockroachDB, etcd, TigerBeetle — A4.2, A4.3 |
| 4 | **Virtual consensus / sealed log segments** (Delos VirtualLog; Restate Bifrost as shared-log prior art) | A4, A6, A7 | Swap or upgrade the consensus engine itself without downtime; natural seam for per-region segments | Adds a metadata tier for the segment chain; one sealing pause per upgrade | Production at Meta ("1.8 billion transactions per day"; protocol swapped live) — A1.7, A4.5 |
| 5 | **Automated voter management** (Consul Autopilot redundancy zones and upgrade-by-voter-rotation; KIP-853 one-change-at-a-time, catch-up-before-promote, directory IDs) | A1 (voter selection), A4, A7 | Direct precedent for "3–5 voters auto-chosen across failure domains"; upgrade without mixed-version quorums | Depends on robust promote/demote, the least-tested `viewstamp` path; Autopilot zones are Enterprise-only | Production in Consul and Kafka — A4.4, A7.4 |
| 6 | **Plumtree over HyParView** push dissemination (`iroh-gossip`, Sans-I/O `proto`) | A2 (latency) | Tree-efficient push with self-repair; Rust and Sans-I/O | Best-effort: must be paired with pull repair; production scale unpublished | Academic (SRDS'07/DSN'07) plus active Rust crate (0.101.0, 2026-09-07) — A2.3 |
| 7 | **Witness voters** (VR §6.1, Spanner witnesses) | A7 | Cheaper fifth voter for region survival | Not documented in `viewstamp` or openraft | Production in Spanner; specified in VR — A7.2 |
| 8 | **Modern lease protocols** (LeaseGuard for Raft; Bodega roster leases; Paxos Quorum Leases) | A3 | Zero-round-trip linearizable reads (LeaseGuard); local linearizable reads at chosen replicas (Bodega, PQL) | Clocks and drift bounds enter the core; writes wait on lease holders (Bodega, PQL); research-stage | LeaseGuard: SIGMOD'26 with TLA+; Bodega: 2025 preprint — A3.2, A3.3 |
| 9 | **OmniPaxos** (Rust, no built-in I/O) | A1, A7 (partial connectivity) | Quorum-connected leader election survives partial partitions; reconfiguration decoupled from replication | "in-development"; no listed production users; small community | EuroSys'23; 223 stars — A1.9 |

**Evaluated and not recommended for intent ordering**:
- **EPaxos, Atlas, Tempo, Accord.** Orchestration commands contend for the same resources and need a global scheduler; EPaxos is bug-prone (A1.4).
- **DAG BFT (Narwhal, Bullshark, Mysticeti, Shoal).** Byzantine machinery the trust model does not need, sized for throughput the workload does not have (A1.6).
- **Multi-Raft with a placement driver.** Breaks one total order over all intent (A6.2).
- **Hermes, CRAQ, chain replication.** Write-all or chain latency; built for 3–7 replicas (A2.6, A3.3).
- **Turbine-style erasure-coded UDP trees.** Built for large blocks under loss (A2.4).
- **CometBFT validator machinery.** Only the delayed-activation idea transfers (A1.10).

## Comparison against the current direction

The current direction is: VSR via `viewstamp`, 3–5 auto-chosen voters, every other node a learner, local reads, writes forwarded to the leader, the scheduler at the leader, and observation off the log.

| Requirement | Current direction (`viewstamp`) | openraft alternative | Evidence-backed gap or mitigation |
|---|---|---|---|
| Crash-fault ordering of intent | Meets (VSR, same class as Raft) | Meets | A1.1 |
| Storage-fault tolerance | **Stronger by design** (protocol-aware recovery inherited from TigerBeetle), but unaudited in this codebase | Weaker (no advertised protocol-aware recovery) | A5.1–A5.3. Mitigation: external audit / Antithesis-style campaign on `viewstamp` |
| Deterministic simulation | **Stronger** (VOPR is a pure function of the seed) | Present (openraft cites simulation fuzzing) | VOPR does not yet cover reconfiguration or new storage faults (#68) |
| Automatic voter churn | **Riskiest path** (SingleChange exists; TigerBeetle never shipped reconfiguration; simulation incomplete) | Mature (joint consensus; years of production in the Raft ecosystem) | A1.2, A4.4, A7.4 |
| Thousands of learners | Not addressed: leader push only; no relay documented | Not addressed: leader push only | Both need pull-based relay plus hash-chain verification (A2.2, A2.5) |
| Local reads | Read-your-writes tokens trivially addable (VR §6.3.2); no linearizable-read path | ReadIndex available; LeaseRead in 0.10 (partially verified) | Linearizable reads at learners are not required if decisions are commands (A3.4) |
| Rolling upgrades | **Fails today** (flag-day, exact-version handshake) | Pre-1.0 API instability; no documented rolling-upgrade protocol either | Range negotiation plus committed version gate (A4.3), or seal-and-succeed virtual log (A4.5) |
| Write scaling | One batched group is enough | Same | Binding limits are state size, learner fan-out and WAN latency, not throughput (A6.1) |
| Multi-region | Supported in principle; no witnesses; round-robin view change is exposed to partial connectivity | Same, plus PreVote/CheckQuorum ecosystem practice | A7.1–A7.4, A1.9 |
| Maturity and bus factor | Pre-0.1, single maintainer | 100+ contributors, 13+ adopters, pre-1.0 | V3, A1.3 |

**Net assessment** (interpretation):
- **Protocol choice.** VSR is the right *protocol family* for storage-fault tolerance, and the target design's shape (small voter set, full-log learners, scheduler at the leader, decisions on the log) is well supported by prior art (KRaft, ZooKeeper, Cassandra TCM, Spanner).
- **Library choice.** The *library* is where the current direction is weakest. `viewstamp`'s gaps (A4 upgrades, A2 relay, reconfiguration simulation coverage) are all in the extensions that Antithesis and Jepsen evidence says are where consensus bugs live. None is a protocol-level impossibility. Each is new engineering on a pre-0.1, single-maintainer codebase.

## Conflicting information

### Conflict 1: Nomad C2M server count
**Position A**: "3" Nomad servers in `us-east-1` — Source: [HashiCorp blog, 2020-12-08](https://www.hashicorp.com/en/blog/hashicorp-nomad-meets-the-2-million-container-challenge), reputation 0.8 (out-of-list primary).
**Position B**: The C2M repository configures `nomad_num_schedulers=6` — Source: [hashicorp/c2m README](https://github.com/hashicorp/c2m/blob/master/README.md), reputation 0.8.
**Assessment**: The blog describes the executed run; the repository may carry a template default or include non-voting schedulers. The conclusion (a single small server group handles 6,000+ clients) holds either way.

### Conflict 2: Is `viewstamp` published?
**Position A**: "Pre-0.1 and unpublished" — Source: [README](https://github.com/al8n/viewstamp), reputation 0.8.
**Position B**: docs.rs serves `viewstamp 0.0.0`, published 2026-09-18 — Source: [docs.rs](https://docs.rs/viewstamp/latest/viewstamp/), reputation 1.0.
**Assessment**: Most likely a name reservation or placeholder publish. The maturity statement (unstable formats, flag-day) is identical in both, so the conflict does not affect conclusions.

### Conflict 3: openraft's current version
**Position A**: README describes 0.10.0-alpha.35 on the main branch — Source: [GitHub](https://github.com/databendlabs/openraft), reputation 0.8.
**Position B**: docs.rs "latest" is 0.9.25 (2026-07-28), and the in-repo changelog's newest entry is v0.9.0 (March 2024) — Source: [docs.rs](https://docs.rs/openraft/latest/openraft/docs/faq/index.html), [change-log.md](https://github.com/databendlabs/openraft/blob/main/change-log.md).
**Assessment**: Consistent once read as "0.9.x stable, 0.10 alpha". The changelog file is stale and should not be used to judge feature availability. The claim that `LeaseRead` exists in 0.10 therefore rests on secondary sources.

### Conflict 4: TigerBeetle upgrade downtime
**Position A**: "There's no need to stop the cluster for upgrades".
**Position B**: "upgrading causes a short period of unavailability as the replicas restart ... on the order of 5 seconds".
Both are from the [same page](https://docs.tigerbeetle.com/operating/upgrading/).
**Assessment**: Not contradictory: no operator-initiated stop is required, but there is a brief availability gap. It matters for the target's "always-on" bar.

### Conflict 5: openraft testing claims vs Antithesis findings
**Position A**: openraft cites deterministic simulation fuzzing and "Jepsen testing covering 8 nemesis scenarios" — Source: [openraft README](https://github.com/databendlabs/openraft).
**Position B**: Antithesis "found bugs in _every_ Raft implementation we've tested, including ... OpenRaft" — Source: [Antithesis](https://antithesis.com/blog/2026/finding-bugs-in-raft-implementations/) (vendor with commercial interest).
**Assessment**: Not strictly conflicting. "Jepsen testing" in the README appears to be self-run Jepsen-style testing, not a jepsen.io analysis, and Antithesis gives no openraft bug details. Treat both as unverified until details are published.

## Knowledge gaps

### Gap 1: `viewstamp` internals not verifiable from documentation
**Issue**: It is unknown whether `viewstamp` implements flexible quorums, witnesses, replica incarnation or directory IDs, pull-based fetch, or learner-to-learner relay. The contributors graph failed to load, and whether `uqio` is a second person is unknown. | **Attempted**: README (rendered and raw), docs.rs, issues page, commits page. | **Recommendation**: Code read of `viewstamp-proto` and a conversation with the maintainer; this document deliberately did not audit code.

### Gap 2: No published evidence of thousands of full-log learners behind one leader
**Issue**: Production precedents stop at "hundreds of Observers" (ZooKeeper) or do not publish broker counts per KRaft cluster; Nomad's 6,000 clients are RPC clients, not replicas. | **Attempted**: ZooKeeper, etcd, KIP-595, Nomad C2M, Kubernetes limits. | **Recommendation**: A simulation or benchmark spike of leader egress and relay-tree latency at 1k–5k learners.

### Gap 3: VR Revisited primary PDF not fetchable
**Issue**: MIT endpoints refused automated fetches; §6.1, §6.3 and §7 were quoted from an HTML mirror (ying-zhang.cn). | **Attempted**: pmg.csail.mit.edu, dspace.mit.edu (two URL forms), web.archive.org, secondary summaries. | **Recommendation**: Verify the quoted passages manually against the MIT TR PDF.

### Gap 4: openraft `LeaseRead` and witness or storage-fault features
**Issue**: The docs.rs page for 0.10's `ReadPolicy` returned 404. No openraft documentation on witnesses or protocol-aware storage recovery was found. | **Attempted**: docs.rs (0.9.25 FAQ, 0.10 alpha path), changelog, README. | **Recommendation**: Read openraft 0.10 source (`ReadPolicy`) and issue tracker.

### Gap 5: Adoption of protocol-aware recovery in Raft libraries
**Issue**: Whether any mainstream Raft library (etcd-raft, HashiCorp Raft, openraft) implements CTRL-style recovery was not established. | **Attempted**: FAST'17/'18 papers, Jepsen NATS, Antithesis posts. | **Recommendation**: Targeted search of each library's storage-corruption handling.

### Gap 6: Using learners as protocol-aware-recovery donors (Finding A5.4)
**Issue**: No system was found that repairs a voter's committed WAL entry from a non-voting replica. | **Attempted**: TigerBeetle, `viewstamp` README, FAST'18. | **Recommendation**: Treat as a research idea; validate in VOPR-style simulation.

### Gap 7: Orchestration-intent write rate
**Issue**: The claim that intent is "low-rate" rests on Nomad C2M's burst (about 1,500 placements/s) and etcd benchmarks, not on a model of the target's workload. | **Recommendation**: Model the write rate (deploys, placements, membership changes, `node down` decisions) for 1k and 10k nodes before any sharding discussion.

### Gap 8: Accord and TCM production status
**Issue**: Cassandra 6.0 is reported as alpha by a vendor source; the CEP pages do not state release status. | **Recommendation**: Check the Cassandra release notes when the 6.0 GA ships.

### Gap 9: Autopilot redundancy zones outside Enterprise, and in Nomad
**Issue**: Only the Consul page was fetched; Nomad's equivalent was not verified independently. | **Recommendation**: Fetch the Nomad autopilot docs if this pattern is pursued.

## Recommendation for the target design

These are **research recommendations**: what to validate, spike or investigate next. They are not design decisions, and each material choice still needs user approval through the DESIGN wave.

1. **Treat rolling upgrades (A4) as a gating criterion for the consensus library, and resolve it before any other `viewstamp` question.** Evaluate two concrete paths:
   - **(a) Range-negotiated wire versions plus a log-committed cluster-version entry.** Nodes advertise `[min, max]` (a fact), the leader commits `ClusterVersion(v)` only when all voters support `v` (a decision), and unsupported nodes are fenced. This is the KIP-584/778, CockroachDB and TigerBeetle pattern.
   - **(b) A seal-and-succeed virtual-log layer**, the Delos pattern, that makes the engine itself replaceable.

   A spike should include a *mixed-version* VOPR scenario, because Jepsen found three of TigerBeetle's twelve issues in upgrade code.
2. **Treat automatic voter churn as the riskiest path in `viewstamp`.** Before relying on automatic voter selection, ask for evidence that the simulator covers `AddLearner/PromoteLearner/DemoteVoter/RemoveLearner` under storage and network faults (upstream issue #68). Adopt KIP-853's rules: one change at a time, catch up before promotion, and replica incarnation/directory identity. Look at Autopilot's redundancy-zone model as the voter-placement policy.
3. **Design learner dissemination as pull-based relay plus hash-chain verification, with optional Plumtree push.** The leader feeds voters and a small relay set. Learners pull committed ranges from a nearby peer, verify them against the hash chain anchored by the leader-published commit head, and re-select their source on failure. Cassandra TCM's "epoch in every message → catch up from any peer" is the lag-detection mechanism to borrow. A simulation spike at 1k–5k learners should measure leader egress and end-to-end apply latency (Gap 2).
4. **Reads: use VR §6.3.2 op-number tokens for read-your-writes, and make linearizable decisions leader-side commands.** No lease or read-index is then required at learners (A3.4). If a linearizable *read API* is required later, prefer ReadIndex forwarding. Adopt a lease only by following a rigorously specified protocol (LeaseGuard) with clock drift injected in simulation.
5. **Keep one batched log for all intent. Revisit only on measured state size, learner fan-out or WAN latency, not write rate** (A6.1). Keep high-volume facts off the log; Kubernetes Events and Consul anti-entropy both show the cost of not doing so.
6. **Make the multi-region survival posture an explicit product decision** (zone survival with fast writes vs region survival at +1 inter-region RTT per write). Evaluate witness voters (VR §6.1, Spanner) for the region-survival posture. Evaluate partial-connectivity-resilient election (OmniPaxos' quorum-connected rule, or Raft-style PreVote/CheckQuorum) for a cross-region WireGuard mesh.
7. **Refine the working hypothesis** to: *facts are owner-authoritative; decisions are log-authoritative; delivery is transport-agnostic; facts that gate safety decisions also reach the leader over a lease/heartbeat channel, and the decision records the inputs it used* (H.3, H.4).
8. **Keep openraft as a live, evaluated fallback on the same criteria.** It wins today on reconfiguration maturity, read paths and bus factor. It loses on storage-fault design and Sans-I/O purity. A time-boxed comparison spike should implement the same learner-relay and version-gate prototypes against both libraries. The deciding factors are then the cost of adding protocol-aware recovery to openraft versus the cost of adding upgrades and relay to `viewstamp`.
9. **Commission an external correctness campaign** (Jepsen-style, or Antithesis) against whichever library is chosen. Scope it to the extensions the target adds: learners at scale, voter churn, upgrades and reads. The 2025–2026 evidence is that these are where every examined implementation broke (A5.3).

## Source Analysis

| Source (grouped) | Domain | Reputation | Type | Access Date | Cross-verified |
|---|---|---|---|---|---|
| VR Revisited (MIT TR 2012-021; HTML mirror) | dspace.mit.edu / ying-zhang.cn | High (mirror of MIT TR) | academic | 2026-09-25 | Y |
| Vive la Différence; Paxos vs Raft; Flexible Paxos; Compartmentalization; Tempo; EPaxos*; Bodega; Hermes; LeaseGuard; WPaxos; Shoal | arxiv.org | High | academic | 2026-09-25 | Y |
| EPaxos Revisited; MongoDB pull-based consensus; Delos; Scalog; Chain Replication; CRAQ; FAST'17; FAST'18 | usenix.org | High | academic | 2026-09-25 | Y |
| Narwhal & Tusk; Omni-Paxos; Paxos Quorum Leases; HAOC'21 Raft partial failures | dl.acm.org | High | academic | 2026-09-25 | Y |
| Flexible Paxos (LIPIcs) | drops.dagstuhl.de | High | academic | 2026-09-25 | Y |
| Epidemic Broadcast Trees (SRDS'07) | dpss.inesc-id.pt | High | academic | 2026-09-25 | Y |
| KIP-584, KIP-595, KIP-631, KIP-778, KIP-853; CEP-15; CEP-21; ZooKeeper Observers | apache.org | High | official | 2026-09-25 | Y |
| etcd learner design, downgrade, performance, limits; etcd x Antithesis blog | etcd.io | High | official | 2026-09-25 | Y |
| Kubernetes large clusters | kubernetes.io | High | official | 2026-09-25 | Y |
| Consul Autopilot; Consul anti-entropy | developer.hashicorp.com | High | official | 2026-09-25 | Partial |
| Spanner replication | cloud.google.com | High | official | 2026-09-25 | Y |
| TiKV Multi-Raft | tikv.org | High | official | 2026-09-25 | Y |
| docs.rs: viewstamp, openraft, iroh-gossip | docs.rs | High | technical_docs | 2026-09-25 | Y |
| Cloudflare "A Byzantine failure in the real world" | blog.cloudflare.com | High | industry | 2026-09-25 | Y |
| CNCF re-post of etcd Antithesis | cncf.io | High | official | 2026-09-25 | Y |
| al8n/viewstamp (README, commits, issues); lua-lunet/uvrr-core; TigerBeetle (vsr.md, ARCHITECTURE.md, replica.zig, PRs); openraft (README, changelog); consensus-essence; OmniPaxos (+fork); Solana Turbine; BIP-152; cassandra NEWS.txt; hashicorp/c2m; CometBFT changelog; cockroach RFC and issue | github.com | Medium-High | industry / OSS primary | 2026-09-25 | Y |
| TigerBeetle docs (upgrading) | docs.tigerbeetle.com | Medium-High (out-of-list primary) | technical_docs | 2026-09-25 | Y |
| Jepsen: TigerBeetle 0.16.11; Jepsen: NATS 2.12.1 | jepsen.io | Medium-High (out-of-list primary) | independent analysis | 2026-09-25 | Y |
| Antithesis: bugs in Raft implementations | antithesis.com | Medium-High (out-of-list primary; commercial interest) | industry | 2026-09-25 | Partial |
| CockroachDB docs (upgrade, survival goals); CockroachDB "Scaling Raft" blog | docs.cockroachlabs.com / cockroachlabs.com | Medium-High (out-of-list primary) | technical_docs | 2026-09-25 | Y |
| CometBFT ABCI++ requirements | docs.cometbft.com | Medium-High (out-of-list primary) | technical_docs | 2026-09-25 | Y |
| Restate architecture | docs.restate.dev | Medium-High (out-of-list primary; shared-log / node-role prior art only) | technical_docs | 2026-09-25 | Partial |
| HashiCorp C2M blog | hashicorp.com | Medium-High (out-of-list primary) | industry | 2026-09-25 | Partial (conflict 1) |
| MongoDB sync-target docs; Agave Turbine; Bitcoin Optech | mongodb.com / docs.anza.xyz / bitcoinops.org | Medium-High (out-of-list primary) | technical_docs | 2026-09-25 | Y |
| Charapko reading group; Davis review of PQL; Decentralized Thoughts; Vanlightly on Delos; freeCodeCamp VR summary; Instaclustr (Accord status) | charap.co / emptysqua.re / decentralizedthoughts.github.io / jack-vanlightly.com / freecodecamp.org / instaclustr.com | Medium | secondary commentary | 2026-09-25 | Used only as corroboration |

Reputation (approximate, by citation count): High ≈ 55% | Medium-high ≈ 38% | Medium ≈ 7% | **Average ≈ 0.90**. No excluded domains were used. Medium sources never stand alone for a claim.

## Full Citations

[1] Liskov, B.; Cowling, J. "Viewstamped Replication Revisited". MIT-CSAIL-TR-2012-021. 2012-07-23. https://dspace.mit.edu/handle/1721.1/71763 (text via mirror http://ying-zhang.cn/dist/2012-vr.html). Accessed 2026-09-25.
[2] al8n. "viewstamp" repository README, commits, issues. GitHub. 2026. https://github.com/al8n/viewstamp. Accessed 2026-09-25.
[3] docs.rs. "viewstamp 0.0.0". 2026-09-18. https://docs.rs/viewstamp/latest/viewstamp/. Accessed 2026-09-25.
[4] lua-lunet. "uvrr-core". GitHub. https://github.com/lua-lunet/uvrr-core. Accessed 2026-09-25.
[5] TigerBeetle. "VSR" internals doc. GitHub. https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/vsr.md. Accessed 2026-09-25.
[6] TigerBeetle. "ARCHITECTURE.md". GitHub. https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/ARCHITECTURE.md and https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/ARCHITECTURE.md. Accessed 2026-09-25.
[7] TigerBeetle. "src/vsr/replica.zig". GitHub. https://github.com/tigerbeetle/tigerbeetle/blob/main/src/vsr/replica.zig. Accessed 2026-09-25.
[8] TigerBeetle. "Upgrading". https://docs.tigerbeetle.com/operating/upgrading/. Accessed 2026-09-25.
[9] TigerBeetle. PR #2635 "replica: add in star replication and closed loop replication" (merged 2025-01-07). https://github.com/tigerbeetle/tigerbeetle/pull/2635. Accessed 2026-09-25.
[10] Kingsbury, K. "Jepsen: TigerBeetle 0.16.11". 2025-06-06. https://jepsen.io/analyses/tigerbeetle-0.16.11. Accessed 2026-09-25.
[11] Van Renesse, R.; Schiper, N.; Schneider, F. B. "Vive la Différence: Paxos vs. Viewstamped Replication vs. Zab". arXiv:1309.5671. 2013/2014. https://arxiv.org/abs/1309.5671. Accessed 2026-09-25.
[12] Howard, H.; Mortier, R. "Paxos vs Raft: Have we reached consensus on distributed consensus?". arXiv:2004.05074. 2020. https://arxiv.org/abs/2004.05074. Accessed 2026-09-25.
[13] Charapko, A. "Reading Group. Viewstamped Replication Revisited". https://charap.co/reading-group-viewstamped-replication-revisited/. Accessed 2026-09-25.
[14] Databend Labs. "openraft" README and change-log. GitHub. https://github.com/databendlabs/openraft. Accessed 2026-09-25.
[15] docs.rs. "openraft 0.9.25". 2026-07-28. https://docs.rs/openraft/latest/openraft/. Accessed 2026-09-25.
[16] drmingdrmer. "Raft ReadIndex" (consensus-essence). GitHub. https://github.com/drmingdrmer/consensus-essence/blob/main/src/list/raft-read-index/raft-read-index.md. Accessed 2026-09-25.
[17] rjwalters/vibesql. Issue #5389 "lease fast path ... openraft 0.10 ReadPolicy::LeaseRead". https://github.com/rjwalters/vibesql/issues/5389. Accessed 2026-09-25.
[18] Apache ZooKeeper. "ZooKeeper Observers". https://zookeeper.apache.org/doc/current/zookeeperObservers.html. Accessed 2026-09-25.
[19] etcd. "Learner" design. https://etcd.io/docs/v3.6/learning/design-learner/. Accessed 2026-09-25.
[20] Apache Kafka. "KIP-595: A Raft Protocol for the Metadata Quorum". https://cwiki.apache.org/confluence/display/KAFKA/KIP-595%3A+A+Raft+Protocol+for+the+Metadata+Quorum. Accessed 2026-09-25.
[21] Zhou, S.; Mu, S. et al. "Fault-Tolerant Replication with Pull-Based Consensus in MongoDB". NSDI'21. https://www.usenix.org/conference/nsdi21/presentation/zhou. Accessed 2026-09-25.
[22] Leitão, J.; Pereira, J.; Rodrigues, L. "Epidemic Broadcast Trees". SRDS 2007. https://www.dpss.inesc-id.pt/~ler/reports/srds07.pdf. Accessed 2026-09-25.
[23] n0. "iroh-gossip 0.101.0". docs.rs. 2026-09-07. https://docs.rs/iroh-gossip/latest/iroh_gossip/. Accessed 2026-09-25.
[24] Solana Labs. "Turbine Block Propagation". GitHub. https://github.com/solana-labs/solana/blob/master/docs/src/consensus/turbine-block-propagation.md. Accessed 2026-09-25.
[25] Corallo, M. "BIP 152: Compact Block Relay". GitHub. https://github.com/bitcoin/bips/blob/master/bip-0152.mediawiki. Accessed 2026-09-25.
[26] van Renesse, R.; Schneider, F. B. "Chain Replication for Supporting High Throughput and Availability". OSDI'04. https://www.usenix.org/conference/osdi-04/chain-replication-supporting-high-throughput-and-availability. Accessed 2026-09-25.
[27] Terrace, J.; Freedman, M. J. "Object Storage on CRAQ". USENIX ATC'09. https://www.usenix.org/conference/usenix-09/object-storage-craq-high-throughput-chain-replication-read-mostly-workloads. Accessed 2026-09-25.
[28] Tollman, S.; Park, S. J.; Ousterhout, J. "EPaxos Revisited". NSDI'21. https://www.usenix.org/conference/nsdi21/presentation/tollman. Accessed 2026-09-25.
[29] Ryabinin, F.; Gotsman, A.; Sutra, P. "Fixing and Simplifying Egalitarian Paxos". arXiv:2511.02743 (OPODIS'25). https://arxiv.org/abs/2511.02743. Accessed 2026-09-25.
[30] Enes, V. et al. "Efficient Replication via Timestamp Stability" (Tempo). EuroSys'21, arXiv:2104.01142. https://arxiv.org/abs/2104.01142. Accessed 2026-09-25.
[31] Enes, V. et al. "State-Machine Replication for Planet-Scale Systems" (Atlas). EuroSys'20. https://software.imdea.org/~gotsman/papers/atlas-eurosys20.pdf. Accessed 2026-09-25.
[32] Apache Cassandra. "CEP-15: General Purpose Transactions". https://cwiki.apache.org/confluence/display/CASSANDRA/CEP-15:+General+Purpose+Transactions. Accessed 2026-09-25.
[33] Howard, H.; Malkhi, D.; Spiegelman, A. "Flexible Paxos: Quorum Intersection Revisited". OPODIS 2016, arXiv:1608.06696. https://arxiv.org/abs/1608.06696. Accessed 2026-09-25.
[34] Danezis, G. et al. "Narwhal and Tusk". EuroSys'22. https://dl.acm.org/doi/10.1145/3492321.3519594. Accessed 2026-09-25.
[35] Spiegelman, A. et al. "Shoal: Improving DAG-BFT Latency And Robustness". arXiv:2306.03058. https://arxiv.org/pdf/2306.03058. Accessed 2026-09-25.
[36] Ding, C. et al. "Scalog". NSDI'20. https://www.usenix.org/conference/nsdi20/presentation/ding. Accessed 2026-09-25.
[37] Balakrishnan, M. et al. "Virtual Consensus in Delos". OSDI'20. https://www.usenix.org/conference/osdi20/presentation/balakrishnan. Accessed 2026-09-25.
[38] Restate. "Architecture". https://docs.restate.dev/references/architecture. Accessed 2026-09-25.
[39] Whittaker, M. et al. "Scaling Replicated State Machines with Compartmentalization". PVLDB 14(11), arXiv:2012.15762. https://arxiv.org/abs/2012.15762. Accessed 2026-09-25.
[40] Ng, H.; Haridi, S.; Carbone, P. "Omni-Paxos: Breaking the Barriers of Partial Connectivity". EuroSys'23. https://dl.acm.org/doi/abs/10.1145/3552326.3587441; code https://github.com/haraldng/omnipaxos. Accessed 2026-09-25.
[41] Cloudflare. "A Byzantine failure in the real world". 2020-11-27. https://blog.cloudflare.com/a-byzantine-failure-in-the-real-world/. Accessed 2026-09-25.
[42] "Examining Raft's behaviour during partial network failures". HAOC'21. https://dl.acm.org/doi/abs/10.1145/3447851.3458739. Accessed 2026-09-25.
[43] CometBFT. "Requirements for the Application (ABCI++)" v0.38. https://docs.cometbft.com/v0.38/spec/abci/abci++_app_requirements. Accessed 2026-09-25.
[44] Moraru, I.; Andersen, D. G.; Kaminsky, M. "Paxos Quorum Leases". SoCC'14. https://dl.acm.org/doi/10.1145/2670979.2671001 (mechanism via https://emptysqua.re/blog/review-paxos-quorum-leases/). Accessed 2026-09-25.
[45] Hu, G.; Arpaci-Dusseau, A.; Arpaci-Dusseau, R. "Bodega: Serving Linearizable Reads Locally from Anywhere at Anytime via Roster Leases". arXiv:2509.07158. 2025. https://arxiv.org/abs/2509.07158. Accessed 2026-09-25.
[46] Katsarakis, A. et al. "Hermes: a Fast, Fault-Tolerant and Linearizable Replication Protocol". ASPLOS'20, arXiv:2001.09804. https://arxiv.org/abs/2001.09804. Accessed 2026-09-25.
[47] Davis, A. J. J.; Demirbas, M.; Deng, L. "LeaseGuard: Raft Leases Done Right". arXiv:2512.15659 (SIGMOD'26). https://arxiv.org/abs/2512.15659. Accessed 2026-09-25.
[48] Apache Kafka. "KIP-584: Versioning scheme for features". https://cwiki.apache.org/confluence/display/KAFKA/KIP-584%3A+Versioning+scheme+for+features. Accessed 2026-09-25.
[49] Apache Kafka. "KIP-778: KRaft to KRaft Upgrades". https://cwiki.apache.org/confluence/display/KAFKA/KIP-778%3A+KRaft+to+KRaft+Upgrades. Accessed 2026-09-25.
[50] Apache Kafka. "KIP-853: KRaft Controller Membership Changes". https://cwiki.apache.org/confluence/display/KAFKA/KIP-853%3A+KRaft+Controller+Membership+Changes. Accessed 2026-09-25.
[51] Apache Kafka. "KIP-631: The Quorum-based Kafka Controller". https://cwiki.apache.org/confluence/display/KAFKA/KIP-631%3A+The+Quorum-based+Kafka+Controller. Accessed 2026-09-25.
[52] Cockroach Labs. "Upgrade CockroachDB". https://docs.cockroachlabs.com/docs/stable/upgrade-cockroach-version. Accessed 2026-09-25.
[53] etcd. "Downgrade etcd from v3.6 to v3.5". https://etcd.io/docs/v3.6/downgrades/downgrade_3_6/. Accessed 2026-09-25.
[54] HashiCorp. "Consul Autopilot". https://developer.hashicorp.com/consul/docs/manage/scale/autopilot. Accessed 2026-09-25.
[55] HashiCorp. "Consul consistency / anti-entropy". https://developer.hashicorp.com/consul/docs/concept/consistency. Accessed 2026-09-25.
[56] Ganesan, A. et al. "Redundancy Does Not Imply Fault Tolerance". FAST'17. https://www.usenix.org/conference/fast17/technical-sessions/presentation/ganesan. Accessed 2026-09-25.
[57] Alagappan, R. et al. "Protocol-Aware Recovery for Consensus-Based Storage". FAST'18. https://www.usenix.org/conference/fast18/presentation/alagappan. Accessed 2026-09-25.
[58] Kingsbury, K. "Jepsen: NATS 2.12.1". 2025-12-08. https://jepsen.io/analyses/nats-2.12.1. Accessed 2026-09-25.
[59] Lim, T. W.; Padhye, R.; Primi, M. "Finding bugs in Raft implementations". Antithesis. 2026-07-27. https://antithesis.com/blog/2026/finding-bugs-in-raft-implementations/. Accessed 2026-09-25.
[60] etcd. "Autonomous Testing of etcd's Robustness". 2025. https://etcd.io/blog/2025/autonomus_testing_with_antithesis/. Accessed 2026-09-25.
[61] etcd. "Performance". https://etcd.io/docs/v3.6/op-guide/performance/. Accessed 2026-09-25.
[62] etcd. "System limits". https://etcd.io/docs/v3.6/dev-guide/limit/. Accessed 2026-09-25.
[63] Kubernetes. "Considerations for large clusters". https://kubernetes.io/docs/setup/best-practices/cluster-large/. Accessed 2026-09-25.
[64] HashiCorp. "HashiCorp Nomad Meets the 2 Million Container Challenge". 2020-12-08. https://www.hashicorp.com/en/blog/hashicorp-nomad-meets-the-2-million-container-challenge; repo https://github.com/hashicorp/c2m. Accessed 2026-09-25.
[65] TiKV. "Multi-Raft". https://tikv.org/deep-dive/scalability/multi-raft/. Accessed 2026-09-25.
[66] Cockroach Labs. "Scaling Raft". https://www.cockroachlabs.com/blog/scaling-raft/; RFC https://github.com/cockroachdb/cockroach/blob/master/docs/RFCS/20160824_quiesce_ranges.md. Accessed 2026-09-25.
[67] Apache Cassandra. "CEP-21: Transactional Cluster Metadata". https://cwiki.apache.org/confluence/display/CASSANDRA/CEP-21%3A+Transactional+Cluster+Metadata; NEWS.txt https://github.com/apache/cassandra/blob/trunk/NEWS.txt. Accessed 2026-09-25.
[68] Google Cloud. "Spanner replication". https://docs.cloud.google.com/spanner/docs/replication. Accessed 2026-09-25.
[69] Cockroach Labs. "Multi-region survival goals". https://docs.cockroachlabs.com/docs/stable/multiregion-survival-goals. Accessed 2026-09-25.
[70] Ailijiang, A.; Charapko, A.; Demirbas, M.; Kosar, T. "WPaxos: Wide Area Network Flexible Consensus". arXiv:1703.08905. https://arxiv.org/abs/1703.08905. Accessed 2026-09-25.

## Research Metadata

Duration: single session (~70 tool turns) | Examined: ~95 sources | Cited: 70 citation entries (~85 distinct URLs) | Cross-refs: every Finding carries a Verification line; 6 findings rest partly on interpretation and are labelled as such | Confidence distribution across the 44 findings: High ≈ 55%, Medium-High/Medium ≈ 39%, Low-Medium ≈ 5% | Tool failures: MIT VR PDF (ECONNREFUSED / HTTP 405), web.archive.org blocked, ACM DL abstracts (HTTP 403) for Omni-Paxos and PQL, openraft 0.10 docs.rs page (404), GitHub contributors graph (load error), theconsensus.dev (403) | Output: `docs/research/architecture/flat-cluster-consensus-and-log-dissemination-comprehensive-research.md`
