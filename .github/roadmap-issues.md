# Roadmap issues — draft from whitepaper §24

Source: `docs/whitepaper.md` §24 (commit at time of extraction: current working tree).

Each row is one proposed GitHub issue. Before bulk-creating:

1. Review titles — shorten, split, or merge as needed.
2. Confirm the label taxonomy (below) is the one we want on the Project.
3. Decide whether to open issues on `overdrive-sh/overdrive` or a dedicated
   planning repo.

## Label taxonomy (proposed)

**Phase** (single-select field on the Project, *not* a label — one per issue):
`phase-1` · `phase-2` · `phase-3` · `phase-4` · `phase-5` · `phase-6` · `phase-7`

**Area** (GitHub labels, can stack):
`area/control-plane` · `area/dataplane` · `area/storage` · `area/security` ·
`area/observability` · `area/gateway` · `area/drivers` · `area/os` ·
`area/sdk` · `area/cli` · `area/testing` · `area/ci`

**Type** (optional, for filtering):
`type/primitive` · `type/integration` · `type/migration` · `type/sdk` ·
`type/hardening` · `type/research`

---

## Phase 1 — Single-Node MVP (Months 1–3)

Goal: single-node orchestrator whose trait boundaries are already HA-shaped and whose DST harness is already running.

| # | Title | Areas | Type | Source | Notes |
|---|---|---|---|---|---|
| 1.1 | Define core data model (Job, Node, Allocation, Policy, Investigation) | `control-plane` | `primitive` | §4, §12 | Rust types + serde/rkyv; newtype IDs per `.claude/rules/development.md`; Route joins the model in Phase 4 (4.10) |
| 1.2 | Control plane API surface (tonic/gRPC) for node-agent + CLI | `control-plane` `cli` | `primitive` | §4 | Internal transport only; no public API yet |
| 1.3 | `IntentStore` trait + `LocalStore` (redb direct) | `storage` | `primitive` | §4, §17 | Single-node implementation; `export_snapshot` / `bootstrap_from` from day one |
| 1.4 | `ObservationStore` trait + in-memory LWW implementation | `storage` | `primitive` | §4 | Final read/write shape for service-backends/verdicts (operator revocation table lands in Phase 5) |
| 1.5 | Nondeterminism traits: `Clock`, `Transport`, `Entropy`, `Dataplane`, `Driver`, `Llm` | `control-plane` `testing` | `primitive` | §21 | DST precondition; lint gate enforcing no bypass |
| 1.6 | turmoil DST harness + `SimDriver` / `SimDataplane` / `SimClock` / `SimTransport` / `SimEntropy` / `SimObservationStore` / `SimLlm` | `testing` | `primitive` | §21 | Tier 1 infrastructure; `cargo xtask dst` entry point |
| 1.7 | Process driver (tokio::process + cgroups v2) | `drivers` | `primitive` | §6 | First concrete `Driver` impl |
| 1.8 | Basic scheduler (first-fit) | `control-plane` | `primitive` | §4 | Bin-packing heuristics come later; taint/toleration support in 1.11 |
| 1.9 | Reconciler primitive — trait + runtime + evaluation broker (cancelable-eval-set) | `control-plane` | `primitive` | §18 | Storm mitigation shipped native, not retrofitted; runtime provisions and manages per-primitive private libSQL DBs for reconciler memory (backoff counters, placement history, resource samples) |
| 1.10 | CLI: `overdrive job submit`, `overdrive node list`, `overdrive alloc status` | `cli` | `primitive` | §24 | Minimal viable operator surface |
| 1.11 | Control-plane cgroup isolation + scheduler taint/toleration support | `control-plane` `os` | `primitive` | §4 | `overdrive.slice/control-plane.slice` reserved budget; default `control-plane:NoSchedule` taint enforced by scheduler |
| 1.12 | Job-lifecycle reconciler (start / stop / migrate / restart; convergence to declared replica count) | `control-plane` | `primitive` | §18 | Foundational §18 Built-in Primitive; primary convergence loop cited by other reconcilers (6.17, 3.13, etc.); drives scheduler (1.8) and selected driver (1.7) |
| 1.13 | Garage object store integration — single-mode local-filesystem replacement + HA deployment toggle | `storage` | `primitive` | §17 | Vendored + configured; single-mode uses same content-addressed interface over local fs (no replication); HA replication toggles on alongside Phase 5 stores |

