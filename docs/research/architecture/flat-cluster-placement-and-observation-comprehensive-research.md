# Research: Placement Through Consensus, Coordination Avoidance, and the Observation Layer for a Flat-Cluster Orchestrator

**Date**: 2026-09-25 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium-High overall (High for C1–C4 and C6; Medium-High for C5 alternatives, which lack at-scale production evidence) | **Sources**: 58 distinct

**Scope**: Problem cluster C — deciding placement (C1), deciding which operations need consensus at all (C2), how workers learn assignments (C3), explicit vs automatic placement (C4), the observation layer (C5), and location-transparent component ports (C6). This document researches a *target* design; it does not evaluate the current codebase, and it does not treat `docs/whitepaper.md` as authoritative.

**Relation to prior repo research** (cited, not repeated):
- `docs/research/scalability/corrosion-crsqlite-production-scale-evidence.md` — Corrosion fleet size (~800 servers / ~40 regions), the 2024 incident corpus (contagion deadlock, nullable-column backfill storm, 150 GB/s backlog storm), backpressure issue #198, Consul gossip ceilings (≤5,000 agents/pool recommended; 66k tested).
- `docs/research/orchestration/crash-observability-under-lww-comprehensive-research.md` — LWW discards intermediate values; bounded "last-state + counter" shape; causal-stability requirement for pruning a gossiped log.
- `docs/research/orchestration/nomad-scheduling-and-reconciler-pattern-research.md` — Nomad plan queue / optimistic concurrency (F1.6), Raft mediating every mutation (F1.8), eval-broker load shedding (F1.9), Fly market-model scheduler (F3.3).
- `docs/research/platform/fly-inside-out-orchestration-overdrive-relevance.md` — Sprites "inside-out" (in-guest orchestration). Note: that is a *different* sense of "inside-out" from flyd's worker-owned state; C3 below covers the flyd sense.
- Sibling document (cluster A, same date): `docs/research/architecture/flat-cluster-consensus-and-log-dissemination-comprehensive-research.md` — consensus protocol choice, voters vs learners, log dissemination. This document does not re-derive those; it takes "a small voter set orders a log, learners hold it" as given and asks what goes *into* the log.

## Executive Summary

**Placement (C1).** The evidence supports committing placement *decisions*, not placement *inputs*.
- Deterministic replay (Calvin) is safe only when every replica runs identical code. TigerBeetle shows the price: a cluster-wide, coordinated version switch. That is a poor fit for a scheduler that changes often.
- Optimistic shared-state scheduling — Omega, Nomad's plan queue, Meta's Twine, Azure's Protean — is the production-dominant *refinement* of "leader commits". Many schedulers compute; one verify-and-apply step is serialized. Published committed-decision schedulers reach ~1,000–1,500 placements per second, and Twine scales further by sharding the decision space.
- Decentralized sampling (Sparrow) drops exactly what an orchestrator needs: anti-affinity, bin-packing, gang scheduling.
- Mesos resource offers were retired with Apache Mesos in 2025.
- Escrow (bounded counters, Twine entitlements) is the strongest complement, but it enforces only numeric invariants.

**Which operations need consensus (C2).** The CALM theorem and invariant-confluence analysis give an operation-by-operation test.
- Operator-chosen names and VIPs, replica counts, anti-affinity, rollout progress, CAS updates, and "node is down" need ordering.
- Per-origin facts, grow-only sets, monotone counters, auto-generated IDs, and addresses drawn from a pre-granted block do not.
- Keep CALM and CRDT On (VLDB 2023) adds that a *read* of converged CRDT state can still be unsafe when it is non-monotone. That is why "no heartbeat ⇒ down" must be logged even when heartbeats are gossiped.

**Assignments and partitions (C3).** Kafka KRaft is a production precedent for the exact shape: brokers are non-voting observers that fetch and materialize the metadata log. Nomad issue #18267 shows the hazard of pulling from a lagging replica: a stale server made clients stop and garbage-collect running allocations. Nomad, Kubernetes, and Twine keep committed work running across a control-plane partition by default and make replacement a separate, delayed decision. Kafka and Nomad's `stop_on_client_after` show the self-fencing alternative for at-most-one workloads.

**Explicit vs automatic placement (C4).** Borg, Twine, Kubernetes, and Nomad all spread across the failure-domain hierarchy by default and treat explicit placement as attribute constraints. Machine-name pinning is fragile. AWS partition placement groups show a middle ground: explicit failure-domain *slots*.

