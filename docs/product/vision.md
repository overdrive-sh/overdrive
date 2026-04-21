# Overdrive — Product Vision

**Source of truth**: `docs/whitepaper.md` (platform design) and `docs/commercial.md` (tenancy, tiers, licensing). This document distils both for product-wave use. When they disagree, the whitepaper and commercial doc win; amend this file.

---

## One-sentence vision

A next-generation workload orchestration platform, written entirely in Rust, that collapses the Kubernetes/etcd/CNI/Envoy/SPIRE/Prometheus stack into one binary — designed for 2026 primitives (stable eBPF, kTLS, WASM, SPIFFE) rather than retrofitted onto 2014 ones.

## Why now

Between 2023 and 2025 a specific set of primitives reached production maturity simultaneously: `aya-rs`, `openraft`, `wasmtime`, `rustls` with kTLS offload, `redb`, BPF LSM, and `cr-sqlite` + Corrosion. Overdrive is the platform that becomes possible when all of these exist at the same time. Kubernetes cannot be rebuilt on these primitives incrementally; a clean-slate design can.

## Design principles (from whitepaper §2 — exact list, condensed)

1. **Own your primitives.** No etcd, Envoy, SPIRE, CNI. Every critical subsystem is built in or is a standard Rust library.
2. **eBPF is the dataplane.** All policy, LB, routing, telemetry, mTLS happens in-kernel. No userspace proxies in the data path.
3. **Security is structural, not configurable.** mTLS is default and cannot be disabled. Every packet carries cryptographic workload identity.
4. **All workload types are first class.** VMs, processes, unikernels, containers, WASM — one control plane, one identity model.
5. **Observability is native.** eBPF gives kernel-level visibility with full identity from day one.
6. **The platform learns.** Self-healing and right-sizing compound with operational age.
7. **Rust throughout.** No FFI to Go or C++ in the critical path.
8. **One binary, any topology.** Role declared at bootstrap, not at build time.
9. **Strong consistency where it matters, gossip where it scales.** Intent through Raft, observation through Corrosion CRDT.

## Who this is for

- **Tier 1 — Managed Overdrive**: platform engineering teams building internal developer platforms; companies migrating off self-managed Kubernetes.
- **Tier 2 — Managed Workloads**: engineering teams that want Nomad-style job submission without operating infrastructure.
- **Tier 3 — Serverless WASM**: developers who want Lambda economics without AWS lock-in; AI agent workloads with egress control requirements.
- **Tier 4 — Bare Metal Dedicated**: ML training, HPC, latency-sensitive workloads; compliance estates needing hardware isolation.
- **Enterprise self-hosted** (regulated industries): financial services, government, healthcare, defence — cannot use cloud platform; need FIPS, HSM, air-gap, compliance packs.

## How Overdrive makes money (commercial flywheel)

- **Cloud platform**: absorb operational complexity, charge per vCPU-hour / GB-hour / invocation.
- **Enterprise self-hosted licence**: FIPS, HSM, air-gap, compliance packs, SLA-backed support.
- **Source-available flywheel**: FSL-1.1-ALv2, converts to Apache 2.0 at two-year anniversary. Community drives adoption; Competing Use restriction prevents hyperscaler commodity competition.

The core insight: **the business is not the software; the business is the operational complexity the software absorbs.**

## Strategic product pillars that depend on correctness-from-day-one

Three pillars of the commercial model rest on technical properties this vision commits to from day one:

1. **Control plane density (commercial.md — "Control Plane Density")**: `LocalStore` in ~30MB RAM makes 1,000 small tenants viable on a single platform node (30GB total vs 24TB for HA everywhere). This requires `IntentStore` abstraction + redb-direct `LocalStore` implementation + non-destructive `export_snapshot` / `bootstrap_from` migration.
2. **Kernel-level precision billing (commercial.md — "Billing Infrastructure")**: eBPF telemetry with cryptographic identity means billing is exact, not estimated. Disputes have kernel-level evidence. This requires SPIFFE identity in every flow event.
3. **Operational memory as retention (commercial.md — "LLM Self-Healing as Retention")**: incident memory, resource profiles, and LLM reasoning compound over time. Switching cost is operational memory, not contractual lock-in. This requires incident-memory libSQL + DST-provable SRE-agent determinism.

## What a Phase 1 product release proves

Phase 1 is the **walking skeleton** of the platform. It proves the foundation is buildable on the claimed testing discipline:

- Every source of nondeterminism is injectable behind a trait.
- The intent/observation consistency boundary is load-bearing and structurally enforced (separate traits, separate types, invariant-testable).
- `LocalStore` works and supports non-destructive HA migration via `export_snapshot` / `bootstrap_from`.
- The turmoil DST harness runs green with core invariants (`single leader`, `intent never crosses into observation`, ESR convergence, replay determinism).
- A CI lint gate blocks `Instant::now()` / `rand::random()` / raw `tokio::net::*` in core crates.

If Phase 1 passes, the §21 DST claim is real, not performative. If it doesn't, every later phase is building on sand.

## Non-goals for Phase 1

- No gRPC API, no reconciler runtime, no job-lifecycle reconciler (→ "convergence engine" feature).
- No process/microvm/WASM drivers, no scheduler, no cgroup isolation (→ "execution layer" feature).
- No CLI, no Garage (→ separate features).
- No eBPF code (→ Phase 2+, gated on Tier 2–4 real-kernel testing in `.claude/rules/testing.md`).

## Success definition

Three orthogonal tests:

1. **Technical**: `cargo xtask dst` runs green on a clean clone, with a seeded harness that exercises every `Sim*` trait pair against the real `LocalStore`. Invariants pass. Seed reproduces bit-for-bit on failure.
2. **Commercial**: `LocalStore` is shown to run a full control plane within the whitepaper-claimed ~30MB RAM envelope with cold start under 50ms, and the snapshot round-trip is bit-identical.
3. **Process**: the CI lint gate catches a deliberate `Instant::now()` smuggled into `overdrive-core`, with a clear message pointing at `development.md`.

---

## Changelog

| Date | Change |
|---|---|
| 2026-04-21 | Distilled from whitepaper §2 + commercial.md for phase-1-foundation DISCUSS wave. |
