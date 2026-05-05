# Architecture — phase-2-xdp-service-map

**Author**: Morgan (Solution Architect)
**Date**: 2026-05-05
**Status**: COMPLETE — handoff-ready for DISTILL (acceptance-designer)
**Mode**: propose (decisions ratified by user as `lgtm`)

This document is the architectural specification for Phase 2.2 (GH #24
— *XDP routing + service load balancing*). It composes with
`docs/product/architecture/brief.md` § 40–§ 43 (Phase 2.1 substrate)
and supersedes the in-flight `proposal-draft.md` in this directory
where the two diverge — see § 16 (housekeeping). All seven
open-question decisions and three drifts surfaced during the
proposal phase are locked here.

---

## § 1 Goal & scope

Fill the empty body of `Dataplane::update_service` (and a new
companion `Dataplane::reverse_nat_lookup`-class concern routed
through the same trait) with a Cilium-style three-map split + Maglev
consistent hashing + REVERSE_NAT path, attach the XDP program at the
NIC driver level on a real iface (veth in tests, virtio-net in
production), drive the kernel-side state from a new
`ServiceMapHydrator` reconciler that observes `service_backends`
ObservationStore rows and emits a typed `Action::DataplaneUpdateService`
on every backend-set change.

**Whitepaper anchors:**

- § 7 *eBPF Dataplane / XDP — Fast Path Packet Processing* — the
  empty body's structural commitment.
- § 15 *Zero Downtime Deployments* — atomic `SERVICE_MAP` swap is
  the WHY: rolling deployments, canary, and blue/green collapse to
  one BPF map update.
- § 19 *Security Model — Defense in depth (Layer 4: XDP network
  policy)* — drop unknown VIPs at line rate.
- § 18 *Reconciler primitive* + § 4 *Intent / Observation split* —
  hydrator reconciler closes the §18 ESR loop against the
  ObservationStore-side service-backends rows.

**Job anchor:** `J-PLAT-004` (reconciler convergence) is activated by
this feature — it flips `deferred → active` in `docs/product/jobs.yaml`,
because Slice 08's hydrator is the first non-trivial reconciler
emitting a typed Action against a real (non-Sim) Dataplane port body.

---

## § 2 Constraints (inherited from DISCUSS, restated for traceability)

These are not debatable in DESIGN; they propagate verbatim into
DISTILL and DELIVER.

1. **Single-kernel in-host** — Tier 3 / Tier 4 run on developer Lima
   VM (`cargo xtask lima run --`) and CI `ubuntu-latest`. The LVH
   matrix from `cargo xtask integration-test vm` stays in place but
   is not exercised; activates when GH #152 lands.
2. **Conntrack is OUT** — stateless Maglev forwarder, ≤ 1% disruption
   per single-backend removal is the flow-affinity bound; conntrack
   is GH #154.
3. **`#![no_std]`, `aya-ebpf`-only kernel side** — kernel programs
   live in `overdrive-bpf` (class `binary`, target
   `bpfel-unknown-none`); userspace lives in `overdrive-dataplane`
   (class `adapter-host`); no `aya` import outside `overdrive-dataplane`.
4. **`Dataplane` port trait is the only consumer-facing surface** —
   reconcilers and the action shim see `Arc<dyn Dataplane>`;
   production wires `EbpfDataplane`, DST wires `SimDataplane`.
5. **Hydrator reconciler purity is non-negotiable** — sync
   `reconcile`, no `.await`, no wall-clock reads; View persistence
   via the runtime-owned redb `ViewStore` (ADR-0035). All I/O is
   typed `Action` values consumed by the action shim (ADR-0023).
6. **Determinism in hydrator-side userspace logic is load-bearing**
   — `BTreeMap` iteration, not `HashMap`, drives Maglev table
   permutation (`development.md` § Ordered-collection choice).
7. **STRICT newtypes** — `ServiceVip`, `ServiceId`, `BackendId`,
   `MaglevTableSize`, `DropClass` ship in `overdrive-core` with full
   FromStr / Display / serde / rkyv / proptest discipline.
8. **Real-infrastructure tests gated `integration-tests`** — default
   lane uses `SimDataplane`; Tier 2 / 3 / 4 test surface stays
   feature-gated.
9. **Native XDP only; warn on generic fallback** — Lima virtio-net,
   `ubuntu-latest` virtio-net, mlx5, ena all support native; a
   native-attach failure logs structured warning.
10. **No new fields on existing aggregates** — `Job` and `Node`
    unchanged; service hydration reads existing `service_backends`
    rows; no schema migration on existing tables.

---

## § 3 Architectural posture (inherited)

- **Style**: Hexagonal (ports & adapters), single-process Rust
  workspace. Inherited from `brief.md` § 1.
- **Paradigm**: OOP (Rust trait-based). Inherited from `brief.md` § 2.
- **Substrate**: ADR-0038 — `overdrive-bpf` (kernel) +
  `overdrive-dataplane` (loader); no new crates. The hydrator lives
  inside the existing `overdrive-control-plane::reconcilers/*`
  module set.

---

## § 4 Reuse Analysis (HARD GATE)

The following 20 components / surfaces are catalogued. Existing
surfaces are EXTENDED; only five are CREATE NEW, each with
documented "no existing alternative" justification.

