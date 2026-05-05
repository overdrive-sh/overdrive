# ADR-0040 — SERVICE_MAP three-map split (SERVICE_MAP / BACKEND_MAP / MAGLEV_MAP) + HASH_OF_MAPS atomic-swap primitive

## Status

Accepted. 2026-05-05. Decision-makers: Morgan (proposing); user
ratified `lgtm` against
`docs/feature/phase-2-xdp-service-map/design/proposal-draft.md`
(2026-05-05). Tags: phase-2, dataplane, kernel-maps, service-map,
load-balancing.

**Companion ADRs**: ADR-0041 (weighted Maglev + REVERSE_NAT shape +
endianness lockstep), ADR-0042 (`ServiceMapHydrator` reconciler +
`Action::DataplaneUpdateService` + `service_hydration_results`
observation table).

## Context

Phase 2.2 (GH #24) fills the empty body of `Dataplane::update_service`
that Phase 2.1's ADR-0038 substrate left as a stub. The body
implements XDP service load balancing per `whitepaper.md` § 7
*eBPF Dataplane / XDP — Fast Path Packet Processing* and § 15
*Zero Downtime Deployments* (atomic backend swap).

Two architectural questions need to be settled together because the
answer to one constrains the other:

1. **How does the kernel-side decompose the
   `(VIP, port) → backend` lookup?** Three credible shapes exist
   in the published reference set:
   - **Cilium / Katran three-map split** — `SERVICE_MAP{(VIP, port) → service_id}` + `BACKEND_MAP{backend_id → backend_entry}` + `MAGLEV_MAP{service_id → slot_array}`. Three single-purpose maps, each with a clear read pattern (research § 2.1, § 2.2, § 6.2).
   - **Single-map shape** — one `BPF_MAP_TYPE_SOCK_HASH` keyed by `(VIP, port)`, value is the full backend list. Small footprint at single-service scale, no atomic-swap primitive at multi-service scale.
   - **Array-based SERVICE_MAP** — `BPF_MAP_TYPE_ARRAY` of fixed size, indexed by hash of `(VIP, port)`. Lock-free; fixed size constraint binds operator and forces collision handling.

2. **How is the backend set rotated atomically when a service's
   backends change?** Three credible mechanisms:
   - **`BPF_MAP_TYPE_HASH_OF_MAPS`** — outer map's value is an inner-map fd; rotating the inner-map fd is one atomic syscall (research § 3). The kernel swaps the entire inner map under the lookup hot path.
   - **In-place mutation of a fixed-size map** — write new entries; no atomic primitive; requires reader-side reconciliation.
   - **Two-map double-buffer** — userspace toggles a generation counter; kernel reads the indicated generation. Requires per-packet generation read.

These questions extend the Phase 2.1 substrate (ADR-0038):

- The kernel side compiles against `bpfel-unknown-none` with
  `#![no_std]` and `aya-ebpf` only.
- The userspace loader compiles against the host triple with `aya`.
- `Dataplane` port trait surface is the only consumer-facing
  contract; no `aya` import outside `overdrive-dataplane`.

A third question — how the kernel-side reads its key tuple from
the wire — falls naturally to ADR-0041's endianness section.

## Decision

### 1. Adopt the Cilium / Katran three-map split

The kernel-side hot path uses three maps, each with a single
typed key shape:

| Map | Type | Key | Value | Purpose |
|---|---|---|---|---|
| `SERVICE_MAP` | `BPF_MAP_TYPE_HASH_OF_MAPS` (outer) | `(ServiceVip, u16 port)` | inner-map fd | `(VIP, port)`-to-inner-map indirection. Outer map atomically rotates its value (the inner-map fd) on backend-set change. Inner = `BPF_MAP_TYPE_HASH` keyed by `BackendId` → `BackendEntry`, `max_entries = 256`. |
| `BACKEND_MAP` | `BPF_MAP_TYPE_HASH` | `BackendId` (u32) | `BackendEntry { ipv4, port, weight, healthy, _pad }` | Single global; backends shared across services. `max_entries = 65_536`. |
| `MAGLEV_MAP` | `BPF_MAP_TYPE_HASH_OF_MAPS` (outer) | `ServiceId` (u64) | inner-map fd | Inner = `BPF_MAP_TYPE_ARRAY` of `BackendId` slots, size = `MaglevTableSize` (default 16_381). One inner per service. |

The trait surface that drives this layout is locked at:

```rust
async fn update_service(
    &self,
    service_id: ServiceId,
    vip: ServiceVip,
    backends: Vec<Backend>,
) -> Result<(), DataplaneError>;
```

(Q-Sig=A — three explicit args at the trait surface; no aggregate
unpack.)

**Drift correction.** The proposal-draft initially framed
"`ServiceId` keys all three maps." That conflated trait surface with
kernel-map shape; the kernel sees wire packets and must look up by
`(VIP, port)`. Corrected:

- `SERVICE_MAP` outer key = `(ServiceVip, u16 port)` — wire-shape
  driven.
- `MAGLEV_MAP` outer key = `ServiceId` — control-plane-shape
  driven.
- `BACKEND_MAP` key = `BackendId` — flat-namespace driven.

Three keys, typed-distinct, traced end-to-end through trait → shim
→ loader → BPF maps.

### 2. Atomic swap via HASH_OF_MAPS outer-map fd replacement

Both `SERVICE_MAP` and `MAGLEV_MAP` are `BPF_MAP_TYPE_HASH_OF_MAPS`
outers. On a backend-set change:

1. Userspace builds the new inner map (HASH or ARRAY, depending).
2. Userspace populates it with the new backend set (HASH) or
   recomputes the Maglev permutation table (ARRAY).
3. Userspace replaces the outer-map's value (an fd) with the new
   inner-map fd. This is **one atomic kernel syscall**.
4. The kernel's reference count on the old inner fd drops; in-flight
   readers complete against the old inner; new readers see the new
   inner.

The userspace mechanism lives in
`crates/overdrive-dataplane/src/swap.rs`. The atomic-swap primitive
is the architectural foundation for ASR-2.2-01 (zero-drop atomic
swap, ≤ 0 packets dropped attributable to the swap boundary over a
30-second swap-storm window).

### 3. Checksum helper choice — kernel helpers (Q1=A)

The forward-path packet rewrite uses `bpf_l3_csum_replace` and
`bpf_l4_csum_replace` from the kernel-helper set, not the
`csum_diff` family from aya. Rationale: kernel helpers are
verifier-clean across the entire kernel matrix without exposing
additional verifier constraints; the `csum_diff` family adds wrapper
indirection that costs verifier-budget without functional gain
(research § 4.1, § 4.2). The choice keeps DROP_COUNTER off the
checksum hot path, preserving Tier 4 verifier-budget headroom.

### 4. Sanity-prologue strategy — shared `#[inline(always)]` Rust helper (Q3=C)

Pre-SERVICE_MAP packet-shape sanity checks (Slice 06) live in
`crates/overdrive-bpf/src/shared/sanity.rs` as
`#[inline(always)]` functions. The functions get inlined at every
call site in `xdp_service_map.rs` and (future) other XDP / TC
programs. This is the canonical aya-rs pattern (research § 8.2)
and matches Cilium's structural shape after their initial
duplication-then-tail-call iteration converged on inlining.

Rejected:
- **Inline duplication** — source drifts asymmetrically across
  programs (research § 8.2 documents the failure shape in
  Cilium's history).
- **`bpf_tail_call` shared helper** — verifier-budget-equivalent
  reasoning *plus* indirection on every packet; no upside.

### 5. HASH_OF_MAPS inner-map size — fixed 256 (Q5=A)

Inner-map `max_entries = 256`, compiled in. Well above any
realistic per-service backend count for Phase 2 (research § 3.3);
keeps the BPF map declaration syntax simple and verifier-friendly
(`#[map(name = "...", max_entries = 256)]`). Operator-tunability
for the algorithmic shape composes via `MaglevTableSize` (ADR-0041);
the inner HASH_OF_MAPS size is a structural constant.

### 6. `DropClass` slot count locked at 6 (Q7=B)

The `DROP_COUNTER` `BPF_MAP_TYPE_PERCPU_ARRAY` is indexed by
`DropClass as u32` with six locked variants. The newtype lives at
`crates/overdrive-core/src/dataplane/drop_class.rs`:

```rust
/// Drop classification for the `DROP_COUNTER` PERCPU_ARRAY.
/// `#[repr(u32)]` makes `as u32` a stable kernel-side index
/// across Rust toolchains (the verified pattern Cilium and
/// Katran use).
///
/// Variant ordering and discriminants are STABLE — additions are
/// minor-version (per ADR-0037 K8s-Condition convention);
/// reordering or removal is a major-version break that requires
/// a new ADR.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DropClass {
    MalformedHeader   = 0,
    UnknownVip        = 1,
    NoHealthyBackend  = 2,
    SanityPrologue    = 3,
    ReverseNatMiss    = 4,
    OversizePacket    = 5,
}
```

`FromStr` parses kebab-case (`malformed-header` →
`MalformedHeader`); `Display` emits kebab-case; the proptest
harness in `crates/overdrive-core/tests/drop_class.rs` exhausts
all six variants and asserts `Display`/`FromStr` round-trip
bit-equivalent — the project STRICT-newtype discipline per
`development.md` § Newtype completeness.

Six slots cover every drop the XDP + TC programs in Phase 2.2
actually emit. Adding later is structurally compatible (PERCPU_ARRAY
index space is `u32`; new slots stay zero on every CPU until next
BPF re-load); reducing later is structurally compatible (unused
slots stay zero). The `#[repr(u32)]` annotation on the enum is
what makes `as u32` a stable index across Rust toolchains.

### 7. `cargo xtask perf-baseline-update` helper deferred (Q4=B)

Slice 07 ships its veristat / xdp-bench baselines via manual
`git mv`. The helper's surface area (4–5 args, file path
canonicalisation, baseline-rotation atomicity) is bigger than the
first three baseline-update commits will exercise; re-evaluate
after #29 / #152 lands.

