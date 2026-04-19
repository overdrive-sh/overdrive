# Helios Whitepaper Review

**Date:** 2026-04-19
**Reviewer:** nw-system-designer agent
**SSOT reviewed:** `docs/whitepaper.md` (Version 0.12 — Draft)
**Branch:** `marcus-sa/helios-image-factory`

---

## Executive Summary

The whitepaper is unusually well-constructed for a pre-implementation design — the Intent/Observation split, the sidecar/policy distinction, the Earned Trust posture in DST, and the disciplined use of existing Rust primitives are all architecturally mature. The gaps are concentrated in three areas: **operational boundary cases** (bootstrap, recovery, key loss), **underspecified contracts between subsystems**, and **a Phase 1 that's too broad to execute coherently**.

Severity ratings: **Blocker** / **Important** / **Nice-to-have**.

---

## 1. Is there anything missing to commence work?

### Blockers

**B1. Bootstrap sequence is not specified end-to-end.**
§4 mentions "role declared at bootstrap" and §23 discusses node images, but nowhere is the first-node bootstrap defined:
- How does node 0 obtain its intermediate CA cert before the CA exists? (Circular: §8 says "Node Intermediate CA issued at bootstrap," but the CA lives in the IntentStore which needs a node to run.)
- In HA mode, how do the first three nodes discover each other before Corrosion is up? The `global_bootstrap` seed (§4.5) is referenced but its contract is undefined.
- §7 *Node Underlay* says "the initial peer set arrives with the SVID before any Corrosion connection is attempted" — but the SVID issuance path depends on the CA, which depends on an IntentStore, which depends on... This needs a step-by-step boot sequence diagram.

**B2. CA key lifecycle is under-addressed.**
§4 says "root CA key lives in the IntentStore, encrypted at rest." Missing:
- Encryption-key-at-rest: where does *that* key live? TPM? Operator-provided envelope? KMS?
- Root CA rotation procedure — short TTLs on leaf SVIDs solve revocation, but what happens when the root itself needs rotating (compromise, 10-year expiry)?
- Disaster recovery: if all three HA nodes are destroyed, what's the restore path for the CA? Garage snapshot (§17) holds it — but the decryption key?

**B3. Error-handling and logging conventions unstated.**
No mention of:
- Error type strategy (thiserror? snafu? custom?)
- Structured logging (tracing is implied by rustls ecosystem but never named)
- Panic policy in the node agent (abort vs unwind; critical for eBPF map consistency)
- How `health.startup.refused` events (Earned Trust principle) propagate — are they log lines, Corrosion rows, or control-plane RPCs?

### Important

**I1. Core trait contracts are declared but not complete.**
- `Driver` trait is shown in §6 but lacks: `logs()`, `exec()` for debugging, `events()` stream. How does the LLM agent tool `get_job_status` (§12) map to this?
- `IntentStore::watch` returns `WatchStream` — type unspecified. Does it deliver events at-least-once? Exactly-once? Compacted?
- `ObservationStore::subscribe` returns `RowStream` — delivery semantics, backpressure, reconnection behavior unstated.
- `Reconciler::reconcile` returns `Vec<Action>` but `Action` is never defined. This is the central coordination type.

**I2. The "single binary, any topology" claim has no build specification.**
§2 principle 8 commits to one binary. But:
- What's the Cargo workspace layout? Single crate? Workspace with feature flags?
- How are `Gateway`, `ControlPlane`, `NodeAgent` optionally linked without bloating the single-node binary?
- Are dev/prod builds the same? (§21 says `rand` and `std::time` are "direct dependencies only in platform wiring crates" — implying multi-crate, but layout unstated.)

**I3. External library maturity is asserted, not validated.**
§1 names `aya-rs`, `openraft`, `wasmtime`, `rustls`-kTLS, `redb`, `cr-sqlite`, `instant-acme`, `hyper`, `tonic`, `rig-rs`, `regorus`, `turmoil` as production-ready. No dependency matrix shows versions, MSRV alignment, or known incompatibilities. `rig-rs` in particular is newer and less battle-tested than the rest.

