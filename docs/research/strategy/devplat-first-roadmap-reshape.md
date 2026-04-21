# Roadmap Reshape Under Developer-Platform-First Framing

**Date**: 2026-04-20 | **Author**: synthesis pass, not new research | **Input**: `whitepaper.md §24` + `docs/research/strategy/demand-signal-orchestrator-vs-devplat.md` + `docs/research/strategy/cloudflare-oss-competitor-pivot.md` + `docs/research/platform/queues-cron-event-bus-primitives.md` + `docs/research/platform/libsql-per-workload-primitive-2026-04-20.md`

## Premise

The demand-signal research landed on **developer-platform-first** as the 2026–2027 framing. The architecture research landed on **keep the current foundation** — eBPF, SPIFFE, Cloud Hypervisor, IntentStore/ObservationStore, BPF LSM — because that foundation is the moat that distinguishes "real OSS Cloudflare" from a Heroku-shape PaaS wrapper. This document does the concrete work: given that premise, what changes in the §24 roadmap?

The short answer: **engineering weight moves from orchestrator-depth to developer-visible surface, and ~4 primitives that are absent from §24 get added.** No load-bearing architecture gets cut. Several items get deferred. A few get pulled forward hard.

## v1 Success Criterion

The v1 devplat launch is *not* "Overdrive runs a cluster." It is:

> A developer runs `overdrive deploy function.ts`, receives a stable public HTTPS URL backed by a per-workload SPIFFE identity, and their function can call `env.KV.get()`, `env.DB.prepare()`, `env.R2.get()`, `env.QUEUE.send()`, `env.SCHEDULE.trigger()`, `env.EVENT.publish()` — all working end-to-end against a single-node Overdrive running on their laptop or a small bare-metal box.

Everything in the roadmap is scored against "does this move us closer to that sentence?"

## Decision Matrix — §24 Line by Line

### Phase 1 — Foundation (Months 1–3)

| Item | Decision | Rationale |
|---|---|---|
| Core data model (Job, Node, Allocation, Policy) | **KEEP** | Substrate — unchanged. Add `Schedule` and `Investigation` later; `Function` / `DurableObject` resources are new in Phase 4. |
| Control plane API (tonic/gRPC) | **KEEP** | Internal transport; customer-facing is CLI + HTTP on top. |
| IntentStore LocalStore | **KEEP** | Single-node devplat sits on LocalStore. No Raft needed for first-workload. |
| ObservationStore trait + in-memory | **KEEP** | Trait shape is right; in-memory is a valid v1 backend for single-node. |
| Injectable traits (Clock, Transport, Entropy, Dataplane, Driver, ObservationStore, Llm) | **KEEP** | DST foundation — non-negotiable. |
| turmoil DST harness + Sim* | **KEEP** | DST is the moat on correctness. Skimp here and everything else gets fragile. |
| Process driver | **KEEP** | Simplest driver; good for DST and early workloads. |
| Basic scheduler (first-fit) | **KEEP** | Sufficient for v1 devplat. |
| CLI (`overdrive job submit` etc.) | **EXPAND** | The operator CLI exists; the *developer* CLI doesn't. Add `overdrive deploy`, `overdrive tail`, `overdrive dev`, `overdrive logs`, `overdrive secret` as a parallel verb tree. **This is the largest net-new Phase 1 item under devplat-first.** |
| Image Factory MVP (Yocto + overdrive-image-factory) | **KEEP, NARROWER** | Devplat users don't boot bare metal; operators do. Ship the minimum that supports a single-node installer + one cloud AMI. Full Image Factory (OCI registry, PXE, dm-verity, Secure Boot) moves to platform-scaling phase. |

### Phase 2 — Networking and Observation (Months 3–6)

