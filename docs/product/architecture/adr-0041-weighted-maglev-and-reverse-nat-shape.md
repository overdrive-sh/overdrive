# ADR-0041 — Weighted Maglev consistent hashing + REVERSE_NAT_MAP shape + endianness lockstep contract

## Status

Accepted. 2026-05-05. Decision-makers: Morgan (proposing); user
ratified `lgtm` against
`docs/feature/phase-2-xdp-service-map/design/proposal-draft.md`
(2026-05-05). Tags: phase-2, dataplane, kernel-maps, maglev,
reverse-nat, endianness.

**Companion ADRs**: ADR-0040 (three-map split + HASH_OF_MAPS atomic-
swap primitive), ADR-0042 (`ServiceMapHydrator` reconciler +
`Action::DataplaneUpdateService` + `service_hydration_results`
observation table).

## Context

ADR-0040 locks the SERVICE_MAP / BACKEND_MAP / MAGLEV_MAP three-map
split. Three concrete questions remain at the algorithmic / wire
level:

1. **How are MAGLEV_MAP slot arrays computed and what `M` value does
   the inner-array carry?** Maglev consistent hashing (research § 5.2,
   § 5.3; NSDI 2016) is the published reference for ≤ 1 % flow
   disruption per single-backend removal under the M ≥ 100·N rule.
   Two algorithmic shapes exist: vanilla Maglev (uniform weights)
   and weighted Maglev (per-backend multiplicity expansion in the
   permutation generator). DISCUSS Decision 8 locked: ship weighted
   directly; no vanilla-then-weighted progression.

2. **How does the response path rewrite `(backend_ip, backend_port)`
   back to `(VIP, vip_port)` so external clients see consistent
   tuples?** This requires a 5-tuple-keyed REVERSE_NAT_MAP and an
   egress hook. Two egress hooks are credible: TC egress (kernel
   ≥ 4.4) and XDP egress (kernel ≥ 5.18).

3. **How is the byte-order contract maintained across the kernel /
   userspace boundary?** Wire packets carry network-order; map
   storage convention is host-order; the kernel-side hot path must
   convert at exactly the right place.

Each of these is locked by a structurally distinct decision.

## Decision

### 1. Weighted Maglev with M=16_381 default

The MAGLEV_MAP inner-array size — the Maglev `M` parameter — is
`16_381` by default. `MaglevTableSize` is a STRICT newtype in
`overdrive-core::dataplane::maglev_table_size` with full FromStr /
Display / serde / rkyv / proptest discipline. Default value
locked at 16_381 (the smallest prime ≥ 16_384; matches Cilium's
default, research § 5.3).

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
    /// prime list. The `M ≥ 100 · N` rule is enforced at
    /// backend-set-update time (separate concern; not at type
    /// construction).
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
//   - roundtrip Display ↔ FromStr for every prime in
//     ALLOWED_PRIMES;
//   - reject every non-prime u32 (exhaustive over a sampled
//     range);
//   - serde Deserialize validates via TryFrom<u32> (the
//     `try_from = "u32"` attribute is the load-bearing surface).
```

The `try_from = "u32"` attribute on `#[serde(...)]` makes
`Deserialize` validate per `development.md` § Newtype completeness
— a wire payload carrying a non-prime is rejected at the
deserialization boundary, not silently accepted.

Validating-constructor properties summarised:

- Membership in `ALLOWED_PRIMES` — Maglev's mathematical
  correctness depends on a prime modulus (research § 5.2); the
  prime list spans every operator-realistic table size from
  small-fanout (251) through high-fanout (131_071).
- The `M ≥ 100 · N` rule (research § 5.2 — the ≤ 1 % disruption
  bound) is **not** enforced at construction — `MaglevTableSize`
  is constructed before the backend-set is known. The rule is
  enforced at backend-set-update time inside
  `overdrive-dataplane::maglev::permutation`, where rejection
  surfaces as a structured `DataplaneError` from `update_service`,
  observable via `service_hydration_results` (ADR-0042).

Q6=A: operator-tunability surface deferred. The newtype ships now;
operator-config wiring (per-service M override via JobSpec)
composes when an operator-config aggregate exists in Phase 3+. The
deferral is one line of JobSpec edit, not a type-system change.

### 2. Weighted permutation via Eisenbud + multiplicity expansion in deterministic order

The userspace permutation generator lives in
`crates/overdrive-dataplane/src/maglev/permutation.rs` (Eisenbud
permutation) and `crates/overdrive-dataplane/src/maglev/table.rs`
(weighted multiplicity expansion).

Algorithmic shape:

1. Read `Vec<Backend>` in caller-provided iteration order. The
   action shim passes the slice straight through from the
   reconciler, which read it from `BTreeMap<BackendId, Backend>::iter()`.
   `BTreeMap` ordering is bit-deterministic (`development.md`
   § Ordered-collection choice).