---

## Phase 2 — The Dataplane That Differentiates (Months 3–6)

Goal: every eBPF primitive that makes Overdrive structurally different from Kubernetes/Nomad.

| # | Title | Areas | Type | Source | Notes |
|---|---|---|---|---|---|
| 2.1 | aya-rs eBPF scaffolding + build pipeline | `dataplane` | `primitive` | §7 | `crates/overdrive-bpf`, loader plumbing |
| 2.2 | XDP routing + service load balancing (`SERVICE_MAP`, O(1) lookup) | `dataplane` | `primitive` | §7 | Replaces kube-proxy-class logic |
| 2.3 | TC egress interception scaffolding (ready for sidecar chain) | `dataplane` | `primitive` | §7, §9 | General interception layer |
| 2.4 | sockops mTLS + kTLS installation (kernel TLS offload) | `dataplane` `security` | `primitive` | §7, §8 | Transparent to workloads; consumes `IDENTITY_MAP` (written by 2.13 `IdentityMgr`) to tag flows with SPIFFE identity |
| 2.5 | BPF LSM programs — `file_open`, `socket_create`, `socket_connect`, `task_setuid`, `bprm_check_security` | `security` | `primitive` | §7, §19 | Per-workload MAC |
| 2.6 | Built-in CA (rcgen + rustls) — root + per-node intermediate + SVID issuance/rotation | `security` | `primitive` | §4, §8 | Replaces SPIRE; root CA key encrypted at rest in IntentStore |
| 2.7 | Real-kernel integration test harness bootstrap — Tier 2 BPF unit tests + Tier 3 kernel-matrix CI (LVH) + Tier 4 verifier/perf gates | `testing` `ci` | `primitive` | §22 | `cargo xtask integration-test vm`; `veristat`, `xdp-bench` baselines; nightly `bpf-next` soft-fail + netem fault-injection soak jobs; per-release aarch64 Tier-3 matrix on self-hosted Graviton runner |
| 2.8 | Tier 3 sockops+kTLS test cases (`ss -K`, veth wire capture) + BPF LSM positive/negative fixtures | `testing` | `hardening` | §22 | First full kernel-matrix gate |
| 2.9 | eBPF flow + resource telemetry programs (`FlowEvent` / `ResourceEvent` ringbuf producers) | `dataplane` `observability` | `primitive` | §7, §12 | XDP flow-export program; kprobes for CPU/memory/IO resource profiles; AF_XDP for telemetry fast path; data source for DuckLake (4.9) |
| 2.10 | Pre-OOM pressure-signal eBPF program (cgroup v2 BPF + memory-pressure kprobes) | `dataplane` `observability` | `primitive` | §14 | Identity-tagged pressure samples — data source for Phase 6 right-sizing reconciler (6.8) |
| 2.11 | `overdrive-tester` in-VM binary (systemd unit, job manifest driver, results export) | `testing` `ci` | `primitive` | §22 | Harness inside Tier 3 LVH VMs — reads manifest, runs test cases, writes results to host-mounted dir, powers off |
| 2.12 | PREVAIL second-opinion static analysis (nightly non-blocking CI) | `testing` `ci` | `hardening` | §22 | Second analyser vs kernel verifier; fails build on accept/reject disagreement |
| 2.13 | Workload `IdentityMgr` subsystem — per-allocation SVID lifecycle + trust bundle store | `security` `control-plane` | `primitive` | §5, §8, §11 | Issue SVID on allocation start, hold in memory, rotate before expiry, drop on stop; owns SPIFFE URI assignment; shared `Arc<IdentityMgr>` across sockops/gateway/telemetry; unified with ACME certs in 4.7 |
| 2.14 | Node enrollment / admission handler — first-boot exchange issues SVID + initial peer set + regional aggregator assignment | `control-plane` `security` | `primitive` | §7, §23 | Runs on control plane; accepts optional TPM attestation; returns trust bundle and node-intermediate CA material to the enrolling agent; 5.11 WireGuard extension hooks in here to write pubkey into `node_health` on admission |
| 2.15 | sockops in-flight connection tracking BPF map + drain detector | `dataplane` `control-plane` | `primitive` | §15 | Tracks live connections per allocation so the rolling-deploy reconciler and 4.12 staged-rollout workflow can terminate workloads only when drained |