## Alternatives Considered

### A — Single-map SOCK_HASH

A single `BPF_MAP_TYPE_SOCK_HASH` keyed by `(VIP, port)`, value =
the full backend list. **Rejected**: no atomic-swap primitive at
multi-service scale; updating a single key writes new bytes
in-place, exposing torn-read windows during long backend lists. The
zero-drop ASR (ASR-2.2-01) is structurally unachievable with this
shape; would require user-space reader-side reconciliation that the
XDP fast path cannot afford.

### B — Array-based SERVICE_MAP

A `BPF_MAP_TYPE_ARRAY` of fixed size, indexed by hash of
`(VIP, port)`. **Rejected**: fixed size at compile time forces
operators to declare a maximum service count up-front. Hash
collisions force a probing strategy that adds verifier-budget cost
on every packet. The HASH_OF_MAPS shape grows naturally; the array
shape cannot.

### C — `bpf_tail_call` for sanity prologue

Tail-call to a shared "prologue" program before SERVICE_MAP lookup.
**Rejected** for Q3: verifier-budget-equivalent reasoning *plus*
indirection on every packet; no upside relative to `#[inline(always)]`
on a Rust helper. Cilium's history (research § 8.2) converged on
inlining for the same reason.

### D — Two-map double-buffer

Two SERVICE_MAPs with a userspace-toggled generation counter. The
kernel reads a third map for the current generation, then looks up
in the indicated SERVICE_MAP. **Rejected**: per-packet additional
map lookup (the generation read) costs verifier budget and an
extra cache line; HASH_OF_MAPS achieves the same property in one
syscall with no per-packet cost.

