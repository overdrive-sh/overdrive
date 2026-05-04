# C4 Diagrams — Overdrive

This file collects per-phase / per-feature C4 diagrams referenced from
`brief.md`, ADRs, and per-feature DESIGN docs. Each section is a
snapshot of the system at the close of a feature; superseded sections
remain for traceability.

---

## Phase 2.1 — eBPF Dataplane Containers

**Source:** `docs/feature/phase-2-aya-rs-scaffolding/design/architecture.md`
**ADR:** ADR-0038
**Date:** 2026-05-04

### C4 Level 1 — System Context

```mermaid
C4Context
  title System Context — Overdrive (Phase 2.1 — eBPF dataplane scaffolding)

  Person(engineer, "Platform Engineer (Ana)", "Writes core-plane logic; runs `cargo xtask dst` and `cargo xtask bpf-build`")
  System(overdrive, "Overdrive node", "Single binary — control plane + worker + dataplane (Phase 2.1: dataplane crate scaffolded with no-op XDP)")
  System_Ext(kernel, "Linux kernel", "BPF subsystem — XDP/TC hooks, BPF maps, BPF_PROG_TEST_RUN syscall (kernels 5.10+ supported)")
  System_Ext(ci, "CI", "GitHub Actions — runs xtask gates incl. `bpf-build` + `bpf-unit` + `integration-test vm`")
  System_Ext(fs, "Local filesystem (redb + cgroupfs)", "redb for IntentStore + LocalObservationStore; /sys/fs/cgroup for workload isolation")

  Rel(engineer, overdrive, "Runs `cargo xtask bpf-build` + `bpf-unit`; `cargo xtask integration-test vm latest` via Lima")
  Rel(engineer, ci, "Pushes PRs to")
  Rel(ci, overdrive, "Runs `cargo xtask {dst, dst-lint, bpf-build, bpf-unit, integration-test vm}`")
  Rel(overdrive, kernel, "Loads BPF programs; reads/writes BPF maps; receives kernel events", "via aya / bpf(2) syscalls")
  Rel(overdrive, fs, "Persists intent + observation to")
```

### C4 Level 2 — Container

```mermaid
C4Container
  title Container Diagram — Overdrive (post Phase 2.1 — eBPF scaffolding)

  Person(engineer, "Platform Engineer (Ana)")

  Container_Boundary(workspace, "Overdrive workspace (10 crates + xtask)") {
    Container(core, "overdrive-core", "Rust crate (class: core)", "Ports + newtypes + aggregates + Reconciler trait + Dataplane trait")
    Container(scheduler, "overdrive-scheduler", "Rust crate (class: core)", "Pure-fn `schedule(...)`")
    Container(store_local, "overdrive-store-local", "Rust crate (class: adapter-host)", "LocalStore + LocalObservationStore (redb)")
    Container(host, "overdrive-host", "Rust crate (class: adapter-host)", "SystemClock, OsEntropy, TcpTransport")
    Container(worker, "overdrive-worker", "Rust crate (class: adapter-host)", "ExecDriver + workload-cgroup mgmt + node_health writer")
    Container(sim, "overdrive-sim", "Rust crate (class: adapter-sim)", "Sim* adapters incl. SimDataplane + turmoil harness")
    Container(ctrl, "overdrive-control-plane", "Rust crate (class: adapter-host)", "Axum router + ReconcilerRuntime + ActionShim + JobLifecycle")
    Container(cli, "overdrive-cli", "Rust binary (class: binary)", "`overdrive` CLI + `serve` composition root")
    Container(xtask, "xtask", "Rust binary (class: binary)", "cargo xtask — incl. NEW `bpf-build`, filled-in `bpf-unit` + `integration-test vm`")

    Container(bpf, "overdrive-bpf  [NEW]", "Rust crate (class: binary, target: bpfel-unknown-none, #![no_std])", "Kernel-side eBPF programs. Phase 2.1: one no-op XDP `xdp_pass` + `LruHashMap<u32,u64>` packet counter. Compiles to ELF object at `target/xtask/bpf-objects/overdrive_bpf.o`.")
    Container(dataplane, "overdrive-dataplane  [NEW]", "Rust crate (class: adapter-host)", "Userspace BPF loader. EbpfDataplane impl of Dataplane trait. Embeds BPF object via include_bytes!. Stub method bodies (Ok(())/empty Vec) deferred to #24/#25/#27.")
  }

  ContainerDb(redb_file, "redb file", "On-disk ACID KV")
  ContainerDb(bpf_obj, "BPF object", "ELF at target/xtask/bpf-objects/overdrive_bpf.o")
  System_Ext(kernel, "Linux kernel BPF subsystem", "/sys/fs/bpf, bpf(2) syscalls, XDP hook on `lo`")
  System_Ext(ci, "CI pipeline")

  Rel(engineer, xtask, "Runs `cargo xtask bpf-build / bpf-unit / integration-test vm`")
  Rel(engineer, cli, "Runs `overdrive ...`")
  Rel(ci, xtask, "Invokes per-PR (incl. NEW bpf-build + bpf-unit + integration-test vm latest)")

  Rel(xtask, bpf, "Compiles via `cargo build --target bpfel-unknown-none`; copies ELF to stable path")
  Rel(xtask, bpf_obj, "Writes (bpf-build); reads (integration-test vm)")
  Rel(xtask, dataplane, "Runs `cargo nextest run -p overdrive-dataplane` for host-side tests")

  Rel(dataplane, core, "Implements Dataplane port from")
  Rel(dataplane, bpf_obj, "Reads at compile time via include_bytes! (build.rs guards artifact presence)")
  Rel(dataplane, kernel, "Loads BPF programs; attaches XDP to `lo`; reads BPF maps via bpftool / aya map API")

  Rel(ctrl, core, "Implements handlers against ports")
  Rel(ctrl, store_local, "Reads/writes intent + observation")
  Rel(worker, kernel, "ExecDriver + cgroup mgmt")
  Rel(host, core, "Implements Clock/Entropy/Transport ports")
  Rel(scheduler, core, "Pure-fn helper for JobLifecycle reconciler")
  Rel(store_local, redb_file, "ACID transactions to")

  Rel(cli, ctrl, "Composition root: instantiates control plane (when role includes control-plane)")
  Rel(cli, worker, "Composition root: instantiates worker (when role includes worker)")
  Rel(cli, dataplane, "Composition root: future — `EbpfDataplane::new` instantiated alongside worker; #24+ wires Arc<dyn Dataplane> into control plane")
```

L3 (component diagram) is intentionally skipped for Phase 2.1 — the
loader is a single struct with three trait methods (two no-ops) and
component decomposition would not add information. L3 becomes
warranted around #25 (SERVICE_MAP) when the loader gains map-update,
flow-event-consumer, and attachment-state components.