---

## Phase 3 — Policy, Workflows, Drivers (Months 6–9)

Goal: policy + workflows + the non-process driver implementations. Still single-node. Operator auth deferred to Phase 5 (see note below).

| # | Title | Areas | Type | Source | Notes |
|---|---|---|---|---|---|
| 3.1 | Regorus policy evaluation → verdict compilation → `policy_verdicts` → BPF map hydration | `security` `dataplane` | `primitive` | §10, §13 | Intent→observation→kernel pipeline |
| 3.2 | Workflow primitive — `Workflow` trait, durable journal in per-primitive libSQL, typed signals, workflow-lifecycle reconciler | `control-plane` | `primitive` | §18 | First-class for platform and app code; SDK load-time version-skew rejection via code-graph hashing (rejects code changes that would deviate from in-flight journals) |
| 3.3 | Certificate rotation as first internal workflow (DST replay-equivalence gated) | `security` | `integration` | §18, §21 | Exercises workflow primitive; workload SVIDs only — operator certs land in Phase 5 |
| 3.4 | Chaos engineering reconciler (reuses §21 DST fault catalogue) | `testing` `control-plane` | `hardening` | §18, §21 | Production fault injection |
| 3.5 | Cloud Hypervisor microVM + VM driver (unified VMM) | `drivers` | `primitive` | §6 | Replaces Firecracker + QEMU default; moved from Phase 4 |
| 3.6 | virtiofsd lifecycle management + cross-workload volume sharing | `drivers` `storage` | `primitive` | §6 | Shared-mount use case; moved from Phase 4 |
| 3.7 | WASM serverless driver (Wasmtime) — warm instance pool, scale-to-zero, fuel budget | `drivers` | `primitive` | §16 | Sub-5ms cold start; moved from Phase 4 (prereq for sidecar runtime) |
| 3.8 | WASM policy engine — per-policy Wasmtime runtime with private libSQL DB access | `security` | `primitive` | §13 | Warm instance pool for μs eval; same sandbox as reconcilers/sidecars; content-addressed modules in Garage |
| 3.9 | WASM policy SDK (Rust `#[policy]` attribute, host interface) | `security` `sdk` | `sdk` | §13 | Typed `Verdict` return; DB host function for historical/stateful reasoning |
| 3.10 | Dual-engine policy chain evaluation (Rego + WASM) | `security` | `primitive` | §13 | `Allow \| Deny \| Defer` chain across engines; uniform `Policy` trait; engine is implementation detail |
| 3.11 | `Action::HttpCall` runtime shim + `external_call_results` observation path | `control-plane` | `primitive` | §18 | Pure reconciler emits action; async dispatcher via `Transport`; results land in ObservationStore and drive next reconcile tick |
| 3.12 | Job-spec `[job.security]` → BPF map compiler | `security` `dataplane` | `primitive` | §19 | TOML profile (`fs_paths`, `allowed_ports`, `allowed_binaries`, `no_raw_sockets`, egress mode/allowlist) → `FS_POLICY_MAP` + socket-policy maps |
| 3.13 | Node drain + workload migration reconciler (Tier 2 reactive self-healing) | `control-plane` | `primitive` | §12, §18 | Named Built-in Primitive; moves workloads off a node marked unhealthy or draining; works with job-lifecycle and scheduler for reschedule |

