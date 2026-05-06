# DESIGN Decisions — phase-2.16-xdp-conntrack

**Wave**: DESIGN (solution-architect)
**Owner**: Morgan
**Date**: 2026-05-06
**Status**: PROPOSED — pending user ratification + reviewer pass
**Mode**: propose (autonomous; user dispatch instruction
"address GH #154 in the next roadmap step")

---

## Six locked decisions (per dispatch — answered with rationale)

The dispatch surfaced six decisions. Each is locked here with the
chosen option + rationale; the full design lockpoint lives in
ADR-0044.

### Decision 1 — Scope and feature boundary: **(b) Spin off as new sibling feature**

The conntrack scope is large enough to deserve its own DESIGN wave
(six roadmap steps; three new newtypes; one new BPF map; three new
DST invariants; three Earned Trust probes; NOTRACK installation in
both production and test paths). Folding this into Phase 2.2 as
"Slice 09" would:

- Distort `phase-2-xdp-service-map`'s DISCUSS-locked story map
  (8 stories → 9; the story map is already finalised and reviewed).
- Conflate two structurally different concerns: stateless Maglev
  forwarder (Phase 2.2's commercial goal) and dataplane-owned
  conntrack (Phase 2.16's commercial goal of bounded
  flow-affinity-under-rotation).
- Make the deferred-conntrack-OUT-of-Phase-2.2 contract carried
  by every DISCUSS artifact retroactively false.

Sibling-feature shape lets Phase 2.2 close at its current Slice 06–08
boundary cleanly. The DISCUSS surface for Phase 2.16 is small (one
story; constraints inherited verbatim from Phase 2.2 with one flip)
and is folded into the design wave directly per
`discuss/brief.md`.

**Rejected (a) New slice in Phase 2.2**: distorts the DISCUSS lock.
**Rejected (c) Tactical bridge**: ruled out by user intent
(*"address GH #154 in the next roadmap step"* — bridge alone defers
the strategic fix). Note: the tactical NOTRACK bridge IS still part
of the path — see Decision 6 — but as a one-step Slice 06 patch,
not as the strategic answer.

### Decision 2 — Conntrack data structure: **per-CPU LRU `BPF_MAP_TYPE_LRU_PERCPU_HASH`**

Per ADR-0044 § Decision 1 — Katran shape. Three sub-decisions:

- **Map type**: per-CPU LRU. The kernel ships
  `BPF_MAP_TYPE_LRU_PERCPU_HASH` natively (no `BPF_F_NO_COMMON_LRU`
  hack required); aya-ebpf 0.1.1 ships the typed kernel-side
  wrapper `LruPerCpuHashMap<K, V>` (per aya-rs research § A.1).
  Userspace falls through to `Map::PerCpuLruHashMap(MapData)` —
  same hand-rolled-handle pattern as the existing
  `Map::Unsupported(MapData)` HoM access in ADR-0040.
- **Key**: `FlowKey { client_ip, client_port, vip, vip_port,
  proto, _pad }` — host-order in map storage, conversion at the
  kernel boundary in `crates/overdrive-bpf/src/shared/sanity.rs`
  (extending the existing `reverse_key_from_packet` site for
  endianness lockstep, ADR-0041 § 11). **Pre-NAT** key — keys on
  what the *client* sent, not the post-NAT backend tuple. This
  is the structural choice: forward-path lookup happens *before*
  Maglev selects a backend, so the key must match what's on the
  wire at that point.
- **Value**: `FlowEntry { backend_id, service_generation,
  last_seen_secs, flow_state, _pad }` — see ADR-0044 § Decision 3
  for the full value-shape rationale. The `service_generation`
  field is the load-bearing primitive that makes
  flow-affinity-under-rotation structural.
- **Eviction**: kernel-native LRU, capacity 1_048_576 entries per
  CPU. Operator-tunability deferred (same shape as
  `MaglevTableSize`'s deferral in ADR-0041 Q6=A).
- **NOTRACK interaction**: `iptables -t raw -j NOTRACK` —
  see Decision 5 below for the full NOTRACK-vs-PRE_ROUTING-tricks
  rationale.

### Decision 3 — Integration points: **forward writes; reverse promotes; no userspace eviction signal**

Three integration points, each minimally extending an existing
program:

- **Forward path** (`xdp_service_map`): on hit, refresh the LRU
  position via `bpf_map_lookup_elem` (LRU implicit via access).
  On miss, run Maglev hash, write a fresh `FlowEntry` with
  `flow_state: SynOnly`. On hit-with-stale-generation,
  branch on `flow_state` per the algorithm in ADR-0044 § Decision
  3 step 4.
- **Reverse path** (`tc_reverse_nat`): on hit, build the FORWARD
  FlowKey from the reverse key + `OriginalDest`, look up
  `CONNTRACK_MAP::get(forward_key)`. If `flow_state == SynOnly`,
  update to `Established`. This is the only conntrack write the
  reverse path performs.
- **Eviction signal**: none. Per CLAUDE.md § Principle 12 the
  Earned Trust probe verifies the kernel honours LRU eviction;
  beyond that, the userspace is read-only on conntrack state for
  telemetry purposes (`ConntrackMapHandle::entry_count`).
  No reconciler extension; the hydrator's contract from ADR-0042
  is unchanged.

### Decision 4 — Slice structure: **six steps, one ADR, three DST invariants**

Six steps, sized for credible mutation-target boundaries (per the
parent feature's roadmap-review feedback that mutation gates need
crate-level decomposition). The ID prefix `16-` matches the
phase-2.16 feature ID; final IDs land via `/nw-roadmap` after
DISTILL completes.

| ID | Name | Tier | Mutation target | Deps | Effort (h) |
|---|---|---|---|---|---|
| 16-01 | `FlowKey` / `FlowEntry` / `FlowState` / `ServiceGeneration` newtypes + SimDataplane mirror; three DST invariants land RED | 1 | `crates/overdrive-core` | none | 5 |
| 16-02 | `CONNTRACK_MAP` declaration (kernel-side) + `ConntrackMapHandle` userspace + Earned Trust probes wired into `Dataplane::probe` | 3 | `crates/overdrive-dataplane` | 16-01 | 6 |
| 16-03 | NOTRACK installation in production `EbpfDataplane::start` + `ThreeIfaceTopology::setup`. Flips S-2.2-17 GREEN | 3 | `crates/overdrive-dataplane` | 16-02 | 4 |
| 16-04 | XDP forward path conntrack lookup-write + `ServiceGeneration` increment in `Dataplane::update_service`. ESR pair `ConntrackPinsBackendUnderRotation` + `ConntrackEventuallyConvergesAfterRotation` go GREEN | 2 + 3 | `crates/overdrive-bpf` + `crates/overdrive-dataplane` | 16-03 | 8 |
| 16-05 | TC reverse path conntrack `flow_state` promotion (SynOnly → Established); `NoTrackProbeRefusesStartOnFailure` GREEN | 2 + 3 | `crates/overdrive-bpf` | 16-04 | 4 |
| 16-06 | Tier 4 verifier-regress + xdp-perf baseline update | 4 | skip-rationale: baseline files are generated artifacts | 16-05 | 2 |

**Total: 29 h.** Within the same effort envelope as Phase 2.2's
Slice 03 (5 h) + Slice 04 (15 h) sum (the structurally comparable
work). All steps gate `--features integration-tests` and run via
`cargo xtask lima run --` on macOS per `.claude/rules/testing.md`
§ "Running tests on macOS — Lima VM".

### Decision 5 — Effects on Slice 06 in-flight work: **option 2 — pause Slice 06, pivot to bridge fix, resume**

Per user dispatch intent. Three sub-questions:

- **Slice 06 status**: 06-01 + 06-02 GREEN; 06-03 + 06-04 + 06-05
  PENDING. Sanity prologue work is independent of conntrack and
  lands cleanly when resumed.
- **Pivot work**: 16-03's NOTRACK installation is the unblocking
  fix for S-2.2-17. Decision 6 below settles whether to ship the
  bridge as a Slice 06 sub-step or wait for the strategic fix in
  Slice 16-03.
- **Resume order**: Slice 06 (06-03..06-05) → Slice 07 (07-01,
  07-02) → Slice 08 (08-01..08-03) → **then** Slice 16
  (16-01..16-06). The Tier 4 baseline (Slice 07) MUST land before
  Slice 16-06 so the conntrack verifier-budget delta is measured
  against a stable baseline. The hydrator (Slice 08) MUST land
  before Slice 16-04 so `ServiceGeneration` has a stable home in
  `Dataplane::update_service`.

This sequencing is architecturally sound: the conntrack work has
genuine ordering dependencies on the perf-gate baseline (Slice 07)
and the hydrator (Slice 08), both of which are already in-flight.
Pivoting earlier would force re-baselining; pivoting later would
keep S-2.2-17 RED for longer.

### Decision 6 — S-2.2-17 unblocking: **bridge fix lands as Slice 06-04.5; strategic fix in 16-03**

The bridge fix (a single helper-line change in
`crates/overdrive-dataplane/tests/integration/helpers/netns.rs`
adding `iptables -t raw -A PREROUTING -j NOTRACK` to
`ThreeIfaceTopology::setup`) lands as part of Slice 06-04 — the
Tier 3 mixed-batch test that already touches the topology helper.
S-2.2-17 GREEN is a side-effect of Slice 06-04 once the bridge is
in.

Slice 16-03 then **deletes** the test-side bridge AND adds
production-side NOTRACK installation in `EbpfDataplane::start`.
Single-cut greenfield migration per
`feedback_single_cut_greenfield_migrations.md` — no deprecation,
no grace period; the bridge exists for the slices between 06-04
and 16-03, then disappears.

This shape is cleaner than either "bridge alone" (defers strategic
fix; eternal test-only NOTRACK is technical debt) or "no bridge"
(S-2.2-17 stays RED for the duration of Slice 06 + Slice 07 +
Slice 08 + Slice 16-01 + Slice 16-02 + Slice 16-03 — over 30 h
of work). The bridge unblocks S-2.2-17 in 1 line of helper code;
the strategic fix replaces it cleanly.

**Where the bridge gets added**:
`crates/overdrive-dataplane/tests/integration/helpers/netns.rs`
already exists in the working tree (untracked file; git status
shows it as `?? crates/overdrive-dataplane/tests/integration/helpers/netns.rs`).
The helper file is the natural home; the bridge is a single command
addition to whichever fn sets up the `lb-ns` netns.

---

## Architecture summary

- **Pattern**: Hexagonal (ports & adapters) — inherited from the
  parent feature.
- **Paradigm**: OOP (Rust trait-based) — inherited.
- **Key components**:
  - `overdrive-bpf` (kernel side): extension to `xdp_service_map`
    + `tc_reverse_nat` for conntrack lookup/write/promotion. New
    `CONNTRACK_MAP` declaration. New `flow_key_from_packet` helper.
  - `overdrive-dataplane` (userspace): new `ConntrackMapHandle`.
    Extension to `EbpfDataplane::start` for NOTRACK installation.
    `Dataplane::probe` extended with three Earned Trust probes per
    ADR-0044 § Decision 5.
  - `overdrive-core::dataplane`: three new types — `FlowKey`,
    `FlowEntry`, `FlowState` — plus `ServiceGeneration` newtype.
    All carry STRICT-newtype discipline (FromStr / Display / serde
    / rkyv / proptest) per `development.md` § Newtypes.
  - `overdrive-sim`: `SimDataplane` extended with per-CPU
    conntrack mirror per ADR-0044 § Decision 6. Three new DST
    invariants in `crates/overdrive-sim/src/invariants/conntrack/`.

---

## Reuse Analysis (HARD GATE)

| # | Component / surface | Disposition | Rationale |
|---|---|---|---|
| 1 | `Dataplane` trait | EXTEND | Add `probe()` method (Earned Trust). Existing `update_service` signature unchanged; `ServiceGeneration` increment is internal to `EbpfDataplane`. |
| 2 | `EbpfDataplane` | EXTEND | New `CONNTRACK_MAP` field; `start()` gains NOTRACK install + probes; `update_service()` increments generation internally. |
| 3 | `SimDataplane` | EXTEND | Per-CPU conntrack mirror; `migrate_flow_cpu` test surface; `probe()` returns Ok unconditionally. |
| 4 | `xdp_service_map` program | EXTEND | Conntrack lookup before Maglev hash; conntrack write on miss. |
| 5 | `tc_reverse_nat` program | EXTEND | `flow_state` promotion on SYN-ACK observation. |
| 6 | `crates/overdrive-bpf/src/shared/sanity.rs` | EXTEND | New `flow_key_from_packet` companion to existing `reverse_key_from_packet`. Same endianness lockstep contract. |
| 7 | `Backend` aggregate | REUSE | No new field. |
| 8 | `BackendId` newtype | REUSE | Carries through unchanged. |
| 9 | `ServiceMapHydrator` reconciler | REUSE | Conntrack is dataplane-internal; reconciler contract unchanged per ADR-0044 § Decision 8. |
| 10 | `Action::DataplaneUpdateService` | REUSE | Variant body unchanged; `ServiceGeneration` increment lives inside `EbpfDataplane::update_service`, not on the Action. |
| 11 | `service_hydration_results` ObservationStore table | REUSE | No schema change; the existing `fingerprint` field already encodes "what backend set was applied." |
| 12 | `ThreeIfaceTopology` test helper (ADR-0043) | EXTEND | NOTRACK install during Slice 06-04 (bridge); production-side NOTRACK in Slice 16-03 supersedes. |
| 13 | `aya-ebpf::maps::LruPerCpuHashMap<K, V>` | REUSE | Kernel-side typed wrapper exists in aya-ebpf 0.1.1 (research § A.1). |
| 14 | `aya::maps::Map::PerCpuLruHashMap(MapData)` | REUSE (with hand-rolled fd extraction) | Same hand-rolled-handle pattern as `Map::Unsupported(MapData)` HoM access. |
| 15 | `BPF_PROG_TEST_RUN` helper | REUSE | Existing `crates/overdrive-dataplane/src/sys/prog_test_run.rs`. |
| 16 | `RedbViewStore` / `IntentStore` / `ObservationStore` | REUSE | None of them touched by this feature. |
| 17 | `xtask bpf-build / bpf-unit / integration-test vm / verifier-regress / xdp-perf` | REUSE | All exist after Phase 2.2 Slice 07. |
| 18 | `FlowKey` newtype | CREATE NEW | No existing 5-tuple newtype. STRICT discipline per `development.md`. |
| 19 | `FlowEntry` POD struct | CREATE NEW | No existing per-flow-state struct. POD per kernel-side BPF map value contract. |
| 20 | `FlowState` enum | CREATE NEW | Stable variant set (`SynOnly=0, Established=1`); future-extensible per ADR-0037-style minor-version semantics. |
| 21 | `ServiceGeneration` newtype | CREATE NEW | No existing rotation-counter newtype; required for cross-rotation flow-affinity per ADR-0044 § Decision 3. |
| 22 | `iptables -t raw -j NOTRACK` install | CREATE NEW (production-side helper in `overdrive-dataplane`) | No existing iptables surface in production code. |
| 23 | Three DST invariants | CREATE NEW | No existing conntrack invariants (the feature creates this concern). |
| 24 | Three Earned Trust probes | CREATE NEW | No existing dataplane probes (this is the first feature where dataplane probing matters per CLAUDE.md § Principle 12). |

**Summary**: 17 EXTEND/REUSE; 7 CREATE NEW (4 newtypes + 1 helper +
3 invariants + 3 probes; the helper and probes share a creation
shape so they're often counted as 1 — depending on how you slice it,
the count is between 6 and 8). All CREATE NEW entries carry
"no existing alternative" justification.

---

## Technology Stack

OSS-only, all already in workspace `Cargo.toml`. **No new top-level
dependencies.**

| Dep | Version | License | Role |
|---|---|---|---|
| `aya` / `aya-ebpf` | 0.13.x | MIT-or-Apache-2 | `LruPerCpuHashMap<K, V>` typed kernel-side; userspace `Map::PerCpuLruHashMap(MapData)` with hand-rolled fd extraction |
| `iptables` | system tool | GPLv2 | NOTRACK rule installation; invoked via `std::process::Command` from `EbpfDataplane::start` |
| All others | — | — | Inherited from Phase 2.2 |

---

## Constraints established

The 10 Phase 2.2 constraints propagate verbatim except:

- **Constraint 2 (CONNTRACK is OUT) is FLIPPED.** This feature is
  why Constraint 2 was deferred to GH #154; pulling forward
  resolves it. The constraint becomes "Conntrack is per-CPU LRU,
  scoped to dataplane-managed VIPs only; kernel netfilter conntrack
  for non-dataplane traffic continues to operate normally." The
  scoping is enforced by NOTRACK rule shape (matches dataplane VIP
  range only).

New DESIGN-specific constraint:

- **Conntrack lookup MUST run before Maglev hash on the forward
  path.** Structural per ADR-0044 § Decision 3; without this, the
  flow-affinity guarantee disappears.

---

## Upstream changes

**Two**:

- New ADR (ADR-0044) added to the brief.md ADR index.
- `phase-2-xdp-service-map`'s constraint 2 ("Conntrack is OUT") is
  amended in the parent feature's `architecture.md` § 2 with a
  forward reference to this feature; the constraint stays in place
  as Phase 2.2's *own* boundary (Phase 2.2 ships stateless), and
  the amendment is purely "see also Phase 2.16 ADR-0044."

No edits to `whitepaper.md`, `commercial.md`, `.claude/rules/*`,
or any other SSOT file outside `docs/product/architecture/` and
this feature directory + the parent feature directory's narrow
amendment.

---

## Changelog

| Date | Change |
|---|---|
| 2026-05-06 | Initial DESIGN wave. Six dispatched decisions answered with rationale; ADR-0044 authored as the design lockpoint; six-step roadmap structure proposed; bridge-fix-then-strategic-fix path locked. — Morgan. |
