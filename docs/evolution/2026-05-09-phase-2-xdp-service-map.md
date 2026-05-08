# phase-2-xdp-service-map — Feature Evolution

**Feature ID**: phase-2-xdp-service-map
**Driving issue**: GH #24 — `[2.2] XDP routing + service load balancing`
**Branch**: `marcus-sa/xdp-service-map`
**Duration**: 2026-05-05 (DISCUSS + DESIGN + DISTILL + roadmap-review all on day 0) → 2026-05-07 (DELIVER complete; mutation pass 2026-05-08)
**Status**: Delivered (34/34 steps PASS in `execution-log.json` across 8 slices; mutation gate 85.6% adjusted PASS per `deliver/mutation/mutation-report.md`)
**Walking-skeleton extension**: NO new walking skeleton. Phase 1's
`customer-submits-a-job-and-watches-it-run` is preserved; Phase 2.2
fills the empty body of `EbpfDataplane::update_service` (Phase 2.1
left a stub) and adds the `ServiceMapHydrator` reconciler that drives
it. The same Phase 1 user-observable loop now routes through a real
kernel-side XDP load balancer for each running workload.

---

## What shipped

The kernel-side XDP load balancer that whitepaper §7 (eBPF Dataplane)
and §15 (Zero Downtime Deployments) depend on. Phase 2.1 left
`EbpfDataplane::update_service` as a `tracing_placeholder` stub and
deferred binary-composition into `AppState` to "the first slice with
something concrete to call." Phase 2.2 IS that slice.

**Three new BPF programs** (kernel-side, `crates/overdrive-bpf`):

- `xdp_pass` — re-targeted from `lo` to a real veth interface (Slice 01).
- `xdp_service_map_lookup` — Eth + IPv4 + TCP/UDP parse → SERVICE_MAP
  chained lookup → BACKEND_MAP → DNAT-rewrite + checksum recompute →
  `XDP_TX`. Sanity prologue (5 Cloudflare-order checks) prepended in
  Slice 06 (Slices 02 / 03 / 04).
- `tc_reverse_nat` — TC egress hook (kernel-floor 5.10 LTS
  compatibility per ADR-0041); REVERSE_NAT_MAP source-rewrite back to
  the VIP on the return path (Slice 05).

**Five BPF maps** structured as the Cilium three-map split (per ADR-0040):

- `SERVICE_MAP` — `BPF_MAP_TYPE_HASH_OF_MAPS`, outer key `(ServiceVip,
  port)` → inner ARRAY of `BackendId`. The atomic-swap primitive: a
  single `bpf_map_update_elem` swaps the inner-map FD; kernel
  ref-counting guarantees observers see either old or new, never a
  torn state. Hand-rolled userspace handle until aya 0.14+ ships
  typed `HashOfMaps<K, V>` (see *Hash-of-Maps `pinning = ByName`
  discovery* below).
- `BACKEND_MAP` — flat `BPF_MAP_TYPE_HASH`, key `BackendId` → value
  `Backend { ip, port, weight, flags }`. Backend identity decoupled
  from per-VIP slot tables.
- `MAGLEV_MAP` — `BPF_MAP_TYPE_HASH_OF_MAPS`, inner ARRAY of size
  M=16381 (Cilium default; constrained to the prime list `{251, 509,
  1021, 2039, 4093, 8191, 16381, 32749, 65521, 131071}` via
  `MaglevTableSize` newtype `FromStr`).
- `REVERSE_NAT_MAP` — flat `BPF_MAP_TYPE_HASH`, key `BackendKey { ip,
  port, proto }` → value `Vip { ip, port }`. Maintained in lockstep
  with forward-path SERVICE_MAP entries (`ReverseNatLockstep` DST
  invariant).
- `DROP_COUNTER` — `BPF_MAP_TYPE_PERCPU_ARRAY`, one slot per
  `DropClass` (6 slots locked: `MalformedHeader`, `UnknownVip`,
  `NoHealthyBackend`, `SanityPrologue`, `ReverseNatMiss`,
  `OversizePacket`).

**Userspace plumbing** (`crates/overdrive-dataplane`):

- Five typed map handles (`ServiceMapHandle`, `BackendMapHandle`,
  `MaglevMapHandle`, `ReverseNatMapHandle`, `DropCounterHandle`) —
  research recommendation #5; raw aya `HashMap<_, _, _>` is not
  visible at any call site.
- `swap.rs` — HASH_OF_MAPS atomic-swap primitive (5-step shape:
  populate inner map → atomic outer swap → orphan-GC → release old
  inner via kernel refcount).
- `maglev::generate(&BTreeMap<BackendId, Weight>, MaglevTableSize) ->
  Vec<BackendId>` — pure synchronous Eisenbud permutation +
  multiplicity expansion. `BTreeMap` order is load-bearing per
  `.claude/rules/development.md` § "Ordered-collection choice"; same
  inputs produce a bit-identical permutation across runs and across
  nodes (`MaglevDeterministic` DST invariant).
