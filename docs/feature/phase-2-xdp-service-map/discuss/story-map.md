# Story Map — phase-2-xdp-service-map

## User: Ana, Overdrive platform engineer (distributed-systems SRE)

## Goal: Land the first non-trivial XDP forwarding path so the platform can claim § 7's "service load balancing — O(1) BPF map lookup for VIP-to-backend resolution, replacing kube-proxy entirely" with evidence — and so § 15's "weighted backends (e.g., 95% v1, 5% v2)" zero-drop atomic-swap claim is demonstrated end-to-end against a real kernel.

This feature **builds on** the eBPF scaffolding landed by
`phase-2-aya-rs-scaffolding` (#23). It does NOT introduce a new
walking skeleton — Phase 1's walking skeleton (`submit-a-job.yaml`)
already shipped and Phase 2.1 added the `Dataplane` port adapter
behind it. This feature **fills the body** of the dataplane port: it
graduates `EbpfDataplane::update_service` from no-op stub to a real
five-step atomic-swap implementation, and brings the SERVICE_MAP /
BACKEND_MAP / MAGLEV_MAP / REVERSE_NAT_MAP shape that § 7 claims into
existence.

> **Phase 2.2 is single-kernel in-host.** Per #152 (deferred
> nested-VM kernel matrix). Tier 3 / Tier 4 integration runs inside
> the developer's local Lima VM and on CI's `ubuntu-latest` runner.
> The kernel-matrix work lands later in Phase 2.

> **Conntrack is OUT of scope.** Per #154 (deferred conntrack /
> sockops slice). Phase 2.2 ships a stateless Maglev forwarder; flow
> affinity is the ≤ 1% Maglev disruption guarantee. Conntrack lands
> later in Phase 2.

## Backbone

User activities, left-to-right in chronological order over the
lifetime of the feature:

| 1. Attach to a real iface | 2. Forward to a backend | 3. Swap backends atomically | 4. Distribute consistently | 5. Close the return path | 6. Drop pathological traffic | 7. Enforce perf gates |
|---|---|---|---|---|---|---|
| Lift Phase 2.1's `xdp_pass` from `lo` to a real veth pair (and to virtio-net inside Lima); prove the loader's iface-resolution + native-mode-attach + structured-warning shape works against a non-loopback driver | Land `xdp_service_map_lookup` with single-VIP `BPF_MAP_TYPE_HASH` lookup, DNAT rewrite, `XDP_TX`. Verifier-clean against the 1M-ceiling | Restructure to `BPF_MAP_TYPE_HASH_OF_MAPS` per Cilium's atomic-swap shape. Prove the § 15 zero-drop claim end-to-end under `xdp-trafficgen` load | Replace random slot selection with weighted Maglev. Prove the ≤ 1% disruption property and ± 2% weighted-distribution accuracy | Add `REVERSE_NAT_MAP` and the egress-side rewrite program. Real `nc` connection completes end-to-end across a veth pair | Insert the Cloudflare-shape sanity prologue (EtherType / IP / proto / TCP flag) before SERVICE_MAP lookup. Per-class drop counter; verifier delta < 20% | Land `cargo xtask verifier-regress` and `cargo xtask xdp-perf` real implementations. Baseline files under `perf-baseline/main/`. CI gates real |

## Ribs (tasks under each activity)

### 1. Attach to a real iface

- 1.1 veth pair creation/teardown helpers in `crates/overdrive-dataplane/tests/integration/`
- 1.2 Loader iface-name → ifindex resolution with `DataplaneError::IfaceNotFound`
- 1.3 Native-mode default with structured `tracing::warn!` on generic-mode fallback
- 1.4 Re-target Phase 2.1's `xdp_pass` and `PACKET_COUNTER` to the veth pair

### 2. Forward to a backend

- 2.1 `xdp_service_map_lookup` program: Eth + IPv4 + TCP/UDP parser, SERVICE_MAP lookup, DNAT rewrite, `XDP_TX`
- 2.2 IP / TCP / UDP checksum recompute (via `bpf_l3_csum_replace` / `bpf_l4_csum_replace` or `csum_diff`)
- 2.3 `SERVICE_MAP` declared as `BPF_MAP_TYPE_HASH` keyed by `ServiceKey`, value `Backend`
- 2.4 STRICT newtypes `ServiceVip`, `ServiceId` in `overdrive-core`
- 2.5 Typed `ServiceMapHandle` newtype in `overdrive-dataplane` wrapping aya `HashMap`
- 2.6 `EbpfDataplane::update_service` graduates from no-op stub to real single-entry insert/remove
- 2.7 Tier 2 PKTGEN/SETUP/CHECK triptych
- 2.8 Tier 3 veth integration test (10 frames forwarded)

### 3. Swap backends atomically

