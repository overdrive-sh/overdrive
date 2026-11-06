# Research: Should Overdrive adopt Hydro (hydro.run)?

**Date**: 2026-11-06 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium | **Sources**: 17 cited

## Executive Summary
Hydro (UC Berkeley, now co-led with AWS; Apache-2.0) is a Rust location-typed distributed dataflow framework (`hydro_lang`) over a Tokio-based dataflow runtime (DFIR), with a built-in deterministic simulator, Hydro Deploy, and published research (SIGMOD'24 protocol rewrites, CALM-based CRDT queries, Flo, Suki). As of 2026-11-06 the latest `hydro_lang` is 0.17.0-alpha.6; every release is 0.x/alpha with type-level breaking changes between alphas, ~6.6k total downloads, two engineers dominate commits, and "Production Ready" is a self-claim with no independent production deployment found.

For Overdrive, no surface is a good fit: its Paxos/Raft/2PC live in a test crate with an in-memory Raft log and fixed cluster size (partial read); no durable-state or workflow engine was found; the simulator explores Hydro programs only and its fault model and clock/RNG injection are undocumented, so it cannot replace turmoil + Sim* adapters over arbitrary Rust; Hydro Deploy and eBPF are irrelevant to an appliance OS.

**Recommendation: borrow ideas only.** The most valuable idea is the CALM/monotonicity lens on reads of eventually-consistent (LWW) observation state, which matches the class of problem recorded in ADR-0077/0078. Revisit only if Hydro ships a stable release with an API-stability policy and a production-reachable Overdrive need appears.

## Research Methodology
**Search Strategy**: Official site and docs, GitHub repo/release/contributor APIs, crates.io and docs.rs metadata, arXiv papers, targeted web searches for independent adoption evidence; Overdrive grounding from repo docs.
**Source Selection**: Types: academic/official/technical_docs/industry | Reputation: high/medium-high; hydro.run treated as authoritative for its own feature claims only, maturity claims cross-checked against registries | Verification: cross-referenced where possible; note that the papers share authors with the project.
**Quality Standards**: Target 3 sources/claim, 2 acceptable, 1 authoritative minimum | Several claims are single-source and rated accordingly | Avg reputation ~0.85. Web fetches were summarised by a small model; treat quoted text as secondary to raw pages.

## Findings

### Finding 1: Maturity - every release is a 0.x alpha with breaking changes
**Evidence**: crates.io lists `hydro_lang` max_version `0.17.0-alpha.6` (2026-09-30); 16 versions since the crate was created 2024-12-23 (0.11.0 ... 0.16.0 on 2026-05-01, then 0.17.0-alpha.0..6 between 2026-06-10 and 2026-09-30). Total downloads 6,597; recent downloads 1,391. GitHub releases for alpha.5 carry "BREAKING" sections (e.g. `InitNone` boundedness variant added to `Optional`; "require matching boundedness to capture by_ref/by_mut handles"); alpha.6 adds a trailing `Scope` type parameter to every handle type. Raft source in-repo says of exported wire types: "not stable API."
**Source**: [crates.io API hydro_lang](https://crates.io/api/v1/crates/hydro_lang) ; [GitHub releases API](https://api.github.com/repos/hydro-project/hydro/releases) - Accessed 2026-11-06
**Confidence**: High (2 independent registries/mirrors of the same facts: crates.io and GitHub; both are first-party data of the project, but they are machine-generated, not marketing)
**Verification**: [hydro-project/hydro repo](https://github.com/hydro-project/hydro); docs home states "The docs for Hydro are still a work in progress" ([hydro.run reference](https://hydro.run/docs/hydro/reference/)).
**Analysis**: Rapid cadence (about 8 releases in 4 months) with type-level breaking changes between alphas. No stability policy was found (see Knowledge Gaps). Downloads of ~6.6k total indicate very small external usage.

### Finding 2: Licence is permissive; team is tiny and concentrated, now at AWS
**Evidence**: Repo licence Apache-2.0. Contributor commits: MingweiSamuel 1302, shadaj 779, jhellerstein 113, next ~86 and falling; two people account for ~2,080 of the visible contribution counts. Hellerstein's blog: he, Shadaj Laddad, Mingwei Samuel and Lucky Katahanas joined AWS and "continue advancing Hydro as an open-source initiative" with Berkeley and Princeton collaborators. hydro.run states Hydro is "co-led by Berkeley and AWS".
**Source**: [GitHub repo](https://github.com/hydro-project/hydro) ; [GitHub contributors API](https://api.github.com/repos/hydro-project/hydro/contributors) ; [Hellerstein blog (personal, medium trust)](https://jhellerstein.github.io/blog/hello-aws) - Accessed 2026-11-06
**Confidence**: Medium-High (contributor data is machine-generated; AWS move comes from the lead's own blog, corroborated by hydro.run wording)
**Verification**: [hydro.run](https://hydro.run)
**Analysis**: Bus factor is roughly two engineers plus a professor; employer-backed (AWS) lowers abandonment risk but gives no support commitment. Interpretation, not fact: the direction of the project is set by AWS/Berkeley priorities, not by third-party users.

### Finding 3: "Production Ready" is a self-description; no independent production deployment was found
**Evidence**: hydro.run heading "Research Backed. Production Ready." and "first production framework with location-oriented programming" (self-claim). GitHub page makes no maturity statement; Releases are alpha. Searches found no named external production deployment, case study, or AWS engineering write-up. A brightcoding.dev post describing a production KV case study uses APIs (`cargo hydro new`, `#[hydro::main]`) that do not match official docs and was discarded as unreliable.
**Source**: [hydro.run](https://hydro.run) ; [GitHub repo](https://github.com/hydro-project/hydro) - Accessed 2026-11-06
**Confidence**: Medium (absence of evidence from searches, not proof of absence)
**Analysis**: Treat "production ready" as unverified. See Knowledge Gaps.

### Finding 4: What Hydro provides (components)
**Evidence**: Repo crates: `hydro_lang`, `dfir_rs`, `dfir_lang`, `dfir_macro`, `dfir_pipes`, `hydro_deploy`, `hydro_std`, `hydro_test`, `lattices`, `variadics`, `sinktools`, `hydro_verus_tests`. hydro.run: location-typed streams (processes and clusters), stream types tracking ordering and retry semantics, code whose result may depend on timing or duplicate delivery rejected unless proven safe or marked `nondet!`. Suki paper (arXiv 2406.14733, Laddad/Cheung/Hellerstein): embedded Rust DSL for choreographed dataflow, staged programming, each location compiled to its own binary; abstract gives no evaluation data.
**Source**: [GitHub repo](https://github.com/hydro-project/hydro) ; [hydro.run](https://hydro.run) ; [arXiv 2406.14733](https://arxiv.org/abs/2406.14733) - Accessed 2026-11-06
**Confidence**: High for description, Low for performance claims ("zero-cost" is unevidenced in the abstract)
**Verification**: three independent-in-form sources (repo, docs, peer-reviewed-venue paper by same group; same-author caveat applies).

### Finding 5: Simulator exists, is deterministic, but explores Hydro-program schedules only
**Evidence**: Docs: "built-in deterministic environment"; runs "the same code you deploy"; `exhaustive()` explores all executions at `nondet!` points, coverage-guided fuzzing otherwise; failing schedule replayable. Support table: Networking Full; Batching/snapshotting Partial; unordered reductions Full for built-ins, arbitrary fold/reduce not accelerated; non-deterministic observations Limited (`assume_retries::<ExactlyOnce>` not supported). Docs do NOT mention message drop, crash/restart, partition faults, clocks or RNG injection, or arbitrary external Rust/async code. Code compiles into a dynamic library built at test time. Hydro's own Raft reports the simulator found a safety bug in an earlier design and requires canonicalised message ordering "for deterministic simulation".
**Source**: [Hydro simulation docs](https://hydro.run/docs/hydro/reference/simulation/) ; [docs.rs hydro_lang 0.16.0 sim source](https://docs.rs/crate/hydro_lang/0.16.0/source/src/sim/flow.rs) ; [hydro_test raft.rs](https://github.com/hydro-project/hydro/blob/main/hydro_test/src/cluster/raft.rs) - Accessed 2026-11-06
**Confidence**: Medium (docs fetch was summarised by an LLM; fault-model absence is "not found", verify in source)
**Analysis**: Simulator is seeded-free exhaustive/fuzz exploration of a *Hydro dataflow*, a different model from turmoil + Sim* adapters driving arbitrary Rust. It cannot drive `overdrive-core` reconcilers, redb, or aya code.

### Finding 6: In-repo Paxos / Raft / 2PC exist as test/benchmark programs, with in-memory state
**Evidence**: `hydro_test/src/cluster/` contains `paxos.rs`, `compartmentalized_paxos.rs`, `paxos_with_client.rs`, `raft.rs`, `two_pc.rs`, benchmarks. Raft (visible ~1000 of 2642 lines): leader election and log replication implemented per Raft paper sections 5.4.1/5.4.2; log is in-memory `Vec<LogEntry<T>>`; `RaftConfig::cluster_size` fixed and "Must match the deployed cluster size"; no membership change, snapshots, or disk persistence seen in the visible portion.
**Source**: [hydro_test cluster dir](https://github.com/hydro-project/hydro/tree/main/hydro_test/src/cluster) ; [raft.rs](https://github.com/hydro-project/hydro/blob/main/hydro_test/src/cluster/raft.rs) - Accessed 2026-11-06
**Confidence**: Medium (partial file read; single source)
**Analysis**: These live in `hydro_test`, a test crate, not a published library crate (not listed on crates.io among the shipped crates in the releases above: releases cover hydro_lang, hydro_std, hydro_deploy, dfir_rs, etc.). A consensus library for production use that lacks durable log, snapshots and reconfiguration cannot replace openraft+redb (which Overdrive needs for HA IntentStore).

### Finding 7: Runtime is Tokio-based; build uses staged macro codegen; durability is not addressed in DFIR/hydro_lang docs
**Evidence**: `hydro_lang` 0.16.0 normal dependencies include `tokio ^1.29`, `tokio-stream`, `tokio-util`, `tokio-metrics`, `stageleft ^0.13.5`, `syn`, `quote`, `bincode ^1.3`, `chrono`, `tracing`; build-dependencies `stageleft_tool`; optional `libloading`, `trybuild-internals-api`, `bolero-hydro`, `aws-sdk-ecs`. `dfir_rs` 0.16.0 likewise depends on tokio. Stageleft: `q!(...)` quoting; needs a companion macro crate and a `build.rs` calling `stageleft_tool`; repo has 28 stars, 21 open issues, pre-1.0. No docs page found (hydro.run, docs.rs) describing persistence, WAL, crash recovery or snapshots; the "State Management" and "Live Collections" reference URLs I tried returned 404 (the reference index lists "State Management" and "Atomic Collections" sections, contents not read).
**Source**: [docs.rs hydro_lang 0.16.0](https://docs.rs/crate/hydro_lang/0.16.0) ; [docs.rs dfir_rs](https://docs.rs/crate/dfir_rs/latest) ; [stageleft repo](https://github.com/hydro-project/stageleft) - Accessed 2026-11-06
**Confidence**: High for dependencies (machine-generated registry data); Low for "no durability" (absence in what I could read)
**Analysis**: Tokio is Overdrive's standard runtime, so no runtime clash. The staged-code build model, the dynamic-library simulator build and the Hydro-owned compile pipeline are the integration cost, not the runtime.

### Finding 8: Research foundations are real and peer-reviewed (ideas are strong even where the product is immature)
**Evidence**: (a) "Optimizing Distributed Protocols with Query Rewrites" (SIGMOD 2024; arXiv 2404.01593; Chu, Panchapakesan, Laddad, ..., Crooks, Hellerstein, Howard): rule-driven decoupling/partitioning, manual application gave ~5x on 2PC and ~3x on Paxos; failure model/durability assumptions not stated on the abstract page. (b) "Keep CALM and CRDT On" (VLDB 2023; arXiv 2210.12605): "CRDT guarantees extend only to data updates; observations of CRDT state are unconstrained and unsafe"; proposes a query model that uses CALM monotonicity to decide which reads are safe without coordination. (c) Flo (arXiv 2411.08274; hydro.run lists POPL 2025): identifies "streaming progress and eager execution" as shared properties; lightweight type system separating bounded from unbounded streams. (d) Suki (arXiv 2406.14733, CP 2024 per hydro.run): choreographic staged DSL, no evaluation on abstract.
**Source**: [arXiv 2404.01593](https://arxiv.org/abs/2404.01593) ; [arXiv 2210.12605](https://arxiv.org/abs/2210.12605) ; [arXiv 2411.08274](https://arxiv.org/abs/2411.08274) ; [arXiv 2406.14733](https://arxiv.org/abs/2406.14733) ; venue list from [hydro.run/research](https://hydro.run/research) - Accessed 2026-11-06
**Confidence**: High that the ideas exist and are published; Medium on venue attribution for Flo/Suki (taken from hydro.run, not independently confirmed at acm.org)
**Verification**: same research group authors on all four (independence limited); 2404.01593 includes external coauthor Heidi Howard (Cambridge).
**Analysis**: Source independence is limited: all are from the Hydro group. The CALM claim about queries over CRDT state is the one directly relevant to Overdrive's LWW observation layer.

### Finding 9: Overdrive's constraints (grounding, from repo docs)
**Evidence**: brief.md: `ObservationStore` is `LocalObservationStore` (redb, production LWW path in Phase 1) / `CorrosionStore` (Phase 2+) with `SimObservationStore`; DST uses `turmoil` pinned (`turmoil = "=0.6.X"`) with `Sim*` adapters; `overdrive-core` forbids tokio/rand/std::net (dst-lint). CLAUDE.md / development.md: reconcilers are pure-sync `reconcile(...)`; "Production code is not shaped by simulation"; workflows are journal-and-replay over redb with all nondeterminism via `ctx`; HA IntentStore is openraft+redb. brief.md also records ADR-0077/0078: a convergent LWW row cannot answer "did it happen", so crashes needed a separate durable counter/snapshot.
**Source**: repo files `docs/product/architecture/brief.md`, `.claude/rules/development.md`, `CLAUDE.md` (first-party, read 2026-11-06)
**Confidence**: High (primary project documents)
**Analysis**: Phase 1 is single-node, so the gossip and HA-consensus surfaces are not yet exercised in production; adoption pressure on those surfaces is future, not present.

## Per-Surface Fit Table

| Surface | Hydro offering | Fit | Verdict | Confidence |
|---|---|---|---|---|
| (a) Gossip / observation layer vs Corrosion (CR-SQLite) | `lattices` crate, CALM-typed dataflow, no SQLite/CR-SQLite-compatible store, no SWIM/gossip membership found | Different model; replacing Corrosion would mean building a store, schema migration and gossip on a dataflow engine. Ideas (monotone-read analysis) transfer | Reject as dependency; borrow CALM analysis | Medium |
| (b) Consensus / IntentStore vs openraft | `paxos.rs`, `compartmentalized_paxos.rs`, `raft.rs` in `hydro_test`; Raft log in-memory, fixed cluster size, no snapshots/membership seen in the ~1000/2642 lines read | Not a replacement: no durable log, no reconfiguration, test crate, unstable API | Reject | Medium (partial file read) |
| (c) Reconciler runtime / control loop | Dataflow with live collections, ticks | Overdrive reconcilers are pure-sync, level-triggered, runtime-owned CBOR View persistence, interest router; Hydro's incremental dataflow would duplicate and conflict with the `reconcile` contract and ADR-0035/0084 | Reject | Medium (judgement) |
| (d) Workflow engine | None found (no journal-and-replay, no durable timers) | Hydro is not a durable-execution engine; Overdrive has its own contract | Not applicable | Medium (absence in docs) |
| (e) DST / simulation vs turmoil | Built-in deterministic exhaustive/fuzz simulator over Hydro programs at `nondet!` points | Strong idea; but only for Hydro dataflow, fault model (drops/crash/restart) undocumented, no clock/RNG injection documented, cannot drive redb/aya/tokio code or Sim* adapters. Adopting it would add a second DST system beside turmoil and bit-identical-seed guarantees | Reject as replacement; optional curiosity only | Medium |
| (f) Deployment vs appliance OS | Hydro Deploy: localhost, GCP/Azure (docs), ECS optional dep; "alpha", ephemeral experiments, "not a replacement for Docker Compose or Kubernetes" | Irrelevant to immutable appliance OS with no SSH | Reject | High |
| (g) Dataplane (eBPF) | None | Irrelevant | N/A | High |

## Integration Costs (Question 3)
- **Tokio**: compatible (dependency is `tokio ^1`); not a blocker. Evidence: Finding 7.
- **core/host/sim split & dst-lint**: Hydro programs perform real I/O and use `chrono`/timers inside generated code; they could not live in `core`-class crates. They would have to be an `adapter-host`-class boundary with a bespoke sim equivalent, i.e. a second sim path. Interpretation based on Findings 7 and 9.
- **redb / rkyv persistence**: Hydro's serialization is bincode/serde; no durable-state API found. Persisted state would remain Overdrive's; Hydro would only be a compute engine. Gap noted.
- **Crash durability**: unknown from Hydro docs (Gap 3).
- **Dynamic membership**: Raft `cluster_size` is a fixed config; docs say simulator support for `Cluster` networking is "Full" but dynamic membership is not documented (Gap 4).
- **Observability**: `tracing`, `tokio-metrics` dependencies exist; Overdrive uses OpenTelemetry-style conventions; not evaluated further.
- **Build**: staged macros, companion crates, `build.rs` stageleft_tool, simulator compiled to dynamic library at test time. Adds compile time and a nonstandard build graph to a workspace already managed with xtask/dst-lint conventions. Magnitude unmeasured (Gap 5).

## Risks (Question 4)
1. **Research-project churn** (High): all releases alpha, type-level breaking changes in consecutive alphas (Finding 1).
2. **Bus factor** (Medium-High): ~2 engineers dominate commits; AWS employment gives funding signal but no commitment (Finding 2).
3. **No independent production evidence** (Medium-High): "Production Ready" is a self-claim (Finding 3).
4. **Debuggability/macro opacity** (Medium, inferred): staged codegen and dynamic-library simulator; not measured.
5. **Hiring/familiarity** (Medium, inferred): choreographic, lattice-typed dataflow is niche; no external survey found.
6. **Conflict with project rules** (High, inferred): "production code is not shaped by simulation" and the single turmoil harness; Hydro's simulator is only meaningful if the code is written in Hydro's model.

## Ideas worth borrowing (Question 5) - interpretation, labelled as such
1. **CALM / monotonicity audit of observation reads** (strongest). "Keep CALM and CRDT On" argues CRDT writes are safe but observations are not. Overdrive hit this exact class in ADR-0077/0078 (a converged LWW row cannot answer "did it happen"). A lightweight review checklist - "is this reconciler hydration read monotone in the observation lattice, or does it depend on absence/ordering?" - could be added to reconciler design review without any dependency. Confidence in applicability: Medium (analogy, not tested).
2. **Bounded/unbounded and ordering/retry types as a design vocabulary**: Flo's boundedness distinction and Hydro's `TotalOrder/NoOrder`, `ExactlyOnce/AtLeastOnce` markers map onto Overdrive's workflow `ctx.run` replay vs at-least-once `HttpCall` rules. Use as vocabulary in ADRs; do not import types. Confidence: Low-Medium.
3. **Location types**: Overdrive already encodes locality through port traits and node-local vs gossiped rows; a newtype-level "this row is node-local authoritative" marker could echo it. Speculative; Low.
4. **Exhaustive exploration at declared nondeterminism points** (`nondet!` with explanation strings): comparable to Overdrive's Quint+quint-connect + turmoil lanes. Nothing to adopt; confirms the design direction. Per project rule "agents propose, the user adds", any new test lane would need a separate proposal.
5. **Query-rewrite scaling (decouple/partition)** from SIGMOD'24: relevant only if Overdrive's Raft or reconciler throughput becomes a bottleneck; today (Phase 1 single node) no reachable production failure motivates it.

## Recommendation
**Borrow ideas only. Do not adopt Hydro as a dependency in any Overdrive surface now.** Revisit only if (i) Hydro publishes a stable (non-alpha) release with a documented API-stability policy, and (ii) a concrete, production-reachable need emerges that openraft/Corrosion/turmoil cannot meet. The immature alpha API, absent durability story, in-memory test-crate consensus, and a simulator that only covers Hydro programs make adoption incompatible with Overdrive's DST, persistence and "Tokio, ports-in-core" disciplines.

## Source Analysis
| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| hydro_lang on crates.io API | crates.io | High (1.0) | registry | 2026-11-06 | Y (GitHub releases) |
| GitHub releases API | api.github.com | Medium-High (0.8) | industry | 2026-11-06 | Y |
| GitHub contributors API | api.github.com | Medium-High (0.8) | industry | 2026-11-06 | N |
| hydro-project/hydro repo | github.com | Medium-High (0.8) | industry/primary | 2026-11-06 | Y |
| hydro.run home | hydro.run | Official-for-own-claims (treated 1.0 for features, 0.6 for maturity claims) | official | 2026-11-06 | partial |
| hydro.run simulation docs | hydro.run | as above | official | 2026-11-06 | Y (docs.rs source) |
| hydro.run research page | hydro.run | as above | official | 2026-11-06 | Y (arXiv) |
| docs.rs hydro_lang / dfir_rs | docs.rs | High | technical | 2026-11-06 | Y |
| hydro_test raft.rs / cluster dir | github.com | Medium-High | primary code | 2026-11-06 | N |
| stageleft repo | github.com | Medium-High | primary | 2026-11-06 | N |
| arXiv 2404.01593, 2210.12605, 2411.08274, 2406.14733 | arxiv.org | High (1.0) | academic | 2026-11-06 | Partial (same group) |
| Hellerstein blog | jhellerstein.github.io | Not in trusted list; medium, used only for team/AWS fact | personal | 2026-11-06 | Y (hydro.run "co-led by AWS") |
| lib.rs listing (via search snippet) | lib.rs | Not in trusted list; used only as corroboration | aggregator | 2026-11-06 | Y |
| brightcoding.dev post | brightcoding.dev | Rejected (APIs inconsistent with official docs) | blog | 2026-11-06 | - |
| Repo docs (brief.md, development.md, CLAUDE.md) | local | High (primary for Overdrive) | internal | 2026-11-06 | - |

Reputation: High ~35%, Medium-high ~45%, other ~20%. Avg ~0.85. Several claims rest on a single group (Hydro authors); independence is limited and flagged.

## Knowledge Gaps
### Gap 1: API-stability policy / semver promise
**Issue**: No document found stating stability guarantees. **Attempted**: GitHub repo page, hydro.run home and reference, release notes. **Recommendation**: Ask the maintainers or read CONTRIBUTING.md/CHANGELOGs directly.
### Gap 2: Independent production deployments
**Issue**: None found. **Attempted**: two web searches, repo, docs. **Recommendation**: Query the Hydro Slack/Discord, AWS Rust/re:Invent talks, GitHub dependents list.
### Gap 3: Durable state / crash recovery in DFIR and hydro_lang
**Issue**: Neither docs nor read source mention WAL, snapshots or restart recovery; `State Management` and `Live Collections` doc URLs returned 404. **Attempted**: docs.rs, hydro.run, one web search. **Recommendation**: Read hydro_lang source for `persist`/state and the Berkeley dissertations (EECS-2025-85, EECS-2025-103) - not read here.
### Gap 4: Dynamic membership and simulator fault model
**Issue**: Simulator docs omit drops, crashes, restarts, partitions, clock/RNG injection; Raft membership change not seen. **Attempted**: simulation docs, raft.rs (partial, ~1000 of 2642 lines). **Recommendation**: Read `hydro_lang/src/sim` and the remaining raft.rs.
### Gap 5: Compile-time and debuggability cost
**Issue**: No measurement. **Recommendation**: A throwaway spike under `spike-scratch/` per `.claude/rules/spike.md` if the user wants numbers (proposal only; not requested).
### Gap 6: Licence beyond repo
**Issue**: Apache-2.0 confirmed for repo, stageleft and crate versions; third-party dependency licence audit not done.
### Gap 7: Venue verification
**Issue**: Flo (POPL 2025) and Suki (CP 2024) venues taken from hydro.run only. **Recommendation**: Check dl.acm.org.
### Gap 8: Unread fetch tails
**Issue**: GitHub releases fetch truncated at 100k of 127k chars; verus test crate purpose unknown; web fetches were LLM-summarised, so quotes should be re-verified against raw pages before being relied on.

## Conflicting Information
### Conflict 1: Hydro Deploy targets
**Position A**: Deploy docs page lists Localhost, GCP, Azure and calls it alpha - Source: hydro.run deploy reference, Evidence: "alpha, documentation incomplete".
**Position B**: `hydro_lang` has optional `aws-sdk-ecs`/`bollard` dependencies and hydro.run describes AWS cloud integrations - Source: docs.rs hydro_lang, hydro.run home.
**Assessment**: Deploy page is probably stale relative to code; irrelevant to the recommendation. Unresolved.
### Conflict 2: "Production Ready" vs alpha releases
**Position A**: hydro.run "Research Backed. Production Ready." **Position B**: all releases are `-alpha`/0.x with breaking changes (crates.io, GitHub). **Assessment**: Release metadata is more objective evidence; marketing claim treated as unverified.

## Recommendations for Further Research
1. If revisiting, read the two 2025 Berkeley dissertations for the durable-state and failure-model story.
2. Examine `hydro_lang/src/sim` to determine whether message loss/crash can be modelled.
3. Evaluate a CALM-style read-monotonicity review checklist against existing reconcilers (needs user approval per design rules).

## Full Citations
[1] hydro-project. "Hydro (GitHub repository)". github.com. https://github.com/hydro-project/hydro. Accessed 2026-11-06.
[2] Hydro project. "Hydro: Research Backed. Production Ready." https://hydro.run. Accessed 2026-11-06.
[3] crates.io. "hydro_lang crate metadata". https://crates.io/api/v1/crates/hydro_lang. Accessed 2026-11-06.
[4] GitHub. "hydro-project/hydro releases". https://api.github.com/repos/hydro-project/hydro/releases. Accessed 2026-11-06.
[5] GitHub. "hydro-project/hydro contributors". https://api.github.com/repos/hydro-project/hydro/contributors. Accessed 2026-11-06.
[6] Hydro project. "Simulation Testing". https://hydro.run/docs/hydro/reference/simulation/. Accessed 2026-11-06.
[7] Hydro project. "Research publications". https://hydro.run/research. Accessed 2026-11-06.
[8] Hydro project. "Hydro Deploy". https://hydro.run/docs/hydro/reference/deploy/. Accessed 2026-11-06.
[9] docs.rs. "hydro_lang 0.16.0"; "dfir_rs". https://docs.rs/crate/hydro_lang/0.16.0 ; https://docs.rs/crate/dfir_rs/latest. Accessed 2026-11-06.
[10] hydro-project. "hydro_test/src/cluster (paxos, raft, two_pc)". https://github.com/hydro-project/hydro/tree/main/hydro_test/src/cluster ; raft.rs. Accessed 2026-11-06.
[11] hydro-project. "stageleft". https://github.com/hydro-project/stageleft. Accessed 2026-11-06.
[12] Laddad, Cheung, Hellerstein. "Suki: Choreographed Distributed Dataflow in Rust". arXiv:2406.14733. 2024. Accessed 2026-11-06.
[13] Chu et al. "Optimizing Distributed Protocols with Query Rewrites". arXiv:2404.01593 (SIGMOD 2024). Accessed 2026-11-06.
[14] Laddad et al. "Flo: a Semantic Foundation for Progressive Stream Processing". arXiv:2411.08274. 2024. Accessed 2026-11-06.
[15] Laddad et al. "Keep CALM and CRDT On". arXiv:2210.12605 (VLDB 2023). Accessed 2026-11-06.
[16] Hellerstein. "Schema Evolution, Career Edition". https://jhellerstein.github.io/blog/hello-aws. Accessed 2026-11-06 (non-listed domain; team fact only).
[17] Overdrive repo: docs/product/architecture/brief.md; .claude/rules/development.md; CLAUDE.md. Read 2026-11-06.

## Research Metadata
Examined ~30 pages | Cited 17 | Tool failures: 3 hydro.run doc URLs returned 404 (safety, state, live-collections) | Confidence: High on maturity/licence, Medium on per-surface fit, Low on durability | Output: docs/research/platform/hydro-framework-fit-comprehensive-research.md