> **Deferred to Phase 5:** whitepaper §24 currently places operator identity, `overdrive cluster init`/`op create`, and the `revoked_operator_certs` table in Phase 3. Decision 2026-04-21: defer. Single-node is dev/edge territory where an unauthenticated local Unix-socket API is acceptable; multi-operator cert machinery only pays for itself once multi-node + real deployments land. The whitepaper §24 phasing needs a matching update when this draft is ratified.

---

## Phase 4 — Sidecars, Gateway, Telemetry (Months 9–12)

Goal: composition surface (sidecars) + ingress (gateway) + native telemetry. End of phase: complete single-node product.

| # | Title | Areas | Type | Source | Notes |
|---|---|---|---|---|---|
| 4.1 | WASM sidecar runtime — generalises TC interception into per-workload chain | `dataplane` | `primitive` | §9 | Foundation for all sidecars; defines `trait Sidecar` (`on_egress` / `on_ingress` / `on_start` / `on_stop`) + `SidecarAction` enum (Pass / Modify / Block / Redirect) + `SidecarContext` |
| 4.2 | Built-in sidecars: `credential-proxy`, `content-inspector`, `rate-limiter`, `request-logger` | `dataplane` `security` | `primitive` | §9 | Native Rust trait objects |
| 4.3 | Sidecar SDK (Rust + TypeScript) | `sdk` | `sdk` | §9 | User-authored WASM sidecars |
| 4.4 | Gateway subsystem (node-agent-embedded; hyper + rustls; in-process BPF map access) | `gateway` | `primitive` | §11 | Not a platform job — infrastructure |
| 4.5 | Gateway protocols: HTTP/1.1, HTTP/2, gRPC, gRPC-Web, WebSocket | `gateway` | `primitive` | §11 | Full L7 surface |
| 4.6 | Gateway middleware pipeline: rate limiting, JWT auth, CORS, circuit breaking, egress inspection | `gateway` | `primitive` | §11 | Declarative per-route config; egress inspection is distinct from per-workload sidecars and runs in the gateway pipeline |
| 4.7 | Embedded ACMEv2 client via `instant-acme` — HTTP-01, DNS-01, TLS-ALPN-01 | `gateway` `security` | `primitive` | §11 | Unified `IdentityMgr` rotation; single rcgen-based cert path; `tls = "operator"` lane for operator-uploaded certs; pluggable DNS-01 provider interface (Route53 etc.) |
| 4.8 | Declarative request replay (`overdrive-replay` header, XDP loop counter, ≤1MB buffer) | `gateway` `dataplane` | `primitive` | §11 | Application-driven routing |
| 4.9 | DuckLake telemetry pipeline (libSQL catalog + Parquet in Garage + time-travel queries) | `observability` `storage` | `primitive` | §12, §17 | Replaces hot/cold split |
| 4.10 | Top-level `Route` resource in IntentStore; routes pushed to gateway nodes | `gateway` `control-plane` | `primitive` | §11 | Routes declared as platform resources, not embedded in job specs; joins the core data model in this phase |
| 4.11 | Private Service VIPs — IPv6 VIP allocation from `fdc2::/16` | `dataplane` `gateway` | `primitive` | §11 | Stable `<job>.svc.overdrive.local` target for non-SPIFFE clients; XDP SERVICE_MAP routing; auto-wake lights up with Phase 6 scale-to-zero (6.9) |
| 4.12 | Staged-rollout workflow (human-in-the-loop with ratification signals) | `control-plane` | `primitive` | §18 | Typed workflow on the Phase-3 primitive (3.2); operator ratification via typed signals at declared checkpoints |
| 4.13 | Cron-invocation reconciler for scheduled WASM functions | `control-plane` `drivers` | `primitive` | §16 | One of three §16 invocation triggers (HTTP via gateway, Schedule via cron reconciler, Event via bus — bus deferred as nice-to-have); cron expressions → function invocation |
| 4.14 | Rolling-deploy reconciler — SERVICE_MAP weighted-backend updates with drain-and-terminate | `control-plane` `dataplane` | `primitive` | §15, §18 | Consumes 2.15 in-flight connection tracking to terminate safely; primitive underlying 4.12 staged-rollout workflow and 4.16 multi-stage deployment workflow |
| 4.15 | Canary promotion / rollback reconciler — weighted-backend stepping with SLO-based promote or rollback | `control-plane` `dataplane` | `primitive` | §15, §18 | Named §18 Built-in Primitive; weight stepping against error-rate / latency / memory / flow-anomaly thresholds; LLM-supervised variant overlays at 6.7 (Tier-3 reasoning) |
| 4.16 | Multi-stage deployment workflow — canary → ramp → promotion with typed rollback signals | `control-plane` | `primitive` | §18 | Distinct from 4.12 staged-rollout (which is HITL ratification); this is the automated multi-stage orchestration built on top of 4.14 + 4.15 reconcilers |