| Item | Decision | Rationale |
|---|---|---|
| aya-rs eBPF scaffolding | **KEEP** | Substrate. |
| XDP routing + service load balancing | **KEEP** | Substrate. Needed for per-workload VIPs the gateway targets. |
| TC egress control | **KEEP** | Substrate — needed for WASM sidecar interception (credential proxy, content inspector). |
| RaftStore (HA mode + single→HA migration) | **PUSH BACK** | Not needed for single-node devplat v1. Move to platform-scaling phase. The trait boundary lets this plug in without code rewrites. |
| CorrosionStore (real Corrosion + cr-sqlite) | **KEEP** | Even on single node, Corrosion is the right backend for the observation layer — subscriptions drive BPF map hydration. `SimObservationStore` only covers DST. |
| Corrosion schema (`alloc_status`, `service_backends`, `node_health`, `policy_verdicts`) | **KEEP, EXTEND** | Add devplat tables: `function_registry`, `kv_partitions`, `schedule_state`, `event_topics` (shape TBD). |
| Additive-only migration tooling | **KEEP** | Saves a production incident. Non-negotiable. |
| Real-kernel integration test harness (§22 Tier 2/3/4) | **KEEP** | Cilium / Tetragon-grade kernel-matrix CI is what makes the eBPF moat defensible. Skimping here means eBPF becomes a liability within 18 months. |

### Phase 3 — Identity and Security (Months 6–9)

| Item | Decision | Rationale |
|---|---|---|
| Built-in CA (rcgen + rustls) | **KEEP** | Substrate. |
| SPIFFE SVID issuance + rotation | **KEEP** | Substrate. Per-workload identity is what enables Durable-Object-shape addressing and credential-proxy sidecars. |
| Operator identity + CLI auth | **KEEP, SCOPE DOWN** | `overdrive cluster init` + `overdrive op create` are needed. **OIDC and Biscuit move to platform-scaling.** |
| sockops mTLS + kTLS | **KEEP** | Substrate. East-west encryption with workload identity. |
| BPF LSM programs | **KEEP** | This is *the* moat against multi-tenant hosting of customer code. Without it, you are a PaaS, not a platform. |
| Regorus policy evaluation | **KEEP, SCOPE DOWN** | v1 devplat uses a small set of platform-default policies (egress allowlist, no raw sockets). Custom policy authoring is a platform-eng-facing feature — defer the Rego authoring UX. |
| Tier 3 sockops + kTLS + LSM test fixtures | **KEEP** | Kernel-matrix CI for security-critical code. |
| **Workflow primitive** | **KEEP, PULL FORWARD Schedule + EventBus** | Workflows themselves stay here. The Schedule and EventBus primitives — recommended native in prior research — ride on the workflow primitive and land in Phase 4 as developer-facing surfaces. |

### Phase 4 — Additional Drivers (Months 9–12)

The original Phase 4 is mostly right under devplat-first — it's where all the runtime primitives live — but several items from Phase 5 pull into it, and four new developer primitives join.