**I4. Testing approach beyond DST is absent.**
§21 covers DST thoroughly. Missing: unit test convention, integration test strategy (real eBPF on kernel), kernel-compat test matrix, chaos testing infrastructure beyond the reconciler, load testing methodology, fuzzing posture.

### Nice-to-have

**N1.** No cross-referenced glossary — "SVID," "intermediate CA," "compiled verdict," "allocation handle" are defined in their home sections but referenced widely.
**N2.** No component ownership table (who owns each subsystem — useful for a team starting work).
**N3.** Metrics/observability conventions for the platform *about itself* (the control plane's own Prometheus-style metrics, not workload telemetry).

---

## 2. Does the roadmap make sense?

### Blockers

**B4. Phase 1 is too broad and has internal ordering problems.**
Phase 1 bundles: data model, gRPC API, IntentStore (LocalStore), ObservationStore abstraction, 6 injectable traits, turmoil harness, SimDriver, SimDataplane, process driver, basic scheduler, CLI, **and** Image Factory MVP (Yocto layer + Rust service). This is 3–6 months of work for a team, not a phase. Splitting it:
- Phase 1a: trait skeleton + sim harness + process driver + CLI (validates DST from day one)
- Phase 1b: Image Factory (orthogonal deliverable, own stream)

**B5. Hidden dependency: Phase 2's Corrosion retires a Phase 1 mechanism.**
"BPF map hydration via Corrosion subscriptions (retires the gRPC push path for dataplane state)" — but Phase 1 includes no gRPC push path. Either Phase 1 specifies a stopgap (which means dead code in Phase 2) or the push path doesn't exist and Phase 2 is the first map-hydration pathway. Reorder: don't build the stopgap.

**B6. Phase 3 ordering problem — kTLS depends on Phase 2 XDP/TC work but also on CA from Phase 3 itself.**
The dependency chain: kTLS needs SVIDs; SVIDs need the built-in CA. The CA is Phase 3, but §4 places CA storage in the IntentStore which is Phase 1. Workable, but the sequence (CA → SVID issuance → sockops → kTLS) should be explicit — currently one bullet.

### Important

**I5. Phase 4 bundles too much.**
Cloud Hypervisor driver, WASM driver, full sidecar runtime, 4 built-in sidecars, Gateway, embedded ACME, DuckLake pipeline, mesh VPN underlay extensions. Any one of these is a multi-week deliverable. Particularly coupled: the Gateway depends on the WASM sidecar runtime, which depends on the Wasmtime driver. This wants to be Phase 4a/4b.

**I6. Phase 5's "persistent microVMs" four-step path hides `helios-fs`.**
Step 2 is "`helios-fs` — Rust-native single-writer chunk store." This is a substantial filesystem implementation: FastCDC, per-rootfs libSQL, WAL streaming to Garage, NVMe 2Q cache, `vhost-user-fs` frontend. Calling this a "step" within Phase 5 understates it. It is arguably Phase 4.5 on its own.

**I7. Phase 1's "basic scheduler (first-fit)" doesn't match the ambition of §14 right-sizing or §4 scheduler that consults `node_health`.**
The Phase 5 right-sizing reconciler "feeds back to the scheduler" — but if the scheduler is first-fit it has no place to feed back into. The scheduler's interface needs to be right from Phase 1 even if the policy is trivial.

**I8. Multi-region federation is Phase 6 but the Corrosion adoption in Phase 2 already designs for it.**
This is fine, but the "thin global membership cluster" (§4.5) needs a Phase 2 design decision (does the single-region Corrosion peer know it's potentially regional?) or Phase 6 is a schema migration risk.

### Nice-to-have

**N4.** No explicit "Phase 0" for build infrastructure, CI, cargo workspace shape, license headers, commit conventions.
**N5.** No roadmap item for documentation site/ADR process.
**N6.** LLM agent (Phase 5) is listed after right-sizing — reasonable, but the LLM is shown as integral to tier-3 self-healing throughout. Consider: is there a "placeholder tier-3" in earlier phases?

---

## 3. What do we start with?

The smallest end-to-end vertical slice that validates the architecture:

**Recommended first concrete step: the "dry process" vertical.**

A single-node Helios binary that:
1. Accepts a minimal `JobSpec` via gRPC (driver = `process`, a command, resources).
2. Persists it through `IntentStore` (LocalStore, redb).
3. Runs the basic scheduler (pick this node — there is only one).
4. Writes `alloc_status` into `ObservationStore` (a plain in-memory LWW map at this stage — not Corrosion).
5. Uses `SimClock` and `SimTransport` in tests; real clock/transport in the binary.
6. Executes via a `Driver::start()` implementation that spawns via `tokio::process` with cgroup v2 placement.
7. The CLI `helios job submit` and `helios alloc status` round-trips.
8. A turmoil test asserts: job submitted → allocation eventually running → assertion `desired == actual`.

This validates, in one PR:
- Trait boundaries (IntentStore, ObservationStore, Clock, Transport, Driver) are correctly shaped
- DST harness works
- The "single binary, role at bootstrap" claim holds
- Reconciler memory pattern works (even if trivial)
- The gRPC/CLI/control-plane contract is coherent

**The first PR should deliberately exclude**: eBPF, CA, mTLS, multi-node, Raft, Corrosion, Gateway, Image Factory, WASM. Each of those is its own vertical slice layered on this spine.

**Pre-step (Phase 0, before the first PR):**
- Cargo workspace skeleton (`helios-core`, `helios-node`, `helios-cli`, `helios-sim`, `helios-api`)
- CI with clippy, rustfmt, `cargo deny`, MSRV pin
- An ADR process under `docs/product/architecture/`
- A dependency audit of the "production-stable" libraries with pinned versions

---

## 4. Are there holes in the architecture?

### Blockers

**H1. Control-plane-on-worker resource isolation is claimed but untested against realistic adversaries.**
§4 says "misbehaving workload cannot starve the IntentStore… regardless of how aggressively it consumes CPU or memory" via cgroup reservations. But:
- PID pressure, inotify pressure, memory bandwidth, NUMA effects, and FD exhaustion are not cgroup-isolated by default.
- A workload doing `fork()` bombs or filesystem metadata floods can degrade a co-located control plane.
- This claim is load-bearing for the three-node all-in-one deployment shape and needs its own residuality analysis.

**H2. The "Corrosion scales to continents" claim is based on one operator.**
§1 says "production-proven at Fly.io's global scale." §12 asserts it subsumes SWIM membership. But:
- No back-of-envelope math: N nodes × M rows/node × average row size × gossip fanout × rows/sec = gossip bandwidth. At 10K nodes globally with 20 allocs each, what's the steady-state gossip bandwidth per node? Is it sane on a 1 Gbps uplink?
- Fly's published Corrosion workload is known; Helios may have different write amplification (every allocation state transition writes a full row per I5 in §4).
- "Per-region blast radius" is asserted but no sizing guidance: at what region size does a new region become necessary?

**H3. The LLM agent's approval gate is underspecified.**
§12 mentions a "graduated approval gate based on action risk level." §13 shows LLM-generated policies with "operator reviews generated source and approves." But:
- Who reviews? How is the operator notified? What's the UX?
- What happens if no operator is available — timeout behavior?
- Risk levels are unnamed and their classification criteria unstated.
- Can the LLM auto-approve low-risk actions? If yes, what's the audit trail?
- This is a load-bearing safety boundary and it's one paragraph.

**H4. Scheduler contract with `node_health` is ambiguous.**
§4 says the scheduler reads `node_health` to bin-pack. But:
- Is this a SQL query per scheduling decision, or a cached view?
- What's the staleness tolerance? A node marked healthy 30 seconds ago but actually down — does the scheduler place work there?
- `node_health` is eventually consistent. Scheduler decisions are intent (linearizable). How does the scheduler prevent double-scheduling on stale capacity data across region Raft leaders?
- This is a fundamental CAP boundary the design straddles, and the boundary is not drawn.

**H5. No disaster recovery story.**
Every component has a happy path. Missing:
- How is a destroyed HA cluster rebuilt from Garage backups?
- How is a corrupted Raft log recovered?
- How is a CR-SQLite database with garbage LWW state quarantined?
- What's the RTO/RPO target the design is engineered against?

### Important

**H6. Earned Trust (user's principle 9) is stated but per-component probes are not enumerated.**
The principle calls for startup probes per component. The whitepaper mentions no probes explicitly. Minimum set that should be specified:
- redb: fsync durability probe (on overlayfs this is a known lie)
- Corrosion: gossip round-trip probe before declaring healthy
- Raft: quorum commit durability probe
- kTLS: probe that crypto offload is genuinely in the kernel path, not silently falling back to userspace
- Clock: NTP sanity probe before Raft leader election

**H7. Sidecar/WASM performance claim is hand-waved.**
§9 claims "~microseconds for simple logic" for warm WASM instances. But sidecars run in the TC eBPF path on every request. Expected steady-state overhead at 10K RPS across a 4-sidecar chain is not estimated. The instance pool sizing is mentioned once, not dimensioned.

**H8. `helios-fs` single-writer enforcement is mechanism-free.**
§17 says "enforced by the allocation lifecycle." But two reconcilers, two leaders (split-brain in Raft recovery), or a stale node that missed a migration could open the same rootfs twice. The actual mutex needs to be specified (advisory lock in libSQL? lease via Raft?).

**H9. Gateway is SPOF on nodes with `gateway.enabled = true`.**
No mention of gateway HA — if the gateway node dies, external traffic dies. ExternalDNS? BGP? Keepalived? L4 load balancer in front? The design punts on the answer.

**H10. Request replay loop prevention is correct but the bounded buffer policy is under-designed.**
§11 says ≤1 MB and 503 on overflow. Common cases it punts on: file uploads, streaming uploads, multipart forms. Most real applications have requests >1 MB. The 503 path is not production-ready for a primary ingress.

**H11. Policy compilation path has no formal model for staleness.**
§10: policy compiled at control plane → verdicts written to Corrosion → gossiped → materialized into BPF maps. Gossip is seconds. A newly-denied connection may succeed for N seconds after policy change. This is fine — but the bound on N is not stated, and for security-critical policies (revoking a compromised workload) this bound matters.

### Nice-to-have

**H12.** Cross-region service discovery's latency math is missing. Tokyo gateway resolving us-east-1 backends — how does routing weighting handle the 150ms round-trip? Is there locality preference beyond `helios-prefer-region`?
**H13.** OCI image upgrade (§23 Phase 2) has no rollback path — what if the new image fails to boot? A/B partitions implied in `wic` layout but not stated as a requirement.
**H14.** The "full rows over field diffs" guardrail (§4) is correct but creates write amplification. Back-of-envelope on how this scales with N allocations × transition rate is not provided.
**H15.** §20's efficiency numbers are "directional estimates" — explicit methodology needed for the ones most likely to be cited (2.3x density, 100x mTLS CPU).
**H16.** No spec on how Regorus evaluation is bounded — a pathological Rego policy can loop. Is there a time budget? Memory budget?

---

## Priority Summary

**Must resolve before Phase 1 starts (Blockers):**
B1 (bootstrap sequence), B2 (CA key lifecycle), B3 (error/logging conventions), B4 (Phase 1 scope split), B5 (Phase 2 dead-code avoidance), B6 (Phase 3 internal order), H1 (co-location isolation claims), H2 (Corrosion scaling math), H3 (LLM approval gate), H4 (scheduler/node_health contract), H5 (DR story).

**Should resolve during Phase 1 (Important):**
I1–I8, H6–H11.

**Can defer (Nice-to-have):**
N1–N6, H12–H16.

The design is strong enough that these are refinements, not rewrites. The single most valuable next action is **defining the Phase 0 workspace + Phase 1a vertical slice** explicitly, which forces most of the Blocker-level contract questions to get answered by code rather than prose.
