# Phase 2.2 DESIGN proposal — continuation (§4 onward)

> §0–§3 are recorded in the prior chat turn (mode confirmation, multi-architect
> context read of `brief.md`, requirements + constraints crystallisation,
> existing-system analysis of `overdrive-bpf` / `overdrive-dataplane` /
> `overdrive-core::reconciler` / port traits). This file picks up at §4
> where the orchestrator's tool-output cap truncated the previous reply
> mid-`Action::DataplaneUpdateService` body.

---

## §4 — `Action::DataplaneUpdateService` — locked variant body

The hydrator reconciler emits exactly one Action variant. The variant lands
in `crates/overdrive-core/src/reconciler.rs`, appended to the existing
`pub enum Action` block (currently terminating at `RestartAllocation`),
under a new section header:

```text
// -----------------------------------------------------------------
// phase-2-xdp-service-map — dataplane-hydration variants
// (US-08, J-PLAT-004). The action shim's `dispatch(actions, ...)`
// consumes this variant and calls `Dataplane::update_service` per
// ADR-0023. Failure surfaces are observation rows (see below);
// no `terminal` channel — service hydration cannot terminate an
// allocation.
// -----------------------------------------------------------------
```

### Variant body (locked)

```rust
/// Replace the backend set for a service VIP in the kernel-side
/// `SERVICE_MAP` / `BACKEND_MAP` / `MAGLEV_MAP` tuple.
///
/// Emitted by the `service-map-hydrator` reconciler when the
/// `service_backends` ObservationStore rows for a `ServiceId`
/// produce a fingerprint distinct from the one persisted in the
/// reconciler's `View`. The action shim consumes this variant,
/// invokes `Dataplane::update_service(service_id, vip, backends)`,
/// and writes the outcome into the `service_hydration_results`
/// observation row (see below). The next reconcile tick reads
/// that row via `actual` and either advances (Completed) or
/// retries on the next backend-set change (Failed).
///
/// `Vec<Backend>` carries weighted backends in deterministic
/// `BTreeMap<BackendId, Backend>::iter()` order — Maglev table
/// generation is byte-deterministic across nodes given identical
/// inputs (see DISCUSS Decision 8 + Constraint
/// "Determinism in the hydrator-side userspace logic").
DataplaneUpdateService {
    /// Identity of the service whose backend set is being
    /// rewritten. Maps 1:1 to a `SERVICE_MAP` outer-map key.
    service_id: ServiceId,
    /// Virtual IP the kernel-side XDP program matches incoming
    /// packets against. Carried explicitly (rather than re-derived
    /// from `ServiceId`) so the shim never needs to look back at
    /// `service_backends` to dispatch.
    vip: ServiceVip,
    /// Backend set, in deterministic iteration order. The shim
    /// passes this slice straight into
    /// `Dataplane::update_service`; userspace Maglev permutation
    /// generation reads it in this exact order.
    backends: Vec<Backend>,
    /// Cause-to-response linkage per the existing `HttpCall`
    /// pattern. Derived from `(service_id, fingerprint)` so the
    /// next tick can locate the `service_hydration_results` row
    /// deterministically. Required, not optional — service
    /// hydration is correlation-keyed end-to-end.
    correlation: CorrelationKey,
},
```

`Backend` is the existing `overdrive-core` aggregate (already used by
`service_backends` observation rows); no new field on it.

### Failure surface — observation, NOT `TerminalCondition`

The action shim wraps `Dataplane::update_service(...)` and:

- **On `Ok(())`** — writes `service_hydration_results` row with
  `status: Completed { fingerprint, applied_at: tick.now }`.
- **On `Err(DataplaneError::*)`** — writes `service_hydration_results`
  row with `status: Failed { reason: Display::to_string(&err),
  failed_at: tick.now }`.

The shim's error type for the dispatch wrapper is a new
`ServiceHydrationDispatchError` enum in
`crates/overdrive-control-plane/src/action_shim/service_hydration.rs`
with `#[from]` pass-through for `DataplaneError` per `development.md`
§ Errors / pass-through embedding. The variant does NOT carry a
`terminal: Option<TerminalCondition>` field (per ADR-0037) —
`TerminalCondition` is exclusively for *allocation lifecycle*
terminal claims. A failed service-map hydration does not terminate
any allocation; it is a pure observation surface that the next
reconcile tick reads and retries against. Mixing the two channels
would erode the ADR-0037 invariant that "every terminal claim has
a single typed source." The hydrator's retry-budget logic lives
in its `View` (see §5).