---

## Phase 5 — HA and Multi-Node (Months 12–15)

Goal: trait-swap phase. Single-node product keeps working; new impls slot in.

| # | Title | Areas | Type | Source | Notes |
|---|---|---|---|---|---|
| 5.1 | `RaftStore` (openraft + redb) — HA `IntentStore` implementation | `storage` | `primitive` | §4, §17 | 3- or 5-node quorum |
| 5.2 | `CorrosionStore` — production `ObservationStore` (Corrosion + cr-sqlite over SWIM/QUIC) | `storage` | `primitive` | §4, §17 | Replaces in-memory LWW |
| 5.3 | Corrosion schema: `alloc_status`, `service_backends`, `node_health`, `policy_verdicts`, `revoked_operator_certs`, `external_call_results`, `investigation_state` | `storage` | `primitive` | §4, §17 | Owner-writer model, CRR tagging |
| 5.4 | Single → HA migration via `export_snapshot` / `bootstrap_from` (zero-downtime) | `storage` | `migration` | §4 | No external tooling; `overdrive cluster upgrade --mode ha --peers ...` CLI subcommand |
| 5.5 | BPF map hydration via Corrosion subscriptions (retire gRPC push path) | `dataplane` `storage` | `migration` | §7 | SQL subscription → BPF map |
| 5.6 | Node-identity-scoped write authorisation on Corrosion peers | `security` `storage` | `hardening` | §4 | CRDT site ID ↔ SVID binding |
| 5.7 | Additive-only schema migration tooling (avoid Fly backfill-storm failure mode) | `storage` | `hardening` | §4 | Two-phase rollout pattern |
| 5.8 | Event-loop watchdogs on Corrosion peers (Fly contagion-deadlock mitigation) | `storage` | `hardening` | §4, §21 | DST-exercised scenario |
| 5.9 | Image Factory MVP — `meta-overdrive` Yocto layer (immutable node OS with BPF LSM, kTLS, KVM, vhost-vsock) | `os` | `primitive` | §23 | First production deployment target |
| 5.10 | `overdrive-image-factory` Rust service (schematic store, artifact cache, HTTP frontend) | `os` | `primitive` | §23 | Thin wrapper over Yocto; serves SPDX SBOM (`sbom.spdx.json` per build via `inherit create-spdx`); async-build orchestrator owns `(schematic_id, version, arch)` tuples, returns 202 + build_id for cache misses, `GET /v1/builds/{build_id}` progress poll |
| 5.11 | Mesh VPN underlay extensions: `wireguard` (platform-managed keys via enrollment) | `dataplane` `os` | `primitive` | §7 | Pubkeys via `node_health` |
| 5.12 | Mesh VPN underlay extensions: `tailscale` (BYO Tailscale/Headscale coord) | `dataplane` `os` | `primitive` | §7 | NAT-traversing deployments |
| 5.13 | DST cross-region tests using real `CorrosionStore` (sparing) | `testing` | `hardening` | §21 | Real SWIM/LWW semantics |
| 5.14 | Operator identity + CLI auth: SPIFFE IDs under `spiffe://overdrive.local/operator/...`, 8h TTL certs, `~/.overdrive/config` | `security` `cli` | `primitive` | §8 | Deferred from Phase 3; global-across-regions from day one |
| 5.15 | `overdrive cluster init` (first admin cert) + `overdrive op create` (additional) + `overdrive op revoke <spiffe_id>` | `security` `cli` | `primitive` | §8 | Operator provisioning flow; revoke writes to `revoked_operator_certs` (5.16) |
| 5.16 | `revoked_operator_certs` table + revocation-sweep reconciler (gossip-propagated revocation) | `security` `storage` | `primitive` | §8 | Lands alongside `CorrosionStore` — gossip is the delivery mechanism |
| 5.17 | Operator trust-bundle federation mechanism across regions | `security` `storage` | `primitive` | §8 | Makes 5.14 "global-across-regions" concrete: either nesting per-region CAs under a cluster-scoped operator root, or distributing the operator trust bundle as ObservationStore state. Prerequisite for 7.1 multi-region |
| 5.18 | Image Factory extension registry + recipe resolver | `os` | `primitive` | §23 | Maps schematic `extensions.official = [...]` (e.g. `nvidia-gpu`) to versioned BitBake recipes; consumed by 5.11 `wireguard`, 5.12 `tailscale`, and 6.16 `persistent-microvm-guest-agent` |