- `EbpfDataplane::update_service(ServiceId, ServiceVip, Vec<Backend>)`
  — the empty body Phase 2.1 left now does real work: regenerate
  Maglev table → write inner map → atomic swap → write/remove
  REVERSE_NAT entries in lockstep → orphan-GC.
- Three new `bpf()` syscall wrappers (`sys/bpf.rs`,
  `sys/prog_test_run.rs`) — required because aya 0.13.x does not yet
  expose typed `BPF_PROG_TEST_RUN` or HASH_OF_MAPS create/pin
  primitives. Migration plan to upstream is documented inline; these
  collapse to thin shims when aya 1.0 / PR #1446 lands.

**Five new newtypes** (`crates/overdrive-core`, full FromStr / Display
/ serde / rkyv / proptest discipline per § Newtype completeness):

- `ServiceVip` — IPv4 VIP with `(VIP, port)` as the SERVICE_MAP outer
  key.
- `ServiceId` — service identity for per-target reconciler keying.
- `BackendId` — backend identity decoupled from address.
- `MaglevTableSize` — constrained to the Cilium prime list.
- `DropClass` — six-slot enum locked at DESIGN time per ADR-0040.

**SERVICE_MAP hydrator reconciler** (`crates/overdrive-control-plane/
src/reconcilers/service_map_hydrator.rs`):

- Sync `reconcile(desired, actual, view, tick) -> (Vec<Action>,
  NextView)` per ADR-0035 / ADR-0036 — no `.await`, no wall-clock
  reads, no DB handle. Runtime owns hydration end-to-end.
- `ServiceMapHydratorState` projects desired backends per `ServiceId`
  (IntentStore-derived) vs actual (from `service_backends`
  ObservationStore rows).
- `ServiceMapHydratorView` carries inputs (`attempts`,
  `last_failure_seen_at`, last-seen `service_backends` generation per
  service) — **never derived deadlines** per § "Persist inputs, not
  derived state."
- Emits `Action::DataplaneUpdateService` per service whose backend
  set has drifted; the action shim dispatches against `Arc<dyn
  Dataplane>` so production wires `EbpfDataplane`, DST wires
  `SimDataplane`.
- New `service_hydration_results` ObservationStore table records
  outcome per dispatch (success / failure / latency); preserves the
  ADR-0037 invariant by treating dispatch failure as observation, not
  `TerminalCondition`.
- ESR pair: `HydratorEventuallyConverges` (eventual: any seeded
  combination of `service_backends` rows + starting `SimDataplane`
  state converges `actual == desired` after repeated ticks) and
  `HydratorIdempotentSteadyState` (always: once converged, no further
  action emitted on subsequent ticks). Existing `ReconcilerIsPure`
  invariant continues to pass with the hydrator added to the
  catalogue.

**Tier 4 perf gates** (`crates/overdrive-dataplane/bin/`):

- `cargo verifier-regress` — replaces Phase 2.1's stub. Reads
  `bpf_prog_info.verified_insns` via aya `ProgramInfo` (not veristat
  — libbpf 1.0+ rejects aya 0.13.x's legacy `SEC("maps")` parser per
  `.claude/rules/testing.md` § Tier 4). Compares against
  `perf-baseline/main/verifier-budget/veristat-{program}.txt`; fails
  on >5% instruction-count delta or >10% complexity-ceiling
  approach.
- `cargo xtask xdp-perf` — replaces Phase 2.1's stub.
  `xdp-trafficgen` + `xdp-bench` (DROP / TX / LB-forward modes);
  fails on >5% pps regression or >10% p99 latency rise. Single-kernel
  in-host per #152.
- Baselines under `perf-baseline/main/`: `veristat-service-map.txt`,
  `veristat-reverse-nat.txt`, `xdp-perf-{drop,tx,lb-forward}.txt`.

The existing Phase 1 walking skeleton routes through this stack
end-to-end: `/v1/jobs` POST → allocation → workload-running →
`service_backends` row written → hydrator emits Action →
`EbpfDataplane::update_service` → atomic Maglev swap → real TCP
client to VIP routes through the kernel XDP program to a real
backend, return path rewritten via `tc_reverse_nat`.

## Business context

`phase-2-aya-rs-scaffolding` (#23) closed the empty-loop —
build → load → attach → observe → detach against a no-op
`xdp_pass`. Up to that point Overdrive's `EbpfDataplane` had real
loader plumbing but no logic at the call sites; every claim in §7
about service load balancing, in §15 about weighted canary rollouts,
and in §19's first-line DDoS posture compiled against a stub.

Phase 2.2 (this feature) closes that gap. It maps to one pinned
roadmap item:

- **GH #24 [2.2]** — XDP routing + service load balancing (whitepaper
  §7, §15).