### One-line file landing

| File | Change |
|---|---|
| `crates/overdrive-core/src/reconciler.rs` | Append `DataplaneUpdateService` variant to `pub enum Action` block. |

---

## §5 — `ServiceMapHydrator` reconciler design

### Identity

```rust
const NAME: &str = "service-map-hydrator";
fn name(&self) -> &ReconcilerName { &self.name } // ReconcilerName::new_const(NAME)
```

### Per-target keying

Target = `ServiceId`. The evaluation broker keys evaluations on
`(reconciler_name, ServiceId)` per ADR-0023's storm-proof ingress —
a row-change burst on N backends of one service collapses to ONE
pending evaluation, not N.

### `type State = ServiceMapHydratorState`

Two parallel projections keyed on `ServiceId`:

```rust
pub struct ServiceMapHydratorState {
    /// Per-service desired backend set, hydrated from
    /// `service_backends` observation rows for the target
    /// `ServiceId`. Keyed `BTreeMap` (NOT `HashMap`) per
    /// development.md § Ordered-collection choice — deterministic
    /// iteration is what makes Maglev permutation byte-identical.
    pub desired: BTreeMap<ServiceId, ServiceDesired>,
    /// Per-service last-known hydration outcome from the
    /// `service_hydration_results` ObservationStore table —
    /// the `actual` projection observes the dataplane's
    /// confirmed state, not the next-action prediction.
    pub actual: BTreeMap<ServiceId, ServiceHydrationStatus>,
}

pub struct ServiceDesired {
    pub vip: ServiceVip,
    pub backends: Vec<Backend>,           // BTreeMap-sorted
    pub fingerprint: BackendSetFingerprint, // u64, content-hash of
                                          // (vip, backends) per
                                          // development.md § Hashing
                                          // requires deterministic
                                          // serialization (rkyv-archived).
}

pub enum ServiceHydrationStatus {
    Pending,    // no row yet
    Completed { fingerprint: BackendSetFingerprint, applied_at: UnixInstant },
    Failed     { fingerprint: BackendSetFingerprint, failed_at: UnixInstant,
                 reason: String },
}
```

**Decision: `actual` is a dedicated `service_hydration_results`
observation row, NOT a derivation of the last-emitted action.** Per
`development.md` § Persist inputs, not derived state — and per the
state-layer table in the same document — the reconciler's `actual`
must observe what *is*, not what was *predicted*. The shim writes
the row after the dataplane call returns; the next reconcile tick
reads it. Retries are driven by fingerprint mismatch
(`desired.fingerprint != actual.fingerprint`), not by re-emitting on
every tick. Deriving `actual` from "the last action I emitted" would
turn the hydrator into a write-only loop that cannot detect a
silently-failed dataplane update — exactly the failure mode
J-PLAT-004 is meant to close.

### `type View = ServiceMapHydratorView` — persists inputs

Per `development.md` § Persist inputs, not derived state — store the
inputs to the retry policy, never the deadline:

```rust
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct ServiceMapHydratorView {
    /// Per-service retry memory. `attempts` increments only on
    /// `DataplaneUpdateService` dispatch (NOT every tick); reset
    /// to 0 on Completed observation.
    pub retries: BTreeMap<ServiceId, RetryMemory>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct RetryMemory {
    pub attempts: u32,
    pub last_failure_seen_at: UnixInstant,
    /// The fingerprint we last attempted to hydrate. If
    /// `desired.fingerprint` differs from this AND from
    /// `actual.fingerprint`, we are mid-flight on an older attempt
    /// AND the desired set has shifted again — the next emit
    /// targets the new desired, retries reset to 0.
    pub last_attempted_fingerprint: Option<BackendSetFingerprint>,
}
```

The next-attempt deadline is **recomputed every tick** as
`last_failure_seen_at + backoff_for_attempt(attempts)`. Never
persisted. `BTreeMap` per § Ordered-collection choice.

### `reconcile` skeleton (sync, pure, no `.await`, no wall-clock read)