- 3.1 SERVICE_MAP restructured to `BPF_MAP_TYPE_HASH_OF_MAPS`; outer key `ServiceId`, inner `BPF_MAP_TYPE_ARRAY` of `BackendId`
- 3.2 `BACKEND_MAP` declared as `BPF_MAP_TYPE_HASH` keyed by `BackendId`, value `Backend { ip, port, weight, flags }`
- 3.3 STRICT newtype `BackendId` in `overdrive-core`
- 3.4 `EbpfDataplane::update_service` five-step swap: BACKEND_MAP upsert → fresh inner map → outer-pointer swap → orphan GC → release
- 3.5 `SimDataplane::update_service` mirrors atomic-swap semantics (BTreeMap reassignment under one mutex acquisition)
- 3.6 DST invariant `BackendSetSwapAtomic`
- 3.7 Tier 3 atomic-swap zero-drop test under `xdp-trafficgen` 100 kpps load
- 3.8 BACKEND_MAP orphan-GC integration test

### 4. Distribute consistently

- 4.1 `MAGLEV_MAP` declared as `BPF_MAP_TYPE_HASH_OF_MAPS`; inner `BPF_MAP_TYPE_ARRAY` size `M=16381` (default)
- 4.2 STRICT newtype `MaglevTableSize` constrained to Cilium prime list
- 4.3 Pure `maglev::generate(&BTreeMap<BackendId, Weight>, MaglevTableSize) -> Vec<BackendId>` in `overdrive-dataplane`
- 4.4 Weighted-Maglev variant (Eisenbud per-backend slot-multiplicity)
- 4.5 XDP program switches lookup from random-slot to MAGLEV_MAP-indexed
- 4.6 `EbpfDataplane::update_service` regenerates Maglev table on every backend-set change, swaps via Slice 03 shape
- 4.7 DST invariants `MaglevDistributionEven`, `MaglevDeterministic`
- 4.8 Tier 3 ≤ 1% disruption test (100 backends, 100k flows, remove one, assert ≤ 1% shift)
- 4.9 Tier 4 veristat baseline update (research's "fits in L1" claim quantified)

### 5. Close the return path

- 5.1 `REVERSE_NAT_MAP` declared as `BPF_MAP_TYPE_HASH`; key `BackendKey`, value `Vip`
- 5.2 `xdp_reverse_nat` program (XDP egress or TC egress — DESIGN picks)
- 5.3 `EbpfDataplane::update_service` writes/removes REVERSE_NAT entries in lockstep with service-backend changes
- 5.4 DST invariant `ReverseNatLockstep`
- 5.5 Tier 3 real `nc` end-to-end TCP connection test

### 6. Drop pathological traffic

- 6.1 Sanity prologue prepended to `xdp_service_map_lookup` and `xdp_reverse_nat` (5 checks, Cloudflare order)
- 6.2 `DROP_COUNTER` `BPF_MAP_TYPE_PERCPU_ARRAY` keyed by drop-class slot
- 6.3 `DropClass` typed enum in `overdrive-core` mapping 1:1 to slot indices
- 6.4 DST invariant `SanityChecksFireBeforeServiceMap`
- 6.5 Tier 2 PKTGEN/SETUP/CHECK per drop class
- 6.6 Tier 3 mixed-batch integration test
- 6.7 veristat re-baseline (delta vs Slice 04 < 20%; absolute ≤ 60% of 1M ceiling)

### 7. Enforce perf gates

- 7.1 `cargo xtask verifier-regress` filled in (replaces Phase 2.1 stub at `xtask/src/main.rs:588`)
- 7.2 `cargo xtask xdp-perf` filled in (replaces Phase 2.1 stub at `xtask/src/main.rs:594`)
- 7.3 Baseline files under `perf-baseline/main/` for veristat (× 2 programs) and xdp-perf (× 3 modes)
- 7.4 CI workflow wiring (per-PR Job E per `.claude/rules/testing.md` § "CI topology")
- 7.5 Single-kernel in-host execution per #152
- 7.6 xtask self-test invariant `PerfBaselineGatesEnforced`

### 8. Close the J-PLAT-004 reconciler loop (cross-cutting against activities 2-6)

The hydrator reconciler is the consumer of the `Dataplane` port body
that activities 2-6 fill in. It is captured as a single slice rather
than a new backbone column because it has no operator-visible
behaviour of its own — it is the §18 reference reconciler that
closes the ESR loop against the dataplane port.

- 8.1 `ServiceMapHydrator` reconciler in `crates/overdrive-control-plane/src/reconcilers/service_map_hydrator.rs`
- 8.2 `type State = ServiceMapHydratorState` (typed projection of intent + observation per `ServiceId`)
- 8.3 `type View = ServiceMapHydratorView` (typed memory: `attempts`, `last_failure_seen_at`, last-seen `service_backends` generation per service — inputs only, never derived deadlines)
- 8.4 Sync `reconcile` per ADR-0035 / ADR-0036 — no `.await`, no wall-clock reads, no DB handle held by the reconciler; `tick.now` is the only wall-clock source
- 8.5 `Action::DataplaneUpdateService` (or DESIGN-chosen variant) emission per service whose backend set has drifted; action shim dispatches against `Arc<dyn Dataplane>`
- 8.6 DST invariant `HydratorEventuallyConverges` (eventual: `actual` reaches `desired` and stays)
- 8.7 DST invariant `HydratorIdempotentSteadyState` (always: no action emitted in steady state given unchanged inputs)
- 8.8 `ReconcilerIsPure` continues to pass with the hydrator added to the catalogue

## Walking Skeleton

This feature does NOT introduce a new product-level walking skeleton.
Phase 1's `submit-a-job.yaml` walking skeleton has been shipped, and
Phase 2.1 added the `Dataplane` port adapter (`EbpfDataplane`) behind
it. This feature fills the empty body of `EbpfDataplane::update_service`
— graduating it from `Ok(())` no-op to a real five-step atomic-swap
implementation that materialises four BPF maps as their § 7 / § 15
forms describe.

There is no internal walking skeleton across Slices 01-08 — every
dataplane slice depends on its predecessor (with two parallel-able
edges, see "Slice ordering" below). The natural minimum-viable
subset is **Slice 01 + Slice 02 + Slice 08**: real-iface attach +
single-VIP forward path + hydrator reconciler proves the
loader-program-reconciler shape end-to-end against a real kernel
and a real ESR consumer. Slices 03-07 are the LB substance on top.

## Release slices (elephant carpaccio) — 8-slice plan

Eight slices: seven carry one ≤1-day backbone-activity outcome each
(Slice 04 acknowledged as 1.5d given its component count — see
`wave-decisions.md` Risk #2 / Decision 4), the eighth (Slice 08)
ships the SERVICE_MAP hydrator reconciler that consumes the
`Dataplane` port body Slices 02-06 fill in. Each slice carries a
learning hypothesis it can disprove, a production-data-shaped
acceptance, and is demonstrable in a single working session.

### Slice 01 — Real-iface XDP attach (veth, not `lo`)

**Outcome**: Phase 2.1's `xdp_pass` attaches to a real `veth0` (and
to virtio-net inside Lima) instead of `lo`. Existing `PACKET_COUNTER`
behaviour proven against a non-loopback driver. Loader's
iface-resolution / native-mode-attach / structured-warning shape
exercised end-to-end.

**Target KPI**: 100% of integration test runs show `PACKET_COUNTER`
incrementing on a real veth pair; 100% of native-attach failures
surface as a `tracing::warn!`; 0 silent fall-throughs to generic
mode.

**Hypothesis**: "If a non-`lo` ifindex changes loader semantics —
attach behaviour, native-mode availability, ifindex resolution
errors — every later slice that touches a real driver will surface
two failure modes at once (slice-specific + driver-specific). Lifting
to veth FIRST collapses the debug surface area to one variable per
later slice."

**Disproves**: "ifindex semantics are uniform across drivers." (We
expect this is true; the slice is the empirical confirmation.)
"`xdp_pass` against `lo` is sufficient evidence that the loader
works." (No — `lo` is XDP-generic-fallback territory; native attach
to virtio-net / mlx5 is a different code path.)

