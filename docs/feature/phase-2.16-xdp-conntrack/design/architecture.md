# Architecture — phase-2.16-xdp-conntrack

**Author**: Morgan (Solution Architect)
**Date**: 2026-05-06
**Status**: PROPOSED — pending user ratification + reviewer pass
**Mode**: propose
**Companion artifacts**: `wave-decisions.md` (decision log);
ADR-0044 (full design lockpoint); `discuss/brief.md` (feature
problem statement).

This document is the architectural specification for Phase 2.16
(GH #154 — *Conntrack*). It composes with
`docs/feature/phase-2-xdp-service-map/design/architecture.md`
(parent feature, Phase 2.2) without rewriting any prior decision.
This feature flips Phase 2.2's Constraint 2 ("Conntrack is OUT")
and adds three primitives: a per-CPU LRU conntrack BPF map, a
NOTRACK installation contract, and three Earned Trust probes.

---

## § 1 Goal & scope

Add **dataplane-owned conntrack** to the Phase 2.2 stateless XDP
forwarder, so:

1. In-flight TCP flows survive arbitrarily-many SERVICE_MAP
   rotations (closes the §15 *Zero Downtime Deployments* claim
   under aggressive canary cutover).
2. Kernel netfilter conntrack does NOT interfere with
   dataplane-managed flows — payload-bearing TCP segments traverse
   the LB without being silently dropped by `nf_conntrack_in()`'s
   window check (closes S-2.2-17).
3. The dataplane refuses to start (with structured
   `health.startup.refused`) if any of three substrate honesty
   probes fails, per CLAUDE.md § Principle 12.

**Whitepaper anchors**: § 7 (eBPF Dataplane / XDP fast path),
§ 15 (Zero Downtime Deployments).
**ADR anchor**: ADR-0044 (this feature's design lockpoint).
**Companion ADRs**: ADR-0040, ADR-0041, ADR-0042, ADR-0043 — all
reused without supersession.

---

## § 2 Constraints

The 10 Phase 2.2 constraints propagate verbatim except:

- **Constraint 2 ("Conntrack is OUT") is FLIPPED.** Phase 2.16's
  raison d'être. The constraint becomes: "Conntrack is per-CPU
  LRU, scoped to dataplane-managed VIPs only; kernel netfilter
  conntrack for non-dataplane traffic continues to operate
  normally." Scoping is enforced by NOTRACK rule shape (matches
  dataplane VIP range only).

New DESIGN-specific constraints:

- **Conntrack lookup MUST run before Maglev hash on the forward
  path.** Without this ordering, the flow-affinity guarantee
  disappears.
- **NOTRACK rule installation MUST happen before any dataplane
  traffic is admitted.** Composition root invariant per CLAUDE.md
  § Principle 12 ("wire then probe then use"); see ADR-0044
  § Decision 5.
- **`ServiceGeneration` increment MUST happen inside
  `Dataplane::update_service` atomically with the SERVICE_MAP
  outer-map slot rotation.** A rotation visible to the conntrack
  lookup with the *old* generation still in the slot would mis-pin
  flows; structural ordering required.

---

## § 3 The six dispatched decisions (locked)

Each restated from `wave-decisions.md` for traceability. ADR-0044
carries the full rationale.

| # | Decision | Locked option |
|---|---|---|
| 1 | Scope and feature boundary | (b) New sibling feature `phase-2.16-xdp-conntrack` |
| 2 | Conntrack data structure | per-CPU LRU `BPF_MAP_TYPE_LRU_PERCPU_HASH`; key = `FlowKey` (5-tuple, pre-NAT, host-order); value = `FlowEntry { backend_id, service_generation, last_seen_secs, flow_state, _pad }`; eviction = kernel-native LRU; capacity = 1_048_576 / CPU |
| 3 | Integration points | Forward path writes; reverse path promotes SynOnly→Established; no userspace eviction signal; hydrator unchanged |
| 4 | Slice structure | 6 steps (16-01..16-06); IDs land via `/nw-roadmap` |
| 5 | Effects on Slice 06 | Pause Slice 06 at 06-02 GREEN; resume to 06-03..06-05; bridge fix in 06-04 |
| 6 | S-2.2-17 unblocking | Bridge fix in Slice 06-04 (single-line NOTRACK in test helper); strategic fix in Slice 16-03 (production-side NOTRACK + test-side bridge deletion, single-cut) |

---

## § 4 Reuse Analysis (HARD GATE)

See `wave-decisions.md` § "Reuse Analysis" for the full 24-row
table. Summary:

- **17 EXTEND/REUSE** — the trait surface, every existing kernel
  program, both ObservationStore tables, the hydrator reconciler,
  the action shim, and the test infrastructure are all reused.
- **7 CREATE NEW** — 4 newtypes (`FlowKey`, `FlowEntry`,
  `FlowState`, `ServiceGeneration`), 1 helper (NOTRACK install),
  3 invariants, 3 probes. Each justified per
  `wave-decisions.md`.

**No unjustified CREATE NEW.**

---

## § 5 Module layout

### `crates/overdrive-bpf/src/`

```
crates/overdrive-bpf/src/
├── programs/
│   ├── xdp_service_map.rs       # EXTEND — conntrack lookup
│   │                            # before Maglev; conntrack write
│   │                            # on miss; rotation-handling
│   │                            # algorithm per ADR-0044 § 3
│   └── tc_reverse_nat.rs        # EXTEND — flow_state promotion
├── maps/
│   ├── conntrack_map.rs         # NEW — CONNTRACK_MAP declaration
│   │                            # (LruPerCpuHashMap<FlowKey, FlowEntry>)
│   └── ...                      # existing maps unchanged
└── shared/
    └── sanity.rs                # EXTEND — flow_key_from_packet
                                 # companion to existing
                                 # reverse_key_from_packet;
                                 # endianness lockstep contract
                                 # extends to FlowKey identically
```

### `crates/overdrive-dataplane/src/`

```
crates/overdrive-dataplane/src/
├── ebpf_dataplane.rs            # EXTEND — start() installs NOTRACK
│                                # + runs Earned Trust probes;
│                                # update_service() increments
│                                # ServiceGeneration internally
├── maps/
│   └── conntrack_map_handle.rs  # NEW — typed userspace handle
│                                # over Map::PerCpuLruHashMap(MapData)
├── notrack.rs                   # NEW — iptables NOTRACK install
│                                # / uninstall; scoped match
└── probe.rs                     # NEW — three Earned Trust probes
                                 # per ADR-0044 § Decision 5
```

### `crates/overdrive-core/src/dataplane/`

```
crates/overdrive-core/src/dataplane/
├── flow_key.rs                  # NEW — FlowKey newtype
├── flow_entry.rs                # NEW — FlowEntry POD
├── flow_state.rs                # NEW — FlowState enum
├── service_generation.rs        # NEW — ServiceGeneration newtype
└── ...                          # existing types unchanged
```

### `crates/overdrive-sim/src/`

```
crates/overdrive-sim/src/
├── adapters/
│   └── dataplane.rs             # EXTEND — per-CPU conntrack mirror
│                                # per ADR-0044 § 6
└── invariants/
    └── conntrack/
        ├── mod.rs               # NEW
        ├── pins_under_rotation.rs            # NEW
        ├── eventually_converges.rs           # NEW
        └── notrack_refuses_start.rs          # NEW
```

### `crates/overdrive-dataplane/tests/integration/helpers/`

```
helpers/
├── netns.rs                     # EXTEND (Slice 06-04) — NOTRACK
│                                # bridge install in
│                                # ThreeIfaceTopology::setup
│                                # (deleted by Slice 16-03)
└── ...                          # existing helpers unchanged
```

---

## § 6 BPF map shape

Extends `phase-2-xdp-service-map/design/architecture.md` § 10 with
one new row.

| Map | Type | Key | Value | Notes |
|---|---|---|---|---|
| `CONNTRACK_MAP` | `BPF_MAP_TYPE_LRU_PERCPU_HASH` | `FlowKey` | `FlowEntry` | Per-CPU; capacity 1_048_576 entries / CPU. Kernel-side typed wrapper exists; userspace hand-rolled handle. |

The four other maps (`SERVICE_MAP`, `BACKEND_MAP`, `MAGLEV_MAP`,
`REVERSE_NAT_MAP`, `DROP_COUNTER`) are unchanged from
ADR-0040/0041.

`SERVICE_MAP` outer-map *value* is unchanged in shape (still
`inner-map fd`) but gains a *metadata field* the
`HashOfMapsHandle` carries alongside the fd: the
`ServiceGeneration` counter for that slot. Implementation
detail: the counter lives in a sidecar `BPF_MAP_TYPE_HASH<
ServiceVip×port, ServiceGeneration>` keyed identically to
SERVICE_MAP — the kernel-side conntrack lookup reads both maps
in the same hot path, both use the same lookup discipline (NULL-
check before deref). This sidecar map is *not* a new top-level
concept; it's an implementation detail of how the
`ServiceGeneration` is propagated to the kernel-side lookup.
Document in the implementation-shape comment, not a separate
top-level map row.

---

## § 7 Endianness lockstep — extension to ADR-0041 § 11

`FlowKey` carries the same lockstep contract as `ReverseKey`:

- **Wire format**: IPs and L4 ports in network byte order.
- **Map storage format**: host byte order.
- **Conversion site**: `crates/overdrive-bpf/src/shared/sanity.rs::
  flow_key_from_packet` (companion to existing
  `reverse_key_from_packet`).

**Lockstep guarantee**: Tier 2 BPF unit roundtrip
(`flow_key_roundtrip`) + userspace proptest in
`overdrive-dataplane::maps::conntrack_map_handle` round-trips
host-order writes against host-order reads. Same shape as
ADR-0041 § 11; same proptest discipline.

---

## § 8 Algorithm — XDP forward path with conntrack

```text
xdp_service_map(packet):
    sanity_prologue(packet)?                    # ADR-0040 Q3=C
    flow_key = flow_key_from_packet(packet)?    # FlowKey or DROP

    # Conntrack lookup (NEW)
    match CONNTRACK_MAP::get(&flow_key):
        Some(entry):
            current_gen = SERVICE_GEN_MAP::get(&service_key)?
            if entry.service_generation == current_gen:
                # Fast path — pinned backend still current
                forward_to(entry.backend_id)
                return XDP_TX
            else:
                # SERVICE_MAP rotated since flow was pinned
                if entry.flow_state == Established:
                    forward_to(entry.backend_id)
                    # Do NOT refresh entry — let LRU expire
                    return XDP_TX
                else:  # SynOnly
                    CONNTRACK_MAP::delete(&flow_key)
                    # Fall through to Maglev re-hash
        None:
            pass  # fall through

    # Maglev path (existing — ADR-0041)
    backend_id = maglev_lookup(flow_key.vip, flow_key.vip_port,
                               flow_key.client_ip,
                               flow_key.client_port)?

    # Conntrack write (NEW)
    CONNTRACK_MAP::insert(&flow_key, &FlowEntry {
        backend_id,
        service_generation: current_gen,
        last_seen_secs: bpf_ktime_get_ns() / 1_000_000_000,
        flow_state: SynOnly,
        _pad: [0; 3],
    })

    forward_to(backend_id)
    return XDP_TX
```

The "Established promotion" happens on the reverse path in
`tc_reverse_nat`:

```text
tc_reverse_nat(packet):
    sanity_prologue(packet)?
    reverse_key = reverse_key_from_packet(packet)?
    original_dest = REVERSE_NAT_MAP::get(&reverse_key)?

    # Build the FORWARD FlowKey from the reverse key + original dest
    forward_key = FlowKey {
        client_ip: reverse_key.client_ip,
        client_port: reverse_key.client_port,
        vip: original_dest.vip,
        vip_port: original_dest.vip_port,
        proto: reverse_key.proto,
        _pad: [0; 3],
    }

    # Promote SynOnly → Established (NEW)
    if let Some(mut entry) = CONNTRACK_MAP::get(&forward_key) {
        if entry.flow_state == SynOnly {
            entry.flow_state = Established
            CONNTRACK_MAP::insert(&forward_key, &entry)
        }
    }

    rewrite_dst(packet, original_dest.vip, original_dest.vip_port)
    return TC_ACT_OK
```

Verifier-budget impact: the forward path adds **two map lookups**
(CONNTRACK_MAP + sidecar SERVICE_GEN_MAP) and **one conditional
write** (CONNTRACK_MAP::insert on miss). The reverse path adds
**one map lookup** + **one conditional write** (the SynOnly →
Established promotion, fires once per flow at SYN-ACK time). Per
ADR-0044 § Consequences, the gate (Phase 2.2 Slice 07) is in
place to measure the delta.

---

## § 9 Earned Trust probes (CLAUDE.md § Principle 12)

Per ADR-0044 § Decision 5. Implementation lives at
`crates/overdrive-dataplane/src/probe.rs`. Three probes; each
returns `Result<(), ProbeError>`; on any failure,
`EbpfDataplane::start` returns `Err(StartError::ProbeFailed
{ probe, kernel_version, errno, ... })` and emits a structured
`health.startup.refused` event.

**Subtype check**: `Dataplane::probe(&self) -> impl Future<Output =
Result<(), ProbeError>>` is a method on the trait; mypy/Rust's
type system rejects calls that skip the probe (the production
wiring's composition root MUST call `probe().await?` before
`start()`). Enforced at compile time.

**Structural check**: an AST pre-commit hook walking
`overdrive-dataplane`'s `EbpfDataplane::start` body asserts the
ordered call sequence `probe().await? → install_notrack().await?
→ <traffic admit>`. Per CLAUDE.md § Principle 12's
"three semantically orthogonal layers" requirement.

**Behavioural check**: a CI gold-test runner exercises catalogued
substrate lies — kernel without `BPF_MAP_TYPE_LRU_PERCPU_HASH`,
host without `iptables -t raw`, kernel that ignores NOTRACK on
hardware-offloaded paths — and asserts `start()` returns Err in
each case.

The behavioural-check catalogue lives in
`crates/overdrive-dataplane/tests/integration/probe_substrate_lies.rs`
gated `integration-tests` and runs on every PR via the existing
Tier 3 lane.

---

## § 10 SimDataplane mirror (per CLAUDE.md § "Production code is not shaped by simulation")

`SimDataplane` extends with a per-CPU mirror of `CONNTRACK_MAP`
keyed by `(cpu_index, FlowKey) → FlowEntry`, with deterministic
LRU eviction matching the kernel's. The DST harness chooses the
target CPU per packet via the injected `Entropy` port (seeded
RNG, modelling RSS deterministically).

**Critical**: the production code path MUST NOT carry any
sim-shaped concession. The kernel-side and userspace code shape
is dictated by the production substrate; `SimDataplane` mirrors
the contract, not the production-shape. If a future
SimDataplane change requires a production-side workaround, the
sim adapter is wrong and must be reshaped — per
`development.md` § "Production code is not shaped by simulation".

`SimDataplane` exposes for harness inspection:

```rust
impl SimDataplane {
    pub fn conntrack_lookup(&self, cpu: u32, key: &FlowKey) -> Option<FlowEntry>;
    pub fn conntrack_insert(&mut self, cpu: u32, key: FlowKey, entry: FlowEntry);
    pub fn conntrack_evict_oldest(&mut self, cpu: u32) -> Option<(FlowKey, FlowEntry)>;
    pub fn migrate_flow_cpu(&mut self, key: &FlowKey, new_cpu: u32);
    pub fn rotate_service_generation(&mut self, vip: ServiceVip, port: u16);
}
```

The DST invariants in § 11 below exercise these surfaces.

---

## § 11 DST invariants (locked names from ADR-0044)

| Name | Property |
|---|---|
| `ConntrackPinsBackendUnderRotation` | `assert_always!`: `flow_state == Established && service_generation_stale → forward_to(entry.backend_id)` until LRU evicts. |
| `ConntrackEventuallyConvergesAfterRotation` | `assert_eventually!`: after a rotation + LRU-expiry-window of ticks with no new packets on stale flows, `conntrack.entries().filter(stale_generation).count() == 0`. |
| `NoTrackProbeRefusesStartOnFailure` | `assert_always!`: any probe failure produces `StartError::ProbeFailed` AND no traffic-admit side effect occurs. |

All three live in `crates/overdrive-sim/src/invariants/conntrack/`
and run on every PR per `.claude/rules/testing.md` § Tier 1.

---

## § 12 Quality-attribute scenarios

| ASR | Quality attribute | Scenario | Pass criterion |
|---|---|---|---|
| ASR-2.16-01 | Reliability — flow affinity across canary cutover | Long-lived TCP session traverses the dataplane while SERVICE_MAP rotates 10 times during the connection's lifetime. | Connection survives; backend stays pinned for the duration. (DST: `ConntrackPinsBackendUnderRotation`.) |
| ASR-2.16-02 | Reliability — bounded conntrack memory | Conntrack table size at steady state ≤ 1_048_576 entries per CPU under arbitrary flow churn. | LRU eviction observed in DST + Tier 3 traffic-generator soak (10 M flow churn over 60 s). |
| ASR-2.16-03 | Correctness — kernel netfilter does not interfere | Real TCP connection with payload completes through the dataplane. | S-2.2-17 GREEN. |
| ASR-2.16-04 | Operability — startup refusal on substrate-lie | Test environment with `BPF_MAP_TYPE_LRU_PERCPU_HASH` disabled / `iptables -t raw` missing / NOTRACK ignored. | `EbpfDataplane::start` returns Err with `health.startup.refused`. (DST: `NoTrackProbeRefusesStartOnFailure`.) |
| ASR-2.16-05 | Maintainability — verifier-budget headroom under conntrack | Slice 16-06's `cargo xtask verifier-regress` after conntrack lands. | Delta ≤ 20 % vs Phase 2.2 Slice 06 baseline; absolute ≤ 60 % of 1M ceiling. |

---

## § 13 Roadmap-step input (for `/nw-roadmap` to dispatch)

The structured input the orchestrator's `/nw-roadmap` will
consume to land the six new steps in
`docs/feature/phase-2.16-xdp-conntrack/deliver/roadmap.json`.
Each step's `acceptance_criteria` is illustrative — the
acceptance-designer (Atlas) writes the AC bodies during DISTILL.

```json
[
  {
    "id": "16-01",
    "name": "FlowKey/FlowEntry/FlowState/ServiceGeneration newtypes; SimDataplane mirror; three DST invariants land RED",
    "criteria": [
      "FlowKey newtype in overdrive-core::dataplane with full STRICT discipline",
      "FlowEntry POD struct in overdrive-core::dataplane (serde / rkyv / proptest; no FromStr — POD shape)",
      "FlowState enum in overdrive-core::dataplane (#[repr(u8)], two variants, ADR-0037-style stable discriminants)",
      "ServiceGeneration newtype in overdrive-core::dataplane",
      "SimDataplane per-CPU conntrack mirror; conntrack_lookup / conntrack_insert / migrate_flow_cpu / rotate_service_generation surfaces",
      "Three DST invariants land RED at module-creation time"
    ],
    "scenario_ref": "S-2.16-01",
    "scenario_name": "newtype_module_completeness",
    "test_file": "crates/overdrive-core/tests/newtype_roundtrip.rs",
    "implementation_scope": [
      "crates/overdrive-core/src/dataplane/flow_key.rs",
      "crates/overdrive-core/src/dataplane/flow_entry.rs",
      "crates/overdrive-core/src/dataplane/flow_state.rs",
      "crates/overdrive-core/src/dataplane/service_generation.rs",
      "crates/overdrive-sim/src/adapters/dataplane.rs",
      "crates/overdrive-sim/src/invariants/conntrack/mod.rs",
      "crates/overdrive-sim/src/invariants/conntrack/pins_under_rotation.rs",
      "crates/overdrive-sim/src/invariants/conntrack/eventually_converges.rs",
      "crates/overdrive-sim/src/invariants/conntrack/notrack_refuses_start.rs"
    ],
    "effort_hours": 5,
    "tier": 1,
    "deps": [],
    "mutation_target": "crates/overdrive-core",
    "risk": "low"
  },
  {
    "id": "16-02",
    "name": "CONNTRACK_MAP declaration + ConntrackMapHandle + Earned Trust probes",
    "criteria": [
      "CONNTRACK_MAP declared in overdrive-bpf::maps::conntrack_map (LruPerCpuHashMap<FlowKey, FlowEntry>)",
      "ConntrackMapHandle in overdrive-dataplane::maps::conntrack_map_handle with get/insert/remove/entry_count surface",
      "Three Earned Trust probes wired into Dataplane::probe()",
      "Probe failure produces StartError::ProbeFailed with structured health.startup.refused event",
      "Tier 3 substrate-lie tests pass on the kernel matrix"
    ],
    "scenario_ref": "S-2.16-02",
    "scenario_name": "conntrack_map_handle_and_probes",
    "test_file": "crates/overdrive-dataplane/tests/integration/probe_substrate_lies.rs",
    "implementation_scope": [
      "crates/overdrive-bpf/src/maps/conntrack_map.rs",
      "crates/overdrive-dataplane/src/maps/conntrack_map_handle.rs",
      "crates/overdrive-dataplane/src/probe.rs",
      "crates/overdrive-dataplane/tests/integration/probe_substrate_lies.rs"
    ],
    "effort_hours": 6,
    "tier": 3,
    "deps": ["16-01"],
    "mutation_target": "crates/overdrive-dataplane",
    "risk": "medium",
    "risk_mitigation": "ADR-0044 § Decision 5 — three orthogonal probe layers; substrate-lie catalogue runs on every PR"
  },
  {
    "id": "16-03",
    "name": "NOTRACK install in production EbpfDataplane::start; remove test-side bridge",
    "criteria": [
      "EbpfDataplane::start installs iptables -t raw -A PREROUTING -j NOTRACK scoped to dataplane VIP range",
      "EbpfDataplane Drop removes the NOTRACK rule",
      "ThreeIfaceTopology::setup test-side bridge (added in Slice 06-04) is DELETED in this commit",
      "S-2.2-17 GREEN under both Lima and ubuntu-latest CI"
    ],
    "scenario_ref": "S-2.16-03",
    "scenario_name": "notrack_install_production_path_unblocks_s2217",
    "test_file": "crates/overdrive-dataplane/tests/integration/reverse_nat_e2e.rs",
    "implementation_scope": [
      "crates/overdrive-dataplane/src/notrack.rs",
      "crates/overdrive-dataplane/src/ebpf_dataplane.rs",
      "crates/overdrive-dataplane/tests/integration/helpers/netns.rs"
    ],
    "effort_hours": 4,
    "tier": 3,
    "deps": ["16-02"],
    "mutation_target": "crates/overdrive-dataplane",
    "risk": "low"
  },
  {
    "id": "16-04",
    "name": "XDP forward conntrack lookup-write + ServiceGeneration increment in Dataplane::update_service",
    "criteria": [
      "xdp_service_map performs CONNTRACK_MAP lookup before Maglev hash",
      "Miss → Maglev hash → CONNTRACK_MAP insert with FlowState::SynOnly",
      "Hit-with-current-generation → fast-path forward; entry refreshed",
      "Hit-with-stale-generation → branch on flow_state per ADR-0044 § Decision 3 step 4",
      "EbpfDataplane::update_service increments ServiceGeneration atomically with SERVICE_MAP outer-map slot rotation",
      "ConntrackPinsBackendUnderRotation + ConntrackEventuallyConvergesAfterRotation invariants GREEN"
    ],
    "scenario_ref": "S-2.16-04",
    "scenario_name": "forward_path_conntrack_lookup_and_write",
    "test_file": "crates/overdrive-bpf/tests/integration/xdp_service_map_with_conntrack.rs",
    "implementation_scope": [
      "crates/overdrive-bpf/src/programs/xdp_service_map.rs",
      "crates/overdrive-bpf/src/shared/sanity.rs",
      "crates/overdrive-dataplane/src/ebpf_dataplane.rs",
      "crates/overdrive-bpf/tests/integration/xdp_service_map_with_conntrack.rs",
      "crates/overdrive-dataplane/tests/integration/canary_rotation_pin.rs"
    ],
    "effort_hours": 8,
    "tier": 3,
    "deps": ["16-03", "08-03"],
    "mutation_target": "crates/overdrive-bpf",
    "risk": "medium",
    "risk_mitigation": "ESR pair landing GREEN gates the §15 zero-downtime claim; verifier-budget delta measured against Slice 06 baseline"
  },
  {
    "id": "16-05",
    "name": "TC reverse path SynOnly→Established promotion; NoTrackProbeRefusesStartOnFailure GREEN",
    "criteria": [
      "tc_reverse_nat builds forward_key from (reverse_key, OriginalDest)",
      "On forward_key hit with flow_state == SynOnly: update to Established; insert back",
      "On miss or already-Established: no write",
      "NoTrackProbeRefusesStartOnFailure invariant GREEN"
    ],
    "scenario_ref": "S-2.16-05",
    "scenario_name": "reverse_path_synonly_to_established_promotion",
    "test_file": "crates/overdrive-bpf/tests/integration/tc_reverse_nat_with_conntrack.rs",
    "implementation_scope": [
      "crates/overdrive-bpf/src/programs/tc_reverse_nat.rs",
      "crates/overdrive-bpf/tests/integration/tc_reverse_nat_with_conntrack.rs"
    ],
    "effort_hours": 4,
    "tier": 2,
    "deps": ["16-04"],
    "mutation_target": "crates/overdrive-bpf",
    "risk": "low"
  },
  {
    "id": "16-06",
    "name": "Tier 4 verifier-regress + xdp-perf baseline update",
    "criteria": [
      "perf-baseline/main/verifier-budget/veristat-service-map.txt updated with conntrack delta",
      "perf-baseline/main/verifier-budget/veristat-reverse-nat.txt updated with promotion delta",
      "perf-baseline/main/xdp-perf/ updated; pps regression within 5% gate",
      "Commit message documents delta + kernel + commit SHA"
    ],
    "scenario_ref": "S-2.16-06",
    "scenario_name": "verifier_budget_and_xdp_perf_baseline_post_conntrack",
    "test_file": "perf-baseline/main/verifier-budget/veristat-service-map.txt",
    "implementation_scope": [
      "perf-baseline/main/verifier-budget/veristat-service-map.txt",
      "perf-baseline/main/verifier-budget/veristat-reverse-nat.txt",
      "perf-baseline/main/xdp-perf/"
    ],
    "effort_hours": 2,
    "tier": 4,
    "deps": ["16-05"],
    "mutation_target": "skip-rationale: baseline files are generated artifacts with no operator sites; mutants empty-filter is a vacuous pass",
    "risk": "low"
  }
]
```

---

## § 14 Handoff to DISTILL

The acceptance designer (Atlas) consumes:

1. This document (`design/architecture.md`)
2. `design/wave-decisions.md`
3. ADR-0044 in `docs/product/architecture/`
4. `discuss/brief.md` (problem statement; no full DISCUSS wave —
   per § "Why no full DISCUSS wave" in brief.md)
5. The parent feature's full DISCUSS / DESIGN artifact set (the
   constraint inheritance is from there)