```rust
fn reconcile(
    &self,
    desired: &ServiceMapHydratorState,
    actual:  &ServiceMapHydratorState,
    view:    &ServiceMapHydratorView,
    tick:    &TickContext,
) -> (Vec<Action>, ServiceMapHydratorView) {
    let mut actions   = Vec::new();
    let mut next_view = view.clone();

    for (service_id, want) in &desired.desired {
        let have    = actual.actual.get(service_id);
        let retry   = view.retries.get(service_id).cloned().unwrap_or_default();

        let needs_emit = match have {
            None                                        => true,
            Some(ServiceHydrationStatus::Pending)       => false,
            Some(ServiceHydrationStatus::Completed { fingerprint, .. })
                if *fingerprint == want.fingerprint     => false,
            Some(ServiceHydrationStatus::Completed { .. }) => true, // drift
            Some(ServiceHydrationStatus::Failed { fingerprint, failed_at, .. })
                if *fingerprint != want.fingerprint     => true,    // new desired
            Some(ServiceHydrationStatus::Failed { failed_at, .. }) => {
                tick.now_unix
                    >= *failed_at + backoff_for_attempt(retry.attempts)
            }
        };

        if needs_emit {
            actions.push(Action::DataplaneUpdateService {
                service_id:  service_id.clone(),
                vip:         want.vip.clone(),
                backends:    want.backends.clone(),
                correlation: CorrelationKey::from((
                    service_id.clone(), want.fingerprint, "hydrate-service-map"
                )),
            });
            let r = next_view.retries.entry(service_id.clone()).or_default();
            r.attempts                  = r.attempts.saturating_add(1);
            r.last_failure_seen_at      = tick.now_unix;
            r.last_attempted_fingerprint = Some(want.fingerprint);
        }

        // Reset attempts on confirmed convergence
        if let Some(ServiceHydrationStatus::Completed { fingerprint, .. }) = have {
            if *fingerprint == want.fingerprint {
                next_view.retries.remove(service_id);
            }
        }
    }

    // GC stale view rows: services no longer in desired
    next_view.retries.retain(|sid, _| desired.desired.contains_key(sid));

    (actions, next_view)
}
```

**Invariant: at most one `DataplaneUpdateService` per tick per
`ServiceId`.** Enforced structurally — `desired.desired` is a `BTreeMap`
keyed on `ServiceId`, the loop emits once per key, the broker collapses
multi-row bursts to one evaluation per `(reconciler, service_id)`.

### Hydration shape (runtime-owned, NOT in `reconcile`)

| Projection | Source | Hydrator |
|---|---|---|
| `desired.desired` | `service_backends` ObservationStore rows for the target `ServiceId` | `hydrate_desired(target: &ServiceId, obs: &dyn ObservationStore)` reads rows, sorts into a `BTreeMap`, computes `BackendSetFingerprint = sha256(rkyv::to_bytes(&(vip, backends)))` truncated to `u64` |
| `actual.actual`   | `service_hydration_results` ObservationStore row for the target `ServiceId` | `hydrate_actual(target: &ServiceId, obs: &dyn ObservationStore)` |
| `view.retries`    | `RedbViewStore::bulk_load` at register-time + `write_through` after each tick | runtime-owned per ADR-0035; reconciler never sees the store |

`hydrate_desired` and `hydrate_actual` are async (they read
`ObservationStore`) and live on `AnyReconciler` per ADR-0036 — the
runtime calls them, packages results with a `TickContext`, and calls
the sync `reconcile`. The reconciler author writes `reconcile` only.

### ESR pair (locked names from DISCUSS)

| DST invariant | Property |
|---|---|
| `HydratorEventuallyConverges` | For every `service_id`, `actual.fingerprint == desired.fingerprint` is reached within a bounded number of ticks given a stable `desired`. |
| `HydratorIdempotentSteadyState` | Once `actual.fingerprint == desired.fingerprint` for all services, the hydrator emits zero `DataplaneUpdateService` actions per tick. |

Both live in `crates/overdrive-sim/src/invariants/` and run on every PR
per `.claude/rules/testing.md` § Tier 1.

---

## §6 — Module layout

### `crates/overdrive-bpf/src/`

```
crates/overdrive-bpf/src/
├── lib.rs                       # `#![no_std]` crate root; re-exports
├── programs/
│   ├── mod.rs
│   ├── xdp_service_map.rs       # XDP attach @ NIC; Slices 02-04 + 06
│   └── tc_reverse_nat.rs        # TC egress hook; Slice 05
├── maps/
│   ├── mod.rs
│   ├── service_map.rs           # SERVICE_MAP (HASH_OF_MAPS outer)
│   ├── backend_map.rs           # BACKEND_MAP
│   ├── maglev_map.rs            # MAGLEV_MAP (HASH_OF_MAPS outer)
│   ├── reverse_nat_map.rs       # REVERSE_NAT_MAP
│   └── drop_counter.rs          # DROP_COUNTER (PERCPU_ARRAY)
└── shared/
    ├── mod.rs
    └── sanity.rs                # `#[inline(always)]` prologue helpers
                                 # — Q3=C shared shape per Slice 06
