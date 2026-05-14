# ADR-0049 — Platform-issued Service VIP allocator: shared pool primitive under `overdrive-dataplane`; `IntentStore`-persisted; submit-time admission; reconciler-driven reclamation

## Status

Proposed. 2026-05-14. Decision-makers: Morgan (proposing); DESIGN-wave
output of `docs/feature/service-vip-allocator/`.

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
ADR-0013 (reconciler primitive).

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

### 1. Shared allocator primitive — pure core + persistence shim

The shared primitive is a **two-layer factoring**: a pure in-memory
core (`PoolAllocator<T>`) parameterised over a token trait, and a
thin persistence shim (`IntentBackedAllocator<T>`) that wraps the
core and writes through to `IntentStore` on every mutation.
`BackendIdAllocator` uses the core directly (no persistence — it
re-hydrates via ADR-0042); `ServiceVipAllocator` uses the shim
(persistence — required by AC-02).

**Module location**:
`crates/overdrive-dataplane/src/allocators/` (plural — the existing
`allocator.rs` moves into the module as `allocators/backend_id.rs`
via a single-cut migration per
`feedback_single_cut_greenfield_migrations.md`; the existing tests
move with it). Layout:

```
crates/overdrive-dataplane/src/allocators/
├── mod.rs                 # re-exports + Token trait + PoolError
├── pool.rs                # generic PoolAllocator<T: Token>
├── intent_backed.rs       # IntentBackedAllocator<T> persistence shim
├── backend_id.rs          # BackendIdAllocator (moved from allocator.rs)
└── service_vip.rs         # NEW — ServiceVipAllocator
```

**Trait — `Token`**:

```rust
/// A type that names a single member of a pool. Implementations are
/// typically newtype-around-fixed-width-integer (BackendId) or
/// newtype-around-addr (ServiceVip).
pub trait Token: Copy + Eq + Ord + std::hash::Hash + Send + Sync + 'static {
    /// The pool's discriminant key — the input the memo table is
    /// keyed on. For BackendId, this is `(ip, port, proto)`. For
    /// ServiceVip, this is the spec digest.
    type Key: Copy + Eq + Ord + Send + Sync + 'static;

    /// Construct the Nth token from a monotonic counter index.
    /// Returns None when the pool's range is exhausted.
    ///
    /// For BackendId: `Self::nth(n)` is `BackendId::new(n)`.
    /// For ServiceVip: `Self::nth(n)` maps `n` onto the configured
    ///   CIDR range, skipping reserved addresses.
    fn nth(n: u32, range: &Self::Range) -> Option<Self>;

    /// The pool's address space (counter range / CIDR / etc.).
    /// Carries pool configuration immutably; constructed once at
    /// allocator boot.
    type Range: Clone + Send + Sync + 'static;
}
```

**Pure core — `PoolAllocator<T: Token>`**:

```rust
pub struct PoolAllocator<T: Token> {
    next: u32,
    by_key: BTreeMap<T::Key, T>,    // memo (input → output)
    by_token: BTreeMap<T, T::Key>,  // reverse memo (for release)
    range: T::Range,
}

impl<T: Token> PoolAllocator<T> {
    pub const fn with_range(range: T::Range) -> Self;
    pub fn allocate(&mut self, key: T::Key) -> Result<T, PoolError>;
    pub fn release(&mut self, key: &T::Key) -> Option<T>;
    pub fn get(&self, key: &T::Key) -> Option<T>;
    pub fn memo_len(&self) -> usize;
}

#[derive(thiserror::Error, Debug)]
pub enum PoolError {
    /// No tokens available — the configured `Range` is fully
    /// allocated. Surfaces AC-04 (#167) at the admission boundary.
    #[error("pool exhausted: {allocated} of {capacity} tokens in use")]
    Exhausted { allocated: u32, capacity: u32 },
}
```

The core is `BTreeMap`-backed per `.claude/rules/development.md`
§ "Ordered-collection choice" (the memo is iterated during release
and observed by DST invariants); the counter is monotonic and never
wraps in practice (the persistence shim's restart story does not
reuse counter values within a single process lifetime, only across
process restarts when the persisted state is re-hydrated).

**Persistence shim — `IntentBackedAllocator<T>`**:

