# ADR-0049 — Platform-issued Service VIP allocator: shared pool primitive under `overdrive-dataplane`; `IntentStore`-persisted; submit-time admission; reconciler-driven reclamation

## Status

**Accepted (2026-05-14, amended 2026-05-14 / 2026-05-15 / 2026-05-19 /
2026-06-28)**.
Decision-makers: Morgan (proposing); DESIGN-wave output of
`docs/feature/service-vip-allocator/`. The 2026-06-28 amendment
supersedes the § 6 *release trigger* (release-on-terminal → release-
on-deletion) per #251 / RCA-251 / the K8s+Nomad lifecycle research.

Tags: phase-1, dataplane, application-arch, allocator-primitive,
admission, persistence-boundary.

**Relates to**: ADR-0046 (BackendId allocator — structural precedent
this ADR generalises); ADR-0042 (`ServiceMapHydrator` — downstream
consumer of VIPs); ADR-0041 (`update_service` shape); ADR-0040
(SERVICE_MAP three-map split); ADR-0047 (`WorkloadSpec::Service` +
`ListenerRow.vip: Option<ServiceVip>` — upstream feature whose
deferral #167 closes); ADR-0048 (rkyv versioned envelope); ADR-0019
(operator config TOML); ADR-0035 / ADR-0036 (reconciler runtime
contract); ADR-0011 (intent vs observation aggregate split);
ADR-0013 (reconciler primitive); ADR-0072 (dial-by-name responder —
the `FrontendAddrAllocator` whose release-on-deletion-only lifecycle
the 2026-06-28 amendment makes the Service VIP symmetric with).

**SSOT**: [overdrive-sh/overdrive#167](https://github.com/overdrive-sh/overdrive/issues/167).
DISCUSS artifacts at `docs/feature/service-vip-allocator/discuss/`.

## Context

ADR-0046 landed a `BackendIdAllocator` (monotonic counter + memo) at
`crates/overdrive-dataplane/src/allocator.rs:31-82`. That allocator
is process-local and re-hydrated on restart by the
`ServiceMapHydrator` reconciler (ADR-0042). It is the structural
precedent — and the only existing in-tree allocator — for what
GH #167 specifies.

ADR-0047 § 4a landed the `ListenerRow.vip: Option<ServiceVip>` spec
field as part of Slice 06 of `workload-kind-discriminator`. The
field is forward-compatible with both "allocate at runtime" and
"reject at admission" outcomes; this ADR resolves the path. Per the
DISCUSS wave's Changed Assumption
(`docs/feature/service-vip-allocator/discuss/wave-decisions.md`
§ Changed Assumptions, 2026-05-14): **VIPs are platform-issued
only**. Operators cannot supply `vip = Some(...)` in a Service
`[[listener]]` block.

*(Amended 2026-05-14 — § 5.)* Per
`.claude/rules/development.md` § "Type-driven design" → "make
invalid states unrepresentable", the `vip` field on `Listener` is
removed at the parser/spec layer. An operator-supplied `vip` is now
unrepresentable in the parsed spec; the prior admission-level
rejection is unnecessary and is deleted. Slice 06's spec shape
back-propagates in the same commit; see `upstream-changes.md`.

The feature must extend the existing `BackendIdAllocator` shape
into a reusable primitive that supports a second consumer
(`ServiceVipAllocator`) whose persistence requirement is stronger
than the backend allocator's. AC-02 of #167 requires that
"allocations are persisted to `IntentStore` and survive control-plane
restart (round-trip test)" — whereas `BackendIdAllocator` deliberately
does not persist, relying on the hydrator to rebuild on boot.

Five DESIGN-wave open questions are resolved together here:

1. **Reclamation trigger** — which reconciler / action shim emits
   the VIP release on terminal-state transition?
2. **When admission allocates** — submit-time vs reconciler-tick.
3. **Pool config shape** — TOML structure under existing `[dataplane]`
   or a new section.
4. **Shared allocator trait shape** — how to factor
   `BackendIdAllocator` + `ServiceVipAllocator` into a reusable
   primitive.
5. **Upstream slice-06 spec shape** — parser-level removal vs
   admission-level rejection of operator-supplied `vip = Some(...)`.
   *(Amended 2026-05-14 to parser-level removal; see § 5.)*

## Decision

### 1. VIP-pool allocator + persistence shim — two concrete allocators, no shared trait

*(Amended 2026-05-14 — was "Shared allocator primitive — pure core +
persistence shim". See § Considered alternatives for the rejection of
the generic `PoolAllocator<T: Token>` core.)*

*(Amended 2026-05-19 — slot-reuse policy on release diverges from
`BackendIdAllocator`. Released VIPs return to the available pool and
may be re-allocated by subsequent `allocate(&different_digest)` calls.
The monotonic `next` counter is removed; address selection on
allocate is "first non-reserved address in `range` not currently held
in `memo.values()`". See § Amendments → 2026-05-19.)*

The Phase 1 allocator surface is **two concrete allocators** living
side-by-side under `crates/overdrive-dataplane/src/allocators/`:

- **`BackendIdAllocator`** (existing, relocated from
  `src/allocator.rs` → `src/allocators/backend_id.rs` via single-cut
  migration per `feedback_single_cut_greenfield_migrations.md`).
  Process-local, no persistence; re-hydrated on restart by
  `ServiceMapHydrator` per ADR-0042.
- **`ServiceVipAllocator`** (new). Persists via the
  `PersistentServiceVipAllocator` shim that wraps it; required by
  AC-02 (allocations survive control-plane restart).

The two allocators **share the memo + memo-hit-returns-existing
shape** but diverge on slot-reuse policy on `release`. They **share
no trait and no generic type**. The shared logic ("memo hit returns
existing; memo miss advances allocation; release deletes the memo
entry") is thinner than the abstraction surface required to factor
it generically — see § Considered alternatives for the full RCA.

**Per-allocator shapes** *(amended 2026-05-19):*

- `BackendIdAllocator` keeps its monotonic counter and no-slot-reuse-on-release
  policy. Its address space is internally-allocated and effectively
  unbounded; monotonicity matches the Snowflake / event-source ID
  precedent and the in-tree shape since ADR-0046.
- `ServiceVipAllocator` reuses VIPs on release (no `next` counter at
  all). Its address space is a finite IPv4 CIDR — `/16` is 65 K
  addresses; `/24` is 254; `/32` is 1. A monotonic-only allocator
  would exhaust the pool after `capacity` total submissions over
  process lifetime regardless of current liveness, and "restart to
  recover" is not an operability story. Every comparable
  Service-VIP allocator in the ecosystem (Kubernetes ClusterIP,
  Cilium IPAM, MetalLB, kube-vip) reuses released addresses.

The 2026-05-14 amendment's "shape-equivalence with `BackendIdAllocator`"
framing was load-bearing when AC-05 still required literal code reuse;
with AC-05 already restated as shape-similarity (not code reuse) and
no shared trait between the two types, divergence on release policy
costs nothing structural.

**Module location**:
`crates/overdrive-dataplane/src/allocators/` (plural — the existing
`allocator.rs` moves into the module as `allocators/backend_id.rs`
via a single-cut migration per
`feedback_single_cut_greenfield_migrations.md`; the existing tests
move with it). Layout (post 2026-05-14 amendment — no generic
`pool.rs`, no `Token` trait):

```
crates/overdrive-dataplane/src/allocators/
├── mod.rs                 # re-exports
├── error.rs               # ServiceVipAllocatorError + VipAllocatorConfigError
├── vip_range.rs           # VipRange (Ipv4Net + reserved set)
├── backend_id.rs          # BackendIdAllocator (moved from allocator.rs)
└── service_vip.rs         # NEW — ServiceVip newtype + ServiceVipAllocator
                           #       (concrete, NOT generic)
```

**`ServiceVipAllocator` (concrete, in-memory)** *(amended 2026-05-19 —
no `next` counter; address selection by scan over `range`)*:

```rust
pub struct ServiceVipAllocator {
    by_digest: BTreeMap<ServiceSpecDigest, ServiceVip>,   // memo (SSOT)
    range: VipRange,
}

impl ServiceVipAllocator {
    pub fn new(range: VipRange) -> Self;

    /// Memo-hit returns the existing VIP. On memo-miss, scans
    /// `range.nth(0)..range.nth(capacity-1)` in canonical order and
    /// returns the first non-reserved address NOT currently held in
    /// `by_digest.values()`. Returns `Exhausted` when no such address
    /// exists. The scan order is deterministic — same `range` + same
    /// `memo` always selects the same next VIP, so DST/proptest
    /// reproducibility (K3 of testing.md § DST) is preserved without
    /// any tie-breaker logic.
    pub fn allocate(&mut self, digest: ServiceSpecDigest) -> Result<ServiceVip, ServiceVipAllocatorError>;

    /// Removes the memo entry. The freed VIP becomes available for
    /// re-allocation to a subsequent `allocate(&different_digest)` call.
    /// Idempotent on already-released keys.
    pub fn release(&mut self, digest: &ServiceSpecDigest);

    pub fn get(&self, digest: &ServiceSpecDigest) -> Option<ServiceVip>;
    pub fn memo_len(&self) -> usize;
}

#[derive(thiserror::Error, Debug)]
pub enum ServiceVipAllocatorError {
    /// No tokens available — the configured `VipRange` is fully
    /// allocated. Surfaces AC-04 (#167) at the admission boundary.
    #[error("VIP pool exhausted: {allocated} of {capacity} addresses in use")]
    Exhausted { allocated: u32, capacity: u32 },
}
```

The memo is `BTreeMap`-backed per `.claude/rules/development.md`
§ "Ordered-collection choice" — the memo is iterated by DST
invariants, at bulk-load time, AND on every `allocate` to find the
next non-held address (the scan is `range.nth(i)` for ascending `i`,
short-circuiting on the first `Ipv4Addr` whose `ServiceVip` is
absent from `by_digest.values()`).

**Reuse on release** *(amended 2026-05-19)*: `release(&digest)`
deletes the memo entry, returning the VIP to the available pool.
A subsequent `allocate(&different_digest)` MAY receive the freed
VIP — specifically, it WILL receive the freed VIP if the freed VIP
is the lowest-indexed non-held address in `range`. The structural
invariant "no two simultaneously-held memo entries share a VIP" is
preserved by construction (the scan refuses any address present in
`by_digest.values()`). The 2026-05-14 amendment's "shape-equivalent
with `BackendIdAllocator`'s non-reuse semantics" framing is
withdrawn; see § Amendments → 2026-05-19 for the RCA. Alt-D
("Counter recycling") is partially accepted by this amendment —
basic free-on-release lands now; LRU / age-based policies remain
deferred.

**Why "scan the memo" instead of a free list**: the free list would
be derived state (recomputable at any moment from `(range, memo)`),
which under `.claude/rules/development.md` § "Persist inputs, not
derived state" should not be persisted. Recomputing on every
allocate is `O(capacity)` worst-case scan over a `BTreeMap` membership
test — at Phase 1 `/16` (65 K addresses) it is microsecond-class and
dominated by the redb fsync that follows on the persistence shim;
KPI K2 (p50 ≤ 5 ms / p99 ≤ 25 ms) is preserved by a wide margin.
If future Phase 3+ deployments with larger pools and higher churn
show scan-cost pressure, a `free: VecDeque<ServiceVip>` cache may
be added as an in-memory optimization (recomputed from memo on
`bulk_load`, never persisted); that's additive and out of scope
here.

**Persistence shim — `PersistentServiceVipAllocator`**:

```rust
pub struct PersistentServiceVipAllocator {
    inner: parking_lot::Mutex<ServiceVipAllocator>,
    store: Arc<dyn IntentStore>,
}

impl PersistentServiceVipAllocator {
    /// Construct empty + bulk-load the persisted state.
    ///
    /// `bulk_load` performs an Earned Trust gate (probe() — see §8):
    /// reads every persisted `(spec_digest, vip)` pair from
    /// `IntentStore` under the `allocator_entries` table, validates
    /// round-trip via the rkyv envelope, and refuses to start
    /// (returning `AllocatorBootError::Envelope`) if any row fails
    /// to decode.
    pub fn bulk_load(
        range: VipRange,
        store: Arc<dyn IntentStore>,
    ) -> Result<Self, AllocatorBootError>;

    /// Allocate-or-memo. Writes through to IntentStore on a fresh
    /// allocation; memo-hits are zero-write.
    ///
    /// Ordering is fsync-then-memory (matches ADR-0035 §
    /// "Step ordering 7 → 8 is load-bearing"): the IntentStore write
    /// commits + fsyncs before the in-memory `ServiceVipAllocator` is
    /// updated. On crash between fsync and memory-update, the next
    /// boot's bulk_load rebuilds the memo from the persisted state.
    pub fn allocate(&self, digest: ServiceSpecDigest) -> Result<ServiceVip, AllocatorError>;

    /// Release-and-delete. Idempotent on already-released keys.
    pub fn release(&self, digest: &ServiceSpecDigest) -> Result<(), AllocatorError>;

    /// Borrow the read-only view of the pool (for diagnostics /
    /// alloc status echo).
    pub fn get(&self, digest: &ServiceSpecDigest) -> Option<ServiceVip>;
}
```

**BackendId** keeps its existing concrete `BackendIdAllocator`
unchanged in body — only the file moves. Its shape (BTreeMap memo
keyed by `(ip, port, proto)`, monotonic counter, no slot reuse) is
the precedent the `ServiceVipAllocator` matches; the
shape-equivalence is documentation, not a shared type.

### 1a. Persistence wire format — rkyv envelope per ADR-0048

*(Amended 2026-05-14 — single concrete envelope for ServiceVip only;
no per-token-type generality.)*

The persisted state crosses an `IntentStore` redb boundary, so it
follows ADR-0048's per-type versioned envelope discipline. One
envelope, specific to the `ServiceVipAllocator` — `BackendId` does
NOT persist (it re-hydrates from observation per ADR-0042). Wire
shape:

```rust
// Persisted row — one per (spec_digest, vip) pair.
// Lives in overdrive-core::dataplane (next to existing dataplane
// types) per the precedent of BackendKey / ServiceVip.
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, ...)]
pub struct ServiceVipAllocatorEntryV1 {
    pub spec_digest: [u8; 32],   // ServiceSpecDigest
    pub vip:         u32,        // host-order IPv4 octets
    pub counter:     u32,        // monotonic counter value at allocation
                                 // (deprecated by V2; see 2026-05-19 amendment)
}

// 2026-05-19 amendment: counter field removed; address selection no
// longer monotonic. V1 → V2 conversion drops `counter` (it carried
// no behavior the V2 allocator needs).
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, ...)]
pub struct ServiceVipAllocatorEntryV2 {
    pub spec_digest: [u8; 32],
    pub vip:         u32,
}

impl From<ServiceVipAllocatorEntryV1> for ServiceVipAllocatorEntryV2 {
    fn from(v1: ServiceVipAllocatorEntryV1) -> Self {
        Self { spec_digest: v1.spec_digest, vip: v1.vip }
        // v1.counter intentionally dropped — V2 allocator has no
        // monotonic counter; address selection is scan-over-range.
    }
}

// Codec-internal envelope (NOT re-exported from lib.rs per UI-01).
// V1 variant retained per ADR-0048 § "Version-bump procedure" —
// "existing fixtures are NEVER touched"; V1 stays in the envelope
// so the per-type golden-bytes fixture continues to assert that
// V1 bytes decode and round-trip through into_latest() → V2.
pub enum ServiceVipAllocatorEntryEnvelope {
    V1(ServiceVipAllocatorEntryV1),
    V2(ServiceVipAllocatorEntryV2),
}

impl VersionedEnvelope for ServiceVipAllocatorEntryEnvelope {
    type Latest = ServiceVipAllocatorEntryV2;
    fn latest(p: Self::Latest) -> Self { Self::V2(p) }
    fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
        match self {
            Self::V1(v1) => Ok(v1.into()),  // drop counter
            Self::V2(v2) => Ok(v2),
        }
    }
}

// Public alias-to-payload (UI-02) — points at V2 latest.
pub type ServiceVipAllocatorEntry = ServiceVipAllocatorEntryV2;
pub type ServiceVipAllocatorEntryLatest = ServiceVipAllocatorEntryV2;
```

Wrapping discipline lives in a codec module on
`ServiceVipAllocatorEntry`
(`ServiceVipAllocatorEntry::archive_for_store` / `from_store_bytes`)
per ADR-0048 § "Typed persistence-boundary codec". Schema-evolution
golden-bytes fixtures under
`crates/overdrive-dataplane/tests/schema_evolution/service_vip_allocator_entry.rs`
pin BOTH V1 archived bytes (per `.claude/rules/testing.md` §
"Archive schema-evolution roundtrip" — existing fixtures are NEVER
touched per ADR-0048 § "Version-bump procedure") AND V2 archived
bytes. The V1 fixture asserts decode → `into_latest()` → V2 projection
equality; the V2 fixture asserts decode → V2-typed payload equality.
The V1 → V2 conversion drops `counter`, which previously carried the
allocator's monotonic-counter value at allocation time — V2's
allocator has no monotonic counter, so the field is structurally
unused under the 2026-05-19 amendment.

The crafter may choose the exact typed name (e.g.
`ServiceVipAllocatorEntry` vs `AllocatorEntry`); the load-bearing
property is one envelope, one persisted shape, scoped to the
ServiceVip allocator. No `AllocatorTokenBytes` sum type — BackendId
never persists, so a generic envelope buys nothing.

### 2. `ServiceVip` and `VipRange` types

```rust
// crates/overdrive-dataplane/src/allocators/vip_range.rs

/// A range of IPv4 VIPs the platform may allocate to Service workloads.
/// Built from operator config (§3); immutable after boot.
#[derive(Clone)]
pub struct VipRange {
    cidr: Ipv4Cidr,                 // from existing workspace dep
    reserved: BTreeSet<Ipv4Addr>,   // platform-reserved (e.g., gateway, broadcast)
}

impl VipRange {
    /// Build + validate. Returns `VipAllocatorConfigError` on
    /// overlapping ranges, out-of-range reserved, or zero capacity.
    pub fn new(cidrs: Vec<Ipv4Net>, reserved: BTreeSet<Ipv4Addr>)
        -> Result<Self, VipAllocatorConfigError>;

    pub fn capacity(&self) -> u32 { /* sum(CIDR sizes) - reserved.len() */ }

    /// Map an index N to the Nth non-reserved address in canonical
    /// order. Returns None when N >= capacity(). Used by
    /// `ServiceVipAllocator::allocate` internally as the scan
    /// primitive — the allocator iterates `nth(0)..nth(capacity-1)`
    /// and returns the first address NOT currently held in its memo
    /// (per 2026-05-19 amendment; pre-amendment this was driven by a
    /// monotonic counter — same `nth` semantics, different caller
    /// behavior).
    pub fn nth(&self, n: u32) -> Option<Ipv4Addr>;
}
```

`ServiceVip` is the canonical newtype at
`overdrive-core::id::ServiceVip(Ipv4Addr)`. The
`crates/overdrive-dataplane/src/allocators/service_vip.rs` module
imports (or re-exports) the canonical newtype rather than declaring
a local one; the consolidation deletes the duplicate at
`crates/overdrive-core/src/aggregate/workload_spec.rs:360` in the
same commit.

The `ServiceVip` newtype is **already declared twice** in the
codebase: at `crates/overdrive-core/src/aggregate/workload_spec.rs:360`
(`pub struct ServiceVip(pub Ipv4Addr)`, used by ADR-0047 spec layer)
and at `crates/overdrive-core/src/id.rs:647`
(`pub struct ServiceVip(std::net::IpAddr)`, used elsewhere). The two
are **inconsistent on IPv4 vs IpAddr**. This ADR consolidates to a
**single `ServiceVip` newtype wrapping `Ipv4Addr`** at
`overdrive-core::id::ServiceVip`; the duplicate in `workload_spec.rs`
is deleted in the same commit. Per the 2026-05-14 amendment (§ 5),
`Listener` carries no `vip` field at all; `ServiceVip` continues to
be used by the allocator (`ServiceVipAllocator` and its persistence
shim `PersistentServiceVipAllocator`), the allocator's persisted
`ServiceVipAllocatorEntry` rkyv payload, and the downstream consumer
surface (`ServiceMapHydrator` consults
`ServiceVipAllocator::get(&spec_digest)` per § 5a — the kernel-side
`Dataplane::update_service(_, vip, _)` parameter remains
`ServiceVip`-typed).

**Why IPv4-only**: IPv6 VIPs are GH #61 (out of scope per #167 and
DISCUSS § Out of scope). Future IPv6 support will not change this
newtype — it will introduce a separate `ServiceVip6` variant or a
sum-typed `ServiceAddress { V4(ServiceVip), V6(ServiceVip6) }`. The
newtype completeness rule (`development.md` § "Newtype completeness")
is preserved: `FromStr`, `Display`, `Serialize`, `Deserialize`,
validating constructor — all already exist on the chosen newtype.

### 3. Operator config — `[dataplane.vip_allocator]` subsection

The operator config gains a new subsection under the
existing/forthcoming `[dataplane]` block per ADR-0019 (TOML config
format). Three options were considered; the chosen shape is (a)
with one extension:

```toml
[dataplane]
client_iface  = "eth0"      # GH #175, existing
backend_iface = "eth0"      # GH #175, existing

[dataplane.vip_allocator]
# Required. Single CIDR or list of CIDRs to draw VIPs from.
# Validated at boot; overlapping CIDRs are a typed
# config-error (`VipAllocatorConfigError::OverlappingRanges`).
ranges = ["10.96.0.0/16"]

# Optional. IPv4 addresses to exclude from allocation
# (e.g., gateway, broadcast, intentionally-reserved). Validated
# at boot to lie within `ranges`; addresses outside `ranges`
# are a typed config error.
reserved = ["10.96.0.0", "10.96.0.1", "10.96.255.255"]
```

| Shape | Verdict |
|---|---|
| (a) `[dataplane.vip_allocator]` nested subsection (chosen) | The allocator is a dataplane concern; nesting reads as "a dataplane sub-component." Aligns with existing `[dataplane.client_iface]` pattern from #175. |
| (b) Top-level `[vip_allocator]` section | Reads as "a separate subsystem"; doesn't reflect that the allocator is structurally part of the dataplane primitive set. **REJECTED**. |
| (c) Fold flat into `[dataplane]` (e.g., `dataplane.vip_ranges = [...]`) | Loses room for additional vip-allocator-specific config (future: per-tenant pools, allocation policy); collapses unrelated knobs into one bag. **REJECTED**. |

**Single CIDR vs list of CIDRs**: list (`ranges = [...]`) is chosen.
Phase 1 single-node deployments use a single CIDR; future
multi-region deployments may carve per-region ranges from disjoint
CIDRs. The list shape is forward-compatible with no schema change.
Validation: ranges must be non-overlapping; total capacity =
sum(CIDR sizes) - `len(reserved)`.

**Defaults (amended 2026-05-15 — supersedes the prior "no defaults"
stance below).** When `[dataplane.vip_allocator]` is absent the
allocator now boots with `ranges = ["10.96.0.0/16"]` and
`reserved = ["10.96.0.0", "10.96.0.1", "10.96.255.255"]`. Boot
emits a single structured `health.startup.warn` event recording
that the default is in use; when the node is configured for HA
mode (Phase 2+) the warn is emitted on every startup. The CLI
renders the default's effective shape ahead of admission so the
operator is not surprised by the first allocation. See
`docs/research/orchestration/service-vip-range-config-patterns.md`
for the comparative analysis (Kubernetes / Cilium / MetalLB / KubeVirt
/ kube-vip / Calico-CNI all ship pinned defaults; Phase 1 single-node
Overdrive matches the same operator expectation, with the HA-mode
warn-every-startup discipline preserving Earned Trust visibility once
multi-node arrives). The supersession trades one bit of strict
"refuse-to-start honesty" for parity with the operator's baseline
expectation across the ecosystem; the warn event + CLI rendering
keep the choice observable.

*(Walked back 2026-05-15.)* The original "no defaults" framing
read: "operators MUST supply `ranges`. Per #167 § 'Out of scope':
'Opinionated default VIP ranges. The allocator is pool-agnostic.'
A missing `[dataplane.vip_allocator]` section surfaces as
`VipAllocatorConfigError::Missing` at boot — refuse to start,
structured `health.startup.refused` event per ADR-0048 intent-layer
discipline." That framing is superseded: the missing-section path
now yields the warn + default-shape boot above, not refuse-to-start.
`VipAllocatorConfigError::Missing` is removed from the typed error
surface; malformed or out-of-range config still produces typed
errors and refuses to start. See § Amendments → 2026-05-15.

### 4. When admission allocates — submit-time, before IntentStore admission

The admission handler (in `overdrive-control-plane`) allocates the
VIP **synchronously, before the `WorkloadSpec::Service` is
written to IntentStore**. The spec digest is computed first; the
allocator's `allocate(spec_digest)` returns the assigned VIP
(memo-hit on resubmit per AC-02); the spec is written to IntentStore
*as-is* (no `vip` field on `Listener` to fold into, per the
2026-05-14 amendment — § 5); the allocator's own persisted
`allocator_entries` row is the durable record of the assignment
(§ 1a, § 5a); submit-echo renders the assigned VIP at the Service
level via `ServiceVipAllocator::get(&spec_digest)`.

| Shape | Pros | Cons | Verdict |
|---|---|---|---|
| (a) Submit-time, before IntentStore admission (chosen) | VIP visible in submit echo (AC-01); idempotency keyed on spec digest is structural (AC-02); pool-exhaustion is a synchronous rejection (AC-04); single failure surface; alignment with "trust what the CLI tells me" (J-OPS-002) | Couples admission to allocator; allocator must be probed-ready at admission time (Earned Trust gate) | **CHOSEN** |
| (b) Reconciler-tick allocation | Decouples admission from allocator; allows transient allocator unavailability to recover | Submit echo renders `vip: None` until first reconcile tick; AC-01 fails byte-equality; pool-exhaustion is an asynchronous event surfaced only via `alloc status`; second source of truth for VIP assignment | **REJECTED** — fails AC-01 + AC-04 surface shape. |

**Failure modes at admission time**:

- `AllocatorError::Exhausted { allocated, capacity }` — typed
  rejection; AC-04 surface; HTTP 503 with `ProblemDetails` per
  `ControlPlaneError`. No partial state persisted (allocator's
  write-through is fsync-then-memory; on Exhausted, neither side is
  mutated).
- `AllocatorError::Envelope` / `AllocatorError::IntentStore` —
  surface as HTTP 500; structured event;
  `health.startup.refused`-class for boot path.
- *(Removed by 2026-05-14 amendment — see § 5.)* The prior
  `AdmissionError::VipNotOperatorAssignable` admission-level
  rejection is replaced by parser-level removal of the `vip` field
  on `Listener`. Operator-supplied `vip = "..."` now fails at TOML
  deserialise with serde's `unknown field` error + named guidance.

**Spec-digest invariance**: the spec digest is computed over the
operator-input `ServiceSpec` directly. Per the 2026-05-14 amendment
(§ 5), `Listener` carries no `vip` field at all; the assigned VIP is
not stored on the spec or on the aggregate (§ 5a — the allocator's
persisted memo IS the source of truth). **Resubmitting an unchanged
spec produces the same digest** by construction — the operator's
input is the only input to the digest, and the assigned VIP cannot
contaminate it because it is not part of the spec. AC-02 is
structural.

### 5. Operator-supplied `vip` — parser-level removal (make invalid states unrepresentable)

**Amended 2026-05-14.** The prior resolution of this question
(admission-level rejection of operator-supplied `vip = Some(...)`,
preserving the `Option<ServiceVip>` field on `Listener`) is
withdrawn. New resolution: **the `vip` field is removed from
`Listener` entirely at the parser/spec layer.** Per
`.claude/rules/development.md` § "Type-driven design" → **make
invalid states unrepresentable**: the prior shape (`vip:
Option<ServiceVip>` validated to always be `None` on operator input)
is a runtime check defending against a state the type system can
exclude structurally. The operator-supplied form of `Listener` has
no business carrying a `vip` field — operators cannot supply one
(the DISCUSS Changed Assumption decided this), so the field is
meaningless on the operator spec and is deleted.

The "forward-compatible if operator-pinned VIPs come back" framing
that motivated the prior resolution is the deferral-without-issue
shape CLAUDE.md § "Deferrals require GitHub issues" forbids:
operator-pinned VIPs are a feature explicitly decided against;
defending future-compatibility with a non-feature preserves a
defense for nothing. Greenfield single-cut migration
(`feedback_single_cut_greenfield_migrations.md`): the field, its
admission-level validator, the upstream slice-06 tests that defend
the removed shape, and the `AdmissionError::VipNotOperatorAssignable`
variant all delete together in one commit.

| Shape | Pros | Cons | Verdict |
|---|---|---|---|
| (a) Parser-level removal of the `vip` field (CHOSEN — amended) | Operator-pinned-VIP state is structurally unrepresentable (type-driven design); failure mode shifts from a runtime check ("vip = Some(...)" returns `AdmissionError`) to a parse error (`unknown field 'vip'` from serde with named guidance); fewer codepaths to maintain — no `AdmissionError::VipNotOperatorAssignable` variant, no admission walk; uniqueness rule simplifies from `(vip, port, protocol)` to `(port, protocol)`; CLI submit-echo and `alloc status` shed the per-listener pending-vs-assigned distinction (the assigned VIP renders once per Service, not per listener) | Real spec-shape back-propagation to slice-06 (its already-shipped parser tests + render shape change in the same commit that lands this feature) | **CHOSEN (amended 2026-05-14)** |
| (b) Admission-level rejection, field preserved (previously CHOSEN; now REJECTED) | Parser stays "pure structural"; preserves Slice 06's forward-compatibility framing | Operator-pinned-VIP state is representable in the parsed spec and only refused at admission — a runtime defense for a type-system-excludable invariant; violates `.claude/rules/development.md` § "Type-driven design"; preserves forward-compatibility for a non-feature (operator-pinned VIPs) the project has explicitly decided against, which is the deferral-without-issue shape CLAUDE.md § "Deferrals require GitHub issues" forbids; adds an admission validator + error variant + tests + render branches that defend a state the type system could exclude | **REJECTED (2026-05-14)** |
| (c) Silent ignore (collapse `Some(_)` to `None`) | — | Silent input mutation; violates the named-guidance discipline | **REJECTED** |

**Parser-side change** (in the workload-kind-discriminator parser
that lands slice-06's `Listener`):

```rust
// Spec-side Listener — no vip field. Parser uses
// #[serde(deny_unknown_fields)] (or the equivalent for the
// existing TOML deserializer) so an operator-supplied `vip = "..."`
// fails at parse time with a typed error naming the field and
// guiding the operator: "the `vip` field is not operator-assignable;
// the platform allocates Service VIPs automatically. Remove it from
// the `[[listener]]` block."
pub struct Listener {
    pub port:     NonZeroU16,
    pub protocol: Proto,
    // vip removed — platform-issued only; see ADR-0049 § 5.
}
```

**Cascade points** (all land in the same commit as this ADR's
implementation, per single-cut migration discipline):

1. **`Listener` struct loses the `vip` field.** Slice-06's
   `Listener` shape becomes `(port, protocol)`-only.
2. **Listener uniqueness rule simplifies** (slice-06 brief lines
   36–38 — was "no two `[[listener]]` blocks within a Service may
   share `(vip, port, protocol)`. When both `vip` are `None`, the
   comparison is on `(port, protocol)` only"; now: "no two
   `[[listener]]` blocks within a Service may share `(port,
   protocol)`").
3. **Submit echo + `alloc status` render** (slice-06 brief lines
   41–45) — listener lines render as `<port>/<protocol>` only. The
   per-listener `<vip-or-pending>:<port>/<protocol>` shape is
   deleted. The **allocator-assigned VIP renders at the Service
   level**, not per-listener — see § 5a below.
4. **`AdmissionError::VipNotOperatorAssignable` is DELETED.** The
   field is gone, the variant is unreachable; per
   `.claude/rules/development.md` § "Deletion discipline" the
   variant and any test that defends it delete in the same commit.
5. **Slice-06 already-shipped tests update**: the parser unit test
   for "mixed-pinned-and-pending VIPs" (slice-06 brief lines
   60–63) is deleted; the integration test that submits "one
   pinned, one pending" listener (lines 64–67) is deleted; the
   property test that round-trips listener triples with `vip`
   (lines 70–71) updates to round-trip `(port, protocol)` pairs.
   Per deletion discipline, the removals land in the same commit as
   the field removal. New tests defending the new shape (`vip` field
   is rejected as `unknown field` at parse with named guidance;
   uniqueness on `(port, protocol)`) are written from scratch.
6. **R6.1 risk mitigation in slice-06 (lines 132–137)** is moot —
   its "the `Option`-shaped field is forward-compatible" framing
   no longer applies because the field is removed. The risk
   resolves by deletion, not by mitigation.

### 5a. Where the assigned VIP lives (placement decision)

With the `vip` field removed from `Listener`, the question of
*where the allocator-assigned VIP is recorded* becomes a fresh
design decision. **One VIP per Service**, shared across all of
the Service's listeners (the standard Cilium/k8s LB shape: one
address, multiple ports). The allocator key per Q4's resolution is
`(service_id, spec_digest) → ServiceVip` — 1:1 per Service.

Three placement options were considered:

| Option | Shape | Pros | Cons | Verdict |
|---|---|---|---|---|
| (A) `Service::assigned_vip: ServiceVip` aggregate field, set by admission post-allocate, before IntentStore write | Single source on the aggregate; restart-survival via IntentStore | The aggregate carries an operator-shape field that is not operator-set; requires `#[serde(skip_deserializing)]` or equivalent to defend; reintroduces a "policy field on the operator-facing struct" that is exactly the smell the parser-level removal is fixing | **REJECTED** |
| (B) Observation-only (`alloc_status` or new `service_assignments` ObservationStore table) | Clean intent/observation split | AC-01 (submit echo renders the assigned VIP) requires synchronous observation-write at admission, which the admission path does not do today; creates a second source of truth (allocator memo + observation row); post-restart hydration must seed the observation table from the allocator — chicken-and-egg | **REJECTED** |
| (C) The allocator's own persisted memo IS the source of truth — no separate aggregate, no observation row (CHOSEN) | `IntentBackedAllocator<ServiceVip>` already persists `(spec_digest → ServiceVip)` mappings via the `allocator_entries` redb table per § 1a. Submit echo and `alloc status --service <id>` consult `ServiceVipAllocator::get(&spec_digest) -> Option<ServiceVip>` at render time. `Job`/`ServiceSpec` stays purely operator-input; the aggregate cannot represent or reference the assigned VIP at all (type-driven-design discipline preserved). Restart hydration is already covered by `IntentBackedAllocator::bulk_load` + probe (§ 8). | One additional read at render time per Service-kind alloc-status row — cheap (O(log N) BTreeMap lookup, no I/O). | **CHOSEN** |

**Downstream consumers of the assigned VIP** (e.g.
`ServiceMapHydrator` per ADR-0042 — the kernel-side dataplane
service-map writer) consult the allocator via `ServiceVipAllocator::get(&spec_digest)`
keyed by the Service's spec_digest at the relevant hydration step.
This is signature-compatible with the prior path (read the VIP from
the spec at hydrate time) but the source-of-truth shifts: from
"the spec's `listener.vip`" to "the allocator's
`get(&spec_digest)`." ADR-0042's contract unchanged; the
hydrator's input changes from "spec-with-vip" to "spec + allocator
handle."

**Why this is type-driven design as intended**: with both
(a) the `vip` field removed from `Listener` and (b) no separate
"assigned_vip" field on the aggregate, the operator-spec data shape
cannot represent the assigned VIP at all. The allocator memo is
the *only* persisted record; the type system structurally enforces
"the assigned VIP is an allocator-owned fact, not a spec-owned
fact." This is the upstream of `.claude/rules/development.md` §
"Persist inputs, not derived state": the spec carries inputs (the
operator-supplied listener `(port, protocol)` tuples); the assigned
VIP is derived from those inputs + the allocator's pool policy and
is owned by the allocator.

### 6. Reclamation — `WorkloadLifecycle` reconciler emits `Action::ReleaseVip`

> **SUPERSEDED 2026-06-28 — the *release trigger* below (release on
> observed terminal state) is reversed.** The VIP is now released on
> **logical-workload deletion (intent withdrawal)**, NOT on a terminal
> alloc. A stopped-or-crashed-but-still-declared Service **retains** its
> VIP, symmetric with the dial-by-name frontend `F`. The reconciler
> primitive, the `Action::ReleaseServiceVip` variant, the action-shim
> wiring, and the View-records-past-emission discipline are all
> **unchanged** — only the *gating condition* moves from
> `actual.allocations.values().any(|row| row.terminal.is_some())` to
> `desired.job.is_none()` (intent absent). See § Amendments → 2026-06-28
> ("withhold-not-release VIP reclamation; symmetric with `F`") for the
> full reversal, the evidence base (#251 / RCA / K8s+Nomad research),
> and the pinned new gating condition the crafter implements. The
> original §6 text below is retained verbatim as historical context.

On terminal-state transition of a Service workload, the VIP is
released back to the pool. The reclamation primitive is a
reconciler-emitted action, not an action-shim hook on
`StopAllocation`.

| Shape | Verdict |
|---|---|
| (a) `WorkloadLifecycle` reconciler emits `Action::ReleaseVip { spec_digest }` on observed terminal state (chosen) | Idempotent on retry (release of already-released is no-op); convergence-shaped ("every terminated Service has no VIP allocation"); single source of truth |
| (b) Action-shim hook on `StopAllocation` action | Couples reclamation to a specific stop path; misses crash-terminal transitions; reconcilers are the §18 convergence primitive — bypassing them is the wrong default |
| (c) Explicit `service_stop` workflow | Workflows orchestrate sequences; reclamation is single-step convergence. Wrong primitive |

**New Action variant** in `overdrive-core::reconciler`:

```rust
pub enum Action {
    // ... existing variants ...
    /// Release a previously-allocated Service VIP back to the
    /// allocator pool. Idempotent — releasing an already-released
    /// key is a no-op. Emitted by `WorkloadLifecycle` (per ADR-0013)
    /// on observed terminal-state transition.
    ReleaseServiceVip {
        spec_digest: ServiceSpecDigest,
        correlation: CorrelationKey,
    },
}
```

**Action-shim wiring**: a new arm in
`overdrive-control-plane::reconciler_runtime::action_shim` per
ADR-0023 dispatches `Action::ReleaseServiceVip { spec_digest, .. }`
to `PersistentServiceVipAllocator::release(&spec_digest)`. The
allocator release is idempotent and write-through (fsync-then-memory
same as allocate).

**`WorkloadLifecycle` View extension**: the reconciler tracks which
VIPs it has emitted release actions for, so it does not re-emit on
every tick after the terminal observation. Per
`.claude/rules/development.md` § "Persist inputs, not derived state",
the View carries the *inputs* (alloc terminal-state-observed-at
timestamp) and the reconcile body computes "should I emit
ReleaseServiceVip?" from those inputs + the observed alloc state.
The View gains a `released_for_terminal: BTreeSet<ServiceSpecDigest>`
field to mark "release action already emitted, do not duplicate"
(this is an input, not a derived deadline — the set IS the record
of past emission, the policy stays elsewhere).

**KPI K3 alignment**: the release fires on the same reconciler tick
that observes the terminal-state row. p99 ≤ 5 s lag is structural —
the tick cadence is 100 ms per ADR-0023, so the worst case is one
tick (≈ 100 ms) + action-shim dispatch + write-through fsync. Well
within KPI bounds.

### 7. Single-cut migration of existing `BackendIdAllocator`

Per `feedback_single_cut_greenfield_migrations.md`: no deprecation
shim, no parallel paths. The single PR that lands this feature also
moves the existing allocator. Per the 2026-05-14 amendment, the
`BackendIdAllocator` migration is a **structural move only**, not a
refactor — the file relocates; the struct body is untouched:

```
DELETE: crates/overdrive-dataplane/src/allocator.rs        (existing file)
CREATE: crates/overdrive-dataplane/src/allocators/mod.rs
CREATE: crates/overdrive-dataplane/src/allocators/error.rs
CREATE: crates/overdrive-dataplane/src/allocators/vip_range.rs
CREATE: crates/overdrive-dataplane/src/allocators/backend_id.rs    (relocated, body unchanged)
CREATE: crates/overdrive-dataplane/src/allocators/service_vip.rs
UPDATE: crates/overdrive-dataplane/src/lib.rs (mod declaration)
UPDATE: every call site of BackendIdAllocator (path change only — API stable)
```

`BackendIdAllocator`'s public API (`new()`, `allocate(ip, port,
proto)`, `release(id)`, `memo_len()`) and its internal representation
are **unchanged**. The existing tests (proptest, collision-witness)
move with the file. R1 from DISCUSS § Risks (hot-path coverage) is
preserved by this stability.

No generic `PoolAllocator<T>` wrapping step exists; the prior framing
of "BackendIdAllocator wraps `PoolAllocator<BackendId>`" was removed
in the 2026-05-14 amendment when the `Token` trait and generic core
were rejected (see § Considered alternatives).

### 8. Earned Trust — `probe()` on the allocator at boot

Per the project's load-bearing principle: every dependency must
demonstrate it can honor its contract. The
`PersistentServiceVipAllocator` specifies a `probe()` method that
runs at composition-root time and verifies:

1. The `IntentStore` is reachable and supports the
   `allocator_entries` table (a known-good throwaway key
   round-trips: write → read → equal → delete).
2. The configured `VipRange` is non-empty
   (`range.capacity() > 0`).
3. The bulk-loaded state is internally consistent (every persisted
   `(key, token)` projects back to a token within `range` — defends
   against config drift where the configured CIDR shrinks below a
   previously-allocated VIP).

Failures are typed `AllocatorBootError` variants and surface as
structured `health.startup.refused` events per ADR-0048's intent-layer
unknown-handling discipline. The control plane refuses to start.

The probe is enforced by the same three-layer discipline ADR-0048
already mandates: subtype check (the `probe()` method is on the
allocator type), structural check (an `xtask::dst_lint` AST scanner
walks every `PersistentServiceVipAllocator` construction site and
asserts `probe()` is called before first `allocate()` / `release()`),
behavioral check (a CI gold-test that configures a
CIDR-too-small-for-persisted-state fixture and asserts the probe
refuses to start).

## Considered alternatives (ADR-level — additional to the per-question shapes above)

### Alt-0 — Generic `PoolAllocator<T: Token>` core + `IntentBackedAllocator<T>` shim (rejected during DELIVER step 01-01, 2026-05-14)

The original DESIGN-wave decision (this ADR's pre-amendment §1)
proposed a two-layer factoring with a pure generic core
`PoolAllocator<T: Token>` and a generic persistence shim
`IntentBackedAllocator<T>`. The `Token` trait abstracted "the Nth
thing in a sequence" with associated types `Key` and `Range`, and
both `BackendIdAllocator` and `ServiceVipAllocator` would have been
type aliases over the generic core.

**Rejected during DELIVER step 01-01 (2026-05-14).** When the
crafter implemented the generic, the resulting design baked
`VipRange` (a CIDR + reserved set) into the generic core via
`T::Range`. `BackendIdAllocator` has no concept of CIDR ranges —
its "range" is a `(start: u32, max: u32)` counter envelope, an
entirely different shape from a CIDR + reserved set. To satisfy
both consumers, the generic `T::Range` had to either:

- Become a sum type (`Range::Counter(u32, u32) | Range::Cidr(VipRange)`)
  — which collapses to "two concrete allocators with a union type
  in the middle," gaining nothing over two concrete allocators;
- Or stay specific to one consumer (the implementation picked CIDR),
  forcing `BackendIdAllocator` into a generic that doesn't fit it.

The actually-shared logic across the two allocators is **thinner
than the abstraction required to factor it**: memo + monotonic
counter + memo-hit-returns-existing. That shape can be described
in a sentence and matched between two concrete types without a
shared trait. The `Token` trait was overstating the abstraction.

The 2026-05-14 amendment **deletes** the generic core, the `Token`
trait, the `IntentBackedAllocator<T>` generic shim, and the
"two-layer allocator primitive" framing's generic implementation.
The replacement is two concrete allocators that follow the same
memo + monotonic-counter shape but share no trait or type. AC-05's
"the underlying allocator logic is shared" is **honest as
shape-similarity**, not as literal code reuse. See § 1 and the
Consequences amendment.

### Alt-E — Ship a pinned default `ranges` instead of refusing to boot (accepted 2026-05-15)

The original § 3 stance treated a missing `[dataplane.vip_allocator]`
section as `VipAllocatorConfigError::Missing` and refused to boot.
The ecosystem precedent points the other way: Kubernetes
(`--service-cluster-ip-range` defaults to `10.96.0.0/12`), Cilium
(`clusterPoolIPv4PodCIDRList` defaults to `10.0.0.0/8`), MetalLB
(documentation-pinned reserved-block guidance), KubeVirt / kube-vip
/ Calico-CNI all ship a pinned default and emit operator-visible
guidance when it is in use. See
`docs/research/orchestration/service-vip-range-config-patterns.md`
for the full comparative.

**Accepted 2026-05-15.** A Phase 1 single-node Overdrive boot with
no operator-supplied VIP config now defaults to
`ranges = ["10.96.0.0/16"]` and
`reserved = ["10.96.0.0", "10.96.0.1", "10.96.255.255"]`. The boot
path emits a structured `health.startup.warn` event recording that
the default is in use; under HA mode (Phase 2+) the warn is emitted
on every startup so the operator never "loses sight" of the implicit
choice. The CLI renders the default's effective shape ahead of the
first admission so the first VIP allocation is not a surprise.

The trade-off is explicit: the strict "refuse-to-start until
operator commits" stance maximises one bit of Earned Trust
honesty — the operator must consciously enumerate the pool — at
the cost of every single-node bring-up failing the first boot with
no allocator config. The accepted shape preserves the visibility
property through `health.startup.warn` + CLI rendering rather than
through boot refusal; the operator who wants the strict stance can
still pin `ranges` explicitly, which suppresses the warn. Malformed
or out-of-range config still refuses to start with a typed error;
only the missing-section path softens.

`VipAllocatorConfigError::Missing` is removed from the typed error
surface (was never a returnable variant under the new shape).
`AllocatorError::Exhausted` is unchanged — pool exhaustion still
surfaces as a synchronous typed admission rejection per § 4.

### Alt-A — Allocator in `overdrive-control-plane` instead of `overdrive-dataplane`

The allocator is consumed by the admission handler (control-plane)
and emits VIPs that the dataplane consumes via `update_service`.
Either crate could host it.

**Rejected.** Per DISCUSS D3 (user direction 2026-05-14): the
existing `BackendIdAllocator` lives in `overdrive-dataplane`, and
the shared primitive that subsumes it must live there too. Moving
the primitive to `overdrive-control-plane` would split the allocator
abstraction across two crates and defeat AC-05. The control-plane
holds an `Arc<ServiceVipAllocator>` injected at composition root —
it does not own the primitive.

### Alt-B — Persist allocator state in the runtime-owned `ViewStore` instead of `IntentStore`

The reconciler runtime's `ViewStore` (ADR-0035 / ADR-0036) is a
CBOR-encoded per-reconciler memory store. It could host the
allocator's persistent state.

**Rejected.** The allocator is not a reconciler (no `desired vs
actual` convergence loop; it has a request/response API
`allocate(key) → token`). The `ViewStore` is keyed by
`(reconciler_name, target_resource)` — the shape does not fit a
flat key-value mapping. The `IntentStore` is the linearizable-state
SSOT (whitepaper §4); allocator state IS intent ("this VIP is
assigned to this spec digest") and belongs there.

### Alt-C — Allocate VIPs in the reconciler-tick `ServiceMapHydrator` instead of admission

Move the allocation into the hydrator: on first observed
`WorkloadSpec::Service` with `vip = None`, the hydrator allocates
and emits `Action::AssignVip` to mutate intent.

**Rejected.** Reconcilers do not mutate intent — they emit actions
that the action-shim dispatches; intent writes go through Raft per
ADR-0035. The submit-time path is simpler: one write, one fsync,
one source of truth. KPI K2 (allocator latency p50 ≤ 5 ms) is
trivially met by the in-memory `PoolAllocator` + single redb
write-through; the reconciler-tick path adds 100 ms + an extra
Action round-trip for no gain.

### Alt-D — Counter recycling

When a VIP is released, recycle its counter slot so the next
allocation reuses the address.

**Partially accepted 2026-05-19.** The basic form — "released VIPs
return to the available pool and may be re-allocated on subsequent
`allocate(&different_digest)` calls" — lands under the 2026-05-19
amendment. The implementation is "scan `range.nth(i)` ascending for
the first non-held address," not a free list (free list would be
derived state per `.claude/rules/development.md` § "Persist inputs,
not derived state" and would force a new envelope shape). The
`next` monotonic counter is removed entirely.

The originally-deferred form — **LRU / age-based recycling policy** —
remains deferred. A future Phase 3+ multi-tenant deployment with
high VIP churn AND latency-sensitive operators MAY benefit from
preferring oldest-released addresses (so a flapping workload doesn't
see its old VIP reassigned to a different workload within seconds
of release); the basic form above does not provide that guarantee
(it returns the lowest-indexed non-held address, which IS the most-
recently-released address if churn is at the high end of the range
and re-allocation rate matches release rate). LRU lands as an
additive amendment if it surfaces as an operability concern; for
Phase 1 single-node, basic reuse is sufficient.

**RCA for the change** *(why the 2026-05-14 "no reuse" call was
wrong)*: the original "Non-reuse on release" rule was copy-pasted
from `BackendIdAllocator`'s pre-existing semantics without
distinguishing the cardinality difference between the two
allocators. `BackendId` has an effectively-unbounded internal
identifier space (u64 / `i64`-shaped); monotonicity is correct.
`ServiceVip` is bounded by IPv4 CIDR — `/16` is 65 K addresses,
`/24` is 254, `/32` is 1 — and a monotonic-only allocator exhausts
the pool after `capacity` total ever-allocated regardless of
current liveness. The failure mode surfaced at DELIVER step 03-03
when the S-VIP-07 acceptance test (released VIP reusable on next
allocation, pool of 1) failed RED with
`Exhausted { allocated: 0, capacity: 1 }`. Every comparable
Service-VIP allocator in the ecosystem (Kubernetes ClusterIP
allocator, Cilium IPAM, MetalLB, kube-vip) reuses released
addresses; the "no reuse" stance was the outlier.

## Consequences

### Positive

1. **AC-05 satisfied as shape-similarity, not literal code reuse**
   (amended 2026-05-14; release-policy divergence amended
   2026-05-19). `BackendIdAllocator` and `ServiceVipAllocator`
   share the memo + memo-hit-returns-existing shape and live
   side-by-side in `crates/overdrive-dataplane/src/allocators/` per
   DISCUSS D3. They share no trait and no generic type — the
   previously-proposed `PoolAllocator<T: Token>` core was rejected at
   DELIVER step 01-01 as overstated abstraction (see § Considered
   alternatives → Alt-0). They diverge on release policy:
   `BackendIdAllocator` keeps monotonic-counter no-reuse semantics
   (correct for its unbounded internal identifier space);
   `ServiceVipAllocator` reuses VIPs on release with the `next`
   counter removed (correct for its finite IPv4 address space; see
   § Amendments → 2026-05-19).
2. **AC-02 structurally satisfied.** The persistence shim's
   write-through guarantees survives-restart by construction; the
   bulk-load probe guarantees consistency on boot.
3. **AC-01 / AC-04 / AC-06 surface as clean typed errors at
   admission time.** Single failure surface; no reconciler-tick
   races; pool exhaustion is a 503 not an `alloc status` ghost.
4. **AC-03 reclamation rides the existing reconciler primitive.**
   No new orchestration surface; the action-shim dispatches the
   release; convergence is structural.
5. **ServiceVip newtype duplication is resolved in the same commit.**
   One canonical `ServiceVip` at `overdrive-core::id::ServiceVip`
   wrapping `Ipv4Addr`. Greenfield single-cut.
6. **Forward-compatible with IPv6 VIPs (GH #61).** A future
   `ServiceVip6` newtype adds a second `Token` impl; `VipRange`
   remains v4-only; a parallel `Ipv6VipRange` lives alongside.
7. **DST-replayable.** The pure `PoolAllocator<T>` is a sync,
   I/O-free type; tests against it need no Sim adapters. The
   persistence shim is wired with `Arc<dyn IntentStore>` per
   `.claude/rules/development.md` § "Port-trait dependencies", so
   DST runs against `LocalStore` reused as sim per the existing
   project pattern.
8. **`BackendIdAllocator` test coverage preserved.** The
   proptest and the deterministic collision witness move with the
   file under `allocators/backend_id.rs`; no test surface area
   shrinks (per `feedback_delete_dont_gate.md`, this is a
   structural move, not a deletion).

### Negative

1. **`ServiceVip` consolidation touches both `id.rs` and
   `workload_spec.rs`.** Bounded: two declarations + their use
   sites; mechanical edit. Greenfield single-cut.
2. **New ObservationStore / IntentStore table.** One redb table
   `allocator_entries` keyed by `(namespace, key_digest)`. Schema
   evolution per ADR-0048 envelope discipline; one golden-bytes
   fixture per envelope version.
3. **Two new error enums** — `AllocatorError`, `AllocatorBootError`,
   `VipAllocatorConfigError`. Each follows the project's typed
   error discipline (`thiserror`, `#[from]` pass-through). Bounded.
4. **Boot-time probe adds dependency on IntentStore being ready
   before admission opens.** This is already the project's
   composition-root invariant ("wire then probe then use") per
   the Earned Trust principle; not a new constraint.
5. **Operator config gains a required section.** Boot fails with a
   typed error if `[dataplane.vip_allocator]` is missing — there
   is no default. Boot-time signal is honest; no silent
   "allocator works without config" failure mode.
   *(Superseded 2026-05-15 — see § Amendments → 2026-05-15. The
   missing-section path now defaults to `10.96.0.0/16` with the
   `[10.96.0.0, 10.96.0.1, 10.96.255.255]` reserved set and emits
   `health.startup.warn` (every startup under HA mode). Malformed
   or out-of-range config still refuses to start with a typed
   error; only the missing-section path softens.)*

### Quality attribute trade-offs (ISO 25010)

| Attribute | Impact | Direction |
|---|---|---|
| Functional correctness | Single canonical VIP source, idempotent on digest, structurally untrue invariants made unrepresentable | + |
| Maintainability | Two-layer factoring isolates persistence concern from allocation policy | + |
| Testability | Pure-core testable without IntentStore; shim testable with real `LocalStore`; DST-replayable | + |
| Performance | KPI K2 ≤ 5 ms p50 met by in-memory allocator + single fsync; no per-tick polling | + |
| Reliability | Survives-restart by construction; Earned Trust probe refuses unhealthy boot | + |
| Security | No new attack surface; allocator state is internal to single-node Phase 1 | 0 |
| Operability | Required config = honest config; pool-exhaustion → typed 503; KPI K4 instrumentable | + |
| Backward compatibility | Single-cut migration of existing `BackendIdAllocator`; greenfield | − (bounded) |

## Implementation note

This ADR resolves all five DESIGN-wave open questions. Slice
ordering for DELIVER is **out of scope per the DESIGN wave's
constraints** (per CLAUDE.md "Roadmap creation belongs exclusively
to DELIVER wave"). The crafter dispatched against this ADR receives
the full ADR + DISCUSS artifacts + `wave-decisions.md` + brief.md
extension as input.

**Expected `ServiceSpecDigest` implementation choice** — the
`ServiceSpecDigest` newtype identified in the Reuse Analysis table
(`docs/feature/service-vip-allocator/design/wave-decisions.md`) is
either a direct alias (`pub type ServiceSpecDigest = ContentHash;`
where `ContentHash` is `overdrive-core::id::ContentHash`) or a
dedicated newtype wrapping `[u8; 32]` with identical wire semantics
to `ContentHash`. Both shapes are acceptable; the crafter chooses
whichever reads more idiomatically at the point of implementation
given the surrounding consumer call sites. The load-bearing property
is wire-format coherence: the digest used as the allocator memo key
MUST equal byte-for-byte the digest computed at render time by
`ServiceVipAllocator::get(&spec_digest)`. Both code paths consult
the same `Job::spec_digest` codec entry point per ADR-0048 § 4b.

## Cross-references

- GH #167 — SSOT for the feature
- ADR-0046 — `BackendIdAllocator` structural precedent (extended,
  not superseded)
- ADR-0047 — `ListenerRow.vip: Option<ServiceVip>` field shape
  (**amended 2026-05-14**: the `vip` field is removed at the parser
  layer per § 5; spec back-propagation tracked in
  `upstream-changes.md`)
- ADR-0042 — `ServiceMapHydrator` reconciler (allocator output is
  consumed via `update_service`'s `vip` parameter)
- ADR-0048 — rkyv versioned envelope (persistence wire format per § 1a)
- ADR-0019 — operator config TOML (§ 3 places `[dataplane.vip_allocator]`)
- ADR-0035 / ADR-0036 — reconciler runtime (write-through ordering
  matches § 1)
- ADR-0013 — reconciler primitive (`WorkloadLifecycle` reclamation
  per § 6)
- `docs/feature/service-vip-allocator/discuss/` — DISCUSS artifacts
- `docs/feature/service-vip-allocator/design/wave-decisions.md` —
  DESIGN-wave decisions

## Amendments

### 2026-05-14 — Generic `PoolAllocator<T: Token>` rejected; two concrete allocators land

During DELIVER step 01-01, the crafter implemented the originally-
designed two-layer factoring (generic `PoolAllocator<T: Token>` core
+ `IntentBackedAllocator<T>` persistence shim) and discovered the
abstraction was overstated: the shared logic between
`BackendIdAllocator` and `ServiceVipAllocator` is only memo +
monotonic counter + memo-hit-returns-existing, while the `T::Range`
slot required to factor the two consumers generically bakes
`VipRange` (a CIDR-shaped concept) into a core that `BackendIdAllocator`
has no use for. Trying to factor a thinner shared shape behind a
heavier trait surface produced the wrong abstraction.

**Resolution (now in code, 6/6 tests passing in Lima as of
2026-05-14):**

- Deleted: `Token` trait, `PoolAllocator<T, K>` generic core,
  `IntentBackedAllocator<T>` generic shim, "two-layer allocator
  primitive" framing's generic implementation.
- Moved: existing `crates/overdrive-dataplane/src/allocator.rs` →
  `crates/overdrive-dataplane/src/allocators/backend_id.rs`
  (untouched internally — same `BackendIdAllocator` struct, just
  relocated).
- Added: `crates/overdrive-dataplane/src/allocators/service_vip.rs`
  — concrete `ServiceVipAllocator` struct (NOT generic), keyed by
  `ServiceSpecDigest`. Memo + monotonic counter. NO slot reuse on
  release (matches `BackendIdAllocator`'s pre-existing semantics).
- Kept: `VipRange` (now consumed only by `ServiceVipAllocator`),
  `VipAllocatorConfigError` (unchanged).
- Renamed: `PoolError` → `ServiceVipAllocatorError` (single variant
  `Exhausted { allocated, capacity }`).
- Persistence shim: `PersistentServiceVipAllocator` (concrete, not
  generic) wraps `ServiceVipAllocator` with redb write-through and
  bulk-load.

Sections rewritten: § 1, § 1a, § 2, § 7, § 8 (allocator-type
references), § Considered alternatives (new Alt-0), § Consequences
→ Positive #1. Roadmap step 01-04 ("BackendIdAllocator single-cut
migration") was absorbed into step 01-01 (the relocation is forced
by the deletion of `PoolAllocator`); roadmap `total_steps` updated
from 11 → 10.

### 2026-05-15 — `[dataplane.vip_allocator]` defaults walked back

The DESIGN-wave § 3 "no defaults" stance was investigated during
DELIVER step 02-03c and found to be out of step with the operator
expectation set by the surrounding orchestrator ecosystem. Comparative
analysis in `docs/research/orchestration/service-vip-range-config-patterns.md`
(Kubernetes, Cilium, MetalLB, KubeVirt, kube-vip, Calico-CNI) shows
every neighbour ships a pinned default VIP range and surfaces
operator-visible guidance when it is in use; Phase 1 single-node
Overdrive declining to boot at all is the outlier. The "refuse-to-start
until operator commits" framing preserved one bit of Earned Trust
honesty (the operator must consciously enumerate the pool) at the
cost of every single-node bring-up failing the first boot with no
allocator config.

**Resolution (DESIGN-wave amendment, no production code touched
under this amendment — landing belongs to DELIVER step 02-03c):**

- Default `ranges = ["10.96.0.0/16"]` with reserved set
  `["10.96.0.0", "10.96.0.1", "10.96.255.255"]` activates when
  `[dataplane.vip_allocator]` is absent.
- Boot emits a single structured `health.startup.warn` event
  recording that the default is in use. Under HA mode (Phase 2+)
  the warn is emitted on every startup so the operator never loses
  sight of the implicit choice.
- CLI renders the default's effective shape ahead of the first
  admission so the first VIP allocation is not a surprise.
- Malformed or out-of-range operator-supplied config still refuses
  to start with a typed `VipAllocatorConfigError` variant. Only the
  missing-section path softens.

Three explicit acknowledgments:

1. **§ 3 walk-back.** The "No defaults: operators MUST supply
   `ranges`" stance is superseded. The missing-section path no
   longer maps to `VipAllocatorConfigError::Missing` /
   `health.startup.refused`; it maps to the warn-plus-default boot
   shape above.
2. **§ Consequences → Negative #5 supersession.** The negative
   consequence "Operator config gains a required section. Boot
   fails with a typed error if `[dataplane.vip_allocator]` is
   missing — there is no default." is superseded by the
   warn-plus-default path. The supersession is marked inline under
   § Consequences → Negative; the original line stays as historical
   context.
3. **`AllocatorError::Exhausted` unchanged.** Pool exhaustion
   continues to surface as a synchronous typed admission rejection
   (HTTP 503 + `ProblemDetails`) per § 4. The default-range
   softening does not change the exhaustion contract: a /16 with
   three reserved addresses yields 65 533 allocatable VIPs, well
   above any plausible Phase 1 single-node service count, but the
   typed rejection still fires on overflow.

Sections rewritten: § 3 (inline annotation + walk-back paragraph),
§ Consequences → Negative #5 (supersession marker),
§ Considered alternatives (new Alt-E entry documenting the
ecosystem precedent and the accepted shape). § 4 admission
failure-mode table unchanged (`AllocatorError::Exhausted` stays as
documented).

Rebuttal to the "operator must consciously choose to avoid
surprises" framing: the visibility property is preserved through
`health.startup.warn` (every startup under HA) plus CLI rendering of
the effective default range ahead of first admission, not through
boot refusal. The operator who wants the strict stance pins
`ranges` explicitly, which suppresses the warn — the strict path
is one TOML line away, not deleted.

Cross-reference:
`docs/research/orchestration/service-vip-range-config-patterns.md`.

### 2026-05-19 — `ServiceVipAllocator` VIP reuse on release (counter removed)

During DELIVER step 03-03 (end-to-end S-VIP-06 + S-VIP-07
acceptance tests on the `submit_workload` handler) the test for
S-VIP-07 ("released VIP reusable on next allocation, pool of 1")
failed RED with `Exhausted { allocated: 0, capacity: 1 }`. The
implementation followed ADR-0049's "Non-reuse on release" rule from
the 2026-05-14 amendment; the test expected the inverse.

The contradiction was real: DISTILL S-VIP-07 specifies VIP reuse
and the DELIVER step 03-03 roadmap acceptance criterion specifies
VIP reuse, but ADR-0049 § 1 + DISTILL S-VIP-12 specified non-reuse.

**RCA.** The 2026-05-14 "Non-reuse on release" rule was copied from
`BackendIdAllocator`'s shape without distinguishing the cardinality
difference between the two allocators:

- `BackendId` is an internally-allocated identifier in an
  effectively-unbounded namespace (u64 / `i64`-shaped). Monotonic
  counter is correct — matches Snowflake / event-source ID
  precedent; no exhaustion concern.
- `ServiceVip` is a finite IPv4 address within a configured CIDR.
  `VipRange::default() = 10.96.0.0/16` is 65 K addresses; a `/24`
  test fixture is 254; the S-VIP-07 fixture is `/32` (1). A
  monotonic-only allocator exhausts the pool after `capacity` total
  ever-allocated regardless of how many are currently held, and
  "restart to recover" is not an operability story.

Every comparable Service-VIP allocator in the ecosystem
(Kubernetes ClusterIP, Cilium IPAM, MetalLB, kube-vip) reuses
released addresses. The "shape-equivalence with `BackendIdAllocator`"
framing from the 2026-05-14 amendment was load-bearing only when
AC-05 still implied literal code reuse; with AC-05 already restated
as shape-similarity (not code reuse) and no shared trait between
the two types, divergence on release policy costs nothing
structural.

**Resolution.** Released VIPs return to the available pool and
become eligible for re-allocation by subsequent
`allocate(&different_digest)` calls within the same process
lifetime. Concretely:

1. **`ServiceVipAllocator::release(&digest)` semantics inverted.**
   Was: "Removes the memo entry. Does NOT return the slot to the
   pool — the monotonic counter never rewinds."
   Now: "Removes the memo entry. The freed VIP becomes available
   for re-allocation to a subsequent `allocate(&different_digest)`
   call. Idempotent on already-released keys."

2. **Address-selection mechanism = recompute-on-allocate (scan over
   `range`).** On memo-miss, `allocate` scans
   `range.nth(0)..range.nth(capacity-1)` ascending and returns the
   first non-reserved address NOT currently held in
   `by_digest.values()`. Returns `Exhausted { allocated, capacity }`
   when no such address exists. The scan order is deterministic —
   same `range` + same `memo` always selects the same next VIP, so
   DST/proptest reproducibility (K3 of `testing.md` § DST) is
   preserved without any tie-breaker logic.

3. **Free list NOT introduced.** A free list would be derived
   state recomputable at any moment from `(range, memo)`. Per
   `.claude/rules/development.md` § "Persist inputs, not derived
   state", deriving on every `allocate` is the right shape;
   persisting a free list would force a second persistence shape
   and a second `bulk_load` invariant for no operability benefit.
   If a future Phase 3+ deployment surfaces scan-cost pressure (a
   `/12` pool or larger with high churn), an in-memory `free:
   VecDeque<ServiceVip>` cache MAY be added — recomputed from memo
   on `bulk_load`, never persisted; that's an additive optimization,
   not a structural change.

4. **Counter field removed entirely.** The `next: u32` field on
   `ServiceVipAllocator` is removed (it carried no behavior under
   the scan shape). The `counter: u32` field on
   `ServiceVipAllocatorEntryV1` is removed by minting
   `ServiceVipAllocatorEntryV2 { spec_digest, vip }` per ADR-0048
   § "Version-bump procedure". The V1 envelope variant is retained
   (per the rule "existing fixtures are NEVER touched"); the V1 →
   V2 conversion drops `counter` structurally; V1 golden-bytes
   fixture stays, V2 golden-bytes fixture lands in the same commit.

5. **No-duplicate-tokens invariant unchanged.** "No two
   simultaneously-held memo entries share a VIP" is preserved by
   construction — the scan refuses any address already in
   `by_digest.values()`. S-VIP-P03 (DISTILL property test) inverts
   from "released slot is NOT reused (monotonic counter)" to "the
   no-duplicate-among-simultaneously-held invariant holds under any
   sequence of allocate/release calls (including release-then-
   reallocate-to-different-digest)".

6. **`BackendIdAllocator` unchanged.** Its monotonic-counter shape
   is correct for its unbounded internal identifier space and
   matches ADR-0046's precedent.

**Sections rewritten:** § 1 (allocator shape, `release` semantics
paragraph), § 1a (envelope V1 → V2 schema bump), § 2 (`VipRange::nth`
rustdoc), § Considered alternatives → Alt-D (basic reuse accepted;
LRU still deferred), this amendment section.

**No production code touched under this amendment** — landing
belongs to the resumed DELIVER step 03-03 crafter dispatch.

### 2026-05-19 — Considered alternative: keep monotonic counter, no reuse

The 2026-05-14 amendment's "Non-reuse on release" rule was
proposed and stood for five days before being amended out. The
rejection rationale is captured here as the counter-balance to
Alt-E's 2026-05-15 acceptance of ecosystem-default behavior.

**Rejected 2026-05-19.** Monotonic counter with no slot reuse on
release is the correct shape for `BackendIdAllocator` (unbounded
internal identifier space, matches Snowflake/UUID/event-source
precedent) but the wrong shape for `ServiceVipAllocator` (finite
IPv4 address space, exhausts after `capacity` total submissions
regardless of current liveness, no restart-to-recover story).

The 2026-05-14 framing rested on "shape-equivalence with
`BackendIdAllocator`" preserving AC-05's "shared allocator logic"
story, but AC-05 was concurrently restated as shape-similarity (not
literal code reuse) — making shape-equivalence on release-policy a
non-load-bearing aesthetic preference. Every comparable
Service-VIP allocator in the ecosystem reuses released addresses;
the "no reuse" stance was the outlier with no operability win to
trade for it.

Cross-reference: § Amendments → 2026-05-19; DELIVER step 03-03
DISTILL S-VIP-07; `docs/feature/service-vip-allocator/distill/test-scenarios.md`
S-VIP-P03 (revised).

### 2026-06-28 — Withhold-not-release VIP reclamation; symmetric with the dial-by-name frontend `F`

**Supersedes the § 6 *release trigger*.** The original § 6 ratified
option **(a)** — `WorkloadLifecycle` emits `Action::ReleaseServiceVip`
on **observed terminal alloc state** ("every terminated Service has no
VIP allocation") — and explicitly **rejected (b)** an action-shim hook
on `StopAllocation` *because it "misses crash-terminal transitions."*
That framing optimised for the wrong invariant. This amendment reverses
the trigger: the VIP is released **only on logical-workload deletion
(intent withdrawal)**; a stopped-or-crashed-but-still-declared Service
**retains** its VIP. The everything-else of § 6 stays
(reconciler-emitted action, the `Action::ReleaseServiceVip` variant, the
action-shim dispatch to `PersistentServiceVipAllocator::release`, the
View records-past-emission discipline) — **only the gating condition
moves.**

#### Why the reversal is correct (addressing § 6's original rationale head-on)

§ 6 chose release-on-terminal *to avoid missing crash-terminal
transitions*. But "release the identity on crash/stop" is exactly the
behaviour that is wrong — it conflates two independent planes:

- **Identity** — the stable virtual IP a workload is *known by*
  (the service VIP, and the dial-by-name frontend `F`). Identity is a
  property of the *declared workload*, and survives a transient
  stop/crash for as long as the workload stays declared.
- **Reachability** — whether the name currently resolves to a *healthy
  backend*. Reachability is a property of the *running instances*, and
  collapses the moment they stop.

The prior-art research
(`docs/research/orchestration/service-vip-dns-lifecycle-stop-vs-delete-k8s-nomad.md`,
2026-06-28; Confidence: High) is the evidence base: **both** Kubernetes
(ClusterIP) **and** Nomad+Consul (mesh VIP) bind the stable VIP to the
*declared-object* lifetime — released only on **delete/purge** — and
gate *name resolution* on healthy backends as a **separate plane**.
K8s de-allocates a ClusterIP only on **Service deletion** (not Pod
churn, scale-to-zero, or zero ready endpoints — K1/K2); the Consul mesh
VIP is keyed to the logical service in the catalog, stable across
instance churn (N3); in both, what disappears on a transient stop is
**backend reachability** (empty EndpointSlices / NXDOMAIN-or-empty
service-DNS — K3/N2/N4), never the stable IP. Neither system exhibits
Overdrive's asymmetry. Against this convention, Overdrive's
**retain-`F`-on-stop matches** and its **release-VIP-on-stop is the
outlier** — so making the VIP symmetric with `F` removes an
identity-lifecycle asymmetry, it does not introduce one.

"Retain across crash/stop, release on delete" is therefore *more*
correct than § 6's "release on terminal," not a regression: it is the
two-plane model the ecosystem documents, and it is what `F` already
does (ADR-0072: `FrontendAddrAllocator::release` is **logical-workload-
DELETION ONLY**, never on an alloc cycle, never on a transient
zero-healthy window). § 6(b)'s "misses crash-terminal transitions"
concern is *answered, not overridden*: under withhold-not-release a
crash-terminal alloc is precisely a case where the VIP **should** be
retained (the workload is still declared), so "missing" the
crash-terminal release is the *correct* behaviour, not a defect.

#### The #251 coupling defect this also unblocks

`overdrive-sh/overdrive#251` (RCA: `docs/analysis/rca-251-withhold-on-stop.md`)
is the proximate trigger. After `job stop` converges a Service to
Terminated (intent **retained**, #249), dial-by-name keeps resolving the
stopped workload's stable `F` forever — never NXDOMAIN. RCA-251 pins the
mechanism (mechanism 3, confirmed by a live Lima population-diff probe):
on the same terminal stop, `WorkloadLifecycle` emits
`Action::ReleaseServiceVip`, whose executor evicts the VIP from the
allocator memo — and the `BackendDiscoveryBridge`'s desired-listener
hydrate (`hydrate_bridge_desired_listeners`) has an undocumented
data-dependency on that memo. The release **nulls the input the
zero-backend retraction needs**, so the retraction is never emitted and
the name resolves `F` indefinitely. It is a *race between two terminal
effects of one stop*, where one destroys the input the other needs. The
root cause is that #251 **couples** the identity plane (VIP) and the
reachability plane (name retraction). Retaining the VIP across stop
removes the coupling at its source: the memo the bridge reads is still
present, so the bridge can project the listener set and emit the
zero-backend retraction on the same stop — and reachability collapses
(NXDOMAIN) while identity (the VIP) is preserved.

---

#### Pinned decisions for the crafter (implement to the design; do not improvise)

**D1 — New VIP release trigger (the gating condition the crafter
implements).**

The release fires on **intent withdrawal (logical-workload deletion)**,
detected as `desired.job.is_none()` — NOT on a terminal alloc. The
`service_vip_release_emission` helper
(`crates/overdrive-core/src/reconcilers/workload_lifecycle.rs`, currently
≈ L891–909) changes its **terminal-observation gate** from:

```rust
// BEFORE (release-on-terminal — superseded):
let terminal_observed =
    actual.allocations.values().any(|row| row.terminal.is_some());
if !terminal_observed {
    return None;
}
```

to an **intent-absence gate**:

```rust
// AFTER (release-on-deletion — withhold-not-release):
// The VIP is an identity bound to the DECLARED workload. Release it
// only when the workload's intent is withdrawn — i.e. when the
// reconciler observes `desired.job.is_none()` (the same signal the
// Absent/GC branch in `reconcile_inner` keys on). A stopped-or-
// crashed-but-still-declared Service (`desired.job.is_some()`) RETAINS
// its VIP, symmetric with the dial-by-name frontend `F`
// (ADR-0072 `FrontendAddrAllocator::release` = deletion-only).
if desired.job.is_some() {
    return None;
}
```

Everything else in the helper is **unchanged**: the
`desired.workload_kind == WorkloadKind::Service` gate (line ≈896), the
`desired.service_spec_digest?` extraction (≈899), the
`view.released_for_terminal.contains(&digest)` idempotency
short-circuit (≈900), the `CorrelationKey::derive(...,
"release-service-vip")` construction (≈908), and the returned
`Action::ReleaseServiceVip { spec_digest, correlation }`. The
`released_for_terminal` View field keeps its name and its role (the set
IS the record "release already emitted for this digest"; it is an input
per `.claude/rules/development.md` § "Persist inputs, not derived
state"). The crafter MAY rename the field to a deletion-centric name
(e.g. `released_for_deletion`) for clarity, but is NOT required to — the
field's *semantics* ("digests we have already emitted release for, do
not re-emit") are identical; a rename is a cosmetic call, not a contract
change, and if chosen MUST land its View-schema implications (additive
serde / the `WorkloadLifecycleView` evolution) in the same commit.

**Caveat the crafter must respect — `desired.job` is the SSOT for
"declared", and `desired_to_stop` is NOT deletion.** A stop intent
(`POST /v1/jobs/{id}/stop`) writes a *separate* stop-intent key and
**retains** the original spec key, so under a stop `desired.job` stays
`Some(_)` and `desired_to_stop` is `true`. The new gate keys on
`desired.job.is_none()` *only* — it MUST NOT key on `desired_to_stop`,
on `is_operator_stopped`, on `row.terminal`, or on the GC terminal
stamp. Those all fire on stop-while-declared, which is exactly the case
that must now **retain** the VIP. The single correct deletion signal is
the absence of the spec intent (`desired.job.is_none()`) — the identical
signal the reconciler's own Absent/GC branch (`reconcile_inner`,
`match desired.job.as_ref() { None => … }`, ≈L433) already uses to
emit the `StoppedBy::SystemGc` terminal claim.

**D2 — Mirror `F`'s mechanism (one deletion-release lifecycle, two
allocators).**

`FrontendAddrAllocator::release(&MeshServiceName)`
(`crates/overdrive-control-plane/src/dns_responder/frontend_addr_allocator.rs:236`)
is documented **"logical-workload-DELETION ONLY"** and is keyed on the
logical `<job>` (`MeshServiceName`). The Service VIP now shares that
*lifecycle* (release-on-deletion-only) but **reaches it through a
different mechanism**, and the divergence is deliberate and load-bearing:

- `F` is released by a **direct allocator method call** from a deletion
  call site (when one exists — see D3). It is keyed on `MeshServiceName`
  and carries no reconciler/action surface.
- The Service VIP is released by the **reconciler → `Action::Release-
  ServiceVip` → action-shim → `PersistentServiceVipAllocator::release`**
  path (the § 6 primitive, **unchanged**). It is keyed on
  `ServiceSpecDigest` (the allocator memo key per § 1) and persists via
  the redb write-through shim (the VIP allocator is the *persistent* one;
  `F` is ephemeral/rebuilt-on-boot).

Why not collapse the VIP onto `F`'s direct-call mechanism: the VIP
allocator is persistent and reconciler-driven by § 6's ratified design;
the VIP key is the spec digest, not the `MeshServiceName`; and the
reconciler is the single source of every terminal/lifecycle claim per
ADR-0037 §4. Forcing the VIP release into a direct-call shape would
bypass the reconciler convergence primitive that § 6 correctly chose.
The **shared invariant** both allocators now honour — and the load-
bearing symmetry this amendment establishes — is the *trigger*: **both
release only on logical-workload deletion (intent withdrawal), neither
on a transient stop/crash while the workload stays declared.** They
share a lifecycle contract, not a code path.

**D3 — Deletion-path reality check (Phase 1 posture). Deletion is NOT
wired today.**

Investigation of the live tree (2026-06-28) establishes:

- **No operator-facing deletion verb exists.** The HTTP surface
  (`crates/overdrive-control-plane/src/handlers.rs`) is `POST /v1/jobs`
  (submit), `GET /v1/jobs/{id}` (describe), `POST /v1/jobs/{id}/stop`
  (stop — writes a stop-intent key, **retains** the spec key),
  `GET /v1/jobs` (list/status). There is **no `DELETE /v1/jobs/{id}`,
  no undeploy, no `stop --purge`**. The operator CLI (`overdrive deploy`
  / `overdrive job {list,stop}`) has no delete verb either.
- **The `IntentStore::delete` primitive exists**
  (`crates/overdrive-core/src/traits/intent_store.rs:193`) and the
  `WorkloadLifecycle` **Absent/GC branch already converges on it**: when
  intent is withdrawn (`desired.job.is_none()`), the reconciler GCs every
  Running alloc with `terminal: Some(Stopped { by: SystemGc })`
  (`reconcile_inner` ≈L433). But the only callers of `IntentStore::delete`
  on a `jobs/` key today are **test fault-injection** (e.g.
  `workload_gc_absent_intent` DST scenario 1, step 3) — no production
  call site withdraws a workload's spec intent.
- **`FrontendAddrAllocator::release` is itself never called from any
  production site.** Confirmed by grep: every `.release(...)` caller is
  either a test or the *different* `NetSlotAllocator` (keyed on
  `AllocationId`, released on alloc-terminal — a separate concern). So
  `F` is, in practice, **retained for the entire process lifetime
  today.**

**Accepted Phase-1 posture (honest consequence):** with no deletion path
wired, **a stopped/crashed Service retains its VIP for the process
lifetime — exactly as `F` is retained today.** This is acceptable for
Phase-1 single-node: the default pool is `10.96.0.0/16` (§ 3, 65 533
allocatable VIPs after the three reserved) — vast headroom relative to
any plausible single-node declared-Service count, and the allocator
re-hydrates its memo from persisted `allocator_entries` on restart
(§ 1a / § 8). The VIP release is pinned to **the same future deletion
trigger `F` will use**: when a deletion/undeploy verb lands (D5 below),
it will (i) withdraw the spec intent via `IntentStore::delete`, which
drives `desired.job.is_none()` and thus the reconciler's new release
emission *for free* (no new wiring on the VIP side), and (ii) call
`FrontendAddrAllocator::release(&job)` for the frontend. The crafter
does **not** invent a deletion verb; the VIP release is wired to the
*existing* `desired.job.is_none()` signal the Absent/GC branch already
produces, and that signal fires the moment any future deletion path
calls `IntentStore::delete`.

**Operational consequence (review-03-01 non-blocking suggestion, made
explicit).** Because release-on-deletion is **inert on the v1
convergence path** — the hydrator (`read_job`) zeroes
`service_spec_digest` alongside `desired.job` on intent withdrawal
(`reconciler_runtime.rs:2309`; `:1688-1689`), so the gate's
`desired.service_spec_digest?` short-circuits *before* the
`desired.job.is_none()` branch can fire — **a stopped/crashed-but-declared
Service permanently holds its `10.96.x` VIP for the entire process
lifetime; VIP retention is unbounded across stop/restart churn until
#211 wires a deletion verb that supplies the digest at hydrate time.**
This is acceptable for single-node Phase 1 (the pool is empty on boot —
§ 1a / § 8 re-hydrate from `allocator_entries` — and carries 65 533-VIP
headroom, § 3), but it is **not a surprise to budget for**: under the
restart-after-stop churn tracked in #249, every stop-without-redeploy
leaks one VIP for the rest of the process lifetime, and the leak is
only reclaimed by a process restart (which re-derives the pool from the
*current* declared set) — never by stop/restart alone — until #211
lands the deletion edge. The inert path is pinned by
`workload_lifecycle.rs::withdrawn_service_without_digest_emits_no_release`
and the inline #211 note at `vip_allocator_lifecycle.rs:733-748`.

**D4 — The decoupling principle, and what the crafter must do about the
bridge.**

The research's real lesson (S3 / caveat 3) is that **identity**
(VIP/`F`: retain-until-delete) and **reachability** (name resolution:
collapse-on-stop) are **two independent planes**, and #251 was a
*coupling* defect. The **target end-state is full decoupling**: name
resolution MUST collapse on stop (NXDOMAIN) **independent of** the VIP
lifecycle.

**Decision (pinned):** this amendment achieves full decoupling **by
retaining the VIP** — and that is *sufficient*; the crafter does **NOT**
also need to sever the bridge's name-retraction from the VIP memo as a
separate change. Rationale: RCA-251 establishes that the bridge's
zero-backend retraction *already works* whenever the VIP memo is present
(Run A of the population diff: with the memo present the retraction
lands and the name goes NXDOMAIN). The #251 failure is *only* that the
release **evicted** the memo mid-stop. Once the VIP is retained across
stop (D1), the memo the bridge reads (`hydrate_bridge_desired_listeners`
→ `allocator.get(&digest)`) is **present for the whole
stopped-but-declared window**, so the bridge projects the listener set,
the reconcile loop body runs, the zero-backend `ServiceBackendRow` is
written, the `name_index` folds it, and resolution collapses to NXDOMAIN
— while the VIP (identity) is retained. **Retaining the VIP keeps the
bridge's retraction working; that delivers the decoupled behaviour
without a second fix.** This is "Option C alone" from the research
framing, and it is sufficient *because the coupling was the release, not
a structural dependency the bridge cannot shed*.

The fuller structural decoupling — making
`hydrate_bridge_desired_listeners` project listeners *without* depending
on the live VIP memo at all (RCA-251 "Option A": fall back to the VIP on
the last-written `service_backends` row when the memo is absent-but-
declared) — is **defence-in-depth, not required by this amendment**, and
the crafter MUST NOT undertake it here unless surfacing it as separate
scope. Under D1, the memo is never absent for a still-declared workload,
so the fallback never fires; it would only matter if a *future* change
re-introduced mid-declared memo eviction. The principle is recorded so a
future reviewer knows the planes are *meant* to be independent; the
Phase-1 implementation realises that independence through VIP retention.

**D5 — Deliberate NXDOMAIN-on-empty (conscious divergence from the K8s
ClusterIP default).**

Record as a **conscious choice**, not an accident, that Overdrive's
dial-by-name responder returns **NXDOMAIN** on zero-healthy backends.
The research (K3) shows this is the **opt-in** posture: the Kubernetes
ClusterIP *default* keeps the name resolving to the stable VIP at zero
endpoints (NODATA-shaped per RFC 8020 — the name still "exists"); CoreDNS
`ignore empty_service` is the *opt-in* flag that flips it to NXDOMAIN.
Overdrive deliberately chooses the opt-in (NXDOMAIN) posture, and the
rationale is the workload-identity model: Overdrive workloads are
**identity-unaware** and dial by name over a plaintext socket (the agent
originates mTLS transparently). Unlike a K8s client dialing a ClusterIP
(which gets a stable VIP and a connection that simply has no backend to
land on — a clean connection-refused), an Overdrive dialer handed a
still-resolving `F` for a dead workload would have the agent originate an
mTLS leg to a backend that is gone — a silent, harder-to-diagnose
failure. **Failing the resolve (NXDOMAIN) is fail-honest**: the dialer
gets an unambiguous "this name has nothing behind it" at resolution time,
not a stalled connection at dial time. This is why Overdrive's
reachability plane collapses to NXDOMAIN where the K8s ClusterIP plane
would stay NODATA — a justified divergence, captured here so the contract
does not read as accidentally drifting from the K8s default. (Note the
*identity* plane — the VIP — is unaffected by this choice; D5 governs
only what the *name* resolves to, which is the reachability plane.)

**D6 — Tradeoff + reaffirmed invariants.**

*Tradeoff (recorded):* a stopped/crashed-but-still-declared Service now
holds its VIP until deletion (D3: process lifetime today, since deletion
is unwired). The cost is VIP-pool occupancy by stopped-not-deleted
Services. This is bounded by the `10.96.0.0/16` default pool's 65 533-VIP
headroom (§ 3) and matches the ecosystem's mitigation (K8s wide
ServiceCIDR, Consul `240.0.0.0/4`; research caveat 1). `AllocatorError::
Exhausted` is unchanged — exhaustion still surfaces as a synchronous
typed admission 503 per § 4; retain-until-delete does not change the
exhaustion contract, only shifts *when* a VIP returns to the pool.

*Invariants reaffirmed (untouched by this amendment):*

1. **`F` retention is unchanged.** The dial-by-name frontend `F` was
   already release-on-deletion-only (ADR-0072; Tier-1 gated at roadmap
   01-04). This amendment does NOT touch `FrontendAddrAllocator` — it
   brings the *VIP* into line with `F`, not the reverse.
2. **`FrontendAddrAllocator::release` is NOT called by this change.** No
   new caller of `F`'s release is added here; `F`'s release stays
   deletion-only and (today) unwired.
3. **VIP-reuse on a fresh `allocate(&digest)` still works** (§ 1 /
   2026-05-19 amendment). Released VIPs (released on *deletion*) return
   to the pool and are re-allocatable; the scan-over-range selection is
   unchanged. The no-duplicate-among-simultaneously-held invariant
   holds by construction.
4. **§ 6's reconciler primitive, `Action::ReleaseServiceVip`, action-shim
   dispatch, and write-through release are all unchanged.** Only the
   emission *gate* moves (terminal-observed → intent-absent).

---

#### Crafter-facing design spec (the exact contract)

| Concern | Pinned contract |
|---|---|
| **Release trigger** | `desired.job.is_none()` (intent withdrawn / logical deletion). NOT `row.terminal.is_some()`, NOT `desired_to_stop`, NOT any terminal/GC stamp. |
| **Site** | `service_vip_release_emission` in `crates/overdrive-core/src/reconcilers/workload_lifecycle.rs` (≈L891–909). Swap the terminal-observation gate for the intent-absence gate (D1). |
| **Unchanged in the helper** | `WorkloadKind::Service` gate; `service_spec_digest?`; `released_for_terminal` idempotency short-circuit; `CorrelationKey::derive(…, "release-service-vip")`; the returned `Action::ReleaseServiceVip { spec_digest, correlation }`. |
| **View field** | `released_for_terminal: BTreeSet<ContentHash>` keeps its role (records past emission, an input). Optional cosmetic rename to a deletion-centric name; if renamed, land the `WorkloadLifecycleView` schema implication in the same commit. NO behavioural change. |
| **Action / shim / allocator** | `Action::ReleaseServiceVip`, the action-shim arm, and `PersistentServiceVipAllocator::release` are all **unchanged**. Do NOT add public API. |
| **Bridge** | NO change required (D4). Retaining the VIP keeps the memo present for the stopped-but-declared window, so `hydrate_bridge_desired_listeners` projects the listener set and the zero-backend retraction lands — name → NXDOMAIN — with no bridge edit. |
| **`F` / `FrontendAddrAllocator`** | Untouched (D6.1/D6.2). |
| **Deletion path** | None wired today (D3); do NOT invent one. The new gate consumes the *existing* `desired.job.is_none()` signal; any future deletion verb drives it for free via `IntentStore::delete`. |

**No new public API.** Every surface the crafter touches already exists:
the `service_vip_release_emission` helper, the `desired.job` field on
`WorkloadLifecycleState`, the `Action::ReleaseServiceVip` variant, the
`released_for_terminal` View field. If the crafter believes a new
type/method/variant/parameter is required to implement D1, that is a
**design gap to STOP and surface** (CLAUDE.md "implement to the design —
never invent API surface"), not licence to improvise — the design as
specified needs none.

**Test impact the crafter must address (specification, not roadmap):**
the existing release-on-terminal acceptance tests
(`crates/overdrive-core/tests/acceptance/workload_lifecycle_release_service_vip.rs`
and the release-dispatch tests under `overdrive-control-plane`) assert
the *old* trigger (release fires on terminal observation while
`desired.job.is_some()`). Per `.claude/rules/development.md` § "Deletion
discipline" / "Behavior change must mark stale adjacent docs", those
assertions invert in the same commit: release MUST NOT fire on a
terminal-but-declared alloc (the new RED → GREEN), and MUST fire on
`desired.job.is_none()`. The #251 Tier-3 oracle
(`dns_responder_nxdomain.rs::after_backend_stops_the_job_is_withheld_nxdomain_never_a_stale_addr`,
currently `#[ignore]`) is un-ignored and becomes GREEN under this change
(stop retains the VIP → bridge retraction lands → NXDOMAIN). The DST
`workload_gc_absent_intent` scenario (intent-delete → GC) is the
positive-direction oracle that the release *does* fire on intent
withdrawal.

**Sections rewritten:** § 6 (supersession marker on the release
trigger; original text retained as history); Status line; **Relates to**
(ADR-0072 added); this amendment block. § 1 / § 1a / § 2 / § 3 / § 4 /
§ 5 / § 5a / § 7 / § 8 and the allocator's `allocate`/`release` *memo*
semantics are **untouched** — release-on-deletion changes *when*
`release` is called, not *what* it does.

**No production code touched under this amendment** — landing belongs to
the next DELIVER crafter dispatch against #251.

Cross-reference: `overdrive-sh/overdrive#251`;
`docs/analysis/rca-251-withhold-on-stop.md`;
`docs/research/orchestration/service-vip-dns-lifecycle-stop-vs-delete-k8s-nomad.md`;
ADR-0072 (`FrontendAddrAllocator` release-on-deletion-only); § 6
(superseded release trigger); #249 (operator-stop retains Service
intent — the precondition this amendment relies on).