| # | Component / surface | Disposition | Rationale |
|---|---|---|---|
| 1 | `Dataplane` trait (`overdrive-core::traits::dataplane`) | **EXTEND** | Add three method args to `update_service(service_id, vip, backends)`; no new trait. |
| 2 | `EbpfDataplane` (`overdrive-dataplane::ebpf_dataplane`) | **EXTEND** | Phase 2.1 stub bodies become real implementations; struct shape unchanged. |
| 3 | `SimDataplane` (`overdrive-sim::adapters::dataplane`) | **EXTEND** | Mirror the new method signature; in-memory `BTreeMap` book-keeping. |
| 4 | `Reconciler` trait (`overdrive-core::reconciler`) | **EXTEND** (new impl, no trait change) | New `ServiceMapHydrator` trait impl; ADR-0035 trait shape unchanged. |
| 5 | `AnyReconciler` enum (`overdrive-core::reconciler`) | **EXTEND** | Add `ServiceMapHydrator` variant; runtime hydration `match` arm extended. |
| 6 | `AnyState` enum (`overdrive-core::reconciler`) | **EXTEND** | Add `ServiceMapHydrator(ServiceMapHydratorState)` variant per ADR-0021/0036. |
| 7 | `Action` enum (`overdrive-core::reconciler`) | **EXTEND** | Add one new variant `Action::DataplaneUpdateService`. |
| 8 | `ReconcilerName` (`overdrive-core::reconciler::name`) | **EXTEND** | Add `service-map-hydrator` const name; no type change. |
| 9 | `EvaluationBroker` (`overdrive-control-plane::reconciler_runtime::broker`) | **REUSE** | Storm-proof keying on `(name, target)` works as-is. |
| 10 | `action_shim::dispatch` match (`overdrive-control-plane::reconciler_runtime::action_shim`) | **EXTEND** | Add `DataplaneUpdateService` arm; existing match exhaustiveness gates it. |
| 11 | Service-backends ObservationStore row shape | **REUSE** | Already declared in `traits/observation_store.rs`; no schema change. |
| 12 | `service_hydration_results` ObservationStore table | **CREATE NEW** (additive-only migration) | No existing observation row carries hydration outcome — required for `actual` projection to observe what *is*, not what was *predicted* (Drift 2; see § 12). |
| 13 | `RedbViewStore` (`overdrive-control-plane::view_store`) | **REUSE** | ADR-0035 already provides `bulk_load` / `write_through` for any typed `View`. |
| 14 | `TickContext` (`overdrive-core::reconciler::tick`) | **REUSE** | Wall-clock injection works as-is; reconciler reads `tick.now_unix`. |
| 15 | `CorrelationKey` (`overdrive-core::correlation`) | **REUSE** | The `(reconciler, target, fingerprint)` shape exists; no extension. |
| 16 | `ServiceVip` newtype | **CREATE NEW** | No existing IPv4/IPv6-VIP newtype in `overdrive-core::id`; required for typed `(VIP, port) → ServiceId` SERVICE_MAP key. No alternative. |
| 17 | `ServiceId` newtype | **CREATE NEW** | No existing service-identity newtype; required for typed Action variant + per-target keying. No alternative. |
| 18 | `BackendId` / `MaglevTableSize` / `DropClass` newtypes | **CREATE NEW** | No existing backend-identity, table-size, or drop-class type; required for STRICT-newtype discipline (Constraint 7). |
| 19 | `aya::Bpf` loader (Phase 2.1 substrate) | **REUSE** | `overdrive-dataplane::loader` already loads ELF; new programs attach through the same path. |
| 20 | `xtask bpf-build / bpf-unit / integration-test vm` | **REUSE** (Slice 07 fills `verifier-regress` + `xdp-perf` stubs from #23) | No new xtask subcommand; the two stubbed subcommands fill in against this feature's first real program. |

**Summary**: 15 EXTEND/REUSE, 5 CREATE NEW (1 observation table + 4
newtypes, plus the unavoidable `ServiceVip`); 0 unjustified CREATE
NEW.

---

## § 5 The seven open-question decisions (locked)

Each decision restates the options surfaced during DISCUSS / proposal
review and the locked recommendation with rationale. References to
research § N point at
`docs/research/networking/xdp-service-load-balancing-research.md`.

### Q-Sig — Trait method signature

**Options:**

- **A** *(locked)* — `update_service(service_id: ServiceId, vip: ServiceVip, backends: Vec<Backend>)`. Three explicit args.
- B — `update_service(record: ServiceRecord)`. One aggregate.
- C — keep Phase 2.1's `update_service(vip, backends)` and key BPF
  maps by VIP only (no `ServiceId`).

**Locked: A.** Three explicit args at the trait surface keeps
`SimDataplane`'s in-memory book-keeping trivial (no aggregate
unpacking) and lets the kernel-side three-map split (Drift 3) read
its key tuple straight from the function arguments without an
intermediate struct unpack. Option B forces an aggregate that
duplicates fields the action shim already passes through
typed-decomposed. Option C breaks the three-map split — `ServiceId`
is the natural inner-key for both `MAGLEV_MAP` and the
`SERVICE_MAP → inner_map_fd` indirection (research § 2.2; § 6.2).

**Drift 3 correction.** During proposal review, Q-Sig framed
"`ServiceId` keys all three maps." That conflated trait surface with
kernel-map shape. Corrected: SERVICE_MAP outer key = `(ServiceVip,
u16 port)` (the kernel sees wire packets and *must* look up by
`(VIP, port)`); MAGLEV_MAP outer key = `ServiceId`; BACKEND_MAP key
= `BackendId`. The three keys are typed-distinct and traced
end-to-end through the trait → shim → loader → BPF maps boundary.

### Q1 — Checksum helper

**Options:**

- **A** *(locked)* — `bpf_l3_csum_replace` + `bpf_l4_csum_replace`
  (kernel helpers).
- B — `csum_diff` family (aya helpers).

**Locked: A.** The kernel-helper path is verifier-clean across the
entire kernel matrix (research § 4.1, § 4.2); the aya `csum_diff`
helpers are a thin wrapper that exposes additional verifier
constraints unnecessarily. The kernel-helper choice also lets the
DROP_COUNTER per-CPU array stay outside the checksum-recompute path,
keeping Tier 4 verifier-budget delta below the 20 % gate (ASR-2.2-03).

### Q2 — Reverse-NAT egress hook

**Options:**

- **A** *(locked)* — TC egress; program `tc_reverse_nat`.
- B — XDP egress (kernel ≥ 5.18).

**Locked: A.** XDP-egress requires kernel 5.18+; Phase 2.2's stated
floor is the 5.10 LTS lineage (per `.claude/rules/testing.md` § Tier
3 — kernel matrix), and even single-kernel in-host on `ubuntu-latest`
runs 6.x where TC has been production-stable for years. The Cilium /
Katran reference path uses TC egress for the same reason; aya 0.13's
TC support is mature (research § 4.3).

### Q3 — Sanity-prologue strategy

**Options:**

- A — duplicate inline at the top of every XDP program.
- B — `bpf_tail_call` shared helper.
- **C** *(locked)* — Shared `#[inline(always)]` Rust helper in
  `overdrive-bpf::shared::sanity`.

**Locked: C.** Verifier-budget-equivalent to A (the call gets
inlined; no tail-call indirection); structurally one source of truth
across `xdp_service_map` and (future) `xdp_*` programs. Option B
costs verifier-budget-equivalent reasoning *plus* indirection on
every packet. Option A duplicates source which then drifts
asymmetrically across programs (research § 8.2 documents this
exact failure shape in Cilium). The `#[inline(always)]` Rust
helper is the canonical aya-rs pattern (research § 8.2 final
recommendation).

### Q4 — `cargo xtask perf-baseline-update` helper

**Options:**

- A — Ship now alongside Slice 07's perf gates.
- **B** *(locked)* — Skip for now.

**Locked: B.** Ship Slice 07 with manual `git mv` flow for
baseline updates; the helper's surface area (4–5 args, file path
canonicalisation, baseline-rotation atomicity) is bigger than the
first three baseline-update commits will exercise. Re-evaluate
after #29 / #152 lands and the kernel matrix actually demands
frequent re-baselining.

### Q5 — HASH_OF_MAPS inner-map size

**Options:**