```rust
pub struct IntentBackedAllocator<T: Token> {
    inner: parking_lot::Mutex<PoolAllocator<T>>,
    store: Arc<dyn IntentStore>,
    namespace: AllocatorNamespace, // newtype; e.g., "service-vip"
}

impl<T: Token> IntentBackedAllocator<T>
where
    T::Key: ArchiveForStore,        // sealed bound; see §1a
    T:      ArchiveForStore,
{
    /// Construct empty + bulk-load the persisted state.
    ///
    /// `bulk_load` performs an Earned Trust gate (probe() — see §6):
    /// reads every persisted `(key, token)` pair from
    /// `IntentStore` under `namespace`, validates round-trip via
    /// the rkyv envelope, and refuses to start (returning
    /// `AllocatorBootError::Envelope`) if any row fails to decode.
    pub fn bulk_load(
        range: T::Range,
        store: Arc<dyn IntentStore>,
        namespace: AllocatorNamespace,
    ) -> Result<Self, AllocatorBootError>;

    /// Allocate-or-memo. Writes through to IntentStore on a fresh
    /// allocation; memo-hits are zero-write.
    ///
    /// Ordering is fsync-then-memory (matches ADR-0035 §
    /// "Step ordering 7 → 8 is load-bearing"): the IntentStore write
    /// commits + fsyncs before the in-memory `PoolAllocator` is
    /// updated. On crash between fsync and memory-update, the next
    /// boot's bulk_load rebuilds the memo from the persisted state.
    pub fn allocate(&self, key: T::Key) -> Result<T, AllocatorError>;

    /// Release-and-delete. Idempotent on already-released keys.
    pub fn release(&self, key: &T::Key) -> Result<(), AllocatorError>;

    /// Borrow the read-only view of the pool (for diagnostics /
    /// alloc status echo).
    pub fn get(&self, key: &T::Key) -> Option<T>;
}
```

**Why pure-core + shim (not generic-with-trait, not separate types)**:

| Shape | Pros | Cons | Verdict |
|---|---|---|---|
| (a) Generic `Allocator<T: Token>` with persistence as type-parameter slot | Single struct, parametric uniformity | Hides the persistence-vs-non-persistence distinction at the call site; makes "BackendId never persists" a runtime convention, not a compile-time property | **REJECTED** — collapses the load-bearing distinction. |
| (b) Pure-core + persistence shim (chosen) | Persistence is a boundary; core is testable without IntentStore mocking; BackendId compile-time-cannot-persist | Two types instead of one; slight duplication in surface area | **CHOSEN** — the persistence boundary is the load-bearing distinction; making it structural is the simplest-honest factoring. |
| (c) Two independent types (`BackendIdAllocator`, `ServiceVipAllocator`) sharing helper fns | Maximal simplicity; no generics | Violates AC-05 ("the underlying allocator logic is shared"); duplicates the memo-table + counter + release semantics; future third consumer copies the duplication | **REJECTED** — fails AC-05 structurally. |

### 1a. Persistence wire format — rkyv envelope per ADR-0048

The persisted state crosses an `IntentStore` redb boundary, so it
follows ADR-0048's per-type versioned envelope discipline. One
envelope per allocator namespace (one for BackendId — not used today
but kept in the trait surface for future Phase 2 persistence; one
for ServiceVip — used). Wire shape:

```rust
// Persisted row — one per (namespace, key) pair.
// Lives in overdrive-core::dataplane (next to existing dataplane
// types) per the precedent of BackendKey / ServiceVip.
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, ...)]
pub struct AllocatorEntryV1 {
    pub key:     [u8; 32],  // SHA-256 of T::Key archived bytes
    pub token:   AllocatorTokenBytes, // sum-typed: BackendId | ServiceVip
    pub counter: u32,       // the monotonic counter value at allocation
}

// Codec-internal envelope (NOT re-exported from lib.rs per UI-01).
pub enum AllocatorEntryEnvelope {
    V1(AllocatorEntryV1),
}

impl VersionedEnvelope for AllocatorEntryEnvelope { /* ... */ }

// Public alias-to-payload (UI-02).
pub type AllocatorEntry = AllocatorEntryV1;
```

Wrapping discipline lives in a codec module on `AllocatorEntry`
(`AllocatorEntry::archive_for_store` / `from_store_bytes`) per
ADR-0048 § "Typed persistence-boundary codec". A schema-evolution
golden-bytes fixture under
`crates/overdrive-dataplane/tests/schema_evolution/allocator_entry.rs`
pins V1 archived bytes per `.claude/rules/testing.md` § "Archive
schema-evolution roundtrip".

**Why a unified `AllocatorEntry` envelope, not one per namespace**: the
serialised shape is identical (key digest + token bytes + counter);
the namespace itself is part of the redb key prefix, not the value.
This collapses two future envelopes into one and means an additional
allocator namespace ships without a new envelope version.

**Why SHA-256 of the key, not the key bytes inline**: `T::Key` for
ServiceVip is the spec digest (already a 32-byte hash). For
BackendId it's `(u32, u16, u8)` — fixed-width and small, but
hashing it for the persisted form keeps the wire layout uniform
across namespaces and avoids a per-namespace key codec.

