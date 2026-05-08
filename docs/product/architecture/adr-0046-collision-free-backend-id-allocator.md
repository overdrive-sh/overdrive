# ADR-0046 -- Collision-free BackendId allocator replaces multiplicative-hash derivation

## Status

Accepted. 2026-05-08. Tags: phase-2, dataplane, correctness,
userspace-only.

## Context

### The bug

`crates/overdrive-dataplane/src/lib.rs:913-916` derives `BackendId`
from the backend endpoint via Knuth's multiplicative hash:

```rust
let backend_id: u32 = pod
    .ipv4_host
    .wrapping_mul(2_654_435_761)
    .wrapping_add(u32::from(pod.port_host));
```

The same derivation is duplicated at `lib.rs:968-971` (Maglev
permutation input). This function is not injective over the 48-bit
`(ipv4: u32, port: u16)` input space. By the birthday bound, the
probability of at least one collision among N distinct endpoints
reaches 50% at N ~ 82,138 (sqrt(pi/2 * 2^32)). At smaller fleet
sizes the probability is lower but non-zero, and collisions are
silent.

### Failure mode

When two distinct endpoints `(ip_A, port_A)` and `(ip_B, port_B)`
produce the same hash `H`:

1. **Last-writer-wins in BACKEND_MAP.** The `BPF_ANY` upsert at
   `lib.rs:917-919` overwrites the first entry. No error is surfaced.
2. **Cross-service traffic leakage.** Service 1's inner ARRAY (Maglev
   permutation) still contains slot values referencing `H`. XDP
   rewrites Service 1 traffic to whichever backend was written last --
   silently forwarding to the wrong service's backend.
3. **Orphan GC cannot reclaim stale entries.** The GC sweep at
   `lib.rs:1036-1043` computes a live-set union across all services.
   Both services reference `H`, so the entry is never orphaned, even
   though one service's backend has been silently replaced.
4. **No observability.** No counter, log, or tracepoint fires. The
   operator sees incorrect routing with no diagnostic signal.

CI does not catch this because test suites use small, hand-picked
endpoint sets where collisions are astronomically unlikely.

### Production precedent

Comprehensive research
(`docs/research/dataplane/l4-lb-backend-identification-research.md`)
examined Cilium, Katran (Meta), Google Maglev (NSDI 2016), Cloudflare
Unimog, and IPVS. Key finding: **every production BPF L4 LB uses
opaque integer IDs (monotonic counters or positional indices), not
hash-derived IDs.** Cilium allocates via `IDAllocator` (monotonic
counter, range 1..65535). Katran uses positional indices into a flat
`reals` array. No production system stores hash-derived values as
backend identifiers.

## Decision

Replace the multiplicative-hash derivation with a monotonic-counter
allocator on `EbpfDataplane`, matching Cilium's `IDAllocator` pattern.

### Allocator specification

A `BackendIdAllocator` struct, owned by `EbpfDataplane`, behind
`parking_lot::Mutex`:

- **Internal state**: `next: u32` (monotonic counter, starts at 1; 0
  reserved for "empty slot" semantics in the inner ARRAY),
  `by_endpoint: BTreeMap<(u32, u16, u8), BackendId>` memo table
  mapping `(ip_host, port_host, proto)` to the assigned `BackendId`.

- **`allocate(&mut self, ip: u32, port: u16, proto: u8) -> BackendId`**:
  returns the existing id if the endpoint has been seen before (memo
  hit); otherwise assigns `next`, increments `next`, inserts into the
  memo, and returns the new id.

- **`release(&mut self, id: BackendId)`**: removes the memo entry
  whose value matches `id`. Called by orphan GC when a `BackendId`
  leaves the live set. Does NOT recycle the counter value -- the
  counter is monotonic and never wraps in practice (2^32 =
  4,294,967,295 distinct endpoints over node lifetime).

- **`BTreeMap`** for the memo table per `.claude/rules/development.md`
  "Ordered-collection choice" -- the memo is iterated during
  `release` and during any future diagnostic dump; deterministic
  order is the right default.

### Integration points

Two call sites change:

1. **BACKEND_MAP populate** (`lib.rs:907-920`): the multiplicative
   hash at lines 913-916 is replaced with
   `allocator.allocate(pod.ipv4_host, pod.port_host, proto)`.

2. **Maglev permutation input** (`lib.rs:964-977`): the duplicated
   hash at lines 968-971 is replaced with the same
   `allocator.allocate(...)` call, which returns the memoised id for
   an endpoint already allocated in step 1.

The orphan GC sweep (`lib.rs:1036-1049`) stays structurally identical.
After sweeping BACKEND_MAP entries not in the live set, it additionally
calls `allocator.release(id)` for each removed id to clean the memo.