- **A** *(locked)* — Fixed 256, compiled in.
- B — Operator-tunable per service.

**Locked: A.** 256 is well above any realistic per-service backend
count for Phase 2 (research § 3.3); compile-time-fixed inner-map
size keeps the BPF map declaration syntax simple
(`#[map(name = "...", max_entries = 256)]`) and verifier-friendly.
Operator-tunability composes via `MAGLEV_MAP`'s own
`MaglevTableSize` (Q6) for the algorithmic shape; the inner
HASH_OF_MAPS size is a structural constant.

### Q6 — Maglev `M` operator-tunability

**Options:**

- **A** *(locked)* — Fixed default M=16_381; newtype shipped, no
  operator surface yet.
- B — Per-service M overrides via `JobSpec`.

**Locked: A.** `MaglevTableSize` is a STRICT newtype with full
FromStr / Display / serde / rkyv / proptest discipline (Constraint
7), so the operator-config surface lands cheaply when an
operator-config aggregate appears (Phase 3+). For Phase 2.2 the
fixed 16_381 default satisfies M ≥ 100·N for any realistic
backend count (research § 5.2). Shipping the newtype now means the
operator-tunability slice (a future Phase 2/3 ticket) is a one-line
JobSpec edit, not a type-system change.

### Q7 — `DropClass` slot count

**Options:**

- A — 4 slots.
- **B** *(locked)* — 6 slots.
- C — 8 slots, future-proof.

**Locked: B (6 slots).** Locked variant set:
`MalformedHeader=0, UnknownVip=1, NoHealthyBackend=2, SanityPrologue=3,
ReverseNatMiss=4, OversizePacket=5`. Six covers every drop the XDP
+ TC programs in Phase 2.2 actually emit; 8 would carry two unused
slots that future drift could populate inconsistently. Adding
later is structurally compatible (PERCPU_ARRAY index space is
`u32`; new slots stay zero on every CPU until the next BPF
re-load) and a one-line edit on the `DropClass` enum (per § 6 the
`#[repr(u32)]` enum maps to PERCPU_ARRAY index).

---

## § 6 New newtypes (and one type alias)

Five newtypes ship in `crates/overdrive-core/src/`, with FromStr
/ Display / serde / rkyv / proptest discipline per
`development.md` § Newtype completeness. One *type alias* —
`BackendSetFingerprint` — ships alongside them, scope-justified
below.

| Newtype | File | Backing | Purpose |
|---|---|---|---|
| `ServiceVip` | `overdrive-core/src/id.rs` (extend) | `IpAddr` (v4 today; v6 future per GH #155) | Virtual IP a kernel-side XDP program matches incoming packets against. Stored host-order; converted at kernel boundary (§ 11). Userspace control-plane newtype only — `service_backends` rows continue to carry `vip: Ipv4Addr` as their wire-shape field; the hydrator wraps at the boundary (§ 5). |
| `ServiceId` | `overdrive-core/src/id.rs` (extend) | `u64` (content-hashed from `(VIP, port, scope)` tuple) | Identity of a service for control-plane addressing. MAGLEV_MAP outer key. |
| `BackendId` | `overdrive-core/src/id.rs` (extend) | `u32` (monotonic) | BACKEND_MAP key. Backends are shared across services; one global map. |
| `MaglevTableSize` | `overdrive-core/src/dataplane/maglev_table_size.rs` (NEW module) | `u32` | Validating constructor enforces membership in Cilium's prime list + default M=16_381. Q6=A: newtype shipped, operator surface deferred. |
| `DropClass` | `overdrive-core/src/dataplane/drop_class.rs` (NEW module) | `#[repr(u32)]` enum, 6 variants | PERCPU_ARRAY index for DROP_COUNTER. Q7=B locked variant set. |

`ServiceVip` and `ServiceId` extend the existing `id.rs` (which
hosts the 11 Phase 1 newtypes); `MaglevTableSize` and `DropClass`
get their own module under a new `dataplane/` sibling because they
are *dataplane-internal* concerns rather than first-class workload
identifiers — the natural-decomposition shape that mirrors
`overdrive-core::traits::dataplane`. Each newtype carries a
proptest harness in `crates/overdrive-core/tests/<newtype>.rs`
following the Phase 1 precedent.

### Type aliases

```rust
/// Content-hash of a `(ServiceVip, &[Backend])` pair. Identifies
/// a unique backend-set state for convergence detection in the
/// hydrator reconciler (§ 8) and for LWW resolution in
/// `service_hydration_results` (§ 12).
///
/// Type alias rather than STRICT newtype because:
///   - the value is derived (a hash), never operator-typed;
///   - it has no canonical string form (no `Display` / `FromStr`
///     surface);
///   - the existing `correlation: u64`-shaped pattern on
///     `CorrelationKey::derive` (`crates/overdrive-core/src/id.rs`)
///     is the project's precedent for content-derived numeric
///     identifiers that travel through the type system without
///     needing newtype machinery.
///
/// The hashing-determinism rule
/// (`development.md` § Hashing requires deterministic
/// serialization) governs *how* the value is computed — rkyv-
/// archived bytes, blake3 keyed hash, truncated to u64 — not the
/// type's wire shape. The fingerprint module owns the only
/// constructor; nobody else fabricates fingerprints.
pub type BackendSetFingerprint = u64;
```

The fingerprint computation lives in
`crates/overdrive-core/src/dataplane/fingerprint.rs` (NEW module):

```rust
use crate::traits::dataplane::Backend;
use crate::dataplane::ServiceVip;

/// Compute the canonical content-hash of a backend set keyed by
/// VIP. The result is bit-identical across nodes given identical
/// inputs (the rkyv archive is canonical by construction; see
/// `development.md` § Hashing requires deterministic serialization).
///
/// Truncates blake3's 256-bit digest to the first 8 bytes
/// (little-endian) — the cluster-lifetime collision probability at
/// O(1k) services × O(1k) churn-per-service is negligible.
pub fn fingerprint(
    vip: &ServiceVip,
    backends: &[Backend],
) -> BackendSetFingerprint {
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&(vip, backends))
        .expect("rkyv archive of (ServiceVip, &[Backend]) is infallible");
    let h = blake3::hash(&bytes);
    let prefix: [u8; 8] = h.as_bytes()[..8]
        .try_into()
        .expect("blake3 digest has at least 8 bytes");
    u64::from_le_bytes(prefix)
}
```

`blake3` is already in the workspace dep graph (`Cargo.toml`
line 74). `rkyv` is the canonical project choice for content-
addressed hashing (`development.md` § Hashing requires
deterministic serialization).

### `DropClass` (Q7=B locked at 6 slots)

```rust
/// Drop classification for the `DROP_COUNTER` PERCPU_ARRAY.
/// `#[repr(u32)]` makes `as u32` a stable kernel-side index
/// across Rust toolchains (research § 7.1 — the verified pattern
/// Cilium and Katran use).
///
/// Variant ordering and discriminants are STABLE — additions are
/// minor-version (per ADR-0037 K8s-Condition convention);
/// reordering or removal is a major-version break that requires
/// a new ADR.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum DropClass {
    MalformedHeader   = 0,
    UnknownVip        = 1,
    NoHealthyBackend  = 2,
    SanityPrologue    = 3,
    ReverseNatMiss    = 4,
    OversizePacket    = 5,
}
```

Plus the project STRICT-newtype surface — `FromStr` parses
kebab-case (`malformed-header` → `MalformedHeader`); `Display`
emits kebab-case; serde uses the kebab-case form via `#[serde(
rename_all = "kebab-case")]` on the enum body; the proptest
harness in `crates/overdrive-core/tests/drop_class.rs` exhausts
all six variants and asserts `Display`/`FromStr` round-trip
bit-equivalent.

### `MaglevTableSize` (Q6=A locked default 16_381, prime-list-validated)

```rust
/// Maglev permutation table size. Constrained to Cilium's prime
/// list: { 251, 509, 1_021, 2_039, 4_093, 8_191, 16_381, 32_749,
/// 65_521, 131_071 }. Default 16_381 supports up to ~160 backends
/// per the M ≥ 100·N rule (research § 5.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct MaglevTableSize(u32);