### 2. `ServiceVipAllocator` — `Token` instantiation

```rust
// crates/overdrive-dataplane/src/allocators/service_vip.rs

/// A range of IPv4 VIPs the platform may allocate to Service workloads.
/// Built from operator config (§3); immutable after boot.
#[derive(Clone)]
pub struct VipRange {
    cidr: Ipv4Cidr,                 // newtype in overdrive-core
    reserved: BTreeSet<Ipv4Addr>,   // platform-reserved (e.g., gateway, broadcast)
}

impl VipRange {
    pub fn capacity(&self) -> u32 { /* CIDR size - reserved count */ }
}

impl Token for ServiceVip {
    type Key = ServiceSpecDigest;   // newtype around [u8; 32]
    type Range = VipRange;

    fn nth(n: u32, range: &VipRange) -> Option<Self> {
        // Walk CIDR addresses in canonical order; skip reserved;
        // return the Nth non-reserved address. Returns None when
        // n >= range.capacity().
    }
}

pub type ServiceVipAllocator = IntentBackedAllocator<ServiceVip>;
```

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
be used by the allocator (`IntentBackedAllocator<ServiceVip>`), the
allocator's persisted `AllocatorTokenBytes::ServiceVip` payload, and
the downstream consumer surface (`ServiceMapHydrator` consults
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

**No defaults**: operators MUST supply `ranges`. Per #167 § "Out of
scope": "Opinionated default VIP ranges. The allocator is
pool-agnostic." A missing `[dataplane.vip_allocator]` section
surfaces as `VipAllocatorConfigError::Missing` at boot — refuse to
start, structured `health.startup.refused` event per ADR-0048
intent-layer discipline.

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
to `ServiceVipAllocator::release(&spec_digest)`. The allocator
release is idempotent and write-through (fsync-then-memory same as
allocate).

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
moves the existing allocator:

```
DELETE: crates/overdrive-dataplane/src/allocator.rs        (140 lines)
CREATE: crates/overdrive-dataplane/src/allocators/mod.rs
CREATE: crates/overdrive-dataplane/src/allocators/pool.rs
CREATE: crates/overdrive-dataplane/src/allocators/intent_backed.rs
CREATE: crates/overdrive-dataplane/src/allocators/backend_id.rs  (from old allocator.rs)
CREATE: crates/overdrive-dataplane/src/allocators/service_vip.rs
UPDATE: crates/overdrive-dataplane/src/lib.rs (mod declaration)
UPDATE: every call site of BackendIdAllocator (path change only — API stable)
```

`BackendIdAllocator`'s public API (`new()`, `allocate(ip, port,
proto)`, `release(id)`, `memo_len()`) stays signature-stable; only
its internal representation changes to wrap `PoolAllocator<BackendId>`.
The existing tests (proptest at `allocator.rs:92-110`,
collision-witness at `:125-138`) move with the file and continue to
pass. R1 from DISCUSS § Risks (hot-path coverage) is preserved by
this stability.

The existing typed `Token` impl for `BackendId` lives at
`allocators/backend_id.rs` and uses `(u32, u16, u8)` as `Key` and a
`BackendIdRange { start: 1, max: u32::MAX }` as `Range`.

### 8. Earned Trust — `probe()` on the allocator at boot

Per the project's load-bearing principle: every dependency must
demonstrate it can honor its contract. The `IntentBackedAllocator`
specifies a `probe()` method that runs at composition-root time and
verifies:

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
already mandates: subtype check (mypy/Protocol-equivalent — the
`probe()` method is on the trait), structural check (an
`xtask::dst_lint` AST scanner walks every `IntentBackedAllocator`
construction site and asserts `probe()` is called before first
`allocate()` / `release()`), behavioral check (a CI gold-test that
configures a CIDR-too-small-for-persisted-state fixture and asserts
the probe refuses to start).

## Considered alternatives (ADR-level — additional to the per-question shapes above)

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

**Deferred (not rejected).** Phase 1 single-node deployments draw
from a configurable CIDR; even a /16 (65k addresses) supports
years of churn without exhaustion. The monotonic-counter shape
matches ADR-0046's BackendId precedent. If KPI K4 (pool exhaustion)
fires in practice, an LRU recycling strategy lands as an additive
amendment to `PoolAllocator` — the `Token::nth` trait surface is
forward-compatible (a recycling impl computes `n` from a free-list
rather than a counter; same return shape).

## Consequences

### Positive

1. **AC-05 structurally satisfied.** `BackendIdAllocator` and
   `ServiceVipAllocator` share the `PoolAllocator<T>` core; the
   shared primitive lives in `crates/overdrive-dataplane/` per
   DISCUSS D3.
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
