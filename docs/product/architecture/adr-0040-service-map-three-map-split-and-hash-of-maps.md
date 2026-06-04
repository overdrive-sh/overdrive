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

---

## Revision 2026-05-07 — Q3 amendment (sanity prologue is ingress-only)

### Status

Amendment. 2026-05-07. Decision-maker: Morgan. Tags: phase-2,
dataplane, sanity-prologue, tc-egress, xdp-ingress, skb-linearisation,
falsification-followup.

### Why this amendment

Decision 4 (Q3=C) above scoped the sanity prologue as a shared
`#[inline(always)]` Rust helper invoked from BOTH `xdp_service_map_lookup`
(ingress) AND `tc_reverse_nat` (egress). That decision was correct
for ingress and wrong for egress. The empirical evidence trail is
captured in ADR-0044 § Falsification (2026-05-07); the short summary:

S-2.2-17 (`real_tcp_connection_completes_through_vip_with_payload_echo`)
shows length-0 TCP segments passing through the dataplane and
length-N segments dropping. A Lima-side bpftrace + netstat + pcap
diagnostic on 2026-05-07 isolated the drop to
`SKB_DROP_REASON_TC_EGRESS = 51` from `dev_queue_xmit` on `lb_a`.
The only path in `tc_reverse_nat` that returns `TC_ACT_SHOT` is
`Verdict::Drop` from the sanity prologue — specifically the
`claimed_pkt_len > packet_len` check at
`crates/overdrive-bpf/src/programs/sanity.rs:259`.

The kernel-side rationale: when the kernel forwards an skb to TC
egress, the IPv4 `total_length` field includes the full L4 payload,
but the skb's linear-buffer length (`data_end - data` in BPF
context) may not. skb linearisation, GSO segmentation, and
forwarded-packet metadata can leave the linear region shorter than
what `total_length` advertises. Length-0 segments pass because
`total_length == header_bytes`. Length-N segments fail check (3)
because `claimed_pkt_len = ipv4_offset + total_len` exceeds
`packet_len` for forwarded skbs.

### Amendment

Q3 is amended to scope the sanity prologue helper to **XDP ingress
only** (`xdp_service_map_lookup`). The TC egress program
(`tc_reverse_nat`) MUST NOT call the prologue.

The egress program does not need its own packet-shape validation:
the ingress program is the enforcement point, and any packet
reaching TC egress on `lb_a` has already passed XDP ingress sanity
checks on `lb_veth_a`. Re-running the prologue at egress is not
defence-in-depth — it is a check whose preconditions (linear-buffer
length matches IPv4 `total_length`) the kernel does not preserve
through forwarding, so the check fires spuriously on every length-N
forwarded segment.

### Concretely

The decision the original Q3=C locked has TWO components:

1. **Helper shape — shared `#[inline(always)]` Rust function.** This
   component stands. Ingress callers continue to import the helper
   from `crates/overdrive-bpf/src/shared/sanity.rs`.
2. **Call sites — ingress AND egress.** This component is amended.
   The egress call site is removed.

The actual code change (removing the call from `tc_reverse_nat`) is
the crafter's responsibility in a follow-up dispatch; this ADR
captures the DECISION, not the implementation.

### Consequences of the amendment

**Positive:**

- S-2.2-17 closes structurally without a conntrack table, without
  NOTRACK, without changing the Phase 2.2 architecture envelope.
  The prologue remains a load-bearing ingress check; the egress
  path stays as it was before Slice 06-02 landed.
- Phase 2.16 (the proposed dataplane-owned conntrack feature) is
  retracted. ADR-0044 is marked SUPERSEDED.
- The Tier 4 verifier-budget envelope improves: removing one helper
  invocation from `tc_reverse_nat` is a small but measurable
  reduction.

**Negative:**

- The egress program no longer carries a structural sanity check.
  Acceptable: XDP ingress is the enforcement point, and forwarded
  skbs at TC egress are kernel-vouched for in a way the prologue's
  `claimed_pkt_len > packet_len` check is not equipped to validate.

**Operational:**

- Slice 06-02's existing scope (the prologue helper itself) stays
  intact. Only the Slice 06-04 attempt to reuse the prologue at TC
  egress is undone.
- The in-flight 06-04 working-tree files (NOTRACK bridge variant,
  IptablesInstall action, etc.) become moot in their conntrack-
  framed shape. The crafter handling 06-04 in the follow-up
  dispatch decides whether to land the prologue-removal-from-egress
  fix as 06-04 or as a renumbered slice.

### Cross-references

- ADR-0044 (`adr-0044-xdp-conntrack-percpu-lru.md`) — SUPERSEDED;
  carries the falsification record at top of file.
- `docs/research/dataplane/length-n-tcp-drop-veth-xdp-tc-reverse-nat-research.md`
  § Update 2026-05-07 — RECOMMENDATION FALSIFIED.
- `docs/research/dataplane/cilium-bpf-fib-lookup-l2-mac-rewrite-comprehensive-research.md`
  § Update 2026-05-07 — primary findings stand; downstream
  conntrack inference falsified.
- `crates/overdrive-bpf/src/programs/sanity.rs:259` — the
  `claimed_pkt_len > packet_len` check that fires spuriously on
  forwarded skbs.
- CLAUDE.md § "Documentation" / `.claude/rules/development.md`
  § "No aspirational docs" — this amendment captures the decision;
  the code change is the crafter's responsibility, not this
  ADR's.

### Changelog (Revision 2026-05-07)

| Date | Change |
|---|---|
| 2026-05-07 | Q3 amendment: sanity prologue scope narrowed from {ingress, egress} to {ingress only}. Empirical falsification trail in ADR-0044. — Morgan. |

---