---

## Phase 6 — Intelligence (Months 15–18)

Goal: self-healing, right-sizing, investigation, persistent stateful workloads.

| # | Title | Areas | Type | Source | Notes |
|---|---|---|---|---|---|
| 6.1 | LLM observability agent (rig-rs) — scaffolding + tool registry | `observability` | `primitive` | §12 | Foundation for §12 primitives |
| 6.2 | `Investigation` first-class resource (ObservationStore live state + incident-memory libSQL on conclusion) | `observability` | `primitive` | §12 | Joins Job/Node/Allocation/Policy |
| 6.3 | Declarative toolset catalog (`builtin:overdrive-core` Rust + WASM third-party, Garage-addressed) | `observability` `sdk` | `primitive` | §12 | Reproducible investigation transcripts |
| 6.4 | Typed `Action` enum with risk-tier approval gate (Tier 0/1/2) | `observability` `control-plane` | `primitive` | §12 | Raft-mediated remediation |
| 6.5 | `correlation_key` on telemetry + SPIFFE-identity joins across DuckLake | `observability` | `primitive` | §12 | Alert dedup, cross-event correlation |
| 6.6 | `llm_spend` reconciler (per-investigation, per-job, cluster-wide budget) | `observability` `control-plane` | `primitive` | §12 | Cost enforcement |
| 6.7 | Self-healing Tier 3 (LLM reasoning over Tier 1–2) | `observability` | `integration` | §12 | Pattern-match + action proposal; includes LLM-supervised canary promotion overlay on 4.15 (error rate / latency / memory / flow-anomaly → promote or rollback) |
| 6.8 | Right-sizing reconciler (cgroup adjustment for processes, CH hotplug for VMs) | `control-plane` `drivers` | `primitive` | §14 | Pre-OOM pressure signal loop; includes resource-profile accumulation subsystem — p95 CPU/memory per job per hour-of-week, rolling 30-day window in libSQL, confidence score on recommendations |
| 6.9 | Scale-to-zero for VM workloads — `suspended` state, idle-eviction, proxy-triggered resume | `drivers` `control-plane` `gateway` | `primitive` | §14 | Extends alloc lifecycle |
| 6.10 | Incident memory (libSQL) with embedding-similarity retrieval | `observability` `storage` | `primitive` | §12 | Runbook + prior-incident lookup |
| 6.11 | Predictive scaling (LLM pattern detection → cron-based schedules) | `observability` `control-plane` | `primitive` | §14 | Complements rule-based scaling |
| 6.12 | Persistent microVMs step 1: CH snapshot/restore + `userfaultfd` lazy paging + VMGenID | `drivers` | `primitive` | §6, §14 | Foundation of persistence |
| 6.13 | Persistent microVMs step 2: `overdrive-fs` — Rust chunk store (Garage chunks + per-rootfs libSQL + NVMe cache + vhost-user-fs) | `storage` | `primitive` | §17 | Single-writer-per-rootfs |
| 6.14 | Persistent microVMs step 3: gateway auto-route (`expose = true`) + credential-proxy sidecar defaults | `gateway` `drivers` | `integration` | §6 | Composes existing primitives |
| 6.15 | Persistent microVMs step 4: idle-eviction reconciler with checkpoint (`snapshot_on_idle_seconds`) | `control-plane` `drivers` | `primitive` | §6 | Scale-to-zero for long-lived state |
| 6.16 | Persistent microVMs step 5: `overdrive-guest-agent` — ttRPC/vsock, SPIFFE, 4-method surface | `drivers` `security` | `primitive` | §6 | Application-consistent snapshots |
| 6.17 | Deterministic rule-based scaler (Rego over ObservationStore metrics, 15 s cadence) | `control-plane` | `primitive` | §14 | Writes desired replica count into IntentStore; complements 6.11 predictive scaler |
| 6.18 | LLM-generated WASM policy authoring flow | `security` `observability` | `integration` | §13 | Operator NL prompt → LLM agent generates WASM policy module → Garage-stored → operator reviews source → activates |
| 6.19 | Bin-packing feedback loop — scheduler reads resource profiles for placement density | `control-plane` | `primitive` | §14 | Promotes 1.8 first-fit scheduler to density-aware using profiles written by 6.8; one of four §14 right-sizing subsystems |

