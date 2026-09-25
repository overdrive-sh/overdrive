# Research: Delos-Style Virtual Log (Virtual Consensus, Sealed Segments) for a Flat-Cluster Orchestrator's Intent Log

**Date**: 2026-09-25 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium-High overall (High on Delos mechanism and production data, which come from two peer-reviewed first-party papers; Medium on sibling-system edge behaviour and on `viewstamp` fit, which rests on its README) | **Sources**: 29 cited external sources, average reputation ≈ 0.84

**Scope**: Whether a Delos-style virtual log is worth adopting for the target design's intent log. This researches a *target* design. It does not evaluate the current codebase (`crates/` was not read), and `docs/whitepaper.md` is not treated as a source of truth.

## Executive Summary

**What Delos is.** Delos (Meta, OSDI'20 and SOSP'21) splits consensus into two layers:
- a **VirtualLog**, a reconfiguration layer that chains log segments ("Loglets"). Only the tail segment accepts appends.
- a **MetaStore**, a single versioned register with compare-and-swap, touched only when the chain changes.

Reconfiguration is *seal the active Loglet → read its tail → CAS the new chain → fetch*. A Loglet need not be fault-tolerant; it only needs a quorum-settable seal bit. The NativeLoglet was built in under four months.

**Production record.** Delos backed Meta's Twine cluster manager (Resource Broker and scheduler). It swapped its ordering protocol live (ZooKeeper-backed → NativeLoglet, 2 April 2019) and once *rolled back* live to work around a NativeLoglet bug. By 2021 it ran on 262 clusters (107 DelosTable, 155 Zelos) of typically 5–7 nodes, handling tens of billions of operations per day. Costs:
- virtualization adds **100–150 µs p99 per append**;
- reconfigurations take **tens of milliseconds** inside one datacenter;
- most reconfigurations come from routine software deploys (98 in one month on one cluster).

**Siblings.** The same three parts appear in every shipped virtual/shared log:
- a per-segment fence;
- a CAS'd chain in a small consensus store;
- immutable sealed segments, tiered to cheap storage.

The systems examined are LogDevice (epochs; archived 2022), BookKeeper/Pulsar (ledger fence + close; managed-ledger list in ZooKeeper or Oxia), CORFU (epoch seal + auxiliary), Pravega (sealed stream segments), Restate Bifrost (Rust; Delos + LogDevice; metadata in Raft, etcd or S3; **BSL 1.1**) and Confluent's Conflux/LogDrive (OSDI'26; a VirtualLog over cloud storage, in production).

**The central finding for rolling upgrades: a virtual log is a complement to a committed cluster-version gate, never a substitute.** Upgrades touch three planes:
- **(P1) Ordering-protocol wire/storage.** A virtual log removes the need for old and new protocol *peers* to interoperate, which is its real value for flag-day `viewstamp`. It still requires every node to run a dual-capable binary before the switch.
- **(P2) Entry semantics.** Sealing changes nothing here: an old-version reader cannot interpret new-format entries in any segment. Delos itself used a **log-committed enable flag** (new code rolled out dormant, then activated "via the log itself"), and ad-hoc P2 roll-outs were "the only source of inconsistency in production".
- **(P3) The meta-store's own format.** It must be frozen (Delos/CORFU) or migrated with downtime (Restate pauses processing).

So the cluster-version gate is mandatory in every design. The virtual log adds live protocol replacement, live rollback, surgical log repair, and sealed-segment tiering.

**Multi-region.** A virtual log chains segments in *time*, not space. It cannot give simultaneous per-region write locality to one totally ordered log. Only FuzzyLog does that, by giving up the total order. Meta ran many small Delos clusters and one geo-spread quorum with leases, not per-region segments.

**Recommendation (research, not a decision).** Adopt the committed cluster-version gate as the baseline. Do **not** adopt a full heterogeneous VirtualLog with a separate MetaStore now. Instead, evaluate an **epoch-segmented VR log** ("Delos-lite"):
- VR §7 epochs are the segments, and the successor record is committed in-band as each epoch's last op (the seal);
- each epoch is pinned to one wire version;
- each epoch hands off through a content-addressed boundary checkpoint, not cross-version messages;
- sealed epochs are immutable and hash-verified, so any peer or object storage can serve them to learners.

This captures most of the upgrade and dissemination value at a fraction of the cost, and it leaves room for a full virtual log later.

## Research Methodology