const ALLOWED_PRIMES: [u32; 10] = [
    251, 509, 1_021, 2_039, 4_093, 8_191, 16_381, 32_749, 65_521, 131_071,
];

impl MaglevTableSize {
    /// Default Maglev `M`. Smallest prime ≥ 16_384; matches Cilium.
    pub const DEFAULT: Self = Self(16_381);

    /// Validating constructor — rejects every value not in the
    /// prime list. The `M ≥ 100 · N` rule is enforced at backend-
    /// set-update time (separate concern; not at construction).
    pub fn new(value: u32) -> Result<Self, ParseError> {
        ALLOWED_PRIMES
            .binary_search(&value)
            .map(|_| Self(value))
            .map_err(|_| ParseError::NotInPrimeList { value })
    }

    pub fn get(self) -> u32 { self.0 }
}

impl Default for MaglevTableSize {
    fn default() -> Self { Self::DEFAULT }
}

impl Display for MaglevTableSize {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for MaglevTableSize {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Self, ParseError> {
        s.parse::<u32>()
            .map_err(|e| ParseError::Malformed(e.to_string()))
            .and_then(Self::new)
    }
}

impl TryFrom<u32> for MaglevTableSize {
    type Error = ParseError;
    fn try_from(v: u32) -> Result<Self, ParseError> { Self::new(v) }
}

impl From<MaglevTableSize> for u32 {
    fn from(v: MaglevTableSize) -> Self { v.get() }
}

// proptest in `crates/overdrive-core/tests/maglev_table_size.rs`:
//   - roundtrip Display ↔ FromStr for every prime in ALLOWED_PRIMES;
//   - reject every non-prime u32 (exhaustive over a sampled range);
//   - serde Deserialize validates via TryFrom<u32> (the
//     `try_from = "u32"` attribute is the load-bearing surface).
```

The `try_from = "u32"` attribute on `#[serde(...)]` makes
`Deserialize` validate per `development.md` § Newtype
completeness — a wire payload carrying a non-prime is rejected at
the deserialization boundary, not silently accepted.

---

## § 7 `Action::DataplaneUpdateService` — locked variant body

The hydrator reconciler emits exactly one Action variant. The
variant lands in `crates/overdrive-core/src/reconciler.rs`,
appended to the existing `pub enum Action` block.

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
/// observation row (§ 12). The next reconcile tick reads
/// that row via `actual` and either advances (Completed) or
/// retries on the next backend-set change (Failed).
///
/// `Vec<Backend>` carries weighted backends in deterministic
/// `BTreeMap<BackendId, Backend>::iter()` order — Maglev table
/// generation is byte-deterministic across nodes given identical
/// inputs (DISCUSS Decision 8 + Constraint 6).
DataplaneUpdateService {
    /// Identity of the service whose backend set is being rewritten.
    /// Maps 1:1 to a `MAGLEV_MAP` outer-map key.
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
    /// pattern. Derived deterministically from
    /// `(target = "service-map-hydrator/<service_id>",
    ///   spec_hash = ContentHash::of(rkyv-archive of fingerprint),
    ///   purpose = "update-service")` so the next tick can locate
    /// the `service_hydration_results` row deterministically.
    /// Required, not optional — service hydration is
    /// correlation-keyed end-to-end.
    correlation: CorrelationKey,
},
```

The hydrator constructs the `correlation` field via the existing
`CorrelationKey::derive(target: &str, spec_hash: &ContentHash,
purpose: &str)` constructor in
`crates/overdrive-core/src/id.rs`:

```rust
let target = format!("service-map-hydrator/{service_id}");
let spec_hash = ContentHash::of(
    &fingerprint.to_le_bytes()[..],   // BackendSetFingerprint = u64
);
let correlation = CorrelationKey::derive(
    &target,
    &spec_hash,
    "update-service",
);
```

This matches the project's existing `CorrelationKey::derive`
precedent — three string-and-content-hash inputs, no new
constructor surface. The hydrator never fabricates raw correlation
strings; it goes through `derive` exactly as the `HttpCall` pattern
in §18 of the whitepaper requires.

`Backend` is the existing `overdrive-core` aggregate (already used
by `service_backends` observation rows); no new field on it.

### Failure surface — observation, NOT `TerminalCondition`

The action shim wraps `Dataplane::update_service(...)` and:

- **On `Ok(())`** — writes `service_hydration_results` row with
  `status: Completed { fingerprint, applied_at: tick.now }`.
- **On `Err(DataplaneError::*)`** — writes `service_hydration_results`
  row with `status: Failed { reason: Display::to_string(&err),
  failed_at: tick.now }`.

The shim's error type is a new `ServiceHydrationDispatchError` enum
in `crates/overdrive-control-plane/src/action_shim/service_hydration.rs`
with `#[from]` pass-through for `DataplaneError`
(`development.md` § Errors / pass-through embedding). It does NOT
carry `terminal: Option<TerminalCondition>` per ADR-0037 — service
hydration cannot terminate an allocation; mixing the channels
would erode ADR-0037's "every terminal claim has a single typed
source" invariant. Retry-budget logic lives in the View (§ 8).

---

## § 8 `ServiceMapHydrator` reconciler

### Identity

```rust
pub const NAME: &str = "service-map-hydrator";
fn name(&self) -> &ReconcilerName { &self.name } // ReconcilerName::new_const(NAME)
```

### Per-target keying

Target = `ServiceId`. Evaluation broker keys evaluations on
`(ReconcilerName, ServiceId)` per ADR-0023's storm-proof ingress —
a row-change burst on N backends of one service collapses to ONE
pending evaluation, not N.

### `type State = ServiceMapHydratorState`

```rust
pub struct ServiceMapHydratorState {
    /// Per-service desired backend set, hydrated from
    /// `service_backends` observation rows for the target
    /// `ServiceId`. Keyed `BTreeMap` (NOT `HashMap`) per
    /// development.md § Ordered-collection choice — deterministic
    /// iteration is what makes Maglev permutation byte-identical.
    pub desired: BTreeMap<ServiceId, ServiceDesired>,
    /// Per-service last-known hydration outcome from the
    /// `service_hydration_results` table (Drift 2) — `actual`
    /// observes the dataplane's confirmed state, not the next-
    /// action prediction.
    pub actual: BTreeMap<ServiceId, ServiceHydrationStatus>,
}

// NOTE on `vip` typing across the boundary:
//
//   - The `service_backends` ObservationStore row continues to
//     carry `vip: Ipv4Addr` (its existing wire-shape field) —
//     Constraint 10 ("no new fields on existing aggregates") is
//     satisfied; no schema migration is implied.
//   - `ServiceVip` is a *userspace control-plane* newtype, not an
//     observation-store schema column type.
//   - The hydrator's async `hydrate_desired` (§ 8 Hydration shape;
//     ADR-0036 placement) wraps the wire-shape `Ipv4Addr` into
//     `ServiceVip` at the read boundary.
//   - When a future migration introduces v6 (GH #155), the wrap
//     site is the single point that needs to learn the new shape;
//     observation rows continue to carry the address as-is.

pub struct ServiceDesired {
    pub vip: ServiceVip,
    pub backends: Vec<Backend>,           // BTreeMap-sorted
    pub fingerprint: BackendSetFingerprint, // u64, content-hash
}

pub enum ServiceHydrationStatus {
    Pending,
    Completed { fingerprint: BackendSetFingerprint, applied_at: UnixInstant },
    Failed    { fingerprint: BackendSetFingerprint, failed_at:  UnixInstant,
                reason: String },
}
```

Drift 2 rationale: deriving `actual` from "the last action I
emitted" produces a write-only loop that cannot detect a
silently-failed dataplane update — exactly the failure mode
J-PLAT-004 is meant to close. `service_hydration_results` is the
typed observation row the shim writes after the dataplane call
returns; the next reconcile tick reads it. Retries are driven by
fingerprint mismatch, not by re-emitting on every tick.

### `type View = ServiceMapHydratorView` — persists inputs (not deadlines)

Per `development.md` § Persist inputs, not derived state:

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
    pub last_attempted_fingerprint: Option<BackendSetFingerprint>,
}
```

The next-attempt deadline is **recomputed every tick** as
`last_failure_seen_at + backoff_for_attempt(attempts)`. Never
persisted. `BTreeMap` per § Ordered-collection choice.

### `reconcile` skeleton

(Sync, pure, no `.await`, no wall-clock read — full code in
proposal-draft.md § 5; key invariants:)

- At most one `DataplaneUpdateService` per tick per `ServiceId`
  (structural — `desired.desired` is `BTreeMap`-keyed on
  `ServiceId`, the loop emits once per key).
- View row reset on confirmed convergence
  (`actual.fingerprint == desired.fingerprint`).
- View row GC for services no longer in `desired`.

### Hydration shape (runtime-owned, NOT in `reconcile`)

| Projection | Source | Hydrator surface |
|---|---|---|
| `desired.desired` | `service_backends` ObservationStore rows for the target `ServiceId` | Free-function arm in the runtime's `hydrate_desired` per ADR-0036 |
| `actual.actual`   | `service_hydration_results` observation row (NEW per § 12) | Free-function arm in the runtime's `hydrate_actual` per ADR-0036 |
| `view.retries`    | `RedbViewStore::bulk_load` at register; `write_through` after each tick | Runtime-owned per ADR-0035 |

The runtime's existing `hydrate_desired` / `hydrate_actual` free
functions in
`crates/overdrive-control-plane/src/reconciler_runtime.rs` (around
line 769 / line 825) gain new match arms for the
`AnyReconciler::ServiceMapHydrator(_)` variant. The arms project
into the existing `AnyState` enum (extended per Reuse Analysis
row 6 with a `ServiceMapHydrator(ServiceMapHydratorState)`
variant). Concrete arm signatures follow the established pattern
exactly:

```rust
// Inside the existing free fn `hydrate_desired` in
// crates/overdrive-control-plane/src/reconciler_runtime.rs.
// New match arm — same shape as the JobLifecycle arm.
async fn hydrate_desired(
    reconciler: &AnyReconciler,
    target: &TargetResource,
    state: &AppState,
) -> Result<AnyState, ConvergenceError> {
    match reconciler {
        // ... existing arms (NoopHeartbeat, JobLifecycle) ...
        AnyReconciler::ServiceMapHydrator(_) => {
            let service_id = service_id_from_target(target)?;
            let rows = state
                .obs
                .service_backends_rows(&service_id)
                .await
                .map_err(|e| ConvergenceError::ObservationRead(e.to_string()))?;
            let mut desired = BTreeMap::<ServiceId, ServiceDesired>::new();
            // Wrap the row's existing `vip: Ipv4Addr` into
            // `ServiceVip` here at the read boundary; the row
            // shape itself does not change.
            // ... assemble ServiceDesired { vip, backends, fingerprint } ...
            // ... compute fingerprint via `dataplane::fingerprint(...)` ...
            Ok(AnyState::ServiceMapHydrator(ServiceMapHydratorState {
                desired,
                actual: BTreeMap::new(),  // populated by hydrate_actual
            }))
        }
    }
}