**Delivers (story)**: US-01.

**Slice taste-test**:
- New components ≤ 4: veth setup helper + iface-resolution helper + structured-warning emission. Three components.
- No hypothetical abstractions: relies only on existing `EbpfDataplane`, `xdp_pass`, `PACKET_COUNTER`.
- Production-shaped AC: real veth pair, real ip-link command, real `bpftool map dump`.
- IN scope: real-iface attach, native-mode default, structured warning on fallback, `IfaceNotFound` error.
- OUT of scope: any new map, any LB logic, conntrack (#154), kernel matrix (#152).

---

### Slice 02 — SERVICE_MAP forward path with single hardcoded backend

**Outcome**: New XDP program `xdp_service_map_lookup` parses Eth + IPv4 + TCP/UDP, looks up VIP+port+proto in `SERVICE_MAP` (`BPF_MAP_TYPE_HASH`), rewrites destination, returns `XDP_TX`. `EbpfDataplane::update_service` graduates from no-op stub to real single-entry insert/remove. Typed `ServiceMapHandle` newtype in `overdrive-dataplane` wraps the aya HashMap so the call site never sees raw kernel-side types. Tier 2 PKTGEN/SETUP/CHECK triptych passes; Tier 3 veth test forwards 10 TCP SYNs end-to-end with valid checksums.

**Target KPI**: 100% of Tier 2 SERVICE_MAP-hit packets return `XDP_TX` with valid checksums; 100% of misses return `XDP_PASS`; veristat instruction count ≤ 50% of 1M-privileged ceiling.

**Hypothesis**: "If the basic SERVICE_MAP shape (parse + lookup + rewrite + checksum + XDP_TX) doesn't fit the verifier budget cleanly when written in aya-rs, every later slice that builds on top is uncertain. Conversely, if the baseline veristat number is well under 50% of ceiling, Maglev (Slice 04) can land without verifier-budget anxiety."

**Disproves**: "We need to write the program in C to fit the verifier budget." (No — research finding 5.4 says Cilium and Katran fit the equivalent in their datapath; aya-rs through LLVM produces equivalent BPF bytecode.) "The single-VIP slice is too thin." (No — it's the smallest possible verifier-clean lookup-and-rewrite, the precondition for every later slice.)

**Delivers (story)**: US-02.

**Slice taste-test**:
- New components ≤ 4: `xdp_service_map_lookup` program + `SERVICE_MAP` declaration + `ServiceMapHandle` newtype + `ServiceVip` newtype. Four components, at the upper end.
- No hypothetical abstractions: depends on Phase 2.1 `EbpfDataplane` and existing `Dataplane` port trait.
- Production-shaped AC: Tier 2 PROG_TEST_RUN + Tier 3 veth integration with tcpdump.
- IN scope: single-VIP forward path, parsing, checksum recompute, typed `ServiceMapHandle`, veristat baseline.
- OUT of scope: multiple backends (Slice 03), Maglev (Slice 04), REVERSE_NAT (Slice 05), sanity prologue (Slice 06), perf gates (Slice 07), IPv6 / ICMP (future), conntrack (#154).

---

### Slice 03 — HASH_OF_MAPS atomic per-service backend swap

**Outcome**: SERVICE_MAP restructures to `BPF_MAP_TYPE_HASH_OF_MAPS` per Cilium reference. New `BACKEND_MAP` (`BPF_MAP_TYPE_HASH`, key `BackendId`, value `Backend`). `EbpfDataplane::update_service` performs the five-step atomic swap (BACKEND_MAP upsert → fresh inner map → outer-pointer swap → orphan GC → release). `SimDataplane` mirrors atomic-swap semantics for DST. Tier 3 integration test under `xdp-trafficgen` 100 kpps load asserts ZERO packet drops across an atomic swap. New DST invariant `BackendSetSwapAtomic`.

**Target KPI**: 0 dropped packets across an atomic backend-set swap under 100 kpps load (Tier 3); 100% `BackendSetSwapAtomic` invariant pass rate (DST); 0 BACKEND_MAP orphans after GC pass.

**Hypothesis**: "If Cilium's HASH_OF_MAPS atomic-swap shape doesn't actually deliver zero drops at 100 kpps in our setup, the § 15 'weighted backends' claim is performative. Conversely, if it does — and the DST `BackendSetSwapAtomic` invariant proves it — Maglev (Slice 04) inherits a known-good substrate for its weighted-canary commitment."

**Disproves**: "Atomic backend swap requires per-flow conntrack to be zero-drop." (No — the swap atomicity is at the map level; conntrack pins individual flows but is not necessary for swap atomicity itself.) "Multiple-backend round-robin can wait until Maglev." (No — the two-level shape is the structural prerequisite for Maglev; the random round-robin in this slice is intentional simplicity that proves the atomic-swap mechanics independent of the algorithm choice.)

**Delivers (story)**: US-03.

**Slice taste-test**:
- New components ≤ 4: HASH_OF_MAPS restructure + BACKEND_MAP + `BackendId` newtype + atomic-swap implementation in `update_service`. Four components, upper end.
- No hypothetical abstractions: extends Slice 02's SERVICE_MAP shape; Cilium reference architecture is the proven precedent.
- Production-shaped AC: real `xdp-trafficgen` 100 kpps load + zero-drop assertion across the swap window.
- IN scope: HASH_OF_MAPS, BACKEND_MAP, atomic 5-step swap, BackendId newtype, orphan GC, atomic-swap DST invariant.
- OUT of scope: Maglev algorithm (Slice 04), REVERSE_NAT (Slice 05), sanity (Slice 06), perf gates (Slice 07), conntrack (#154).

---

### Slice 04 — Maglev consistent hashing inside MAGLEV_MAP (1.5 days)

**Outcome**: New `MAGLEV_MAP` (`BPF_MAP_TYPE_HASH_OF_MAPS`, inner `BPF_MAP_TYPE_ARRAY` size `M=16381`). New STRICT `MaglevTableSize` newtype in `overdrive-core` constrained to Cilium prime list. Pure `maglev::generate(&BTreeMap<BackendId, Weight>, MaglevTableSize) -> Vec<BackendId>` in `overdrive-dataplane`. Weighted-Maglev variant ships in this slice (no vanilla-then-weighted progression). XDP program lookup switches from Slice 03's random slot to MAGLEV_MAP-indexed. Tier 3 disruption test (100 backends, remove one, assert ≤2% total flow shift — 1% forced + ≤1% incidental per Maglev's published bound). Tier 4 veristat baseline updated (research's "fits in L1" claim quantified). Effort acknowledged as 1.5 days (not 1) given the component count: full `MaglevTableSize` newtype discipline + weighted multiplicity expansion + Eisenbud permutation + lookup switch + veristat baseline + 2 DST invariants. Splitting would create two near-duplicate slices (would fail the carpaccio taste test); kept as one slice with 1.5-day budget per `wave-decisions.md` Risk #2.

**Target KPI**: ≤2% total flow shift on single-backend removal among 100 backends — 1% forced + ≤1% incidental, per Maglev's ≤1%-incidental-disruption bound (Tier 3); ± 5% even-distribution under equal weights (Tier 3 + DST); ± 2% weight-honoring under skewed weights (Tier 3); veristat instruction count ≤ 50% of 1M ceiling (Tier 4).

**Hypothesis**: "If Maglev's lookup-table-driven indexing pattern doesn't fit comfortably under the verifier complexity ceiling when written in aya-rs, the §15 weighted-canary claim either ships without consistent-hashing affinity or pushes verifier budget into red. Conversely, if it does — and the disruption proptest proves ≤2% total flow shift (1% forced + ≤1% incidental) — every later slice (POLICY_MAP, IDENTITY_MAP, conntrack) inherits a known-clean veristat budget headroom."

**Disproves**: "Vanilla Maglev is enough; weighted Maglev is a follow-on." (No — research § 5.3 says weighted-Maglev's algorithmic delta is in userspace permutation generation; verifier delta is negligible. Splitting saves no engineering time and ships an incomplete §15 commitment.) "Maglev is too verifier-expensive for aya-rs." (No, per finding 5.4; the slice is the empirical disproof.)

**Delivers (story)**: US-04.

**Slice taste-test**:
- New components ≤ 4: MAGLEV_MAP + `maglev::generate` + `MaglevTableSize` newtype + XDP-side lookup switch. Four components.
- No hypothetical abstractions: extends Slice 03's HASH_OF_MAPS shape; the algorithm is published (Eisenbud NSDI 2016).
- Production-shaped AC: Tier 3 disruption test + Tier 4 veristat baseline update against real packets / real verifier.
- IN scope: Maglev table generation, weighted variant, MAGLEV_MAP, XDP-side indexing switch, disruption + distribution proptests, veristat baseline update.
- OUT of scope: REVERSE_NAT (Slice 05), sanity (Slice 06), perf gates (Slice 07), conntrack-based flow pinning (#154), kernel matrix (#152).

---

### Slice 05 — REVERSE_NAT_MAP for response-path rewrite

**Outcome**: New `REVERSE_NAT_MAP` (`BPF_MAP_TYPE_HASH`, key `BackendKey`, value `Vip`). New XDP program `xdp_reverse_nat` (or TC-egress equivalent — DESIGN picks). `EbpfDataplane::update_service` writes/removes REVERSE_NAT entries in lockstep with service-backend changes. Tier 3 integration test runs a real `nc` server and client across a veth pair and asserts the connection completes end-to-end with payload echoed.

**Target KPI**: 100% of Tier 3 `nc` connection-completion runs succeed; 100% `ReverseNatLockstep` invariant pass rate; 0 stale REVERSE_NAT entries after backend removal.

**Hypothesis**: "If REVERSE_NAT can't be added without entangling the existing forward-path program, the program shape is wrong and every later slice carries that confusion. Conversely, if the egress program is a clean independent module that shares only the parsing helpers, every later slice has a known-good template for additional egress logic (sockops integration, conntrack lookups, etc.)."

**Disproves**: "Forward and return paths can share one program." (No, per Cilium reference; splitting keeps each program small, verifier-clean, independently veristat-able.) "REVERSE_NAT is a Phase 3 concern." (No — without it, the LB is functionally inert against any real client/backend pair.)

**Delivers (story)**: US-05.

**Slice taste-test**:
- New components ≤ 4: REVERSE_NAT_MAP + `xdp_reverse_nat` program + lockstep update path in `update_service` + DST invariant. Four components.
- No hypothetical abstractions: extends Slice 04's three-map split with the third Cilium-reference map.
- Production-shaped AC: real `nc` end-to-end TCP across veth.
- IN scope: REVERSE_NAT_MAP, egress program, lockstep update, lockstep invariant, real `nc` integration test.
- OUT of scope: sanity prologue (Slice 06), perf gates (Slice 07), cross-node REVERSE_NAT (future Phase 2 slice), conntrack-based reverse-NAT optimisation (#154).

---

### Slice 06 — Pre-SERVICE_MAP packet-shape sanity checks

**Outcome**: Sanity prologue prepended to `xdp_service_map_lookup` and `xdp_reverse_nat`. Five Cloudflare-order checks (EtherType → IP version+IHL → IP total_length → protocol → TCP flag sanity). `DROP_COUNTER` (`BPF_MAP_TYPE_PERCPU_ARRAY`) records drops per class. `DropClass` typed enum in `overdrive-core` mapping 1:1 to slot indices. Tier 2 triptych per drop class. Tier 3 mixed-batch integration test. veristat re-baseline (instruction-count delta vs Slice 04 < 20%; absolute ≤ 60% of ceiling).

**Target KPI**: 100% of synthetic pathological frames are dropped at the prologue (Tier 3); per-class counter is correct on every Tier 2 test; verifier instruction-count delta vs Slice 04 baseline < 20% (Tier 4).

**Hypothesis**: "If the static sanity checks don't quantify their verifier cost (delta vs the Slice 04 baseline), we don't know whether to budget for them in future slices or fold them into POLICY_MAP / #25's compile-on-rule-change shape. Conversely, if delta < 20% and absolute remains comfortable, the Phase 2 verifier-budget plan stays on track and operator-tunable POLICY_MAP rules can land later without revisiting the static prologue."

**Disproves**: "Sanity checks are free (verifier-wise)." (No — every check costs branches the verifier walks; the slice quantifies how much.) "Operator-tunable rules belong in this slice." (No — that's POLICY_MAP / #25, with materially different mechanics.)

**Delivers (story)**: US-06.

**Slice taste-test**:
- New components ≤ 4: sanity prologue + `DROP_COUNTER` + `DropClass` enum + sanity-fires-before-service DST invariant. Four components.
- No hypothetical abstractions: ships the static prologue both XDP programs invoke; the operator-tunable layer is explicitly OUT (POLICY_MAP).
- Production-shaped AC: Tier 2 per drop class + Tier 3 mixed-batch + Tier 4 veristat delta budget.
- IN scope: 5-check prologue, per-class DROP_COUNTER, DropClass enum, lockstep insertion in both forward and reverse XDP programs, DST invariant.
- OUT of scope: operator-tunable DDoS rules (POLICY_MAP / #25), perf gates (Slice 07), conntrack (#154), kernel matrix (#152).

---

### Slice 07 — Tier 4 perf gates + veristat baseline land on `main`

**Outcome**: `cargo xtask verifier-regress` and `cargo xtask xdp-perf` filled in (replace the Phase 2.1 stubs at `xtask/src/main.rs:588-594`). Baseline files under `perf-baseline/main/` for veristat (× 2 programs) and xdp-perf (× 3 modes: DROP, TX, LB-forward). CI workflow Job E enforces both gates per `.claude/rules/testing.md` § "CI topology". Single-kernel in-host per #152.

**Target KPI**: 100% of PRs that breach the 5% / 10% / 5% thresholds fail CI (no false negatives); 0 false positives on the first three Phase 2.3+ follow-on PRs (hand-validated); both xtask subcommands return well-formed structured output on every run.

**Hypothesis**: "Without a real PR-blocking perf gate, every later Phase 2 slice (POLICY_MAP, IDENTITY_MAP, FS_POLICY_MAP, conntrack #154, sockops, kTLS) has no way to detect verifier or pps regressions before merge — the gates are theoretical until they actually trip. The first hand-validated false-positive trip on a follow-on PR is the disproof attempt: if the gate fires on a PR that is genuinely not regressing, the gate logic is wrong; if it correctly catches a true regression, the gate is paying off."

**Disproves**: "Tier 4 can stay deferred to a later phase." (No — every Phase 2.3+ slice will keep adding eBPF code; without enforcement the baseline dies of attrition.) "Absolute-pps thresholds work." (No, per `.claude/rules/testing.md` — relative-delta only.)

**Delivers (story)**: US-07.

**Slice taste-test**:
- New components ≤ 4: `verifier-regress` xtask body + `xdp-perf` xtask body + baseline directory + CI workflow wiring. Four components.
- No hypothetical abstractions: replaces existing Phase 2.1 stubs; uses existing xtask plumbing.
- Production-shaped AC: real veristat / real xdp-bench against compiled programs; real CI workflow integration.
- IN scope: both xtask subcommand bodies, baseline files, CI workflow Job E, single-kernel in-host execution, xtask self-test invariant.
- OUT of scope: kernel matrix (#152), nested-VM execution, conntrack-related metrics (#154).

---

### Slice 08 — SERVICE_MAP hydrator reconciler converges Dataplane port

**Outcome**: A `ServiceMapHydrator` reconciler lands in `crates/overdrive-control-plane/src/reconcilers/service_map_hydrator.rs` per ADR-0035 / ADR-0036. Sync `reconcile`, no `.await`, no wall-clock reads inside the function (`tick.now` snapshot from the runtime), no DB handle held by the reconciler — runtime owns hydration of intent / observation / View end-to-end. Typed `ServiceMapHydratorState` projects desired backends (from IntentStore) and actual backends (from `service_backends` ObservationStore rows) per `ServiceId`; typed `ServiceMapHydratorView` carries retry inputs (`attempts`, `last_failure_seen_at`, last-seen `service_backends` generation). The reconciler emits `Action::DataplaneUpdateService` (or DESIGN-chosen Action variant) per service whose backend set has drifted; the action shim dispatches against `Arc<dyn Dataplane>`. Two new DST invariants in `overdrive-sim::invariants`: `HydratorEventuallyConverges` and `HydratorIdempotentSteadyState`.

**Target KPI**: 100% pass rate of `HydratorEventuallyConverges` and `HydratorIdempotentSteadyState` across every DST seed on every PR; `ReconcilerIsPure` continues to pass with the hydrator added; 0 `.await` / wall-clock / DB-handle violations in `reconcile` (`dst-lint` clean).

**Hypothesis**: "If ESR convergence holds against the new dataplane port — `actual` driven from `service_backends` rows, `desired` driven from IntentStore-derived projection, hydrator emits `Action::DataplaneUpdateService` — then the §18 reference shape works for every later dataplane reconciler. A regression here means the SimDataplane ↔ EbpfDataplane port shape is wrong and every later slice (POLICY_MAP / IDENTITY_MAP / conntrack #154 / sockops / kTLS) inherits that confusion."

**Disproves**: "The hydrator can wait until Phase 2.3+." (No — without it, every earlier slice's `Dataplane` port plumbing is untested against an ESR-shaped consumer; J-PLAT-004's activation is performative.) "The hydrator needs to know about HASH_OF_MAPS / Maglev / atomic-swap mechanics." (No — it calls `update_service(service_id, backends)` and the `EbpfDataplane` impl owns the swap; the hydrator can land in parallel with Slices 03-06.)

**Delivers (story)**: US-08.

**Slice taste-test**:
- New components ≤ 4: `ServiceMapHydrator` reconciler + `ServiceMapHydratorState` + `ServiceMapHydratorView` + 2 DST invariants (`HydratorEventuallyConverges`, `HydratorIdempotentSteadyState`). Four components.
- No hypothetical abstractions: depends on Slice 02's `Dataplane::update_service` body; uses the existing §18 reconciler runtime and ADR-0035 ViewStore. The §18 reference shape is published; this is the first non-trivial use of it against a real dataplane port.
- Production-shaped AC: DST invariants on every PR; Tier 1 (DST) is the primary surface; Tier 2 / Tier 3 are secondary and exercised end-to-end by Slice 02 / 03 / 04's existing integration tests because `EbpfDataplane::update_service` is the call site the action shim drives.
- IN scope: hydrator reconciler with State + View types, `Action::DataplaneUpdateService` emission, 2 DST invariants, `ReconcilerIsPure` continues to pass with the hydrator added.
- OUT of scope: Maglev table generation (Slice 04), atomic-swap mechanics (Slice 03), cross-node propagation (#156 / Phase 5.20), the exact Action variant shape if a new one is needed (DESIGN-time concern), conntrack (#154), kernel matrix (#152).

## Slice ordering — dependency chain + learning leverage

The dependency chain is largely linear, with two parallel-able edges:

```
Slice 01 (real-iface attach)
    │
    ▼
Slice 02 (SERVICE_MAP forward + single VIP)
    │
    ├──────────────────────────────────┐
    ▼                                  ▼
Slice 03 (HASH_OF_MAPS atomic swap)   Slice 08 (SERVICE_MAP hydrator
    │                                  reconciler — depends only on
    ▼                                  Slice 02's Dataplane port body
Slice 04 (Maglev consistent hashing)   and the service_backends
    │                                  ObservationStore rows; can run
    ├───────────┐                      in parallel with Slices 03-07)
    ▼           ▼
Slice 05    Slice 06
(REVERSE_NAT)  (sanity prologue)
    │           │
    └─────┬─────┘
          ▼
    Slice 07 (perf gates lock in baseline)
```

1. **Slice 01 (Real-iface attach)** — the cheapest and most-isolating slice. Lifts the assumption that ifindex semantics are uniform; lands first so Slices 02-06 can attribute attach-side failures to the slice they're built on, not to a driver mismatch.
2. **Slice 02 (SERVICE_MAP single VIP)** — establishes the parsing + lookup + rewrite + verifier-clean shape.
3. **Slice 03 (HASH_OF_MAPS atomic swap)** — restructures the map shape to support § 15's atomic-swap claim. Random-slot lookup intentional simplicity; Maglev comes next.
4. **Slice 04 (Maglev)** — the consistent-hashing layer. Replaces random with weighted Maglev. Ships weighted variant directly per research § 5.3.
5. **Slices 05 (REVERSE_NAT) and 06 (sanity prologue)** — parallel-able. Both extend the Slice 04 program shape but in independent directions (egress vs ingress prologue). DESIGN may sequence them in either order.
6. **Slice 07 (perf gates)** — last among the dataplane slices because it baselines the program shape AS IT STANDS at the end of Slice 06. Sequencing it earlier would baseline an incomplete shape and force re-baselining on every subsequent slice.
7. **Slice 08 (SERVICE_MAP hydrator reconciler)** — lands AFTER Slice 02 (it needs `Dataplane::update_service` to do something) and can run in parallel with Slice 03 onward. The hydrator does NOT need to know about HASH_OF_MAPS / Maglev / sanity-prologue mechanics — it calls `update_service(service_id, backends)` and the `EbpfDataplane` impl owns the swap. Can therefore land any time after Slice 02 closes; effectively parallel-able with Slices 03-07.

## Priority Rationale

All eight slices are inside the dataplane substance for Phase 2.2.
None of them on their own delivers the full "§ 7 SERVICE_MAP exists
end-to-end with § 15 atomic-swap evidence + J-PLAT-004 reconciler
loop closed" outcome, but each is **demonstrable in isolation** and
disproves a named hypothesis if wrong.

| Priority | Slice | Why this order |
|---|---|---|
| 1 | Slice 01 (Real-iface attach) | Cheapest, most-isolating; lands first so later slices attribute attach-side failures cleanly. |
| 2 | Slice 02 (SERVICE_MAP single VIP) | First non-`xdp_pass` program; verifier baseline established here. Unblocks both Slice 03 (HASH_OF_MAPS) and Slice 08 (hydrator). |
| 3 | Slice 03 (HASH_OF_MAPS atomic swap) | Structural prerequisite for Slice 04; § 15 zero-drop claim made concrete here. |
| 4 | Slice 04 (Maglev — 1.5d) | The § 15 weighted-canary claim becomes evidence-backed; verifier-budget question (research § 5.4) answered. |
| 5 | Slice 05 (REVERSE_NAT) | Closes the return path; without it the LB is functionally inert. Parallel-able with Slice 06. |
| 6 | Slice 06 (sanity prologue) | § 7 / § 19 defense-in-depth posture; quantifies static-sanity verifier cost. Parallel-able with Slice 05. |
| 7 | Slice 07 (perf gates) | Last among the dataplane slices because it baselines the post-Slice-06 program shape; sequencing earlier would force re-baselining. |
| 8 | Slice 08 (SERVICE_MAP hydrator reconciler) | The §18 reference reconciler against the new dataplane port; closes the J-PLAT-004 loop. Lands any time after Slice 02 closes — does NOT depend on HASH_OF_MAPS / Maglev / sanity / perf-gate mechanics; effectively parallel-able with Slices 03-07. Sequenced last in this table for narrative cleanliness, not for technical dependency. |

Slices 05 / 06 can run in parallel; Slice 08 can run in parallel with Slices 03-07; all others are sequential.

## Slice taste-tests against the 8-slice plan

Re-running the elephant-carpaccio taste tests against the 8-slice
plan:

| Property | S01 | S02 | S03 | S04 | S05 | S06 | S07 | S08 | Verdict |
|---|---|---|---|---|---|---|---|---|---|
| ≤ 4 new components | PASS (3) | PASS (4) | PASS (4) | PASS (4) | PASS (4) | PASS (4) | PASS (4) | PASS (4) | OK — every slice at or below the cap |
| No hypothetical abstractions landing later | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS | OK |
| Disproves a named pre-commitment | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS | OK |
| Production-data-shaped AC | PASS (real veth) | PASS (Tier 2 + Tier 3) | PASS (xdp-trafficgen 100 kpps) | PASS (proptest 100 backends) | PASS (real `nc`) | PASS (Tier 2 per class) | PASS (real CI gate) | PASS (DST invariants on every PR) | OK |
| Demonstrable in single session | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS | OK |
| Same-day dogfood moment | PASS (Lima dev VM) | PASS | PASS | PASS | PASS | PASS | PASS | PASS (DST replay) | OK |

The 8-slice plan is **right-sized**: each slice is independently
shippable, every one passes the taste tests, and no slice cross-cuts
more than 2 crates (`overdrive-bpf` + `overdrive-dataplane` for the
seven dataplane slices; `overdrive-control-plane` for Slice 08's
hydrator reconciler — additive against the existing reconciler set,
no new crate). `overdrive-core` newtypes are additive on each slice
that needs one. Of the eight slices, four ship multi-component
user-side work (Slices 02 / 03 / 04 / 08); the remaining four are
single-component or scaffolding-shaped — well within the ≤ 4 cap.

## Scope Assessment: PASS — 8 stories, 3 crates touched, estimated 8.5 days

- **Story count**: 8 stories (US-01 through US-08). At the upper end of the right-sized band but each slice fits ≤ 1.5 days (Slice 04 acknowledged as 1.5d, the rest ≤ 1d) and each delivers one demonstrable outcome.
- **Bounded contexts / crates**: 3 (`overdrive-bpf` for kernel-side eBPF programs + maps; `overdrive-dataplane` for loader, hydration, typed map-handle newtypes; `overdrive-core` for STRICT identifier newtypes additive per slice). The hydrator reconciler in Slice 08 lives in the existing `overdrive-control-plane` reconciler set — no new crate boundary. Within the ≤ 3-bounded-context oversized signal.
- **Walking-skeleton integration points**: 4 (Phase 1 walking skeleton via `Dataplane::update_service`; Phase 2.1 `EbpfDataplane` adapter; veth integration test harness; CI workflow Job E). Within the ≤ 5 oversized-signal threshold.
- **Estimated effort**: ~8.5 focused days (Slice 04 at 1.5d; the rest ≤ 1d each). Slices 05 / 06 parallel-able; Slice 08 (hydrator reconciler) can run in parallel with Slices 03-07 because it depends only on Slice 02's `Dataplane::update_service` body. Wall-clock can compress to ~6.5 days.
- **Multiple independent user outcomes worth shipping separately**: no — the eight slices are sequential / parallel on the same § 7 / § 15 / J-PLAT-004 commitment. Each demonstrably moves the platform forward, but the "§ 7 SERVICE_MAP delivers § 15 atomic-swap evidence + J-PLAT-004 reconciler loop closed" outcome only fully exists at the end of Slice 08.
- **Verdict**: **RIGHT-SIZED** — 8 stories at the upper end of the band, 3 crates well-bounded, 4 integration points, every slice passes carpaccio taste tests.

## Changelog

| Date | Change |
|---|---|
| 2026-05-05 | Initial story map for `phase-2-xdp-service-map` DISCUSS wave. Lean shape: no JTBD, no Journey design (dataplane infrastructure with no UX). 7 carpaccio slices; right-sized verdict; Slices 05 + 06 parallel-able. |
| 2026-05-05 | Eclipse-review remediation: added Slice 08 (SERVICE_MAP hydrator reconciler) restoring the user-ratified IN-scope decision. Slice taste-test grid extended to 8 columns; scope assessment updated to 8 stories / 3 crates / 8.5 days; Slice 04 acknowledged as 1.5 days; priority rationale section updated with explicit Slice 08 row noting parallel-with-Slices-03-07 dependency edge. Walking skeleton minimum-viable subset extended to Slice 01 + Slice 02 + Slice 08. |