| Item | Decision | Rationale |
|---|---|---|
| Cloud Hypervisor microVM + VM driver | **KEEP** | Substrate for Durable Objects + stateful workloads. |
| virtiofsd lifecycle | **KEEP** | Needed for `overdrive-fs`. |
| WASM serverless driver (Wasmtime) | **KEEP** | Primary runtime for developer functions. |
| WASM sidecar runtime + TC interception | **KEEP** | Substrate for credential proxy, content inspector, user sidecars. |
| Built-in sidecars (credential-proxy, content-inspector, rate-limiter, request-logger) | **KEEP** | All four are directly relevant to developer workloads. |
| Sidecar SDK (Rust + TS) | **KEEP** | Dogfoods the bindings ABI. |
| Gateway (hyper + rustls) | **KEEP, PULL FORWARD** | Was Phase 4, stays Phase 4 — but becomes *the* v1 critical path item alongside the WASM driver. |
| Embedded ACMEv2 via `instant-acme` | **KEEP** | Public-trust HTTPS is table stakes. |
| DuckLake telemetry pipeline | **KEEP, SCOPE DOWN** | v1 needs `overdrive tail` + `overdrive logs` over a DuckLake-backed event stream. Full DuckLake SQL / time-travel API is platform-scaling-phase. |
| Mesh VPN underlay extensions (wireguard, tailscale) | **PUSH BACK** | Devplat v1 on single-region doesn't need overlay networking. Keep the extension mechanism (Image Factory), defer the actual integrations. |
| **Persistent microVMs — step 1 (CH snapshot/restore + userfaultfd + VMGenID)** | **PULL FORWARD** (was Phase 5) | This is Durable Objects. Can't ship a CF-shape devplat without it. |
| **Persistent microVMs — step 2 (`overdrive-fs`)** | **PULL FORWARD** (was Phase 5) | Ditto. Single-writer rootfs is what makes DO-shape workloads resumable. |
| **Persistent microVMs — step 3 (gateway auto-route + credential-proxy default)** | **PULL FORWARD** (was Phase 5) | Stable-URL-per-workload is the developer-facing pitch. |
| **Persistent microVMs — step 4 (scale-to-zero)** | **PULL FORWARD** (was Phase 5) | `snapshot_on_idle_seconds` is load-bearing for devplat cost economics. |
| **Persistent microVMs — step 5 (guest agent)** | **KEEP in Phase 5** | App-consistent snapshots are nice-to-have. v1 devplat ships crash-consistent; stronger guarantees in v1.1. |
| **NEW: Schedule primitive** | **PULL FORWARD** from research | `env.SCHEDULE.cron("0 0 * * *", handler)` — first-class resource, peer to Job/Workflow, backed by §18 workflow primitive. Prior research in `queues-cron-event-bus-primitives.md`. |
| **NEW: EventBus primitive** | **PULL FORWARD** from research | `env.EVENT.publish()` + `env.EVENT.subscribe()` — thin Rust trait over `ObservationStore::subscribe`. Subsumes §16 WASM event triggers. Prior research. |
| **NEW: KV primitive** | **NEW — add** | CF-shape `env.KV.get/put/list`. Implementation: dedicated Corrosion table with LWW semantics, eventually consistent. Cheap writes. No research doc yet; this is a platform design task. |
| **NEW: D1-shape per-workload libSQL** | **PULL FORWARD** from research | `env.DB.prepare(sql).run()` — per-workload SQLite, addressable from other workloads via SPIFFE. Prior research in `libsql-per-workload-primitive-2026-04-20.md`. |
| **NEW: R2 bindings** | **NEW — add** | Thin `env.R2` wrapper over Garage with CF-compatible API shape. Garage itself already in the stack. |
| **NEW: Queue primitive (embedded)** | **MOVE UP** | Prior research recommended "curated broker job type in Phase 5." Devplat-first requires a native `env.QUEUE.send() / consumer` primitive. Initial implementation: embedded Rust queue backed by `overdrive-fs` chunk store, single-consumer-group per topic. Community-grade broker remains a Phase 5 option. |
| **NEW: Scale-to-zero for WASM** | **NEW — add** | §16 warm-pool management plus genuine zero-instance idle. Paired with persistent-microVM scale-to-zero (step 4 above) so the control-plane contract is uniform. |

### Phase 5 — Intelligence (Months 12–18)

Under devplat-first, this phase becomes "operational intelligence and Durable Object polish," not "LLM self-healing as the headline." Several items move out.