The forward pointer at `lib.rs:892-894` ("A future BackendId allocator
can replace this") is resolved by this ADR.

### What stays unchanged

- `BackendId: u32` newtype in `overdrive-core` -- preserved.
- BACKEND_MAP key type: `u32` -- unchanged.
- SERVICE_MAP inner ARRAY value type: `u32` (BackendId) -- unchanged.
- Maglev permutation generator signature:
  `BTreeMap<BackendId, u16> -> Vec<BackendId>` -- unchanged.
- Kernel-side BPF programs: zero changes.
- Per-service inner-map memory: 16,381 x 4 = 65 KiB -- unchanged.
- Verifier budget: zero delta.
- `service_backends` tracker type:
  `BTreeMap<ServiceKey, BTreeSet<u32>>` -- unchanged.

## Consequences

### Positive

- **Collision class eliminated structurally.** A monotonic counter
  never assigns the same id to distinct endpoints.
- **Production-validated pattern.** Matches Cilium's `IDAllocator`
  (the most battle-tested BPF L4 LB) without inheriting its 16-bit
  range limitation.
- **Minimal change surface.** ~50 LOC addition (allocator struct +
  impl + two call-site replacements + GC integration). Userspace
  only. No kernel-side changes, no memory increase, no verifier
  budget impact, no Tier 4 baseline regeneration.
- **Deduplication preserved.** Same `(ip, port, proto)` yields the
  same `BackendId` across `update_service` calls because the memo
  table returns the existing assignment.

### Negative

- **Memo table memory.** O(distinct-backends-currently-live) entries,
  each ~15 bytes (12-byte key + 4-byte value in a BTreeMap node).
  At 10,000 backends: ~150 KiB. Cleaned by orphan GC `release`.
- **Crash recovery.** On process restart the allocator starts fresh
  (counter at 1, empty memo). All services are re-hydrated by the
  `ServiceMapHydrator` reconciler (ADR-0042), which calls
  `update_service` for every active service, rebuilding the memo
  from scratch. Existing BACKEND_MAP entries from the prior process
  are overwritten with new ids. This is correct because the inner
  ARRAYs are also rebuilt atomically during re-hydration (the
  SERVICE_MAP outer-map `set` is atomic per ADR-0040). No stale
  cross-reference survives.

## Alternatives Considered

### B. Drop BackendId, rekey BACKEND_MAP on `BackendKeyPod` directly

Explored in a discarded draft ADR-0046. Eliminates collisions
structurally by making the key the full `(ip: u32, port: u16,
proto: u8, _pad: u8)` endpoint (8 bytes). Rejected:

- Doubles per-service inner-map memory (65 KiB to 131 KiB; 512 MiB
  at 4,096 services).
- Requires kernel-side BPF changes (ARRAY value width, BACKEND_MAP
  key width) and Tier 4 verifier-budget + xdp-bench baseline
  regeneration.
- No production BPF LB stores 8-byte endpoint structs in Maglev
  table slots. The research document's Conflict 1 analysis notes
  that Direction B's structural-purity argument ("the key IS the
  endpoint") diverges from production practice where the universal
  pattern is small opaque integer indices.

### C. Wider hash (SipHash-2-4 to 64 bits)

Reduces collision probability (birthday bound shifts to N ~ 5.4
billion) but does not eliminate the class. Still non-injective. Same
kernel-side widening cost as B (inner ARRAY value becomes 8 bytes)
without the structural clarity of making the key the actual endpoint.
Rejected: partial fix at full cost.

### D. Per-service Maglev table sizing / indirect slot encoding

Explored as Directions C1-C3 in the research document. Theoretically
offers memory savings (down to ~18 KiB/service with 1-byte indices,
or ~3 KiB/service with variable M) but has zero production precedent,
300-600 LOC complexity, and adds a third map-lookup hop on the
per-packet fast path. Orthogonal to the collision bug and can be
explored independently as a memory optimization in a future ADR. Not
rejected, but out of scope for this correctness fix.

## Regression Tests

1. **Proptest: no duplicate BackendIds.** Generate N random
   `(ip, port, proto)` triples (N >= 1000), allocate BackendIds for
   all via the allocator, assert no two distinct endpoints received
   the same `BackendId`. Covers the allocator's injectivity property.

2. **Deterministic collision witness.** Find a specific `(ip_1, p_1)`
   and `(ip_2, p_2)` pair that collides under the OLD multiplicative
   hash (search is trivial; the birthday bound guarantees one exists
   in a few thousand random trials). Allocate both via the new
   allocator. Assert they receive distinct ids. This test documents
   the class of bug this ADR fixes and will fail if the old derivation
   is accidentally reintroduced.

## References

- `docs/research/dataplane/l4-lb-backend-identification-research.md` --
  production precedent analysis (Cilium, Katran, Maglev, Unimog, IPVS).
- ADR-0040 -- three-map split and HASH_OF_MAPS atomic swap (preserved).
- ADR-0041 -- weighted Maglev and REVERSE_NAT shape (preserved).
- ADR-0042 -- `ServiceMapHydrator` reconciler (preserved; re-hydration
  rebuilds the allocator memo on restart).
- Cilium issue #16121 -- 16-bit `MaxSetOfBackendID` scaling limitation;
  motivates the 32-bit counter choice here.