```

### `crates/overdrive-dataplane/src/`

```
crates/overdrive-dataplane/src/
├── lib.rs                       # re-exports `EbpfDataplane`
├── ebpf_dataplane.rs            # impl `Dataplane` for `EbpfDataplane`
├── loader.rs                    # aya-rs program load + attach
├── maps/
│   ├── mod.rs
│   ├── service_map_handle.rs    # typed `ServiceMapHandle` newtype
│   ├── backend_map_handle.rs
│   ├── maglev_map_handle.rs
│   ├── reverse_nat_map_handle.rs
│   └── drop_counter_handle.rs
├── swap.rs                      # atomic HASH_OF_MAPS inner-map swap
│                                # (Slice 03 — zero-drop primitive)
└── maglev/
    ├── mod.rs
    ├── permutation.rs           # Eisenbud permutation generation
    └── table.rs                 # weighted multiplicity expansion
```

### `crates/overdrive-control-plane/src/reconcilers/service_map_hydrator/`

```
crates/overdrive-control-plane/src/reconcilers/service_map_hydrator/
├── mod.rs                       # `pub struct ServiceMapHydrator`,
│                                # `impl Reconciler for ...`
├── state.rs                     # ServiceMapHydratorState,
│                                # ServiceDesired, ServiceHydrationStatus,
│                                # BackendSetFingerprint
├── view.rs                      # ServiceMapHydratorView, RetryMemory
└── hydrate.rs                   # async hydrate_desired / hydrate_actual
                                 # (called by runtime, not reconciler)
```

The action-shim wrapper for `DataplaneUpdateService` lands at
`crates/overdrive-control-plane/src/action_shim/service_hydration.rs`
(new file, alongside the existing per-action shim files).

---

## §7 — BPF map shapes

| Map | Type | Key | Value | Notes |
|---|---|---|---|---|
| `SERVICE_MAP` | `BPF_MAP_TYPE_HASH_OF_MAPS` (outer) | `ServiceVip` (u32 IPv4 BE → host-order in map; see §8) + `u16` port | inner-map fd | Inner = `BPF_MAP_TYPE_HASH` keyed by `BackendId` (u32) → `BackendEntry`. Atomic swap via outer-map fd replace per Slice 03 |
| `BACKEND_MAP` | `BPF_MAP_TYPE_HASH` | `BackendId` (u32) | `BackendEntry { ipv4: u32, port: u16, weight: u16, healthy: u8, _pad: [u8; 3] }` | Single global; backends shared across services. 8-byte aligned. Max entries = 65_536 |
| `MAGLEV_MAP` | `BPF_MAP_TYPE_HASH_OF_MAPS` (outer) | `ServiceId` (u64) | inner-map fd | Inner = `BPF_MAP_TYPE_ARRAY` of `BackendId` slots, size = `MaglevTableSize` (u32, default 16_381). One inner per service. Atomic swap on backend-set change |
| `REVERSE_NAT_MAP` | `BPF_MAP_TYPE_HASH` | `ReverseKey { client_ip: u32, client_port: u16, backend_ip: u32, backend_port: u16, proto: u8, _pad: [u8; 3] }` | `OriginalDest { vip: u32, vip_port: u16, _pad: [u8; 2] }` | All values stored host-order; conversion at kernel boundary (§8). Max entries operator-tunable, default 1_048_576 |
| `DROP_COUNTER` | `BPF_MAP_TYPE_PERCPU_ARRAY` | `u32` (= `DropClass as u32`) | `u64` (count) | Slot count = `DropClass::variant_count()`. Userspace sums across CPUs at read time. Slots: `MalformedHeader=0`, `UnknownVip=1`, `NoHealthyBackend=2`, `SanityPrologue=3`, `ReverseNatMiss=4`, `OversizePacket=5` (final count locked in DESIGN per Priority Two #7) |

`MaglevTableSize` is `u32` per the open-question pin in §10 — `u16` is
insufficient (16_381 default exceeds `u16::MAX` headroom for operator
tuning to 65_537 / 131_071 prime sizes for high-fanout services).

---

## §8 — Endianness lockstep (REVERSE_NAT_MAP)

**Wire format** — IPv4 packets carry IPs and L4 ports in network byte
order (big-endian). The XDP / TC programs read them via
`*((__be32 *)&iph->saddr)` and friends; the kernel exposes these as
`__be32` / `__be16` in `<linux/in.h>`.

**Map storage format** — REVERSE_NAT_MAP keys and values are stored in
**host byte order** (little-endian on every kernel matrix entry per
`testing.md` § "Kernel matrix"; x86-64 + aarch64 are both LE). This
matches `BACKEND_MAP` storage. Userspace control-plane code reads /
writes the maps in host order without `htonl` / `ntohl` calls;
**only the kernel-side hot path performs the conversion**.

### Conversion site (locked)

A single `#[inline(always)]` helper in `crates/overdrive-bpf/src/shared/sanity.rs`:

