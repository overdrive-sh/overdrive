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
5. **Upstream slice-06 admission policy** — parser-level vs
   admission-level rejection of operator-supplied `vip = Some(...)`.

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
is deleted in the same commit. Per ADR-0047 § Listener field the
spec field stays `Option<ServiceVip>` — the type the field references
just moves to a single canonical location.

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
allocator's `allocate(spec_digest)` returns the assigned VIP (memo-hit
on resubmit per AC-02); the VIP is folded into the spec via the
`ListenerRow.vip = Some(allocated_vip)` projection; the full spec is
then written to IntentStore.

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
- `AdmissionError::VipNotOperatorAssignable { listener_idx }` —
  see § 5; admission rejects with named guidance before the
  allocator is consulted.

**Spec-digest invariance under VIP-as-input vs VIP-as-output**: the
spec digest is computed over the spec *without* the listener.vip
field (the field is `None` at digest-compute time per Slice 06
shape). The allocator memo is keyed on the digest of the VIP-free
spec; the assignment writes `vip = Some(...)` onto the listener
post-allocation. This means **resubmitting an unchanged spec
produces the same digest** (the operator never types the VIP, so
the digest is stable across submits — AC-02 is structural).

### 5. Operator-supplied `vip = Some(...)` — admission-level rejection

Per the DISCUSS Changed Assumption: operators cannot pin a VIP. The
choice is between rejecting at parser-level (the TOML parser fails
when `vip` is present) or admission-level (the parser produces a
parsed spec with `vip: Some(...)`; the admission handler rejects).

| Shape | Pros | Cons | Verdict |
|---|---|---|---|
| (a) Parser-level rejection | Earliest failure; CLI message names the file + line; no half-parsed state | Schema change: must remove `vip` from the parser's `ListenerInput`; introduces a parser-time policy ("operators don't get to pin VIPs") that is technically not a parse error but a policy error; reverses Slice 06's forward-compatibility framing | Mixed |
| (b) Admission-level rejection (chosen) | Parser stays a pure structural validator; the `Option<ServiceVip>` field shape from ADR-0047 is preserved verbatim; future tier may allow operator-pinned VIPs without parser surface change; rejection happens before any IntentStore write, so AC-06 ("no allocator state is mutated and no admission occurs") is preserved | One additional admission step (cheap — type-tag check on the listener); error surface is "submit" rather than "load file" | **CHOSEN** |
| (c) Silent ignore (collapse `Some(_)` to `None`) | — | Violates AC-06 ("rejected with named guidance"); silent input mutation | **REJECTED** (per #167 AC-06) |

**Admission handler check** (in `overdrive-control-plane`, before
spec-digest compute):

```rust
fn validate_service_listeners(spec: &ServiceSpec) -> Result<(), AdmissionError> {
    for (idx, listener) in spec.listeners.iter().enumerate() {
        if listener.vip.is_some() {
            return Err(AdmissionError::VipNotOperatorAssignable {
                listener_idx: idx,
            });
        }
    }
    Ok(())
}
```

**Upstream `ListenerRow` field shape stays `Option<ServiceVip>`**: the
spec-side `Option` carries `None` at submit time and `Some(vip)` at
post-allocation persistence time. The forward-compatibility framing
from ADR-0047 § Listener field is preserved verbatim.

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

## Cross-references

- GH #167 — SSOT for the feature
- ADR-0046 — `BackendIdAllocator` structural precedent (extended,
  not superseded)
- ADR-0047 — `ListenerRow.vip: Option<ServiceVip>` field shape
  (preserved; admission-rejection lands at the admission layer per § 5)
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