Atlas's Phase 2 extracts AC into
`distill/test-scenarios.md` per the established pattern. The
three DST invariants in § 11 land as concrete property tests in
`crates/overdrive-sim/src/invariants/conntrack/`.

---

## § 15 Open uncertainty (surfaced explicitly per dispatch)

Three things the dispatch asked to call out where they are still
uncertain:

1. **Sidecar `SERVICE_GEN_MAP` vs in-band SERVICE_MAP outer-map
   value extension.** § 6 documents the sidecar shape. An
   alternative is to extend the SERVICE_MAP outer-map *value*
   shape to carry both the inner-map fd AND the
   `ServiceGeneration` counter — this would save one BPF map
   lookup on the forward path. Rejected as the locked design
   because: (a) the outer-map value shape is part of ADR-0040's
   locked surface; extending it touches a stable contract; (b)
   the sidecar adds one verifier-budget map lookup but keeps the
   contract surface narrow. **If Slice 16-06's verifier-budget
   delta exceeds 20 %, raise an ADR amendment to fold the counter
   into the outer-map value shape.**
2. **Conntrack TTL beyond LRU.** The kernel-native LRU eviction
   is the only TTL primitive in this design. Some operator
   workloads (e.g., short-lived UDP flows that should not
   occupy a conntrack slot for the entire LRU window) would
   benefit from a hard TTL on `last_seen_secs`. Deferred — the
   `last_seen_secs` field exists for this future extension; the
   policy that consults it is operator-tunable Phase 3+ work.
3. **Cross-CPU flow migration empirical bound.** ADR-0044
   § Decision 1's rationale claims "the new entry usually
   selects the same backend on re-hash" with the Maglev ≤ 1 %
   guarantee. This is structurally true but the empirical
   bound under realistic RSS profiles has not been measured.
   Slice 16-06's perf gate measures this implicitly (cross-CPU
   re-hash cost shows up in the pps delta); if the delta
   exceeds the 5 % gate, the Cilium shared-LRU shape becomes
   the fallback (raise ADR amendment).

These three are flagged here, not papered over; the user can
weigh in on whether any of them warrants pulling forward.

---

*End of architecture.md. This document is read-only at handoff
time. Future amendments require a new ADR with `supersedes` /
`amends` semantics per `brief.md` ADR convention.*