```rust
#[inline(always)]
fn reverse_key_from_packet(
    iph: &Ipv4Hdr, l4: &L4Hdr, proto: u8,
) -> ReverseKey {
    ReverseKey {
        client_ip:    u32::from_be(unsafe { iph.saddr }),
        client_port:  u16::from_be(l4.sport()),
        backend_ip:   u32::from_be(unsafe { iph.daddr }),
        backend_port: u16::from_be(l4.dport()),
        proto,
        _pad: [0; 3],
    }
}

#[inline(always)]
fn original_dest_to_wire(d: &OriginalDest) -> (u32 /* be */, u16 /* be */) {
    (d.vip.to_be(), d.vip_port.to_be())
}
```

**Lockstep guarantee** — the BPF unit tests (Tier 2) include a roundtrip
assertion: a synthetic packet with known wire-order bytes through
`reverse_key_from_packet` produces the host-order `ReverseKey` the
userspace test seeded into the map. This closes the Eclipse-review
remediation note explicitly. A proptest in
`overdrive-dataplane::maps::reverse_nat_map_handle` round-trips
host-order writes against host-order reads to assert no userspace-side
endian flip sneaks in.

---

## §9 — Quality-attribute scenarios (extending `brief.md` §32)

| ASR | Quality attribute | Scenario | Pass criterion |
|---|---|---|---|
| ASR-2.2-01 | Reliability — zero-drop atomic swap | Source: synthetic XDP traffic at 50 kpps (CI) / 100 kpps (Lima) traversing a service VIP. Stimulus: SERVICE_MAP outer-map inner-fd swap to a new backend set during sustained traffic. Environment: native XDP on virtio-net. Response: every packet either matches old backend set XOR new — never drops on the swap boundary. Measure: zero packets dropped attributable to the swap (verified via `bpftool` counters + tcpdump on veth). | 0 swap-boundary drops over a 30-second swap-storm window (research § 3) |
| ASR-2.2-02 | Reliability — flow-affinity bound under churn | Source: synthetic 5-tuple connection set. Stimulus: backend churn — remove 1/N backends, rebuild Maglev table, atomically swap inner-map. Environment: M=16_381, N=100, M ≥ 100·N rule (research § 5.2). Response: the fraction of pre-existing 5-tuples that remap to a different backend post-churn. Measure: histogram across 1000 churn cycles. | ≤ 1% of 5-tuples remap per single-backend removal (research § 5.2) |
| ASR-2.2-03 | Maintainability — verifier-budget headroom | Source: `cargo xtask verifier-regress` on each PR. Stimulus: any change to `xdp_service_map.rs` or shared sanity prologue. Environment: Linux 6.8 (`ubuntu-latest`), aya-rs `--release`. Response: instruction-count delta vs Slice 04 baseline + absolute fraction of 1M verifier ceiling. Measure: `veristat` JSON output. | Delta ≤ 20% per PR; absolute ≤ 60% of 1M ceiling (DISCUSS Risk #6) |
| ASR-2.2-04 | Correctness — hydrator ESR closure | Source: DST harness with `SimDataplane` + `SimObservationStore`. Stimulus: arbitrary sequence of `service_backends` row mutations + injected `DataplaneError` failures + clock advances. Environment: Tier 1, every PR. Response: `assert_always!(HydratorIdempotentSteadyState)` + `assert_eventually!(HydratorEventuallyConverges)`. Measure: `cargo xtask dst --workspace`. | Both invariants hold across the seeded fault catalogue (J-PLAT-004) |

These slot under `brief.md` §32 "Quality Attribute Scenarios" as a new
sub-section §32.x "Phase 2.2 — XDP service load balancing". The exact
sub-section number is determined when DESIGN edits land.

---

## §10 — Open questions left for user

**None blocking ratification.** The following latent ambiguities surfaced
while writing §4–§9 and have been pinned with explicit recommendations
inline:

1. **`MaglevTableSize` integer width** — pinned to **`u32`**. `u16` is
   insufficient headroom; `u64` is over-engineering for a value that
   never exceeds 131_071 in any realistic operator tuning.
2. **`DropClass` slot count** — pinned to **6 slots** (see §7 row 5
   final list). DESIGN's Priority Two #7 from DISCUSS (`wave-decisions.md`
   §"Priority Two") asked for 4–6; the locked set is 6. Reducing later
   is structurally compatible (PERCPU_ARRAY index space is u32; unused
   slots stay zero); adding later requires a one-line edit + baseline
   re-bump on `DROP_COUNTER`.