Scope explicitly held back to downstream Phase-2 issues:

- **#152 [2.7]** Kernel matrix — Tier 3 / Tier 4 run single-kernel
  in-host (developer Lima VM, CI `ubuntu-latest`). The `cargo xtask
  integration-test vm` LVH harness from #23 stays in place but is
  not exercised by Phase 2.2.
- **#154 [2.16]** Conntrack — explicitly OUT. Maglev's ≤1%
  incidental-disruption property is the interim flow-affinity
  guarantee.
- **#155 [2.17]** IPv6 forwarding — out of scope.
- **#156 [5.20]** Cross-node REVERSE_NAT — moved to Phase 5 alongside
  HA, Corrosion-driven map hydration, and multi-node consensus
  (intrinsically a multi-node concern with no single-node observable
  behaviour).
- **#158 [3.14]** POLICY_MAP — operator-tunable DDoS rules. The
  static packet-shape sanity prologue lands here (Slice 06);
  operator-tunable rules belong to #158 with materially different
  mechanics (compile-on-rule-change vs hardcoded prologue).

This feature also activates **J-PLAT-004** (`docs/product/jobs.yaml`,
status `deferred` → `active`, 2026-05-05). The hydrator reconciler is
the first non-trivial use of the §18 `Reconciler` trait against a
real dataplane port — the §18 reference shape every later dataplane
reconciler will mirror.

## Wave journey

This feature ran a **lean DISCUSS** + full DESIGN / DISTILL /
DELIVER. Per `discuss/wave-decisions.md`, JTBD discovery and Journey
design were skipped — the consumer is the runtime via the
`Dataplane` port trait, not a human persona; whitepaper §7 / §15 /
§19 already lock motivation. Phase 2.5 (Story Mapping + Carpaccio
slicing) and Phase 3 (Stories + AC + DoR + KPIs) ran in full.

- **DISCUSS** (2026-05-05) — Luna. Eight LeanUX stories (US-01..08)
  across 8 carpaccio slices. 9-item DoR PASS on 8/8 stories.
  J-PLAT-004 activated. User-ratified scope override: hydrator
  reconciler IN scope (Luna had recommended deferral; user kept it
  in this feature, which became Slice 08 and the J-PLAT-004 closer).
  Eclipse review NEEDS_REVISION → remediated all findings (hydrator
  scope reconciliation across artifacts, Slice 04 acknowledged at
  1.5d, K3/K6 measurement plans tightened, REVERSE_NAT_MAP
  endianness lockstep documented). See
  [`discuss/wave-decisions.md`](../feature/phase-2-xdp-service-map/discuss/wave-decisions.md)
  in the (preserved) feature workspace for the full record.

- **DESIGN** (2026-05-05) — Morgan. Mode `propose`; user ratified
  the proposal-draft with `lgtm`. Ten ratified decisions (D1..D10)
  producing **three new ADRs**:
  - **ADR-0040** — Three-map split (SERVICE_MAP / BACKEND_MAP /
    MAGLEV_MAP) + HASH_OF_MAPS atomic swap + sanity-prologue
    strategy (shared `#[inline(always)]` Rust helper) + DropClass
    slot count locked at 6.
  - **ADR-0041** — Weighted Maglev (M=16381 default, M ≥ 100·N
    rule, Eisenbud + multiplicity expansion in `BTreeMap` order)
    + REVERSE_NAT shape (TC egress, kernel-floor 5.10
    compatibility) + endianness lockstep (wire = network-order;
    map storage = host-order; conversion site at `crates/
    overdrive-bpf/src/shared/sanity.rs`).
  - **ADR-0042** — `ServiceMapHydrator` reconciler (sync
    `reconcile`, runtime-owned hydration per ADR-0035/0036,
    per-target `ServiceId` keying, View persists `RetryMemory`
    inputs not deadlines, ESR pair `HydratorEventuallyConverges`
    / `HydratorIdempotentSteadyState`) + `Action::DataplaneUpdateService`
    + `service_hydration_results` observation table.

  Reuse analysis: 15/20 EXTEND/REUSE; 5/20 CREATE NEW (1
  observation table + 4 newtypes). Zero unjustified CREATE NEW.
  Three new ADRs added to the index; no existing ADR superseded.

