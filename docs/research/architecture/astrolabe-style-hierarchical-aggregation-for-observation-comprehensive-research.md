# Research: Astrolabe-Style Hierarchical Aggregation as the Observation Layer for a Flat-Cluster Orchestrator

**Date**: 2026-09-25 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium-High overall (High for what Astrolabe guarantees and for the production precedents; Low-Medium for the bandwidth estimates, which rest on assumed write rates) | **Sources**: 21 cited (plus 4 prior repo documents)

**Scope**: Whether Astrolabe-style hierarchical aggregation (zones, aggregation functions, representatives) should be the observation layer of the target design, and if so for which consumers and at what scale. This document researches a *target* design; it does not evaluate the current codebase, and it does not treat `docs/whitepaper.md` as authoritative.

**Relation to prior repo research** (built on, not repeated):
- `docs/research/architecture/flat-cluster-placement-and-observation-comprehensive-research.md` — C5 (Corrosion 2026 state, Scuttlebutt, chitchat, CEP-21 trend, epoch-fenced single-writer facts); C5.7 surfaced Astrolabe as the top unexplored alternative.
- `docs/research/architecture/flat-cluster-membership-discovery-and-admission-comprehensive-research.md` — discovery, signed node records, admission.
- `docs/research/architecture/flat-cluster-consensus-and-log-dissemination-comprehensive-research.md` — voters + learners, relay-tree dissemination of the log.
- `docs/research/scalability/corrosion-crsqlite-production-scale-evidence.md` — Fly's ~800 servers / ~40 regions envelope and regionalization (F1–F2).

## Executive Summary

**What Astrolabe is.** Astrolabe (ACM TOCS 2003) arranges nodes in an operator-named zone tree.
- Each node gossips full detail only with its leaf-zone siblings (typically 32–64 hosts).
- For every higher level it holds one bounded summary row per sibling zone. Summaries are computed by signed SQL aggregation functions and limited to about 1 KB per zone.
- Per-node state and bandwidth therefore grow with branching factor × depth, not with N. That is the property the target design lacks under full replication.
- The guarantees are weak, and the authors say so. An update is eventually reflected with probability 1. Two readers at the same instant may see different values, snapshots are not ordered, a partitioned zone's summary goes stale and then vanishes, and a representative can publish a false aggregate for its zone.
- Measured dissemination is ~20–35 s on a 63-host testbed. The deployment evidence is ~60 machines.

**Successors and production.** The *mechanism* — self-electing gossip representatives computing runtime-installed aggregates — has no production descendant that this research could find. SDIMS and Moara improved on it in research prototypes. The *shape*, however, is proven at scale: detail stays in the region that produced it, while summaries and queries cross regions.
- Google Monarch uses autonomous regional zones, push-down queries, and explicit partial results with "pruned zone" notices.
- Borgmon and Ganglia run fixed aggregator trees.
- Fly.io's regionalization keeps per-region Corrosion detail plus a global app→region map.
- NATS super-clusters take a different route: they cut cross-region traffic by *interest*, not by summarising.

**Consumers.** No single delivery model serves every consumer:
- The dataplane needs exact backend rows for the services it routes to. That calls for interest-scoped delta subscription (xDS semantics); aggregates cannot help.
- The scheduler can pre-filter regions and zones on summaries, but must commit against exact, logged state, as Borg does with a cached cell state that the master re-validates.
- The operator needs aggregates for cluster-wide views and on-demand scatter-gather to drill down.
- Failure detection may *use* aggregates as evidence, for example to throttle replacement during a regional reachability collapse. It must never *decide* from them.

**Verdict.** Do not adopt Astrolabe as the observation layer. Adopt its shape as a summary tier over a regional, single-writer fact layer:
- Start with two levels: regional full replication plus a small global tier of projections and summaries.
- Serve the dataplane by interest-scoped subscription, and operators by on-demand queries.
- Derive the zone tree from committed node labels.
- Compile in a fixed set of aggregation functions, and stamp every summary with completeness, age and label epoch.

Estimates in Q7 suggest a third, intra-region level pays off only when *a single region* outgrows one gossip pool (low thousands of nodes). Region count is not the trigger: 50–100 region summaries fit in about 100 KB.

## Research Methodology

**Search Strategy**:
- Read the four prior repo documents first and built on C5.7 (where Astrolabe was surfaced) without re-deriving their findings.
- Read the Astrolabe TOCS paper page by page from the author copy (the web extractor could not decode the PDFs, so they were read as page images). SDIMS, Monarch, Borg and the Astrolabe companion paper were read the same way.
- Used official documentation for NATS, Envoy xDS, Kubernetes, Service Fabric and Consul, and web search for lineage (Moara, Ganglia) and for implementations.
**Source Selection**: Types: academic (ACM TOCS, SIGCOMM, EuroSys, Middleware, PVLDB), official docs, technical docs | Reputation: medium-high minimum for load-bearing claims | Verification: cross-reference across independent organisations (Cornell, UT Austin, Google, Berkeley, Fly, Envoy, Kubernetes, Microsoft).
**Quality Standards**: Target 3 sources/claim (min 1 authoritative) | Major claims cross-referenced | Avg reputation ≈0.97 (see Source Analysis).
**Out-of-list primaries** (per the brief): cs.cornell.edu (Astrolabe, companion paper), cs.utexas.edu (SDIMS), vldb.org (Monarch), research.google (Borg), sre.google (Borgmon), docs.nats.io (NATS), sciencedirect.com (Ganglia). They are scored high where they are copies of ACM/VLDB papers and medium-high otherwise.
**Adversarial validation**: all fetched content was treated as untrusted. Several fetches returned tool summaries instead of text (Fly blog, Cilium, Kubernetes API concepts); only phrases the tool marked as quotes were used, and gaps are recorded. No prompt-injection content was detected.

## Target design under examination (restated)