3. **`actual` projection source** — pinned to a dedicated
   `service_hydration_results` ObservationStore row (§5), NOT a
   derivation of the last-emitted action. Per `development.md`
   § Persist inputs, not derived state. Adds one new table to the
   ObservationStore schema; additive-only migration per
   `whitepaper.md` § "Consistency Guardrails".

If any of these three is unsatisfactory, flag at ratification — they
are recommendations, not commitments.

---

## §11 — Deliverables ready to write on user ratification

On `Approved`, the following files land in a single DESIGN commit:

### New files

- `docs/feature/phase-2-xdp-service-map/design/architecture.md` —
  the architecture document itself, body of the proposal expanded
  with full rationale, prior-art citations from the research doc,
  and traceability matrix to US-01..US-08 + K1..K8 + ASR-2.2-01..04.
- `docs/feature/phase-2-xdp-service-map/design/wave-decisions.md` —
  DESIGN-wave decisions + handoff package for DISTILL.
- `docs/product/architecture/adr-0040-service-map-three-map-split-and-hash-of-maps.md`
  — three-map split (SERVICE_MAP, BACKEND_MAP, MAGLEV_MAP) +
  HASH_OF_MAPS atomic-swap primitive. Locks Slice 02 / Slice 03
  shape.
- `docs/product/architecture/adr-0041-weighted-maglev-and-reverse-nat-shape.md`
  — weighted Maglev permutation, M=16_381 default, M ≥ 100·N rule,
  REVERSE_NAT_MAP key/value shape, host-vs-network endianness
  contract, conversion site location. Locks Slice 04 / Slice 05.
- `docs/product/architecture/adr-0042-service-map-hydrator-reconciler.md`
  — hydrator reconciler shape, `Action::DataplaneUpdateService`
  variant rationale, `service_hydration_results` observation row
  rationale, ESR pair, retry-memory inputs. Locks Slice 08.

**ADR count: 3** (numbered ADR-0040 through ADR-0042). Three was
chosen because the three concerns are independently citable from
later ADRs and from `whitepaper.md`: Slice 03 / Slice 06 reference
ADR-0040; Slice 04 / Slice 05 reference ADR-0041; J-PLAT-004
references ADR-0042. Collapsing to one ADR would force every later
citation to point at sub-sections, which has burned us before.
Splitting further (e.g. one per slice) would dilute the
"architectural decision" threshold — the sanity prologue and Tier 4
gates are slice-implementation choices, not architectural.

### Updates to existing files

- `docs/product/architecture/brief.md` — append new sub-section
  §44.x "Phase 2.2 — XDP service load balancing" (sub-numbered from
  the current §44 Phase 2.1 section); update Status row, ADR index,
  and Changelog. Goes through the architect agent per
  `feedback_delegate_to_architect.md`.
- `docs/product/architecture/c4-diagrams.md` — add a Component (L3)
  diagram for the dataplane subsystem showing
  `EbpfDataplane` → `aya::Bpf` → `[xdp_service_map, tc_reverse_nat]`
  → `[SERVICE_MAP, BACKEND_MAP, MAGLEV_MAP, REVERSE_NAT_MAP,
  DROP_COUNTER]`, with the `ServiceMapHydrator` reconciler arrow
  into `Dataplane::update_service`. C4 Container (L2) does NOT
  change — `overdrive-bpf` and `overdrive-dataplane` are already on
  the L2 diagram from Phase 2.1 / ADR-0038.

### NOT written until ratification

- `architecture.md`, `wave-decisions.md`, ADR-0040, ADR-0041,
  ADR-0042, `brief.md` edits, `c4-diagrams.md` edits.

This proposal is the only artifact written under
`docs/feature/phase-2-xdp-service-map/design/` until the user
ratifies. Awaiting `Approved` / `ChangeRequested`.