**Search Strategy**:
- Read both Delos papers in full from primary PDFs (usenix.org; the author-hosted SOSP'21 copy, because the ACM PDF returned 403).
- Followed the lineage (CORFU, Scalog, FuzzyLog, LogDevice) and the shipped siblings (BookKeeper/Pulsar, Pravega, Restate Bifrost, Confluent Conflux/LogDrive) through official documentation and papers.
- Re-read VR Revisited §7 and the `viewstamp` README for fit.
- Searched for production failure reports in shipped virtual logs.

**Source Selection**: Types: academic (usenix.org, ACM), official/open-source docs (apache.org), project-primary docs outside the trusted list (docs.restate.dev, logdevice.io, pravega.io, atscaleconference.com; marked out-of-list primary, medium-high), and practitioner commentary (jack-vanlightly.com, micahlerner.com; medium, used for corroboration only). Reputation: medium-high minimum for load-bearing claims. Verification: independent cross-reference per claim, with *lineage dependence* flagged. Many papers share Mahesh Balakrishnan as author, and are treated as one research programme, not independent confirmations.
**Quality Standards**: Target 3 sources per claim (minimum 1 authoritative). Interpretation is labelled "Analysis (interpretation)". Average reputation ≈ 0.84.
**Out of scope by instruction**: `crates/` and `docs/whitepaper.md` were not read. Restate appears only as shared-log/virtual-log prior art (Bifrost). No durable-execution or workflow feature of Restate was researched or used. Temporal is not used.

## Target design under examination (restated)

- **Topology**: flat, no single/HA mode split. Every node is HA by default; the cluster grows by joining a node, from one node to thousands, possibly across regions. It is a single-binary Rust orchestrator (microVM workloads, eBPF dataplane).
- **Intent/decisions** go on a replicated log: workload specs, placement, admission, roles, voter set, node-down decisions, cluster version. The leading candidate is VR (VR Revisited 2012 / TigerBeetle VSR / the `viewstamp` crate: Sans-I/O, storage-fault model, SingleChange reconfiguration, learners, content-addressed snapshots). The alternative is openraft.
- **Replication**: 3–5 voters order the log. Every other node is a learner holding the full log, possibly fed by pull-based chained replication with hash-chain verification.
- **Known gap**: `viewstamp` upgrades are flag-day (mixed versions rejected at connect), and reconfiguration is not yet covered by its simulator.
- **Trust model**: single operator, trusted nodes, crash faults only (CFT).

## Relation to prior research

This document builds on, and does not repeat:
- `flat-cluster-consensus-and-log-dissemination-comprehensive-research.md`: VR vs Raft, `viewstamp` state, the cluster-version-gate pattern (A4.2–A4.3), voter rotation (A4.4), and the first mention of Delos (A1.7, A4.5);
- `flat-cluster-membership-discovery-and-admission-comprehensive-research.md`: Autopilot-style voter management;
- `flat-cluster-placement-and-observation-comprehensive-research.md`: replicated decisions vs replicated code, and hierarchy beyond a few thousand nodes.

**One correction to prior research.** Finding A4.5's analysis presented the virtual log as an *alternative* path, "(b)", to the version gate. It suggested that old-version replicas finish the sealed segment while "learners replay across segments", with a pause "comparable to TigerBeetle's roughly 5 seconds". The primary papers show:
- (1) it is a *complement*: Delos used a log-committed flag for semantic changes (Finding 1.7 / 3.2);
- (2) old-binary learners cannot read a new-protocol or new-format segment (Findings 3.1–3.2);
- (3) the measured seal pause is tens of milliseconds within one datacenter (Finding 1.5).

See Conflicting information, Conflict 1.

## Q1. Delos itself

### Finding 1.1: The VirtualLog is a reconfiguration layer that chains Loglets. Only the tail segment is appendable. The Loglet interface is five calls, and `seal` is the only one that must be fault-tolerant

**Evidence**:
- Split of responsibilities: Delos "splits the logic of consensus into the VirtualLog, a generic and reusable reconfiguration layer; and pluggable ordering protocols called Loglets" (Abstract).
- Loglet API (Fig. 2): `append(Entry)`, `checkTail()` returning `pair<logpos_t,bool>`, `readNext(min,max)`, `prefixTrim(pos)` and `seal()`.
- VirtualLog additions: `reconfigExtend`, `reconfigTruncate` and `reconfigModify`.
- Segments: "only the last log in the chain is appendable (we call this the active segment); the other logs are sealed and return errors on appends (we call these sealed segments)" (§3.1).
- Division of labour: "consensus in the VirtualLog is simple and fault-tolerant (but not necessarily fast, since it is invoked only on reconfigurations), while consensus in the Loglet is simple and fast (but not necessarily fault-tolerant)" (§3).
- Weakness of seal: "A seal bit does not require fault-tolerant consensus ... It can be implemented via a fault-tolerant atomic register ... weaker than consensus and not subject to the FLP impossibility result" (§3.4).

**Source**: [Balakrishnan et al., "Virtual Consensus in Delos", OSDI'20](https://www.usenix.org/system/files/osdi20-balakrishnan.pdf) (usenix.org, high). Accessed 2026-09-25.
**Confidence**: High
**Verification**: [Balakrishnan et al., "Log-structured Protocols in Delos", SOSP'21](https://maheshba.bitbucket.io/papers/delos-sosp2021.pdf), §4: "The BaseEngine runs over the VirtualLog [9], which implements fault-tolerant consensus" (author-hosted copy of the [ACM DL paper](https://dl.acm.org/doi/10.1145/3477132.3483544), high). [Vanlightly, "An Introduction to Virtual Consensus in Delos"](https://jack-vanlightly.com/blog/2025/2/5/an-introduction-to-virtual-consensus-in-delos) gives an independent practitioner reading (medium): "Consensus is separated into redundancy (Loglets) and fault-tolerance (VirtualLog)". His follow-up, ["Steady on!"](https://jack-vanlightly.com/blog/2025/2/6/steady-on-separating-failure-free-ordering-from-fault-tolerant-consensus), frames the same split as "failure-free ordering" vs "fault-tolerant consensus".
**Analysis**: The design moves fault tolerance *out of* the ordering protocol. A Loglet may be a static configuration with a single sequencer and no leader election, as long as it can be sealed by a quorum. For the target design this means a VR group is overqualified as a Loglet: it already does its own view change. That is allowed (Delos ran ZooKeeper and LogDevice as Loglets), but it makes the VirtualLog a *second* reconfiguration mechanism layered over VR's own.

### Finding 1.2: The MetaStore is a single versioned register with conditional write. Delos first put it in ZooKeeper, then embedded it as an unoptimised single-slot Paxos chain

**Evidence**:
- API: "The MetaStore component has a simple API: it is a single versioned register supporting a conditional write. Reading the MetaStore returns a value with an attached version. Writing to it requires supplying a new value and an expected existing version" (§3.2).
- Role: "The VirtualLog MetaStore is a necessary and sufficient source of fault-tolerant consensus in our architecture ... it requires a fault-tolerant consensus protocol like Paxos ... not required to be particularly fast, since it is accessed by the VirtualLog only during reconfigurations" (§3.3).
- Why not in-band: storing the next configuration inside the current log "requires the Loglet itself to be highly available for writes (i.e., implement fault-tolerant consensus) ... With a separate MetaStore, we eliminate the requirement of fault-tolerant consensus for each Loglet" (§3.3).
- Implementation history: "Initially, Delos went to production with the MetaStore residing on an external ZooKeeper service as a single key/value pair. Later, to remove this external dependency, we implemented an embedded MetaStore that runs on the same set of Delos servers ... we used Lamport's construction of a replicated state machine from the original Paxos paper ... a simple, unoptimized implementation of canonical single-slot Paxos, incurring two round-trips to a quorum for both writes and reads ... Each Paxos instance stores the membership of the next instance" (§4.1).

**Source**: [Virtual Consensus in Delos, OSDI'20](https://www.usenix.org/system/files/osdi20-balakrishnan.pdf) (usenix.org, high). Accessed 2026-09-25.
**Confidence**: High (single authoritative primary; peer-reviewed; first-party)
**Verification**: [Vanlightly, Delos introduction](https://jack-vanlightly.com/blog/2025/2/5/an-introduction-to-virtual-consensus-in-delos) (medium) describes the same register-with-conditional-write MetaStore. Sibling systems use the same shape (see Q4: LogDevice epoch store, BookKeeper ledger metadata, Bifrost `Logs` in the metadata store).
**Analysis**: The meta-consensus is deliberately tiny: one value, compare-and-set, touched only at reconfiguration. The regress ("what reconfigures the MetaStore?") is ended by making the MetaStore *self-reconfiguring*: each Paxos slot names the membership of the next slot. That is Lamport's original "α = 1" reconfiguration, which is simple because throughput does not matter.

### Finding 1.3: Reconfiguration is seal → checkTail → conditional write of the new chain → fetch. Any client can trigger it. An interrupted reconfiguration is rolled forward by *cloning the old segment's configuration*

**Evidence**:
- Steps: "Reconfiguration involves three steps: sealing the old chain, installing the new chain on the MetaStore, and fetching the new chain from the MetaStore ... After sealing the active segment, the client calls checkTail to retrieve its tail; this determines the start of the new active segment" (§3.2).
- Races: the MetaStore "only accepts the new chain Ci+1 if the existing chain is Ci ... multiple reconfiguring clients ... can race to install the new chain ... with at most one guaranteed to win" (§3.2).
- Roll-forward: "after a time-out period, the client 'rolls forward' the reconfiguration by installing its own new chain. Note that the client completing the reconfiguration does not know the original intention of the failed client (e.g., if it was reconfiguring to a different Loglet type); hence, it creates a default new chain by cloning the configuration of the previous active segment" (§3.2).
- Policy drivers: "planned reconfigurations (e.g., upgrading to a faster Loglet) are driven via a command line tool by operators"; trims call `reconfigTruncate`; and Loglets that lack leader election detect failures and request `reconfigExtend` (§3.2).
- Sealing is needed only on conflict: "an old chain only has to be sealed if it conflicts with a newer chain" (§3.2).
- Zombie appends: "calling checkTail on a sealed log can return increasing values for the tail position even after the log is successfully sealed. These 'zombie' appends ... do not appear on the VirtualLog's address space" (§3.4).

**Source**: [Virtual Consensus in Delos, OSDI'20](https://www.usenix.org/system/files/osdi20-balakrishnan.pdf) (usenix.org, high). Accessed 2026-09-25.
**Confidence**: High
**Verification**: The same seal-then-publish sequence appears independently in [Restate Bifrost](https://docs.restate.dev/references/architecture) ("reconfiguration seals the active segment and atomically publishes a new segment as the head") and in [LogDevice's sealing](https://logdevice.io/docs/Concepts.html) (see Q2).
**Analysis**: The roll-forward rule matters for upgrades. If an operator's "switch to Loglet v2" reconfiguration stalls after the seal, another client completes it *with the old v1 configuration*. The system stays live but silently does not upgrade. An upgrade driver therefore has to check the outcome and retry. In an orchestrator this would be a reconciler-style convergence ("desired Loglet version vs installed chain"), not a fire-and-forget command.

### Finding 1.4: NativeLoglet is a primary-driven protocol with a quorum-sealable bit and no leader election. It took under 4 months to build

**Evidence**:
- Availability: "The NativeLoglet is available for seal and checkTail as long as a majority of LogServers are alive; and for append if the sequencer is also alive. Each LogServer stores a local on-disk log, along with a seal bit; once the seal bit is set, the LogServer rejects new appends" (§4.2.1).
- Effort: "we found this protocol much easier to implement than fault-tolerant consensus: it took just under 4 months to implement and deploy a production-quality NativeLoglet" (§4.2.1).
- Failure detection: "a combination of in-band detection ... and out-of-band signals (via a gossip-based failure detector, as well as information from the container manager) to trigger reconfiguration" (§4.2.1).
- The other Loglets (Fig. 5):

  | Loglet | Consensus | Deployment | In production | Use |
  |---|---|---|---|---|
  | ZK | yes | disaggregated | yes | bootstrap |
  | Native | no | converged or disaggregated | yes | primary |
  | Backup | yes | disaggregated | yes | backup |
  | LogDevice | yes | disaggregated | no | performance |
  | Striped | no | converged or disaggregated | no | performance |

  StripedLoglet is "a shim layer with only around 300 lines of code" and "has to be sealed as a whole" (§4.2.2).

**Source**: [Virtual Consensus in Delos, OSDI'20](https://www.usenix.org/system/files/osdi20-balakrishnan.pdf) (usenix.org, high). Accessed 2026-09-25.
**Confidence**: High
**Verification**: [Vanlightly, Delos introduction](https://jack-vanlightly.com/blog/2025/2/5/an-introduction-to-virtual-consensus-in-delos) (medium): "The NativeLoglet ... uses quorum-based replication and a quorum-based seal mechanism–once a quorum of storage servers of the loglet have set the seal bit, the seal operation is complete". The [Meta At Scale talk abstract](https://atscaleconference.com/2021/03/15/virtualizing-consensus/) (first-party, out-of-list primary) confirms the ZooKeeper → NativeLoglet switch.
**Analysis**: The NativeLoglet is essentially chain-free primary-backup whose *only* fault-tolerant operation is a quorum-set sticky bit. Delos's gossip-based failure detector triggering `reconfigExtend` is the same "facts from gossip, decisions through the authority" split the prior research recommended (consensus doc, Executive Summary).

### Finding 1.5: Production evidence: the consensus protocol was swapped live (ZKLoglet → NativeLoglet, 2 April 2019). Virtualization adds 100–150 µs p99 per append, and reconfigurations take tens of milliseconds, most of them caused by software deploys

**Evidence**:
- Timeline: "Delos reached production within 8 months, and 4 months later upgraded its consensus protocol without downtime for a 10X latency improvement" (Abstract).
- Scale in 2020: "in production for over 18 months and currently processes over 1.8 billion transactions per day across all our deployments. One of its use cases is Twine's Resource Broker [44], which stores metadata for the fleet of servers in Facebook; each Delos deployment runs on 5 to 9 machines" (§1).
- The switch: "the actual switch-over happening from ZKLoglet to a converged NativeLoglet for the first time on a Delos production instance on April 2nd 2019 ... 10X improvements for multi-gets and indexed queries, and a 5X improvement for multi-puts ... p99 latency spikes for indexed queries during reconfiguration, but otherwise service availability is not disrupted. The latency improvement is largely due to the unoptimized nature of our ZKLoglet implementation" (§5.1). The switch consisted of a `reconfigExtend` (seal ZKLoglet) followed minutes later by a non-sealing `reconfigTruncate`.
- Cost: "For append and checkTail, virtualization adds 100-150 µseconds to p99 latency ... readNext ... adds only a few µseconds" (Fig. 11: "6 µs for readNext").
- Reconfigurations: "Reconfigurations occur within 10s of ms ... The vast majority of these reconfigurations are triggered by 1) continuous deployment of software upgrades; and 2) machine preemptions for hardware maintenance, kernel upgrades, etc. Actual failures constitute a small percentage ... one of our production clusters was reconfigured 98 times in the 1-month period" (§5.2).
- Production workload: "425 queries/sec and 150 puts/sec ... Each deployment stores between 1GB and 10GB" (§5).
- Operational use: "we discovered latent bugs in the NativeLoglet; we reconfigured to ZKLoglet, rolled out hotfixes, and then reconfigured back"; and "a 'poison' entry caused hangs on all learners processing the log", which was removed by "changing the metadata of the VirtualLog" (§6).

**Source**: [Virtual Consensus in Delos, OSDI'20](https://www.usenix.org/system/files/osdi20-balakrishnan.pdf) (usenix.org, high). Accessed 2026-09-25.
**Confidence**: High (peer-reviewed, first-party production data; the independent corroboration below covers the headline claims only)
**Verification**: [ACM DL / USENIX abstract](https://www.usenix.org/conference/osdi20/presentation/balakrishnan); [Meta At Scale talk](https://atscaleconference.com/2021/03/15/virtualizing-consensus/); [Vanlightly](https://jack-vanlightly.com/blog/2025/2/5/an-introduction-to-virtual-consensus-in-delos).
**Analysis**: Three points temper the headline. (1) The 10X was mostly an artefact of a deliberately naive ZKLoglet, so the result demonstrates *swappability*, not a faster protocol. (2) A reconfiguration costs tens of milliseconds *within one datacenter*. The paper says cross-region may need in-band planned reconfiguration (Finding 1.6). (3) Delos turns every software deploy into a VirtualLog reconfiguration, because the NativeLoglet has no leader election, so a sequencer restart requires a seal. Reconfiguration is therefore the *hot* path of operations, which is why it must be cheap and well tested. The rollback-to-ZKLoglet episode is the strongest evidence of real value: a live protocol *rollback*, which a committed version gate alone does not give.

### Finding 1.6: The OSDI paper names geo-distribution as unsolved and expects planned cross-region reconfiguration to move in-band

**Evidence**: "In our current setting (control plane applications running within a single data center), reconfiguration latencies of 10s of ms are tenable. If reconfiguration is driven by failure, the latency of failure detection is typically multiple seconds in any case ... In the future, when we run across regions, it may be important to optimize for planned reconfiguration (e.g., replacing servers); since the Loglet is still available in this case, we can potentially reconfigure by storing inline commands within the Loglet itself, borrowing existing techniques such as α-windows" (§3.3). Stated limitation: "The reusability of VirtualLog-driven reconfiguration comes with a latency hit for certain types of reconfigurations such as planned leader changes" (§1).

**Source**: [Virtual Consensus in Delos, OSDI'20](https://www.usenix.org/system/files/osdi20-balakrishnan.pdf) (usenix.org, high). Accessed 2026-09-25.
**Confidence**: High (for what the paper says). Low for any claim about what Meta later built; see Knowledge Gaps.
**Verification**: The SOSP'21 paper reports a "geo-distributed DelosTable cluster with 5 servers distributed across the continental USA (this is a common deployment mode for us when disaster tolerance is required)", with quorum reads at "a p99 latency of roughly 48ms" ([SOSP'21 §5.2](https://maheshba.bitbucket.io/papers/delos-sosp2021.pdf)). So Delos did run cross-region, but with one quorum spanning regions and no per-region segments.
**Analysis**: Nothing in either paper supports "per-region segments" as a Delos feature. Delos's geo story is a single 5-node quorum spread across a continent plus leases (LeaseEngine) for local reads. See Q6.

### Finding 1.7: The SOSP'21 follow-up shows Delos upgrades *application semantics* with a log-committed enable flag, which is a cluster-version gate, not a segment swap. Ad-hoc roll-outs were the only source of production inconsistency

**Evidence**:
- Two-phase protocol (§3.4, "Dynamic Updates"): "Inserting a new engine into the stack can be fraught: if an engine begins to operate before all servers are updated, it can cause inconsistent state across servers. In practice, we use a two-phase upgrade protocol to insert engines. In the first phase, we perform a rolling upgrade to add the engine to the stack: it can immediately piggyback its header on outgoing proposals ... but is not allowed to change the LocalStore within its apply upcall ... Once all servers have the new engine, we enable it by sending a command via the log itself. This ensures that the effects of the engine are visible on the local store beyond a consistent log position ... This update protocol requires all activity within an engine's apply upcall to be guarded by a flag which can only be toggled via the log. Further, adding an engine requires us to first upgrade all servers in the deployment with the new binary. We assume that we can upgrade, kill, or fence servers via container infrastructure."
- Format tolerance: an early layout of one pushed/popped header per engine "was brittle against stack upgrades"; entries became "a map of headers", so "each engine can simply check within the apply upcall if its own header is within the entry" (§3.4). Entries are Thrift-serialized (§4).
- Incidents (§6): "Ad-hoc stack updates resulted in inconsistency events across servers in production, which caused us to formalize the two-phase update protocol ... Surprisingly, engine roll-out has been the only source of inconsistency in production so far."
- Scale in May 2021: "DelosTable is in production on 107 clusters and handles more than 3B transactions per day in aggregate ... Zelos is in production on 155 clusters and handles 6.5B writes and 30B reads per day on the ZooKeeper API" (§5). "Each Delos database is typically replicated on 5 or 7 machines" (§4).
- Users: DelosTable serves the "Tupperware Resource Broker, which maintains a ledger of all machines in our data centers and their allocation status" (as summarised by [Lerner](https://www.micahlerner.com/2021/11/23/log-structured-protocols-in-delos.html); the OSDI paper calls the same service "Twine's Resource Broker").

**Source**: [Balakrishnan et al., "Log-structured Protocols in Delos", SOSP'21](https://maheshba.bitbucket.io/papers/delos-sosp2021.pdf) (author-hosted copy of [ACM DL 10.1145/3477132.3483544](https://dl.acm.org/doi/10.1145/3477132.3483544), high). Accessed 2026-09-25.
**Confidence**: High
**Verification**: [Lerner's summary](https://www.micahlerner.com/2021/11/23/log-structured-protocols-in-delos.html) (medium) summarises the engine stack. The [Meta At Scale talk abstract](https://atscaleconference.com/2021/03/15/virtualizing-consensus/) (first-party, 2021-03-17) states "We incrementally upgraded production databases without downtime simply by adding new engines". The pattern matches the KIP-584/CockroachDB/TigerBeetle gate identified in prior research (consensus doc, Finding A4.3).
**Analysis**: This is the most important finding for Q3. **Delos itself does not use virtual consensus to upgrade what the log *means*.** Segment swaps upgrade *how entries are ordered and stored*. Changes to *what entries mean* (new engines, new apply behaviour) go through (1) a rolling binary upgrade in which new code runs dormant, then (2) a log-committed enable command. That is exactly the "committed cluster-version gate". The two mechanisms are complementary layers of one upgrade story, not alternatives.

### Finding 1.8: Delos is *cluster-manager* control-plane prior art. It is the backing store for Meta's Twine scheduler and Resource Broker

**Evidence**: Delos is "a storage system at the bottom of the Facebook stack, operating as the backing store for the Twine scheduler and processing more than 2B TXes/day ... a shared log design (based on Corfu)" (talk abstract, PODC'21 ApPLIED workshop). The OSDI'20 paper lists "Twine's Resource Broker [44], which stores metadata for the fleet of servers in Facebook". Twine's own paper describes Resource Broker as "deployed to each DC" and recording "whether a machine in the DC is free or assigned to an entitlement".
**Source**: [ApPLIED 2021 speakers page](https://www.cse.chalmers.se/~elad/ApPLIED2021/speakers.html) (academic workshop, medium-high); [Delos OSDI'20](https://www.usenix.org/system/files/osdi20-balakrishnan.pdf) (high); [Twine, OSDI'20](https://www.usenix.org/conference/osdi20/presentation/tang) (high; Resource Broker description via the search summary of the paper). Accessed 2026-09-25.
**Confidence**: High that Delos backs Twine components. Medium on which Twine components beyond Resource Broker, because the "Twine scheduler" claim comes from a single talk abstract.
**Verification**: Two first-party Meta sources (OSDI'20 paper, 2021 talk) plus Lerner's SOSP'21 summary naming the "Tupperware Resource Broker" (Tupperware was Twine's earlier name).
**Analysis**: This is the closest production analogue to Overdrive's intent log: a fleet-wide orchestrator's allocation ledger on a 5–9-node replicated log, one deployment per datacenter (Resource Broker per DC). Note the *topology*. Meta runs **many small Delos clusters** (107 DelosTable + 155 Zelos in 2021), one per scope. It does not run one flat log across a region or the world.


## Q2. Lineage and sibling systems

### Finding 2.1: Restate Bifrost is a Rust virtual log explicitly derived from Delos and LogDevice. Its "consensus" is a metadata compare-and-swap, backed by built-in Raft, etcd or S3. Its licence is BSL 1.1 (Apache 2.0 after 4 years)

**Evidence**:
- Lineage: "Bifrost's (the log's) mechanism is based on a mix of Delos (Virtual Consensus) and LogDevice"; "To the outside and the partition processors, everything looks like a single contiguous log" ([Restate blog, 2025-02-20](https://restate.dev/blog/building-a-modern-durable-execution-engine-from-first-principles)).
- Reconfiguration: "On reconfiguration, the controller seals the active segment (a quorum of replicas refuses further appends), determines the authoritative tail of the log segment, elects a new sequencer/replica set, and performs a metadata CAS to publish the new segment as head"; "Segmentation enables clean and fast leadership changes, placement updates, and other reconfiguration without copying data" ([Restate architecture](https://docs.restate.dev/references/architecture)).
- Consensus boundary: "Restate abstracts its consensus to just an atomic compare-and-swap (CAS) metadata operation ... the built-in metadata store backs with an implementation of the RAFT consensus algorithm" (blog).
- Metadata backends: "Replicated (default): Built-in Raft-based consensus metadata server", an external etcd cluster, or Amazon S3. It holds "cluster membership and node roles", partition distribution, deployment registrations and "replicated log configuration" ([Restate metadata storage](https://docs.restate.dev/server/metadata)).
- Migrating between metadata backends needs "all cluster nodes to be restarted in migration mode", and "invocation processing will be paused during migration" (same page).
- Licence: Business Source License 1.1; Change License Apache 2.0 at "4 years after release"; Additional Use Grant forbids use "for a Public Restate Platform Service" ([restate LICENSE](https://github.com/restatedev/restate/blob/main/LICENSE)).

**Source**: Restate docs and blog (out-of-list primary, medium-high); GitHub LICENSE (github.com, medium-high). All accessed 2026-09-25.
**Confidence**: Medium-High. The mechanism is documented by the vendor and matches Delos. Implementation internals such as loglet provider kinds were not verified from code.
**Verification**: The same seal → tail → CAS sequence is in [Delos §3.2](https://www.usenix.org/system/files/osdi20-balakrishnan.pdf). [Restate issue #5377](https://github.com/restatedev/restate/issues/5377) ("Log trimming can silently do nothing when replicated-loglet clients have not discovered their tails") confirms a "replicated loglet" component with a "log-chain metadata" of segments. [Issue #2027](https://github.com/restatedev/restate/issues/2027) ("Enable cluster controller to control replicated loglet") is cited by title only; it was seen in search results, not fetched.
**Analysis**: Bifrost is the closest existing Rust artefact to what the target would build. Three lessons:
- (1) Its meta-consensus is an ordinary Raft group that the operator can swap for etcd or S3, which shows the MetaStore can sit at a *different* trust/availability tier from the log.
- (2) Migrating *that* store is a stop-the-world operation. The regress in Q3 is real in shipped code.
- (3) The licence rules out copying code into Overdrive (FSL-1.1-ALv2) without importing BSL terms. Reading the design for ideas is unaffected. This is a licensing observation, not legal advice.

### Finding 2.2: LogDevice (Meta, archived January 2022) is the epoch-based ancestor. Sealing is "reject records with smaller epochs", driven by a new sequencer, with epoch numbers from a ZooKeeper epoch store

**Evidence**:
- LSNs: "The sequence numbers of records in LogDevice are not integers, but pairs of integers. The first component of the pair is called the epoch number, the second one is offset within epoch."
- Epoch store: "When a new sequencer comes up, it receives a new epoch number from the metadata component called the epoch store ... Today we use Apache Zookeeper as the epoch store"; activation takes "just two roundtrips to the epoch store. This normally takes no more than a few milliseconds."
- Nodeset history lives "in a special internal log called the metadata log" ([LogDevice Concepts](https://logdevice.io/docs/Concepts.html)).
- Recovery seals the old epoch: "It tells the storage nodes in the nodeset of the epoch to reject records with epochs smaller than" the new epoch. The new sequencer "can immediately start taking appends in the new epoch" while recovery proceeds, and advances the "release pointer ... to the 'last known good' LSN" ([LogDevice Recovery](https://logdevice.io/docs/Recovery.html)).
- Status: "This is an archived project and is no longer supported or updated by Facebook" ([facebookarchive/LogDevice](https://github.com/facebookarchive/LogDevice)).

**Source**: logdevice.io (out-of-list primary, medium-high) and github.com (medium-high). Accessed 2026-09-25.
**Confidence**: High (mechanism). Status verified directly.
**Verification**: Delos OSDI'20 §4.2.2 (independent Meta team and paper): "LogDevice embeds an epoch number (generated by its own internal reconfiguration mechanism) within the log position".
**Analysis**: LogDevice's epoch is VR's view number applied per log. Its *metadata log* (the history of nodesets, read by readers to find where old records live) is the same idea as Delos's segment chain. The archival matters: Meta moved its control-plane database to Delos, and LogDevice survives only as a design reference.

### Finding 2.3: BookKeeper's ledger *fence + close* is a seal. Pulsar's managed ledger is a virtual log: a list of ledgers in a metadata store, rolled over on failure or size

**Evidence**:
- Ledger states: "OPEN", "CLOSED" or "IN_RECOVERY".
- Fencing: recovery "send[s] a fence message to all the bookies in the last fragment of the ledger"; acknowledgement from "(Qw - Qa) + 1 bookies from each write quorum" ensures that if "the old writer is alive and tries to add a new entry there will be no write quorum in which Qa bookies will accept".
- Close: recovery "updates the state in the metadata to `CLOSED`, and sets the last entry of the ledger". Metadata lives in ZooKeeper with CAS updates, and a racing writer "will fail on the CAS metadata write" ([BookKeeper protocol](https://bookkeeper.apache.org/docs/development/protocol)).
- Pulsar: a managed ledger "uses multiple BookKeeper ledgers"; "After a failure, a ledger is no longer writable and a new one needs to be created"; the ledger list is kept in the metadata store (Oxia recommended, ZooKeeper, or RocksDB standalone); on broker failover the old ledger goes "through a recovery process that will finalize the state of the ledger" ([Pulsar architecture](https://pulsar.apache.org/docs/next/concepts-architecture-overview/)).

**Source**: bookkeeper.apache.org and pulsar.apache.org (apache.org, high). Accessed 2026-09-25.
**Confidence**: High
**Verification**: [Vanlightly, strong vs weak sealing (2026-08-28)](https://jack-vanlightly.com/blog/2026/8/28/the-atomiclog-logdrive-strong-vs-weak-sealing): the NativeLoglet's seal is "basically the same as what Apache BookKeeper does" (active, inline fencing), and [Vanlightly on Delos](https://jack-vanlightly.com/blog/2025/2/5/an-introduction-to-virtual-consensus-in-delos): "Apache Pulsar, in combination with Apache BookKeeper, shares a lot of common ground with the NativeLoglet". Independent of both Delos papers.
**Analysis**: This is the widest-deployed virtual-log design in open source, and it has run for a decade. It proves the operational shape: per-segment fencing plus a CAS'd list of segments in a separate consensus store. It is also a warning. Pulsar has long depended on a separate ZooKeeper tier and is now moving to Oxia, a new metadata store. The meta-store is the part that needs replacing over time.

### Finding 2.4: The seal only has to be an *acknowledgement fence*. "Strong" (inline) and "weak" (post-write check) sealing are both valid, and the latest research line (Confluent's LogDrive/Conflux, OSDI'26) runs the VirtualLog over cloud object storage in production

**Evidence**:
- Seal types: the strong seal "is enforced inline by the append path" (NativeLoglet, BookKeeper). With a weak seal "the data write operation is oblivious to sealing ... its success is conditional on a subsequent check", which costs "one more round-trip". The requirement is only that "an append must not be allowed to return successfully" after sealing, because "Delos explicitly allows a failed append to nevertheless become durable" ([Vanlightly, 2026-08-28](https://jack-vanlightly.com/blog/2026/8/28/the-atomiclog-logdrive-strong-vs-weak-sealing)).
- Conflux (LogDrive, OSDI'26): "Conflux provides a third option by storing a shared log on cloud storage and using it to replicate state across VMs. A key innovation in Conflux is the separation of durability from sequencing ... Conflux is deployed in production at Confluent as the metadata service for an S3-based publish-subscribe system called K2"; "Conflux-over-DynamoDB slashes metadata cost by 10X and overall cost by 3X"; composition across regions: "we can run a single DynamoDBLogDrive per region; and then construct a QuorumLogDrive that writes to two out of three regions before responding". Fig. 1 layers VirtualLog (chaining) over AtomicLog (a Loglet) over LogDrives ([Vickers et al., OSDI'26](https://www.usenix.org/system/files/osdi26-vickers.pdf)).

**Source**: usenix.org (high); jack-vanlightly.com (practitioner and co-author of LogDrive, medium). Accessed 2026-09-25.
**Confidence**: Medium-High. The OSDI'26 paper was read directly for the abstract and introduction; the seal taxonomy rests on one (well-informed) author.
**Verification**: The OSDI'20 Delos paper independently states the fence-only requirement ("all we need is that any append on a sealed log throws an exception, and that any checkTail returns the seal status correctly", §3.4). Mahesh Balakrishnan is an author of Delos, CORFU and LogDrive, so this is *one research lineage*, not independent confirmation of the design's merit.
**Analysis**: For the target, the weak-seal model is what makes "a sealed segment in object storage" coherent. Once a segment is sealed and its tail recorded in the chain, the segment is immutable and its content can be addressed by hash and served from anywhere. The *active* segment still needs a quorum-sealable replica set.

### Finding 2.5: CORFU introduced the pattern: epoch-tagged storage units, a `seal` command, and an "auxiliary" holding the sequence of projections. Scalog and FuzzyLog extend it to scale and to geo-distribution

**Evidence**:
- CORFU seal: "flash units are required to support a 'seal' command. Each incoming message to a flash unit is tagged with an epoch number. When a particular epoch number is sealed at a flash unit, it must reject all subsequent messages sent with an epoch equal or lower to the sealed epoch ... [and] send back an acknowledgment ... including the highest page offset that has been written" (§3.1).
- CORFU auxiliary: "The auxiliary is a durably stored sequence of projections in the system, where the position of the projection in the sequence is equivalent to its epoch ... The auxiliary can be implemented in multiple ways: on a conventional disk volume, as a Paxos state machine, or even as a CORFU instance with a static, never-changing projection" (§3.2.1). Reconfiguration is "1. Sealing the current projection" then "2. Writing the new projection at the auxiliary", where a losing writer "aborts its own reconfiguration" (§3.2.1).
- CORFU partial seal: "A flash unit in Pi has to be sealed only if a log position mapped to one of its flash pages by Pi is no longer mapped to the same page by Pi+1" (§3.2.1).
- Scalog: "supports reconfiguration with no loss in availability" and "can totally order up to 52 million records per second" (NSDI'20 abstract).
- FuzzyLog: "for deployments that span geographical regions, a total order may be impossible: a network partition can cut off clients from the sequencer or a required quorum"; "A color is a set of independent, totally ordered chains, where each chain contains updates originating in a single geographical region. Chains within a color are connected by cross-links that represent update causality" (§1).

**Source**: [Balakrishnan et al., CORFU, NSDI'12](https://www.usenix.org/system/files/conference/nsdi12/nsdi12-final30.pdf); [Ding et al., Scalog, NSDI'20](https://www.usenix.org/conference/nsdi20/presentation/ding); [Lockerman et al., FuzzyLog, OSDI'18](https://www.usenix.org/system/files/osdi18-lockerman.pdf). All usenix.org (high), accessed 2026-09-25.
**Confidence**: High (for each paper's own claims)
**Verification**: Delos OSDI'20 cites all three as antecedents (refs 7, 16, 33) and positions itself as "the first to propose a virtualized shared log composed from heterogeneous log implementations". The Delos authors overlap heavily with the CORFU, Tango and FuzzyLog authors (Balakrishnan throughout), so this is one research programme viewed from several angles, not independent validation.
**Analysis**: Two points. (1) CORFU's auxiliary already allowed the meta-store to be "a CORFU instance with a static, never-changing projection", the same trick as Delos's self-reconfiguring single-slot Paxos: end the regress by making the meta-store's own membership part of its own log. (2) FuzzyLog is the only member of the lineage that gives per-region *write* locality, and it does so by giving up the total order (causal across regions). Tango and vCorfu (object and stream layers over CORFU) were not re-read; they add nothing to sealing or meta-consensus beyond CORFU.

### Finding 2.6: Pravega seals *stream segments* on scaling and chains successors. Its "virtual log" is keyspace-partitioned, with metadata in a Controller backed by ZooKeeper, and it tiers durable storage from BookKeeper to object storage

**Evidence**: A stream is "split into a set of shards or partitions generally referred as Stream Segments". On scale-up, "The Stream Segment 1 is sealed and stops accepting writes", and successor segments take new writes. Segments covering contiguous key ranges "can also be merged". Metadata is owned by "the Controller" with Apache ZooKeeper for coordination. Data lives in a Tier-1 durable log (Apache BookKeeper) and Tier-2 long-term storage (HDFS or object storage).
**Source**: [Pravega concepts](https://cncf.pravega.io/docs/latest/pravega-concepts/) (out-of-list primary for Pravega's own behaviour, medium-high). Accessed 2026-09-25.
**Confidence**: Medium (single project source)
**Verification**: Its use of BookKeeper as the Tier-1 log inherits the fencing semantics of Finding 2.3. Pravega's current CNCF maturity/archival status was not verified (see Knowledge Gaps).
**Analysis**: Pravega shows the *sealed-is-immutable → tier to object storage* move in production-grade open source. Its segments seal on *load* (scaling), not on upgrade or failure. It is the closest analogue to "per-key-range segments", but it gives up a single total order across segments, as FuzzyLog does.

### Finding 2.7 (synthesis): every shipped virtual/shared log has the same three parts: a per-segment fence, a CAS'd chain in a small consensus store, and immutable sealed segments. They differ in what triggers a seal

| System | Seal primitive | Chain / metadata store | Seal trigger in practice | Status |
|---|---|---|---|---|
| Delos VirtualLog | Loglet `seal()`; quorum-set seal bit (NativeLoglet) | MetaStore: versioned register, CAS. ZooKeeper first, then embedded single-slot Paxos | Deploys, preemptions, failures; operator-driven protocol swaps | Production at Meta (2019–2021 data) |
| LogDevice | New epoch; storage nodes reject lower epochs | Epoch store (ZooKeeper) plus internal metadata log | Sequencer failover | Archived January 2022 |
| BookKeeper / Pulsar | Ledger fence (`(Qw−Qa)+1` per write quorum) then CLOSED | Ledger metadata in ZooKeeper (CAS); managed-ledger list in Oxia or ZooKeeper | Writer/broker failover; size/time rollover | Production (Apache) |
| CORFU | Epoch-tagged `seal` on flash units | Auxiliary: sequence of projections (disk, Paxos, or static CORFU) | Unit failure; tail extension | Research, then CorfuDB |
| Restate Bifrost | Quorum of replicas refuses appends | Metadata store (built-in Raft, etcd, or S3); CAS | Leadership change, placement | Production (BSL 1.1) |
| Pravega | Segment sealed on scale-up/down | Controller + ZooKeeper | Load-based scaling | Open source (status unverified) |
| Conflux / LogDrive | Strong or weak seal over cloud storage | Built on the Delos model; details not read | Not read | Production at Confluent (K2 metadata) |

**Source**: Findings 1.1–2.6 above.
**Confidence**: High for the rows backed by primary papers and docs; Medium for the Pravega and Conflux cells marked "not read" or unverified.
**Analysis**: *No* system in this set reports using segment sealing as its primary mechanism for **application-semantic** rolling upgrades. Delos is the only one that reports swapping the *ordering protocol* live, and for application semantics it uses a log-committed flag (Finding 1.7).


## Q3. Does a virtual log actually solve rolling upgrades?

An upgrade touches three separate "version planes", and they behave differently:
- **(P1) the ordering-protocol wire/storage version**: how the replicas of *one* segment talk and persist;
- **(P2) the entry/semantic version**: what a committed entry means, and how `apply` interprets it;
- **(P3) the meta-store version**: how the chain of segments is stored and read.

### Finding 3.1: A virtual log removes the flag-day for P1 only, and only by *moving* it to a precondition: every reader and appender must carry the new Loglet client before the switch

**Evidence**:
- Transparency: virtualization "should be transparent to applications, which should be unmodified and oblivious to the virtualized nature of the log" (OSDI'20 §3).
- Routing is done by a client-side library: "Any append and checkTail commands on a VirtualLog are directed to the last Loglet in the chain, while readNext commands on a range are routed to the Loglet storing that range" (§3.1), and "The VirtualLog is composed of two distinct components: a client-side layer ... and a logically centralized metadata component (MetaStore)" (§3.2).
- In the one documented live swap, the new NativeLoglet was *converged*, running as a "NativeLoglet server (or LogServer)" inside every Delos server (§4.2.1). Its code therefore had to be deployed fleet-wide before the `reconfigExtend`.
- Delos names the prerequisite explicitly for its other upgrade mechanism: "adding an engine requires us to first upgrade all servers in the deployment with the new binary" (SOSP'21 §3.4).

**Source**: [Delos OSDI'20](https://www.usenix.org/system/files/osdi20-balakrishnan.pdf); [Delos SOSP'21](https://maheshba.bitbucket.io/papers/delos-sosp2021.pdf) (high). Accessed 2026-09-25.
**Confidence**: Medium-High. The mechanism is stated. That the ZK→Native switch required a prior fleet-wide rollout is an *inference* from the converged deployment and the client-side routing; the paper does not narrate the rollout.
**Verification**: The same precondition is the first step of every gate-based upgrade in prior research: Kafka's "rolling restart ... with the new binary" before finalization, and CockroachDB's automatic finalization once "all nodes have rejoined the cluster using the new binary" (consensus doc, Finding A4.3).
**Analysis**: A virtual log does not let an old-binary node *participate in or read* a new-protocol segment. What it removes is the need for old and new **protocol peers to talk to each other**. Old-protocol replicas finish and seal their segment, and new-protocol replicas start fresh. For `viewstamp`, whose handshake rejects any other wire version (consensus doc, Finding A4.1), that is the genuine benefit: no wire-version negotiation *inside* VR. It does **not** remove the need for a dual-capable binary on every node, which is the same "N−1/N" discipline the cluster-version gate requires.

### Finding 3.2: A virtual log does nothing for P2 (entry semantics). Delos itself used a log-committed enable flag for P2, and P2 roll-outs were its only production inconsistency source

**Evidence**: SOSP'21 §3.4 describes the two-phase protocol: new code runs dormant, then "we enable it by sending a command via the log itself", with the engine's apply "guarded by a flag which can only be toggled via the log"; "Ad-hoc stack updates resulted in inconsistency events across servers in production ... engine roll-out has been the only source of inconsistency in production so far" (§6). Entries were made tolerant of unknown headers ("a map of headers") after the positional layout proved "brittle against stack upgrades" (§3.4).
**Source**: [Delos SOSP'21](https://maheshba.bitbucket.io/papers/delos-sosp2021.pdf) (high). Accessed 2026-09-25.
**Confidence**: High
**Verification**: The same pattern appears at Kafka (`metadata.version` as a `FeatureLevelRecord` in the log), CockroachDB (version gates) and TigerBeetle (upgrade op), as covered in consensus doc Findings A4.2–A4.3. The prior placement research also found that deterministic replay of *code* is "Unsafe without cluster-wide version switch" (placement doc, C1 synthesis).
**Analysis**: This answers the mixed-version question directly. **An old-version reader cannot safely read new-format entries, whatever segment they sit in.** Sealing changes where entries are ordered, not their bytes or meaning. For the target, whose learners are thousands of nodes holding the full log, P2 is the plane that actually freezes intent during a flag-day. It needs the committed cluster-version gate with or without a virtual log.

### Finding 3.3: The meta-store (P3) needs its own upgrade story, and shipped systems either freeze it or accept downtime to change it

**Evidence**:
- Delos kept its MetaStore deliberately minimal ("a single versioned register supporting a conditional write") and "simple, unoptimized" single-slot Paxos in which "Each Paxos instance stores the membership of the next instance" (OSDI'20 §3.2, §4.1). It moved the MetaStore from ZooKeeper to an embedded implementation; the paper does not describe *how* that migration was done.
- Restate's metadata migration requires "all cluster nodes to be restarted in migration mode", and "invocation processing will be paused during migration" ([Restate metadata](https://docs.restate.dev/server/metadata)).
- CORFU allowed the auxiliary to be "a CORFU instance with a static, never-changing projection" (NSDI'12 §3.2.1).

**Source**: as cited, accessed 2026-09-25.
**Confidence**: Medium-High. Direct evidence from two systems; the Delos migration path is unknown (Knowledge Gaps).
**Verification**: Pulsar's move of its metadata store to Oxia (Finding 2.3) is further evidence that the meta tier is what gets replaced over a system's life.
**Analysis — chasing the regress**: The regress ends in one of three ways:
- **(a) Freeze the MetaStore protocol.** Make it so small that it never needs to change: a CAS register, a versioned chain descriptor, and a frozen handoff format. This is the Delos/CORFU answer. It requires designing the chain descriptor to be forward-extensible (opaque per-segment config blobs) from day one.
- **(b) Give the MetaStore range-negotiated versions.** This is the cluster-version-gate machinery again, one level down.
- **(c) Accept a brief stop** for MetaStore format changes, as Restate does.

For the target, (a) is attractive precisely *because* the chain changes rarely. But the chain descriptor becomes a forever-compatible format, a new permanent compatibility obligation.

### Finding 3.4 (verdict input): What the virtual log gives that a version gate cannot: live *protocol* replacement, live *rollback*, surgical log repair, and tiering. What the gate gives that the virtual log cannot: semantic upgrades

**Evidence**:
- Rollback: "we discovered latent bugs in the NativeLoglet; we reconfigured to ZKLoglet, rolled out hotfixes, and then reconfigured back".
- Repair: "a 'poison' entry caused hangs on all learners processing the log", fixed by "changing the metadata of the VirtualLog".
- Tiering: "migrating older segments to a Loglet layered on cold storage (BackupLoglet)" (OSDI'20 §1, §6).
- Semantics: a log-committed enable flag (SOSP'21 §3.4).

**Source**: [Delos OSDI'20](https://www.usenix.org/system/files/osdi20-balakrishnan.pdf); [Delos SOSP'21](https://maheshba.bitbucket.io/papers/delos-sosp2021.pdf). Accessed 2026-09-25.
**Confidence**: High (first-party production reports)
**Verification**: Tiering of sealed segments is independently shipped in Pulsar ("once a segment is sealed in BookKeeper, it becomes immutable and can be copied to long-term storage", [Pulsar tiered storage](https://pulsar.apache.org/docs/next/tiered-storage-overview/)) and Pravega (Finding 2.6).
**Analysis**: The two mechanisms compose, and the composition is what Delos actually ran: **segments for P1, a committed flag for P2, a frozen minimal MetaStore for P3.** A committed cluster-version gate is *necessary* in every combination. A virtual log is *additional* and buys P1 flexibility and rollback.

## Q4. The meta-consensus problem

### Finding 4.1: Every surveyed system keeps the segment chain in a small, strongly consistent CAS store that is off the append path

**Evidence** (see Findings 1.2, 2.1–2.3, 2.5):
- Delos: versioned register, first in ZooKeeper, then embedded single-slot Paxos, "accessed by the VirtualLog only during reconfigurations".
- LogDevice: ZooKeeper epoch store plus an internal metadata log.
- BookKeeper: ZooKeeper ledger metadata with CAS; Pulsar's managed-ledger list in Oxia or ZooKeeper.
- Bifrost: built-in Raft metadata server, etcd or S3, with CAS.
- CORFU: the auxiliary, which may be a disk, Paxos, or a static CORFU instance.

**Source**: primary sources cited in those findings. Accessed 2026-09-25.
**Confidence**: High (five independent systems)
**Verification**: Five organisations/codebases (Meta ×2, Apache, Restate, Microsoft Research/VMware CORFU lineage).
**Analysis**: The meta-store's requirements are small: **linearizable CAS on one small value, available whenever a reconfiguration is needed, not on the append path.**

### Finding 4.2: When the meta-store is unavailable, appends continue and reconfiguration stops. The log is then only as available as its current active segment

**Evidence**: Delos: the MetaStore "is not required to be particularly fast, since it is accessed by the VirtualLog only during reconfigurations"; clients route using "its locally cached copy of the chain" in steady state (OSDI'20 §3.2–3.3). NativeLoglet appends need only the sequencer and a majority of LogServers (§4.2.1). LogDevice's new sequencer needs "two roundtrips to the epoch store" to *activate* ([LogDevice Concepts](https://logdevice.io/docs/Concepts.html)), so a failover needs the epoch store.
**Source**: as cited, accessed 2026-09-25.
**Confidence**: Medium-High for Delos and LogDevice (explicit). The BookKeeper/Pulsar and Bifrost behaviour under meta-store outage was *not* found stated in their docs (see Knowledge Gaps).
**Verification**: [BookKeeper protocol](https://bookkeeper.apache.org/docs/development/protocol): recovery and ensemble changes are CAS writes to ZooKeeper, so fencing and close need the metadata store.
**Analysis**: Meta-store unavailability converts a *recoverable* active-segment failure into an outage. So the meta-store must be at least as available as the voter set: co-located on the same 3–5 voters (Delos embedded MetaStore) or deliberately more available.

### Finding 4.3 (options for the target): the meta-store can be (a) a separate tiny Paxos on the voters, (b) in-band in the VR group itself, as VR §7 already does, or (c) an external store

**Evidence**:
- **(a)** Delos's embedded MetaStore: single-slot Paxos on the same servers, where "Each Paxos instance stores the membership of the next instance" (OSDI'20 §4.1).
- **(b)** Delos explains why it *avoided* in-band configuration: in-band requires "the Loglet itself to be highly available for writes (i.e., implement fault-tolerant consensus)" (§3.3). It names in-band reconfiguration as the likely future for planned cross-region reconfiguration: "we can potentially reconfigure by storing inline commands within the Loglet itself" (§3.3). VR Revisited §7 is in-band: "A reconfiguration is triggered by a special client request. This request is run through the normal case protocol by the old group. When the request commits, the system moves to a new epoch", and "the reconfiguration request is the last request processed in the current epoch". But VR leaves discovery out-of-band: "a new client needs a way to find the current configuration. This requires an out-of-band mechanism, e.g., the current configuration can be obtained by communicating with a web site run by the administrator" ([VR Revisited §7, mirror](http://ying-zhang.cn/dist/2012-vr.html)).
- **(c)** Bifrost supports etcd or S3 as the metadata store ([Restate metadata](https://docs.restate.dev/server/metadata)). Conflux uses DynamoDB/S3 ([LogDrive OSDI'26](https://www.usenix.org/system/files/osdi26-vickers.pdf)).

**Source**: as cited, accessed 2026-09-25.
**Confidence**: High for what each source says. The mapping to the target is interpretation.
**Verification**: CORFU's "static never-changing projection" auxiliary (NSDI'12) is a fourth variant of option (a).
**Analysis (interpretation)**: Delos's argument for a separate MetaStore is that its Loglets are *not* fault-tolerant. **A VR segment is fault-tolerant**, so the reason for a separate MetaStore disappears for VR segments. Option (b), an in-band successor record committed as the last op of segment *k*, is exactly VR §7, LogDevice's metadata-log idea, and Delos's own stated direction for geo-distribution.

The remaining gap is *discovery*: how a learner or new node finds the head of the chain. In the target, every node already holds the log. Two facts can bootstrap discovery:
- a gossiped fact, "latest segment id + epoch + hash";
- a committed, hash-chained successor record in the predecessor segment, which makes the gossiped fact verifiable.

This is the "facts via gossip, decisions via log" split again, and the same shape as Cassandra TCM's "epoch in every message → catch up from any peer" (consensus doc, Executive Summary).

What (b) cannot do: seal a segment whose VR group has permanently lost quorum. But neither can Delos: NativeLoglet seal also needs a majority, and a CFT system cannot survive losing a majority of its voters without operator intervention anyway.

## Q5. Fit with VR

### Finding 5.1: A VR group can serve as a Loglet (Delos ran fault-tolerant ZooKeeper and LogDevice as Loglets), but `viewstamp` exposes no seal, halt or terminal-op API today

**Evidence**:
- Delos Fig. 5 lists ZK, Backup and LogDevice Loglets as "Consensus: Yes", and "Loglets that implement their own leader election or reconfiguration protocols typically expose sparse address spaces" (OSDI'20 §3.4, §4.2).
- `viewstamp` README: membership is `SingleChange` with a "closed four-delta vocabulary" (`AddLearner`, `PromoteLearner`, `DemoteVoter`, `RemoveLearner`); "multi-change joint consensus is future work"; snapshots are "content-addressed incremental snapshots — checkpoints are a content-addressed block DAG behind the `BlockStore` trait"; upgrades are flag-day, and mixed-version peers "reject each other at the handshake". No halt, drain or terminal-op API is documented.

**Source**: [Delos OSDI'20](https://www.usenix.org/system/files/osdi20-balakrishnan.pdf) (high); [al8n/viewstamp README](https://raw.githubusercontent.com/al8n/viewstamp/main/README.md) (github.com, medium-high). Accessed 2026-09-25.
**Confidence**: High for Delos. Medium for `viewstamp`: the conclusion rests on the README only; code was not audited, consistent with the prior research's scope.
**Verification**: The prior consensus research reached the same README-level reading of `viewstamp` (consensus doc, Findings V1, A4.1).
**Analysis (interpretation)**: For `viewstamp` to be a Loglet it would need these primitives:
1. **`seal`**: idempotent; durable in the superblock; replicas reject `Prepare`/requests for the segment afterwards; callable without a live primary (NativeLoglet-style quorum bit) or as a committed terminal op (VR §7-style).
2. **`check_tail`**: returns `(commit_op, sealed)`. After a seal, the authoritative tail is what a view-change-style quorum merge (DoViewChange's "largest (view, op)" log) establishes. This is VR's own analogue of Delos's `checkTail` repair of the "all-sealed diff-tail" case.
3. **A read-only mode** serving committed ops by range from a sealed group.
4. **A boundary checkpoint**: the content-addressed snapshot at the tail op, which seeds the successor segment's initial state *without cross-version messages*.

VR §7's `StartEpoch`/`EpochStarted` messages go from old replicas to new ones, and those are cross-version messages. A virtual-log boundary would replace them with "successor fetches the boundary snapshot by hash". That removes the only point where old and new wire versions would otherwise have to talk.

### Finding 5.2: Sealed segments are immutable, so they can be served by content address from any peer or from object storage. Three shipped systems do this

**Evidence**:
- Pulsar: "once a segment is sealed in BookKeeper, it becomes immutable and can be copied to long-term storage", and offloaded data remains readable ([Pulsar tiered storage](https://pulsar.apache.org/docs/next/tiered-storage-overview/)).
- Pravega tiers from BookKeeper to HDFS or object storage (Finding 2.6).
- Bifrost: "Quorum replication to nodes with async batch writes to S3" ([Restate blog](https://restate.dev/blog/building-a-modern-durable-execution-engine-from-first-principles)).
- Delos migrated "older segments to a Loglet layered on cold storage (BackupLoglet)" (OSDI'20 §1).
- Conflux stores the whole shared log on cloud storage ([LogDrive OSDI'26](https://www.usenix.org/system/files/osdi26-vickers.pdf)).

**Source**: as cited, accessed 2026-09-25.
**Confidence**: High (four or more independent systems)
**Verification**: Independent organisations: Apache (Pulsar), Dell/CNCF (Pravega), Restate, Meta, Confluent.
**Analysis**: For the target's learners (thousands of full-log replicas), this is the most practical benefit of segmentation, and it does not need heterogeneous Loglets.
- **Sealed segments**: once a segment is sealed and its hash is committed in its successor record, a learner can fetch it from *any* peer, a relay tree or object storage, and verify it against one trusted hash. This is the pull-based, hash-chain-verified dissemination the prior research proposed (consensus doc, Executive Summary).
- **The active segment**: still needs a leader or relay path.

Segmenting the log by epoch is therefore useful for dissemination and learner catch-up on its own, without the MetaStore machinery.

### Finding 5.3: Reconfiguration frequency: Delos made the seal path the *hot* path, 98 reconfigurations per month on one cluster. A VR-backed virtual log would make it a *cold* path

**Evidence**: Delos reconfigurations are "triggered by 1) continuous deployment of software upgrades; and 2) machine preemptions ... one of our production clusters was reconfigured 98 times in the 1-month period" (OSDI'20 §5.2). The NativeLoglet has no leader election (§4.2.1). Prior research found upgrade code "the most bug-dense area" in Jepsen's TigerBeetle analysis, and that `viewstamp`'s simulator does not cover reconfiguration (consensus doc, Findings A4.2, V3).
**Source**: [Delos OSDI'20](https://www.usenix.org/system/files/osdi20-balakrishnan.pdf); consensus doc. Accessed 2026-09-25.
**Confidence**: Medium-High (the facts are sourced; the frequency contrast is interpretation)
**Verification**: The Jepsen TigerBeetle upgrade findings (consensus doc A4.2) are independent evidence that rarely exercised upgrade paths harbour bugs.
**Analysis (interpretation)**: Delos's seal path is trustworthy partly *because* it runs daily: every deploy exercises it. If `viewstamp` handles leader failure internally (view change), a virtual-log seal on top would run only at protocol upgrades and voter-set epochs, a few times a year. That is the classic shape of a latent-bug path. Mitigations: run the seal path deliberately and often (for example, seal a segment on every voter-set change or every N ops, the way Pulsar rolls ledgers by size or time), and put seal/roll-forward races under deterministic simulation from day one.

## Q6. Per-region segments and multi-region

### Finding 6.1: A virtual log chains segments in *time*, not space. It gives one active write locus per log, so it cannot give simultaneous per-region write locality to one totally ordered intent log

**Evidence**:
- Delos: "only the last log in the chain is appendable" (OSDI'20 §3.1). Even the StripedLoglet waits "until all prior logical positions have been filled, across all stripes" before acknowledging (§4.2.2).
- Delos's cross-region deployment is one quorum spread geographically: a "geo-distributed DelosTable cluster with 5 servers distributed across the continental USA", with strongly consistent reads at "roughly 48ms" p99, or 220 µs with the LeaseEngine (SOSP'21 §5.2).
- FuzzyLog: "a total order may be impossible" across regions; per-region chains with causal cross-links (OSDI'18 §1).
- Conflux composes durability across regions with "a QuorumLogDrive that writes to two out of three regions before responding" (OSDI'26 §1). That is cross-region *durability*, not locality.

**Source**: as cited (usenix.org / author-hosted ACM, high). Accessed 2026-09-25.
**Confidence**: High
**Verification**: Four papers from the same research lineage agree. The prior placement research reached the same conclusion from Fly's regionalization and Astrolabe (placement doc, "Plan for hierarchical aggregation").
**Analysis**: What a virtual log *can* do across regions:
- **(1) Move the active segment's voters/leader between regions** with a planned reconfiguration ("follow the sun"), at one seal pause per move. Delos says this would need in-band planned reconfiguration to be cheap.
- **(2) Keep sealed segments in each region's object storage** for local catch-up.

Per-region *write* locality needs **multiple logs**: per-region intent logs plus a global log for cross-region decisions. That is a sharding/hierarchy decision, independent of virtual consensus. Meta's own pattern is many small Delos clusters (one Resource Broker per datacenter), not one global log (Finding 1.8).

## Q7. Costs

### Finding 7.1: The steady-state cost is small, 100–150 µs p99 per append in Delos. The real costs are a second consensus component, a new reconfiguration path to verify, and trim/GC coordination across segments

**Evidence**:
- Latency: "For append and checkTail, virtualization adds 100-150 µseconds to p99 latency ... readNext ... adds only a few µseconds"; "Reconfigurations occur within 10s of ms" (OSDI'20 §5.2). Seal-time pause in production: "p99 latency spikes for indexed queries during reconfiguration, but otherwise service availability is not disrupted" (§5.1).
- Effort: NativeLoglet "took just under 4 months to implement and deploy"; the embedded MetaStore is "simple, unoptimized" single-slot Paxos (§4.1–4.2).
- Subtle semantics that must be tested: "zombie" appends visible after seal (§3.4), and roll-forward that "creates a default new chain by cloning the configuration of the previous active segment" (§3.2).
- Production-grade failure mode in a shipped Rust virtual log: Restate issue #5377 (opened 2026-09-21, open), "Log trimming can silently do nothing when replicated-loglet clients have not discovered their tails". Eighteen loglets took a no-op trim path while reporting success, and "Old segments remained in the log-chain metadata" ([restatedev/restate#5377](https://github.com/restatedev/restate/issues/5377)).
- Delos's own production inconsistencies came from P2 roll-outs, not from the VirtualLog (SOSP'21 §6).

**Source**: as cited, accessed 2026-09-25.
**Confidence**: Medium-High. Latency and effort are first-party. The failure-mode corpus is thin: one open issue, plus Delos's statement.
**Verification**: The 100–150 µs figure is attributed by the paper to "the overhead of an asynchronous Future-based API", not to the protocol, so it is implementation-specific. No Jepsen or Antithesis analysis of any virtual log (Delos, Bifrost) was found (Knowledge Gaps).
**Analysis**: Against the target's WAN commit latencies (tens of milliseconds cross-region), the per-append overhead is noise. The costs that matter are engineering and verification:
- (1) **The meta-store.** Even a tiny Paxos is a second consensus implementation, unless option (b), in-band in VR, is taken.
- (2) **Seal, checkTail repair, roll-forward and chain-CAS races.** These are exactly the interleavings deterministic simulation must cover, and the prior research found `viewstamp`'s simulator does not yet cover reconfiguration.
- (3) **Cross-segment trim/snapshot/GC.** Restate's open bug shows this is where a shipped Rust implementation still fails.
- (4) **The P2 gate is still needed.** So the virtual log is always *additional* engineering, never a substitute.

## Delos — mechanism and production evidence

**Mechanism** (Findings 1.1–1.4):

| Element | What it is | Fault-tolerance requirement |
|---|---|---|
| VirtualLog | Client-side library that routes `append`/`checkTail` to the tail Loglet and `readNext` by range; adds `reconfigExtend`/`Truncate`/`Modify` | The only fault-tolerant consensus in the system, used only at reconfiguration |
| MetaStore | Single versioned register with conditional write; holds the chain as segments with start/stop positions and opaque per-Loglet config | Paxos-class. First ZooKeeper, then embedded single-slot Paxos whose slots name the next slot's membership |
| Loglet | `append`, `checkTail → (tail, sealed)`, `readNext`, `prefixTrim`, `seal` | Only `seal` must be highly available; appends may stop when a sequencer dies |
| NativeLoglet | Sequencer + LogServers with a quorum-set seal bit; 5-state `checkTail` repair | Seal/checkTail need a majority; append also needs the sequencer |
| Reconfiguration | seal → checkTail → CAS chain → fetch; any client may drive it; stalled attempts are rolled forward *with the old segment's configuration* | Seal is idempotent; CAS picks one winner |

**Production evidence** (Findings 1.5, 1.7, 1.8):
- Production in 8 months on a ZooKeeper-backed Loglet. The consensus protocol was replaced live 4 months later (2 April 2019), with 10×/5× p99 gains, largely because the first Loglet was deliberately naive.
- Live rollback to ZKLoglet to hotfix NativeLoglet bugs. Surgical removal of a "poison" entry via metadata.
- 1.8B transactions/day in 2020; more than 2B/day in 2021 as the Twine scheduler's store; 107 DelosTable clusters (more than 3B txns/day) and 155 Zelos clusters (6.5B writes + 30B reads/day) in May 2021. Clusters typically have 5–7 nodes.
- Virtualization overhead 100–150 µs p99 per append and 6 µs per read. Reconfiguration takes tens of milliseconds and happens mostly on deploys and preemptions.
- Application-semantic upgrades used a dormant-then-enable-via-log protocol, and ad-hoc roll-outs were the only production inconsistency source.
- **No post-2021 public evidence** of Delos's status was found (Knowledge Gap 3).

## Sibling systems compared

See the table in Finding 2.7. In short:

- **Converged shape.** Fence + CAS'd chain in a small consensus store + immutable sealed segments. Found in Delos, LogDevice, BookKeeper/Pulsar, CORFU, Bifrost, Pravega and Conflux.
- **Seal triggers differ.** Failover (LogDevice, BookKeeper, Bifrost), load (Pravega), size/time rollover (Pulsar), and deploys plus operator-driven protocol swaps (Delos).
- **Meta-store choice.** ZooKeeper dominates historically (LogDevice, BookKeeper, Pravega, early Delos). Newer systems embed it (Delos single-slot Paxos, Bifrost Raft) or push it to cloud services (Bifrost S3/etcd, Conflux DynamoDB/S3, Pulsar Oxia).
- **Rust prior art.** Restate Bifrost is the only Rust implementation found. It is BSL 1.1 (Apache 2.0 after 4 years; no "Public Restate Platform Service"): design reference only, not a code source. An open issue from September 2026 (#5377, silent no-op trims leaving old segments in chain metadata) shows cross-segment GC is still where shipped implementations fail.
- **Heterogeneous Loglets.** Only Delos reports swapping the *ordering protocol* itself in production. Every sibling reconfigures *within one protocol* (new epoch, new ledger, new segment, same code).

## Does it solve rolling upgrades? (verdict)

**Partly, and only in combination with a committed cluster-version gate.**

| Upgrade plane | Virtual log alone | Cluster-version gate alone | Both (Delos's actual practice) |
|---|---|---|---|
| P1 ordering-protocol wire/storage | **Yes**, if all nodes first run a dual-capable binary. Old and new protocol peers never talk | Yes, but only with in-protocol range negotiation (which `viewstamp` lacks) | Segment boundary confines each wire version to one epoch; no in-protocol negotiation needed |
| P2 entry semantics / apply behaviour | **No.** Old readers cannot interpret new entries in any segment | **Yes** (dormant code + committed enable) | Gate does P2 |
| P3 meta-store format | Creates a new plane that needs its own story | N/A (no meta-store) | Freeze the chain descriptor; migrate with a pause if ever needed |
| Rollback of a protocol change | **Yes** (Delos reconfigured back to ZKLoglet) | Only before finalization (CockroachDB model) | Virtual log adds post-cut-over rollback |

**Does sealing avoid a flag-day or move it?** It *moves* it:
- from "stop every node, upgrade, restart" to "roll dual-capable binaries, then one atomic chain CAS";
- the downtime shrinks to one seal pause (tens of milliseconds in-DC);
- the precondition that every participant has the new code remains, which is the same precondition a version gate has.

**Does the MetaStore then need rolling upgrades?** Yes, unless its format is frozen. Delos and CORFU froze it by keeping it trivially small; Restate accepts a processing pause to migrate it.

**Regress verdict**: it terminates if the chain descriptor is designed once, minimal and forward-extensible (opaque per-segment config).

## Meta-consensus options

| Option | Precedent | Availability coupling | Cost for the target | Fit |
|---|---|---|---|---|
| (a) Separate tiny Paxos/CAS register on the voters | Delos embedded MetaStore; CORFU auxiliary-as-Paxos | Needs a majority of the same voters, so no extra failure domain | A second consensus implementation to build, simulate and never change | Good if Loglets are *not* fault-tolerant (NativeLoglet style) |
| (b) In-band successor record, committed as the last op of epoch *k* | VR Revisited §7 (reconfiguration is "the last request processed in the current epoch"); LogDevice metadata log; Delos §3.3's stated future direction | Needs the old epoch's quorum to seal, which Delos's seal also needs | Reuses VR; discovery via gossiped, hash-verifiable "latest epoch" facts | **Best fit for VR segments**, which are fault-tolerant, so Delos's reason for (a) does not apply |
| (c) External store (etcd, S3/DynamoDB conditional writes) | Bifrost (Raft, etcd or S3); Conflux (DynamoDB/S3); Pulsar (Oxia/ZooKeeper) | Adds an external dependency | Contradicts single-binary, flat, no-external-dependency goals | Poor for the target, except object storage as a *tier* for sealed segments |

**When the meta-store is unavailable** (Finding 4.2), appends continue on the cached chain, but no reconfiguration, failover-by-reconfiguration or upgrade cut-over can happen. Under option (b) this coupling collapses: the "meta-store" is the log itself, and it is unavailable exactly when the active VR epoch has lost its quorum.

## Fit with Viewstamped Replication / viewstamp

- **VR §7 is already a single-protocol virtual log.** Epoch numbers are segment ids, the reconfiguration request is the seal ("primary immediately stops accepting other client requests"), and `StartEpoch`/`EpochStarted` is the handoff. What it lacks relative to Delos:
  - (1) permission for epoch *k+1* to run a different wire version;
  - (2) a handoff that needs no cross-version messages;
  - (3) a defined chain-discovery mechanism, which VR explicitly leaves "out-of-band".
- **A VR group as a Loglet needs four primitives** (Finding 5.1):
  - idempotent durable `seal`;
  - `check_tail → (commit_op, sealed)` via a quorum log merge;
  - read-only serving of a sealed epoch by op range;
  - a content-addressed boundary checkpoint that seeds the successor.

  `viewstamp`'s README documents none of these. Its content-addressed block-DAG snapshots and learner seats are the right building blocks for the last two.
- **Sealed epochs by content address.** Once an epoch is sealed and its final hash is committed in the successor record, its ops and boundary checkpoint are immutable. Any peer, relay or object store can serve them, verified against one hash. This directly strengthens the prior research's pull-based, hash-chain-verified learner dissemination, and it is independent of heterogeneous Loglets (Finding 5.2).
- **Reconfiguration coverage.** `viewstamp`'s simulator does not yet cover reconfiguration (consensus doc, V3). Adding seal/epoch handoff *extends* the least-tested path, so it must land with deterministic-simulation coverage of seal, check-tail repair, handoff and discovery races.

## Costs and failure modes

| Cost / failure mode | Evidence | Severity for the target |
|---|---|---|
| Per-append latency | +100–150 µs p99 (Delos, attributed to its async API) | Negligible against WAN commits |
| Seal pause | Tens of milliseconds in-DC; p99 spikes during reconfiguration | Low. It happens only at epoch changes |
| Second consensus component (MetaStore) | Delos needed one because its Loglets are not fault-tolerant | Avoidable with option (b) |
| New reconfiguration path to verify | Seal/checkTail repair, zombie appends, roll-forward, chain-CAS races | **High.** It extends `viewstamp`'s least-simulated area |
| Rarely exercised seal path | Delos ran about 98 reconfigurations a month (hot path); a VR-backed seal would run only at upgrades (cold path) | **High** unless epochs roll routinely |
| Roll-forward silently reverting an upgrade | A stalled reconfiguration is completed "by cloning the configuration of the previous active segment" | Medium. The upgrade driver must converge, not fire and forget |
| Cross-segment trim/GC bugs | Restate #5377 (open, September 2026): silent no-op trims, stale chain metadata | Medium |
| Meta-store format becomes a permanent compatibility contract | Delos/CORFU freeze it; Restate pauses to migrate | Medium; design once |
| Semantic roll-outs still dangerous | Delos: engine roll-out was the "only source of inconsistency in production" | **High**, but the version gate addresses it, not the virtual log |
| Licensing of the only Rust prior art | Bifrost BSL 1.1 | Low if only the design is referenced |

**Is the complexity justified for a single-operator orchestrator?**
- The *full* Delos VirtualLog (heterogeneous Loglets + separate MetaStore) is justified mainly by (i) planning to swap consensus implementations in production (e.g. `viewstamp` ↔ openraft), or (ii) wanting disaggregated/object-storage log durability. Neither is in the stated target.
- The *epoch-segmented* subset is justified by two needs that *are* stated: the flag-day wire problem, and learner dissemination at thousands of nodes. It is a smaller increment over VR §7.

## Conflicting information

### Conflict 1: Is the virtual log an alternative to the cluster-version gate, and how long is the seal pause?
**Position A**: The prior repo research (consensus doc A4.5, A4 recommendation 1) presents the virtual log as path "(b)" beside the version gate "(a)". It suggests learners "replay across segments" and a pause "comparable to TigerBeetle's roughly 5 seconds" — Source: `docs/research/architecture/flat-cluster-consensus-and-log-dissemination-comprehensive-research.md` (internal synthesis).
**Position B**: Delos used a log-committed enable flag for semantic changes alongside segment swaps (SOSP'21 §3.4). Its measured reconfiguration latency is "10s of ms" in-DC (OSDI'20 §5.2) — Source: [Delos OSDI'20](https://www.usenix.org/system/files/osdi20-balakrishnan.pdf), [SOSP'21](https://maheshba.bitbucket.io/papers/delos-sosp2021.pdf), reputation 1.0.
**Assessment**: Position B rests on primary peer-reviewed sources. Treat the virtual log as a complement to the gate. The ~5 s figure is TigerBeetle's restart-based upgrade pause, not a seal pause. A cross-region seal would be slower than Delos's in-DC figure; no measurement was found.

### Conflict 2: Delos production scale figures differ between sources
**Position A**: "over 1.8 billion transactions per day" (OSDI'20, written 2020).
**Position B**: "more than 2B TXes/day" as the Twine scheduler's store (ApPLIED talk, 2021); DelosTable "3.3B ops/day" (SOSP'21 contributions) and "more than 3B transactions per day" on 107 clusters (SOSP'21 §5); Zelos "36.5B ops/day" = "6.5B writes and 30B reads".
**Assessment**: Not a contradiction. The figures come from different dates and different scopes (all deployments, one service, one database). Growth over 2020–2021 explains them.

### Conflict 3: "Twine's Resource Broker" vs "Tupperware Resource Broker"
**Position A**: OSDI'20 Delos: "Twine's Resource Broker". **Position B**: SOSP'21 (via Lerner's summary): "Tupperware Resource Broker".
**Assessment**: Same system. Tupperware was Twine's earlier name. No conflict of substance.

## Knowledge gaps

### Gap 1: No independent audit of any virtual log
**Issue**: No Jepsen, Antithesis or formal-verification report was found for Delos (closed source) or Restate Bifrost. All Delos evidence is first-party. | **Attempted**: primary papers, vendor docs, Restate issue tracker. No dedicated search of jepsen.io for Restate was run. | **Recommendation**: Search jepsen.io/antithesis.com for Restate before relying on Bifrost as a design reference for failure handling.

### Gap 2: How Delos migrated its MetaStore from ZooKeeper to the embedded Paxos
**Issue**: The OSDI paper states the migration happened but not how. This is the key evidence for the meta-store upgrade regress (Finding 3.3). | **Attempted**: OSDI'20 §4.1, SOSP'21. | **Recommendation**: Ask the authors, or treat P3 migration as unsolved and design the chain descriptor to be frozen.

### Gap 3: Delos's status after 2021
**Issue**: No public source after 2021 on whether Delos still backs Twine, or has been replaced, was found. The key author has since moved to Confluent (LogDrive, OSDI'26). | **Attempted**: the papers; the ApPLIED 2021 abstract; the At Scale 2021 talk. No targeted search for 2022–2026 Meta posts. | **Recommendation**: Search engineering.fb.com / atscaleconference.com for 2022+ Delos mentions.

### Gap 4: Meta-store outage behaviour in BookKeeper/Pulsar and Bifrost
**Issue**: Their fetched docs do not state whether appends continue when ZooKeeper/Oxia or the Restate metadata store is unavailable. | **Attempted**: BookKeeper protocol page, Pulsar architecture, Restate metadata page. | **Recommendation**: Read the BookKeeper client and Bifrost source, or run a fault-injection experiment.

### Gap 5: Whether the ZK→Native switch required a prior fleet-wide rollout
**Issue**: Finding 3.1 infers this from the converged deployment and client-side routing; the paper does not narrate the rollout. | **Attempted**: OSDI'20 §4–5. | **Recommendation**: Low priority. The architectural argument (clients must speak the new Loglet) holds regardless.

### Gap 6: `viewstamp` seal feasibility at code level
**Issue**: The four primitives in Finding 5.1 are derived from the README and VR §7, not from `viewstamp-proto` code. Whether a quorum-bit seal composes with its view-change and storage-fault repair is unknown. | **Attempted**: README (raw), prior research's docs.rs reading. | **Recommendation**: A code read plus a maintainer conversation, followed by a simulator spike covering seal and handoff under storage faults.

### Gap 7: Conflux internals, Tango/vCorfu, Pravega status
**Issue**: Only the LogDrive abstract/introduction was read (not its seal or metadata sections). Tango and vCorfu were not re-read. Pravega's current CNCF maturity/archival status was not verified. | **Recommendation**: Read LogDrive §3–5 if object-storage durability becomes a goal.

### Gap 8: Cross-region seal latency
**Issue**: No measured seal/reconfiguration latency for a geo-distributed virtual log was found. Delos measured in-DC only. | **Recommendation**: Measure in simulation or a spike; expect roughly one or two cross-region round trips per seal and checkTail, plus the chain write.

## Recommendation for the target design

These are research recommendations for the architect, not decisions.

1. **Make the committed cluster-version gate the non-negotiable baseline.** Nodes advertise supported ranges; the leader commits `ClusterVersion(v)`; new behaviour ships dormant and activates at the committed op; unsupported nodes are fenced. Delos itself needed this (SOSP'21 §3.4). It is the only mechanism that addresses P2, the plane that freezes intent across thousands of learners. Make every entry format tolerant of unknown fields, as Delos's "map of headers" was.
2. **Do not adopt the full Delos VirtualLog (heterogeneous Loglets + separate MetaStore) now.** Its distinctive benefit, a live swap of the consensus *implementation* with live rollback, pays off only if the project expects to change engines in production. Its cost is a second consensus component plus a new reconfiguration path in the least-simulated area of `viewstamp`. Re-evaluate if either (a) switching between `viewstamp` and openraft in a running cluster becomes a goal, or (b) disaggregated/object-storage log durability (Conflux-style) becomes a goal.
3. **Evaluate an epoch-segmented VR log ("Delos-lite") as the P1 answer for flag-day `viewstamp`**, as an alternative to building in-protocol wire negotiation:
   - each VR epoch (VR §7) is a segment pinned to one wire version;
   - the successor descriptor is committed as the epoch's final op (the seal);
   - the successor group starts from a content-addressed boundary checkpoint fetched by hash, so old and new wire versions never exchange messages;
   - binaries carry N−1/N codecs selected *per epoch*, not per connection. This is simpler than per-connection negotiation.

   Before relying on it, specify and simulate `seal`, `check_tail`, sealed-read and boundary checkpoint (Finding 5.1).
4. **Use option (b) for meta-consensus; avoid a separate MetaStore and any external store.** Keep the chain in-band. Freeze a minimal, forward-extensible chain-descriptor format (epoch id, wire version, voter set, opaque config, predecessor hash) as a permanent contract. Discover the chain head through gossiped facts (latest epoch + hash) that are verifiable against committed successor records.
5. **Exploit sealed-epoch immutability for learner dissemination.** Serve sealed epochs from any peer, a relay tree or object storage, verified by hash. This is the part of the virtual-log idea most valuable at thousands of learners, and it is valuable even without heterogeneous Loglets.
6. **Keep the seal path hot.** Roll epochs routinely (on every voter-set change, and on a size/time schedule like Pulsar's ledger rollover), so the seal/handoff path runs daily, as Delos's did, rather than only at upgrades. Put seal, roll-forward and discovery races under the VOPR-style deterministic simulator from the first commit. Treat an interrupted upgrade cut-over as a converging desired-vs-installed state, because a rolled-forward reconfiguration may silently keep the old version.
7. **Do not expect multi-region write locality from segmentation.** If per-region intent throughput or latency becomes binding, it needs per-region logs plus a global decision log (hierarchy), which is a separate architectural decision. Meanwhile, planned epoch reconfiguration can move the voter set or leader between regions, and sealed epochs can be cached per region.
8. **Treat Restate Bifrost as a design reference only (BSL 1.1).** Read its replicated-loglet and trim handling (including issue #5377) for failure-mode lessons. Do not copy code.

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|---|---|---|---|---|---|
| Delos OSDI'20 paper + presentation page | usenix.org | High (1.0) | academic | 2026-09-25 | Y |
| Delos SOSP'21 (author copy of ACM paper) + ACM DL entry | maheshba.bitbucket.io / dl.acm.org | High (1.0) | academic | 2026-09-25 | Y |
| LogDrive / Conflux, OSDI'26 | usenix.org | High (1.0) | academic | 2026-09-25 | Partial (lineage-dependent) |
| CORFU NSDI'12; Scalog NSDI'20; FuzzyLog OSDI'18; Twine OSDI'20 | usenix.org | High (1.0) | academic | 2026-09-25 | Y |
| VR Revisited (MIT TR, via HTML mirror) | ying-zhang.cn (mirror of dspace.mit.edu) | High (1.0) content; mirror | academic | 2026-09-25 | Y (prior research) |
| BookKeeper protocol; Pulsar architecture; Pulsar tiered storage | apache.org | High (1.0) | official OSS | 2026-09-25 | Y |
| ApPLIED 2021 speakers (Delos talk abstract) | chalmers.se | Medium-High (0.8) | academic workshop | 2026-09-25 | Y |
| Meta At Scale 2021 talk abstract | atscaleconference.com | Medium-High (0.8) | first-party, out-of-list primary | 2026-09-25 | Y |
| LogDevice Concepts; LogDevice Recovery | logdevice.io | Medium-High (0.8) | out-of-list primary | 2026-09-25 | Y |
| facebookarchive/LogDevice; restatedev/restate LICENSE, #5377; al8n/viewstamp README | github.com | Medium-High (0.8) | OSS primary | 2026-09-25 | Y |
| Restate architecture; Restate metadata; Restate blog | docs.restate.dev / restate.dev | Medium-High (0.8) | out-of-list primary | 2026-09-25 | Y |
| Pravega concepts | cncf.pravega.io | Medium-High (0.8) | out-of-list primary | 2026-09-25 | Partial |
| Vanlightly ×4 (Delos intro, "Steady on", LogDrive, strong vs weak sealing) | jack-vanlightly.com | Medium (0.6) | practitioner (LogDrive co-author) | 2026-09-25 | Corroboration only |
| Lerner, SOSP'21 summary | micahlerner.com | Medium (0.6) | secondary | 2026-09-25 | Corroboration only |

Reputation: High: 11 (38%) | Medium-High: 13 (45%) | Medium: 5 (17%) | Avg ≈ 0.84 (29 cited external sources; the three internal repo research documents are not counted).

**Bias notes**: The Delos, CORFU, FuzzyLog and LogDrive papers share an author (Balakrishnan) and advocate the shared-log approach, so they count as one lineage for independence. Restate and Confluent sources are vendor-authored. Their claims are used only for their own systems' behaviour. The Delos "10X" claim is self-qualified by the authors as largely due to an unoptimized baseline.

## Full Citations

[1] Balakrishnan, M.; Flinn, J.; Shen, C.; et al. "Virtual Consensus in Delos". OSDI'20. 2020. https://www.usenix.org/system/files/osdi20-balakrishnan.pdf (presentation: https://www.usenix.org/conference/osdi20/presentation/balakrishnan). Accessed 2026-09-25.
[2] Balakrishnan, M.; Shen, C.; Jafri, A.; et al. "Log-structured Protocols in Delos". SOSP'21. 2021. https://maheshba.bitbucket.io/papers/delos-sosp2021.pdf ; ACM DL https://dl.acm.org/doi/10.1145/3477132.3483544. Accessed 2026-09-25.
[3] Vickers, G.; Bradstreet, L.; Balakrishnan, M.; et al. "The LogDrive: Composable Durability for Cloud-Based Shared Logs". OSDI'26. 2026. https://www.usenix.org/system/files/osdi26-vickers.pdf. Accessed 2026-09-25.
[4] Balakrishnan, M.; Malkhi, D.; Prabhakaran, V.; et al. "CORFU: A Shared Log Design for Flash Clusters". NSDI'12. 2012. https://www.usenix.org/system/files/conference/nsdi12/nsdi12-final30.pdf. Accessed 2026-09-25.
[5] Ding, C.; et al. "Scalog: Seamless Reconfiguration and Total Order in a Scalable Shared Log". NSDI'20. https://www.usenix.org/conference/nsdi20/presentation/ding. Accessed 2026-09-25.
[6] Lockerman, J.; et al. "The FuzzyLog: A Partially Ordered Shared Log". OSDI'18. https://www.usenix.org/system/files/osdi18-lockerman.pdf. Accessed 2026-09-25.
[7] Tang, C.; et al. "Twine: A Unified Cluster Management System for Shared Infrastructure". OSDI'20. https://www.usenix.org/conference/osdi20/presentation/tang. Accessed 2026-09-25.
[8] Liskov, B.; Cowling, J. "Viewstamped Replication Revisited", §6.1, §7. MIT-CSAIL-TR-2012-021. 2012. https://dspace.mit.edu/handle/1721.1/71763 (text via http://ying-zhang.cn/dist/2012-vr.html). Accessed 2026-09-25.
[9] Apache BookKeeper. "The BookKeeper protocol". https://bookkeeper.apache.org/docs/development/protocol. Accessed 2026-09-25.
[10] Apache Pulsar. "Architecture overview". https://pulsar.apache.org/docs/next/concepts-architecture-overview/. Accessed 2026-09-25.
[11] Apache Pulsar. "Tiered storage overview". https://pulsar.apache.org/docs/next/tiered-storage-overview/. Accessed 2026-09-25.
[12] Balakrishnan, M. "Virtual Consensus in the Delos Storage System" (talk abstract). ApPLIED Workshop @ PODC'21. https://www.cse.chalmers.se/~elad/ApPLIED2021/speakers.html. Accessed 2026-09-25.
[13] Balakrishnan, M. "Virtualizing Consensus in Delos for Rapid Upgrades and Happy Engineers". Meta @Scale. 2021-03-17. https://atscaleconference.com/2021/03/15/virtualizing-consensus/. Accessed 2026-09-25.
[14] LogDevice. "Architecture (Concepts)". https://logdevice.io/docs/Concepts.html. Accessed 2026-09-25.
[15] LogDevice. "Recovery after the failure of a sequencer". https://logdevice.io/docs/Recovery.html. Accessed 2026-09-25.
[16] Facebook. "facebookarchive/LogDevice" README (archived 2022-01-07). https://github.com/facebookarchive/LogDevice. Accessed 2026-09-25.
[17] Restate. "Restate Architecture". https://docs.restate.dev/references/architecture. Accessed 2026-09-25.
[18] Restate. "Metadata Storage". https://docs.restate.dev/server/metadata. Accessed 2026-09-25.
[19] Restate. "Building a modern Durable Execution Engine from First Principles" (Bifrost sections only). 2025-02-20. https://restate.dev/blog/building-a-modern-durable-execution-engine-from-first-principles. Accessed 2026-09-25.
[20] Restate. LICENSE (Business Source License 1.1). https://github.com/restatedev/restate/blob/main/LICENSE. Accessed 2026-09-25.
[21] Restate. Issue #5377 "Log trimming can silently do nothing when replicated-loglet clients have not discovered their tails". 2026-09-21. https://github.com/restatedev/restate/issues/5377. Accessed 2026-09-25.
[22] Restate. Issue #2027 "Enable cluster controller to control replicated loglet" (title only). https://github.com/restatedev/restate/issues/2027. Seen 2026-09-25.
[23] Pravega. "Pravega Concepts". https://cncf.pravega.io/docs/latest/pravega-concepts/. Accessed 2026-09-25.
[24] al8n. "viewstamp" README. https://raw.githubusercontent.com/al8n/viewstamp/main/README.md. Accessed 2026-09-25.
[25] Vanlightly, J. "An Introduction to Virtual Consensus in Delos". 2025-02-05. https://jack-vanlightly.com/blog/2025/2/5/an-introduction-to-virtual-consensus-in-delos. Accessed 2026-09-25.
[26] Vanlightly, J. "Steady on! Separating Failure-Free Ordering from Fault-Tolerant Consensus". 2025-02-06. https://jack-vanlightly.com/blog/2025/2/6/steady-on-separating-failure-free-ordering-from-fault-tolerant-consensus. Accessed 2026-09-25.
[27] Vanlightly, J. "The LogDrive: Flexible Composition Through Abstraction in Shared Logs". 2026-08-25. https://jack-vanlightly.com/blog/2026/8/25/the-logdrive-flexible-composition-through-abstraction-in-shared-logs. Accessed 2026-09-25.
[28] Vanlightly, J. "The AtomicLog + LogDrive: Strong vs weak sealing". 2026-08-28. https://jack-vanlightly.com/blog/2026/8/28/the-atomiclog-logdrive-strong-vs-weak-sealing. Accessed 2026-09-25.
[29] Lerner, M. "Log-structured Protocols in Delos" (paper review). 2021-11-23. https://www.micahlerner.com/2021/11/23/log-structured-protocols-in-delos.html. Accessed 2026-09-25.
[30] Internal: `docs/research/architecture/flat-cluster-consensus-and-log-dissemination-comprehensive-research.md`, `...membership-discovery-and-admission...`, `...placement-and-observation...` (2026-09-25).

## Research Metadata

Duration: ~50 turns | Examined: ~36 sources | Cited: 29 external + 3 internal | Cross-refs: every Delos mechanism claim checked against at least one sibling system or a second first-party source | Confidence: High ≈ 55%, Medium-High ≈ 35%, Medium ≈ 10% of findings | Tool notes: the ACM DL PDF returned 403 (the author-hosted copy was used); the usenix.org and bitbucket PDFs were read page-by-page; DeepWiki was consulted only to locate Restate's metadata backends, which were then confirmed on docs.restate.dev and are not cited | Output: `docs/research/architecture/delos-style-virtual-log-comprehensive-research.md`