**Observation (C5).** Corrosion is actively developed in 2026, with a Plumtree broadcast loop and supervised actors, but has no tagged release since v1.0.0 (May 2024). Its backpressure issue is closed without a visible PR, and tombstone garbage collection is an open problem (#533) that the maintainers expect to cost availability. Three large systems (Cassandra CEP-21, ScyllaDB 6.0, Kafka KRaft) moved correctness-bearing membership and ownership metadata from gossip to a log and kept gossip for liveness input and "non-correctness impacting" state. Consul, Nomad, and Kubernetes push per-node facts *through* consensus and plateau near ~5k nodes.

**Verdict on the hypothesis.** The hypothesis "gossip carries facts, the log carries decisions" survives as a scaling strategy but needs three amendments:
1. Classify by correctness impact, not by fact vs decision.
2. Let the log carry *grants* (address blocks, quota shares) so that many decisions become local.
3. Never give a fact a second writer. Commit ownership transfer as a fencing epoch in the log.

**Ports (C6).** Waldo et al. (1994), Orleans' at-most-once default, NATS' "no responders" signal, and wasmCloud's 2025 turn from implicit to intentional remoting all say the same thing: a port that may be remote must carry deadlines, a three-way outcome (`Ok` / `DefinitelyNotExecuted` / `OutcomeUnknown`), declared idempotency and delivery semantics, and a staleness index.

**Top unexplored alternatives:**
1. Epoch-fenced single-writer facts, with ownership transfer committed in the log.
2. Escrow grants for capacity, quota, IDs, and addresses.
3. Astrolabe-style hierarchical aggregation for an observation layer spanning thousands of nodes.

## Research Methodology

**Search Strategy**:
- Read the four named prior repo documents and the sibling cluster-A skeleton first, and cite them rather than repeat them.
- For each of C1–C6, fetch primary sources directly: USENIX/ACM/arXiv papers (reading PDF pages where HTML extraction failed), official docs (kubernetes.io, developer.hashicorp.com, docs.nats.io, learn.microsoft.com, docs.aws.amazon.com, cwiki.apache.org), and GitHub repositories and issue trackers for latest-state claims (Corrosion, chitchat, Hydro, Nomad, TigerBeetle).
- Use web search to discover alternatives outside the seed list: Twine, Protean, Hawk, CEP-21, ScyllaDB Raft topology, KRaft, Rateless IBLT, AWS partition placement groups, `bcounter`.
**Source Selection**: Types: academic, official, technical_docs, industry | Reputation: medium-high minimum | Verification: independent cross-reference per claim
**Quality Standards**: Target 3 sources/claim (min 1 authoritative) | All major claims cross-referenced | Avg reputation: ≈0.93 (see Source Analysis)

**Adversarial validation**: every fetched page was treated as untrusted input. One fetch (Kubernetes virtual-IPs page) initially echoed the prompt's own wording back as a "quote"; it was discarded, and the page was re-fetched for verbatim text. Pages that returned tool summaries rather than verbatim text are flagged inline and in Knowledge Gap 4. Out-of-list primary sources (hydro.run, scylladb.com, quickwit.io, datadoghq.com, cs.umd.edu, cs.cornell.edu, gatech.edu mirror, allthingsdistributed.com, developers.redhat.com, hashicorp.com blog) are marked as such and scored 0.8 or less unless they are author copies of peer-reviewed papers.

## Target design under examination (restated)

- **Topology**: flat; every node HA by default; the cluster grows by joining a node, from one node to thousands, possibly across regions.
- **Per-node components**: gateway, web, control-plane, worker, telemetry, wasm, and storage. All are enabled by default, and each port is satisfied by a local or a remote adapter.
- **Discovery**: gossip and peer exchange over a WireGuard mesh.
- **Intent**: an ordered replicated log with a small voter set (3–5); every other node is a non-voting learner holding the full log. Reads are local and writes are forwarded to the leader. The leader runs the scheduler and commits placement *decisions*. Workers learn their assignments by reading their local replica.
- **Observation**: high-volume, per-node-owned facts kept off the log and available under partition. Corrosion is the incumbent; per-origin append-only logs with Scuttlebutt anti-entropy are the considered alternative.
- **Trust**: a single operator, trusted nodes, crash faults only.
- **Hypothesis under test**: "gossip carries facts, the log carries decisions."

## C1. Placement decided through consensus

The question is not "consensus or not" but **what is replicated**: the placement *inputs* (and every replica recomputes), or the placement *outputs* (one party computes, the log carries the result). The six options fall on that spectrum.

| Option | What the log carries | Who computes placement | Divergence risk under mixed versions |
|---|---|---|---|
| (a) Leader computes, commits decision | Output ("W#2 → M") | Leader only | None — followers apply data, not code |
| (b) Deterministic execution (Calvin) | Input ("schedule W") | Every replica | High — replicas must run identical code |
| (c) Optimistic shared state (Omega/Nomad/Twine) | Output, after a serialized verify step | Many parallel schedulers | None at apply; conflicts retried |
| (d) Decentralized sampling (Sparrow) | Nothing global | Each scheduler, probing workers | N/A — no shared record |
| (e) Node-owned capacity / escrow (Mesos offers, bounded counters) | Grant of rights (rarely) | Worker admits locally within its grant | None — local decision |
| (f) Every node proposes, consensus picks | Winning proposal | Every proposer | Low if output committed |

### Finding C1.1: Deterministic execution replicates *inputs* and therefore requires every replica to execute identical, deterministic logic

**Evidence**: Calvin's abstract: "By replicating transaction inputs rather than effects, Calvin is also able to support multiple consistency levels — including Paxos-based strong consistency across geographically distant replicas — at no cost to transactional throughput." Section 1.3: "Both parallel plan execution and replay of plan history require activity plans to be deterministic — otherwise replicas might diverge or history might be repeated incorrectly." Section 2: "simply replicating transactional input is not generally sufficient to ensure that replicas do not diverge … two replicas may choose to process the input in manners equivalent to different serial orders."
**Source**: [Thomson et al., "Calvin: Fast Distributed Transactions for Partitioned Database Systems", SIGMOD 2012 (author PDF, cs.umd.edu)](https://www.cs.umd.edu/~abadi/papers/calvin-sigmod12.pdf) — Accessed 2026-09-25. Out-of-list primary (author copy of an ACM SIGMOD paper); score 1.0 as peer-reviewed.
**Confidence**: High
**Verification**: TigerBeetle's architecture document (prior repo research, `nomad-scheduling-and-reconciler-pattern-research.md` F3.5): "Because all replicas start with the same (empty) state and the state transition function is deterministic, the replicas arrive at the same state" ([tigerbeetle ARCHITECTURE.md](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/ARCHITECTURE.md)). TigerBeetle's upgrade guide shows the operational consequence — the version switch must itself be coordinated: "this will restart with the new binary available, but still running the older version. TigerBeetle will then coordinate the actual upgrade when all replicas are ready and have the latest version available" ([TigerBeetle upgrading.md](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/operating/upgrading.md), accessed 2026-09-25).
**Analysis**: This is the formal basis for the current direction's choice (a) over (b). If the log carries "schedule W", every learner re-runs the scheduler; a scheduler bug fix, a changed scoring weight, or a new constraint type in a rolling upgrade produces *different placements on different replicas from the same log*. The only safe forms of (b) are (i) a cluster-wide, log-ordered version switch (TigerBeetle's multiversion binary — every replica carries old and new code and switches at a coordinated point), or (ii) a placement function so small and frozen that it never changes. An orchestrator scheduler (constraints, spread, bin-packing, preemption) changes often; it is the worst candidate for (b). Calvin also needs read/write sets declared before execution, which a scheduler scanning cluster capacity cannot supply cheaply.

### Finding C1.2: Optimistic shared-state scheduling is the production-dominant form of "commit the decision", and conflict rates are empirically low

**Evidence**: Omega (EuroSys 2013) proposes "parallelism, shared state, and lock-free optimistic concurrency control" as the answer to monolithic-scheduler limits, and evaluates "how much interference between schedulers occurs and how much it matters in practice." Meta's Twine allocator: "uses multiple threads to perform concurrent allocations for different jobs, and relies on optimistic concurrency control to resolve conflicts. Before committing an allocation, a thread verifies that all impacted machines still have sufficient resources left for the allocation. If the verification fails, it retries a different allocation." Microsoft Azure's Protean allows "multiple AAs [allocation agents] to run concurrently on the same inventory, resulting in increased throughput … with negligible conflict rate."
**Source**: [Schwarzkopf et al., "Omega: flexible, scalable schedulers for large compute clusters", EuroSys 2013 (research.google)](https://research.google/pubs/omega-flexible-scalable-schedulers-for-large-compute-clusters/); [Tang et al., "Twine: A Unified Cluster Management System for Shared Infrastructure", OSDI 2020](https://www.usenix.org/conference/osdi20/presentation/tang); [Hadary et al., "Protean: VM Allocation Service at Scale", OSDI 2020](https://www.usenix.org/conference/osdi20/presentation/hadary) — all accessed 2026-09-25.
**Confidence**: High (three independent organizations — Google, Meta, Microsoft — peer-reviewed)
**Verification**: Nomad's plan queue is the same pattern (prior research F1.6): parallel scheduling workers, the leader's plan applier "checks for over-subscription, and does partial or complete rejections" ([Scheduling in Nomad](https://developer.hashicorp.com/nomad/docs/concepts/scheduling/scheduling)).
**Analysis**: Option (c) is a *refinement* of (a), not an alternative to it. The commit is still "decision" not "input"; what changes is that computation is spread across many scheduler workers and only a cheap verify-and-apply step is serialized through the leader. For the target design this matters because it decouples scheduling throughput from leader CPU: learners (or dedicated scheduler-role nodes) can compute plans against their local replica and submit them; the leader only validates capacity against the latest committed state and commits. Upgrade safety is preserved because the log still carries outputs.

### Finding C1.3: Published throughput of committed-decision schedulers — 1,000–1,500 placements/s class, with sharding as the scale-out lever

**Evidence**: Nomad's C2M benchmark: "Nomad scheduled 2 million containers in 22 minutes and 14 seconds at an average rate of nearly 1,500 containers per second" on 6,100 hosts across 10 AWS regions with 3 schedulers. Twine: "At its peak, a large allocator performs ≈1,000 job allocations per second, with an average job size of 36 tasks." Twine shards schedulers by entitlement: "The largest shard manages ≈170K machines … Assuming each shard manages 50K machines in the future, a single Twine deployment can manage 1M machines with 20 shards." Firmament (centralized min-cost-flow) "scales to over ten thousand machines at sub-second placement latency."
**Source**: [HashiCorp, "HashiCorp Nomad Meets the 2 Million Container Challenge"](https://www.hashicorp.com/en/blog/hashicorp-nomad-meets-the-2-million-container-challenge) (vendor, out-of-list primary; configs at [github.com/hashicorp/c2m](https://github.com/hashicorp/c2m)); [Twine, OSDI 2020, §3.1](https://www.usenix.org/system/files/osdi20-tang.pdf); [Gog et al., "Firmament: Fast, Centralized Cluster Scheduling at Scale", OSDI 2016](https://www.usenix.org/conference/osdi16/technical-sessions/presentation/gog) — accessed 2026-09-25.
**Confidence**: Medium-High (three independent sources; Nomad figure is a vendor benchmark, not a production measurement)
**Verification**: Twine and Firmament are peer-reviewed; the Nomad benchmark's configurations are public on GitHub.
**Analysis**: A single leader committing decisions is not the throughput bottleneck for an orchestrator whose workloads are long-running microVMs: even the most demanding published committed-decision scheduler needs ~10³ placements/s. Twine's lesson is that when it does bind, the lever is **sharding the decision space** (by entitlement/tenant), not moving to (b) or (d). This maps onto cluster A's "write scaling beyond one group" (sibling doc A6).

### Finding C1.4: Decentralized sampling (Sparrow) buys sub-10 ms latency by giving up exactly the constraints an orchestrator needs

**Evidence**: Sparrow "provides response times within 12% of an ideal scheduler, schedules with median queueing delay of less than 9ms." But: "Sparrow does not allow certain types of placement constraints (e.g., 'my job should not be run on machines where User X's jobs are running'), does not perform bin packing, and does not support gang scheduling." And it assumes "a long-running executor process is already running on each worker machine … These executor processes may be launched within a static portion of a cluster, or via a cluster resource manager (e.g., YARN, Mesos, Omega)."
**Source**: [Ousterhout et al., "Sparrow: Distributed, Low Latency Scheduling", SOSP 2013](https://sigops.org/s/conferences/sosp/2013/papers/p69-ousterhout.pdf) (ACM DOI [10.1145/2517349.2522716](https://dl.acm.org/doi/10.1145/2517349.2522716)) — accessed 2026-09-25.
**Confidence**: High (primary peer-reviewed source; the limitation is the authors' own statement)
**Verification**: Firmament (OSDI 2016) independently reports it "matches the placement latency of distributed schedulers for workloads of short tasks" while exceeding their placement quality — i.e., the latency advantage of decentralization disappears at 10k-machine scale, and the quality gap remains.
**Analysis**: Sparrow's model is sub-second analytics tasks launched into pre-provisioned executors. Overdrive places long-lived microVMs with anti-affinity, spread, and capacity invariants — precisely the features Sparrow drops. Sampling is useful as a *heuristic inside* a committed-decision scheduler (probe d candidates rather than scan all), not as the source of truth. Hawk reaches the same conclusion from the other side: "Long jobs are scheduled using a centralized scheduler, while short ones are scheduled in a fully distributed way," with "a small portion of the cluster … reserved for the use of short jobs" ([Delgado et al., "Hawk: Hybrid Datacenter Scheduling", USENIX ATC 2015](https://www.usenix.org/conference/atc15/technical-session/presentation/delgado), accessed 2026-09-25). Overdrive's workloads are all "long jobs" in Hawk's taxonomy.

### Finding C1.5: Escrow / bounded counters let a node admit locally within a pre-granted share, with consensus only on the (rare) rebalancing of rights

**Evidence**: "We present a new replicated data type, called bounded counter, which adds support for numeric invariants to eventually consistent geo-replicated databases … Our approach adapts ideas from escrow transactions to devise a solution that is decentralized, fault-tolerant and fast. Our evaluation shows much lower latency and better scalability than the traditional approach of using strong consistency to enforce numeric invariants."
**Source**: [Balegas et al., "Extending Eventually Consistent Cloud Databases for Enforcing Numeric Invariants", SRDS 2015 (arXiv:1503.09052)](https://arxiv.org/abs/1503.09052) — accessed 2026-09-25.
**Confidence**: Medium-High (peer-reviewed; production use of the exact data type not found)
**Verification**: Twine's *entitlements* are an industrial escrow at coarser grain — "Conceptually, an entitlement is a pseudo cluster … An entitlement grants a business unit a quota … A machine is either free or assigned to an entitlement, and it can be dynamically reassigned" ([Twine §2.2](https://www.usenix.org/system/files/osdi20-tang.pdf)). A 2026 Rust implementation exists: `bcounter`, "An escrow bounded counter for distributed capacity quotas, plus a map of them for hierarchical (path) quotas" — early-stage, no production users stated ([github.com/kostja/bcounter](https://github.com/kostja/bcounter), [docs.rs/bcounter](https://docs.rs/bcounter/latest/bcounter/)).
**Analysis**: This is the strongest *unexplored* alternative for C1 — but note what it actually solves. Escrow keeps a **numeric invariant** (sum of usage ≤ capacity) without per-operation coordination. A machine's capacity is *already* naturally escrowed: only the machine can run the workload, so the machine can always refuse. What escrow cannot do is choose *which* machine — spread, anti-affinity, and "exactly N replicas" are not numeric invariants on one counter. The viable hybrid is: the leader commits placement decisions (a/c), and the worker treats a committed assignment as an *offer it may refuse* when its local reality (a failed disk, a capacity mis-estimate) disagrees; the refusal is a fact that re-triggers scheduling. This is Mesos' two-level idea inverted (see C1.6).

### Finding C1.6: Resource offers (two-level scheduling) split "how much" from "which" — and the reference implementation has been retired

**Evidence**: Mesos "introduces a distributed two-level scheduling mechanism called resource offers. Mesos decides *how many* resources to offer each framework, while frameworks decide *which* resources to accept and which computations to run on them … While this decentralized scheduling model may not always lead to globally optimal scheduling, we have found that it performs surprisingly well in practice." Scaled to "50,000 (emulated) nodes." Status in 2025: the Apache Mesos PMC voted to retire the project for inactivity; it "retired in August 2025 and the move to the Attic was completed in October 2025."
**Source**: [Hindman et al., "Mesos: A Platform for Fine-Grained Resource Sharing in the Data Center", NSDI 2011](https://www.usenix.org/conference/nsdi11/mesos-platform-fine-grained-resource-sharing-data-center) (author PDF [berkeley.edu](https://people.eecs.berkeley.edu/~alig/papers/mesos.pdf)); [The Apache Attic — Mesos](https://attic.apache.org/projects/mesos.html) — accessed 2026-09-25.
**Confidence**: High (peer-reviewed + foundation record)
**Verification**: The Omega paper (C1.2) was written explicitly as the shared-state alternative to Mesos' offer model; Kubernetes, Nomad, and Twine all converged on shared-state + committed decisions rather than offers.
**Analysis**: Offers solve *multi-framework* sharing (many independent schedulers over one pool). Overdrive has one scheduler; the offer model's benefit does not apply, and its costs (offer hoarding, frameworks seeing a partial view) do. Its enduring idea — the resource owner has the last word — survives in C1.5's "committed assignment is an offer the worker may refuse."

### Finding C1.7: "Every node proposes, consensus picks" — in a crash-fault model this reduces to multi-leader ordering plus deterministic conflict resolution, i.e., option (c)

**Evidence**: Mencius (OSDI 2008) partitions "the sequence of values to be decided … in a round-robin fashion, assigning a different process as the leader for each partition", derived from Paxos, to get high throughput in WANs without a single leader bottleneck.
**Source**: [Mao, Junqueira, Marzullo, "Mencius: Building Efficient Replicated State Machines for WANs", OSDI 2008 (ACM DL)](https://dl.acm.org/doi/10.5555/1855741.1855767) — accessed 2026-09-25 (abstract-level; full text not fetched).
**Confidence**: Medium (one peer-reviewed source for the mechanism; the reduction argument below is analysis)
**Verification**: Compartmentalized Paxos (Whittaker et al., [arXiv:2012.15762](https://arxiv.org/pdf/2012.15762)) surveys multi-leader designs as throughput techniques, not as a change in what is replicated.
**Analysis (interpretation)**: Blockchain re-execution exists because validators do not trust the proposer (Byzantine model); every validator re-runs the transaction to check it. Under the target's CFT, single-operator trust model that reason disappears: a follower has no need to verify the leader's placement computation. "Every node proposes" then only buys (i) parallel computation — which (c) already provides — and (ii) leader-bottleneck relief — which multi-leader ordering (Mencius, EPaxos) provides at the *ordering* layer, not the placement layer. Two nodes proposing conflicting placements for the same capacity still need a deterministic tie-break at apply time; that tie-break is Nomad's plan applier. Option (f) is therefore not a distinct design point for Overdrive; it is (c) with a multi-leader log.

### Finding C1.8: The committed-decision model degrades gracefully under partition — running work continues, only *new* decisions stop

**Evidence**: Twine's reliability principles: "**Tasks keep running:** Even if all Twine components fail, existing tasks continue to run. New jobs cannot be created and existing tasks cannot be updated until Twine recovers. If a DC is partitioned from the scheduler, existing tasks in the DC continue to run." And: "**Rate-limit destructive operations:** … a bug or fault might cause Twine to perform a large number of destructive operations quickly … We protect against this failure by ensuring all components have fail-safe mechanisms to rate-limit destructive operations."
**Source**: [Twine, OSDI 2020, §4](https://www.usenix.org/system/files/osdi20-tang.pdf) — accessed 2026-09-25.
**Confidence**: High (primary; corroborated for Nomad and Kubernetes in C3)
**Verification**: See C3.2 (Nomad `disconnect` block) and C3.3 (Kubernetes node lifecycle) — both keep running work in place across a control-plane partition.
**Analysis**: Correctness under partition for option (a)/(c): a minority side cannot commit placements, so it cannot double-place; it can only keep running what was already committed. That is the correct CAP choice for *decisions*. The failure mode to design for is the opposite one — a majority side declaring the minority's nodes dead and re-placing their workloads while the originals still run (the "lost node" problem, C3.2/C5.6). Option (e) (escrow) is the only option that keeps *admitting new work* in a minority partition, and only within pre-granted rights.

### C1 synthesis

| Criterion | (a) leader commits | (b) deterministic replay | (c) optimistic shared state | (d) sampling | (e) escrow/offers | (f) all propose |
|---|---|---|---|---|---|---|
| Throughput | 10³/s class; leader CPU-bound | Every replica pays full cost | 10³/s+; scales with scheduler workers | 10⁵–10⁶/s (short tasks) | Local, unbounded | ≈ (c) |
| Correctness under partition | Minority stops deciding; no double placement | Same as (a) for ordering | Same as (a) | No global invariant at all | Minority keeps admitting within grants | ≈ (c) |
| Upgrade safety | Safe (data, not code, is replicated) | Unsafe without cluster-wide version switch | Safe | N/A | Safe | Safe if output committed |
| Constraint expressiveness | Full | Full | Full | Minimal (no anti-affinity, bin-packing, gang) | Numeric only | Full |
| Production evidence | Nomad, K8s | Calvin (research DB), TigerBeetle (not schedulers) | Omega, Nomad, Twine, Protean | Sparrow (research) | Twine entitlements; bounded counters (research) | None for schedulers |

**Reading**: (a) is correct as the *commit* shape; (c) is the natural evolution of (a) once scheduler CPU matters, and it changes nothing in the log format. (e) is a complementary layer for capacity/quota invariants and minority-side admission, not a replacement.


## C2. Coordination avoidance — which operations need consensus at all?

### Finding C2.1: CALM — an operation has a coordination-free consistent implementation exactly when it is monotonic

**Evidence**: "The CALM Theorem shows that the programs that have consistent, coordination-free distributed implementations are exactly the programs that can be expressed in monotonic logic."
**Source**: [Hellerstein & Alvaro, "Keeping CALM: When Distributed Consistency is Easy", arXiv:1901.01930 (published CACM 63(9), 2020)](https://arxiv.org/abs/1901.01930) — accessed 2026-09-25.
**Confidence**: High
**Verification**: Laddad et al. apply the theorem to reads over CRDTs: CRDT "guarantees extend only to data updates; observations of CRDT state are unconstrained and unsafe", and "monotone queries over CRDTs are exactly the queries that only need a local view of the system to be correct" ([Keep CALM and CRDT On, PVLDB 16(4) 2023, arXiv:2210.12605](https://arxiv.org/abs/2210.12605); [ACM DL](https://dl.acm.org/doi/abs/10.14778/3574245.3574268)).
**Analysis**: The CRDT extension is the most important result for the observation layer. Corrosion guarantees that replicas converge on the *rows*; it guarantees nothing about decisions *made by reading* those rows. "No heartbeat from node N for 30 s ⇒ N is down" is a **non-monotone query** (it reasons from absence), so a decision derived from gossiped state can differ between nodes and can be retracted when a late row arrives. This is the formal reason "node is down" belongs in the ordered log (as the target design already states) even though heartbeats themselves do not.

### Finding C2.2: Invariant confluence gives an operation-by-operation test — and a ready-made table for orchestrator-shaped invariants

**Evidence**: "invariant confluence analysis provides a necessary and sufficient condition for safe, coordination-free execution" and yielded "a 25-fold improvement over prior TPC-C New-Order performance on a 200 server cluster." The paper's Table 2 classifies invariant/operation pairs: Uniqueness + "Choose specific value" — **No**; Uniqueness + "Choose some value" — **Yes**; AUTO_INCREMENT insert — **No**; Foreign key insert — **Yes**; Foreign key delete — **No**; Foreign key cascading delete — **Yes**; Materialized views update — **Yes**; `>` threshold with counter increment — **Yes**; `>` threshold with decrement — **No**; `[NOT] CONTAINS` on sets — **Yes**; `SIZE=` with mutation — **No**. On IDs: "The difference is subtle ('grant this record this specific, unique ID' versus 'grant this record some unique ID'), but, in a system model with membership (e.g., server or replica IDs), is powerful. If replicas assign unique IDs within their respective portion of the ID namespace, then merging locally valid states will also be globally valid."
**Source**: [Bailis, Fekete, Franklin, Ghodsi, Hellerstein, Stoica, "Coordination Avoidance in Database Systems", PVLDB 8(3) 2015, arXiv:1402.2237](https://arxiv.org/abs/1402.2237) (Table 2, §5) — accessed 2026-09-25.
**Confidence**: High (peer-reviewed; formal proofs in the paper's appendix)
**Verification**: Balegas et al. (C1.5) show the `>`-with-decrement case becomes coordination-free *in the common case* once rights are escrowed per replica. Kubernetes' own ClusterIP allocator treats "a specific IP for this Service" as requiring coordination: "an internal allocator atomically updates a field in `etcd` prior to creating the Service … which ensures that no two Services can be assigned the same IP address" ([Kubernetes — Virtual IPs and Service Proxies](https://kubernetes.io/docs/reference/networking/virtual-ips/)). Conversely, Kubernetes escrows pod address space per node — the node controller "assigns a CIDR block to the node when it is registered (if CIDR assignment is turned on)", after which pod IPs come from that node-owned range ([Kubernetes — Nodes](https://kubernetes.io/docs/concepts/architecture/nodes/); [Cluster Networking](https://kubernetes.io/docs/concepts/cluster-administration/networking/)) — accessed 2026-09-25.
**Analysis**: The Kubernetes pairing is a production instance of Bailis' "specific value vs some value" split: *one* coordinated decision (grant node N the block) turns *many* subsequent allocations into local, coordination-free choices. The same move applies to Overdrive's per-workload addresses, SVID serials, allocation IDs, and any VIP that does not need to be operator-chosen.

### Finding C2.3: Hydro (latest) moves the CALM analysis into the type system, but is still alpha

**Evidence**: Hydro "is a high-level distributed programming framework for Rust … providing types and programming constructs for ensuring distributed safety." Stream types carry distribution properties, e.g. `Stream<T, Loc, Bounded, TotalOrder, ExactlyOnce>`; "`AtLeastOnce` means there may be non-deterministic duplicates" and `NoOrder, AtLeastOnce` streams "have set semantics." Recent publications: "Optimizing Distributed Protocols with Query Rewrites" (SIGMOD 2024 — rule-driven decoupling/partitioning rewrites that use "order-insensitivity and data dependency analysis"), "Flo: a Semantic Foundation for Progressive Stream Processing" (POPL 2025), "The Free Termination Property of Queries Over Time" (ICDT 2025 — when nodes can safely terminate without coordination). Current crates are versioned `hydro_lang v0.17.0-alpha.5`.
**Source**: [hydro-project/hydro (GitHub)](https://github.com/hydro-project/hydro); [Hydro research publications (hydro.run, out-of-list primary)](https://hydro.run/research/); [hydro_lang rustdoc](https://hydro.run/rustdoc/hydro_lang/all); [hydro_lang v0.17.0-alpha.5 release](https://github.com/hydro-project/hydro/releases/tag/hydro_lang-v0.17.0-alpha.5) — accessed 2026-09-25.
**Confidence**: Medium-High (project-primary plus peer-reviewed papers; maturity inferred from version tags)
**Verification**: SIGMOD 2024 and POPL 2025 venues confirmed on the research page and by the search index; CALM foundation per C2.1.
**Analysis**: Hydro is not a dependency candidate for Overdrive (alpha, and it owns the whole dataflow runtime). Its *idea* is directly borrowable: mark every cross-node channel in the type system as ordered/unordered and exactly-once/at-least-once, so that a consumer that needs order cannot silently be fed a gossip stream. This is the port-contract question of C6 seen from the data side.

### Classification of orchestrator operations (analysis, derived from C2.1–C2.2)

The verdict column applies Bailis' Table 2 row named in the "Nearest I-confluence row" column; where a row is "No", the right-hand column names the cheapest coordination that restores safety. **Interpretation, not a sourced verdict** — each mapping is the researcher's application of the cited rows.

| Orchestrator operation | Invariant at stake | Nearest I-confluence row | Needs ordering? | Cheapest safe mechanism |
|---|---|---|---|---|
| Create workload with operator-chosen name | Name unique | Uniqueness / choose specific value → No | **Yes** | Log commit (name claim) |
| Create allocation / instance ID | ID unique | Uniqueness / choose some value → Yes | No | Node-prefixed or ULID-style IDs |
| Update workload spec (blind overwrite) | Latest spec wins | Attribute equality → Yes (LWW) | Partly | Log commit anyway: the spec *generation* is an operator-meaningful sequence and a rollout trigger |
| Update spec conditioned on current version (CAS) | Read-modify-write | SIZE=/uniqueness class → No | **Yes** | Log commit |
| Place instance under per-machine capacity | free ≥ 0 under decrement | `>` with decrement → No | Not globally | **Machine is sole executor** → local admission check is sufficient for safety |
| Keep exactly N replicas | count = N | SIZE= with mutation → No | **Yes** | Log commit (the decision) |
| Anti-affinity / at most k per failure domain | bounded count per domain | SIZE=/uniqueness class → No | **Yes** | Log commit |
| Tenant or cluster quota | Σ usage ≤ limit | `>` with decrement → No | Rarely | Escrow / bounded counter (C1.5) |
| Node joins gossip membership | membership ⊇ node | [NOT] CONTAINS → Yes | No | Gossip (SWIM) |
| Change voter set | quorum intersection | not a data invariant — consensus-internal | **Yes** | Log commit (joint consensus / reconfiguration) |
| Declare node down / fence it | decision from absence | Non-monotone query (C2.1) | **Yes** | Log commit (with epoch) |
| Delete workload with live allocations | no dangling allocations | FK delete → No; cascading delete → Yes | Depends | Commit a tombstone that cascades; never a bare delete |
| Operator-chosen VIP / DNS name | unique specific value | choose specific value → No | **Yes** | Log commit |
| Auto-assigned VIP / workload address | unique some value | choose some value → Yes | No (after block grant) | Per-node block granted once through the log |
| Allocation status, health, heartbeat | owner's latest fact | single-writer per row → trivially confluent | No | Gossip / per-origin log |
| Restart count, event counters | monotone counter | `>` with increment → Yes | No | Per-origin G-counter |
| Certificate revocation list | set grows | [NOT] CONTAINS → Yes | No | Gossip (grow-only set) |
| Rolling-update progress (max unavailable) | count bound during change | SIZE= → No | **Yes** | Log commit |

**Reading**: every row marked "Yes" is a *decision* under the target's vocabulary, and every coordination-free row is a *fact* or a *locally scoped choice*. The table supports the working hypothesis for current-state facts, but with three refinements: (1) facts **read as decisions** (heartbeat → "down") need the log even though the facts don't; (2) several apparently global decisions (IDs, auto-assigned addresses, capacity) become coordination-free after **one** logged grant; (3) deletes are safe only as **cascading tombstones**.


## C3. Workers learning assignments: local replica (pull) vs push

Three production shapes exist: **pull from a server** (Nomad, Kubernetes), **pull from a local replica of the log** (Kafka KRaft brokers — the target design's exact shape), and **push to a worker that is itself the source of truth** (Fly flyd).

### Finding C3.1: Kafka KRaft is the closest production precedent — every broker is a non-voting observer that fetches and materializes the metadata log locally

**Evidence**: KIP-500: "brokers will fetch updates from the active controller via the new MetadataFetch API" and track "the offset of the last updates it fetched, and only request newer updates." Brokers "will register themselves with the controller quorum", and the active controller removes a broker "if it has not sent a MetadataFetch heartbeat in a long enough time." A broker "will re-enter the fenced state if it can't contact the active controller" — "Brokers cannot continue to be members of the cluster if they cannot receive metadata updates."
**Source**: [KIP-500: Replace ZooKeeper with a Self-Managed Metadata Quorum (Apache Kafka wiki, cwiki.apache.org)](https://cwiki.apache.org/confluence/display/KAFKA/KIP-500:+Replace+ZooKeeper+with+a+Self-Managed+Metadata+Quorum) — accessed 2026-09-25.
**Confidence**: High (foundation design record; behaviour shipped in Kafka 3.x/4.x KRaft)
**Verification**: [KIP-595: A Raft Protocol for the Metadata Quorum](https://cwiki.apache.org/confluence/x/Li7cC) defines voters vs observers; Red Hat's 2025 KRaft deep dive describes brokers as observers that "fetch and replay the same metadata log … and they never vote" ([Red Hat Developer, 2025-09-17](https://developers.redhat.com/articles/2025/09/17/deep-dive-apache-kafkas-kraft-protocol)).
**Analysis**: This validates "voters + all-node learners + pull" at production scale, and it shows the *registration heartbeat riding the same fetch* — the fetch itself is the liveness signal, so no separate heartbeat stream is needed for learners that are actively fetching. Note the design choice: Kafka brokers **self-fence** on losing the controller because a partition leader cut off from metadata must not keep accepting writes. See C3.4 for when Overdrive workloads need the same.

### Finding C3.2: Pulling from a lagging replica is a known production hazard — a worker must never act destructively on a stale absence

**Evidence**: Nomad issue #18267 (Aug 2023, closed, accepted bug): after a server restarts and rebuilds Raft state, a client's `GetClientAllocs` blocking query can return stale data; "the response may not be up-to-date resulting in partial or complete deletion of local state (which affects running workloads)." Proposed fix: "add a staleness check for the data returned for any blocking query within Nomad Client … the data returned corresponds to a steadily increasing index."
**Source**: [hashicorp/nomad#18267 "Nomad Server may instruct Clients to erroneously stop and GC all of their allocations"](https://github.com/hashicorp/nomad/issues/18267) — accessed 2026-09-25.
**Confidence**: High (primary bug report from the vendor's tracker)
**Verification**: Kubernetes had to add freshness confirmation before serving "consistent" reads from its watch cache: in v1.31 (beta, on by default), when a consistent read is requested the API server "first checks if the watch cache is up-to-date", using etcd progress notifications, and only then serves from cache ([Kubernetes v1.31: Consistent Reads from Cache](https://kubernetes.io/blog/2024/08/15/consistent-read-from-cache-beta/), accessed 2026-09-25; paraphrase of the post). KIP-500's offset tracking (C3.1) is the same monotonicity guard.
**Analysis**: This is the most important operational rule for the target design's "workers read their local replica". The local replica is, by construction, possibly behind. Absence of an assignment in a replica at index *i* means "not assigned as of *i*", not "unassigned". Required guards: (1) the replica's applied index is monotone and never regresses (snapshot install included); (2) a worker stops a running allocation only on an **explicit, committed stop/tombstone**, never on absence; (3) a node rejoining after wipe must re-sync to at least the index at which it last acted before taking destructive actions.

### Finding C3.3: Across Nomad, Kubernetes, and Twine, a worker cut off from the control plane keeps running committed work by default; replacement is a *separate, delayed, logged* decision

**Evidence**: Nomad's `disconnect` block: "By default, without a `disconnect` block, if an allocation is on a node that misses heartbeats, the allocation will be marked `lost` and will be replaced." With `lost_after`, allocations go to `unknown` instead of `lost`; `replace` controls whether a replacement is scheduled; `reconcile` "Specifies which allocation to keep once the previously disconnected node regains connectivity" (`keep_original`, `keep_replacement`, `best_score`, `longest_running`); `stop_on_client_after` "Specifies a duration after which a disconnected Nomad client will stop its allocations." Kubernetes adds default tolerations of `tolerationSeconds=300` for the `node.kubernetes.io/unreachable` and `not-ready` taints before evicting. Twine: "If a DC is partitioned from the scheduler, existing tasks in the DC continue to run."
**Source**: [Nomad — `disconnect` block](https://developer.hashicorp.com/nomad/docs/job-specification/disconnect); [Kubernetes — Taints and Tolerations, "Taint based Evictions"](https://kubernetes.io/docs/concepts/scheduling-eviction/taint-and-toleration/) (the 300 s default was confirmed on the page's structure; the section body was truncated in fetch — see Knowledge Gaps); [Twine §4](https://www.usenix.org/system/files/osdi20-tang.pdf) — accessed 2026-09-25.
**Confidence**: High (three independent systems)
**Verification**: Fly: "flyd is the source of truth for all the VMs running on a particular worker" and "Every `flyd` keeps a `boltdb` database of its current state, which is an append-only log of all the operations applied to the worker" ([Fly — Carving The Scheduler Out Of Our Orchestrator](https://fly.io/blog/carving-the-scheduler-out-of-our-orchestrator/)).
**Analysis**: The partition behaviour decomposes into two independent decisions that production systems keep separate and make *configurable per workload*: (i) **does the cut-off worker keep running?** (default yes; `stop_on_client_after` makes it self-fence) and (ii) **does the majority side replace?** (default yes after a grace period; `replace = false` suppresses it). When both are "yes", duplicates exist during the partition, and the `reconcile` policy decides on heal. The target design needs the same two knobs; the log is the natural home for (ii) because "replace W#2 because M is presumed dead" is exactly the "node is down" decision the design already routes through consensus.

### Finding C3.4: Pull vs push is a secondary choice; what matters is who holds the durable truth for "what runs here"

**Evidence**: Fly's flaps "uses Corrosion to find all the workers in a particular region. It has direct connectivity to every `flyd`"; the scheduler "operates like a market. Requests to schedule jobs are bids for resources; workers are suppliers." (Push to an authoritative worker.) Nomad: clients poll servers with blocking queries (pull from server; #18267). KRaft: observers fetch the log (pull from log).
**Source**: [Fly — Carving The Scheduler](https://fly.io/blog/carving-the-scheduler-out-of-our-orchestrator/); [nomad#18267](https://github.com/hashicorp/nomad/issues/18267); [KIP-500](https://cwiki.apache.org/confluence/display/KAFKA/KIP-500:+Replace+ZooKeeper+with+a+Self-Managed+Metadata+Quorum) — accessed 2026-09-25.
**Confidence**: Medium-High (three primary sources; the comparative framing is analysis)
**Verification**: Prior repo research F3.3 (`nomad-scheduling-and-reconciler-pattern-research.md`) documents Fly's immediate-or-cancel market model.
**Analysis (interpretation)**: Fly makes the *worker* authoritative and the cluster a directory; the target design makes the *log* authoritative and the worker a follower. Fly's shape gives synchronous yes/no placement (a worker can refuse *now*) and survives total control-plane loss for new placements into reachable workers; the log shape gives a single ordered history for replay, audit, and upgrade-safe decisions, at the cost that a placement is only "real" once committed and fetched. The hybrid suggested by C1.5 — committed assignment + worker right of refusal written back as a fact — captures most of Fly's benefit without giving up the ordered history. Pull-from-local-replica is strictly better than pull-from-server for the target (no read load on voters; reads survive voter loss), **provided** the C3.2 guards are in place.


## C4. Explicit vs automatic placement

### Finding C4.1: Production orchestrators express placement as constraints over *attributes of failure domains*, split into hard filters and soft scores

**Evidence**: Kubernetes topology spread constraints "control how Pods are spread across your cluster among failure-domains such as regions, zones, nodes, and other user-defined topology domains." Fields: `maxSkew`, `topologyKey` ("the key of node labels"), `whenUnsatisfiable` (`DoNotSchedule` default vs `ScheduleAnyway`), `minDomains` (v1.30+ without feature gate), `matchLabelKeys` (beta since v1.27), `nodeAffinityPolicy`/`nodeTaintsPolicy` (beta since v1.26). Nomad: "The `spread` block allows operators to increase the failure tolerance of their applications by specifying a node attribute that allocations should be spread over … Nodes are scored according to how closely they match the desired target percentage defined in the spread block. Spread scores are combined with other scoring factors such as bin packing"; targets can be percentages per attribute value (e.g., `${meta.rack}` r1 60 % / r2 40 %). Twine's allocator applies "hard requirements (e.g., using Skylake machines only) and soft preferences (e.g., spreading tasks across fault domains)", and "By default, Twine spreads tasks of the same job across DCs and MSBs [main switchboards]" — a job spread over 60 MSBs needs ≈1.7 % buffer capacity versus ≈8.3 % over 12.
**Source**: [Kubernetes — Pod Topology Spread Constraints](https://kubernetes.io/docs/concepts/scheduling-eviction/topology-spread-constraints/); [Nomad — `spread` block](https://developer.hashicorp.com/nomad/docs/job-specification/spread); [Twine §2.2–2.3](https://www.usenix.org/system/files/osdi20-tang.pdf) — accessed 2026-09-25.
**Confidence**: High (three independent systems)
**Verification**: Borg: the scheduler accounts for failure domains so that "it won't run all of a job's tasks on the same rack" ([Google SRE Book — The Production Environment at Google](https://sre.google/sre-book/production-environment/); [Verma et al., Borg, EuroSys 2015](https://research.google/pubs/large-scale-cluster-management-at-google-with-borg/)).
**Analysis**: The consensus across Borg, Twine, Kubernetes, and Nomad is that **the default is automatic spread across the failure-domain hierarchy** and the operator's lever is a *constraint on attributes* (hard) or a *preference* (soft). Twine's buffer arithmetic is the quantitative argument for making spread the default: spreading wider reduces the spare capacity needed to survive a domain loss. For the target design this means node attributes (region, zone, rack, power domain, hardware class) must be first-class, gossiped-then-committed facts, and the scheduler's input is the attribute set, never the name.

### Finding C4.2: Spread is enforced at placement time only — constraints decay as the cluster changes, and rebalancing is a separate mechanism

**Evidence**: Kubernetes' documented limitation: there is no guarantee the constraints remain satisfied when Pods are removed; scaling down can leave an imbalanced distribution; a descheduler is suggested to rebalance. Twine: "The addition or removal of machines and workload evolution may result in hotspots … ReBalancer runs asynchronously and continuously to improve upon the allocator's allocation decisions … ReBalancer uses a constraint solver to perform these time-consuming global optimizations."
**Source**: [Kubernetes — Topology Spread Constraints, "Known limitations"](https://kubernetes.io/docs/concepts/scheduling-eviction/topology-spread-constraints/) (section summarized by the fetch tool — wording not verbatim); [Twine §2.3](https://www.usenix.org/system/files/osdi20-tang.pdf) — accessed 2026-09-25.
**Confidence**: Medium-High (two independent sources; the Kubernetes wording is a summary)
**Verification**: Nomad's spread is likewise a score contribution at placement ("Spread scores are combined with other scoring factors"), with no continuous enforcement stated in its docs.
**Analysis**: A spread constraint is a *placement-time* invariant, not a *standing* invariant. In a cluster that grows by joining nodes — the target's defining property — every join changes the domain set, so a workload placed when there were 2 zones stays in 2 zones when a 3rd appears unless something re-evaluates it. The target design needs an explicit stance: either (i) spread is placement-time only (Kubernetes/Nomad default), or (ii) a rebalancer reconciler continuously re-derives it (Twine ReBalancer/descheduler). (ii) is a reconciler over committed decisions and must itself commit moves through the log, rate-limited (Twine's "rate-limit destructive operations", C1.8).

### Finding C4.3: Pinning by machine name is fragile; the durable alternative is to expose failure-domain *slots* rather than machines

**Evidence**: Kubernetes documents `nodeName` limitations: if the named node does not exist the Pod will not run; if it lacks resources the Pod fails; and node names in cloud environments are not always predictable or stable. `nodeName` bypasses the scheduler, and `nodeSelector`/affinity are recommended instead. AWS EC2 partition placement groups expose failure domains as numbered slots: "Amazon EC2 ensures that each partition within a placement group has its own set of racks. Each rack has its own network and power source. No two partitions within a placement group share the same racks … You can also launch instances into a specific partition … partition placement groups offer visibility into the partitions — you can see which instances are in which partitions. You can share this information with topology-aware applications, such as HDFS, HBase, and Cassandra." Spread placement groups place each instance "on distinct hardware" with "a maximum of seven running instances in each Availability Zone", and "If you start or launch an instance in a spread placement group and there is insufficient unique hardware to fulfill the request, the request fails."
**Source**: [Kubernetes — Assigning Pods to Nodes, `nodeName`](https://kubernetes.io/docs/concepts/scheduling-eviction/assign-pod-node/) (fetch returned a summarized rendering; the three limitations match the upstream page's list); [AWS EC2 — Placement strategies](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/placement-strategies.html) (verbatim) — accessed 2026-09-25.
**Confidence**: High (two independent authoritative sources; Twine's "pin its machines and jobs to a specific DC" for locality-bound workloads is a third, noted as "in the minority")
**Verification**: [Twine §2.2](https://www.usenix.org/system/files/osdi20-tang.pdf): "For workloads that require better locality for compute and storage, Twine allows an entitlement to override the default spread policy and pin its machines and jobs to a specific DC. These workloads are in the minority."
**Analysis**: Machine-name pinning fails three ways in the target design: (1) a node rejoining after reinstall may get a new identity, silently orphaning the pin; (2) a pinned workload on a dead node has no legal replacement, so the "node is down" decision produces an unschedulable workload rather than a move; (3) pins bypass capacity and spread checks. The AWS partition model is the missing middle ground between "auto" and "this machine": the operator (or a topology-aware workload like a replicated database) asks for **"replica k goes to failure-domain slot k"**, and the platform maps slots to disjoint racks/zones. Stateful workloads that genuinely need stickiness are sticky to *their data* (a volume's location), which is again an attribute, not a name. Recommended research direction: placement vocabulary = {hard attribute constraints, soft spread preferences, failure-domain slots, data-locality affinity}, with machine-name pinning reserved for break-glass operations.


## C5. Observation layer alternatives to Corrosion

Prior repo research already establishes Corrosion's 2024 incident corpus, its ~800-server published envelope, and Consul's ≤5,000-agents-per-pool guidance (`corrosion-crsqlite-production-scale-evidence.md` F1–F12). This section adds the 2025–2026 state and the alternatives.

### Finding C5.1: Corrosion (2026) — actively maintained, no tagged release since v1.0.0, backpressure issue closed without a visible PR, and tombstone GC still open

**Evidence**: The GitHub releases page lists **v1.0.0 (May 14, 2024)** as the latest tagged release; v1.0.0 notes say "the database schema has changed … a running v0 cluster cannot be upgraded in-place." The `main` branch is active through September 2026 — e.g., "Prevent Plumtree timers from starving behind inbound traffic (#551)" (Sep 4, 2026), "Cap decompressed output from `CompressedChange` (#561)" (Sep 7, 2026), "Run change maintenance independently of batch timeouts (#553)", and a Postgres wire-protocol front end ("corro-pg … (#559)"). Issue #198 ("Apply backpressure once changes queue reaches a set length") is now **closed**; the page shows no linked PR. Issue #533 "Explore active GC" (opened Aug 26, 2026, open): "Currently tombstones accumulate in the clock tables indefinitely"; the existing reaper risks deleted rows reappearing because "nodes can be restored from older backups at any time"; the proposal adds per-site watermarks and "will most definitely trade a bit of Availability for storage footprint." PR #527 (open) adds supervised restartable actors for "sync, membership notifications, gossip sending, incoming-change processing, buffered-change application … the configured Gossip/Plumtree broadcast loop."
**Source**: [superfly/corrosion releases](https://github.com/superfly/corrosion/releases); [commits on main](https://github.com/superfly/corrosion/commits/main); [issue #198](https://github.com/superfly/corrosion/issues/198); [issue #533](https://github.com/superfly/corrosion/issues/533); [PR #527](https://github.com/superfly/corrosion/pull/527) — all accessed 2026-09-25.
**Confidence**: High for the repository facts; Low for any inference about whether backpressure was actually implemented (closure without a visible PR — see Knowledge Gaps).
**Verification**: Fly's October 2025 retrospective and the regionalization architecture are covered in prior research F2 ([Fly — Corrosion](https://fly.io/blog/corrosion/)). The tombstone problem is the same "causal stability" requirement identified in `crash-observability-under-lww-comprehensive-research.md` F7.
**Analysis**: The latest state strengthens two of the known concerns and softens none: (1) garbage collection of deletes in a CRDT store requires cluster-wide knowledge (watermarks) — i.e., coordination — which Corrosion is only now exploring; (2) the Plumtree broadcast layer and the actor-supervision work show the dissemination path is still being hardened. The single-vendor evidence base is unchanged (no independent production operator found).

### Finding C5.2: Scuttlebutt is strictly single-writer-per-origin and sends only the *latest* version per key — it is a state-sync protocol, not a log-shipping protocol, and it has a hard capacity ceiling

**Evidence**: "A participant *p* is only allowed to update its own state μ*p*(*p*) directly. μ*p*(*q*), *p* ≠ *q*, can only be updated indirectly through gossip." "The paper shows that anti-entropy protocols can process only a limited rate of updates, and proposes and evaluates a new state reconciliation mechanism as well as a flow control scheme." "While expected latencies continue to grow logarithmic in N, the size of gossip messages grows linearly in N … If the rate of updates is too high, latency may grow without bounds." "Applications are typically more interested in recency of information than in how often it is sampled."
**Source**: [van Renesse, Dumitriu, Gough, Thomas, "Efficient Reconciliation and Flow Control for Anti-Entropy Protocols", LADIS 2008 (ACM DL)](https://dl.acm.org/doi/10.1145/1529974.1529983) (author PDF [cs.cornell.edu](https://www.cs.cornell.edu/home/rvr/papers/flowgossip.pdf)) — accessed 2026-09-25.
**Confidence**: High (peer-reviewed primary; authors at Amazon at the time)
**Verification**: chitchat (Quickwit's Rust implementation): "A anti-entropy gossip algorithm called scuttlebutt is in charge of spreading a common state to all nodes"; "A node can only edit its own node state"; deletions are "versioned tombstone[s]" removed after `marked_for_deletion_grace_period`; caveat: nodes joining after `dead_node_grace_period` "won't know about deleted dead-node state", and "Disconnected nodes reconnecting may re-spread old information" ([quickwit-oss/chitchat](https://github.com/quickwit-oss/chitchat)). Cassandra's membership gossip is Scuttlebutt-derived (per search index of Cassandra docs; not separately fetched).
**Analysis**: This sharpens the "per-origin append-only logs with Scuttlebutt" alternative. Scuttlebutt's single-writer rule removes LWW conflict *between writers* (there is only one), but it does **not** by itself preserve intermediate values: for a mutable key only the highest version travels, exactly like LWW. To carry *occurrences* (crashes, restarts) the origin must write **append-only keys** (e.g., `event/<seq>`), which Scuttlebutt then ships completely — at the price of unbounded state that needs a GC rule. chitchat's GC rule (grace-period tombstones; reset on missed GC) is the practical answer and its documented caveats (late joiners, resurrecting stale state) are the same hazard Corrosion #533 describes. The flow-control result is the direct answer to Corrosion's backpressure gap: Scuttlebutt's design includes a per-participant rate-adaptation scheme; Corrosion's issue tracker shows backpressure was retrofitted.

### Finding C5.3: chitchat is the most mature *Rust* Scuttlebutt, but its steward changed hands in 2025

**Evidence**: chitchat: 378 stars, 65 forks, 228 commits; phi-accrual failure detection; partial deltas "to fit UDP packets"; used by Quickwit for cluster membership and metadata. Quickwit was acquired by Datadog in January 2025; Quickwit announced the open-source project would be relicensed to Apache 2.0.
**Source**: [quickwit-oss/chitchat](https://github.com/quickwit-oss/chitchat); [Quickwit — "Quickwit joins Datadog" (out-of-list primary)](https://quickwit.io/blog/quickwit-joins-datadog); [Datadog — "Datadog acquires Quickwit"](https://www.datadoghq.com/blog/datadog-acquires-quickwit/) — accessed 2026-09-25.
**Confidence**: Medium-High (repository facts + two vendor announcements)
**Verification**: Repository statistics and acquisition cross-checked across GitHub and both company blogs.
**Analysis**: chitchat's design (per-node owned KV, scuttlebutt, phi-accrual, UDP-sized deltas) matches the "strictly single-writer rows, no CRDTs" alternative almost exactly, and its state model is intentionally small (cluster metadata, not tens of millions of rows). It is a better fit for *membership and small per-node facts* than for high-cardinality per-allocation observation; the latter would need either sharding or a different carrier (C5.5). Post-acquisition maintenance cadence is a knowledge gap.

### Finding C5.4: The industry trend is to move *correctness-bearing* cluster metadata off gossip onto a log, while keeping gossip for liveness input and "non-correctness impacting" state

**Evidence**: Apache Cassandra CEP-21 (Transactional Cluster Metadata, shipping in Cassandra 6): "Probabilistic propagation of cluster state is suboptimal and not necessary … since there is no strict order for changes propagated through gossip, there can be no universal 'reference' state at any given time." Schema, token ownership, and membership move to a linearized log. "Complete removal of gossip" is a non-goal: Cassandra will "retain gossip as both an input to failure detection and for the dissemination of transient and/or non-correctness impacting cluster metadata, such as RPC readiness, storage load, etc." Catch-up: "requesting the sequence of log entries with a greater epoch than any previously seen by the node. As the log is immutable and totally ordered, this request can be made to any peer as the results must be consistent, if not exhaustive." ScyllaDB: in 6.0 "Information about cluster members is now propagated through Raft instead of Gossip", strongly consistent topology is the default, and it became mandatory in 2025.2.
**Source**: [CEP-21: Transactional Cluster Metadata (cwiki.apache.org)](https://cwiki.apache.org/confluence/display/CASSANDRA/CEP-21:+Transactional+Cluster+Metadata); [CASSANDRA-18330 delivery ticket](https://issues.apache.org/jira/browse/CASSANDRA-18330); [ScyllaDB — "ScyllaDB 6.0: with Tablets & Strongly-Consistent Topology Updates" (out-of-list primary)](https://www.scylladb.com/2024/06/12/introducing-scylladb-6-0-with-tablets-and-strongly-consistent-topology-updates/); [ScyllaDB docs — Enable Consistent Topology Updates](https://docs.scylladb.com/manual/branch-2025.1/upgrade/upgrade-guides/upgrade-guide-from-2024.x-to-2025.1/enable-consistent-topology.html) — accessed 2026-09-25.
**Confidence**: High (two independent database projects plus Kafka's ZooKeeper→KRaft move in C3.1; each migrated in production)
**Verification**: Kafka KIP-500 (C3.1) — broker registration and fencing go through the controller log, not a side channel.
**Analysis**: This is the strongest external evidence bearing on the working hypothesis, and it both **supports and sharpens** it. Supports: three large production systems independently moved membership/ownership *decisions* from gossip to an ordered log and kept gossip for liveness input. Sharpens: Cassandra's line is not "facts vs decisions" but **"correctness-impacting vs not"**. A fact that feeds a correctness decision (a node's token ownership, its fenced/unfenced state) is logged even though it is "about" one node; a fact that only informs heuristics (storage load, RPC readiness) is gossiped. CEP-21's "request can be made to any peer" is also the peer-exchange dissemination the target design wants for learners (see sibling doc A2).

### Finding C5.5: Production orchestrators *do* put per-node facts through consensus — Consul, Nomad, and Kubernetes all do — which is part of why their single-cluster ceilings are ~5k nodes

**Evidence**: Consul anti-entropy: "Consul treats the state of the agent as authoritative. If there are any differences between the agent's view and the catalog view, the agent uses its local view"; agents sync services and health checks into the server catalog, which is replicated via Raft; "In addition to detecting agent changes, it periodically syncs service and health check information to the catalog," with sync intervals scaling from 1 minute (1–128 nodes) to 4 minutes (513–1,024 nodes), staggered randomly. Nomad writes every evaluation and allocation status change through Raft (prior research F1.8). Twine's comparison: Kubernetes' "centralized components become bottlenecks and limit Kubernetes' scalability to 5K machines."
**Source**: [Consul — Consistency / anti-entropy](https://developer.hashicorp.com/consul/docs/concept/consistency); [Twine §3.2](https://www.usenix.org/system/files/osdi20-tang.pdf); prior research `nomad-scheduling-and-reconciler-pattern-research.md` F1.8 ([Load shedding in the Nomad eval broker](https://www.hashicorp.com/en/blog/load-shedding-in-the-nomad-eval-broker)) — accessed 2026-09-25.
**Confidence**: Medium-High (three sources; the causal link "facts-through-consensus → ceiling" is inferred, not measured by any source)
**Verification**: Nomad's eval-storm arithmetic (prior research F1.9: 10 % of 5,000 nodes missing a heartbeat → 60,000 evaluations) is a direct measurement of what happens when liveness facts feed consensus-committed work.
**Analysis**: This is the **counter-evidence** the hypothesis needs to survive. The mainstream orchestrators do not separate facts from decisions at the storage layer; they rate-limit fact ingestion into the log (Consul's scaled sync intervals, Nomad's eval load-shedding) and accept a ~5k-node ceiling per cluster. Separating a gossip fact layer from the decision log is exactly what lets Fly (Corrosion) and Cassandra/Scylla (gossip for liveness) exceed that without the log becoming the bottleneck. The hypothesis holds *as a scaling strategy*, provided the log still receives the small set of facts that gate correctness (C5.4).

### Finding C5.6: NATS JetStream KV is CP per stream — the wrong carrier for facts that must remain writable under partition, but a close structural analogue of the target's *decision* plane

**Evidence**: "a bucket is a JetStream stream named `KV_<bucket>` … a `put` of a value appends a message, a `get` of a value reads the last message for a subject, and a `watch` of a bucket opens a consumer." JetStream clustering: "The meta group" coordinates stream and consumer placement; each stream gets "its own Raft group, separate from the meta group"; "With one of three left, no write can reach a majority, so the stream stops accepting new messages until a server comes back"; "a stream keeps at most five copies"; production clusters "typically three or five" servers.
**Source**: [NATS docs — Key/Value Store](https://docs.nats.io/nats-concepts/jetstream/key-value-store); [NATS docs — JetStream Clustering](https://docs.nats.io/running-a-nats-service/configuration/clustering/jetstream_clustering) — accessed 2026-09-25.
**Confidence**: High (official documentation)
**Verification**: [NATS docs — Clustering & Replication Deep Dive](https://docs.nats.io/learn/clustering/) describes the same meta-leader-decides-placement model.
**Analysis**: As a *fact* bus, JetStream KV fails the "available under partition" requirement: a minority-side node cannot publish its own heartbeat into an R3 bucket whose leader is on the majority side. It is, however, an instructive analogue for the *decision* side: a meta group whose leader decides and commits placements, with data planes that follow. NATS core request/reply (C6) is separately relevant to ports.

### Finding C5.7: Reconciliation-efficiency techniques — delta-state CRDTs, Merkle trees, rateless IBLTs, and hierarchical aggregation — address Corrosion's two worst failure modes (backfill storms, rejoin backlogs)

**Evidence**: Delta-state CRDTs "achieve the best of both worlds: small messages with an incremental nature, as in operation-based CRDTs, disseminated over unreliable communication channels, as in traditional state-based CRDTs," with "an anti-entropy algorithm for eventual convergence, and another one that ensures causal consistency" (Almeida, Shoker, Baquero, JPDC 111, 2018). Dynamo: "To detect the inconsistencies between replicas faster and to minimize the amount of transferred data, Dynamo uses Merkle trees." Rateless IBLT (SIGCOMM 2024): "the first set reconciliation protocol … that achieves low computation cost and near-optimal communication cost across a wide range of scenarios: set differences of one to millions," reducing computation "by 2–2000×, while incurring a communication cost of less than 2× the information-theoretic lower bound." Astrolabe "organizes the resources into a hierarchy of domains called zones", where "information in zones is summarized before being exchanged between zones", computed by SQL aggregation queries.
**Source**: [Almeida et al., "Delta State Replicated Data Types", arXiv:1603.01529 / JPDC 2018](https://arxiv.org/abs/1603.01529); [DeCandia et al., "Dynamo", SOSP 2007 (via allthingsdistributed.com, author-hosted)](https://www.allthingsdistributed.com/2007/10/amazons_dynamo.html); [Yang, Gilad, Alizadeh, "Practical Rateless Set Reconciliation", SIGCOMM 2024 (arXiv:2402.02668)](https://arxiv.org/pdf/2402.02668), [ACM DL](https://dl.acm.org/doi/10.1145/3651890.3672219); [van Renesse, Birman, Vogels, "Astrolabe", ACM TOCS 21(2), 2003](https://dl.acm.org/doi/10.1145/762483.762485) — accessed 2026-09-25.
**Confidence**: High for each technique's claims (peer-reviewed); production use in an orchestrator's observation layer not found.
**Verification**: Scuttlebutt's digest exchange (C5.2) is the per-origin-version equivalent of Merkle comparison; chitchat implements it.
**Analysis**: These are orthogonal building blocks, not whole alternatives:
- **Rejoin after a long partition** (Corrosion's "millions of changes" backlog): with strictly single-writer, per-origin versioned rows, a Scuttlebutt digest is already O(origins); for the *content* diff, set reconciliation (rateless IBLT) makes cost proportional to the difference, not the table.
- **Schema/backfill storms** (the nullable-column incident): these are a property of column-level CRDT metadata in cr-sqlite. A per-origin log of immutable, versioned *records* (encoded rows, not CRDT columns) has no per-column clock to backfill; a schema change is a new record version written by each origin at its own pace.
- **Global fan-out at thousands of nodes**: Astrolabe's zone hierarchy is the only surveyed design where a node does not hold every other node's facts. It answers "a node in region A needs aggregate facts about region B, not every row" — the same conclusion Fly reached with regionalization, derived 20 years earlier.

### Finding C5.8: When a fact needs a second writer, production systems either overwrite the owner's record from the control plane (Kubernetes) or add an explicit intermediate state and a reconcile rule (Nomad); fencing tokens are the general mechanism

**Evidence**: Kubernetes node controller: when a node becomes unreachable the controller updates the node's `Ready` condition to `Unknown` (a record the kubelet otherwise owns), then after a grace period triggers API-initiated eviction of its Pods, with eviction rate-limited per zone and special behaviour for unhealthy zones; kubelets heartbeat via `Lease` objects in `kube-node-lease`. Nomad: a disconnected node's allocations move to `unknown` (not `lost`) during `lost_after`, and `reconcile` picks `keep_original`/`keep_replacement`/`best_score`/`longest_running` on reconnect. Chubby: a lock holder can obtain a **sequencer** (lock name, mode, lock generation number) and pass it to downstream servers, which check it and reject operations from holders whose lock is no longer current; for servers without sequencer support, a lock-delay (typically one minute) is used.
**Source**: [Kubernetes — Nodes, "Node controller" and "Node heartbeats"](https://kubernetes.io/docs/concepts/architecture/nodes/) (fetch returned a summarized rendering); [Nomad — `disconnect` block](https://developer.hashicorp.com/nomad/docs/job-specification/disconnect); [Burrows, "The Chubby lock service for loosely-coupled distributed systems", OSDI 2006](https://www.usenix.org/conference/osdi-06/chubby-lock-service-loosely-coupled-distributed-systems) (mechanism per search index of the paper; full text not fetched) — accessed 2026-09-25.
**Confidence**: Medium-High (three independent systems; two fetches were summaries rather than verbatim)
**Verification**: CEP-21's epochs (C5.4) and KIP-500's fenced broker state (C3.1) are the same idea: a logged generation number that later messages must carry.
**Analysis — what breaks with a second writer, and how to commit the transfer**:
1. **Two writers to one LWW row** (Kubernetes' Ready condition shape): the value converges to whichever write has the later timestamp; the dead node's last "Ready=True" and the controller's "Unknown" race, and a resurrected node can clobber the controller's verdict. LWW turns an ownership question into a clock question.
2. **Strictly single-writer rows** (Scuttlebutt/chitchat shape) make the race impossible — but then nothing can mark a dead node's allocations as gone; its rows linger until a grace-period GC, and a node that returns re-spreads them (chitchat's documented caveat).
3. **The clean transfer** (synthesis of CEP-21 epochs, KIP-500 fencing, Chubby sequencers): the *decision* "node N is fenced as of epoch E; its allocations are reassigned" is committed to the log. Facts are never rewritten by a second party; instead, **readers interpret** any fact from N carrying an owner-epoch < E as stale, the new owner writes *its own* rows, and N — on rejoining — must read the log, observe E, and stop or re-register under a new epoch before its facts count again. Nomad's `unknown` state and `reconcile` policy are the operator-facing surface of the same transfer.
This keeps the observation layer strictly single-writer (no CRDT merge needed) while putting the one operation that genuinely has two parties — the transfer of ownership — on the log, where Bailis' analysis (C2) says it belongs.


## C6. Location-transparent component ports (local vs remote adapters)

### Finding C6.1: Waldo et al. — partial failure makes local and remote *interfaces* different in kind; transparency must not hide it

**Evidence**: "We argue that objects that interact in a distributed system need to be dealt with in ways that are intrinsically different from objects that interact in a single address space. These differences are required because distributed systems require that the programmer be aware of latency, have a different model of memory access, and take into account issues of concurrency and partial failure." "In a distributed system, the failure of a network link is indistinguishable from the failure of a processor on the other side of that link." "Partial failure requires that programs deal with indeterminacy … the interfaces that are used for the communication must be designed in such a way that it is possible for the objects to react in a consistent way to possible partial failures." "Being robust in the face of partial failure requires some expression at the interface level … there must be interfaces that allow reconstruction of a reasonable state when failure occurs and the cause cannot be determined." The two paths to a unified model are to treat all objects as local (the result "is essentially indeterministic in the face of partial failure and consequently fragile and non-robust") or to "design all interfaces as if they were remote."
**Source**: [Waldo, Wyant, Wollrath, Kendall, "A Note on Distributed Computing", Sun Microsystems Laboratories SMLI TR-94-29, November 1994](https://sites.cc.gatech.edu/classes/AY2010/cs4210_fall/papers/smli_tr-94-29.pdf) (course mirror; ACM DL record [10.5555/974938](https://dl.acm.org/doi/pdf/10.5555/974938)) — accessed 2026-09-25.
**Confidence**: High (primary source; foundational and uncontested)
**Verification**: wasmCloud's 2025 retrospective (C6.2) and Orleans' delivery-guarantee documentation (C6.3) independently re-state the same lesson in current systems.
**Analysis**: For the target's "each component port is satisfied by a local or a remote adapter", Waldo forces a choice: the port contract must be the **remote** contract (Waldo's second path), and the local adapter is a degenerate case that happens never to exhibit some failure modes. The reverse — a local-shaped port with a remote adapter slotted in — is the path Waldo shows to be fragile.

### Finding C6.2: A location-transparent lattice (wasmCloud) is moving from implicit to intentional remoting — the 2025 re-discovery of Waldo

**Evidence**: wasmCloud's lattice: when a component exports a function, "it creates a queue subscription on a NATS subject that other components can call" (per wasmCloud v1 docs). July 2025: "Wouldn't it be nice if two components running on the same host that are linked together could call each other's functions *directly* instead of going out to NATS?" … "Seamless distributed networking is a magical aspect of wasmCloud. In distributed systems, magic is a little scarier than it is desirable" … component networking via wRPC "should be used intentionally rather than implicitly." The Q3 2025 roadmap (per search index; roadmap page not fetched directly) plans for the next major release that "the new scheduling API will not use NATS to communicate between components by default," with providers becoming wRPC servers over TCP, NATS, QUIC, or UDP.
**Source**: [wasmCloud — "Charting the next steps for wasmCloud" (Jul 3, 2025)](https://wasmcloud.com/blog/charting-the-next-steps-for-wasmcloud/); [wasmCloud — Lattice (v1 docs)](https://wasmcloud.com/docs/v1/concepts/lattice/) — accessed 2026-09-25.
**Confidence**: Medium-High (project-primary; roadmap details via search index only)
**Verification**: Waldo (C6.1) predicts exactly this failure of implicit transparency; NATS' own docs (C6.4) expose the remote-only failure signals the lattice was hiding.
**Analysis**: The most relevant practitioner signal for the target design: a CNCF project that built its whole model on location transparency concluded that turning a "nanosecond in-process call" into a distributed request *implicitly* was a mistake. For Overdrive the lesson is not "avoid remote adapters" but "make the adapter choice visible in configuration and in the port's error type", so a caller can tell a remote-shaped failure from a local one.

### Finding C6.3: Virtual actors make location transparent but make delivery semantics explicit — at-most-once by default, and a timeout does not tell you whether the call ran

**Evidence**: "Orleans messaging delivery guarantees are **at-most-once** by default. Optionally, if you configure retries upon timeout, Orleans provides at-least-once delivery instead." "Every message in Orleans has an automatic timeout … If the reply doesn't arrive on time, the returned Task breaks with a timeout exception." "In a system with retries … the message might arrive multiple times. Orleans currently doesn't durably store which messages have already arrived or suppress subsequent deliveries. (We believe this would be quite costly.)" The page also records that the original Orleans technical report "accidentally only mentioned the second option with automatic retries."
**Source**: [Microsoft Learn — Orleans messaging delivery guarantees (updated 2026-01-27)](https://learn.microsoft.com/en-us/dotnet/orleans/implementation/messaging-delivery-guarantees) — accessed 2026-09-25.
**Confidence**: High (official documentation)
**Verification**: Dapr (v1.18 docs) applies named **timeouts**, **retries** (constant/exponential backoff), and **circuit breakers** to apps via service invocation, to components, and to actors; its overview page does not state an idempotency requirement for retries ([Dapr — Resiliency overview](https://docs.dapr.io/operations/resiliency/resiliency-overview/)). Prior repo research F3.4 covers Orleans' virtual-actor model.
**Analysis**: Two production systems that sell location transparency both (a) put a **timeout on every call** and (b) make **retry → duplicate** the caller's problem. Orleans documents this honestly; Dapr's overview leaves it implicit — an example of exactly the gap a port contract must close.

### Finding C6.4: NATS request/reply distinguishes "no one is there" from "too slow" — the one failure signal location-transparent transports can give honestly

**Evidence**: "When you send a request to a subject with zero subscribers, the server knows immediately that nobody can answer. Rather than let your timeout run, it sends back a **no responders** signal right away: a reply carrying a `503` status … This is the difference between 'the inventory service is slow' (you get a timeout after 2s) and 'the inventory service isn't running at all' (you get no responders in milliseconds)." "A timeout tells you the answer didn't arrive in time, not *why* — the responder might be slow, or not there at all." Queue groups: "you run several copies and let NATS hand each request to exactly one of them … built-in load balancing with no broker in the middle." Core NATS delivery: "core NATS is at-most-once … If a subscriber is offline, restarting, or not subscribed yet, it never sees that message."
**Source**: [NATS docs — Request-reply](https://docs.nats.io/learn/core-nats/request-reply); [NATS docs — Core NATS request-reply overview](https://docs.nats.io/nats-concepts/core-nats/reqreply) — accessed 2026-09-25.
**Confidence**: High (official documentation)
**Verification**: Orleans' timeout semantics (C6.3) and Waldo's indistinguishability claim (C6.1).
**Analysis**: "No responders" is the only failure that a subject-routed transport can report *with certainty* (a definite "not executed"). Everything else is a timeout of unknown outcome. That gives a minimum honest error taxonomy for ports.

### Port contract — what a local/remote-agnostic port must carry (synthesis, interpretation)

Derived from C6.1–C6.4; each clause names the evidence that forces it.

| Contract element | Why it is required | Local adapter behaviour | Remote adapter behaviour |
|---|---|---|---|
| **Deadline on every call** (caller-supplied, propagated) | Orleans/Dapr/NATS all time out every call; Waldo latency | Usually met immediately | Enforced; expiry → `Unknown` |
| **Three-way outcome**: `Ok` / `DefinitelyNotExecuted` / `OutcomeUnknown` | Waldo indeterminacy; NATS no-responders vs timeout | Never returns `OutcomeUnknown` | Must map transport errors honestly: routing miss / refused connect → `DefinitelyNotExecuted`; timeout / reset after send → `OutcomeUnknown` |
| **Idempotency key or declared non-idempotence** per operation | Orleans: retries ⇒ duplicates, no dedup | Harmless | Required for any retry by caller or adapter |
| **Delivery semantics declared** (at-most-once vs at-least-once) | Orleans default at-most-once; core NATS at-most-once | Trivially exactly-once | Must be stated, not inherited from the transport |
| **Ordering declared** (ordered / unordered stream) | Hydro's `TotalOrder`/`NoOrder` types (C2.3) | Ordered | May reorder across reconnects unless sequenced |
| **Staleness bound on reads** (index or timestamp returned with data) | nomad#18267; Kubernetes consistent-reads-from-cache (C3.2) | Current | Returns the index it read at; caller rejects regressions |
| **Cancellation** | Waldo partial failure; cancelled remote work may still run | Stops work | Best-effort; outcome `Unknown` unless acknowledged |
| **Adapter identity visible to the caller** (which node served it) | wasmCloud "intentional, not implicit" (C6.2) | `local` | `remote(node)` — needed for fencing and for diagnosing split-brain |
| **Backpressure signal** (`Overloaded` distinct from `Unavailable`) | Scuttlebutt flow control; Corrosion #198 retrofit | Rare | Required so callers shed load rather than retry-storm |

The design point this table argues for: **write every port as if it were remote** (Waldo's second path), give the local adapter the same signature, and forbid adapters from converting `OutcomeUnknown` into either `Ok` or `Err(NotDone)`.


## Challenging the hypothesis: "gossip carries facts, the log carries decisions"

**Verdict (interpretation, grounded in C2, C3, C5)**: the hypothesis survives as a *scaling strategy*, but it is the wrong *classification rule*. Three amendments are needed, and one counter-example must be acknowledged.

**Evidence for** — three large systems independently moved membership/ownership decisions from gossip to an ordered log and kept gossip for liveness: Cassandra CEP-21, ScyllaDB 6.0 (mandatory from 2025.2), and Kafka KRaft (from ZooKeeper, with brokers fenced through the log) (C5.4, C3.1). Fly separates a gossip fact layer (Corrosion) from worker-held state and exceeds the ~5k-node ceiling of systems that do not (C5.5; prior research F1).

**Counter-evidence** — Consul, Nomad, and Kubernetes route per-node facts (health checks, allocation status, pod status) *through* consensus, rate-limited, and they work — up to ~5k nodes per cluster (C5.5). So "facts must not go through the log" is not a correctness truth; it is what lets a flat cluster exceed that ceiling.

**Amendment 1 — classify by *correctness impact*, not by fact/decision.** CEP-21's line is "transient and/or non-correctness impacting" state on gossip; everything a correctness decision depends on goes to the log. Some *facts* therefore belong in the log: node registration and fencing epochs, voter set, capacity/range grants. And by CALM (C2.1), a gossiped fact becomes a decision the moment it is read non-monotonically ("no heartbeat ⇒ down") — the *reading* must be logged even if the heartbeats are not.

**Amendment 2 — decisions can be pre-granted.** The log does not need to carry *every* decision; it can carry a **grant** (an address block, an ID prefix, a capacity or quota share) after which many decisions are local and coordination-free (Bailis "choose some value" + escrow; Kubernetes podCIDR; Twine entitlements) (C1.5, C2.2).

**Amendment 3 — facts never get a second writer; ownership transfers are decisions.** When a fact's owner dies, the transfer is committed as an epoch/fence in the log; readers discount facts from a fenced epoch; the new owner writes its own records (C5.8). This preserves strict single-writer facts — which removes the need for LWW/CRDT merge — and is the only place the two planes must interact.

**Where the hypothesis is silent and must be extended** — *occurrences*. A gossip layer that syncs latest state (Scuttlebutt, LWW) loses intermediate values by construction (C5.2; prior research on crash observability). Crash/restart occurrences need append-only per-origin keys with bounded retention, which a per-origin log provides and an LWW table does not.

**Restated hypothesis (research proposal)**: *The log carries everything a correctness decision depends on — decisions, grants, fences, and the non-monotone readings of facts; gossip carries per-origin, single-writer facts whose loss or delay can only make decisions late, never wrong.*

## Known candidates — latest state

| Candidate | Problem | Latest state (2024–2026) | Maturity | Finding |
|---|---|---|---|---|
| Leader commits placement decision | C1 | Unchanged model; Nomad C2M ~1,500 placements/s; Twine ~1,000 job allocs/s per allocator | Production (Nomad, K8s, Twine) | C1.2–C1.3 |
| Calvin-style deterministic replay | C1 | Unsafe for evolving scheduler code without a cluster-wide version switch (TigerBeetle multiversion) | Production only in DBs | C1.1 |
| Omega / Nomad plan queue (optimistic) | C1 | Twine, Protean confirm negligible conflict rates at Meta/Azure scale | Production | C1.2 |
| Sparrow / Hawk (decentralized) | C1 | No new production adoption; Firmament shows centralized matches latency at 10k machines | Research | C1.4 |
| Mesos resource offers | C1 | Apache Mesos retired Aug 2025, moved to Attic Oct 2025 | Retired | C1.6 |
| Escrow / bounded counters | C1, C2 | 2026 Rust prototype (`bcounter`); Twine entitlements are the industrial analogue | Research / prototype | C1.5 |
| CALM / I-confluence | C2 | Keep CALM and CRDT On (VLDB 2023) extends to *reads* over CRDTs | Theory, settled | C2.1–C2.2 |
| Hydro / DFIR | C2, C6 | SIGMOD 2024, POPL 2025, ICDT 2025; `hydro_lang` 0.17.0-alpha | Alpha | C2.3 |
| Pull from local log replica | C3 | Kafka KRaft (brokers as observers) in production; Nomad #18267 stale-read hazard | Production | C3.1–C3.2 |
| Kubernetes topology spread / Nomad spread | C4 | K8s `minDomains` usable without a feature gate since v1.30; `matchLabelKeys` beta v1.27; spread remains placement-time only | Production | C4.1–C4.2 |
| Corrosion | C5 | Active 2026 development (Plumtree, supervision, corro-pg); no release since v1.0.0 (May 2024); #198 closed without visible PR; tombstone GC open (#533) | Production at Fly only | C5.1 |
| Scuttlebutt / chitchat | C5 | chitchat maintained under Datadog-owned Quickwit; small-state design | Production (Quickwit) | C5.2–C5.3 |
| NATS JetStream KV | C5 | CP per stream; minority stops accepting writes | Production | C5.6 |
| Consul catalog anti-entropy | C5 | Facts into Raft with size-scaled sync intervals | Production (≤5k agents/DC guidance) | C5.5 |
| Orleans / Dapr / wasmCloud / NATS req-reply | C6 | Orleans at-most-once default (doc updated 2026-01); wasmCloud moving to intentional remoting (2025) | Production | C6.2–C6.4 |

## Unexplored alternatives (ranked)

Ranked by (expected benefit to the target design) × (strength of evidence), highest first. "Beats / loses" is relative to the current direction as stated in the brief.

1. **Epoch-fenced single-writer facts with ownership transfer committed in the log** (C5.8, C5.4, C3.1).
   - *Solves*: the second-writer problem (C5), stale workers after partition (C3), without CRDT merge.
   - *Trade-offs*: every reader must apply an epoch filter; a returning node must catch up before its facts count.
   - *Evidence*: Cassandra CEP-21 epochs, Kafka KIP-500 fencing, Chubby sequencers — all production.
   - *Beats*: LWW rows (no clock race between owner and controller); pure Scuttlebutt (dead-node rows cannot be retracted except by grace-period GC).
   - *Loses*: nothing structural; costs one log entry per ownership change.
2. **Escrow grants — capacity shares, quota shares, ID prefixes, and address blocks committed once, consumed locally** (C1.5, C2.2).
   - *Solves*: coordination-free ID and address allocation; minority-side admission within grants; quota enforcement without per-operation consensus.
   - *Trade-offs*: rights can be stranded on a partitioned or dead node until reclaimed (a logged decision); rebalancing policy is needed.
   - *Evidence*: Kubernetes podCIDR per node, Twine entitlements (production); bounded counters (peer-reviewed); `bcounter` (prototype).
   - *Beats*: one log entry per allocation for addresses and IDs.
   - *Loses*: to per-decision commit when a *specific* value is required (operator-chosen names and VIPs).
3. **Hierarchical aggregation of facts (Astrolabe-style zones)** (C5.7).
   - *Solves*: the "every node holds every fact" scaling wall at thousands of nodes across regions.
   - *Trade-offs*: queries across zones see summaries, not rows; the aggregation functions must be defined up front.
   - *Evidence*: peer-reviewed (TOCS 2003); Fly's regionalization is an ad-hoc two-level instance (prior research F2).
   - *Beats*: a flat Corrosion or flat Scuttlebutt pool beyond ~5k nodes.
   - *Loses*: simplicity at small scale. No current open-source Rust implementation was found.
4. **Rateless set reconciliation (Rateless IBLT) for rejoin anti-entropy** (C5.7).
   - *Solves*: the rejoin-with-millions-of-changes backlog (Corrosion #198 class); cost scales with the difference, not the state.
   - *Evidence*: SIGCOMM 2024 (2–2000× less computation than prior IBLT schemes, under 2× the information-theoretic bound on communication).
   - *Maturity*: research; no orchestrator use found.
5. **Optimistic parallel scheduling on learners, verify-and-commit on the leader** (C1.2).
   - *Solves*: leader CPU as the scheduling bottleneck, without changing the log's shape.
   - *Evidence*: Omega, Twine, Protean, Nomad.
   - *Status*: this is seed (c) — listed because the brief treats it as an *alternative* when it is the natural *evolution* of (a).
6. **Failure-domain slots as a placement vocabulary (AWS partition placement groups)** (C4.3).
   - *Solves*: explicit placement without machine names; topology-aware replicated workloads.
   - *Evidence*: AWS EC2 (production).
7. **Worker right of refusal** — a committed assignment is an offer that the worker may reject, with the rejection recorded as a fact (C1.5, C1.6, C3.4).
   - *Solves*: capacity estimates that are stale or wrong; it captures most of Fly's synchronous-yes/no benefit.
   - *Evidence*: Mesos offers and Fly's market model, in inverted form. This is a synthesis; no system does exactly this.
8. **Fetch-as-heartbeat for learners (KRaft)** (C3.1).
   - *Solves*: a separate liveness stream for nodes that are already fetching the log.
   - *Evidence*: Kafka (production).
9. **Typed ordering and delivery markers on channels (Hydro)** (C2.3, C6).
   - *Solves*: silently feeding unordered or at-least-once data to consumers that need order.
   - *Evidence*: Hydro (alpha); the idea is borrowable without the dependency.

## Comparison against the current direction

| Problem | Current direction (brief) | Evidence verdict | Where an alternative wins | Where the current direction wins |
|---|---|---|---|---|
| C1 placement | Leader computes and commits decisions | **Supported** (C1.1–C1.3, C1.8) | (c) optimistic parallel scheduling when scheduler CPU binds; (e) escrow for quotas and minority-side admission | Upgrade safety and replay stability vs (b); constraint expressiveness vs (d) |
| C2 what needs consensus | "Facts vs decisions" | **Partially supported** — replace with correctness-impact rule + grants + fencing | CEP-21 classification; Bailis table | Already routes "node is down" through the log, which CALM says is required |
| C3 assignments | Workers pull from local replica; nothing pushed | **Supported**, with mandatory stale-read guards (C3.2) | Fly push-to-authoritative-worker gives synchronous yes/no | Ordered history; no read load on voters; KRaft precedent |
| C3 partition | Cut-off worker keeps running committed work | **Supported as default**; needs per-workload self-fence option (Kafka, Nomad `stop_on_client_after`) | Self-fencing for singleton / at-most-one workloads | Availability for the common replicated case |
| C4 placement input | Attributes and failure domains preferred; auto-spread default | **Supported** (Borg, Twine, K8s, Nomad) | Failure-domain slots (AWS) for explicit placement; rebalancer for standing spread | Name-pinning avoided |
| C5 observation | Corrosion incumbent; per-origin logs + Scuttlebutt considered | **Considered alternative favoured on evidence**, conditional on epoch fencing, flow control, and GC design | Per-origin single-writer + epochs avoids LWW loss and backfill storms; hierarchical aggregation for scale | Corrosion exists, is maintained, and has an 800-server production record; the alternative has no equivalent at-scale evidence |
| C6 ports | Local or remote adapter behind one port | **Supported only if ports are remote-shaped** (Waldo) | — | — |

## Conflicting information

### Conflict 1: Should a worker cut off from the control plane keep running?
**Position A**: Yes — "Even if all Twine components fail, existing tasks continue to run" ([Twine §4](https://www.usenix.org/system/files/osdi20-tang.pdf), reputation 1.0); Nomad's default keeps allocations running and replaces them on the majority side ([Nomad `disconnect`](https://developer.hashicorp.com/nomad/docs/job-specification/disconnect), 1.0).
**Position B**: No — a Kafka broker "will re-enter the fenced state if it can't contact the active controller" ([KIP-500](https://cwiki.apache.org/confluence/display/KAFKA/KIP-500:+Replace+ZooKeeper+with+a+Self-Managed+Metadata+Quorum), 1.0); Nomad's `stop_on_client_after` stops allocations on a disconnected client.
**Assessment**: Not a real contradiction — the right answer depends on whether the workload carries an *at-most-one* invariant (a partition leader, a singleton writer). Both are authoritative; the target design needs both behaviours as a per-workload policy.

### Conflict 2: Must per-node facts stay off the consensus log?
**Position A**: Consul syncs agent-authoritative services and health checks into a Raft-replicated catalog ([Consul consistency](https://developer.hashicorp.com/consul/docs/concept/consistency), 1.0); Nomad commits allocation status through Raft (prior research F1.8).
**Position B**: Cassandra keeps gossip for failure-detection input and non-correctness state ([CEP-21](https://cwiki.apache.org/confluence/display/CASSANDRA/CEP-21:+Transactional+Cluster+Metadata), 1.0); Fly keeps facts in Corrosion ([Fly — Corrosion](https://fly.io/blog/corrosion/), 1.0 vendor-primary).
**Assessment**: Both work. The evidence supports Position B only as a scaling strategy beyond ~5k nodes, and only for facts that do not directly gate correctness.

### Conflict 3: Is Corrosion's backpressure gap still open?
**Position A**: Prior repo research (2026-04-20) recorded issue #198 as open.
**Position B**: On 2026-09-25 #198 is closed, with no linked PR visible ([issue #198](https://github.com/superfly/corrosion/issues/198)).
**Assessment**: Unresolved. The closure may reflect an implementation, a duplicate, or a wont-fix. It should not be read as "backpressure implemented" without a code reference (see Knowledge Gaps).

### Conflict 4: Is decentralization needed for low-latency scheduling?
**Position A**: Sparrow — a centralized design has "throughput and availability limitations" for sub-second tasks ([Sparrow, SOSP 2013](https://sigops.org/s/conferences/sosp/2013/papers/p69-ousterhout.pdf), 1.0).
**Position B**: Firmament "matches the placement latency of distributed schedulers for workloads of short tasks" at over ten thousand machines ([Firmament, OSDI 2016](https://www.usenix.org/conference/osdi16/technical-sessions/presentation/gog), 1.0).
**Assessment**: The later paper answers the earlier one for clusters up to ~10k machines. For long-running microVMs neither latency regime is binding.

## Knowledge gaps

### Gap 1: Corrosion backpressure resolution
**Issue**: Whether #198's closure corresponds to shipped backpressure. | **Attempted**: issue page, recent commits list, PR search. | **Recommendation**: inspect the Corrosion source for a bounded change queue, or ask the maintainers.

### Gap 2: No at-scale production evidence for per-origin logs with Scuttlebutt carrying high-cardinality observation rows
**Issue**: chitchat and Cassandra gossip carry small per-node state; no system was found that carries tens of thousands of per-allocation rows per node this way. | **Attempted**: chitchat README, Scuttlebutt paper, Cassandra CEP-21. | **Recommendation**: build a bounded prototype and measure digest size, bandwidth per node, and rejoin cost at 1k/5k/10k simulated nodes before committing.

### Gap 3: No orchestrator found using escrow or bounded counters for placement or quota
**Issue**: Twine entitlements are coarse (whole machines); bounded counters are proven only in databases (Riak). | **Attempted**: Balegas SRDS 2015, `bcounter`, Twine. | **Recommendation**: treat as research; start with address blocks and quotas, where the invariant really is numeric.

### Gap 4: Verbatim text not retrieved for some official pages
**Issue**: The Kubernetes "Taint based Evictions", topology-spread "Known limitations", `nodeName` limitations, and node-controller pages came back as tool summaries; the Chubby sequencer mechanism came from a search index of the paper. | **Attempted**: direct fetches. | **Recommendation**: re-fetch these pages before quoting them in an ADR; the substance matches the upstream pages as the researcher knows them, but the wording is not guaranteed.

### Gap 5: wasmCloud's next major release
**Issue**: The Q3 2025 roadmap statement ("will not use NATS to communicate between components by default") was seen only through the search index; release status in 2026 was not verified. | **Recommendation**: check the wasmCloud releases page.

### Gap 6: Workload-specific sizing inputs
**Issue**: No source gives an orchestrator's fact write rate per node (allocation-status churn, health flaps) or Overdrive's required placements per second. The 10³/s figures come from Nomad and Twine workloads. | **Recommendation**: derive from the target's expected workload mix; this sizing drives the choice between a flat and a hierarchical observation layer.

### Gap 7: Mencius and EPaxos full texts not fetched
**Issue**: C1.7's reduction of option (f) to (c) is an argument, supported only by the Mencius abstract. | **Recommendation**: acceptable for a design-level conclusion; revisit only if multi-leader ordering is proposed for cluster A.

### Gap 8: chitchat maintenance after the Datadog acquisition
**Issue**: The repository shows active history, but its 2026 release cadence was not measured. | **Recommendation**: check commit dates before taking a dependency.

## Recommendation for the target design

These are **research recommendations**, not decisions. Each one names the evidence it rests on and the question that would overturn it.

1. **Keep "leader commits placement decisions" and name optimistic verify-and-commit as its scaling path** (C1.1–C1.3). Reject deterministic replay of placement inputs: a scheduler changes too often for a cluster-wide version switch to be cheap. Treat "every node proposes" as a multi-leader *ordering* question for cluster A, not a placement design. Use sampling only as a heuristic inside the scheduler. *Overturned if*: the placement function can be frozen and versioned in the log, so that replay is cheap and safe.
2. **Replace the "facts vs decisions" rule with a correctness-impact rule, plus grants and fences** (C2 table, C5.4). The log carries:
   - name claims, replica counts, spread and anti-affinity decisions, and rollout progress;
   - node registration and fencing epochs, and the voter set;
   - deletes, as cascading tombstones;
   - block, range, and quota grants.
   Gossip carries per-origin facts. *Overturned if*: a fact whose loss only delays decisions turns out to cause a wrong decision.
3. **Specify the worker-side read contract before building "workers read their local replica"** (C3.2). The replica's applied index must be monotone and never regress. A worker takes destructive action only on an explicit committed stop or tombstone, never on absence. A node that rejoins catches up past its last-acted index before taking destructive action. *Evidence*: Nomad #18267 happened in production.
4. **Make partition behaviour a per-workload policy with three settings, mirroring Nomad's `disconnect` block** (C3.3):
   - keep running or self-fence on losing the log;
   - replace or not after a grace period (a logged decision);
   - which instance to keep on reconnect.
   Add a mass-failure brake: stop replacing when a large fraction of nodes looks dead at once. This follows Kubernetes' per-zone eviction limits and Twine's rate-limits on destructive operations.
5. **Prototype the observation layer as strictly single-writer, epoch-stamped per-origin records** before committing to Corrosion or replacing it (C5.1–C5.8). Measure the prototype against Corrosion on its three documented failure modes: rejoin backlog, schema change, and tombstone GC. Build in:
   - Scuttlebutt digests with the paper's flow control;
   - append-only event keys with bounded retention, for crash and restart occurrences;
   - ownership transfer through the log, as epochs.
   Plan for hierarchical aggregation (zones or regions) instead of one flat pool beyond a few thousand nodes.
6. **Placement vocabulary** (C4): hard attribute constraints, a default soft spread across the failure-domain hierarchy, failure-domain slots for topology-aware workloads, and data-locality affinity. Reserve machine-name pinning for break-glass use. Decide explicitly whether spread holds only at placement time or continuously. If continuously, run a rebalancer as a rate-limited reconciler that commits its moves through the log.
7. **Port contract** (C6): write every port as if it were remote. Carry:
   - deadlines and a three-way outcome (`Ok` / `DefinitelyNotExecuted` / `OutcomeUnknown`);
   - declared idempotency, delivery semantics, and ordering;
   - a staleness index on reads;
   - which adapter served the call, and an overload signal.
   The local adapter implements the same contract.

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|---|---|---|---|---|---|
| Calvin (SIGMOD 2012) | cs.umd.edu (author copy of ACM paper) | High (1.0) | academic | 2026-09-25 | Y (TigerBeetle) |
| Omega (EuroSys 2013) | research.google | High (1.0) | academic | 2026-09-25 | Y (Twine, Protean, Nomad) |
| Twine (OSDI 2020) | usenix.org | High (1.0) | academic | 2026-09-25 | Y |
| Protean (OSDI 2020) | usenix.org | High (1.0) | academic | 2026-09-25 | Y |
| Firmament (OSDI 2016) | usenix.org | High (1.0) | academic | 2026-09-25 | Y |
| Sparrow (SOSP 2013) | sigops.org / dl.acm.org | High (1.0) | academic | 2026-09-25 | Y (Firmament, Hawk) |
| Hawk (ATC 2015) | usenix.org | High (1.0) | academic | 2026-09-25 | Y |
| Mesos (NSDI 2011) | usenix.org / berkeley.edu | High (1.0) | academic | 2026-09-25 | Y (Omega) |
| Mencius (OSDI 2008) | dl.acm.org | High (1.0) | academic | 2026-09-25 | Partial |
| Compartmentalized Paxos | arxiv.org | High (1.0) | academic | 2026-09-25 | Partial |
| Bounded counters (SRDS 2015) | arxiv.org | High (1.0) | academic | 2026-09-25 | Y (Twine) |
| Keeping CALM (CACM 2020) | arxiv.org | High (1.0) | academic | 2026-09-25 | Y |
| Keep CALM and CRDT On (VLDB 2023) | arxiv.org / dl.acm.org | High (1.0) | academic | 2026-09-25 | Y |
| Coordination Avoidance (VLDB 2015) | arxiv.org | High (1.0) | academic | 2026-09-25 | Y (Kubernetes) |
| Scuttlebutt (LADIS 2008) | dl.acm.org / cs.cornell.edu | High (1.0) | academic | 2026-09-25 | Y (chitchat) |
| Delta-state CRDTs (JPDC 2018) | arxiv.org | High (1.0) | academic | 2026-09-25 | Y |
| Dynamo (SOSP 2007) | allthingsdistributed.com (author-hosted) | Medium-High (0.8) | academic | 2026-09-25 | Y |
| Rateless IBLT (SIGCOMM 2024) | arxiv.org / dl.acm.org | High (1.0) | academic | 2026-09-25 | Partial |
| Astrolabe (TOCS 2003) | dl.acm.org | High (1.0) | academic | 2026-09-25 | Y (Fly regionalization) |
| Chubby (OSDI 2006) | usenix.org | High (1.0) | academic | 2026-09-25 | Y (CEP-21, KIP-500) |
| Waldo et al. (TR-94-29) | gatech.edu mirror / dl.acm.org | High (1.0) | academic | 2026-09-25 | Y |
| Borg (EuroSys 2015) + SRE book | research.google / sre.google | High (1.0) | academic / official | 2026-09-25 | Y |
| Kubernetes docs (virtual IPs, nodes, networking, taints, topology spread, assign-pod-node) | kubernetes.io | High (1.0) | official | 2026-09-25 | Y |
| Kubernetes blog (consistent reads from cache, v1.31) | kubernetes.io | High (1.0) | official | 2026-09-25 | Y |
| Nomad docs (disconnect, spread, scheduling) | developer.hashicorp.com | High (1.0) | official | 2026-09-25 | Y |
| Consul consistency / anti-entropy | developer.hashicorp.com | High (1.0) | official | 2026-09-25 | Y |
| AWS EC2 placement strategies | docs.aws.amazon.com | High (1.0) | technical docs | 2026-09-25 | Y (Twine) |
| NATS docs (KV, JetStream clustering, request-reply) | docs.nats.io | High (1.0) | official | 2026-09-25 | Y |
| Dapr resiliency (v1.18) | docs.dapr.io | High (1.0) | official | 2026-09-25 | Y |
| Orleans delivery guarantees | learn.microsoft.com | High (1.0) | official | 2026-09-25 | Y |
| Apache Attic — Mesos | attic.apache.org | High (1.0) | official | 2026-09-25 | N (foundation record) |
| KIP-500, KIP-595 | cwiki.apache.org | High (1.0) | official | 2026-09-25 | Y |
| CEP-21, CASSANDRA-18330 | cwiki.apache.org / issues.apache.org | High (1.0) | official | 2026-09-25 | Y (ScyllaDB) |
| Fly blog (Carving the scheduler; Corrosion) | fly.io | High (1.0) | vendor-primary | 2026-09-25 | Y |
| wasmCloud blog + lattice docs | wasmcloud.com | High (1.0) | official | 2026-09-25 | Partial |
| TigerBeetle ARCHITECTURE.md, upgrading.md | github.com | Medium-High (0.8) | industry | 2026-09-25 | Y |
| hashicorp/c2m + C2M blog | github.com / hashicorp.com | Medium-High (0.8) | vendor benchmark | 2026-09-25 | Partial |
| kostja/bcounter + docs.rs | github.com / docs.rs | Medium-High (0.8) | prototype | 2026-09-25 | N |
| hydro-project/hydro + releases; hydro.run | github.com / hydro.run (out-of-list primary) | Medium-High (0.8) | industry / research | 2026-09-25 | Y (SIGMOD/POPL papers) |
| nomad#18267 | github.com | Medium-High (0.8) | industry (bug tracker) | 2026-09-25 | Y (K8s, KIP-500) |
| superfly/corrosion releases, commits, #198, #533, #527 | github.com | Medium-High (0.8) | industry | 2026-09-25 | Partial |
| quickwit-oss/chitchat | github.com | Medium-High (0.8) | industry | 2026-09-25 | Y (Scuttlebutt paper) |
| Red Hat KRaft deep dive | developers.redhat.com (out-of-list) | Medium-High (0.8) | industry | 2026-09-25 | Y |
| ScyllaDB 6.0 blog + docs | scylladb.com (out-of-list primary) | Medium-High (0.8) | vendor-primary | 2026-09-25 | Y (CEP-21) |
| Quickwit / Datadog acquisition posts | quickwit.io, datadoghq.com (out-of-list) | Medium (0.6) | vendor | 2026-09-25 | Y (each other) |
| Nomad load-shedding blog (via prior research) | hashicorp.com | Medium-High (0.8) | vendor engineering | 2026-09-25 | Y |

Reputation: High (1.0): ~40 sources (≈69 %) | Medium-high (0.8): ~16 (≈28 %) | Medium (0.6): 2 (≈3 %) | **Avg ≈ 0.93**

Bias notes: Fly, HashiCorp, ScyllaDB, Quickwit/Datadog, and wasmCloud describe their own products (vendor-primary; commercial interest), so every load-bearing claim from them is paired with an independent or peer-reviewed source, or its confidence is reduced. Twine and Protean are also vendor systems, but they appear in peer-reviewed venues.

## Full Citations

[1] Thomson, A., Diamond, T., Weng, S.-C., Ren, K., Shao, P., Abadi, D. J. "Calvin: Fast Distributed Transactions for Partitioned Database Systems". SIGMOD 2012. https://www.cs.umd.edu/~abadi/papers/calvin-sigmod12.pdf. Accessed 2026-09-25.
[2] Schwarzkopf, M., Konwinski, A., Abd-El-Malek, M., Wilkes, J. "Omega: flexible, scalable schedulers for large compute clusters". EuroSys 2013. https://research.google/pubs/omega-flexible-scalable-schedulers-for-large-compute-clusters/. Accessed 2026-09-25.
[3] Tang, C., et al. (Facebook). "Twine: A Unified Cluster Management System for Shared Infrastructure". OSDI 2020. https://www.usenix.org/system/files/osdi20-tang.pdf. Accessed 2026-09-25.
[4] Hadary, O., et al. (Microsoft). "Protean: VM Allocation Service at Scale". OSDI 2020. https://www.usenix.org/conference/osdi20/presentation/hadary. Accessed 2026-09-25.
[5] Gog, I., Schwarzkopf, M., Gleave, A., Watson, R. N. M., Hand, S. "Firmament: Fast, Centralized Cluster Scheduling at Scale". OSDI 2016. https://www.usenix.org/conference/osdi16/technical-sessions/presentation/gog. Accessed 2026-09-25.
[6] Ousterhout, K., Wendell, P., Zaharia, M., Stoica, I. "Sparrow: Distributed, Low Latency Scheduling". SOSP 2013. https://sigops.org/s/conferences/sosp/2013/papers/p69-ousterhout.pdf. Accessed 2026-09-25.
[7] Delgado, P., Dinu, F., Kermarrec, A.-M., Zwaenepoel, W. "Hawk: Hybrid Datacenter Scheduling". USENIX ATC 2015. https://www.usenix.org/conference/atc15/technical-session/presentation/delgado. Accessed 2026-09-25.
[8] Hindman, B., et al. "Mesos: A Platform for Fine-Grained Resource Sharing in the Data Center". NSDI 2011. https://www.usenix.org/conference/nsdi11/mesos-platform-fine-grained-resource-sharing-data-center. Accessed 2026-09-25.
[9] The Apache Software Foundation. "Mesos — The Apache Attic". https://attic.apache.org/projects/mesos.html. Accessed 2026-09-25.
[10] Mao, Y., Junqueira, F. P., Marzullo, K. "Mencius: Building Efficient Replicated State Machines for WANs". OSDI 2008. https://dl.acm.org/doi/10.5555/1855741.1855767. Accessed 2026-09-25.
[11] Whittaker, M., et al. "Scaling Replicated State Machines with Compartmentalization". arXiv:2012.15762. https://arxiv.org/pdf/2012.15762. Accessed 2026-09-25.
[12] Balegas, V., et al. "Extending Eventually Consistent Cloud Databases for Enforcing Numeric Invariants". SRDS 2015. https://arxiv.org/abs/1503.09052. Accessed 2026-09-25.
[13] kostja. "bcounter — escrow bounded counter for distributed capacity quotas". https://github.com/kostja/bcounter ; https://docs.rs/bcounter/latest/bcounter/. Accessed 2026-09-25.
[14] Hellerstein, J. M., Alvaro, P. "Keeping CALM: When Distributed Consistency is Easy". CACM 2020 / arXiv:1901.01930. https://arxiv.org/abs/1901.01930. Accessed 2026-09-25.
[15] Laddad, S., Power, C., Milano, M., Cheung, A., Crooks, N., Hellerstein, J. M. "Keep CALM and CRDT On". PVLDB 16(4), 2023. https://arxiv.org/abs/2210.12605. Accessed 2026-09-25.
[16] Bailis, P., Fekete, A., Franklin, M. J., Ghodsi, A., Hellerstein, J. M., Stoica, I. "Coordination Avoidance in Database Systems". PVLDB 8(3), 2015. https://arxiv.org/abs/1402.2237. Accessed 2026-09-25.
[17] Hydro project. "hydro-project/hydro"; "Research Publications"; "hydro_lang v0.17.0-alpha.5". https://github.com/hydro-project/hydro ; https://hydro.run/research/ ; https://github.com/hydro-project/hydro/releases/tag/hydro_lang-v0.17.0-alpha.5. Accessed 2026-09-25.
[18] Kubernetes. "Virtual IPs and Service Proxies". https://kubernetes.io/docs/reference/networking/virtual-ips/. Accessed 2026-09-25.
[19] Kubernetes. "Nodes". https://kubernetes.io/docs/concepts/architecture/nodes/. Accessed 2026-09-25.
[20] Kubernetes. "Cluster Networking". https://kubernetes.io/docs/concepts/cluster-administration/networking/. Accessed 2026-09-25.
[21] Kubernetes. "Taints and Tolerations". https://kubernetes.io/docs/concepts/scheduling-eviction/taint-and-toleration/. Accessed 2026-09-25.
[22] Kubernetes. "Pod Topology Spread Constraints". https://kubernetes.io/docs/concepts/scheduling-eviction/topology-spread-constraints/. Accessed 2026-09-25.
[23] Kubernetes. "Assigning Pods to Nodes". https://kubernetes.io/docs/concepts/scheduling-eviction/assign-pod-node/. Accessed 2026-09-25.
[24] Kubernetes Blog. "Kubernetes v1.31: Accelerating Cluster Performance with Consistent Reads from Cache". 2024-08-15. https://kubernetes.io/blog/2024/08/15/consistent-read-from-cache-beta/. Accessed 2026-09-25.
[25] Apache Kafka. "KIP-500: Replace ZooKeeper with a Self-Managed Metadata Quorum". https://cwiki.apache.org/confluence/display/KAFKA/KIP-500:+Replace+ZooKeeper+with+a+Self-Managed+Metadata+Quorum. Accessed 2026-09-25.
[26] Apache Kafka. "KIP-595: A Raft Protocol for the Metadata Quorum". https://cwiki.apache.org/confluence/x/Li7cC. Accessed 2026-09-25.
[27] Red Hat Developer. "A deep dive into Apache Kafka's KRaft protocol". 2025-09-17. https://developers.redhat.com/articles/2025/09/17/deep-dive-apache-kafkas-kraft-protocol. Accessed 2026-09-25.
[28] HashiCorp. "Nomad Server may instruct Clients to erroneously stop and GC all of their allocations" (issue #18267). 2023-08-20. https://github.com/hashicorp/nomad/issues/18267. Accessed 2026-09-25.
[29] HashiCorp. "Nomad `disconnect` block". https://developer.hashicorp.com/nomad/docs/job-specification/disconnect. Accessed 2026-09-25.
[30] HashiCorp. "Nomad `spread` block". https://developer.hashicorp.com/nomad/docs/job-specification/spread. Accessed 2026-09-25.
[31] HashiCorp. "Scheduling in Nomad". https://developer.hashicorp.com/nomad/docs/concepts/scheduling/scheduling. Accessed 2026-09-25.
[32] HashiCorp. "HashiCorp Nomad Meets the 2 Million Container Challenge"; hashicorp/c2m. https://www.hashicorp.com/en/blog/hashicorp-nomad-meets-the-2-million-container-challenge ; https://github.com/hashicorp/c2m. Accessed 2026-09-25.
[33] HashiCorp. "Load shedding in the Nomad eval broker". https://www.hashicorp.com/en/blog/load-shedding-in-the-nomad-eval-broker. Accessed 2026-09-25 (via prior repo research).
[34] HashiCorp. "Consul — Consistency (anti-entropy)". https://developer.hashicorp.com/consul/docs/concept/consistency. Accessed 2026-09-25.
[35] Fly.io. "Carving The Scheduler Out Of Our Orchestrator". https://fly.io/blog/carving-the-scheduler-out-of-our-orchestrator/. Accessed 2026-09-25.
[36] Fly.io. "Corrosion". 2025-10-22. https://fly.io/blog/corrosion/. Accessed 2026-09-25 (via prior repo research).
[37] Verma, A., et al. "Large-scale cluster management at Google with Borg". EuroSys 2015. https://research.google/pubs/large-scale-cluster-management-at-google-with-borg/ ; Google SRE Book, "The Production Environment at Google". https://sre.google/sre-book/production-environment/. Accessed 2026-09-25.
[38] Amazon Web Services. "Placement strategies for your placement groups". https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/placement-strategies.html. Accessed 2026-09-25.
[39] Fly.io / superfly. "corrosion — releases, commits, issues #198, #533, PR #527". https://github.com/superfly/corrosion. Accessed 2026-09-25.
[40] van Renesse, R., Dumitriu, D., Gough, V., Thomas, C. "Efficient Reconciliation and Flow Control for Anti-Entropy Protocols". LADIS 2008. https://dl.acm.org/doi/10.1145/1529974.1529983. Accessed 2026-09-25.
[41] Quickwit. "chitchat". https://github.com/quickwit-oss/chitchat. Accessed 2026-09-25.
[42] Quickwit. "Quickwit joins Datadog"; Datadog. "Datadog acquires Quickwit". https://quickwit.io/blog/quickwit-joins-datadog ; https://www.datadoghq.com/blog/datadog-acquires-quickwit/. Accessed 2026-09-25.
[43] Apache Cassandra. "CEP-21: Transactional Cluster Metadata"; CASSANDRA-18330. https://cwiki.apache.org/confluence/display/CASSANDRA/CEP-21:+Transactional+Cluster+Metadata ; https://issues.apache.org/jira/browse/CASSANDRA-18330. Accessed 2026-09-25.
[44] ScyllaDB. "ScyllaDB 6.0: with Tablets & Strongly-Consistent Topology Updates" (2024-06-12); "Enable Consistent Topology Updates". https://www.scylladb.com/2024/06/12/introducing-scylladb-6-0-with-tablets-and-strongly-consistent-topology-updates/ ; https://docs.scylladb.com/manual/branch-2025.1/upgrade/upgrade-guides/upgrade-guide-from-2024.x-to-2025.1/enable-consistent-topology.html. Accessed 2026-09-25.
[45] NATS. "Key/Value Store"; "JetStream Clustering"; "Clustering & Replication Deep Dive"; "Request-reply"; "Core NATS request-reply". https://docs.nats.io/nats-concepts/jetstream/key-value-store ; https://docs.nats.io/running-a-nats-service/configuration/clustering/jetstream_clustering ; https://docs.nats.io/learn/clustering/ ; https://docs.nats.io/learn/core-nats/request-reply ; https://docs.nats.io/nats-concepts/core-nats/reqreply. Accessed 2026-09-25.
[46] Almeida, P. S., Shoker, A., Baquero, C. "Delta State Replicated Data Types". JPDC 111, 2018. https://arxiv.org/abs/1603.01529. Accessed 2026-09-25.
[47] DeCandia, G., et al. "Dynamo: Amazon's Highly Available Key-value Store". SOSP 2007. https://www.allthingsdistributed.com/2007/10/amazons_dynamo.html. Accessed 2026-09-25.
[48] Yang, L., Gilad, Y., Alizadeh, M. "Practical Rateless Set Reconciliation". SIGCOMM 2024. https://arxiv.org/pdf/2402.02668 ; https://dl.acm.org/doi/10.1145/3651890.3672219. Accessed 2026-09-25.
[49] van Renesse, R., Birman, K. P., Vogels, W. "Astrolabe: A Robust and Scalable Technology for Distributed System Monitoring, Management, and Data Mining". ACM TOCS 21(2), 2003. https://dl.acm.org/doi/10.1145/762483.762485. Accessed 2026-09-25.
[50] Burrows, M. "The Chubby lock service for loosely-coupled distributed systems". OSDI 2006. https://www.usenix.org/conference/osdi-06/chubby-lock-service-loosely-coupled-distributed-systems. Accessed 2026-09-25.
[51] Waldo, J., Wyant, G., Wollrath, A., Kendall, S. "A Note on Distributed Computing". Sun Microsystems Laboratories SMLI TR-94-29, November 1994. https://sites.cc.gatech.edu/classes/AY2010/cs4210_fall/papers/smli_tr-94-29.pdf. Accessed 2026-09-25.
[52] wasmCloud. "Charting the next steps for wasmCloud" (2025-07-03); "Lattice" (v1 docs). https://wasmcloud.com/blog/charting-the-next-steps-for-wasmcloud/ ; https://wasmcloud.com/docs/v1/concepts/lattice/. Accessed 2026-09-25.
[53] Microsoft. "Orleans messaging delivery guarantees" (updated 2026-01-27). https://learn.microsoft.com/en-us/dotnet/orleans/implementation/messaging-delivery-guarantees. Accessed 2026-09-25.
[54] Dapr. "Resiliency overview" (v1.18). https://docs.dapr.io/operations/resiliency/resiliency-overview/. Accessed 2026-09-25.
[55] TigerBeetle. "ARCHITECTURE.md"; "Upgrading". https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/ARCHITECTURE.md ; https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/operating/upgrading.md. Accessed 2026-09-25.

Prior repo research cited (not re-fetched): `docs/research/scalability/corrosion-crsqlite-production-scale-evidence.md`; `docs/research/orchestration/crash-observability-under-lww-comprehensive-research.md`; `docs/research/orchestration/nomad-scheduling-and-reconciler-pattern-research.md`; `docs/research/platform/fly-inside-out-orchestration-overdrive-relevance.md`; `docs/research/gossip-protocols/serf-internals-and-overdrive-fit.md` (Serf user events: fire-and-forget broadcast with TTL and Lamport clocks, Finding 2.1).

## Research Metadata

Duration: single session | Examined: ~75 pages and papers | Cited: 58 distinct sources (55 numbered citation entries, several bundling related pages from the same publisher) | Cross-references: 30 findings, each with a Verification line | Confidence across the 30 findings: High 19 (≈63 %), Medium-High 10 (≈33 %), Medium 1 (≈3 %) | Tool notes: several PDFs failed HTML extraction and were read as rendered pages (Calvin, Sparrow, Twine, Mesos, Bailis, Scuttlebutt, Waldo); four official pages returned summaries rather than verbatim text (Knowledge Gap 4); one fetch echoed the prompt and was discarded | Output: `docs/research/architecture/flat-cluster-placement-and-observation-comprehensive-research.md`
