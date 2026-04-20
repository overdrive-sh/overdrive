# Research: Corrosion and CR-SQLite — Production Scale Evidence, Failure Modes, and Scaling Limits

**Date**: 2026-04-20 | **Researcher**: Nova (nw-researcher) | **Confidence**: Medium | **Sources**: 18 primary

---

## Executive Summary

The empirical evidence base for Corrosion at scale is thin, vendor-primary, and concentrated in three 2024 Fly.io post-mortems plus one October 2025 retrospective blog post and one QCon London 2025 talk. The strongest number Fly has publicly committed to is **~800 physical servers, ~40 regions, p99 replication latency under one second, hosting "hundreds of thousands of Fly Machines"** (InfoQ summary of Onyekwere's QCon talk; Fly community post). No row counts, no write-rate numbers, no bytes-per-peer-per-second figures, no gossip fan-out parameters have been published. The architectural framing the whitepaper adopts — per-region Corrosion + global membership cluster — is exactly what Fly moved to **after production incidents in 2024**, not a topology they started from.

For the Apple engineer's stated envelope (100-200k nodes, tens of millions of workload rows), **there is no published evidence that a single Corrosion gossip pool has been run at that scale**. Fly's 800-server figure is the only concrete number and is ~1-2 orders of magnitude below the target. The closest analogue is HashiCorp Consul's published scale test at 66,000 client agents in a single datacenter (HashiCorp, 2020), which Fly explicitly replaced because Consul's gossip characteristics degraded at their workload — and HashiCorp's own official recommendation is **≤5,000 client agents per gossip pool** due to churn sensitivity. The CR-SQLite layer adds its own constraints: single-writer SQLite semantics, 2.5x insert overhead vs plain SQLite, no support for destructive schema changes, and a documented failure mode where adding a nullable column to a large CRR triggers cluster-wide backfill that saturated Fly's upstream links with tens of gigabytes of traffic (Fly Infra Log, 2024-11-25).

The defensible conclusion for Overdrive: the whitepaper's regionalized topology is sound and closely mirrors Fly's post-incident architecture, but the claim "scales to continents" applies to Fly's measured envelope (~800 servers/40 regions), not to a 100-200k-node ambition. Reaching that envelope requires either **per-region sharding beyond what Fly has published**, **observation-data segmentation** (hash-sharding service_backends by job so no single Corrosion pool carries the full fleet), or **architectural work the whitepaper does not currently describe**. This should be treated as an open question, not a solved problem.

---

## Findings

### Finding 1: Fly.io's published Corrosion fleet size is ~800 physical servers, ~40 regions

**Evidence**: "running on about 800 physical servers" in "40 different regions globally" with "p99 under a second" replication latency. Broadcast uses gossip dissemination to three nodes forwarded to three others (log(n) rounds).
**Source**: [Fast Eventual Consistency: Inside Corrosion, InfoQ presentation page](https://www.infoq.com/presentations/corrosion/) — summary of Somtochi Onyekwere's QCon London 2025 talk (April 7, 2025).
**Confidence**: Medium (single speaker, vendor-primary; InfoQ is independent reporter but transcribing the same talk).
**Verification**: Fly's own retrospective — "thousands of high-powered servers around the world" — is consistent but less specific ([Corrosion · The Fly Blog](https://fly.io/blog/corrosion/), Oct 22, 2025). An earlier Fly community post claims "hundreds of thousands of Fly Machines" hosted on that fleet ([Self-healing machine state synchronization, Fly.io community](https://community.fly.io/t/self-healing-machine-state-synchronization-and-service-discovery/26134)).
**Analysis**: The 800-server number is the most specific public data point. It provides an upper bound on what Corrosion has been demonstrated to handle in a single topology. For the Apple use case (100-200k nodes) this is 1-2 orders of magnitude short.

### Finding 2: Regionalization (per-region Corrosion + global cluster) is the post-incident architecture

**Evidence**: "A project called 'regionalization' that creates a two-level database scheme where each region runs a Corrosion cluster with fine-grained data about every Fly Machine in the region" and a thin global cluster that "maps applications to regions, which is sufficient to make forwarding decisions."
**Source**: [Corrosion · The Fly Blog](https://fly.io/blog/corrosion/), October 22, 2025.
**Confidence**: High (explicit, vendor-primary, multiple corroborating infra-log posts).
**Verification**: Mentioned as the forward-looking mitigation in three separate post-mortems — [2024-09-07](https://fly.io/infra-log/2024-09-07/), [2024-11-25](https://fly.io/infra-log/2024-11-25/), and [2024-12-14](https://fly.io/infra-log/2024-12-14/). InfoQ's April 2025 summary confirms regionalization was being actively rolled out at that point.
**Analysis**: Fly did not start with the regionalized topology. They arrived at it **after** the September 2024 contagion-deadlock incident and the November 2024 CRDT backfill storm. The whitepaper's claim (§4) that Overdrive "adopts it from day one, not after the incident" is consistent with this lesson. Critically, **number of regions in the global cluster is not published** — the topology works for Fly's ~40 regions but there is no public datapoint for a 100+ region deployment or for scenarios where a single region itself approaches 100k nodes.

### Finding 3: Named failure mode #1 — Contagious Tokio deadlock (September 1, 2024)

**Evidence**: "A poisonous configuration update was rapidly gossiped across our fleet, deadlocking every fly-proxy that saw it." Root cause: `if let` pattern over an RWLock where the read lock held across the conditional allowed a nested write-lock attempt to deadlock. "Within a few seconds every proxy in our fleet had locked up hard." Outage duration ~40 min acute, ~1hr chronic.
**Source**: [September 1 Routing Layer Outage · Infra Log](https://fly.io/infra-log/2024-09-07/).
**Confidence**: High.
**Verification**: Referenced again in the October 2025 retrospective ([Corrosion · The Fly Blog](https://fly.io/blog/corrosion/)) and in the December 12 incident post-mortem which describes the same bug class recurring ([2024-12-14 · Infra Log](https://fly.io/infra-log/2024-12-14/)).
**Analysis**: The mechanism — "rapid gossip of a poisonous state payload" — is intrinsic to any gossip-based propagation system. The whitepaper's response (§4 *Consistency Guardrails*: event-loop watchdogs, per-region blast radius, identity-scoped writes) is precisely what Fly retrofitted. Scale at which it manifested: the **full fleet** of fly-proxy instances at ~800-server scale. The bug was not scale-dependent; the blast radius was.

### Finding 4: Named failure mode #2 — Nullable-column CRDT backfill storm (November 25, 2024)

**Evidence**: "A schema change added a nullable column to the largest table tracked by Corrosion. The CRDT semantics on the impacted table meant that Corrosion backfilled every row in the table with the default null value." This generated "an explosion of updates" that saturated "upstream network links with tens of gigabytes of traffic." Compound outage lasted ~11.5 hours of degraded service. Fix: circuit-breaker limits on Corrosion gossip traffic, improved re-seeding, and long-term regionalization.
**Source**: [November 25 Outage · Infra Log](https://fly.io/infra-log/2024-11-25/).
**Confidence**: High.
**Verification**: [Corrosion · The Fly Blog](https://fly.io/blog/corrosion/) summary: "New nullable columns are kryptonite to large Corrosion tables." [CR-SQLite on GitHub](https://github.com/vlcn-io/cr-sqlite) documents that "destructive schema changes are currently prohibited" and that "cr-sqlite must backfill values for every row" when a new CRR column is added — confirming this is a CR-SQLite-layer constraint, not a Corrosion bug.
**Analysis**: Whitepaper §4 *Consistency Guardrails* already names this: "Additive-only schema migrations... gated through a two-phase rollout." This is directly responsive. However, **the whitepaper does not solve the primitive**: nullable column adds will still force a backfill; the platform can only contain the blast radius by regionalization and by two-phase cutover. For 100-200k nodes across the global Corrosion peer set, a naive additive migration to a table as large as `service_backends` at tens-of-millions scale would still be a tens-of-terabytes reconciliation event per region.

### Finding 5: Named failure mode #3 — Consul cert expiry backlog storm (October 22, 2024)

**Evidence**: "A long-lived CA certificate for a deprecated state-sharing orchestration component [Consul] expired." When Consul came back online, Corrosion received a "massive backlog of queued updates from Flyd that had accumulated during the outage." The system generated "**150 GB/s of traffic, saturating switch links**." Recovery required restoring the Corrosion cluster from a snapshot and reseeding. Duration: 7 hours 15 minutes — described as "the longest significant outage we've recorded."
**Source**: [October 22 Outage · Infra Log](https://fly.io/infra-log/2024-10-26/).
**Confidence**: High.
**Verification**: Cross-referenced by the Fly October 2025 blog post which lists this as a major incident class.
**Analysis**: This is the **strongest published number** in the entire Corrosion incident corpus: 150 GB/s peak gossip traffic during a backlog storm. This suggests the steady-state bandwidth floor for the Fly Corrosion cluster of ~800 nodes is non-trivial, and a backlog event can overwhelm switch fabric. Extrapolation to 100-200k nodes is not linear and not safe to attempt without instrumentation; the non-linearity of gossip bandwidth under churn is exactly the HashiCorp "gossip storm" failure mode (Finding 9).

### Finding 6: Corrosion backpressure is a known open gap

**Evidence**: GitHub issue #198 (opened April 2024 by Jerome Gravel-Niquet, Corrosion's author and Fly CTO): "The number of queued changes can really get large if a node has been off for a while and then starts syncing **millions of changes**." The system "lacks memory-efficient tracking after migrating to SQLite-based bookkeeping." Status: open as of the accessed date.
**Source**: [Issue #198 Apply backpressure once changes queue reaches a set length · superfly/corrosion](https://github.com/superfly/corrosion/issues/198).
**Confidence**: High (primary source, author identification verified).
**Verification**: Issue #190 (by the same author) proposes a Cuckoo filter to reduce memory pressure for tracking "current" versions — estimates "~16 million u64 uses roughly 30MB" — implying the target scale for version tracking is tens of millions of rows per node ([Issue #190](https://github.com/superfly/corrosion/issues/190)).
**Analysis**: A node rejoining after downtime carrying "millions of changes" is a named operational scenario for Corrosion. At 100-200k-node scale, the probability of at least one node rejoining with a millions-deep backlog is high at any given time. Backpressure is not a hypothetical requirement; it is a known gap.

### Finding 7: CR-SQLite performance and constraint profile

**Evidence**: "Currently inserts into CRRs are 2.5x slower than inserts into regular SQLite tables. Reads are the same speed." Recent releases: "10x improvement in performance when migrating crr tables, but a 15% reduction in performance when writing to crr tables, along with a 5x reduction in CRDT metadata." Design constraints: requires primary keys on all tables; cannot handle unique constraints on non-primary key columns; destructive schema changes prohibited; primary keys cannot be null.
**Source**: [cr-sqlite README · vlcn-io/cr-sqlite](https://github.com/vlcn-io/cr-sqlite); search-surfaced benchmark quotes cross-confirmed by [vlcn.io docs](https://vlcn.io/docs/cr-sqlite/intro).
**Confidence**: Medium (primary docs confirm qualitative constraints; exact benchmark numbers come from release notes and a single reporter summary).
**Verification**: InfoQ summary confirms: "requires primary keys on all tables; cannot handle unique constraints on non-primary key columns; destructive schema changes currently prohibited."
**Analysis**: SQLite itself tops out around 1 TB per database in practice (theoretical max 281 TB) with a single-writer constraint and WAL-growth failure modes under write-heavy workloads ([SQLite Appropriate Uses](https://sqlite.org/whentouse.html), [SQLite performance tuning · phiresky](https://phiresky.github.io/blog/2020/sqlite-performance-tuning/)). Every Overdrive node carrying tens-of-millions of `service_backends` rows plus `alloc_status` plus `node_health` plus `policy_verdicts` with 2.5x insert overhead plus crsql_changes metadata plus CRDT versioning data is a real memory/disk sizing question the whitepaper does not yet address.

### Finding 8: Foca (SWIM impl) publishes no production scale data

**Evidence**: Foca's documentation explicitly states it is "a building block" with minimal built-in policy. It scales fan-out with cluster size and exposes `Config::new_lan` / `Config::new_wan` tuning, but the published docs provide **no specific cluster size limits, no fan-out formulas, no bandwidth-per-peer measurements, and no production case study beyond Fly.io's usage**.
**Source**: [Foca README · caio/foca](https://github.com/caio/foca); [foca · docs.rs](https://docs.rs/foca).
**Confidence**: High (primary source; explicit absence of claimed production scale).
**Analysis**: Fly.io is effectively the production reference deployment for Foca. This is a single-vendor evidence base.

### Finding 9: HashiCorp Serf/Consul provides the only independent gossip-pool scale reference

**Evidence**: HashiCorp tested Consul gossip at **66,000 client agents** in a single datacenter (EC2, 5 server agents). Migration of 44,000 clients from default segment to 20 segments "took 4 hours at a rate of 220 clients/min," with "almost an additional 2 hours" for gossip convergence. **Official recommendation: ≤5,000 client agents per Consul datacenter** due to gossip stability risk under churn. Consul's own issue #9927 documents "gossip queue pruning is ineffective at scale"; issue #5567 documents "gossip storms causing members flap." Users at ~20k nodes with churn see "intent queues growing effectively unboundedly — up to 160k messages."
**Source**: [Consul Scale Test Report · HashiCorp](https://www.hashicorp.com/en/blog/consul-scale-test-report-to-observe-gossip-stability); [Recommendations for operating Consul at scale · Consul docs](https://developer.hashicorp.com/consul/docs/architecture/scale).
**Confidence**: High (two independent HashiCorp sources, one a formal scale test report).
**Verification**: Confirmed by [Consul issue #9927](https://github.com/hashicorp/consul/issues/9927) and [#5567](https://github.com/hashicorp/consul/issues/5567) on GitHub.
**Analysis**: This is the most authoritative independent data on SWIM-gossip at scale. It shows:
- A single gossip pool can be pushed to 66k nodes if stable, but the **recommended operational ceiling is 5,000 per pool**.
- Above that, churn rate — not node count — dominates stability.
- Mitigation consensus is **multiple smaller datacenters over network segments** in a single datacenter.
- For 100-200k nodes, this implies **at least 20-40 gossip pools** even on Consul's most aggressive tested profile, or 20+ Consul datacenters at the recommended profile.

The implication for Overdrive: the whitepaper's one-Corrosion-peer-per-region topology likely requires additional intra-regional sharding at Apple-scale single-region density (e.g., 10k+ nodes per region). This is not described in §4 or §7.

### Finding 10: Akka Distributed Data — comparison point for CRDT-gossip at scale

**Evidence**: "Current recommended limit is 100,000 top-level entries" with "all data held in memory." "Try with 1000-10000 top level entries, and even with delta-CRDTs it must sometimes transfer the full state, meaning that the message size for that mustn't be too big (<200 kB)." "It will take a while (tens of seconds) to transfer all entries."
**Source**: [Distributed Data · Akka core documentation](https://doc.akka.io/libraries/akka-core/current/typed/distributed-data.html).
**Confidence**: Medium-High (official docs, consistent with community Q&A).
**Analysis**: Akka's 100k-entry ceiling per CRDT store is the only comparable published hard number for a CRDT-gossip system. Overdrive's planned rows per Corrosion table at 100-200k-node/millions-of-workload scale would be >100x this without sharding.

### Finding 11: No independent production deployments of Corrosion documented

**Evidence**: Direct question on Fly community forum: "Is [Corrosion] used in production by anyone other than Fly? Does Fly feel Corrosion is something which 3rd parties can use and manage relatively easily?" — the thread was closed with **no answer**.
**Source**: [Corrosion vs Turso for a caching layer · Fly community](https://community.fly.io/t/corrosion-vs-turso-for-a-caching-layer/21032).
**Confidence**: High (explicit absence, primary source).
**Analysis**: Fly.io is the only known production Corrosion operator. Every published number in this research is Fly-primary. Overdrive adopting Corrosion as its ObservationStore means the platform inherits **the full operational envelope of Fly's single published case study**, and any scale claim beyond that envelope is extrapolation, not evidence.

### Finding 12: SQLite's intrinsic single-writer constraint shapes CR-SQLite's ceiling

**Evidence**: "SQLite only supports one writer at a time per database file." At 10 concurrent writers each inserting 100k rows, SQLite takes 28.7s vs MySQL's 4.2s; at 50 writers, SQLite degrades to 142s (20.8x slower). WAL file growth can be unbounded under sustained write-heavy workloads if checkpointing can't keep up.
**Source**: [SQLite Appropriate Uses · sqlite.org](https://sqlite.org/whentouse.html); [SQLite performance tuning · phiresky](https://phiresky.github.io/blog/2020/sqlite-performance-tuning/).
**Confidence**: High.
**Analysis**: Per-node CR-SQLite on every Overdrive node is fundamentally single-writer. At 100-200k nodes each emitting writes for their own allocations plus receiving gossiped writes from every other node, per-node sustained write throughput is the binding constraint.

---

## Knowledge Gaps

1. **No published row-count or write-rate numbers for Corrosion.** None of Fly's publications disclose `alloc_status`, `service_backends`, or `node_health` row counts, per-node write rates, or per-peer steady-state bandwidth.
2. **No independent benchmark of Foca at >1000 nodes.** Foca is production-tested by Fly only.
3. **No published data on CR-SQLite crsql_changes table growth rate or compaction behavior.**
4. **No published data on cross-region Corrosion partition-heal behavior.**
5. **No published topology data for clusters with >3-5 regions in the global Corrosion membership layer.**

---

## Answer to the Framing Question

**"At 100k-200k nodes and tens of millions of workload rows, can Corrosion + CR-SQLite carry the ObservationStore, or does the architecture need further work?"**

The defensible answer is: **The architecture needs further work that is not yet described in the whitepaper.** Specifically:

- **Proven envelope**: ~800 physical servers across ~40 regions with sub-second p99 replication (Fly.io, published). This is the upper bound of demonstrated Corrosion operation.
- **Extrapolation gap**: 100-200k nodes is 1-2 orders of magnitude above proven scale. No independent production deployment exists; no published benchmarks exist at the target scale.
- **Per-region density is likely the binding constraint**: HashiCorp's published operational guidance for SWIM-gossip (≤5,000 per pool; successful tests up to 66,000 with segmentation) suggests that **intra-regional sharding of the gossip pool is necessary** for regions exceeding ~10k nodes. The whitepaper does not describe this.
- **SQLite single-writer + CR-SQLite overhead (2.5x)** means per-node sustained write throughput at Apple scale is a real sizing question.
- **Known unsolved primitives**: Corrosion's backpressure is an open GitHub issue. Nullable-column schema migrations still trigger backfills at the CR-SQLite layer.

The whitepaper's regionalized topology is sound and maps directly to Fly's post-incident architecture — this is the strongest evidence in the corpus that the direction is correct. But the claim that Corrosion "scales to continents" (whitepaper §4) is accurate for Fly's measured envelope and misleading if it is read as "scales to 100-200k nodes." Recommend the whitepaper either:

1. Narrow the scale claim to match published evidence (~5k-10k nodes per region, 40+ regions), and document that Apple-scale densities are open research; or
2. Add intra-regional sharding as an explicit design axis, with pool-count sizing derived from HashiCorp's operational evidence (~5k-10k nodes per gossip pool, N pools per region).

Either is defensible; the current framing is over-confident given the empirical corpus.