---

## Phase 7 — Federation and Ecosystem (Months 18+)

Goal: horizontal scale, user-facing SDKs, operational polish.

| # | Title | Areas | Type | Source | Notes |
|---|---|---|---|---|---|
| 7.1 | Multi-region federation: per-region IntentStore (Raft) + global ObservationStore (Corrosion) | `storage` | `primitive` | §4 | Autonomous regions |
| 7.2 | Regional Corrosion clusters + thin global membership cluster (regionalized blast radius) | `storage` | `hardening` | §4 | Fly-learned-mid-incident shape |
| 7.3 | Region-aware scheduler + gateway (reads `node_health.region` locally) | `control-plane` `gateway` | `integration` | §4 | No WAN RPC |
| 7.4 | Cross-region partition tolerance (local-commit + LWW heal) | `storage` | `hardening` | §4 | Intent never stretched |
| 7.5 | Region preference hints (`overdrive-prefer-region` / `overdrive-force-region`) in XDP fast path | `dataplane` `gateway` | `primitive` | §11 | BPF map lookup, no userspace hop |
| 7.6 | WASM Component Model SDK (Rust, TypeScript, Go) for serverless functions | `sdk` | `sdk` | §16 | Language-agnostic via Component Model; platform event bus for `ctx.emit_event` deferred as nice-to-have — lands here when a concrete use case arises |
| 7.7 | Workflow WASM SDK (Rust, TypeScript, Go) — application-facing durable execution | `sdk` | `sdk` | §18 | Same primitive as internal workflows |
| 7.8 | OTel export adapter | `observability` | `integration` | §12 | External interop; OTel as export, not foundation |
| 7.9 | Unikernel drivers (Nanos, Unikraft with virtiofs) | `drivers` | `primitive` | §6 | Extreme density tier |
| 7.10 | QEMU opt-in driver (exotic hardware emulation only) | `drivers` | `primitive` | §6 | Not default |
| 7.11 | Runbook primitive (HolmesGPT-format markdown + YAML frontmatter, Garage-addressed, libSQL-indexed) | `observability` | `primitive` | §12 | Community catalog reuse |
| 7.12 | Platform-signed diagnostic-probe catalog + `Action::AttachDiagnosticProbe` deadline reconciler | `observability` `security` | `primitive` | §12 | Hypothesis verification via owned dataplane |
| 7.13 | `ShardedIntentStore` — Twine-shape pluggable backend (gated on design partner) | `storage` | `research` | §4 | Single-region density beyond openraft+redb ceiling |
| 7.14 | Image Factory polish: OCI registry frontend (in-place upgrades via registry pull) | `os` | `primitive` | §23 | Talos-shape upgrade path |
| 7.15 | Image Factory polish: PXE boot, dm-verity + TPM attestation, Secure Boot signing | `os` `security` | `hardening` | §23 | Phase-7 only |
| 7.16 | OIDC enrolment bridge for operators (`overdrive login`) — Authorization Code + PKCE | `security` | `primitive` | §8 | Offboarding becomes IdP concern |
| 7.17 | Biscuit tokens for CI delegation (`biscuit-auth` over mTLS) | `security` | `primitive` | §8 | Capability attenuation; additive to mTLS |
| 7.18 | Cross-region migration workflow (quiesce source → metadata handoff → resume target) | `control-plane` `storage` | `primitive` | §18 | Uses workflow primitive (3.2) + `overdrive-fs` single-writer lifecycle (6.13) |
| 7.19 | OTel Collector as pre-configured platform job | `observability` | `integration` | §12 | Reference deployment of 7.8 OTLP export adapter as first-party platform job |
| 7.20 | A/B partition upgrade mechanism (digest verify → inactive-partition write → reboot) | `os` | `primitive` | §23 | Verifies OCI-pulled image digest vs schematic ID; composes with 7.14 OCI registry frontend |
| 7.21 | ESR verification targets for first-party reconcilers (Verus/Anvil-style) | `testing` `control-plane` | `research` | §18 | Eventually-Stable Reconciliation specs shipped with each built-in reconciler; mechanically checkable |
| 7.22 | WASM third-party reconciler / workflow loader + ESR precondition enforcement at load time | `control-plane` `security` | `primitive` | §18 | Distinct from 7.7 (application-facing user workflows); this is the platform-extension path for third-party reconcilers and workflows, content-addressed in Garage, ESR preconditions declared in manifest and enforced by the runtime at load time |