// Inside the existing free fn `hydrate_actual` in the same file.
async fn hydrate_actual(
    reconciler: &AnyReconciler,
    target: &TargetResource,
    state: &AppState,
) -> Result<AnyState, ConvergenceError> {
    match reconciler {
        // ... existing arms ...
        AnyReconciler::ServiceMapHydrator(_) => {
            let service_id = service_id_from_target(target)?;
            let rows = state
                .obs
                .service_hydration_results_rows(&service_id)
                .await
                .map_err(|e| ConvergenceError::ObservationRead(e.to_string()))?;
            let mut actual = BTreeMap::<ServiceId, ServiceHydrationStatus>::new();
            // ... project rows into ServiceHydrationStatus ...
            Ok(AnyState::ServiceMapHydrator(ServiceMapHydratorState {
                desired: BTreeMap::new(),  // populated by hydrate_desired
                actual,
            }))
        }
    }
}
```

Three points to note about the existing surface:

- The runtime's `hydrate_desired` / `hydrate_actual` are **free
  functions in the runtime module**, NOT methods on
  `AnyReconciler` — `AnyReconciler` is the dispatch enum, the
  match-arm body lives in the runtime free function. ADR-0036's
  "runtime owns hydration" placement is preserved.
- Both arms read **only the `ObservationStore`** (`state.obs.*`).
  The `service-map-hydrator` is purely an observation-driven
  reconciler — `service_backends` is observation per ADR-0023,
  `service_hydration_results` is observation per § 12. Neither
  arm touches the IntentStore. The `state.store` (IntentStore)
  field in `AppState` is unused for this reconciler kind, present
  on the receiver only because the existing function signature
  carries it for the JobLifecycle arm.
- Each arm produces a *partial* `ServiceMapHydratorState` — the
  `desired` arm fills `desired` and leaves `actual` empty;
  `hydrate_actual` does the inverse. The runtime merges the two
  partials into the single `State` value passed to `reconcile`,
  matching the JobLifecycle precedent's `desired.allocations` /
  `actual.allocations` projection split (`reconciler_runtime.rs`
  ~line 788 / ~line 847).

The reconciler author writes `reconcile` only.

### ESR pair (locked names from DISCUSS)

| DST invariant | Property |
|---|---|
| `HydratorEventuallyConverges` | For every `service_id`, `actual.fingerprint == desired.fingerprint` is reached within a bounded number of ticks given a stable `desired`. |
| `HydratorIdempotentSteadyState` | Once `actual.fingerprint == desired.fingerprint` for all services, the hydrator emits zero `DataplaneUpdateService` actions per tick. |

Both live in `crates/overdrive-sim/src/invariants/` and run on every
PR per `.claude/rules/testing.md` § Tier 1.

---

## § 9 Module layout

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
                                 # + endianness conversion site (§ 11)
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

### `crates/overdrive-control-plane/src/action_shim/service_hydration.rs`

The shim wrapper for `DataplaneUpdateService` lands as a NEW file
alongside the existing per-action shim files. Hosts
`ServiceHydrationDispatchError` enum + `dispatch` function that
calls `Dataplane::update_service` and writes the outcome row.

---

## § 10 BPF map shapes

| Map | Type | Key | Value | Notes |
|---|---|---|---|---|
| `SERVICE_MAP` | `BPF_MAP_TYPE_HASH_OF_MAPS` (outer) | `(ServiceVip, u16 port)` (host-order in map; converted at kernel boundary § 11) | inner-map fd | **Drift 3 locked outer key.** Inner = `BPF_MAP_TYPE_HASH` keyed by `BackendId` → `BackendEntry`. Atomic swap via outer-map fd replace per Slice 03. Inner `max_entries = 256` per Q5=A. |
| `BACKEND_MAP` | `BPF_MAP_TYPE_HASH` | `BackendId` (u32) | `BackendEntry { ipv4: u32, port: u16, weight: u16, healthy: u8, _pad: [u8; 3] }` | Single global; backends shared across services. 8-byte aligned. `max_entries = 65_536`. |
| `MAGLEV_MAP` | `BPF_MAP_TYPE_HASH_OF_MAPS` (outer) | `ServiceId` (u64) | inner-map fd | Inner = `BPF_MAP_TYPE_ARRAY` of `BackendId` slots, size = `MaglevTableSize` (u32, default 16_381). One inner per service. Atomic swap on backend-set change. |
| `REVERSE_NAT_MAP` | `BPF_MAP_TYPE_HASH` | `ReverseKey { client_ip: u32, client_port: u16, backend_ip: u32, backend_port: u16, proto: u8, _pad: [u8; 3] }` | `OriginalDest { vip: u32, vip_port: u16, _pad: [u8; 2] }` | All values stored host-order; conversion at kernel boundary (§ 11). `max_entries = 1_048_576` (operator-tunable in future; Phase 2.2 fixed). |
| `DROP_COUNTER` | `BPF_MAP_TYPE_PERCPU_ARRAY` | `u32` (= `DropClass as u32`) | `u64` (count) | Slot count = `DropClass::variant_count()` = 6. Userspace sums across CPUs at read time. Slots locked per Q7=B. |

`MaglevTableSize` is `u32` because `u16` lacks headroom for
operator-tuning to 65_537 / 131_071 prime sizes for high-fanout
services.

---

## § 11 Endianness lockstep (REVERSE_NAT_MAP)

**Wire format** — IPv4 packets carry IPs and L4 ports in network
byte order (big-endian). XDP / TC programs read them via
`*((__be32 *)&iph->saddr)` and friends; the kernel exposes these
as `__be32` / `__be16`.

**Map storage format** — REVERSE_NAT_MAP keys / values are stored in
**host byte order** (little-endian on every kernel matrix entry per
`testing.md` § Kernel matrix; x86-64 + aarch64 are both LE). This
matches `BACKEND_MAP` storage. Userspace control-plane code reads /
writes the maps in host order without `htonl` / `ntohl` calls;
**only the kernel-side hot path performs the conversion**.

### Conversion site (locked)

A single `#[inline(always)]` helper in
`crates/overdrive-bpf/src/shared/sanity.rs`:

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