| Item | Decision | Rationale |
|---|---|---|
| LLM observability agent (native SRE investigation primitives) | **PUSH BACK** | This is an *orchestrator-differentiator* pitch (platform-eng audience). Not load-bearing for the devplat v1 story. Keep the trait boundaries (§12 specifies `Llm` trait; don't lose that), defer the full implementation to the orchestrator-track phase. |
| Self-healing Tier 3 | **PUSH BACK** | Same — infra-operator feature. Tiers 1 and 2 (BPF reflexive + reconciler reactive) are already in earlier phases and cover 90% of the devplat user's needs. |
| Right-sizing reconciler | **PUSH BACK** | Operator-facing feature. Devplat v1 can ship without adaptive right-sizing; static resource limits are fine. |
| Incident memory (libSQL + embedding similarity) | **PUSH BACK** | Ties to LLM agent. Same phase. |
| Predictive scaling | **PUSH BACK** | Ties to LLM agent. Same phase. |
| Persistent microVMs steps 1–4 | **MOVED** to Phase 4. |
| Persistent microVMs step 5 (guest agent) | **KEEP** | App-consistent snapshots — v1.1 feature. |
| **NEW: Bindings ABI** | **NEW — major item** | The WASM host interface that exposes `env.KV`, `env.DB`, `env.R2`, `env.QUEUE`, `env.EVENT`, `env.SCHEDULE`, `env.DO` to function code. This is the single largest DX deliverable. Should be CF-compatible where possible to enable "port your Worker" stories. |
| **NEW: Wrangler-equivalent CLI** | **NEW — major item** | `overdrive deploy`, `overdrive dev`, `overdrive tail`, `overdrive logs`, `overdrive secret`, `overdrive kv / r2 / d1` verbs. Rust binary, same repo, ships with the server. |
| **NEW: Miniflare-equivalent local dev** | **NEW — major item** | `overdrive dev` spins up a single-node Overdrive + runs the function against local KV/DB/R2/Queue stubs with full bindings. Hot-reload. Essential for devplat UX. |
| **NEW: `overdrive-ff` SDK (Rust, TS, Python)** | **NEW — parallel repo** | Apache-2.0 client-side SDK. `defineFunction`, `defineDurableObject`, `defineWorkflow`, `defineSchedule`. FSL stays on server; Apache-2.0 on SDK. Published to npm / crates.io / PyPI from day one. |

### Phase 6 — Federation and Ecosystem (Months 18+)

Reshaped heavily. This phase becomes two tracks running in parallel: **platform-scaling** (for orchestrator-side buyers) and **devplat-breadth** (Pages-equivalent, AI, etc.).

| Item | Decision | Rationale |
|---|---|---|
| WASM Component Model SDK (Rust, TS, Go) | **KEEP** | Core devplat deliverable. |
| Workflow WASM SDK | **KEEP** | Workflows on §18 primitive. |
| OTel export adapter | **KEEP** | Standard interop. |
| Unikernel drivers (Nanos, Unikraft with virtiofs) | **CUT from v1 roadmap; revisit in Phase 8+** | Zero devplat-user demand. Zero orchestrator-buyer demand outside tiny research niches. Defer indefinitely unless a design partner appears. |
| QEMU opt-in driver | **CUT** | Exotic hardware emulation. Not devplat. Not even orchestrator-critical. Kill it from the roadmap entirely unless someone requests it. |
| Runbook primitive | **PUSH BACK** to an orchestrator-track phase | Investigation-agent feature. Orchestrator-track deliverable. |
| Platform-signed diagnostic-probe catalog | **PUSH BACK** | Investigation-agent feature. |
| Multi-region federation (per-region Raft + global Corrosion) | **PUSH BACK** to platform-scaling phase | Devplat v1 is single-region. Multi-region is an orchestrator-/sovereign-cloud-track deliverable. The trait boundary (IntentStore is already per-region-capable) means nothing blocks this from landing later. |
| `ShardedIntentStore` (Twine-shape) | **KEEP DEFERRED** | Already gated on design partner per whitepaper. No change. |
| Image Factory OCI registry + PXE + dm-verity + TPM attestation + Secure Boot | **PUSH BACK** to platform-scaling phase | Operator-facing. Not devplat. |
| OIDC enrolment bridge for operators | **PUSH BACK** to platform-scaling phase | Operator-facing. |
| Biscuit tokens for CI delegation | **PUSH BACK** to platform-scaling phase | Operator-facing. |
| **NEW: Pages-equivalent git-driven build pipeline** | **NEW — devplat-track Phase 6** | Static + SSR hosting on top of WASM driver + Gateway. Git integration, build pipeline, preview URLs. Large item; flagship for Phase 6. |
| **NEW: Workers AI-equivalent** | **MAYBE — v1.1 at earliest** | Requires GPU scheduling, model store, inference runtime. Possibly deferrable entirely. Research before committing. |
| **NEW: Vectorize-equivalent** | **MAYBE — v1.1 at earliest** | Vector DB primitive. Deferrable unless a Rust-native option composes cleanly (qdrant / LanceDB embedded?). |
| **NEW: Zero Trust Access-equivalent** | **MAYBE — v1.1** | SPIFFE mesh already covers workload-to-workload; user-to-workload Access-shape is a net-new primitive. Evaluate based on actual demand from first reference customers. |

## Net-New Items Not in Current §24

These are the items that the reshape introduces. Each needs owner-naming and a design doc.

1. **Developer CLI surface** (`overdrive deploy`, `overdrive dev`, `overdrive tail`, `overdrive logs`, `overdrive secret`, per-primitive verbs `overdrive kv`, `overdrive r2`, `overdrive d1`, `overdrive queue`). Parallel verb tree to the existing operator CLI in one binary. **Phase 1–5 cross-cutting.**
2. **Bindings ABI** — the WASM host interface that makes `env.KV` etc. work inside a function. **Phase 5.**
3. **Miniflare-equivalent local dev.** `overdrive dev` with hot-reload and bindings stubs. **Phase 5.**
4. **`overdrive-ff` SDK** (Rust / TS / Python) — Apache-2.0 client-side. **Phase 5.**
5. **KV primitive.** CF-shape eventually-consistent key-value over Corrosion. **Phase 4.**
6. **Queue primitive (embedded).** Native Rust pull-based queue over `overdrive-fs`. **Phase 4.**
7. **R2 bindings.** Thin wrapper over Garage with CF-compatible API. **Phase 4.**
8. **D1-shape addressable libSQL.** Per-workload SQLite with cross-workload SPIFFE-identity access. **Phase 4.** (Research done.)
9. **Schedule primitive.** First-class resource, peer to Job/Workflow. **Phase 4.** (Research done.)
10. **EventBus primitive.** Thin Rust trait over `ObservationStore::subscribe`. **Phase 4.** (Research done.)
11. **WASM scale-to-zero.** True idle-zero for WASM functions. **Phase 4.**
12. **Pages-equivalent git-driven build pipeline.** **Phase 6 devplat-track.**

## Proposed Revised Phase Structure

```
Phase 1 — Foundation (Months 1–3)
  Core types + traits; DST harness; Process driver; basic scheduler.
  IntentStore LocalStore; ObservationStore trait.
  Operator CLI + initial developer CLI skeleton (`overdrive deploy` stub).
  Image Factory MVP (minimum single-node installer + one cloud AMI).

Phase 2 — Dataplane + Observation (Months 3–6)
  aya-rs eBPF; XDP routing + SERVICE_MAP; TC egress.
  CorrosionStore (real Corrosion + cr-sqlite).
  BPF map hydration via Corrosion subscriptions.
  §22 Tier 2/3/4 real-kernel CI bootstrapped alongside XDP/TC.

Phase 3 — Identity + Runtime Base (Months 6–9)
  Built-in CA; SPIFFE issuance + rotation; operator identity (basic).
  sockops mTLS + kTLS; BPF LSM.
  Regorus policy evaluation (platform-default set only).
  Workflow primitive (durable async, journal, replay).
  Tier 3 sockops/kTLS/LSM test fixtures.

Phase 4 — Runtime + Developer Primitives (Months 9–14)
  Cloud Hypervisor; virtiofsd; Wasmtime WASM driver.
  WASM sidecar runtime + built-in sidecars.
  Gateway (hyper + rustls) + ACME (instant-acme).
  DuckLake telemetry (basic tail/logs interface).
  Persistent microVMs steps 1–4 (snapshot/restore, overdrive-fs, gateway auto-route, scale-to-zero).
  WASM scale-to-zero.
  * Schedule primitive
  * EventBus primitive
  * KV primitive (Corrosion-backed)
  * D1-shape addressable libSQL
  * R2 bindings (over Garage)
  * Queue primitive (embedded, overdrive-fs-backed)

Phase 5 — Developer Experience (Months 14–18)
  * Bindings ABI (env.KV, env.DB, env.R2, env.QUEUE, env.EVENT, env.SCHEDULE, env.DO)
  * Wrangler-equivalent CLI (overdrive deploy/dev/tail/logs/secret + per-primitive verbs)
  * Miniflare-equivalent local dev
  * `overdrive-ff` SDK (Rust, TS, Python) — Apache-2.0, parallel repo
  Persistent microVM guest agent (step 5).
  * v1 DEVPLAT LAUNCH: `overdrive deploy function.ts` against a single-node box produces a working URL with KV/DB/R2/Queue/Event/Schedule bindings.

Phase 6 — Devplat Breadth (Months 18–24)
  Pages-equivalent git-driven build pipeline.
  Workers AI (gated on GPU scheduling spike).
  Vectorize (gated on Rust-native option availability).
  WASM Component Model SDK; Workflow WASM SDK; OTel export.

Phase 7 — Platform Scaling (Months 18–24, parallel to Phase 6 if team size allows)
  RaftStore (HA mode) + single→HA migration.
  Multi-region federation (per-region Raft + global Corrosion).
  Regional Corrosion clusters + thin global membership.
  Image Factory: OCI registry, PXE, dm-verity + TPM, Secure Boot.
  LLM observability agent + native SRE investigation primitives.
  Self-healing Tier 3; right-sizing; incident memory; predictive scaling.
  Runbook primitive; diagnostic probe catalog.
  OIDC enrolment bridge; Biscuit tokens for CI delegation.
  Mesh VPN extensions (wireguard, tailscale).
  ShardedIntentStore (if design partner materialises).

Phase 8+ — Long tail
  Unikernel drivers (if demand); QEMU opt-in (if demand).
```

## Critical-Path Observations

- **Phase 4 becomes the longest phase** (~5 months). This is unavoidable under devplat-first: the primitives all land together because the bindings ABI in Phase 5 needs them all to wire against. Can be compressed by parallel tracks if team size allows.
- **Phase 5 is the flagship**. This is what ships the pitch. All prior phases are foundation investment that pays off here.
- **Phase 7 runs in parallel with Phase 6** for teams that can afford it. A small team sequences them (devplat breadth first, then platform scaling). A larger team runs two tracks. The dual-framing requires eventually landing both, but the order is enforceable by small-team staffing reality.
- **The §22 kernel-matrix CI investment stays in Phase 2**. Pulling this back would seem like a devplat-first simplification but would compound into untestable eBPF changes by Phase 4. The short-term cost is worth it.

## What Gets Outright Cut (Relative to Current §24)

- **Unikernel drivers**. Defer indefinitely. Zero devplat demand, zero orchestrator-buyer demand.
- **QEMU opt-in driver**. Cut. Exotic hardware emulation is not a roadmap item.
- Nothing else is cut — everything else is just re-sequenced.

## What Gets Deferred to Orchestrator-Track Phase 7

Everything in this list is orchestrator-buyer-facing and does not make the devplat v1 pitch better:

- Multi-region federation
- HA RaftStore (optional later)
- LLM SRE investigation agent + runbook primitive + diagnostic probes
- Right-sizing + predictive scaling
- Incident memory
- OIDC enrolment + Biscuit delegation
- Mesh VPN extensions
- Full Image Factory (OCI registry, PXE, dm-verity, TPM attestation, Secure Boot)
- `ShardedIntentStore`

All of these exist in the whitepaper; none are deleted. They just run on a second track that starts once the devplat v1 is out and the team has evidence that the orchestrator-buyer pipeline deserves active investment.

## Risks to the Reshape

1. **Phase 4 over-scope.** Six net-new primitives (Schedule, EventBus, KV, D1-shape, R2 bindings, embedded Queue) plus pulled-forward persistent-microVM work plus WASM scale-to-zero is a lot. Mitigation: start each primitive at "minimum viable" (KV with single-region eventual-consistency; Queue as single-consumer-group; D1 as single-writer-per-workload). Ship narrow, widen in v1.1.

2. **Bindings ABI debt.** The single biggest engineering item in Phase 5 is the bindings ABI. Under-designing it means every primitive that lands later has to retrofit. Mitigation: design the ABI *before* Phase 4 primitive work starts — even a paper spec counts. The primitives land against a known target shape.

3. **Devplat audience doesn't appear.** If Phase 5 ships and the "OSS Cloudflare" pitch doesn't find traction, the orchestrator-track phases are a sunk-cost recovery story ("but we also replace K8s"). This is the acceptable failure mode — the architecture investment isn't wasted either way. The unacceptable failure mode is shipping the orchestrator pitch first, discovering devplat is where the demand is, and having to refactor the primitive set under time pressure.

4. **CI costs escalate with kernel-matrix + persistent-microVM + all-primitives-together**. Per-PR budget needs monitoring. Mitigation: nightly-only for non-critical matrix kernels; Phase-4 primitive tests run against single-node single-kernel in PR gate; the full matrix runs nightly.

## Not Decided Here

- Whether `overdrive-ff` and the CLI ship from day one under Apache-2.0, or whether the CLI stays under FSL with a carve-out for the SDK. (Prior research recommends Apache-2.0 on all client-side code; decision stands.)
- Exact API surface of the bindings ABI. CF-compatibility-where-possible is the stated goal; the specific divergences (e.g. `env.DO.get()` naming, Queue consumer semantics, KV list pagination shape) need a design doc of their own.
- Whether to fork or integrate Miniflare. Miniflare is Cloudflare-licensed MIT / Apache-2.0 and could be adapted — research before reimplementing from scratch.
- Whether Workers AI and Vectorize are v1.1 items or Phase 8+ items. Gate on first-reference-customer demand signal.

## One-Line Summary

**Keep the whole architecture; move engineering weight from Phase 5/6 intelligence + federation work into Phase 4/5 developer primitives and DX; add ~6 CF-shape primitives and a CLI/SDK that aren't in §24 today; defer orchestrator-buyer-facing features to a parallel Phase 7 track.**