---

## Summary

| Phase | Issues | Themes |
|---|---|---|
| 1 | 13 | Traits, DST, single-node, cgroup isolation, job lifecycle, Garage |
| 2 | 15 | eBPF, mTLS, LSM, kernel CI, telemetry programs, pressure signal, PREVAIL, IdentityMgr, enrollment, drain tracking |
| 3 | 13 | Policy (Rego + WASM + chain), workflows, drivers, HttpCall runtime, job-sec compiler, node drain |
| 4 | 16 | Sidecars, gateway, Routes, VIPs, telemetry, staged-rollout, cron trigger, rolling deploy, canary, multi-stage workflow |
| 5 | 18 | HA, Corrosion, Image Factory, mesh VPN, operator auth + federation, extension registry |
| 6 | 19 | LLM agent, right-sizing, rule-based scaler, LLM policy authoring, persistent microVMs, bin-pack feedback |
| 7 | 22 | Federation, SDKs, migration workflow, OTel Collector, A/B upgrade, ESR targets, WASM ext loader |
| **Total** | **116** | |

## Open decisions before bulk-create

1. **Target repo.** `overdrive-sh/overdrive`, or a separate planning repo (e.g. `overdrive-sh/roadmap`)?
2. **Granularity.** Some rows (e.g. 5.9 "Image Factory MVP", 6.1 "LLM agent scaffolding") are already multi-week epics. Split into sub-issues now, or open as tracking issues and let the assignee shard?
3. **Assignees / milestones.** Who gets auto-assigned at issue creation? Which phases get date-bound milestones vs. flow-only board tracking?
4. **Labels first.** The `area/*` and `type/*` labels need to exist in the repo before `gh issue create --label ...` works. OK to create those as step 0 of the bulk-create?
5. **Body template.** Proposed issue body: one-line summary, "Source" link back to whitepaper anchor, "Depends on" (predecessors), "Acceptance" (≤3 bullets). Draft this template before bulk-create?