**Lockstep guarantee** — Tier 2 BPF unit tests include a roundtrip
assertion: a synthetic packet with known wire-order bytes through
`reverse_key_from_packet` produces the host-order `ReverseKey` the
userspace test seeded into the map. Closes the Eclipse-review
remediation note explicitly. A proptest in
`overdrive-dataplane::maps::reverse_nat_map_handle` round-trips
host-order writes against host-order reads to assert no
userspace-side endian flip sneaks in.

---

## § 12 New ObservationStore table — `service_hydration_results`

Drift 2 surfaced this table during proposal review. It is the only
schema addition this feature ships.

### Schema

| Column | Type | Notes |
|---|---|---|
| `service_id` | `ServiceId` (u64) | PK |
| `fingerprint` | `BackendSetFingerprint` (u64) | Content-hash of `(vip, backends)` per `development.md` § Hashing requires deterministic serialization (rkyv-archived). |
| `status` | tagged enum: `Pending` / `Completed` / `Failed` | See `ServiceHydrationStatus` shape in § 8. |
| `applied_at` / `failed_at` | `UnixInstant` | One of the two; tagged-enum payload. |
| `reason` | `String` | `Failed`-variant only. |
| `lamport_counter` / `writer_node_id` | per ObservationStore convention | Forward-compat with Phase 2 Corrosion gossip. |

### Migration discipline

- **Additive-only** per `whitepaper.md` § *Consistency Guardrails*.
  No `ALTER TABLE ADD COLUMN NULL` against existing tables; this
  is a fresh table at first introduction.