2. Expand each backend `b` into `b.weight` distinct entries in the
   pre-permutation list (multiplicity expansion).
3. Generate the Eisenbud permutation `(offset[i], skip[i])` for
   each entry.
4. Fill the `[BackendId; M]` slot array via the standard Maglev
   "first available slot" loop.

The result is byte-deterministic across nodes given identical
inputs; this property is necessary for the
`HydratorIdempotentSteadyState` DST invariant (ADR-0042) and
sufficient for cross-node convergence in future Corrosion-driven
multi-node hydration (Phase 5).

The same byte-deterministic-archive property underpins the
`BackendSetFingerprint` type alias (a `pub type
BackendSetFingerprint = u64;` in
`crates/overdrive-core/src/dataplane/mod.rs`) used as the
convergence-detection key across the hydrator (ADR-0042 § 2),
the `Action::DataplaneUpdateService` payload (ADR-0042 § 1),
and the `service_hydration_results` row (ADR-0042 § 4).
`BackendSetFingerprint` is a type alias rather than a STRICT
newtype because the value is a derived hash with no canonical
human-typed form — see architecture.md § 6 *Type aliases* for
the full rationale and the canonical computation site at
`crates/overdrive-core/src/dataplane/fingerprint.rs`.

### 3. REVERSE_NAT_MAP shape

```
key   = ReverseKey {
    client_ip:    u32,
    client_port:  u16,
    backend_ip:   u32,
    backend_port: u16,
    proto:        u8,
    _pad:         [u8; 3],
}
value = OriginalDest {
    vip:      u32,
    vip_port: u16,
    _pad:     [u8; 2],
}
```

`max_entries = 1_048_576` (1 Mi entries). Operator-tunable in a
future ticket; Phase 2.2 fixed.

The map is written by the forward path of `xdp_service_map`
(`(client, vip, vip_port) → (client, backend, backend_port)`) and
read by the reverse path of `tc_reverse_nat`
(`(client, backend, backend_port) → (client, vip, vip_port)`).
Both paths key on the same `ReverseKey`; `proto` discriminates
TCP / UDP. The `_pad` fields are explicit zero-padding required for
8-byte alignment in BPF map storage (matches `BackendEntry` shape
in ADR-0040).

### 4. TC-egress for `tc_reverse_nat` (Q2=A)

Rationale:

- **Kernel-floor compatibility.** Phase 2.2's Tier 3 floor is the
  5.10 LTS lineage. XDP-egress requires kernel ≥ 5.18; TC-egress
  is stable since 4.4. Even single-kernel in-host on
  `ubuntu-latest` (currently 6.x), TC's maturity reduces verifier
  surprise.
- **Reference-shape alignment.** Cilium and Katran both use TC
  egress for the same concern.
- **aya 0.13 TC support is mature** (research § 4.3); no new
  primitive needed.

The program lives at
`crates/overdrive-bpf/src/programs/tc_reverse_nat.rs` and attaches
to the egress qdisc on the same iface as `xdp_service_map`'s XDP
hook. Userspace loader and attach mechanics in
`crates/overdrive-dataplane/src/loader.rs` extend with a `TcLink`
companion to the existing XDP attach path.

### 5. Endianness lockstep contract

**Wire format** — IPv4 packets carry IPs and L4 ports in network
byte order (big-endian). XDP / TC programs read them via
`*((__be32 *)&iph->saddr)`-equivalent aya helpers; the kernel
exposes these as `__be32` / `__be16`.

**Map storage format** — REVERSE_NAT_MAP keys / values are stored
in **host byte order** (little-endian on every kernel matrix entry
per `.claude/rules/testing.md` § Kernel matrix; x86-64 + aarch64
are both LE). This matches `BACKEND_MAP` storage. Userspace
control-plane code reads / writes the maps in host order without
`htonl` / `ntohl` calls; **only the kernel-side hot path performs
the conversion**.

**Conversion site** — a single `#[inline(always)]` Rust helper in
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

**Lockstep guarantees:**

- Tier 2 BPF unit tests include a roundtrip assertion: a synthetic
  packet with known wire-order bytes through `reverse_key_from_packet`
  produces the host-order `ReverseKey` the userspace test seeded
  into the map. Closes the Eclipse-review remediation note
  explicitly.
- A proptest in `overdrive-dataplane::maps::reverse_nat_map_handle`
  round-trips host-order writes against host-order reads to assert
  no userspace-side endian flip sneaks in.

The conversion-site centralisation is structural, not stylistic:
endianness drift across multiple sites is one of the most common
eBPF correctness bugs in production; the single-helper shape
makes it impossible by construction.

## Alternatives Considered

### A — Rendezvous hashing instead of Maglev

Rendezvous (HRW) hashing computes per-packet
`argmax(hash(packet, backend))` over the backend set. **Rejected**:
N hash computations per packet (vs O(1) Maglev table lookup);
verifier budget collapses; no atomic-swap primitive at the kernel
side. Maglev's pre-computed permutation table is the published
production answer (research § 5.2) and the only shape that fits
into the verifier-budget envelope.