## Revision 2026-05-07 (later) — Q2 reopened (kernel IP-forward + TCX-egress retired)

### Status

Amendment. 2026-05-07. Decision-maker: Morgan. Tags: phase-2,
dataplane, q2-reopen, bpf-redirect-neigh, supersession-pointer,
falsification-followup-2.

**GitHub tracking issue**: #159 — *[2.x] Replace IP-forward +
TCX-egress with bpf_redirect_neigh datapath*. Production work for
this amendment lives under that issue.

### Why this amendment

The Q3 amendment earlier today scoped the sanity prologue to XDP
ingress only after probes 1–4 falsified the egress-side prologue
invocation. Continued investigation through probes 5–7 (recorded in
`docs/analysis/e1-bpftrace-results.md`) extended the falsification
to the *entire TCX-egress reverse-NAT shape* locked by Q2=A above.

The short summary (full causal chain in ADR-0045 § "The empirical
chain that falsified the locked datapath"):

S-2.2-17 still drops length-N TCP segments at
`__dev_queue_xmit → qdisc_pkt_len_init →
kfree_skb_reason(SKB_DROP_REASON_TC_EGRESS = 51)` on the
client-facing veth egress, *before* the TCX dispatcher invokes
`tc_reverse_nat`. Probe 5's `kernel.bpf_stats_enabled` polling
shows the loaded program has `run_cnt = 16` during the test window;
probe 6's `pwru` per-skb trace shows the *dropped* skb hits zero
TC-classifier dispatchers — the program's invocations are firing on
ARP frames and other unrelated traffic, not on the data segments.
Probe 7's drop-site skb metadata identifies the unique signature:
`data_len=20, nr_frags=1` paged skbs with stale CHECKSUM_PARTIAL
metadata (`csum_start=288, csum_offset=16, ip_summed=0`) left over
from the kernel's `pskb_expand_head + skb_checksum_help` sequence
during IP-forwarding. The kernel's egress pre-classifier rejects
these skbs before TCX runs.

This is a structural defect of the *locked architecture* (kernel
IP-forwarder in the request data path, TCX-egress reverse-NAT on
the response path), not of `tc_reverse_nat`'s body. Test-fixture
mitigations (`ethtool -K $iface tso off gso off gro off`) were
falsified during the probe chain — the helper already disabled
offloads; the paged-skb shape comes from veth-peer delivery
semantics, not iface offload settings.

There is no in-program fix. The kernel mechanism that produces the
bug must be removed from the path entirely.

### Amendment

**Q2 is reopened.** The TC-egress reverse-NAT shape locked by Q2=A
is empirically falsified for the data-bearing skb path under
Linux 6.8 (and structurally on any kernel where `pskb_expand_head`
+ `skb_checksum_help` interact with paged skbs the same way — the
mechanism is not specific to one LTS).

**The resolution moves to a new ADR**: ADR-0045 (`adr-0045-bpf-
redirect-neigh-datapath.md`) locks the post-pivot architecture
— XDP-ingress L3+L2 rewrite + `bpf_fib_lookup` +
`bpf_redirect_neigh`, on both the client-facing veth (request path)
and the backend-facing veth (response path). The kernel IP-forwarder
is removed from both directions; `tc_reverse_nat` is retired as a
TC program; its reverse-NAT logic moves to a new XDP program
(`xdp_reverse_nat_lookup`) attached on the backend-facing veth
ingress. See ADR-0045 for the full decision and alternatives.

### What this amendment supersedes vs preserves in Q1–Q7 above

| Original decision | Status |
|---|---|
| Q1=A (kernel-helper checksum choice — `bpf_l3_csum_replace`, `bpf_l4_csum_replace`) | **Preserved.** The L3/L4 incremental checksum update runs on both post-pivot XDP programs identically. |
| Q2=A (TC-egress reverse-NAT, kernel IP-forward in the data path) | **Superseded by ADR-0045.** |
| Q3=C (sanity prologue helper, ingress-only after the earlier 2026-05-07 amendment) | **Preserved.** Both post-pivot programs are XDP-ingress, so the ingress-only scope is structurally satisfied. |
| Decision 1 (three-map split: SERVICE_MAP / BACKEND_MAP / MAGLEV_MAP) | **Preserved in full.** Map shapes, key types, and inner-map structure are direction-agnostic. |
| Decision 2 (atomic swap via HASH_OF_MAPS outer-map fd replacement) | **Preserved in full.** The atomic-swap primitive is independent of how packets traverse the dataplane. |
| Q5=A (HASH_OF_MAPS inner-map size 256) | **Preserved.** |
| Q7=B (DROP_COUNTER 6 slots) | **Preserved.** No new `DropClass` variant required by the pivot. |

The original Q2=A reasoning above is **not deleted** — it remains
historically accurate as the decision made on the evidence
available at the time. The supersession is recorded by this
amendment, not by rewriting the original decision body.

### Concretely

The architectural decisions that were made under Q2=A's evidence
chain — TC-egress attach for `tc_reverse_nat`, kernel IP-forwarder
in the request data path, `tc_reverse_nat` body reading and
rewriting REVERSE_NAT_MAP entries on response — see ADR-0045 § 1–3
for the post-pivot shape. The reverse-NAT *logic* is preserved
verbatim; only the *attach layer* moves from TCX-egress on the
client-facing veth to XDP-ingress on the backend-facing veth.

The actual code changes (deleting `tc_reverse_nat.rs`, adding
`xdp_reverse_nat.rs`, extending `xdp_service_map.rs` with FIB +
`bpf_redirect_neigh`, retiring the loader's TC-link plumbing) are
the crafter's responsibility under GH #159 — this amendment
captures the DECISION that the change is needed; the new ADR
captures the SHAPE; the crafter implements.

### Consequences of the Q2 reopen

**Positive:**

- S-2.2-17 closes structurally without further test-fixture
  experimentation. The kernel mechanism that produces the bug is
  removed from the path entirely.
- The dataplane aligns with the published reference (Cilium L4LB
  shape) on stable kernels, removing the project's exposure to
  "we are the only ones doing it this way."
- Future kernel-version sensitivity is reduced — `bpf_fib_lookup` +
  `bpf_redirect_neigh` semantics are stable from 5.10 onwards (the
  project's floor); the pre-pivot path's failure mode was sensitive
  to kernel-version-specific paged-skb handling.

**Negative:**

- Slice 05's TC-egress reverse-NAT work is partially obsoleted (the
  REVERSE_NAT_MAP shape and endianness lockstep contract from
  ADR-0041 are preserved; the TC-egress attach + `tc_reverse_nat`
  body are retired).
- Slice 06-05's veristat baseline is reset to the new program
  shape; trend tracking restarts. A single-PR cost.
- One-time engineering cost for the new XDP program, the extension
  of `xdp_service_map_lookup`, and loader changes. Bounded; tracked
  under #159.

**Operational:**

- ADR-0040 stays the SSOT for the SERVICE_MAP / BACKEND_MAP /
  MAGLEV_MAP / HASH_OF_MAPS shape decisions (Decisions 1, 2, Q5, Q7).
- ADR-0045 is the SSOT for the post-pivot dataplane shape
  (request-path forwarding, response-path reverse-NAT, FIB lookup,
  `bpf_redirect_neigh` semantics).
- ADR-0041 (REVERSE_NAT_MAP shape, endianness lockstep) is
  unaffected.
- ADR-0042 (`ServiceMapHydrator`) is unaffected.
- ADR-0043 (3-iface test topology) is unaffected.

### Cross-references

- ADR-0045 (`adr-0045-bpf-redirect-neigh-datapath.md`) — the
  resolution ADR for the reopened Q2.
- `docs/analysis/e1-bpftrace-results.md` probes 1–7 — empirical
  evidence trail.
- GH #159 — production work under this amendment.
- ADR-0044 (`adr-0044-xdp-conntrack-percpu-lru.md`) — already
  marked SUPERSEDED by the earlier 2026-05-07 amendment; this
  Q2 reopen is independent and consistent.

### Changelog (Revision 2026-05-07, later)

| Date | Change |
|---|---|
| 2026-05-07 (later) | Q2 reopened: TC-egress reverse-NAT + kernel IP-forward in the data path superseded by ADR-0045's `bpf_redirect_neigh` datapath. Empirical falsification chain in `docs/analysis/e1-bpftrace-results.md` probes 1–7. Tracked under GH #159. — Morgan. |

---

## Revision 2026-06-03 — SERVICE_MAP outer key gains L4 protocol (`(VIP, port)` → `(VIP, port, proto)`)

### Status

Amendment. 2026-06-03. Decision-maker: Morgan; user-locked
(resolves P2-Q4 on the `udp-service-support` feature — "do
`(vip, port, proto)` as IPVS"). Tags: phase-2, dataplane,
service-map, l4-proto-keying, ipvs-alignment, udp-service-support.

**Feature SSOT**: `docs/feature/udp-service-support/feature-delta.md`
§ "Wave: DESIGN / [REF] P2-Q4 resolution — proto in the service-LB
map keys". **Decision record**:
`docs/feature/udp-service-support/design/wave-decisions.md` (P2-Q4).
**Evidence base**:
`docs/research/dataplane/service-map-l4-proto-keying-research.md`
(Nova, 2026-06-03, High confidence, 13 trusted-domain sources).

### Why this amendment

Decision 1 above locked the `SERVICE_MAP` outer-map key as
`(ServiceVip, u16 port)` — **proto-less**. That shape cannot
represent two services that share a VIP **and** a port but differ in
L4 protocol — the canonical case being DNS (`tcp/53 + udp/53` on one
VIP) and the fast-growing HTTP/3 case (`443/tcp` HTTPS alongside
`443/udp` QUIC). Both listeners hash to a single outer-map slot under
a proto-less key; the second listener overwrites the first.

The research is decisive that proto-less keying is the wrong
architecture, not merely an under-optimisation:

- **Linux IPVS keys every virtual service on `{protocol, addr, port}`
  natively** (UAPI `struct ip_vs_service_user`: `__u16 protocol;
  __be32 addr; __be16 port;`). Protocol is a *keying* field, not a
  config option. kube-proxy's iptables mode emits per-protocol rule
  chains. In the two oldest, most-deployed Kubernetes dataplanes,
  proto is in the service key by construction (research § Q2).
- **Cilium's eBPF `lb4_key` now carries `__u8 proto` and treats
  omitting it as the defect it spent ~5.5 years closing** — issue
  #9207 (opened 2019-09-16, "do not differentiate between UDP and TCP
  services") → PR #37164 (merged 2025-01-23). The `proto` byte sat
  reserved-but-unused (`IPPROTO_ANY`/0) the entire time; during that
  window Cilium could not place TCP and UDP services on the same port
  — the exact CoreDNS case (research § Q1). Proto-less was a known
  bug, not a valid long-term model anywhere.
- **Kubernetes treats TCP+UDP-on-same-port as first-class** — the
  `MixedProtocolLBService` feature gate, the canonical CoreDNS
  Service shape (`name: dns-tcp / dns-udp`, both `port: 53`), and the
  AWS/Istio/Emissary dual-listener-on-443 pattern (research § Q4).
- **Widening a HASH_OF_MAPS outer key is structurally free** — the
  kernel docs place no size penalty on a composite POD outer key
  (research § Q3), and Overdrive's `ServiceKey` is already an 8-byte
  `#[repr(C)]` POD with a zeroed 2-byte `_pad`; the proto byte
  consumes one already-reserved pad byte with **no change to the
  map's byte width** (mirrors Cilium's own `__u8 proto; __u8 scope;
  __u8 pad[2]` tail).

This amendment is the SERVICE_MAP-forward-key half of the proto
dimension. Its companion — proto in the **REVERSE_NAT** key, the #163
response-path surface — is already locked by ADR-0060 (`BackendKey {
ip, port, proto }`), and proto is already present at the dataplane
boundary via `ServiceFrontend { vip, port, proto }` (ADR-0060). This
amendment threads that same proto into the SERVICE_MAP **forward**
key, closing the OQ-1 / D8 deferral that ADR-0060 left open.

### Amendment

Decision 1's `SERVICE_MAP` row is amended. The outer-map key changes
from `(ServiceVip, u16 port)` to `(ServiceVip, u16 port, Proto)`:

| Map | Type | Key (amended) | Value | Purpose |
|---|---|---|---|---|
| `SERVICE_MAP` | `BPF_MAP_TYPE_HASH_OF_MAPS` (outer) | **`(ServiceVip, u16 port, Proto)`** | inner-map fd | `(VIP, port, proto)`-to-inner-map indirection. A TCP listener and a UDP listener on the same `(VIP, port)` occupy **two distinct outer-map slots**. |

`BACKEND_MAP` (keyed by `BackendId`) and `MAGLEV_MAP` (keyed by
`ServiceId`) are **unchanged** — neither is keyed by the wire
`(VIP, port)` tuple, so neither needs proto. The proto dimension is
purely a SERVICE_MAP-outer-key concern on this map-split.

#### Struct layout — absorb the pad byte, keep 8 bytes, keep deterministic hashing

The kernel-side `ServiceKey`
(`crates/overdrive-bpf/src/maps/service_map.rs:74-78`) and its
userspace mirror
(`crates/overdrive-dataplane/src/maps/service_map_handle.rs:59-66`)
both carry a trailing `_pad: u16`. The proto byte consumes one of the
two pad bytes; the struct **stays 8 bytes**:

```rust
// from-state (proto-less, 8 bytes)
#[repr(C)]
pub struct ServiceKey {
    pub vip_host: u32,
    pub port_host: u16,
    pub _pad: u16,        // 2 reserved bytes, zeroed
}

// to-state (proto in key, still 8 bytes — mirrors Cilium's
// `__u8 proto; __u8 scope; __u8 pad[2]` tail)
#[repr(C)]
pub struct ServiceKey {
    pub vip_host: u32,
    pub port_host: u16,
    pub proto: u8,        // IANA L4 proto: IPPROTO_TCP=6 / IPPROTO_UDP=17
    pub _pad: u8,         // 1 trailing pad byte, MUST stay zeroed
}
```

Two load-bearing layout disciplines, both grounded in the research's
implementation note (§ Q3):

1. **The trailing `_pad: u8` MUST be deterministically zero-initialised**
   (the existing codebase convention already zeroes `_pad`). BPF hash
   maps key on the raw struct bytes; a non-zero or uninitialised pad
   byte makes two logically-equal keys hash to different slots. This
   is a one-line discipline (`_pad: 0` in every constructor), not a
   structural obstacle — and the codebase already does it for the
   `u16` pad.
2. **`proto` is lowered to the kernel `u8` at the map-write edge** —
   the userspace handle accepts the typed `Proto` enum (ADR-0060's
   `overdrive_core::dataplane::backend_key::Proto`, IANA-valued via
   `Proto::as_u8()` → 6/17) and writes the `u8` discriminant into the
   key struct. The kernel-side `xdp_service_map_lookup` already reads
   `proto` from the IPv4 header
   (`crates/overdrive-bpf/src/programs/xdp_service_map.rs:247`) and
   builds the key at `:268`; the proto byte slots into the existing
   key construction with no new packet parsing.

### Migration — single-cut, reconciler-repopulated; no shim

Per `feedback_single_cut_greenfield_migrations.md` and
`.claude/rules/reconcilers.md`: the `SERVICE_MAP` is repopulated from
intent on agent boot by the `ServiceMapHydrator` reconciler
(ADR-0042). The migration is therefore "**the key struct changes;
the map is recreated on next boot**" — there is **NO** live in-place
migration, **NO** dual-key compatibility shim, **NO** deprecation
path, and **NO** generation-counter cutover. The research (§ Q5)
contrasts this explicitly with Cilium, whose proto cutover was
hazardous *because* of live in-place upgrade with established
connections (issue #13529); Overdrive's reconciler-repopulated,
single-cut posture removes that hazard entirely. DELIVER must NOT
build a migration shim — the key struct edit + the hydrator's
existing repopulation IS the migration.

### What this amendment supersedes vs preserves

| Original decision | Status |
|---|---|
| Decision 1 — three-map split (SERVICE_MAP / BACKEND_MAP / MAGLEV_MAP) | **Preserved in shape.** Only the SERVICE_MAP *outer-key tuple* widens; the map *types*, the inner-map structure, and the BACKEND_MAP / MAGLEV_MAP keys are unchanged. |
| Decision 1 — SERVICE_MAP outer key `(ServiceVip, u16 port)` | **Amended** to `(ServiceVip, u16 port, Proto)`. |
| Decision 2 — atomic swap via HASH_OF_MAPS outer-fd replacement | **Preserved verbatim.** The atomic-swap primitive is per-outer-slot; a wider outer key means more slots, not a different swap mechanism. The research (§ Q3) confirms outer-key widening is independent of nesting/value mechanics. |
| Q1=A (kernel-helper checksum), Q3=C (sanity prologue, ingress-only), Q5=A (inner-map size 256), Q7=B (6 DropClass slots) | **Preserved.** None interacts with the outer-key tuple. |
| Q2 (datapath shape; reopened, superseded by ADR-0045) | **Unaffected.** The forward/response datapath is orthogonal to the outer-key tuple. |

### Consequences

**Positive.**

- The DNS case (`tcp/53 + udp/53` on one VIP) and the HTTP/3 case
  (`443/tcp + 443/udp`) are representable on day one — the SERVICE_MAP
  forward path installs two distinct outer-map slots, one per proto.
- Aligns the forward key with IPVS / kube-proxy (proto-in-key) and
  with Cilium's post-#37164 `lb4_key` — removes Overdrive's exposure
  to "we are the only L4 LB keying proto-less."
- Symmetry with the response path: the REVERSE_NAT key already carries
  proto (ADR-0060 `BackendKey { ip, port, proto }`); the forward key
  now matches. The `(VIP, port, proto)` frontend and the
  `(ip, port, proto)` backend key share one proto dimension end-to-end.
- Zero byte-width cost: 8-byte key before and after; one reserved pad
  byte is consumed.

**Negative / accepted.**

- The kernel-side and userspace `ServiceKey` structs change layout
  (proto byte at offset 6, pad narrows to 1 byte). Single-cut per the
  migration section; the endianness-lockstep proptest
  (`service_map_handle.rs:262-314`), the Tier 2 lookup test
  (`crates/overdrive-bpf/tests/integration/xdp_service_map_lookup.rs`),
  and the Tier 3 forward/swap tests
  (`service_map_forward.rs`, `atomic_swap.rs`,
  `multi_listener_tcp_udp_e2e.rs`) update in the same PR. These are
  DELIVER concerns, noted here for blast-radius visibility, not
  authored by this ADR.
- `OQ-1` / `D8` (SERVICE_MAP forward-key granularity — VIP-only per
  the shipped `validate.rs:218` vs `(VIP, port)` per architecture.md
  § 5 Drift-3) is **subsumed** by this amendment: the forward key is
  now unambiguously `(VIP, port, proto)`. The `validate.rs` write-key
  classifier (`port_opt: None`) must widen to carry port **and** proto
  to match; that is a DELIVER site, flagged in the feature-delta.

### Endianness note

No new endianness discipline (consistent with ADR-0060 § "Endianness
note (D7)"). `proto` is a single IANA byte (`Proto::as_u8()` → 6/17)
with no byte-order concern. The § 11 host-order/network-order lockstep
continues to govern `vip_host` and `port_host` only. The kernel-side
program reads the IPv4-header proto byte directly (no swap needed for
a single byte); userspace writes the `u8` discriminant directly.

### Cross-references

- `docs/research/dataplane/service-map-l4-proto-keying-research.md`
  — the citable evidence base (Cilium #9207/#37164/#13529, IPVS
  `ip_vs_service_user`, Kubernetes `MixedProtocolLBService`, kernel
  `map_of_maps` docs).
- ADR-0060 (`ServiceFrontend { vip, port, proto }` at the dataplane
  boundary; REVERSE_NAT key carries proto) — the companion that put
  proto on the boundary; this amendment threads it into the
  SERVICE_MAP forward key. ADR-0060 § "Flagged for US-05 (D8)" is the
  deferral this amendment closes.
- ADR-0053 (LOCAL_BACKEND_MAP) — its companion 2026-06-03 amendment
  threads the same proto dimension into the same-host cgroup path.
- ADR-0042 (`ServiceMapHydrator`) — the reconciler that repopulates
  SERVICE_MAP on boot; the migration mechanism.
- `crates/overdrive-bpf/src/maps/service_map.rs:74-78` (kernel key),
  `crates/overdrive-dataplane/src/maps/service_map_handle.rs:59-66`
  (userspace mirror), `:73-84` (`from_vip_port` builder — gains a
  proto param),
  `crates/overdrive-bpf/src/programs/xdp_service_map.rs:247,268`
  (proto already read; key built).

### Changelog (Revision 2026-06-03)

| Date | Change |
|---|---|
| 2026-06-03 | SERVICE_MAP outer key `(ServiceVip, u16 port)` → `(ServiceVip, u16 port, Proto)`, IPVS-style. Proto byte absorbs one of the two reserved `_pad` bytes; struct stays 8 bytes; trailing pad stays zeroed for deterministic hashing. Single-cut reconciler-repopulated migration; no shim. Resolves P2-Q4 (`udp-service-support`) and subsumes OQ-1/D8. Evidence: `service-map-l4-proto-keying-research.md`. — Morgan (user-locked). |

---

## Revision 2026-06-03 (companion) — `ServiceId` derivation gains L4 protocol; `MAGLEV_MAP` outer key re-partitions by proto (`(vip, port, purpose)` → `(vip, port, proto, purpose)`)

### Status

Amendment. 2026-06-03. Decision-maker: Morgan; user-locked
(completes P2-Q4 at the control-plane-identity layer — the user
directed **Model A**: one `ServiceId` per `(vip, port, proto)`
dataplane slot). Tags: phase-2, dataplane, service-id, maglev-map,
l4-proto-keying, ipvs-alignment, udp-service-support,
control-plane-identity.

**Companion of** the SERVICE_MAP-outer-key revision immediately above
(same date) — that revision widened the *wire-shape* forward key;
this revision widens the *control-plane-shape* identity (`ServiceId`)
and therefore the `MAGLEV_MAP` outer key, closing the half of P2-Q4
that revision explicitly deferred ("`MAGLEV_MAP` … is **unchanged**").

**Feature SSOT**:
`docs/feature/udp-service-support/design/wave-decisions.md`
(§ "P2-Q4 ServiceId-layer completion — Model A"). **Decision record**:
this ADR. **Implementation predicate** (DELIVER, not authored here):
widen `ServiceId::derive` + thread `proto` through the three
production derive sites
(`crates/overdrive-control-plane/src/listener_facts.rs:100`,
`crates/overdrive-control-plane/src/reconciler_runtime.rs:1681`,
`:1799`), their test mirrors, and the observability guard.

### Why this amendment (the gap the SERVICE_MAP revision left open)

The companion SERVICE_MAP revision above widened the **wire-boundary
outer key** to `(ServiceVip, u16 port, Proto)` and explicitly recorded
that `MAGLEV_MAP` (keyed by `ServiceId`) was "**unchanged** — neither
is keyed by the wire `(VIP, port)` tuple, so neither needs proto."
That statement was correct *for the wire key in isolation* but left
`ServiceId` itself proto-less — and `ServiceId` is the
control-plane-side identity that:

- is the **`MAGLEV_MAP` outer-map key** (Decision 1, this ADR — "one
  inner per service"), and
- is the **`service/<id>` `TargetResource`** the reconciler runtime
  reconciles (`reconciler_runtime.rs`), and
- is the **content-addressed, persisted (rkyv), gossiped** identity
  backing observation rows and per-listener control-plane projections.

`ServiceId::derive` content-addresses `(vip, port, purpose)`
(`crates/overdrive-core/src/id.rs:797-836`). With the wire key now
proto-distinct but `ServiceId` still proto-less, `tcp/53` and `udp/53`
on one VIP derive **the same `ServiceId`** — and the proto-distinct
SERVICE_MAP slots the companion revision landed **cannot be
populated** for the same-port/different-proto case, because the
control-plane layer that drives the per-listener fan-out collapses the
two listeners into one identity *before* the dataplane is ever
touched. Concretely the collapse fires at three control-plane sites:

1. `ListenerFactStore.primary: BTreeMap<ServiceId, ListenerRow>` —
   the second listener's `primary.insert` silently overwrites the
   first (`listener_facts.rs:97-108`).
2. The `BackendDiscoveryBridge` desired-side projection
   `BTreeMap<ServiceId, ProjectedListener>` — same collapse
   (`reconciler_runtime.rs:1716-1808`).
3. The `service_lifecycle` per-listener
   `ServiceDataplaneIdentity` derivation
   (`reconciler_runtime.rs:1680-1686`).

So P2-Q4's verbatim guarantee — *"each listener gets its own
`(VIP, port, proto)` key … no listener overwrites another"*
(`wave-decisions.md`) — was honored at the *dataplane slot* layer and
**broken at the `ServiceId` layer**. The two are not independent: the
control-plane identity is the thing that *drives* the per-slot writes.
CoreDNS (`tcp/53 + udp/53`) is the canonical day-one driver, and under
the proto-less `ServiceId` it is unrepresentable end-to-end. This
amendment closes that gap.

### Amendment

#### `ServiceId::derive` — add the proto axis

The derivation defined in Decision 1 / § 1 of this ADR (and
*referenced* by ADR-0052 § 1) gains an L4-protocol input. The
signature changes from

```text
ServiceId::derive(vip: &ServiceVip, port: NonZeroU16, purpose: &str) -> ServiceId
```

to

```text
ServiceId::derive(vip: &ServiceVip, port: NonZeroU16, proto: Proto, purpose: &str) -> ServiceId
```

where `Proto` is `overdrive_core::dataplane::backend_key::Proto` (the
same type ADR-0060 / P2-Q4 use, IANA-valued via `Proto::as_u8()` → 6 /
17). `ServiceId` content-addresses **one dataplane slot per
`(vip, port, proto)`** — `tcp/53` and `udp/53` become two distinct
`ServiceId`s.

#### Hash input — proto byte slots in deterministically, zero-separated

Per `.claude/rules/development.md` § "Hashing requires deterministic
serialization", the proto byte enters the SHA-256 pre-image as its
canonical IANA value, zero-separated like the existing inputs. The
**exact byte order** of the hash input becomes (proto is inserted as
the third field, between `port` and `purpose`):

| # | Bytes fed to `Sha256::update` | Source |
|---|---|---|
| 1 | `vip.to_string().as_bytes()` | `ServiceVip` `Display` (canonical `IpAddr::fmt` wire form) |
| 2 | `[0u8]` | zero separator |
| 3 | `port.get().to_be_bytes()` | big-endian `u16` |
| 4 | `[0u8]` | zero separator |
| 5 | `[proto.as_u8()]` | **new** — single IANA byte (TCP=6 / UDP=17) |
| 6 | `[0u8]` | **new** — zero separator |
| 7 | `purpose.as_bytes()` | namespacing token (canonically `"service-map"`) |

The first 8 bytes of the digest are interpreted big-endian as the
`u64`, unchanged. Proto sits at **input field 5** (the 5th
`update` call, after the second separator and before the `purpose`
token). The byte position is load-bearing: inserting proto
*before* `purpose` rather than appending it after keeps the
human-readable namespacing token last (matching the
`(addr, port, proto)` field order IPVS and `BackendKey` use), and
fixes a single canonical pre-image the implementing crafter must
reproduce bit-for-bit.

No endianness concern for the proto byte itself — it is a single
IANA scalar (consistent with D7 and the SERVICE_MAP revision's
"Endianness note"). The § 11 host-order/network-order lockstep
continues to govern `vip` / `port` only.

#### `MAGLEV_MAP` outer key

Decision 1's `MAGLEV_MAP` row keyed by `ServiceId` is **unchanged in
type** (`ServiceId` stays a `u64`; the map is still
`BPF_MAP_TYPE_HASH_OF_MAPS` keyed by `ServiceId`), but its
**partitioning changes**: because `ServiceId` now content-addresses
`(vip, port, proto)`, each proto gets its own `MAGLEV_MAP` inner table
(its own Maglev permutation). Where Decision 1 said "one inner per
service," the precise reading post-amendment is "**one inner per
`(vip, port, proto)` slot**" — a TCP service and a UDP service on the
same `(VIP, port)` now have two independent Maglev tables, which is
the correct behaviour (their backend sets and weightings are
independent).

`BACKEND_MAP` (keyed by `BackendId`, flat global namespace) is
**unaffected** — backends are shared across services regardless of
proto.

### Model A (locked) vs Model B (rejected)

The companion gap admits two structurally distinct resolutions. The
user directed **Model A**.

**Model A — `ServiceId` = content-address of `(vip, port, proto, purpose)` (LOCKED).**
One `ServiceId` per dataplane slot. The fix is to widen `derive()` and
thread the listener's `proto` through the three production derive
sites; the existing one-listener-per-entry projection shape
(`BTreeMap<ServiceId, ProjectedListener>`,
`BTreeMap<ServiceId, ListenerRow>`) **stays as-is** — each proto now
occupies a distinct key, so no entry collides. Model A is
**consistent with the already-widened SERVICE_MAP / LOCAL_BACKEND_MAP
keys** (the control-plane identity matches the wire/cgroup keys on the
same `(vip, port, proto)` axis) and gives each proto its own
`MAGLEV_MAP` table for free.

**Model B — `ServiceId` stays per-`(vip, port)`; projections become one-to-many (REJECTED).**
Keep `ServiceId` as a coarser "service" identity (one per
`(VIP, port)`, both protos under it) and change the projection *data
shape* to fan out per proto inside each entry —
`BTreeMap<ServiceId, Vec<ProjectedListener>>` (or re-key the
projections on a composite `(ServiceId, Proto)`). Rejected for three
reasons:

1. **Larger structural change to the projection data shape.** Model B
   rewrites the value type of `ListenerFactStore.primary`, the
   bridge's per-listener projection, and every consumer that iterates
   them — versus Model A, which changes only the *key derivation* and
   leaves the `BTreeMap<ServiceId, _>` shape intact.
2. **Diverges from the per-listener fan-out model P2-Q4 already
   assumes.** The hydrator emits **one `update_service` per listener**
   (ADR-0060); P2-Q4 records that this "maps 1:1 onto distinct
   proto-keyed slots." A one-`ServiceId`-per-slot identity is the
   natural 1:1 partner of one-`update_service`-per-listener. Model B
   reintroduces a 1:many indirection the rest of the design has
   already shed.
3. **Leaves `ServiceId` semantically inconsistent with the
   proto-keyed dataplane.** The SERVICE_MAP forward key and the
   LOCAL_BACKEND_MAP cgroup key are both now `(vip, port, proto)`;
   under Model B the control-plane identity that *drives* those writes
   would remain proto-less — the same split that caused this gap,
   re-entrenched one layer up.

**The one piece of evidence that hints at Model B, and why it does
not carry.** The `reconcile_conflict` observation row records **both**
a `service_id` field **and** a `(vip, vip_port, proto)` tuple
(`reconciler_runtime.rs:1168`,
`ConflictingServiceWrites { service_id, vip, vip_port, proto, … }`).
A reader could take this as evidence that `ServiceId` is *coarser
than* `(vip, port, proto)` — i.e. that the tuple disambiguates within
a `ServiceId` (Model B). It does not, under Model A: post-amendment
the `service_id` **is** the content-address of `(vip, vip_port,
proto)`, so the two are two encodings of the **same** conflict slot —
the opaque `u64` identity, plus its human-readable `(vip, port,
proto)` decode for operator queries (`bpftool`/`reconcile_conflict`
output is unreadable as a bare `u64`). The tuple is therefore
**retained as intended debugging/observability granularity, not
removed as redundant** — it is the legible projection of the
identity, the same way `BackendKey`'s `Display` renders
`"<ip>:<port>/<proto>"` rather than a hash. Keeping it costs nothing
and serves operators; it is not a latent argument for Model B.

### Migration — single-cut, reconciler-repopulated; no shim

Per `feedback_single_cut_greenfield_migrations.md` and the companion
SERVICE_MAP revision: the derived `ServiceId` values change for
**every** service (proto now participates in the hash), so every
`MAGLEV_MAP` outer key, every `service/<id>` `TargetResource`, and
every persisted observation row keyed on the old `ServiceId` is
recomputed/recreated on next agent boot by the existing reconcilers
(`ServiceMapHydrator` for the dataplane maps; `BackendDiscoveryBridge`
for the projections). There is **NO** live in-place migration, **NO**
dual-derivation shim, **NO** deprecation path. DELIVER must NOT build
one — widening `derive()` + the reconcilers' existing repopulation IS
the migration.

### What this amendment supersedes vs preserves

| Decision | Status |
|---|---|
| Decision 1 / § 1 — `ServiceId` derivation `(vip, port, purpose)` | **Amended** to `(vip, port, proto, purpose)`. Proto inserted as hash-input field 5 (before `purpose`). |
| Decision 1 — `MAGLEV_MAP` outer key = `ServiceId` (u64) | **Preserved in type; re-partitioned in meaning.** Still keyed by `ServiceId`; now one inner Maglev table per `(vip, port, proto)` slot rather than per `(vip, port)`. |
| Companion SERVICE_MAP revision (same date) — "`MAGLEV_MAP` … is unchanged" | **Completed.** That statement held for the wire key in isolation; this amendment supplies the control-plane-identity half it deferred. The two revisions together make the proto dimension consistent across the wire key, the cgroup key, *and* the control-plane identity. |
| Decision 1 — three-map split; Decision 2 — atomic swap; Q1/Q3/Q5/Q7 | **Preserved.** None interacts with the `ServiceId` derivation. The atomic-swap primitive is per-outer-slot; more `ServiceId`s means more `MAGLEV_MAP` slots, not a different swap mechanism. |
| `BACKEND_MAP` key = `BackendId` | **Preserved.** Backends are proto-independent; flat global namespace unchanged. |

### Consequences

**Positive.**

- CoreDNS (`tcp/53 + udp/53`) and HTTP/3 (`443/tcp + 443/udp`) are
  representable **end-to-end** — the control-plane identity, the
  SERVICE_MAP forward slot, and the LOCAL_BACKEND_MAP cgroup slot all
  partition on the same `(vip, port, proto)` axis. P2-Q4's
  no-listener-overwrites-another guarantee now holds at every layer.
- The three control-plane projection collisions
  (`listener_facts.rs:97-108`, `reconciler_runtime.rs:1716-1808`,
  `:1680-1686`) are resolved **without changing the projection data
  shape** — only the key derivation widens.
- Each proto gets an independent `MAGLEV_MAP` Maglev table — correct,
  since TCP and UDP backend sets/weightings for the same `(VIP, port)`
  are independent.
- Control-plane identity is now consistent with the dataplane keys;
  Overdrive carries one proto dimension uniformly from intent →
  identity → wire/cgroup slot.

**Negative / accepted.**

- **`ServiceId` content-addresses change for every service** (proto
  now enters the hash). Acceptable under the single-cut greenfield
  posture — maps and rows are recreated on boot; no operator-visible
  identifier is externally pinned across this change.
- The three production derive sites and their test mirrors must thread
  `proto` (the `Listener.protocol` field, already a `Proto`); golden
  fixtures in `crates/overdrive-core/tests/schema_evolution/*` that
  hardcode a `ServiceId` derived from `(vip, port)` are regenerated by
  the implementing crafter. These are DELIVER concerns, noted here for
  blast-radius visibility, not authored by this ADR.

### Schema-evolution note (for the DELIVER crafter)

**No rkyv envelope version bump is forced by this amendment.**
`ServiceId` stays a `u64` — the rkyv *archived layout* of every row
that embeds it (`AllocStatusRow`, `ServiceBackendRow`,
`ServiceHydrationResultRow`, the `service/<id>` projection, etc.) is
**byte-identical** before and after; only the *derived value* of the
`u64` changes. Per `.claude/rules/development.md` § "rkyv schema
evolution", a version bump is required only when the archived *layout*
changes; a change to a derived value at a stable layout is not a
schema change. The crafter therefore **does not** add a new envelope
variant or a new golden fixture for a layout bump — the existing
schema-evolution fixtures keep asserting the same layout. The crafter
**does** regenerate any fixture whose *expected `ServiceId` value* was
computed from the old `(vip, port)` derivation, because the canonical
pre-image changed. Do **not** bump schema versions for this work.

### Cross-references

- The companion SERVICE_MAP-outer-key revision (same date, immediately
  above) — the wire-shape half of the proto dimension.
- ADR-0052 § 1 (`adr-0052-backend-discovery-bridge-and-ebpf-production-boot.md`)
  — its `ServiceId` derivation cross-reference is brought in line with
  this amendment (updated in the same change to read
  `(assigned_vip, listener.port, listener.protocol, "service-map")`).
- ADR-0053 (LOCAL_BACKEND_MAP) 2026-06-03 amendment — the cgroup-path
  half; same `(vip, port, proto)` axis on the same-host path.
- ADR-0060 — source of the `Proto` type and the
  `ServiceFrontend { vip, port, proto }` boundary shape; P2-Q4
  records it needs "no change."
- `crates/overdrive-core/src/id.rs:797-836` (`ServiceId::derive`) —
  the derivation this amendment widens.
- `crates/overdrive-core/src/dataplane/backend_key.rs:46-81`
  (`Proto`, `Proto::as_u8()`) — the proto type and IANA byte.
- Three production derive sites:
  `crates/overdrive-control-plane/src/listener_facts.rs:100`,
  `crates/overdrive-control-plane/src/reconciler_runtime.rs:1681`,
  `:1799`.
- Projection-collision sites:
  `crates/overdrive-control-plane/src/listener_facts.rs:97-108`
  (`primary`), `reconciler_runtime.rs:1716-1808`
  (`BackendDiscoveryBridge` projection),
  `reconciler_runtime.rs:1168` (`reconcile_conflict` row — retained
  granularity, see Model A vs B).

### Changelog (Revision 2026-06-03, companion)

| Date | Change |
|---|---|
| 2026-06-03 (companion) | `ServiceId::derive` gains a `Proto` axis — `(vip, port, purpose)` → `(vip, port, proto, purpose)`; proto enters the SHA-256 pre-image as a single IANA byte (`Proto::as_u8()` → 6/17) at input field 5 (before `purpose`), zero-separated. `MAGLEV_MAP` outer key re-partitions per `(vip, port, proto)` (type unchanged; one Maglev table per proto). Locks **Model A** (one `ServiceId` per dataplane slot); records **Model B** (coarse `ServiceId` + one-to-many projections) as rejected. Completes the control-plane-identity half of P2-Q4 that the SERVICE_MAP revision deferred. `ServiceId` stays `u64` — no rkyv layout change, no envelope bump; derived values change (single-cut, reconciler-repopulated). — Morgan (user-locked, Model A). |