- **Single-writer in Phase 2.2** — only the action shim's
  `service_hydration` module writes. The hydrator reconciler is
  the sole reader. The single-writer constraint is consistent
  with `LocalObservationStore`'s Phase 1 model (ADR-0012 revised).
  Phase 2 Corrosion adoption inherits the same row shape; LWW
  semantics on `(service_id, fingerprint)` are deterministic
  because the fingerprint is content-hashed.

### Trait surface

Reads / writes go through the existing `ObservationStore` trait;
typed row helpers `service_hydration_results_rows(service_id)` /
`write_service_hydration_result(row)` are added to the trait as
the natural extension shape (matching the existing
`alloc_status_rows` / `node_health_rows` precedent).

---

## § 13 Quality-attribute scenarios

Extending `brief.md` § 32 / § 38. ASR = Architecturally Significant
Requirement.

| ASR | Quality attribute | Scenario | Pass criterion |
|---|---|---|---|
| ASR-2.2-01 | Reliability — zero-drop atomic swap | Source: synthetic XDP traffic at 50 kpps (CI) / 100 kpps (Lima) traversing a service VIP. Stimulus: SERVICE_MAP outer-map inner-fd swap to a new backend set during sustained traffic. Environment: native XDP on virtio-net. Response: every packet either matches old backend set XOR new — never drops on the swap boundary. Measure: `bpftool` counters + tcpdump on veth. | 0 swap-boundary drops over a 30-second swap-storm window (research § 3) |
| ASR-2.2-02 | Reliability — flow-affinity bound under churn | Source: synthetic 5-tuple connection set. Stimulus: backend churn — remove 1/N backends, rebuild Maglev table, atomically swap inner-map. Environment: M=16_381, N=100, M ≥ 100·N rule (research § 5.2). Response: histogram of remapped 5-tuples across 1000 churn cycles. | ≤ 1% of 5-tuples remap per single-backend removal (research § 5.2) |
| ASR-2.2-03 | Maintainability — verifier-budget headroom | Source: `cargo xtask verifier-regress` on each PR. Stimulus: any change to `xdp_service_map.rs` or shared sanity prologue. Environment: Linux 6.8 (`ubuntu-latest`), aya-rs `--release`. Response: instruction-count delta vs Slice 04 baseline + absolute fraction of 1M verifier ceiling. Measure: `veristat` JSON output. | Delta ≤ 20 % per PR; absolute ≤ 60 % of 1M ceiling (DISCUSS Risk #6) |
| ASR-2.2-04 | Correctness — hydrator ESR closure | Source: DST harness with `SimDataplane` + `SimObservationStore`. Stimulus: arbitrary sequence of `service_backends` row mutations + injected `DataplaneError` failures + clock advances. Environment: Tier 1, every PR. Response: `assert_always!(HydratorIdempotentSteadyState)` + `assert_eventually!(HydratorEventuallyConverges)`. Measure: `cargo xtask dst --workspace`. | Both invariants hold across the seeded fault catalogue (J-PLAT-004) |

These slot under `brief.md` § 32 / § 38 as a new sub-section under
Phase 2.2.

### Test file inventory (advisory for DISTILL)

The following file paths are advisory mappings from each ASR /
contract to the test surface that exercises it. The acceptance
designer (Atlas) locks the final paths in
`distill/test-scenarios.md`; nothing here pre-binds the DISTILL
output, but each path has an obvious crate home that follows
existing project precedent.

| ASR / contract | Tier | Test home (advisory) |
|---|---|---|
| ASR-2.2-01 — zero-drop SERVICE_MAP atomic swap | Tier 3 | `crates/overdrive-dataplane/tests/integration/atomic_swap.rs` driven via `cargo xtask integration-test vm` (or the Lima inner-loop wrapper); gated on the `integration-tests` feature per `testing.md` § *Integration vs unit gating* |
| ASR-2.2-02 — ≤ 1 % Maglev disruption per single-backend removal | Tier 1 + Tier 3 | Tier 1 DST proptest at `crates/overdrive-sim/tests/integration/maglev_churn.rs` (1024-case default, seeded `Entropy`); Tier 3 confirming run on real veth in the same `cargo xtask integration-test vm` harness |
| ASR-2.2-03 — verifier-budget delta ≤ 20 % per PR | Tier 4 | `cargo xtask verifier-regress`; baseline at `perf-baseline/main/verifier-budget/` (companion to `perf-baseline/main/xdp-perf/`) |
| ASR-2.2-04 — hydrator ESR closure | Tier 1 | `crates/overdrive-sim/src/invariants/service_map_hydrator.rs` — the two named DST invariants `HydratorEventuallyConverges` + `HydratorIdempotentSteadyState`; runs on every PR per `testing.md` § Tier 1 |
| Endianness lockstep (§ 11) | Tier 2 + userspace proptest | Tier 2 BPF unit at `crates/overdrive-bpf/tests/integration/reverse_key_roundtrip.rs` (`BPF_PROG_TEST_RUN`-driven, PKTGEN/SETUP/CHECK triptych); userspace mod-tests proptest at `crates/overdrive-dataplane/src/maps/reverse_nat_map_handle.rs` covers the host-order write/read roundtrip |

These paths are **advisory for DISTILL — the acceptance designer
locks the final paths in `distill/test-scenarios.md`.** Several
paths land under `tests/integration/` per
`testing.md` § *Layout — integration tests live under
tests/integration/* and are gated on the `integration-tests`
feature; the unit-vs-integration boundary is the crafter's call,
guided by the rule's wall-clock and real-infra criteria.

---

## § 14 Traceability

Mapping US-01 .. US-08 → slices → ADRs → ASRs.

| User story | Slice | Anchoring ADR(s) | Anchoring ASR(s) | Whitepaper § |
|---|---|---|---|---|
| US-01 Real-iface XDP attach (veth, not `lo`) | slice-01-real-iface-attach | ADR-0038 (substrate) | — | § 7 |
| US-02 SERVICE_MAP forward path with single backend | slice-02-service-map-single-vip | ADR-0040 (three-map split) | ASR-2.2-03 (verifier baseline) | § 7 / § 15 |
| US-03 HASH_OF_MAPS atomic per-service backend swap | slice-03-hash-of-maps-atomic-swap | ADR-0040 (HASH_OF_MAPS) | ASR-2.2-01 | § 15 |
| US-04 Maglev consistent hashing inside MAGLEV_MAP | slice-04-maglev-consistent-hashing | ADR-0041 (weighted Maglev) | ASR-2.2-02 | § 7 / § 15 |
| US-05 REVERSE_NAT_MAP for response-path rewrite | slice-05-reverse-nat | ADR-0041 (REVERSE_NAT shape + endianness) | — | § 7 |
| US-06 Pre-SERVICE_MAP packet-shape sanity checks | slice-06-sanity-prologue | ADR-0040 (Q3=C inline-helper shape) | ASR-2.2-03 | § 19 |
| US-07 Tier 4 perf gates + veristat baseline land on `main` | slice-07-tier4-perf-gates | ADR-0040 (Q4=B defer perf-baseline-update helper) | ASR-2.2-03 | § 7 |
| US-08 SERVICE_MAP hydrator reconciler converges Dataplane port | slice-08-service-map-hydrator-reconciler | ADR-0042 (hydrator reconciler) | ASR-2.2-04 | § 18 / § 4 |

K1–K8 from `outcome-kpis.md` map 1:1 with US-01 .. US-08 already
(per DISCUSS); ASR-2.2-01 .. 04 cross-cut the slices that exercise
the reliability / maintainability / correctness boundary.

---

## § 15 Handoff to DISTILL

The acceptance designer (Atlas) consumes:

1. **This document** (`design/architecture.md`) — full
   architectural specification.
2. **`design/wave-decisions.md`** — D1 .. D10 decision log; Reuse
   Analysis; constraints; tech stack; upstream changes.
3. **`design/proposal-draft.md`** — kept for traceability of the
   propose-mode dialogue (see § 16).
4. **The three new ADRs** in
   `docs/product/architecture/adr-{0040,0041,0042}-*.md`.
5. **DISCUSS artifacts** — `discuss/user-stories.md` (8 stories),
   `discuss/story-map.md` (8 slices), `discuss/wave-decisions.md`,
   `discuss/outcome-kpis.md` (8 KPIs), `discuss/dor-validation.md`.
6. **Slice briefs** — `slices/slice-{01..08}-*.md`.

Atlas's Phase 2 extracts AC into `distill/test-scenarios.md` (Rust
`#[test]` / `#[tokio::test]` BDD bodies, no `.feature` files per
`testing.md`); each scenario references one or more of the four
ASRs above. The hydrator's ESR pair lands as concrete DST
invariant property tests in `crates/overdrive-sim/src/invariants/`.

---

## § 16 Housekeeping — `proposal-draft.md`

`proposal-draft.md` is **kept** in this directory as a reference
record of the propose-mode dialogue (§4 onward of the locked
recommendations). The user ratified its contents with `lgtm`; this
file (`architecture.md`) is the authoritative DESIGN-wave
deliverable, but the proposal preserves the decision provenance
for future readers tracing why each open question landed where it
did. A future cleanup may consolidate; for now, leave both. This
choice is explicitly recorded in `wave-decisions.md` § Upstream
Changes.

---

## § 17 Cross-cutting concerns surfaced during DESIGN

- **DISCUSS slice 04 budget acknowledged** — `discuss/wave-decisions.md`
  flagged Slice 04 as 1.5d. This DESIGN does not change the slice
  shape, so no DISCUSS edit is required. Logged as informational.
- **No upstream changes** — the additive `service_hydration_results`
  table, four newtypes, one Action variant, and one reconciler are
  all *additive* relative to the Phase 2.1 substrate. No prior ADR
  is superseded; no aggregate is mutated. The brief.md ADR index
  grows from 32 entries (post-ADR-0038) to 35; no entry changes
  status.

---

*End of architecture.md. This document is read-only at handoff
time. Future amendments require a new ADR with `supersedes` /
`amends` semantics per `brief.md` ADR convention.*

---

## Review

| Field | Value |
|---|---|
| Review ID | `arch-rev-2026-05-05-phase2.2-xdp-service-map` |
| Reviewer | Atlas (`nw-solution-architect-reviewer`, Haiku 4.5) |
| Date | 2026-05-05 |
| Initial verdict | `NEEDS_REVISION` |
| Final verdict | **APPROVED after remediation** (2026-05-05) |

### Reviewer's praise (verbatim quote)

> "The three-map split decision correctly resolves the wire-layer
> /control-plane-layer keying confusion that the proposal-draft
> initially carried; the locked outer keys
> (`(ServiceVip, u16 port)` for SERVICE_MAP / `ServiceId` for
> MAGLEV_MAP / `BackendId` for BACKEND_MAP) are typed-distinct
> end-to-end. Architecture is exceptionally coherent given the
> phase-2.2 scope envelope."

### Findings and resolution

Atlas surfaced 5 blocking-class issues + 2 non-blocking questions.
All seven were addressed in a single remediation pass on
2026-05-05. None of the seven user-ratified open-question
decisions or the three drifts were re-litigated; this pass is
artifact lockdown only — no design decisions changed.

| ID | Finding (Atlas) | Resolution location |
|---|---|---|
| B1 | ADR-0042 § 2 deferred concrete `hydrate_desired` / `hydrate_actual` shape to ADR-0036; lockpoint must carry the signatures inline. | architecture.md § 8 *Hydration shape* (free-function arm signatures + 3-bullet rationale) + ADR-0042 § 2 *Hydration shape* (mirrors the architecture.md text). |
| B2 | ADR-0042 § 4 referenced architecture.md § 12 for the `service_hydration_results` schema; the schema lockpoint must be inline in the ADR. | ADR-0042 § 4 *Schema* (full table inline; LWW resolution semantics; additive-only migration rationale). |
| B3 | `BackendSetFingerprint` is referenced throughout the design but never defined. | architecture.md § 6 *Type aliases* (alias declaration + computation rule + module placement) + ADR-0040/§-6-companion-cite + ADR-0041 § 2 (Maglev context cites the alias) + ADR-0042 § 1/§ 4 (Action variant + table cite the alias). |
| S4 | `DropClass` and `MaglevTableSize` described in prose; need actual Rust code blocks. | architecture.md § 6 (full code blocks for both) + ADR-0040 § 6 (DropClass code block) + ADR-0041 § 1 (MaglevTableSize code block). |
| S5 | `CorrelationKey` derivation pinned only as prose; should reference the existing `CorrelationKey::derive` constructor surface explicitly. | architecture.md § 7 (locked code snippet citing `crates/overdrive-core/src/id.rs`'s existing `derive` shape) + ADR-0042 § 1 (mirrors the snippet). |
| Q1 | Tier 2 / Tier 3 test file homes not surfaced in the design. | architecture.md § 13 *Test file inventory (advisory for DISTILL)* — five test paths listed, advisory not binding on DISTILL. |
| Q2 | Whether `service_backends.vip` is `u32` / `Ipv4Addr` / something else, and how that interacts with Constraint 10. | architecture.md § 8 inline note before `ServiceDesired` — Case A confirmed: `service_backends` row carries `vip: Ipv4Addr` as its existing wire-shape field; `ServiceVip` is a userspace control-plane newtype; the hydrator wraps at the `hydrate_desired` boundary; no schema migration. |

B1–B3 + S4–S5 + Q1–Q2 addressed in a single pass on 2026-05-05;
Atlas not re-invoked because all changes are mechanical artifact
lockdowns, not new design decisions. The three peer-review iteration
budget is preserved for genuine design-revision rounds.