## Consequences

**Positive:**

- ASR-2.2-01 (zero-drop atomic swap) becomes structurally achievable
  via HASH_OF_MAPS outer-fd replacement.
- Three single-purpose maps map cleanly to typed Rust handles in
  `overdrive-dataplane::maps/*` — no `BPF_MAP_TYPE_*` choice
  visible at call sites (research recommendation #5; matches
  "make invalid states unrepresentable" from
  `development.md` § Type-driven design).
- Verifier-budget delta is budgeted ≤ 20 % per PR (ASR-2.2-03);
  kernel-helper checksum choice + `#[inline(always)]` sanity-helper
  shape stay inside this envelope.
- Six drop-class slots cover Phase 2.2's drop surface without
  reserving unused index space.

**Negative:**

- Locks the kernel-floor at 5.10 LTS (HASH_OF_MAPS is stable from
  4.18+; Phase 2.2's Tier 3 floor of 5.10 is well above). Future
  Phase 2 features that want kernel features ≥ 5.18 (XDP-egress in
  particular) need their own kernel-floor uplift.
- Userspace permutation generation is one-time-per-change cost
  (DISCUSS Risk #5 acknowledged); production rate is
  ops-per-minute scale.
- Three maps in the kernel-side BPF object grow the per-program
  verifier baseline; mitigated by the `veristat` baseline gate.

**Operational implications:**

- `cargo xtask integration-test vm` continues to be available but
  not exercised by Phase 2.2 (single-kernel in-host per
  Constraint 1).
- Lima image already carries `bpf-linker` from Phase 2.1 (#23
  ADR-0038); no additional infra change.
- `cargo xtask bpf-build` regenerates the ELF; `cargo xtask
  verifier-regress` (Slice 07) baselines it; CI gates kick in
  per-PR for any change to `crates/overdrive-bpf/**`.

## References

- `docs/feature/phase-2-xdp-service-map/design/architecture.md` § 5,
  § 10, § 14.
- `docs/feature/phase-2-xdp-service-map/design/wave-decisions.md`
  D1, D3, D5.
- `docs/research/networking/xdp-service-load-balancing-research.md`
  § 2.1, § 2.2, § 3, § 3.3, § 4.1, § 4.2, § 6.2, § 8.2.
- `docs/whitepaper.md` § 7 *eBPF Dataplane*, § 15 *Zero Downtime
  Deployments*, § 19 *Security Model*.
- ADR-0038 (eBPF crate layout + build pipeline) — substrate.
- ADR-0041 (weighted Maglev + REVERSE_NAT) — companion.
- ADR-0042 (`ServiceMapHydrator`) — companion.
