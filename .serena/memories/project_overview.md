# Overdrive — Project Overview

**Platform**: Rust-native workload orchestration platform (replaces Kubernetes/Nomad/Talos).
**License**: FSL-1.1-ALv2 (source-available; Apache 2.0 after 2 years).
**Repo URL**: https://github.com/overdrive-sh/overdrive
**Rust edition**: 2024, MSRV 1.85

## Core concept
Single binary; role declared at bootstrap (`control-plane`, `worker`, `control-plane+worker`).
Unifies VMs, processes, unikernels, WASM under one control plane with eBPF dataplane, mTLS, BPF LSM.

## Architecture style
Hexagonal (ports & adapters). Ports are `trait` objects in `overdrive-core`. Adapters are structs in adapter crates. All I/O behind injectable traits for DST.

## Crate topology (Phase 1)
- `crates/overdrive-core` — class=core; ports (traits), newtypes, errors. NO I/O, NO tokio/rand/std::net.
- `crates/overdrive-control-plane` — class=adapter-host; axum router, reconciler runtime, eval broker, TLS bootstrap.
- `crates/overdrive-store-local` — class=adapter-host; LocalStore (redb), LocalObservationStore.
- `crates/overdrive-sim` — class=adapter-sim; Sim* adapters, turmoil DST harness, invariants.
- `crates/overdrive-cli` — class=binary; `overdrive` CLI, eyre error handling, reqwest HTTP client.
- `crates/overdrive-host` — class=adapter-host; OS/kernel bindings (SystemClock, OsEntropy, TcpTransport).
- `xtask/` — class=binary; build/lint/DST runner.

## Key design patterns
- Intent/Observation split: `IntentStore` (linearizable, redb/Raft) vs `ObservationStore` (eventually-consistent, CR-SQLite/Corrosion).
- Reconciler primitive: `hydrate` (async, libSQL) + `reconcile` (sync, pure).
- Newtypes STRICT for all domain IDs: `JobId`, `AllocationId`, `NodeId`, `SpiffeId`, etc.
- `HashMap` banned in core unless `// dst-lint: hashmap-ok <reason>`.
- `BTreeMap` default for keyed maps in core (DST determinism).