### B — XDP-egress for `tc_reverse_nat`

Use the XDP-egress hook (kernel ≥ 5.18) for the reverse-NAT
program. **Rejected**: incompatible with the 5.10 LTS kernel
floor. Phase 2.2 deliberately stays portable across the LTS
matrix even though single-kernel in-host runs 6.x. A future
"XDP-egress" ticket can replace TC-egress when the floor uplifts;
the program shape is portable.

### C — Per-flow conntrack (instead of REVERSE_NAT_MAP)

Track every flow's forward decision in a kernel conntrack table;
reverse path looks up the original decision. **Rejected (out of
scope)**: this is GH #154. Phase 2.2's stateless Maglev forwarder
gives ≤ 1 % flow disruption per backend change as the interim
flow-affinity guarantee. Conntrack's exactly-once flow affinity
matters under aggressive churn + long-lived flows; #154 lands it
when those workloads materialise.

### D — Vanilla Maglev with weight in a follow-on slice

Ship vanilla Maglev (uniform weights) in this feature, weighted
Maglev in a follow-on slice. **Rejected** per DISCUSS Decision 8:
the weighted variant's algorithmic delta is in userspace
permutation generation; the kernel-side lookup shape is identical.
Splitting forces a verifier-baseline re-bump on the follow-on
slice with no algorithmic gain. Ship weighted directly.

### E — Network-order map storage

Store REVERSE_NAT_MAP keys in network order to match wire-format
reads. **Rejected**: matches `BACKEND_MAP` storage at host order
is the established convention; userspace code becomes simpler
(no `htonl` calls); the conversion at the kernel boundary is a
single `#[inline(always)]` helper that costs zero verifier budget
on x86 / aarch64 (both LE — `from_be` is a single byte-swap
instruction).

## Consequences

**Positive:**

- ASR-2.2-02 (≤ 1 % disruption per single-backend removal)
  becomes structurally achievable via M ≥ 100·N rule + Maglev
  pre-computed permutation.
- Weighted backends shipped from day one — `whitepaper.md` § 15's
  "weighted backends (e.g., 95 % v1, 5 % v2)" commitment lands in
  one slice, not two.
- Endianness-lockstep contract is centralised in one helper file;
  the most common eBPF correctness bug class is structurally
  prevented.
- Userspace permutation cost is bounded one-time-per-backend-set-
  change; the steady-state forward path is one MAGLEV_MAP lookup
  + one BACKEND_MAP lookup + one REVERSE_NAT_MAP write — three
  O(1) operations.
- Newtype `MaglevTableSize` ships now with full discipline; the
  per-service operator-tunability surface composes cheaply when
  an operator-config aggregate appears.

**Negative:**

- TC-egress instead of XDP-egress costs a small per-packet egress
  overhead vs the (kernel ≥ 5.18) XDP-egress alternative. Phase
  2.2 stays portable; a future ticket can swap.
- REVERSE_NAT_MAP at 1 Mi entries consumes ~32 MiB of kernel
  memory at full saturation (`sizeof(ReverseKey) +
  sizeof(OriginalDest) + overhead`). Operator-tunable size
  becomes an operational concern in Phase 3+; for Phase 2.2 the
  fixed size is well within node-RAM budgets.
- Userspace permutation generation at M=16_381 + 100 backends
  takes nontrivial CPU during atomic swap (DISCUSS Risk #5
  acknowledged); production rate is ops-per-minute scale.

**Operational implications:**

- Tier 2 endianness roundtrip + Tier 3 wire-capture verification
  (per ASR-2.2-02 measurement plan) gate the lockstep contract.
- Proptest `maglev_permutation_is_byte_deterministic` runs on
  every PR; flake-rate must stay zero.
- Per-service M overrides (Q6=B-deferred-to-Phase-3+) pre-
  validates against M ≥ 100·N at action-emit time; rejection
  surfaces as a structured `DataplaneError` from `update_service`,
  observable via `service_hydration_results` (ADR-0042).

## References

- `docs/feature/phase-2-xdp-service-map/design/architecture.md` § 6,
  § 10, § 11, § 14.
- `docs/feature/phase-2-xdp-service-map/design/wave-decisions.md`
  D4, D6, D7.
- `docs/research/networking/xdp-service-load-balancing-research.md`
  § 4.3, § 5.2, § 5.3.
- `docs/whitepaper.md` § 7 *eBPF Dataplane*, § 15 *Zero Downtime
  Deployments — weighted backends*.
- `.claude/rules/testing.md` § Tier 2 / § Tier 3 / § Kernel matrix.
- ADR-0038 (eBPF crate layout + build pipeline) — substrate.
- ADR-0040 (three-map split + HASH_OF_MAPS) — companion.
- ADR-0042 (`ServiceMapHydrator`) — companion.