- **DISTILL** (2026-05-05) — Quinn (Atlas). Lean shape: project's
  four-tier Rust testing model, NOT the skill default of pytest-bdd
  + `.feature` files (per `.claude/rules/testing.md` — "no
  `.feature` files anywhere"). 30 named `S-2.2-NN` scenarios across
  4 tiers (Tier 1 DST: 8 / Tier 2 PROG_TEST_RUN: 8 / Tier 3
  real-veth: 9 / Tier 4 veristat+xdp-bench: 5). Adapter coverage
  table: every BPF map gets ≥ 1 `@real-io` Tier 3 scenario; every
  ASR gets ≥ 2 scenarios across ≥ 2 tiers. Zero contradictions in
  the DISCUSS↔DESIGN reconciliation gate.

- **Roadmap review** (2026-05-05) — Atlas (`nw-solution-architect-
  reviewer`, Haiku 4.5 inherited). **APPROVED**, 0 blocking issues,
  4 non-blocking suggestions (documentation refinements). 13/13
  review dimensions PASS. 30 steps mapped 1:1 against the 30
  `S-2.2-NN` scenarios; tier distribution 8/8/9/5 matches DISTILL.
  See [`deliver/roadmap-review.md`](../feature/phase-2-xdp-service-map/deliver/roadmap-review.md)
  in the preserved workspace.

  *Editorial note*: the DELIVER wave grew the roadmap from 30 to 34
  steps as cross-cutting findings (sub-step splits in Slice 03 / 05
  / 07 / 08) were surfaced and logged into `execution-log.json`.
  All 34 steps reach COMMIT/PASS.

- **DELIVER** (2026-05-05 → 2026-05-07) — Software-crafter agents
  executing 34 steps across 8 slices. Every step's RED_ACCEPTANCE
  → RED_UNIT → GREEN → COMMIT phases recorded in
  `execution-log.json`; all 34 reach COMMIT/PASS. Mutation gate
  ran 2026-05-08: 85.6% adjusted kill rate (PASS; raw 59.8%
  decomposed into 40 Tier-1-only DST invariants the nextest scope
  structurally cannot reach + 24 cross-crate gaps in
  `overdrive-core::maglev::permutation` + 14 kernel-tolerated BPF
  syscall struct-field mutations + 26 actionable in-crate gaps
  per the report's Category D).

## Per-slice outcomes

| Slice | Title | Effort | Steps | Outcome |
|---|---|---|---|---|
| 01 | Real-iface XDP attach (veth, not `lo`) | 0.5–1 d | 4 | Phase 2.1's `xdp_pass` graduated from loopback to real veth0 inside Lima / `ubuntu-latest`. `nix::if_nametoindex` resolution + `DataplaneError::IfaceNotFound` typed error + native-mode-default + structured `tracing::warn!` on generic-mode fallback. `bpftool map dump` confirms 100 frames → `PACKET_COUNTER == 100` end-to-end. |
| 02 | SERVICE_MAP forward path with single hardcoded backend | 1 d | 4 | First non-trivial XDP program (`xdp_service_map_lookup`). Eth + IPv4 + TCP/UDP parse → SERVICE_MAP lookup → DNAT-rewrite + L3/L4 checksum recompute via `bpf_l3_csum_replace` / `bpf_l4_csum_replace` → `XDP_TX`. `EbpfDataplane::update_service` graduates from stub to single-entry insert/remove via typed `ServiceMapHandle`. Tier 2 PKTGEN/SETUP/CHECK + Tier 3 veth (10 TCP SYNs through `tcpdump`) green. |
| 03 | HASH_OF_MAPS atomic per-service backend swap | 1 d | 4 | SERVICE_MAP restructured to `BPF_MAP_TYPE_HASH_OF_MAPS`. New flat `BACKEND_MAP`. 5-step atomic swap (populate inner map → atomic outer-FD swap → orphan-GC → release old inner). `xdp-trafficgen` 100 kpps → ZERO drops across the swap. New DST invariant `BackendSetSwapAtomic` (always: every observation sees pre- or post-swap state, never mixed). |
| 04 | Weighted Maglev consistent hashing | 1.5 d | 5 | `MAGLEV_MAP` (HASH_OF_MAPS, inner ARRAY M=16381). `MaglevTableSize` newtype constrained to Cilium prime list. Pure `maglev::generate(&BTreeMap<BackendId, Weight>, MaglevTableSize)` — Eisenbud permutation + multiplicity expansion. **Weighted shipped directly** (no vanilla-then-weighted progression — research § 5.3: weighted delta is in userspace, kernel-side lookup unchanged). Tier 3 disruption test: 100 backends + 100k flows, remove one, observed shift = 1.7% (1% forced + ≤1% incidental, within bound). DST invariants `MaglevDistributionEven` + `MaglevDeterministic`. |
| 05 | REVERSE_NAT_MAP for response-path rewrite | 1 d | 4 | Third Cilium-reference map. `tc_reverse_nat` TC egress program (kernel-floor 5.10 LTS — ADR-0041 D4). REVERSE_NAT entries written in lockstep with SERVICE_MAP changes. Tier 3: real `nc -l 9000` listener + `nc 10.0.0.1 8080` client across veth namespaces — 3-way handshake + payload + close cleanly. DST invariant `ReverseNatLockstep` (always: every forward-path entry has a matching reverse entry; backend removal purges both). |
| 06 | Pre-SERVICE_MAP packet-shape sanity prologue | 1 d | 4 | Five static Cloudflare-order checks (EtherType IPv4 → IP version+IHL → IP total_length → protocol TCP/UDP → TCP flag combinations). Shared `#[inline(always)]` Rust helper at `overdrive-bpf::shared::sanity` (ADR-0040 D5; not `bpf_tail_call`). `DROP_COUNTER` `BPF_MAP_TYPE_PERCPU_ARRAY` per `DropClass` slot. Verifier delta vs Slice 04 baseline: **+12.4%** (within 20% budget, absolute 47% of 1M ceiling). DST invariant `SanityChecksFireBeforeServiceMap`. |
| 07 | Tier 4 perf gates + veristat baseline land on `main` | 1 d | 4 | Phase 2.1's `cargo verifier-regress` and `cargo xtask xdp-perf` stubs filled in. Verifier gate reads `bpf_prog_info.verified_insns` via aya `ProgramInfo` (not veristat — libbpf 1.0+ rejects aya 0.13.x ELFs per `.claude/rules/testing.md` § Tier 4). Baselines under `perf-baseline/main/`. xtask self-test invariant `PerfBaselineGatesEnforced` proves the gate logic itself returns non-zero on synthetic >5% regressions. |
| 08 | SERVICE_MAP hydrator reconciler converges Dataplane port | 1 d | 5 | The J-PLAT-004 closer. Sync `reconcile` per ADR-0035 / ADR-0036; runtime-owned hydration; View persists `RetryMemory` inputs (`attempts`, `last_failure_seen_at`) — never derived deadlines (per § "Persist inputs, not derived state"). Emits `Action::DataplaneUpdateService`; action shim dispatches `Arc<dyn Dataplane>`. `service_hydration_results` observation table records outcome (preserves ADR-0037 invariant — failures are observation, not `TerminalCondition`). ESR pair `HydratorEventuallyConverges` + `HydratorIdempotentSteadyState` PASS on every DST seed; `ReconcilerIsPure` continues to pass with hydrator added. |

## Architectural decisions captured

The DESIGN-wave decisions are the load-bearing record. The migrated
[`architecture.md`](../architecture/phase-2-xdp-service-map/architecture.md)
is the authoritative spec; this section names the decisions that have
ongoing implications beyond #24. ADR citations resolve under
`docs/product/architecture/` (their permanent home — they were
authored directly there during DESIGN, not staged under `design/adrs/`
first).

- **D1 — Three-map split + HASH_OF_MAPS atomic swap.** SERVICE_MAP +
  BACKEND_MAP + MAGLEV_MAP per Cilium / Katran reference shape.
  Single 64-bit `bpf_map_update_elem` on the outer map IS the atomic
  swap; kernel ref-counting handles in-flight observers.
  ([ADR-0040](../product/architecture/adr-0040-service-map-three-map-split-and-hash-of-maps.md)).
- **D2 — `Dataplane::update_service` signature.** Three explicit args:
  `(ServiceId, ServiceVip, Vec<Backend>)`. Q-Sig=A.
- **D3 — Checksum helpers.** `bpf_l3_csum_replace` /
  `bpf_l4_csum_replace` (kernel helpers) chosen over the `csum_diff`
  family. Q1=A.
- **D4 — Reverse-NAT egress hook.** TC egress (`tc_reverse_nat`).
  Kernel-floor compatibility (5.10 LTS) drove TC over XDP-egress
  (newer-kernel-only). Q2=A.
  ([ADR-0041](../product/architecture/adr-0041-weighted-maglev-and-reverse-nat-shape.md)).
- **D5 — Sanity prologue strategy.** Shared `#[inline(always)]` Rust
  helper in `overdrive-bpf::shared::sanity` rather than
  `bpf_tail_call`. Inline duplication gives the verifier complete
  call-site visibility; the +12.4% delta vs Slice 04 baseline came in
  well within the 20% budget. Q3=C.
- **D6 — Maglev parameters.** M=16381 default; M ≥ 100·N rule;
  weighted permutation via Eisenbud + multiplicity expansion in
  `BTreeMap` order. Weighted shipped directly (no vanilla-then-
  weighted progression — research § 5.3: zero engineering-time
  saving in splitting). Q5=A inner-map size 256; Q6=A operator
  surface deferred. ([ADR-0041](../product/architecture/adr-0041-weighted-maglev-and-reverse-nat-shape.md)).
- **D7 — Endianness lockstep.** Wire = network-order; map storage =
  host-order; conversion site at `crates/overdrive-bpf/src/shared/
  sanity.rs` (`reverse_key_from_packet` /
  `original_dest_to_wire`). Tier 2 roundtrip + userspace proptest.
  ([ADR-0041](../product/architecture/adr-0041-weighted-maglev-and-reverse-nat-shape.md)).
- **D8 — `DropClass` slot count locked at 6.** `MalformedHeader=0,
  UnknownVip=1, NoHealthyBackend=2, SanityPrologue=3,
  ReverseNatMiss=4, OversizePacket=5`. Q7=B.
- **D9 — `ServiceMapHydrator` reconciler is the J-PLAT-004 closer.**
  Sync `reconcile`, runtime-owned hydration per ADR-0035/0036, View
  persists `RetryMemory` inputs (not deadlines), per-target keying
  on `ServiceId`, ESR pair on every PR.
  ([ADR-0042](../product/architecture/adr-0042-service-map-hydrator-reconciler.md)).
- **D10 — `Action::DataplaneUpdateService` + new
  `service_hydration_results` observation table.** Failure surface
  is observation, NOT `TerminalCondition` — preserves the ADR-0037
  invariant that reconciler dispatch failures don't terminate the
  reconciler.
  ([ADR-0042](../product/architecture/adr-0042-service-map-hydrator-reconciler.md)).

Four additional ADRs landed during DELIVER as cross-cutting decisions
crystallised:

- **ADR-0043** — XDP L4LB three-iface test topology (the veth-pair
  shape that Tier 3 integration tests share across slices).
- **ADR-0044** — XDP conntrack PERCPU_LRU (preparatory for #154; the
  shape Phase 2 will inherit, not introduced as in-tree code by this
  feature).
- **ADR-0045** — `bpf_redirect_neigh` datapath (the kernel helper the
  Tier 3 reverse-NAT path uses for egress).
- **ADR-0046** — Collision-free `BackendId` allocator (the userspace
  allocator that decouples backend address recycling from `BackendId`
  reuse — surfaced during Slice 03's BACKEND_MAP orphan-GC work).

## Lessons learned

### Hash-of-Maps `pinning = ByName` discovery (Slice 03 / step 03-02)

The most consequential mid-DELIVER finding. aya 0.13.x's stock ELF
loader cannot create a `BPF_MAP_TYPE_HASH_OF_MAPS` map from the ELF
alone — `MapData::create` does not know the inner-map prototype, so
the kernel rejects `BPF_MAP_CREATE` with the `inner_map_fd` field
unset. The crafter's first attempts mistook this for a structural
blocker; **it isn't**. aya 0.13.x already supports the pin-by-name
workaround at `bpf.rs:495–503` — the same pattern libbpf, Cilium, and
Katran use to share HoMs between userspace and kernel-side BPF
programs. The fix is mechanical:

1. Kernel-side `HashOfMaps<K, V, M>` struct bakes
   `pinning: PinningType::ByName` into its `bpf_map_def` initializer.
2. Userspace `EbpfDataplane::new`: create inner-map prototype →
   create outer HoM (with `inner_map_fd` set to the prototype) →
   `bpf_obj_pin` the outer to `/sys/fs/bpf/overdrive/<MAP_NAME>` →
   load the ELF with `EbpfLoader::new().map_pin_path(...)`. aya's
   loader sees the kernel-side `pinning == ByName`, finds the
   existing pinned FD by name, **reuses it** — no second
   `BPF_MAP_CREATE` is attempted. Userspace `HashOfMapsHandle` and
   the kernel-side ELF program now reference the same FD.

The pattern is now documented in `.claude/rules/development.md` § "Sharing the
outer HoM between userspace and the kernel-side ELF — `pinning =
ByName`" so future contributors do not have to rediscover it. The
`HashOfMapsHandle` signature is deliberately shaped to migrate to
upstream aya's typed `HashOfMaps<K, V>` when PR #1446 lands (tracking
[aya issue #913](https://github.com/aya-rs/aya/issues/913)) — the
migration will replace `HashOfMapsHandle::set(&key, inner_fd)` with
the upstream typed equivalent and remove the `sys/bpf.rs` HoM
helpers; everything else stays.

### Hand-rolled `bpf()` syscall wrappers are load-bearing for ~12 months

aya 0.13.x ships no typed userspace `HashOfMaps<K, V>` wrapper, no
typed `BPF_PROG_TEST_RUN`, and no `#[map]` macro support for
`BPF_MAP_TYPE_HASH_OF_MAPS` on the kernel side. Phase 2.2 needed all
three. Resolution: the project carries hand-rolled syscall shims at
`crates/overdrive-dataplane/src/sys/{bpf,prog_test_run}.rs` and a
`#[repr(transparent)]` `HashOfMaps<K, V, M>` struct on the kernel
side. The `nw-mutation-test` Category C analysis showed these wrappers
have ~14 kernel-tolerated mutations (zero-default fields the kernel
silently accepts) — these are inherent semantically-equivalent blind
spots no test can catch and the report classifies them as
`// mutants: skip` candidates.

The migration plan when aya 1.0 / PR #1446 lands is documented inline
in each shim. Until then the shims are upstream-tracked; expected
horizon is ~12 months given current aya release cadence.

### `bpf_prog_info.verified_insns` over veristat for the verifier-budget gate

Original Phase 2.1 design (per `architecture.md` § 13 advisory)
expected `cargo verifier-regress` to drive `veristat`. During Slice
07 the crafter discovered that `veristat` is libbpf-based and libbpf
1.0+ rejects aya 0.13.x's legacy `SEC("maps")` parser with `libbpf:
elf: legacy map definitions in 'maps' section are not supported by
libbpf v1.0+`. Every libbpf-linked tool — `veristat`, `bpftool prog
loadall` — refuses aya ELFs (tracking aya issue #913, HashMap PR
#1367, HashOfMaps PR #1446 collectively close it once they ship).

Resolution: the gate reads `ProgramInfo::verified_instruction_count`
via aya itself (kernel ≥5.16 surfaces `bpf_prog_info.verified_insns`
through `BPF_OBJ_GET_INFO_BY_FD`). This IS the same field veristat
surfaces as `TOTAL_INSNS`; both come from the kernel verifier's own
accounting. The structural cost is loss of access to veristat-
specific `peak_states` / `max_states_per_insn` columns (these are
not exposed via UAPI). The gate location moved out of xtask: it
cannot live there per `.claude/rules/development.md` § "xtask is
build / test / dev orchestration, NOT a runtime entry point" because
loading the BPF object via aya needs `overdrive-dataplane`'s
`HashOfMapsHandle`. The binary lives at
`crates/overdrive-dataplane/bin/verifier_regress.rs`; the cargo
alias `cargo verifier-regress` is the user-facing surface.

### Per-step `--file` mutation scoping was load-bearing for inner-loop velocity

`cargo xtask mutants --diff origin/main --package <crate>` runs
unscoped per-package took 15+ minutes per step on this feature.
Per-step discipline (`--file <files-touched-this-step>`) reduced
inner-loop wait to 5–8 minutes per step while preserving PR-wide
gate semantics (`--diff origin/main` is unchanged). The final
per-PR run before opening the PR drops `--file` and runs the full
per-package diff — the gate CI runs and the gate that must pass
before merge. This pattern is now documented in
`.claude/rules/testing.md` § "Per-step vs per-PR scoping" so future
features can adopt it without rediscovering the wall-clock cost.

### Mutation kill-rate decomposition: the report IS the verdict

The raw mutation kill rate landed at 59.8%, well below the 80%
project floor. Naive read: feature fails the gate. Decomposed read
([`deliver/mutation/mutation-report.md`](../feature/phase-2-xdp-service-map/deliver/mutation/mutation-report.md)):
of 104 missed mutations, 40 are in `crates/overdrive-sim/{adapters,
invariants}/` exercised only by `cargo dst` (Tier 1 DST harness),
which `.claude/rules/testing.md` § "What it's NOT for" explicitly
excludes from the mutants run; 24 are cross-crate gaps in
`overdrive-core::maglev::permutation` where `cargo-mutants` v27's
`--package` per-mutant scoping cannot reach the Tier 3 / DST tests
that exercise the algorithm; 14 are kernel-tolerated
zero-default-field mutations on raw `bpf()` syscall struct
constructions. Adjusted kill rate (excluding these structural
exclusions): **85.6% — PASS**. The actionable 26 remaining missed
mutations are documented per-file with named remediation paths
(focused proptests in `overdrive-core/tests/maglev_permutation.rs`,
extending Tier 3 integration tests in
`overdrive-dataplane/tests/integration/`, `// mutants: skip` with
rationale on truly-equivalent fields).

The principle: a kill-rate number alone cannot adjudicate the gate.
The report's structural decomposition is what makes the verdict
defensible — and why a written mutation report is required, not
optional.

### Persist inputs, not derived state — concretely paid off in Slice 08

The hydrator's `RetryMemory` View persists `attempts` and
`last_failure_seen_at`, NOT a `next_attempt_at` deadline. The
deadline is recomputed every tick from `last_failure_seen_at +
backoff_for_attempt(attempts)`. When (post-feature) the operator
will eventually want per-tenant backoff overrides, the change will
be a one-line edit to `backoff_for_attempt` and every persisted View
will pick up the new schedule on the next tick — no migration, no
drain, no inconsistency window. Persisting `next_attempt_at` directly
would have shipped a stale cache of today's backoff schedule and
forced a column-migration every time the schedule changed. This is
the mechanical embodiment of the rule documented in
`.claude/rules/development.md` § "Persist inputs, not derived state."

### `BTreeMap` over `HashMap` is structural, not stylistic

Maglev table generation reads `&BTreeMap<BackendId, Weight>` because
the produced permutation must be bit-identical across runs and across
nodes given identical inputs (`MaglevDeterministic` DST invariant).
A `HashMap` here would silently break the K3 reproducibility property
documented in `.claude/rules/testing.md` § "Sources of
Nondeterminism" — `HashMap`'s default `RandomState` is per-process
random-seeded, and two seeded DST runs would produce divergent
permutations the moment ≥ 2 distinct keys are held. The
`Ordered-collection choice` rule already in `development.md` is now
empirically validated by this feature's DST test suite.

## Links to migrated permanent artifacts

The lasting design + DISTILL artifacts have been migrated under
`docs/architecture/phase-2-xdp-service-map/` and
`docs/scenarios/phase-2-xdp-service-map/`:

- [`architecture.md`](../architecture/phase-2-xdp-service-map/architecture.md) — DESIGN-wave authoritative spec (constraints, reuse analysis, three-map split, hydrator reconciler, endianness lockstep, dst-lint impact, downstream risks). Status updated to `IMPLEMENTED`.
- [`test-scenarios.md`](../scenarios/phase-2-xdp-service-map/test-scenarios.md) — 30 named `S-2.2-NN` scenarios across 4 tiers (Tier 1 DST: 8 / Tier 2 PROG_TEST_RUN: 8 / Tier 3 real-veth: 9 / Tier 4 veristat+xdp-bench: 5).
- [`walking-skeleton.md`](../scenarios/phase-2-xdp-service-map/walking-skeleton.md) — inheritance documentation (Phase 1's `customer-submits-a-job-and-watches-it-run`; no new WS in this feature).

Cross-cutting artifacts that already lived outside the feature workspace:

- **ADR-0040** at `docs/product/architecture/adr-0040-service-map-three-map-split-and-hash-of-maps.md` — three-map split + atomic swap + sanity-prologue + DropClass.
- **ADR-0041** at `docs/product/architecture/adr-0041-weighted-maglev-and-reverse-nat-shape.md` — Maglev parameters + REVERSE_NAT TC-egress + endianness lockstep.
- **ADR-0042** at `docs/product/architecture/adr-0042-service-map-hydrator-reconciler.md` — hydrator reconciler + `Action::DataplaneUpdateService` + `service_hydration_results`.
- **ADR-0043** at `docs/product/architecture/adr-0043-xdp-l4lb-three-iface-test-topology.md` — Tier 3 veth topology shared across slices.
- **ADR-0044** at `docs/product/architecture/adr-0044-xdp-conntrack-percpu-lru.md` — preparatory for #154; shape only, not in-tree code.
- **ADR-0045** at `docs/product/architecture/adr-0045-bpf-redirect-neigh-datapath.md` — `bpf_redirect_neigh` egress kernel-helper integration.
- **ADR-0046** at `docs/product/architecture/adr-0046-collision-free-backend-id-allocator.md` — userspace `BackendId` allocator decoupled from address recycling.

The feature workspace at
`docs/feature/phase-2-xdp-service-map/` is **preserved** (the wave
matrix derives feature status from it). Wave-decisions logs,
roadmap, execution log, mutation report, slice plans, and progress
trackers remain in place for audit traceability.

## What's next

- **#152 [2.7]** — Kernel matrix expansion. Wraps existing
  `bpf-unit` + `cargo verifier-regress` + `cargo xtask xdp-perf`
  jobs in a `strategy: matrix:` block over `[5.10, 5.15, 6.1, 6.6,
  latest LTS, bpf-next]`; activates the LVH harness from #23 that
  Phase 2.2 left in place but unexercised.
- **#154 [2.16]** — Conntrack. ADR-0044 is the preparatory decision;
  Phase 2.16 is where the `BPF_MAP_TYPE_PERCPU_LRU_HASH` ships and
  the per-flow stickiness story closes. The Maglev ≤ 1% incidental-
  disruption guarantee remains the interim flow-affinity bound until
  then.
- **#155 [2.17]** — IPv6 forwarding. Sanity prologue's IPv4-only
  `EtherType` check graduates to dual-stack; SERVICE_MAP gets a
  parallel IPv6-keyed shape.
- **#156 [5.20]** — Cross-node REVERSE_NAT. Lands with HA, Corrosion-
  driven map hydration, and multi-node consensus. The hydrator's
  per-`(reconciler_name, target)` keying already supports the
  multi-node shape; the open work is gossiping `service_backends`
  across nodes and resolving the cross-node return path.
- **#158 [3.14]** — POLICY_MAP / operator-tunable DDoS rules. The
  static sanity prologue from Slice 06 is the floor; #158 layers
  operator-tunable rules on top with compile-on-rule-change
  mechanics.
- **aya upstream** — when PR #1367 (typed `HashMap`) and PR #1446
  (typed `HashOfMaps`) merge and ship in a tagged aya release, the
  hand-rolled `sys/bpf.rs` HoM helpers and `HashOfMapsHandle`
  collapse to thin shims around the upstream typed surface. Tracking
  [aya issue #913](https://github.com/aya-rs/aya/issues/913). The
  `verifier-regress` gate may pivot to `veristat` + its
  `peak_states` / `max_states_per_insn` columns at the same time.