- **Topology**: flat, with no single/HA mode split. The cluster grows by joining a node, from one node to thousands, possibly across regions. Components are enabled per node, over a WireGuard mesh with gossip discovery and signed, CA-backed node records.
- **Decisions**: a consensus log (leading candidate Viewstamped Replication). A small voter set orders the log; every node is a learner holding the full log.
- **Observation**: high-volume, per-node-owned facts (heartbeats, allocation status, health, service backends, utilisation). It must stay available under partition and stays off the log. Facts are single-writer; ownership transfer is committed to the log as a fencing epoch. Anything a safety decision rests on goes through the log (the prior research's correctness-impact amendment).
- **Problem**: Corrosion and per-origin Scuttlebutt logs both replicate every fact to every node, which does not scale to thousands of nodes across regions.

## Q1. Astrolabe itself

All quotes in this section are from the author-hosted copy of the TOCS paper, read page by page: van Renesse, Birman, Vogels, "Astrolabe: A Robust and Scalable Technology for Distributed System Monitoring, Management, and Data Mining", *ACM Transactions on Computer Systems* 21(2):164–206, May 2003 ([ACM DL](https://dl.acm.org/doi/10.1145/762483.762485); author copy [cs.cornell.edu PDF](https://www.cs.cornell.edu/home/rvr/papers/astrolabe.pdf), out-of-list primary, peer-reviewed so scored high). Page numbers refer to the author copy. Accessed 2026-09-25.

### Finding Q1.1: The zone tree is administrator-named; each node holds only its ancestors' sibling tables, so per-node state is O(branching factor × depth), not O(N)

**Evidence**: "A zone is recursively defined to be either a host or a set of non-overlapping zones … The leaves of this tree represent the hosts" (§2, p. 4). "The zone hierarchy is implicitly specified when the system administrator initializes these agents with their names … Thus the zone hierarchy is formed in a decentralized manner, but one ultimately determined by system administrators" (§2, p. 5). "Conceptually, each zone periodically chooses another sibling zone at random, and the two exchange the MIBs of all their sibling zones. After this exchange, each adopts the most recent MIBs according to the issued timestamps" (§4.1, p. 15). "At 390,625 members and branching factor 5, there are 8 levels in the tree. Thus each member only has to maintain and gossip only 8 × 5 = 40 MIBs … With a branching factor of 25, each member maintains and gossips 4 × 25 = 100 MIBs. In the limit (flat gossip), each member would maintain and gossip an impractical 390,625 MIBs" (§7.1, p. 30). "Any given user sees only its parent zones, and those of its children" (§3.1, p. 9).
**Source**: Astrolabe TOCS 2003 (above), §2, §4.1, §7.1.
**Confidence**: High (peer-reviewed primary; the O(bf·depth) state bound is also restated by SDIMS, Q2.1)
**Verification**: SDIMS (Yalagandula & Dahlin, SIGCOMM 2004) describes Astrolabe the same way — see Q2.1. The prior repo finding C5.7 quoted the same zone definition.
**Analysis**: This is the property that motivates the whole investigation: a node does not hold every other node's facts. It holds (a) its own leaf MIB, (b) the full MIBs of its leaf-zone siblings (typically 32–64 hosts, Q1.6), and (c) one aggregate row per sibling zone at each higher level. The zone tree is *named by operators*, not discovered — Astrolabe is not self-organizing in the topological sense; only representatives are self-selected (Q1.3).

### Finding Q1.2: Attributes of inner zones are not writable; they are the output of signed SQL aggregation programs (AFCs) whose outputs must be bounded in size

**Evidence**: "Unlike SNMP, the Astrolabe attributes are not directly writable, but generated by so-called aggregation functions … An aggregation function for a zone is an SQL program, which takes a list of the MIBs of the zone's child zones and produces a summary of their attributes" (§2, p. 5). "The code of these functions is embedded in so-called aggregation function certificates (AFCs), which are signed and time-stamped certificates that are installed as attributes inside MIBs" (p. 6). "Aggregation is intended as a summarizing mechanism. For example, an aggregate could count the number of nodes satisfying some property, but not to concatenate their names into a list, since that list could be of unbounded size … The number of aggregating queries active within any given scope is assumed to be reasonably small, and independent of system size … nodes seeking to introduce a new aggregating function must have administrative rights within the scope where it will be computed" (§1, p. 3). The built-in functions are MIN, MAX, SUM, AVG (weighted, "usually set to nmembers"), OR, AND (bitmaps), FIRST(n, attr), RANDOM(n, attr) (Table 2, p. 26). "Astrolabe is designed under the assumption that MIBs will be relatively small objects – a few hundred or even thousand bytes, not millions" (p. 6). Conclusion: "the amount of information maintained by the attributes of a Astrolabe zone cannot grow beyond about a kilobyte" (§9, p. 40). An AFC "can specify an expiration time; unless the time is periodically advanced … the aggregation function will eventually expire" (§6.2, p. 27).
**Source**: Astrolabe TOCS 2003, §1, §2, §6, §9.
**Confidence**: High (primary)
**Verification**: SDIMS (Q2.1) and Moara (Q2.2) both identify the fixed-aggregation-function/limited-query-count constraint as the Astrolabe limitation they set out to relax.
**Analysis**: The ~1 KB-per-zone-MIB budget and "reasonably small" number of AFCs are the load-bearing constraints for the target design. An orchestrator cannot put "the list of backends for service S" into an inner-zone aggregate — that is exactly the "concatenate their names into a list" case the paper excludes. Aggregates are for counts, sums, extremes, bitmaps (Bloom-filter-style membership, p. 12), and "first n" samples.

### Finding Q1.3: Representatives ("contacts") are chosen by an aggregation function; the default picks the first three, and one representative per zone is fragile

**Evidence**: The MIB of every zone carries `id`, `rep` ("the representative agent for the zone—the agent that generated the MIB of the zone"), `issued`, `contacts` ("a small set of addresses for representative agents of this zone, used for the peer-to-peer protocol"), `servers`, and `nmembers` (§4.1, p. 16). "The contacts attribute is dynamically computed based on an aggregation function … each zone elects the set of agents that gossip on behalf of that zone. The election can be arbitrary, or based on characteristics like load or longevity of the agents" (p. 16). The default AFC: `SELECT SUM(nmembers) AS nmembers, MAX(depth) + 1 AS depth, FIRST(3, contacts) AS contacts, FIRST(3, servers) AS servers` (§6.1, p. 26). "The maximum number of zones an agent can represent is bounded by the number of levels in the Astrolabe tree" (p. 16). "With only one representative per zone, Astrolabe is highly sensitive to host crashes. The protocols still work, as faulty representatives are detected and replaced automatically, but this detection and replacement takes time and leads to significant delays in dissemination. Astrolabe is preferably configured with more than one representative in each non-leaf zone … three times as many representatives also leads to three times as much load on the routers" (§7.1, p. 30).
**Source**: Astrolabe TOCS 2003, §4.1, §6.1, §7.1.
**Confidence**: High (primary)
**Verification**: Figure 7 (p. 30) quantifies it: at branching factor 25, moving from 1 to 3 representatives lowers expected rounds to reach ~390k members from ≈29 to ≈24.
**Analysis**: "Election" here is not consensus: it is a deterministic function over gossiped child MIBs, so two agents with different gossip views can briefly disagree on who the representatives are. That is harmless for dissemination (a redundant gossiper) and is why Astrolabe needs no agreement protocol. It also means representative identity is *not* a safe basis for anything exclusive (a leader, a lock holder).

### Finding Q1.4: The consistency guarantee is probabilistic eventual consistency of aggregates; snapshots are not atomic, not identical across readers, and not ordered — monotonicity is available only at the cost of fewer updates

**Evidence**: "When retrieving an aggregate value, does it incorporate all the latest changes to the distributed state? When two users retrieve the same attribute at the same time, do they obtain the same result? Do snapshots reflect a single instance in time? When looking at snapshot 1, and then later at snapshot 2, is it guaranteed that snapshot 2 was taken after snapshot 1? The answer to all these questions is no." "Given an aggregate attribute X that depends on some other attribute Y, Astrolabe guarantees with probability 1 that when an update u is made to Y, either u itself, or an update to Y made after u, is eventually reflected in X. We call this eventual consistency, because if updates cease, replicated aggregate results will eventually be the same." "Aggregate attributes are updated frequently, but their progress may not be monotonic" (§4.3, p. 18). With (min, max) issued-interval tracking, "An update with such an interval is not accepted unless the minimum issued time of the new MIB is at least as large as the maximum issued time of the current one … an update will only be generated after a completely new set of input attributes has had a chance to propagate … this propagation may take many rounds of gossip (5 — 35 rounds for practical Mariner hierarchies)" (§4.3, p. 19). Also: "The Astrolabe system looks to a user much like a database, although it is a virtual database that does not reside on a centralized server, and does not support atomic transactions" (§1, p. 3). Clocks: "as we scaled up the system we could not rely on clocks being synchronized … we now only compare the issued timestamps of the same agent, identified by rep" (§4.1, p. 17).
**Source**: Astrolabe TOCS 2003, §1, §4.1, §4.3.
**Confidence**: High (primary; the authors state the negative guarantees explicitly)
**Verification**: Monarch's "availability over consistency" stance (Q2.5) and SDIMS's "eventual" semantics (Q2.1) are the same class of guarantee in successor systems.
**Analysis**: An Astrolabe aggregate is an *estimate with no staleness bound*: no "as-of" time, no completeness indicator beyond `nmembers`. Two nodes may read different values at the same moment, and a later read may reflect older inputs unless the monotonic mode is used. That rules aggregates out for any decision whose correctness depends on the value (Q4).

### Finding Q1.5: Convergence grows O(log n) but hierarchy is roughly 2× slower than flat gossip; measured end-to-end latency on a 63-host testbed was ~20–35 s

**Evidence**: "gossip within a zone spreads quickly, with dissemination time growing O(log n), where n is the number of child zones of the zone … Gossip is going on continuously, its rate being independent of the rate of updates" (§4.1, p. 17). Figure 6 (simulation to 5⁸ = 390,625 members): flat gossip needs ≈15 rounds; hierarchical needs ≈32 (bf 5), ≈29 (bf 25), ≈26 (bf 125) rounds (p. 29). "Hierarchical gossip also scales well, but is significantly slower than flat gossip. Latency improves when the branching factor is increased, but doing so also increases overhead" (p. 30). "Typically, Astrolabe agents are configured to gossip once every two to five seconds" (p. 29). "If we configure Astrolabe so that agents gossip once every two to five seconds, as we normally do, we can see that updates propagate with latencies on the order of tens of seconds" (p. 31). Emulab validation: 48–126 agents on 63 hosts, gossip every 5 s, three representatives; mean dissemination latencies ≈22–35 s across the eight hierarchies of Table 3, with 95% intervals reaching ≈47 s (Figure 12, pp. 34–35). Loss up to 15% and up to 8% crashed hosts raise rounds only modestly (Figures 8–9, pp. 31–32).
**Source**: Astrolabe TOCS 2003, §4.1, §7.1, §7.3.
**Confidence**: High for the reported numbers; Medium for extrapolation (the scale results are simulation, synchronous rounds, one update source; see Q1.8)
**Verification**: Figures 6–9 and 12 are internally consistent (simulation vs Emulab). No independent re-measurement of Astrolabe was found (Knowledge Gaps).
**Analysis (interpretation)**: At a 1 s gossip interval, a 3-level tree for ~10⁴ nodes would converge in roughly 15–25 s by the paper's curves; at the paper's 2–5 s intervals it is ~0.5–2 minutes. That is adequate for capacity hints and dashboards, and far too slow for failover or backend removal. Flat gossip over the same population is faster but carries O(N) state per node — the trade the target design is trying to escape.

### Finding Q1.6: Per-node bandwidth is bounded by the branching factor, not N — ~200–300 bytes per compressed MIB, one message per round per represented zone, recommended branching factor 5–50

**Evidence**: "Astrolabe requires about 200 to 300 bytes per (compressed) MIB in a gossip message … UDP messages are typically limited to approximately 8 Kbytes, which can therefore just contain about 25 - 40 MIBs" (§4.5, p. 22). "on average, each agent receives one message per round for each zone it represents. Thus, if there are k levels, an agent that represents zones on each level with have the worst average load of k messages per second. Obviously, this load grows O(log n)" (§7.2, p. 31). Figure 10: maximum messages per round at ~390k members ≈56 (bf 5), ≈32 (bf 25), ≈21 (bf 125) (p. 33). "the number of child zones should be approximately between 5 to 50" (§7.1, p. 28). "Since zones typically contain 32 to 64 members, the vast majority of messages are exchanged between sibling leaf nodes" (§4.2, p. 18). Signature checking: naive checking grows "O(log² n)"; buffering and checking only the newest MIB per zone limits it to "at most k × (bf − 1)" checks per round (pp. 32–33). Contrast (§8.1, p. 37): Clearinghouse's "storage grows O(n), while the total bandwidth taken up by gossip grows O(n²)"; "Flat gossip would be impractical in a real system, as the required memory grows linearly, and network load quadratically with the membership" (p. 29).
**Source**: Astrolabe TOCS 2003, §4.2, §4.5, §7.1, §7.2, §8.1.
**Confidence**: High (primary; numbers are the authors' own)
**Verification**: The flat-gossip O(N)-message-size result is independently stated for Scuttlebutt in the prior repo finding C5.2 ("the size of gossip messages grows linearly in N"), by the same first author in 2008.
**Analysis (estimate, labelled)**: With bf = 32 and three levels (32³ ≈ 32k nodes), a node gossiping one ~8 KB message per level per 2 s round sends ≈12 KB/s and holds ≈96 MIBs ≈ 25 KB of Astrolabe state. The estimate uses only the paper's figures; it excludes any per-allocation data, which would not fit Astrolabe's MIB budget anyway.

### Finding Q1.7: Security is integrity-only, via a per-zone CA hierarchy — no confidentiality, and no protection against a keyholder lying in an aggregate

**Evidence**: "Security through certificates: Astrolabe uses digital signatures to identify and reject potentially corrupted data … The zone tree itself forms the Public Key Infrastructure" (§1, p. 4). "Security in Astrolabe is currently only concerned with integrity and write access control, not confidentiality" (§5, p. 22). "Each zone in Astrolabe has a corresponding Certification Authority (CA)"; four certificate types: zone certificate (signed by the parent CA), MIB certificate ("signed by the private zone key … prevents the introduction of 'false gossip'"), AFC (installed only if "issued directly by one of its ancestor zones … or by one of their clients"), and client certificate (§5.1, p. 23). "zone certificates are not chained … Chaining would imply transitive trust" (p. 24). Hazards acknowledged: "the more hosts that have the private key for a zone, the more fault-tolerant the system, but also the more likely the key will get compromised" (p. 24); certificates "cannot be easily revoked until they expire" (p. 25); "they do not prevent authorized agents and clients from lying about their attributes … an agent that holds a private key of a zone can gossip a value different from the computed minimum of its child zones' loads" (p. 25); "Astrolabe is not secured against faulty aggregation functions" (fn. 12, p. 25). Zones may choose public-key, shared-key, or no crypto; "experience with the real system suggests that the shared key cryptography option often represents the best trade-off between security and overhead" (p. 22).
**Source**: Astrolabe TOCS 2003, §1, §5, fn. 12.
**Confidence**: High (primary)
**Verification**: The same "representative can misreport an aggregate" hazard is the reason later work examined verifiable aggregation; not further researched here (Knowledge Gaps).
**Analysis**: This maps well onto the target's CA-backed node records and WireGuard mesh (transport confidentiality is supplied by WireGuard, not by the aggregation layer). The unresolved hole is the one that matters for an orchestrator: any node that is a representative for zone Z can publish a false aggregate for Z, and every node outside Z has only the aggregate. Leaf facts are signed by their origin; aggregates are signed by *whoever computed them*.

### Finding Q1.8: Failure behaviour — failed representatives and empty zones age out after T_fail; partitions produce independent trees that re-glue by multicast/"relatives"; the published evidence is simulation plus a ~60-machine deployment

**Evidence**: "When an agent has not seen an update for a zone from a particular representative agent for that zone for some time T_fail, it removes its corresponding MIB. When the last MIB of a zone is removed, the zone itself is removed from the agent's list of zones. This algorithm will always detect and remove failed participants and empty zones within T_fail seconds. T_fail should grow logarithmically with membership size" (§4.2, pp. 17–18). "Either because of true network partitions, or because of setting T_fail too aggressively, it is possible that the Astrolabe tree splits up into two or more independent pieces … Astrolabe relies on IP multicast to set up the initial contact between trees … at a fixed rate ρ which is typically on the order of ten seconds"; agents can also be configured with "relatives" contacted "using point-to-point messages" (p. 18). Churn: "Many peer-to-peer systems suffer degraded performance when network partitioning or a high rate of churn occurs. We are currently focused on using Astrolabe in comparatively stable settings" (fn. 10, p. 16). Deployment: "Although Astrolabe is currently deployed on approximately 60 machines, this is not a sufficiently large system to evaluate its scalability" (fn. 13, p. 28); "Neither has Astrolabe at this time [been scaled to more than a few hundred servers], but analysis and simulation indicate that Astrolabe could potentially scale to millions" (§8.1, p. 37). Also: "the hierarchical epidemics used in Astrolabe cannot be used as is in [sensor] networks, as the protocol does send messages across large distances, albeit occasionally" (§8.4, p. 38).
**Source**: Astrolabe TOCS 2003, §4.2, §7.1, §8.
**Confidence**: High (primary; the limitations are self-reported)
**Verification**: No later independent deployment report was found; see Q2.8 and Knowledge Gaps.
**Analysis**: Under partition each side keeps a self-consistent *local* view and a *stale* remote aggregate until T_fail expires, then drops the remote zone entirely — an aggregate silently "disappears" rather than being marked stale. For an orchestrator, a region's capacity summary vanishing from the root after a WAN partition is the right signal only if consumers know "absent" means "unknown", not "zero". The paper's largest evidence base is ~60 machines in deployment and 126 agents on a testbed.

## Q2. Lineage, successors, and production analogues

### Finding Q2.1: SDIMS (SIGCOMM 2004) identified Astrolabe's replicate-every-aggregate-to-every-node strategy as the limit on attribute count, and replaced it with per-attribute DHT trees and a tunable read/write propagation API

**Evidence**: "Astrolabe provides the abstraction of a single logical aggregation tree that mirrors a system's administrative hierarchy … its strategy of replicating all aggregated attribute values for a subtree to all nodes in the subtree … allows any node to answer any query using local information. This high degree of replication, however, may limit the system's ability to accommodate large numbers of attributes. Also, although the approach works well for read-dominated attributes, an update at one node can eventually affect the state at all nodes" (§1). SDIMS maps each attribute key to a separate tree: "by aggregating an attribute along the aggregation tree corresponding to DHTtree_k for k = hash(attribute type, attribute name), different attributes will be aggregated along different trees" (§4.1). Propagation is chosen per attribute: Update-Local, Update-Up, Update-All and "Update-Upk-Downj" (§3); "we observe Update-Local to be efficient for read-to-write ratios below 0.0001, Update-Up around 1, and Update-All above 50000" (§7). Administrative isolation: "queries about an administrative domain's information can be satisfied within the domain so that the system can operate during disconnections from other domains" (§1), via an "Autonomous DHT" with path locality and path convergence (§4.2). Consistency: "it is difficult or impossible to insist that the aggregation value returned by a probe corresponds to the function computed over the current values at the leaves at the instant of the probe. Therefore our system provides only weak consistency guarantees – specifically eventual consistency as defined in [Astrolabe]" (§2). "During reconfigurations, a probe might return a stale value" (§6); evaluation: 4,096-node simulations; testbed "283 SDIMS nodes" on 180 machines plus "69 machines of the PlanetLab testbed"; "the maximum node stress in our system is an order lower than observed with an Update-All, gossiping approach" (§7).
**Source**: [Yalagandula, Dahlin, "A Scalable Distributed Information Management System", SIGCOMM 2004 (ACM DL)](https://dl.acm.org/doi/10.1145/1030194.1015509); author copy [cs.utexas.edu PDF](https://www.cs.utexas.edu/~dahlin/projects/sdims/papers/sdims-sigcomm.pdf) (out-of-list primary of an ACM paper, scored high) — accessed 2026-09-25.
**Confidence**: High (peer-reviewed; describes Astrolabe consistently with Q1)
**Verification**: Astrolabe's own conclusion (Q1.2) names the same ~1 KB-per-zone and "number of concurrently installed aggregations" limits as future work. Moara (Q2.2) cites SDIMS and Astrolabe for the same single-global-tree limitation.
**Analysis**: SDIMS is the most useful successor for the target because its "read-to-write ratio chooses propagation" result maps directly onto observation consumers: capacity summaries are read often and change moderately (Update-Up or Update-All), whereas per-allocation detail is written often and read rarely (Update-Local — i.e. do not propagate; probe on demand). Its administrative isolation — a domain answers its own queries while disconnected — is the region-autonomy property the target needs.

### Finding Q2.2: Moara (Middleware 2008) adds group-scoped aggregation trees because operators query *groups* of machines, not the whole system

**Evidence**: "Users and administrators of large-scale infrastructures (e.g., datacenters and PlanetLab) frequently need to monitor groups of machines, but existing distributed querying systems are not group-based and mostly focus on querying the entire system." Contributions: "First, it builds aggregation trees for different groups and adaptively maintains the trees to optimize total message cost. Second, it supports a query language allowing groups to be specified implicitly via predicates consisting of arbitrarily nested unions and intersections."
**Source**: [Ko, Yalagandula, Gupta, Talwar, Milojicic, Iyer, "Moara: Flexible and Scalable Group-Based Querying System", Middleware 2008 (ACM DL)](https://dl.acm.org/doi/10.5555/1496950.1496975); [Springer chapter](https://link.springer.com/chapter/10.1007/978-3-540-89856-6_21) — abstract via search index; full text not fetched (author PDF returned 404). Accessed 2026-09-25.
**Confidence**: Medium (peer-reviewed venue, abstract only)
**Verification**: The same group-scoping motivation appears in SDIMS's domain-scoped queries (Q2.1) and in Monarch's field-hints pruning (Q2.3); independent groups.
**Analysis**: Moara's point generalises: an orchestrator's queries are almost always scoped — "allocations of job J", "nodes with label GPU=true in region R" — which favours scoped fan-out or per-group trees over one global aggregate tree.

### Finding Q2.3: Monarch (VLDB 2020) is the strongest production evidence for zone-local ownership plus global federation — with explicit partial results, zone pruning, and consistency traded for availability

**Evidence**: "Monarch readily trades consistency for high availability and partition tolerance … Monarch must serve the most recent data in a timely fashion; for that, Monarch drops delayed writes and returns partial data for queries if necessary. In the face of network partitions, Monarch continues to support its users' monitoring and alerting needs, with mechanisms to indicate the underlying data may be incomplete or inconsistent" (§2). "The primary organizing principle of Monarch … is local monitoring in regional zones combined with global management and querying … Each Monarch zone is autonomous, and consists of a collection of clusters, i.e., independent failure domains, that are in a strongly network-connected region" (§2). "Monarch cannot use them [Bigtable, Colossus, Spanner, Blobstore, F1] on the alerting path to avoid a potentially dangerous circular dependency" (§2). Query tree: "global queries are evaluated in a tree hierarchy of three levels. A root mixer receives the query and fans out to zone mixers, each of which fans out to leaves in that zone" (§5.2). Pushdown: "In practice, this allows up to 95% of standing queries to be fully evaluated at zone level by zone evaluators, greatly increasing tolerance to network partition" (§5.3). Field hints index: "FHIs reduce query fanout by around 99.5% at zone level and by 80% at root level"; "Missing updates to the root FHI are thus reliable indicators of zone unavailability" (§5.4). Zone pruning: "almost all (99.998%) successful global queries start to stream results from zones within the first half of their deadlines … A zone is pruned if it is completely unresponsive by the soft query deadline … Users are notified of pruned zones as part of the query results" (§5.5). Collection aggregation: deltas bucketed with an admission window; "rejected writes comprise only a negligible fraction of traffic"; finalization takes "T_B + T_W … normally delayed by up to around 70 seconds" (§4.3). Scale: "close to a petabyte of compressed time series data in memory, ingests terabytes of data per second, and serves millions of queries per second" (§1).
**Source**: [Adams et al., "Monarch: Google's Planet-Scale In-Memory Time Series Database", PVLDB 13(12):3181–3194, 2020](https://www.vldb.org/pvldb/vol13/p3181-adams.pdf) (out-of-list primary; VLDB-published, scored high) — accessed 2026-09-25.
**Confidence**: High (peer-reviewed, production system, first-party)
**Verification**: [Google SRE book, ch. 10 "Practical Alerting from Time-Series Data"](https://sre.google/sre-book/practical-alerting/) describes the predecessor Borgmon's hierarchical per-datacenter-then-global aggregation (Q2.4); Monarch §1 lists Borgmon's manual sharding and query-tree setup as limitations it removed.
**Analysis**: Monarch is not Astrolabe (no gossip, no SQL-in-certificates, no every-node replication), but it validates the *shape* that matters: (1) detail stays in the zone where it is produced; (2) cross-zone views are computed by pushing work down and shipping only results; (3) the answer carries an explicit completeness signal ("pruned zones"), and missing index updates are *treated as* a zone-unavailability signal; (4) the monitoring system refuses to depend on the strongly consistent systems it monitors. Point (3) is exactly what Astrolabe lacks (Q1.4, Q1.8).

### Finding Q2.4: NATS super-clusters are a production instance of zone-scoped dissemination by *interest*, not by aggregation

**Evidence**: "A super-cluster … joins two independent clusters into one logical system without combining them into a single mesh." "A route ties two servers together and assumes they're close; a gateway ties two clusters together and assumes they're far apart." "Each server opens just one gateway connection to each other cluster, never one to every remote server." "A gateway doesn't blindly forward every message to the other side. It carries a message across only when the remote cluster has a subscriber interested in that subject." "If nobody in `west` subscribes to `orders.created`, not one of those messages crosses the gateway." "NATS prefers a local queue subscriber first."
**Source**: [NATS docs — Gateways / Super-clusters](https://docs.nats.io/running-a-nats-service/configuration/gateways) (out-of-list primary for NATS's own behaviour; nats.io is on the trusted list, docs.nats.io is its documentation host) — accessed 2026-09-25.
**Confidence**: Medium-High (official docs; single source for the mechanism)
**Verification**: The prior repo research C5.6 used docs.nats.io for JetStream behaviour; NATS subject-interest routing is also the basis of leaf nodes (not separately fetched — Knowledge Gaps).
**Analysis**: NATS demonstrates the *second* way to stop every node seeing every fact: do not summarise, *filter by declared interest* at the region boundary. It carries exact data, only to where a subscriber exists. For the dataplane (backends of services this node routes to) this is the right shape; for "how much free capacity does region B have" it is not (no summarisation).

### Finding Q2.5: Borgmon and Ganglia are the long-running production precedents for *hierarchical monitoring aggregation* — both use a fixed tree of designated aggregators, not gossip, and both filter what goes up

**Evidence**: Borgmon: "Typically, a team runs a single Borgmon per cluster, and a pair at the global level." "A Borgmon can collect from other Borgmon, so we can build hierarchies that follow the topology of the service, aggregating and summarizing information and discarding some strategically at each level." "Upper-tier Borgmon can filter the data they want to stream from the lower-tier Borgmon, so that the global Borgmon does not fill its arena with all the per-task time-series from the lower tiers." "a streaming protocol is used to transmit time-series data between Borgmon." Monarch later cited as Borgmon limitations that it "requires users to manually shard the large number of monitored entities of global services across multiple Borgmon instances and set up a query evaluation tree" (Monarch §1). Ganglia: "based on a hierarchical design targeted at federations of clusters. It relies on a multicast-based listen/announce protocol to monitor state within clusters and uses a tree of point-to-point connections amongst representative cluster nodes to federate clusters and aggregate their state"; "currently in use on over 500 clusters around the world."
**Source**: [Google SRE book, ch. 10 "Practical Alerting from Time-Series Data"](https://sre.google/sre-book/practical-alerting/) (out-of-list primary, first-party Google publication, medium-high); [Monarch PVLDB 2020 §1](https://www.vldb.org/pvldb/vol13/p3181-adams.pdf); [Massie, Chun, Culler, "The Ganglia distributed monitoring system: design, implementation, and experience", *Parallel Computing* 30(7), 2004 (ScienceDirect abstract)](https://www.sciencedirect.com/science/article/abs/pii/S0167819104000535), author copy [theether.org](http://www.theether.org/papers/ganglia-twocol.pdf) (abstract text via search index; peer-reviewed journal, scored medium-high) — accessed 2026-09-25.
**Confidence**: High (three independent organisations — Google SRE, Google Monarch team, UC Berkeley — describe hierarchical monitoring in production)
**Verification**: Ganglia's "flat within cluster, tree across clusters" is structurally the same as Fly's regionalization (Q2.6) and Astrolabe's leaf-zone gossip + representative tree (Q1.1).
**Analysis**: Hierarchical *aggregation* is well proven in production — but as a fixed, configured tree of aggregators with within-cluster full sharing, not as Astrolabe's self-electing gossip hierarchy. The operational complaint that led Google from Borgmon to Monarch was *manual* sharding and tree setup, which argues for deriving the tree automatically (Q5).

### Finding Q2.6: Fly.io's regionalization is a two-level *full-replication* scheme: fine-grained per region, a thin app→region map globally

**Evidence**: "we took on a project we call 'regionalization', which creates a two-level database scheme. Each region we operate in runs a Corrosion cluster with fine-grained data about every Fly Machine in the region. The global cluster then maps applications to regions, which is sufficient to make forwarding decisions at our edge proxies." "Regionalization reduces the blast radius of state bugs. Most things we track don't have to matter outside their region." "To allow an HTTP request in Tokyo to find the nearest instance in Sydney, we really do need some kind of global map of every app we host." "We moved away from distributed consensus … Consensus protocols like Raft break down over long distances."
**Source**: [Fly.io, "Corrosion" (blog, October 2025)](https://fly.io/blog/corrosion/) (fly.io on trusted list; tool returned partly summarised text, quotes above were returned as verbatim) — accessed 2026-09-25.
**Confidence**: Medium-High (first-party; prior repo research F2 cross-checked it against three Fly infra-log post-mortems and an InfoQ summary)
**Verification**: `docs/research/scalability/corrosion-crsqlite-production-scale-evidence.md` F2 (2024-09-07, 2024-11-25, 2024-12-14 infra-log posts; InfoQ April 2025).
**Analysis**: Fly's global tier is not an aggregate in Astrolabe's sense (a count or sum); it is a *projection* — a small, exact, low-churn index (`app → regions`) that answers the one cross-region question the edge proxy has. Everything else stays regional. This is a hand-picked instance of "hierarchical aggregation with one aggregation function", which is the cheapest useful point on the spectrum.

### Finding Q2.7: Service Fabric's federation layer is a membership/ownership ring, not an aggregation hierarchy; its health data is aggregated into a *centralized* health store

**Evidence**: "The federation subsystem is built on top of distributed hash tables with a 128-bit token space. The subsystem creates a ring topology over the nodes … For failure detection, the layer uses a leasing mechanism based on heart beating and arbitration. The federation subsystem also guarantees through intricate join and departure protocols that only a single owner of a token exists at any time." Health Manager: "Cluster entities (such as nodes, service partitions, and replicas) can report health information, which is then aggregated into the centralized health store … The health query APIs return the raw health data stored in the health store or the aggregated, interpreted health data for a specific cluster entity."
**Source**: [Microsoft Learn — Architecture of Azure Service Fabric](https://learn.microsoft.com/en-us/azure/service-fabric/service-fabric-architecture) (learn.microsoft.com, high; page dated 2026-03-22) — accessed 2026-09-25.
**Confidence**: High for the documented architecture
**Verification**: [Kakivaya et al., "Service Fabric: A Distributed Platform for Building Microservices in the Cloud", EuroSys 2018 (ACM DL)](https://dl.acm.org/doi/10.1145/3190508.3190546) — search index states the neighbourhood-heartbeat, lease and arbitration design; the PDF returned HTTP 403, so the paper was not read (Knowledge Gaps).
**Analysis**: Service Fabric is evidence *for* the target's correctness split (membership and single-ownership decided by a strongly consistent mechanism — here arbitration, in the target the log) and *against* treating it as a hierarchical-aggregation precedent: its health view is centralised per cluster. It belongs in the comparison as "consistent membership + centralised aggregate", not as an Astrolabe descendant.

### Finding Q2.8: No production system running Astrolabe (or a gossip-hierarchy descendant) was found; the published deployment is ~60 machines

**Evidence**: The TOCS paper: "Although Astrolabe is currently deployed on approximately 60 machines, this is not a sufficiently large system to evaluate its scalability" (fn. 13); "Neither has Astrolabe at this time [been scaled beyond a few hundred servers]" (§8.1). The companion paper (Birman, van Renesse, Vogels, "Navigating in the Storm: Using Astrolabe for Distributed Self-Configuration, Monitoring and Adaptation", ~2003) describes the design and "a hypothetical commercial web service application" but reports no deployment. Searches for Astrolabe in production (including at Amazon, where Vogels became CTO in 2005, and via the authors' company Reliable Network Solutions) returned only biographical links, not deployment evidence. A crates.io crate named `astrolabe` is an unrelated date/time library; GitHub searches returned no Astrolabe implementation.
**Source**: Astrolabe TOCS 2003 fn. 13 and §8.1; [Birman, van Renesse, Vogels, "Navigating in the Storm" (cs.cornell.edu PDF)](https://www.cs.cornell.edu/projects/Quicksilver/public_pdfs/Navigating%20in%20the%20Storm.pdf) (out-of-list primary, workshop paper, medium-high); [Werner Vogels — Wikipedia](https://en.wikipedia.org/wiki/Werner_Vogels) (used only for the RNS/Amazon dates; not a trusted domain, not load-bearing); [crates.io — astrolabe](https://crates.io/crates/astrolabe) — accessed 2026-09-25.
**Confidence**: Medium (absence of evidence after targeted searches; a closed-source deployment could exist)
**Verification**: SDIMS and Moara evaluate against Astrolabe by *simulation* (Q2.1), consistent with there being no public large deployment to measure.
**Analysis**: The honest statement is: **Astrolabe's design is peer-reviewed and simulated to ~390k nodes; its production record is ~60 machines; no successor that keeps its gossip-hierarchy mechanism is known to run in production.** What *is* in production at scale is the *shape* — regional detail plus global summaries — in Monarch, Borgmon, Ganglia, and Fly (Q2.3, Q2.5, Q2.6), built with configured trees or two-level replication, not self-electing gossip representatives.

## Q3. Consumers and delivery models

### Finding Q3.1: Kubernetes shows the cost of full replication of service backends to every node — and scopes node agents to their own objects

**Evidence**: "With kube-proxy running on each Node and watching EndpointSlices, every change to an EndpointSlice becomes relatively expensive since it will be transmitted to every Node in the cluster." "By default, the control plane creates and manages EndpointSlices to have no more than 100 endpoints each … up to a maximum of 1000." "EndpointSlices act as the source of truth for kube-proxy when it comes to how to route internal traffic." Node authorizer: "Kubelets are limited to reading their own Node objects, and only reading pods bound to their node." Field selectors: Pods support `spec.nodeName`, `status.phase`, and others; "Supported field selectors vary by Kubernetes resource type."
**Source**: [Kubernetes — EndpointSlices](https://kubernetes.io/docs/concepts/services-networking/endpoint-slices/); [Kubernetes — Node Authorization](https://kubernetes.io/docs/reference/access-authn-authz/node/); [Kubernetes — Field Selectors](https://kubernetes.io/docs/concepts/overview/working-with-objects/field-selectors/) (kubernetes.io, high) — accessed 2026-09-25.
**Confidence**: High (official documentation, three pages)
**Verification**: Envoy's delta xDS exists for the same reason (Q3.2); independent project.
**Analysis**: Kubernetes uses *two* delivery models side by side: **full replication** of service endpoints to every node (kube-proxy), made tolerable by slicing (one change ≈ one ≤100-endpoint slice re-sent, not the whole list), and **interest-scoped** delivery for the node agent (pods with `spec.nodeName` = this node). Neither uses aggregation. The expensive one is exactly the dataplane consumer.

### Finding Q3.2: Envoy's incremental ("delta") xDS is the reference design for interest-scoped subscription with on-demand loading

**Evidence**: Delta xDS "Allows the protocol to communicate on the wire in terms of resource/resource name deltas … This supports the goal of scalability of xDS resources"; "the management server only needs to deliver the single cluster that changed." Subscriptions use "resource_names_subscribe and resource_names_unsubscribe fields in the DeltaDiscoveryRequest." It "Allows the Envoy to on-demand / lazily request additional resources. For example, requesting a cluster only when a request for that cluster arrives." Wildcard: "For Listener and Cluster resource types, there is also a 'wildcard' subscription"; "other xDS clients (such as gRPC clients that use xDS) may explicitly subscribe to specific resource names." "The client will silently ignore any supplied resources that were not explicitly requested." Ordering: "sequencing of updates should follow a make before break model, wherein: CDS updates (if any) must always be pushed first."
**Source**: [Envoy — xDS REST and gRPC protocol](https://www.envoyproxy.io/docs/envoy/latest/api-docs/xds_protocol) (envoyproxy.io, high) — accessed 2026-09-25.
**Confidence**: High (official protocol specification)
**Verification**: NATS gateway interest propagation (Q2.4) and Kubernetes field-scoped watches (Q3.1) are independent instances of "only send what the consumer declared interest in".
**Analysis**: xDS gives the target a vocabulary for the dataplane consumer: a node subscribes by *name* to the services it routes to (explicit subscription), receives deltas, and can lazily subscribe on the first connection to an unknown service (on-demand). The "make before break" ordering rule is a reminder that interest-scoped delivery still needs cross-resource ordering, which a per-origin gossip layer does not provide by itself.

The full consumer-by-consumer mapping, including where aggregates are sufficient and where exact rows are required, is in the closing section "Consumer-to-representation map".

## Q4. Correctness boundary for aggregates

### Finding Q4.1: Production schedulers already decide on stale, cached state — safely, because a single committer validates the decision against current state

**Evidence**: Borg: "A scheduler replica operates on a cached copy of the cell state. It repeatedly: retrieves state changes from the elected master …; updates its local copy; does a scheduling pass to assign tasks; and informs the elected master of those assignments. The master will accept and apply these assignments unless they are inappropriate (e.g., based on out of date state), which will cause them to be reconsidered in the scheduler's next pass. This is quite similar in spirit to the optimistic concurrency control used in Omega" (§3.4). "Relaxed randomization: … the scheduler examines machines in a random order until it has found 'enough' feasible machines to score, and then selects the best within that set" (§3.4). Liveness is decided by the master, not by the scheduler's view: "If a Borglet does not respond to several poll messages its machine is marked as down and any tasks it was running are rescheduled on other machines" (§3.3). Borg "rate-limits finding new places for tasks from machines that become unreachable, because it cannot distinguish between large-scale machine failure and a network partition" (§4). Link shards "aggregate and compress this information by reporting only differences to the state machines, to reduce the update load at the elected master" (§3.3).
**Source**: [Verma et al., "Large-scale cluster management at Google with Borg", EuroSys 2015](https://dl.acm.org/doi/10.1145/2741948.2741964), author copy [research.google PDF](https://research.google.com/pubs/archive/43438.pdf) (out-of-list primary of an ACM paper, scored high) — accessed 2026-09-25.
**Confidence**: High (peer-reviewed, production)
**Verification**: Prior repo research C1 (Omega, Nomad plan queue, Twine) establishes optimistic shared-state scheduling as the production-dominant pattern; Monarch's partial-results stance (Q2.3) and Astrolabe's own "no" list (Q1.4) define the staleness the scheduler must tolerate.
**Analysis**: This is the template for where aggregates may be used: **as candidate filters for a decision that is re-validated by the committer against authoritative state**. A stale "region B has 40 free GPUs" aggregate can pre-filter regions; the commit step must re-check against the chosen node's committed reservations (or the node must refuse at admission). Borg's "marked as down" is made by the elected master from its own poll results, never from an aggregate, and replacement is rate-limited because partition and mass failure look the same.

### Finding Q4.2: The Astrolabe authors both warn against and — in a companion paper — propose failover driven by aggregates; the formal guarantee does not support the latter

**Evidence**: TOCS: "When two users retrieve the same attribute at the same time, do they obtain the same result? … The answer to all these questions is no" (§4.3). "Navigating in the Storm": "When an event occurs that disrupts state – a machine crashes, or a service hangs – the deviation from the nominal state will become globally evident within seconds. Every healthy program will simultaneously notice failures or degradation … some server might take over tasks that a failed server had been responsible for, and advertise its new role." The same paper concedes: "if updates are more frequent, a 'new' value could overwrite an 'older' value, so that some machines might see the new update but miss the prior one"; "different users see events in different order and may not even see the identical events! This tradeoff seems to be fundamental to our style of distributed data fusion"; and for completeness "by comparing a count of the number of reporting child zones with a separately maintained count of the total number of children, applications can be shielded from seeing the results of an aggregation computation until the output is stable."
**Source**: Astrolabe TOCS 2003 §4.3; [Birman, van Renesse, Vogels, "Navigating in the Storm"](https://www.cs.cornell.edu/projects/Quicksilver/public_pdfs/Navigating%20in%20the%20Storm.pdf) §4.1, §4.3, §5 — accessed 2026-09-25.
**Confidence**: High (both primary, same authors)
**Verification**: Keep CALM and CRDT On (prior repo C2) — a non-monotone read of converged state (e.g., "no heartbeat ⇒ down") is unsafe without coordination; CEP-21 (prior repo C5.4) keeps gossip only as "input to failure detection".
**Analysis**: "Simultaneously" is marketing, not a guarantee: two nodes can read different aggregate values at the same instant (TOCS §4.3), so "take over tasks of a failed server" on an aggregate is a split-brain recipe. The reusable idea in the companion paper is the **completeness check** — report `reporting_children / total_children` so a consumer knows how partial an aggregate is. Monarch does the same with "pruned zones" (Q2.3).

### Finding Q4.3: The line between aggregate-safe and log-only uses (interpretation, grounded in Q1.4, Q2.3, Q4.1, Q4.2 and prior C2/C5.4)

**Analysis**: A decision may use an aggregate when *all* of the following hold; otherwise it must use the log (or exact, epoch-fenced facts plus a commit-time check):
1. **Monotone or re-validated**: the decision is re-checked by a single committer against authoritative state (Borg/Omega), or is safe under any stale value (a hint).
2. **Reversible or cheap to retry**: a wrong choice costs a retry or a slightly worse placement, not a double-run or a lost write.
3. **Completeness-aware**: the consumer sees how many children reported (Astrolabe's completeness count; Monarch's pruned zones) and treats "missing" as "unknown", never as zero.
4. **Not exclusive**: the decision grants nothing exclusive (no ownership, lease, fencing token, address, or leader role).

| Use | Aggregate allowed? | Why |
|---|---|---|
| Region/zone pre-filter for placement ("which regions have ≥N free cores?") | **Yes, as a hint** | Commit step re-validates against node reservations (Q4.1) |
| Spread / anti-affinity counts across zones | **Hint only** | A stale count can violate a hard anti-affinity constraint; the constraint must be checked against committed placements in the log |
| Dashboards, capacity trends, SLO roll-ups | **Yes** | Read-only; Monarch-style completeness indicator required |
| Rollout gating ("≥95% of region healthy") | **Hint + confirm** | Use the aggregate to trigger, confirm against exact facts before advancing a committed rollout step |
| "Node N is down" / replacement of its allocations | **No** | Non-monotone; needs the log (prior C2, C5.4; Borg's master-owned `down`) |
| Fencing epochs, ownership transfer, leases | **No** | Exclusive; must be committed (prior C5.8) |
| Service backend membership for routing | **No — needs exact rows** | A count of backends cannot route a packet; see Q3 |
| Quota / admission against hard limits | **No** (or escrow grants) | Over-admission is a correctness failure; prior C1 escrow |

## Q5. Zones, failure domains, and who owns the hierarchy

### Finding Q5.1: Every surveyed system aligns the hierarchy with network locality and failure domains, and treats zone *assignment* as configuration, not as something discovered by gossip

**Evidence**: Astrolabe: "the zone hierarchy has to be based on the network topology so that load on network links and routers remains within reason. As a rule of thumb, if a collection of machines can be divided into two groups separated by a single router, these groups should be in disjoint zones"; "Initially, new machines can be added simply to leaf zones, but at some point it becomes necessary to divide the leaf zones into smaller zones. Note that this re-configuration only involves the machines in that leaf zone" (§7.1); zone names are set by administrators (§2); "the topology of the tree is determined by the human administrator's assignment of zone names" (§3.2). Monarch: "Each Monarch zone is autonomous, and consists of a collection of clusters, i.e., independent failure domains, that are in a strongly network-connected region" (§2); "The location-to-zone mapping is specified in configuration to ingestion routers and can be updated dynamically" (§4.1). Kubernetes: "When nodes start up, the kubelet on each node automatically adds labels to the Node object … These labels can include zone information"; topology spread constraints spread Pods "among fault domains: regions, zones, and even specific nodes."
**Source**: Astrolabe TOCS 2003 §2, §3.2, §7.1; Monarch PVLDB 2020 §2, §4.1; [Kubernetes — Running in multiple zones](https://kubernetes.io/docs/setup/best-practices/multiple-zones/) — accessed 2026-09-25.
**Confidence**: High (three independent systems)
**Verification**: Borg spreads "tasks of a job across failure domains such as machines, racks, and power domains" (Borg §4); Fly's per-region clusters (Q2.6).
**Analysis**: No production system lets the monitoring hierarchy invent its own shape. The *shape* (which node is in which zone) is configuration or a node-reported label; only *representative selection* is automatic. For the target this suggests: **derive the zone path from committed node labels on the log** (`region/zone/rack`), so every node computes the same tree deterministically; let representatives be chosen by an Astrolabe-style deterministic function over gossiped facts (non-exclusive, so no consensus needed — Q1.3).

### Finding Q5.2: Zone membership changes are cheap in Astrolabe because they are local — but the old entry lingers until T_fail, which double-counts during a move (interpretation)

**Evidence**: Re-partitioning a leaf zone "only involves the machines in that leaf zone. Other parts of Astrolabe do not need to know about the re-configuration" (§7.1). Failed or moved participants are removed only after "T_fail" without updates from their representative (§4.2). Monarch moves target ranges between leaves with overlap: "both the source and destination leaves are collecting, storing, and logging the same data simultaneously to provide continuous data availability" (§4.2), and resolves duplicates at query time by "replica resolution" (§5.2).
**Source**: Astrolabe TOCS 2003 §4.2, §7.1; Monarch PVLDB 2020 §4.2, §5.2 — accessed 2026-09-25.
**Confidence**: Medium (the double-count consequence is an inference from the two mechanisms, not stated by either paper)
**Verification**: Monarch's explicit dual-write-then-resolve is independent evidence that a move needs de-duplication.
**Analysis**: If a node's zone changes (relabel, or a region split), its facts appear in the new zone at once and in the old zone until the old MIB ages out: a SUM over the root can briefly count it twice. With labels committed on the log, each node's leaf facts can carry the *label epoch*, and aggregators can drop inputs whose epoch is older than the committed one — the same epoch-fencing move the prior research recommends for ownership (C5.8). That makes the hierarchy's shape consensus-derived while its *contents* stay gossip-maintained.

## Q6. Implementation landscape

### Finding Q6.1: There is no maintained open-source Astrolabe implementation, in Rust or otherwise; the Rust building blocks are flat membership/gossip libraries

**Evidence**: Searches of GitHub and crates.io for Astrolabe implementations returned only unrelated projects (the crates.io `astrolabe` crate is a date/time library) (Q2.8). The available Rust pieces: chitchat — Scuttlebutt anti-entropy where "A node can only edit its own node state", phi-accrual failure detection, "partial deltas to fit UDP packets", maintained under Datadog-owned Quickwit (prior repo C5.2–C5.3); foca — SWIM membership "a building block" with LAN/WAN tuning and "no specific cluster size limits" published (prior repo corrosion-scale doc F8); iroh-gossip — "epidemic broadcast trees … based on the papers HyParView and PlumTree" ([crates.io — iroh-gossip](https://crates.io/crates/iroh-gossip)). NATS leaf nodes: "A leaf node is a NATS server that opens an outbound connection to a remote NATS system and bridges subject interest across it"; "leaf links compose into trees, not just a single hub and spoke" ([NATS docs — Leaf Nodes](https://docs.nats.io/running-a-nats-service/configuration/leafnodes)).
**Source**: As cited inline — accessed 2026-09-25.
**Confidence**: Medium-High (absence claim from targeted searches; library facts from primary repositories/docs)
**Verification**: Prior repo research (C5.3, corrosion-scale doc) independently surveyed chitchat and foca and found no hierarchy support.
**Analysis — what building it would take (estimate, labelled)**: On top of a per-origin, single-writer fact layer (chitchat-style), an Astrolabe-style tier needs: (1) a zone path per node, derived from committed labels (Q5.1); (2) one gossip group per zone level, each carrying fixed-size zone summaries (not rows); (3) a small, *compiled-in* set of aggregation functions (count, sum, min/max, bitmap-OR, first-n) instead of SQL-in-certificates — the target has no need for runtime-installed mobile code, which removes the AFC security problem (Q1.7); (4) deterministic representative choice (e.g., lowest node-ID among the k most recent heartbeats); (5) completeness counters and label epochs on every summary (Q4.2, Q5.2); (6) summary signing by the representative's node key. Items (2)–(6) are a few thousand lines of protocol code, but the testing burden (partition, relabel, representative churn under DST) is the real cost. On NATS, the equivalent is a subject hierarchy (`obs.<region>.<zone>.summary`) with per-zone aggregator subscribers — simpler to build, but it puts a second messaging system in the path and aggregators become designated, not self-elected.

## Q7. Cost/benefit vs regionalized full replication plus interest-scoped subscription

### Finding Q7.1: Per-node cost of the four delivery models (estimates, labelled)

**Evidence (inputs)**: Astrolabe: ~200–300 B per compressed MIB, branching factor 5–50, leaf zones 32–64 members, one message per round per represented level (Q1.6). Flat anti-entropy message size "grows linearly in N" (prior C5.2) and Clearinghouse-style flat gossip has "storage [that] grows O(n), while the total bandwidth taken up by gossip grows O(n²)" (Q1.6). Consul LAN gossip pools: ≤5,000 agents recommended, 66,000 tested (prior corrosion-scale doc F9, from HashiCorp's scale test report and scale docs). Kubernetes: every EndpointSlice change "will be transmitted to every Node" (Q3.1). Consul already splits a datacenter's gossip below the region level: network segments are "isolated LAN gossip pools that only require full connectivity between agent members on the same segment", and "Server agents are members of all segments" ([Consul — Network segments](https://developer.hashicorp.com/consul/docs/multi-tenant/network-segment), accessed 2026-09-25).
**Estimates (assumptions stated; not measured)**: Assume each node owns 1 node record + 20 allocation records, 300 B each (≈6.3 KB of facts per node), and changes one record every 10 s (0.1 updates/s/node). Ignore gossip redundancy (Plumtree-style broadcast approaches 1× delivery; epidemic push is several ×).

| Model | State held per node | Inbound update bytes per node | 5,000 nodes / 10 regions | 50,000 nodes / 50 regions |
|---|---|---|---|---|
| Flat full replication | N × 6.3 KB | N × 0.1 × 300 B | ≈32 MB; ≈150 KB/s | ≈315 MB; ≈1.5 MB/s (cluster total ≈75 GB/s) |
| Regionalized full replication (Fly) | N_region × 6.3 KB + global index | N_region × 0.1 × 300 B + index churn | 500/region: ≈3 MB; ≈15 KB/s | 1,000/region: ≈6 MB; ≈30 KB/s |
| Astrolabe (bf 32, 3 levels) | ≈bf × depth × 1 KB | ≈depth × 8 KB per 2 s round | ≈100 KB; ≈12 KB/s | ≈100 KB; ≈12 KB/s |
| Interest-scoped (dataplane) | Σ backends of subscribed services | churn of those services only | Depends on fan-in, e.g. 20 services × 50 backends ≈ 1,000 rows ≈ 50 KB | Same — independent of N |

**Source**: derived from the cited inputs; all figures are estimates, not measurements.
**Confidence**: Low-Medium (arithmetic is simple; the input rates are assumptions — no source gives an orchestrator's per-node fact write rate, see Knowledge Gaps)
**Verification**: Astrolabe's own Figure 10 (≈21–56 messages per round at 390k members, Q1.6) bounds the hierarchical row; Fly's ~800-server / ~40-region envelope (prior corrosion F1) sits in the regionalized row's comfortable range.
**Analysis**:
- **Regionalized full replication is enough for the "thousands of nodes across regions" target** as long as *each region* stays within one gossip pool's comfortable size (low thousands; ≤5,000 by Consul's guidance). The global tier stays tiny because it carries only projections (Fly's app→region map) or per-region summaries: 50 regions × ~1 KB summary = ~50 KB, cheap even if every node holds it.
- **The binding constraint is per-region density, not region count.** True multi-level aggregation (node → rack/AZ → region → global) starts to pay off when a *single region* grows past a few thousand nodes — at that point per-node regional state (~6 MB per 1,000 nodes in this model) and regional fan-out grow linearly again, and an intra-region zone level is the fix. Astrolabe's own recommendation of 32–64-member leaf zones implies ~3 levels at ~10⁴–10⁵ nodes.
- **Aggregation does not replace exact delivery for the heaviest consumer.** The dataplane needs exact backend rows; aggregation cannot serve it. Interest-scoped subscription (xDS-style delta, NATS-style interest) is what bounds that cost, and it is needed in *every* model.
- **Operator drill-down is cheapest as on-demand scatter-gather** (SDIMS probe / Monarch query tree): the rows stay where they are produced, and the query carries a completeness indicator.

## Astrolabe — what it actually guarantees

From the primary paper (Q1), stated as the authors state them:

**Guaranteed**
- **Eventual reflection of updates, with probability 1**: an update to an input attribute, or a later update to it, is eventually reflected in every dependent aggregate. If updates stop, replicas converge (Q1.4).
- **Bounded per-node cost**: state ≈ branching factor × depth MIBs; message load ≈ one message per round per represented level, O(log n) (Q1.1, Q1.6).
- **Failure cleanup within T_fail**: failed participants and empty zones are removed within T_fail seconds (Q1.8).
- **Integrity, not secrecy**: MIBs, zones and aggregation functions are signed under a per-zone CA hierarchy; outsiders cannot inject zones or "false gossip" (Q1.7).
- **Optional monotonic aggregates**: with (min, max) issued-time tracking, a newer aggregate is built only from strictly newer inputs — at the cost of waiting 5–35 gossip rounds (Q1.4).

**Not guaranteed** (the authors answer "no" to each)
- That an aggregate includes the latest changes.
- That two readers at the same time see the same value.
- That a snapshot reflects one instant.
- That a later snapshot was taken after an earlier one (unless monotonic mode is on).
- Protection against a keyholder or representative reporting a false aggregate, or against faulty aggregation functions (Q1.7).
- A staleness bound: there is no "as of" time, and a partitioned zone's aggregate is kept until T_fail and then *disappears* rather than being marked stale (Q1.8).

**Measured, not guaranteed**: ~20–35 s mean dissemination on a 63-host testbed at 5 s gossip; "tens of seconds" in simulation to ~390k members (Q1.5). Deployment evidence: ~60 machines (Q1.8, Q2.8).

## Successors and production evidence

| System | Relation to Astrolabe | Mechanism | Evidence level | Finding |
|---|---|---|---|---|
| SDIMS (2004) | Direct successor | Per-attribute DHT trees; tunable Update-Up/Down; administrative isolation | Peer-reviewed; 4,096-node simulation; 283-node + 69-node PlanetLab testbeds | Q2.1 |
| Moara (2008) | Successor (group-scoped queries) | Per-group aggregation trees on a DHT | Peer-reviewed; abstract only read | Q2.2 |
| Google Monarch (2020) | Same *shape*, different mechanism | Regional autonomous zones; root/zone mixers; push-down; field-hints index; partial results | Production at planet scale; peer-reviewed | Q2.3 |
| Borgmon | Same shape, older | Fixed per-cluster + global aggregator tree; upper tiers filter what streams up | Production (SRE book) | Q2.5 |
| Ganglia (2004) | Same shape | Multicast within cluster; tree of representative nodes across clusters | Production on >500 clusters (per paper abstract) | Q2.5 |
| Fly.io regionalization | Two-level, ad hoc | Per-region Corrosion (full detail) + global app→region map | Production; first-party blog + post-mortems | Q2.6 |
| NATS super-clusters / leaf nodes | Different axis: interest, not aggregation | Gateways forward only subjects with remote interest; leaf trees | Production; official docs | Q2.4, Q6.1 |
| Service Fabric federation | Not a descendant | Consistent ring membership + centralised health store | Production; official docs | Q2.7 |
| Borg link shards | Aggregation toward a master | Shards aggregate Borglet state and send diffs to the elected master | Production; peer-reviewed | Q4.1 |
| **Astrolabe itself or a gossip-hierarchy descendant in production** | — | — | **None found** | Q2.8 |
| San Fermín, Willow, PIER | Lineage | — | **Not researched** (turn budget) | Knowledge gaps |

The pattern that is proven in production is **"detail stays regional, summaries and queries cross regions"**. The pattern that is *not* proven is **self-electing gossip representatives computing runtime-installed aggregates**.

## Consumer-to-representation map

| Consumer | What it needs | Representation | Delivery model | Aggregate sufficient? |
|---|---|---|---|---|
| **Dataplane** (this node's eBPF maps) | Exact backend rows (address, port, readiness, weight) for services this node routes to | Per-service backend set, single-writer rows from each backend's node | Interest-scoped delta subscription (xDS-style name subscribe + on-demand on first connect, Q3.2); within a region from regional replicas, across regions only for global services | **No** — a count cannot route a packet |
| **Gateway** | Route table (host → service) and where a service is present | Routes are intent (log); presence is a per-region projection (service → regions with ready backends, plus per-region ready count) | Routes from the log; presence as a small global projection (Fly's app→region map, Q2.6); exact backends fetched from the chosen region | **Partly** — region presence/counts decide *which region*; exact rows decide *which backend* |
| **Scheduler** | Where capacity exists; then exact capacity on candidates | Zone/region summaries: free resources by class, schedulable-node count, histogram buckets; exact per-node free capacity + committed reservations for candidates | Hierarchical summaries for pre-filtering; exact facts + log at commit (Q4.1) | **Yes for pre-filtering; no for commit** |
| **Operator CLI — cluster-wide** | Counts, health roll-ups, "how many allocations failing" | Aggregates with completeness (reporting/total zones) | Read local copy of summaries | **Yes**, with completeness shown |
| **Operator CLI — drill-down** | One allocation's exact state and history | Owner's single-writer rows + occurrence records | On-demand scatter-gather to the owning region/node (SDIMS probe, Monarch query tree) | **No** |
| **Failure-detection evidence** | Heartbeats of monitored peers; correlated-failure signal | Exact per-peer heartbeat among neighbours (within zone); zone-level "fraction reachable" aggregate | Local exact gossip; aggregate for correlated failure (rate-limit replacement as Borg does, Q4.1) | Evidence only; **the decision goes on the log** |
| **Worker** (own assignments) | Committed assignments | Log entries | Learner replay (sibling doc A2) | Not observation at all |
| **Dashboards / SLOs** | Trends, percentiles | Aggregates, histograms | Summaries + Monarch-style queries with partial-result flags | **Yes** |

## Correctness boundary

The rule that falls out of Q4 (interpretation, consistent with the prior research's correctness-impact amendment):

- **Aggregates may inform; they may not decide anything exclusive or irreversible.** Allowed uses are hints that a single committer re-validates (placement pre-filter), read-only views (dashboards), and triggers that are confirmed against exact facts before a committed step (rollout gating).
- **Forbidden uses**: node-down, allocation replacement, fencing epochs and ownership transfer, leases, backend membership for routing, and hard quota/anti-affinity enforcement. These need the log or exact, epoch-fenced facts plus a commit-time check.
- **Every aggregate must carry completeness and age**: reporting-children / total-children (Astrolabe companion paper, Q4.2) and the minimum input issue time (Astrolabe's min/max tracking, Q1.4). A missing zone is "unknown", never zero (Monarch's pruned-zone notice, Q2.3).
- **Correlated-failure signals come from aggregates, and they should *slow* decisions, not trigger them**: Borg rate-limits replacement because partition and mass failure look alike (Q4.1). A region-level "fraction reachable" collapse is exactly the case to throttle replacement decisions on the log.

## Comparison vs regionalized full replication and interest-scoped subscription

| Criterion | Flat full replication | Regionalized full replication (Fly) | Interest-scoped subscription (xDS / NATS / k8s field-scoped watch) | Hierarchical aggregation (Astrolabe-style) | On-demand scatter-gather (SDIMS probe / Monarch) |
|---|---|---|---|---|---|
| Per-node state | O(N) | O(N_region) + small global projection | O(what the node subscribed to) | O(bf × depth) | none standing |
| Per-node bandwidth | O(N × write rate) | O(N_region × write rate) | O(churn of subscribed items) | O(depth) per round, independent of write rate | per query |
| Exact rows | Yes, all | Yes in-region; projections globally | Yes, for subscribed items | **No** — bounded summaries only | Yes, for the queried scope |
| Freshness | Seconds (gossip) | Seconds in-region | Push latency of the source | Tens of seconds; hierarchy ~2× slower than flat (Q1.5) | As fresh as the source, plus query latency |
| Partition behaviour | Each side keeps stale remote rows | Region autonomous; global projection stale | Subscription breaks at the boundary; needs resubscribe/relist | Remote zone aggregate stale, then vanishes at T_fail | Partial results; needs completeness flag |
| Serves | Everything, at a cost | Everything in-region; gateway forwarding | Dataplane, gateway, node agent | Scheduler pre-filter, dashboards, correlated-failure evidence | Operator drill-down, ad hoc queries |
| Production evidence | Corrosion ≤ ~800 servers (prior F1) | Fly (production) | Envoy/Istio, Kubernetes, NATS | **~60 machines** (Astrolabe); shape proven by Monarch/Borgmon/Ganglia | Monarch, SDIMS testbeds |
| Build cost on the target | Existing candidates | Regional pools + one global projection | Subscription API over the fact layer | New protocol tier + DST burden (Q6.1) | Query routing over the region tier |

**Where true hierarchical aggregation starts to pay (estimate, Q7.1)**: not at a region count — 50–100 regions' summaries fit in ~100 KB — but when **one region outgrows one gossip pool** (low thousands of nodes; ≤5,000 by Consul's guidance). Below that, a two-level scheme (regional detail + global projections/summaries) *is* a depth-2 hierarchy, and nothing deeper is needed.

## Conflicting information

### Conflict 1: Do all nodes see a failure at the same time?
**Position A**: "Every healthy program will simultaneously notice failures or degradation … some server might take over tasks that a failed server had been responsible for" — [Navigating in the Storm](https://www.cs.cornell.edu/projects/Quicksilver/public_pdfs/Navigating%20in%20the%20Storm.pdf), workshop paper, reputation 0.8.
**Position B**: "When two users retrieve the same attribute at the same time, do they obtain the same result? … The answer to all these questions is no" — [Astrolabe TOCS 2003 §4.3](https://dl.acm.org/doi/10.1145/762483.762485), peer-reviewed journal, reputation 1.0. The companion paper itself concedes "different users see events in different order and may not even see the identical events!"
**Assessment**: Position B is authoritative: it is the formal guarantee in the peer-reviewed journal version, and Position A's own later section contradicts its "simultaneously". Failover must not be driven by Astrolabe aggregates.

### Conflict 2: Scale claims vs deployment evidence
**Position A**: "Astrolabe could scale to thousands and perhaps millions of nodes" (TOCS abstract) — simulation to 390,625 members.
**Position B**: "Astrolabe is currently deployed on approximately 60 machines" (TOCS fn. 13); SDIMS: Astrolabe's replicate-all-aggregates strategy "may limit the system's ability to accommodate large numbers of attributes" (SDIMS §1).
**Assessment**: Not contradictory once separated: the claim is about *node count* under a small, fixed set of aggregates; SDIMS's objection is about *attribute count*. The node-count claim is simulation-only.

### Conflict 3: Fly's fleet size
**Position A**: "running on about 800 physical servers" in "40 different regions" (prior repo corrosion doc F1).
**Position B**: The 2025 blog, as returned by the fetch tool, refers to "thousands of high-powered servers around the world" and "dozens of regions".
**Assessment**: Unresolved; the second phrasing came through a summarising fetch and may describe Fly's whole fleet rather than Corrosion's membership. The ~800-server figure remains the specific published Corrosion envelope.

### Conflict 4 (minor): Dissemination latency
Navigating in the Storm: "Using a 2-second gossip rate, an update would thus reach all members in a system of 10,000 computers in roughly 25 seconds." TOCS Emulab measurement: ≈22–35 s mean on 63 hosts at a 5 s gossip interval. These agree once the gossip interval is normalised (≈5–7 rounds at 5 s vs ≈12 rounds at 2 s); the 10,000-node figure is a projection, not a measurement.

## Knowledge gaps

### Gap 1: No production or independent measurement of Astrolabe
**Issue**: Beyond the authors' ~60-machine deployment and 126-agent testbed, no deployment or third-party measurement was found. | **Attempted**: GitHub and crates.io searches; searches for Amazon/Reliable Network Solutions use; SDIMS and Moara evaluations (both simulate Astrolabe). | **Recommendation**: treat all Astrolabe scale numbers as simulation; validate any adopted tier under DST and a Tier-3 multi-region test.

### Gap 2: San Fermín, Willow and PIER not researched
**Issue**: Named in the brief; not fetched within the turn budget. | **Attempted**: none beyond SDIMS and Moara. | **Recommendation**: a short follow-up on San Fermín and Willow (both later aggregation designs, venues and mechanisms not verified here) and on PIER (a DHT-based query engine), to check whether any addresses representative fragility (Q1.3) in a way the target could reuse.

### Gap 3: Moara, Service Fabric (EuroSys 2018) full texts not read
**Issue**: Moara's author PDF returned 404; the Service Fabric ACM PDF returned 403. Findings Q2.2 and Q2.7 rely on the abstract and on Microsoft Learn respectively. | **Recommendation**: read the full papers if Moara's group-scoped trees or Service Fabric's arbitration become design candidates.

### Gap 4: Uber M3 aggregation tiers and Cilium ClusterMesh global services not verified
**Issue**: m3db.io aggregator page returned 404; Cilium's global-services page returned a redirect stub. | **Recommendation**: if cross-cluster service export (Cilium-style "shared" services) is considered as the gateway model, fetch the Cilium global-services guide directly.

### Gap 5: No orchestrator per-node fact write rate
**Issue**: Q7.1's bandwidth table rests on an assumed 0.1 updates/s/node and 21 records/node. Prior research recorded the same gap. | **Recommendation**: measure allocation-status, health and heartbeat write rates in a realistic deploy; the trigger for an intra-region zone level depends on it.

### Gap 6: Verifiable / Byzantine-resistant aggregation not researched
**Issue**: Astrolabe admits a representative can report a false aggregate (Q1.7). Later work on verifiable aggregation was not surveyed. | **Recommendation**: only needed if nodes are not mutually trusted; with CA-backed node records and a single operator this is lower priority.

### Gap 7: Kubernetes watch semantics page truncated
**Issue**: The API-concepts page (watch bookmarks, 410 Gone) returned truncated content; Q3.1 uses the EndpointSlice, node-authorizer and field-selector pages instead. | **Recommendation**: fetch the watch section if a List-then-Watch contract is designed for the subscription API.

## Recommendation for the target design

These are research recommendations for the architect, not decisions.

1. **Do not adopt Astrolabe as the observation layer.** It cannot carry exact rows (Q1.2), its production evidence is ~60 machines (Q2.8), and its heaviest would-be consumer — the dataplane — needs exact backends (Q3). Adopt its *shape* as a **summary tier on top of** a regional, single-writer fact layer.
2. **Start with two levels; design for three.** Regional full replication of per-origin, epoch-fenced facts (the prior research's recommended fact layer) plus a small global tier holding (a) projections such as service → regions-with-ready-backends and (b) fixed per-region summaries. This is Fly's scheme generalised (Q2.6) and is a depth-2 hierarchy. Add an intra-region zone level (rack/AZ) only when a region approaches one gossip pool's comfortable size — low thousands of nodes (Q7.1).
3. **Serve each consumer with its own delivery model** (consumer map above): interest-scoped delta subscription for the dataplane and gateway (xDS semantics, Q3.2); summaries for scheduler pre-filtering and dashboards; on-demand scatter-gather for operator drill-down (SDIMS/Monarch, Q2.1, Q2.3); the log for workers' assignments.
4. **Derive the hierarchy from the log; keep representative choice gossip-local.** Zone paths come from committed node labels, so every node computes the same tree (Q5.1). Representatives are chosen by a deterministic function over gossiped facts; they are non-exclusive, so no consensus is needed (Q1.3). Stamp every leaf fact and summary with the label epoch to drop inputs from a node's old zone after a relabel (Q5.2).
5. **Compile in a fixed set of aggregation functions** (count, sum, min/max, bitmap-OR, first-n, histogram merge). Drop Astrolabe's runtime-installed SQL in certificates: it creates a mobile-code security problem (Q1.7) the target does not need. Sign each summary with the computing node's key.
6. **Make staleness and completeness first-class** on every summary: reporting/total children, minimum input time, and label epoch. Consumers treat a missing zone as unknown (Q4.2, Q2.3).
7. **Hold the correctness boundary** (above): aggregates may pre-filter and alert; the log decides node-down, replacement, fencing and ownership; hard constraints are re-checked at commit (Q4.1, Q4.3). Use a region-level reachability collapse to *throttle* replacement, as Borg does.
8. **Before building the summary tier**, measure per-node fact write rates (Gap 5) and prototype regional replication plus interest-scoped subscription; the measured rates set the node count at which the third level is triggered.

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|---|---|---|---|---|---|
| Astrolabe, ACM TOCS 2003 (author copy) | dl.acm.org / cs.cornell.edu | High (1.0) | academic | 2026-09-25 | Y (SDIMS, companion paper) |
| Navigating in the Storm (Birman et al.) | cs.cornell.edu (out-of-list) | Medium-High (0.8) | academic (workshop) | 2026-09-25 | Y (TOCS) |
| SDIMS, SIGCOMM 2004 (author copy) | dl.acm.org / cs.utexas.edu | High (1.0) | academic | 2026-09-25 | Y (Astrolabe, Moara) |
| Moara, Middleware 2008 (abstract) | dl.acm.org / springer | High (1.0) | academic | 2026-09-25 | Partial (abstract only) |
| Monarch, PVLDB 2020 | vldb.org (out-of-list, VLDB) | High (1.0) | academic / production | 2026-09-25 | Y (SRE book, Borg) |
| Borg, EuroSys 2015 (author copy) | dl.acm.org / research.google | High (1.0) | academic / production | 2026-09-25 | Y (prior C1: Omega, Nomad) |
| Google SRE book ch. 10 (Borgmon) | sre.google (out-of-list) | Medium-High (0.8) | industry | 2026-09-25 | Y (Monarch §1) |
| Ganglia, Parallel Computing 2004 (abstract) | sciencedirect.com (out-of-list) | Medium-High (0.8) | academic | 2026-09-25 | Y (Borgmon, Astrolabe shape) |
| Fly.io — Corrosion blog | fly.io | High (1.0) | industry / official | 2026-09-25 | Y (prior corrosion F1–F2) |
| NATS — Gateways | docs.nats.io (nats.io) | High (1.0) | official | 2026-09-25 | Y (leaf nodes, xDS pattern) |
| NATS — Leaf Nodes | docs.nats.io (nats.io) | High (1.0) | official | 2026-09-25 | Y (gateways) |
| Envoy — xDS protocol | envoyproxy.io | High (1.0) | official | 2026-09-25 | Y (k8s, NATS) |
| Kubernetes — EndpointSlices | kubernetes.io | High (1.0) | official | 2026-09-25 | Y (xDS) |
| Kubernetes — Node Authorization | kubernetes.io | High (1.0) | official | 2026-09-25 | Y (field selectors) |
| Kubernetes — Field Selectors | kubernetes.io | High (1.0) | official | 2026-09-25 | Y |
| Kubernetes — Running in multiple zones | kubernetes.io | High (1.0) | official | 2026-09-25 | Y (Monarch, Astrolabe) |
| Microsoft Learn — Service Fabric architecture | learn.microsoft.com | High (1.0) | official | 2026-09-25 | Partial (EuroSys paper unread) |
| Service Fabric, EuroSys 2018 (index only) | dl.acm.org | High (1.0) | academic | 2026-09-25 | N (403) |
| Consul — Network segments | developer.hashicorp.com | High (1.0) | official | 2026-09-25 | Y (prior corrosion F9) |
| crates.io — iroh-gossip | crates.io | High (1.0) | technical | 2026-09-25 | N |
| crates.io — astrolabe (negative evidence) | crates.io | High (1.0) | technical | 2026-09-25 | Y (GitHub search) |
| Wikipedia — Werner Vogels (dates only, non-load-bearing) | wikipedia.org (not on list) | excluded from scoring | reference | 2026-09-25 | — |

Reputation (21 scored): High 18 (86%) | Medium-High 3 (14%) | Avg ≈ 0.97. Prior repo documents are cited as internal context and not scored.

## Full Citations

[1] van Renesse, R., Birman, K. P., Vogels, W. "Astrolabe: A Robust and Scalable Technology for Distributed System Monitoring, Management, and Data Mining". ACM TOCS 21(2):164–206. 2003. https://dl.acm.org/doi/10.1145/762483.762485 (author copy https://www.cs.cornell.edu/home/rvr/papers/astrolabe.pdf). Accessed 2026-09-25.
[2] Birman, K. P., van Renesse, R., Vogels, W. "Navigating in the Storm: Using Astrolabe for Distributed Self-Configuration, Monitoring and Adaptation". c. 2003. https://www.cs.cornell.edu/projects/Quicksilver/public_pdfs/Navigating%20in%20the%20Storm.pdf. Accessed 2026-09-25.
[3] Yalagandula, P., Dahlin, M. "A Scalable Distributed Information Management System". ACM SIGCOMM 2004. https://dl.acm.org/doi/10.1145/1030194.1015509 (author copy https://www.cs.utexas.edu/~dahlin/projects/sdims/papers/sdims-sigcomm.pdf). Accessed 2026-09-25.
[4] Ko, S. Y., Yalagandula, P., Gupta, I., Talwar, V., Milojicic, D., Iyer, S. "Moara: Flexible and Scalable Group-Based Querying System". Middleware 2008. https://dl.acm.org/doi/10.5555/1496950.1496975 ; https://link.springer.com/chapter/10.1007/978-3-540-89856-6_21. Accessed 2026-09-25.
[5] Adams, C. et al. "Monarch: Google's Planet-Scale In-Memory Time Series Database". PVLDB 13(12):3181–3194. 2020. https://www.vldb.org/pvldb/vol13/p3181-adams.pdf. Accessed 2026-09-25.
[6] Verma, A. et al. "Large-scale cluster management at Google with Borg". EuroSys 2015. https://dl.acm.org/doi/10.1145/2741948.2741964 (author copy https://research.google.com/pubs/archive/43438.pdf). Accessed 2026-09-25.
[7] Google. "Practical Alerting from Time-Series Data". Site Reliability Engineering, ch. 10. 2016. https://sre.google/sre-book/practical-alerting/. Accessed 2026-09-25.
[8] Massie, M. L., Chun, B. N., Culler, D. E. "The Ganglia distributed monitoring system: design, implementation, and experience". Parallel Computing 30(7). 2004. https://www.sciencedirect.com/science/article/abs/pii/S0167819104000535. Accessed 2026-09-25.
[9] Fly.io. "Corrosion". Fly.io blog. 2025. https://fly.io/blog/corrosion/. Accessed 2026-09-25.
[10] NATS project. "Gateways (super-clusters)". NATS docs. https://docs.nats.io/running-a-nats-service/configuration/gateways. Accessed 2026-09-25.
[11] NATS project. "Leaf Nodes". NATS docs. https://docs.nats.io/running-a-nats-service/configuration/leafnodes. Accessed 2026-09-25.
[12] Envoy Project. "xDS REST and gRPC protocol". https://www.envoyproxy.io/docs/envoy/latest/api-docs/xds_protocol. Accessed 2026-09-25.
[13] Kubernetes. "EndpointSlices". https://kubernetes.io/docs/concepts/services-networking/endpoint-slices/. Accessed 2026-09-25.
[14] Kubernetes. "Using Node Authorization". https://kubernetes.io/docs/reference/access-authn-authz/node/. Accessed 2026-09-25.
[15] Kubernetes. "Field Selectors". https://kubernetes.io/docs/concepts/overview/working-with-objects/field-selectors/. Accessed 2026-09-25.
[16] Kubernetes. "Running in multiple zones". https://kubernetes.io/docs/setup/best-practices/multiple-zones/. Accessed 2026-09-25.
[17] Microsoft. "Architecture of Azure Service Fabric". Microsoft Learn. 2026-03-22. https://learn.microsoft.com/en-us/azure/service-fabric/service-fabric-architecture. Accessed 2026-09-25.
[18] Kakivaya, G. et al. "Service Fabric: A Distributed Platform for Building Microservices in the Cloud". EuroSys 2018. https://dl.acm.org/doi/10.1145/3190508.3190546. Accessed 2026-09-25 (not read; HTTP 403).
[19] HashiCorp. "Network segments overview". Consul docs. https://developer.hashicorp.com/consul/docs/multi-tenant/network-segment. Accessed 2026-09-25.
[20] n0 / iroh. "iroh-gossip". crates.io. https://crates.io/crates/iroh-gossip. Accessed 2026-09-25.
[21] "astrolabe" (date/time library; negative evidence). crates.io. https://crates.io/crates/astrolabe. Accessed 2026-09-25.

Internal context (not scored): `docs/research/architecture/flat-cluster-placement-and-observation-comprehensive-research.md` (C1, C2, C5.2–C5.8); `docs/research/architecture/flat-cluster-consensus-and-log-dissemination-comprehensive-research.md` (A2); `docs/research/architecture/flat-cluster-membership-discovery-and-admission-comprehensive-research.md`; `docs/research/scalability/corrosion-crsqlite-production-scale-evidence.md` (F1, F2, F8, F9).

## Research Metadata

Duration: ~50 turns | Examined: ~30 sources | Cited: 21 (+4 internal) | Cross-refs: every High finding has ≥2 independent organisations | Confidence (25 findings): High 17 (68%), Medium-High 3 (12%), Medium 4 (16%), Low-Medium 1 (4%) | Tool failures: PDF text extraction failed for all academic PDFs (read as page images instead); HTTP 404 (Moara author PDF, m3db.io aggregator page), HTTP 403 (Service Fabric EuroSys PDF), truncated pages (Kubernetes API concepts, labels reference), redirect stub (Cilium global services) | Output: docs/research/architecture/astrolabe-style-hierarchical-aggregation-for-observation-comprehensive-research.md
