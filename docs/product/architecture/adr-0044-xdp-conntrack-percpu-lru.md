# ADR-0044 — XDP per-CPU LRU conntrack table + flow-affinity-over-SERVICE_MAP-changes

## Status

Proposed. 2026-05-06. Decision-makers: Morgan (proposing). Tags:
phase-2, dataplane, conntrack, kernel-maps, xdp, l4lb, lru, percpu,
flow-affinity.

**Companion ADRs**: ADR-0040 (three-map split + HASH_OF_MAPS atomic-
swap primitive), ADR-0041 (weighted Maglev + REVERSE_NAT shape +
endianness lockstep), ADR-0042 (`ServiceMapHydrator` reconciler +
`Action::DataplaneUpdateService`), ADR-0043 (three-iface transit
test topology).

**Related issues**: GH #154 (Phase 2.16 conntrack — pulled forward
from its 2.16 slot to immediately follow Phase 2.2 Slice 06; see
§ Decision 1 below).

## Context

Phase 2.2 ships a *stateless* Maglev forwarder. The flow-affinity
guarantee for the stateless forwarder is the M ≥ 100·N rule:
≤ 1 % of 5-tuples remap on a single-backend removal (research
§ 5.2; ASR-2.2-02). This is sufficient for *most* canary cutover
shapes — until two structural failure modes meet a real workload:

1. **Stateless XDP bypass meets kernel netfilter conntrack.** The
   forward path runs in XDP and `bpf_redirect()`s the rewritten frame
   to a peer iface; netfilter's `nf_conntrack_in()` never sees the
   forward direction. The reverse path (TC egress reverse-NAT)
   *does* traverse netfilter's hooks. Conntrack's TCP tracker, with
   `nf_conntrack_tcp_loose=1`, mid-stream-picks-up a flow it only
   ever saw in one direction and cannot validate the sequence
   window. With `nf_conntrack_tcp_be_liberal=0` (the kernel
   default), payload-bearing segments outside the inferred window
   are flagged INVALID and silently dropped in `nf_conntrack_in()`.
   Length-0 segments (SYN-ACK, ACK, FIN-ACK) trivially pass the
   window check and survive; length-N segments with `seq` outside
   the inferred window do not. Symptom observed in S-2.2-17
   (`real_tcp_connection_completes_through_vip_with_payload_echo`):
   handshake completes, FIN-ACK delivered, every retransmit of the
   payload segment dropped.

2. **No flow affinity across SERVICE_MAP backend-set changes.**
   Maglev's ≤ 1 % disruption holds *per single-backend removal*.
   Operator workloads that combine long-lived TCP sessions with
   aggressive canary cutover (multiple successive backend-set
   rotations within a connection's lifetime) compose the disruption
   probabilities multiplicatively. A 1 %-per-rotation × 10
   rotations within the lifetime of a connection = ~10 % cumulative
   misroute probability. The §15 *Zero Downtime Deployments* claim
   weakens accordingly.

Both classes are solved structurally by **dataplane-owned
conntrack**: a 5-tuple → backend table the XDP forward path writes
on first-packet-in-flow and reads on every subsequent packet, with
`iptables -t raw -j NOTRACK` rules installed on the LB host so
kernel netfilter conntrack never sees the flow at all. Flow
affinity is then determined at *connection-establishment time*, not
at *every-packet hash time*; subsequent SERVICE_MAP rotations are
invisible to in-flight flows.

This is the architecture every production XDP L4LB ships:

- **Katran (Meta)**: `BPF_MAP_TYPE_ARRAY_OF_MAPS` of
  `BPF_MAP_TYPE_LRU_HASH` (one inner map per CPU) for `lru_mapping`
  (research § 4.1). 5-tuple key, backend-id value. Per-CPU
  eliminates cross-CPU LRU-list contention.
- **Cilium (with conntrack enabled, the default)**: a single shared
  `cilium_ct4_global` keyed by 5-tuple. Cilium accepts the
  cross-CPU contention in exchange for cross-CPU flow visibility
  (a flow that lands on CPU 0 and migrates to CPU 1 mid-stream
  still hits the same conntrack entry). The shared shape pays the
  contention cost; the per-CPU shape pays a stale-entry cost when
  RSS migrates flows.

The kernel exposes `BPF_F_NO_COMMON_LRU` precisely to support the
per-CPU shape: each CPU maintains its own LRU eviction list, no
cross-CPU lock on every update (research § 4.2; kernel BPF
hash-map docs).

The structural choice between per-CPU LRU and shared LRU is the
load-bearing design question. Three secondary questions follow:

- What goes in the value (BackendId only? generation counter for
  rotation detection? TTL? flow-state classification)?
- How does the BPF conntrack interact with kernel netfilter
  conntrack — NOTRACK injection, PRE_ROUTING ordering, or
  separate-by-construction?
- What is the userspace-side shape — does the hydrator know about
  conntrack, or is conntrack purely kernel-side state?

## Decision

### 1. Per-CPU LRU conntrack — `BPF_MAP_TYPE_LRU_PERCPU_HASH`

Adopt the Katran shape. Single map of type
`BPF_MAP_TYPE_LRU_PERCPU_HASH` (the kernel-native per-CPU LRU
variant — equivalent to `BPF_MAP_TYPE_LRU_HASH` with
`BPF_F_NO_COMMON_LRU` set, but with the simpler typed wrapper
`aya_ebpf::maps::LruPerCpuHashMap<K, V>` already shipped by aya-ebpf
0.1.1 — research § 4.2 + aya-rs research § A.1).

**Rejected alternative**: shared `BPF_MAP_TYPE_LRU_HASH` (Cilium
shape). Cross-CPU contention on the LRU list is a measurable per-
packet cost on every update (kernel BPF docs); per-CPU sidesteps
this entirely. The flow-migration-across-CPUs failure mode the
shared shape protects against is bounded: most kernels with RSS +
RPS pin a 5-tuple to a single CPU for the duration of the flow.
A flow migrating CPUs sees its conntrack entry "miss"; the next
packet falls through to Maglev and writes a fresh entry on the new
CPU. Per the Maglev ≤ 1 % disruption guarantee, the new entry
selects the same backend with high probability — the affinity is
preserved even on miss. The pathological case (flow migrates AND
SERVICE_MAP rotated AND Maglev's disruption hit this flow) is
strictly bounded by the same probability the stateless shape
already accepts; conntrack's job is to bound it tighter, not to
eliminate it.

**Rejected alternative**: in-place mutation of a fixed-size hash
without LRU eviction. Without LRU, the table grows monotonically
and becomes the dataplane's memory leak. The kernel's
`max_entries`-on-`HASH` semantics (rejection on insert) would
produce silent flow misroutes when the table fills. LRU eviction
is structurally required.

**Capacity**: 1_048_576 entries per CPU (matches the existing
REVERSE_NAT_MAP capacity in ADR-0041). At a typical 64 entries × 8
CPUs = 8 MiB resident, comfortably under any node's BPF memory
budget. Operator-tunability is deferred to a future Phase 2/3
slice (same shape as `MaglevTableSize`'s deferral in ADR-0041
Q6=A).

### 2. Key shape — 5-tuple `FlowKey` newtype

```rust
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct FlowKey {
    pub client_ip:    u32,   // host-order
    pub client_port:  u16,   // host-order
    pub backend_ip:   u32,   // host-order — populated AFTER backend selection
    pub backend_port: u16,   // host-order
    pub proto:        u8,    // IPPROTO_TCP or IPPROTO_UDP
    pub _pad:         [u8; 3],
}
```

Wait — re-examining the per-Cilium and per-Katran reference set:
the conntrack key is the **PRE-NAT 5-tuple** (`client_ip`,
`client_port`, `vip`, `vip_port`, `proto`), not the post-NAT
backend tuple. Forward-path lookup keys on what the *client*
sent; the result of the lookup IS the chosen backend. The key
must therefore be:

```rust
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct FlowKey {
    pub client_ip:    u32,   // host-order
    pub client_port:  u16,   // host-order
    pub vip:          u32,   // host-order — pre-NAT destination
    pub vip_port:     u16,   // host-order
    pub proto:        u8,    // IPPROTO_TCP or IPPROTO_UDP
    pub _pad:         [u8; 3],
}
```

This is structurally identical to `ReverseKey` (ADR-0041 § 11) but
with VIP-side fields instead of backend-side fields, and the value
inverted: `ReverseKey → OriginalDest` for response-path
de-translation; `FlowKey → FlowEntry` for forward-path
backend pinning. Both keys live in
`crates/overdrive-core/src/dataplane/` and share the same
endianness lockstep contract (host-order in map storage; conversion
in `crates/overdrive-bpf/src/shared/sanity.rs`).

`FlowKey` is a `pub struct` newtype (not a trivial alias) per
`development.md` § Newtypes — STRICT. Validating constructor
requires `proto ∈ {6, 17}` (TCP or UDP); the kernel-side helper
that builds the key from a packet (`flow_key_from_packet` —
companion to the existing `reverse_key_from_packet`) returns
`Option<FlowKey>` so non-TCP/UDP packets never hit conntrack.

### 3. Value shape — `FlowEntry` with rotation-aware backend reference

```rust
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct FlowEntry {
    /// The backend chosen for this flow when first seen.
    pub backend_id: BackendId,
    /// Generation counter from the SERVICE_MAP slot AT THE TIME
    /// this flow was pinned. Compared against the current
    /// generation on every read; a mismatch means the SERVICE_MAP
    /// rotated since this flow was pinned.
    pub service_generation: ServiceGeneration,
    /// Last-seen wall-clock-second (kernel-side `bpf_ktime_get_ns`
    /// truncated to seconds). Updated on every read for LRU
    /// freshness; the LRU eviction policy uses access-time, so this
    /// field exists for *userspace* introspection only — the LRU
    /// list itself is the eviction signal. Persisted-input
    /// (`development.md` § Persist inputs, not derived state) — the
    /// "is this entry fresh enough to trust" decision is recomputed
    /// against the live TTL policy on every read; never persisted
    /// as a deadline.
    pub last_seen_secs: u32,
    /// TCP flow state classification, narrow set:
    ///   `Established=0` (saw both directions), `SynOnly=1`
    ///   (forward only). UDP flows are always Established at
    ///   first packet (no handshake). Used to decide whether
    ///   to honour the entry on rotation: a `SynOnly` flow on a
    ///   rotated service is dropped and re-hashed; an
    ///   `Established` flow stays on the original backend until
    ///   the LRU evicts it or the connection closes.
    pub flow_state: FlowState,
    pub _pad: [u8; 3],
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum FlowState {
    SynOnly     = 0,
    Established = 1,
}
```

`ServiceGeneration` is a new newtype in `overdrive-core`:

```rust
/// Monotonic counter incremented by `Dataplane::update_service` on
/// every backend-set rotation. Stored in the SERVICE_MAP outer-map
/// value alongside the inner-map fd. Read by the XDP forward path
/// at conntrack-write time and at conntrack-read time.
pub struct ServiceGeneration(pub u64);
```

The generation counter is the load-bearing piece. Without it, a
conntrack entry pinned before a SERVICE_MAP rotation cannot
distinguish "the chosen backend is still alive" from "the chosen
backend was rotated out." With it, the forward-path lookup is:

```text
1. Build FlowKey from packet
2. CONNTRACK_MAP::get(&key) → Option<FlowEntry>
3. If Some(entry) AND entry.service_generation == current_generation:
       → forward to entry.backend_id (fast path; no Maglev hash)
4. If Some(entry) AND entry.service_generation != current_generation:
       → if entry.flow_state == Established: still forward to
         entry.backend_id (preserve in-flight TCP), but DO NOT
         refresh the entry — let the LRU expire it naturally
       → if entry.flow_state == SynOnly: drop the conntrack entry,
         fall through to Maglev
5. If None:
       → Maglev hash → backend_id
       → CONNTRACK_MAP::insert(key, FlowEntry { backend_id,
                                                 service_generation,
                                                 last_seen_secs,
                                                 flow_state: SynOnly })
```

The "Established" promotion happens on the reverse path: the TC
egress program (`tc_reverse_nat`) — which already has the reverse
5-tuple in its `ReverseKey` — performs a conntrack lookup AGAINST
THE FORWARD KEY (`FlowKey { client_ip = reverse_key.client_ip,
client_port = reverse_key.client_port, vip =
original_dest.vip, vip_port = original_dest.vip_port, proto }`)
and, if found with `flow_state == SynOnly`, updates it to
`Established`. This composes with ADR-0041's REVERSE_NAT_MAP
naturally: the response-path program already does one map lookup;
adding a second targeted update on first SYN-ACK is a fixed-cost
extension.

**Why persist `flow_state`** rather than deriving it from
`(packet.tcp_flags & ACK) != 0`: the decision happens at the
conntrack-read site in the FORWARD path, where the program does
not reliably see the ACK side of the handshake. The state is an
input to the rotation-handling policy in step 4 above; that policy
is a piece of code that lives in `xdp_service_map`, and its
inputs must be persistent because the FORWARD program does not
have access to the TC egress program's view of the flow.

### 4. NOTRACK installation — `iptables -t raw -j NOTRACK` on the LB host

Kernel netfilter conntrack must NOT see flows the BPF conntrack is
tracking. Two failure modes if it does:

1. **Half-tracked flow drops** (the S-2.2-17 symptom). Kernel
   conntrack with `tcp_loose=1` mid-stream-picks-up a flow it only
   saw in one direction; payload-bearing segments outside the
   inferred window are silently dropped.
2. **Doubled cost**. Every packet pays both BPF map lookups AND
   netfilter's hash table walks; the latter is what XDP exists to
   bypass.

The fix is `iptables -t raw -A PREROUTING -j NOTRACK` — installs a
"don't track" rule at the earliest netfilter hook the kernel
exposes (PREROUTING in the `raw` table runs before
`nf_conntrack_in()` in the `mangle`/`filter`/`nat` tables). Once
the rule is in place, the kernel's conntrack hooks see the packet
flagged `NOTRACK` and skip it entirely.

**Where the NOTRACK rule is installed**:

- **In production**: `EbpfDataplane::start()` installs NOTRACK on
  the loopback netns's `raw/PREROUTING` chain at startup, after a
  Earned Trust probe (per CLAUDE.md § Principle 12 — see § 5
  below). Removed on shutdown via `Drop`. The rule is scoped to
  match traffic destined for the dataplane-managed VIP range only,
  not all traffic; a future operator config surface can refine
  the match.
- **In Tier 3 tests**: `ThreeIfaceTopology::setup` (per ADR-0043)
  installs NOTRACK on `lb-ns`'s `raw/PREROUTING` chain. Removed
  on `Drop`. Matches the ADR-0043 transit-host posture.

**Rejected alternative**: PRE_ROUTING ordering tricks (running
the BPF conntrack in a position that "wins" against netfilter
without actually disabling netfilter). The XDP forward path runs
*before* netfilter at all — there is no ordering question on the
forward side. On the reverse side, TC egress also runs before
netfilter's egress hooks. The issue is purely the *forward
asymmetry*: netfilter sees only the response. NOTRACK is the
load-bearing fix; ordering tricks do not solve it.

**Rejected alternative**: rely on `nf_conntrack_tcp_be_liberal=1`
to accept out-of-window payload segments. This is a kernel-wide
sysctl that weakens TCP-state validation for *every* flow on the
host — including flows the operator legitimately wants tracked
(e.g., gateway-terminated TLS sessions in a future Phase 3 slice).
Touching a kernel-wide sysctl to fix one feature's bug is the
canonical "configuration creep" anti-pattern.

### 5. Earned Trust probe (CLAUDE.md Principle 12)

Per CLAUDE.md § Principle 12 (*"Every dependency you don't probe
is an act of faith you made for the user"*): the conntrack
subsystem depends on three external substrate behaviours that
must be probed at composition-root startup, not assumed:

1. **`BPF_MAP_TYPE_LRU_PERCPU_HASH` is creatable on the running
   kernel.** Probe: `bpf(BPF_MAP_CREATE)` for a 1-entry test map
   of this type at `EbpfDataplane::new`. Failure → structured
   `health.startup.refused` event with kernel version + errno.
   The 5.10 LTS floor (per `.claude/rules/testing.md` § Kernel
   matrix) DOES support this map type; the probe catches kernel
   builds that disabled it via `CONFIG_BPF_LRU_HASH=n` (rare but
   possible on hardened kernels).
2. **`iptables -t raw` is functional on the host.** Probe:
   `iptables -t raw -L PREROUTING -n` at startup. Failure →
   `health.startup.refused` with the iptables stderr. Catches
   container hosts where the `raw` table is excluded from the
   netns capability set, hosts running `nftables`-only with no
   `iptables-translate` shim, and hosts where the operator has
   explicitly disabled `raw` for security.
3. **Kernel netfilter conntrack does NOT see flows we mark
   NOTRACK.** Probe: install a temporary NOTRACK rule for a
   bogus VIP, send a synthetic packet (via `prog_test_run` or
   `AF_PACKET`), confirm `cat /proc/net/nf_conntrack` does not
   gain an entry for it, remove the rule. Failure →
   `health.startup.refused` with the conntrack-table state.
   Catches kernels with kernel-side conntrack offload modules
   that bypass NOTRACK (rare, but exists on certain Mellanox
   hardware-offload paths).

The three probes compose in the existing `Dataplane::probe`
surface (added as part of ADR-0035's `ViewStore::probe` precedent
extended to the dataplane port). `SimDataplane::probe()` returns
`Ok(())` unconditionally — the simulation does not have a kernel
to lie about.

### 6. SimDataplane mirror

Per `development.md` § "Production code is not shaped by
simulation" — the SimDataplane must mirror the conntrack table's
behaviour faithfully so the production code never carries a
sim-shaped concession. The mirror is:

```rust
pub struct SimDataplane {
    // ... existing fields (services, backends, maglev tables) ...
    /// Per-CPU mirror of CONNTRACK_MAP. Outer key = CPU index;
    /// inner BTreeMap keyed by FlowKey for deterministic iteration
    /// (per development.md § Ordered-collection choice).
    /// LRU eviction policy is the same one the kernel implements:
    /// the inner map's `last_accessed: VecDeque<FlowKey>` orders
    /// access-time; the eldest is evicted on insert when
    /// max_entries is reached.
    conntrack_per_cpu: BTreeMap<u32, SimLruMap<FlowKey, FlowEntry>>,
    /// Number of simulated CPUs (configurable per-test;
    /// defaults to 4 for DST seed reproducibility).
    cpu_count: u32,
}

struct SimLruMap<K: Ord, V> {
    entries:       BTreeMap<K, V>,
    access_order:  VecDeque<K>,   // eldest at front
    max_entries:   usize,
}
```

`SimDataplane` exposes `conntrack_lookup`, `conntrack_insert`,
`conntrack_evict_oldest` for harness inspection. The DST harness
chooses CPU per packet via the injected `Entropy` port (seeded
RNG) — modelling RSS deterministically. Cross-CPU flow migration
is a first-class scenario the harness can drive (`SimDataplane::
migrate_flow_cpu(flow, new_cpu)`); this is the test surface for
the cross-CPU miss-then-rehash path described in § 1.

### 7. Three new DST invariants

| Name | Property |
|---|---|
| `ConntrackPinsBackendUnderRotation` | Always: a flow with `flow_state == Established` and a stale `service_generation` continues to forward to its pinned `backend_id` until the LRU evicts it. |
| `ConntrackEventuallyConvergesAfterRotation` | Eventually: after a SERVICE_MAP rotation + LRU expiry-window worth of ticks with no new packets on stale flows, the conntrack table contains zero entries with stale `service_generation`. |
| `NoTrackProbeRefusesStartOnFailure` | Always: if any of the three Earned Trust probes (§ 5) fails, `EbpfDataplane::start` returns Err with `health.startup.refused` and the dataplane is not advertised as ready. |

All three live in `crates/overdrive-sim/src/invariants/conntrack/`
and run on every PR per `.claude/rules/testing.md` § Tier 1.
The first invariant is the load-bearing one for the §15 *Zero
Downtime Deployments* claim under canary cutover.

### 8. Userspace surface — typed handle, no hydrator extension

`ConntrackMapHandle` lands in
`crates/overdrive-dataplane/src/maps/conntrack_map_handle.rs` as
a typed wrapper over `aya::maps::Map::PerCpuLruHashMap(MapData)`
(per aya-rs research § A.1 — userspace requires hand-rolled
fd-extraction; aya 0.13.x has no typed `LruPerCpuHashMap<T, K, V>`
wrapper). Method surface mirrors the existing `ReverseNatMapHandle`:

```rust
impl ConntrackMapHandle {
    pub fn new(bpf: &mut Ebpf, name: &str) -> Result<Self, MapError>;
    pub fn get(&self, key: &FlowKey) -> Result<Option<FlowEntry>, MapError>;
    pub fn insert(&self, key: &FlowKey, entry: &FlowEntry) -> Result<(), MapError>;
    pub fn remove(&self, key: &FlowKey) -> Result<(), MapError>;
    /// For Tier 3 tests + operator introspection. Sums per-CPU
    /// counts across all online CPUs.
    pub fn entry_count(&self) -> Result<u64, MapError>;
}
```

The `ServiceMapHydrator` reconciler (ADR-0042) does NOT gain new
responsibilities. Conntrack is purely kernel-side state populated
on the data path; the control plane reads the entry count for
telemetry but does not write conntrack rows. The hydrator's
existing `Action::DataplaneUpdateService` carries the new
`ServiceGeneration` field implicitly: every call to
`Dataplane::update_service` increments the generation counter
inside `EbpfDataplane`, and the new value is written into the
SERVICE_MAP outer-map slot's metadata. Reconciler authors do not
see this — it lives entirely inside the dataplane adapter.

## Consequences

### Positive

- **Closes S-2.2-17.** With NOTRACK installed, the kernel
  netfilter conntrack interference vanishes; payload-bearing TCP
  segments traverse the dataplane unimpeded. The S-2.2-17
  acceptance test goes GREEN as a side-effect of landing
  Decision 4 (NOTRACK installation), independent of the rest of
  the conntrack work — see § Roadmap impact below for the slice
  ordering that exploits this.
- **Bounded flow-affinity under operator-paced canary cutover.**
  Per Decision 3, in-flight TCP flows survive arbitrarily-many
  SERVICE_MAP rotations; the §15 *Zero Downtime Deployments*
  claim becomes structural rather than statistical.
- **The §18 reconciler primitive remains untouched.** Conntrack
  is a dataplane-internal concern; no new Action variant, no new
  ObservationStore table, no new reconciler. The hydrator stays
  exactly the shape ADR-0042 locked.
- **Production XDP L4LB shape parity.** Katran and Cilium both
  ship dataplane-owned conntrack; Overdrive's Phase 2.16
  pulled-forward delivery brings the platform to parity at the
  same point in its phase ordering.

### Negative

- **Kernel conntrack is now off the table for any future feature
  that needs it on dataplane-managed flows.** Phase 3+ features
  that want stateful packet inspection on dataplane-managed
  traffic must extend the BPF conntrack, not lean on netfilter.
  Acceptable cost: every production XDP L4LB makes the same
  tradeoff; the kernel conntrack is a fallback for slow-path
  flows, not a primitive to extend.
- **Per-CPU shape costs cross-CPU flow visibility.** A flow that
  RSS-migrates between CPUs sees a "miss" on the second CPU and
  re-hashes; with the Maglev ≤ 1 % disruption guarantee, the
  re-hash usually picks the same backend, so affinity is
  preserved in expectation. Acceptable cost — the alternative
  (shared LRU) pays a per-packet contention cost that's worse
  than the bounded probability of a single re-hash.
- **NOTRACK rule installation requires `CAP_NET_ADMIN` on the
  LB host.** This is already a precondition for XDP attach and
  TC egress; Decision 4 adds no new capability requirement.
  Tier 3 tests already gate on `CAP_NET_ADMIN` (see
  `crates/overdrive-dataplane/tests/integration/helpers/netns.rs`
  precedent).
- **Three new DST invariants** (Decision 7) extend the Tier 1
  surface by ~150 LoC and a few seconds of per-PR DST runtime.
  Acceptable; the closure of the §15 claim justifies it.
- **One new newtype, one new struct, one new enum** in
  `overdrive-core::dataplane`: `ServiceGeneration`, `FlowKey`,
  `FlowState` (and `FlowEntry` as a `#[repr(C)]` POD type).
  Each carries the full FromStr / Display / serde / rkyv /
  proptest discipline per `development.md` § Newtype
  completeness; `FlowState` discriminants stable per
  ADR-0037-style minor-version semantics.

### Neutral / informational

- **Aya 0.13.x typed-wrapper status**: kernel-side
  `LruPerCpuHashMap<K, V>` IS shipped by `aya-ebpf 0.1.1` (aya-rs
  research § A.1). Userspace side falls through to `Map::
  PerCpuLruHashMap(MapData)` — same hand-rolled-handle pattern
  the project uses for `Map::Unsupported(MapData)` HoM access in
  ADR-0040. No new substrate gap.

## Alternatives considered

### Alt 1 — Tactical NOTRACK in test setup only; defer strategic conntrack to Phase 3+

Add `iptables -t raw -j NOTRACK` to `ThreeIfaceTopology::setup` as
a single-line helper change; flip S-2.2-17 GREEN; defer the
dataplane-owned conntrack table to a Phase 3 slot.

**Rejected**: the user's explicit dispatch instruction was *"let's
address GH #154 in the next roadmap step"* — option (c) in the
dispatch's Decision 1 is ruled out by user intent. Beyond that:
the bridge fix masks the structural failure (kernel conntrack
interference is real; deferring it leaves the
flow-affinity-across-rotation gap open, which is the *other* half
of #154). Doing the strategic fix now is also the clean path
because the bridge's NOTRACK rule needs to come out anyway when
the strategic fix lands; doing both at once avoids two PRs that
each touch the test topology. The bridge IS, however, what
Decision 4 ships in the BPF dataplane production path.

### Alt 2 — Cloudflare Unimog "previous-bucket" daisy-chain

Cloudflare's Unimog ships no per-flow conntrack at all. Each
forwarding-table bucket carries `current_DIP` + `previous_DIP`;
when a packet lands on the wrong server because the bucket
changed, that server forwards it to the previous owner via TC at
layer 7. "Less than 1 %" of packets need the second hop in steady
state (research § 4.1).

**Rejected**: Unimog's shape requires backends to participate in
the daisy-chain (the wrong-server forward is at L7, not at
dataplane level). Overdrive's whitepaper §7-§8 architecture has
backends as opaque application servers; requiring them to run a
TC-level forwarder would contradict the whitepaper's "all workload
types are first class" principle. Unimog's tradeoff is correct
for Cloudflare's edge-server fleet, where every host is a
Cloudflare-controlled L7 proxy; it is wrong for Overdrive's
arbitrary-workload model.

### Alt 3 — Single shared `BPF_MAP_TYPE_LRU_HASH` (Cilium shape)

See § Decision 1 rejection-rationale above. Cross-CPU contention
on every update vs the per-CPU LRU shape's bounded re-hash cost
on flow migration.

### Alt 4 — Defer to POLICY_MAP / operator-tunable rules (#158)

A future POLICY_MAP could carry "should this flow be conntrack-
tracked" as a per-VIP rule, deferring the decision to operator
configuration.

**Rejected**: a per-VIP opt-in for conntrack creates two code
paths in the XDP forward program (with and without conntrack
lookup), which doubles verifier-budget cost and forks the test
surface. Conntrack is structural to the §15 claim; making it
optional makes the §15 claim conditional. Operator-tunability is
the right shape for *behaviour* knobs (TTL, capacity), not for
*structural* primitives.

## Roadmap impact

This ADR is the design lockpoint for a new feature directory at
`docs/feature/phase-2.16-xdp-conntrack/` that opens immediately
after Phase 2.2 Slice 06 closes. The dispatch-ratified ordering
is:

1. **Slice 06 finishes first** (06-03, 06-04, 06-05). The sanity
   prologue is independent of conntrack and lands cleanly.
2. **Slice 07 (Tier 4 perf gates) lands next** as already
   planned. Conntrack changes the verifier-budget baseline; the
   gate must be in place before the conntrack slice lands so the
   delta is measured properly.
3. **Slice 08 (hydrator reconciler)** lands next as already
   planned. The hydrator's structural shape does not change; the
   `ServiceGeneration` increment lives entirely inside the
   dataplane adapter (Decision 8).
4. **NEW Phase 2.16 slices land after Phase 2.2's Slice 08**:
   - 16-01: `FlowKey` / `FlowEntry` / `FlowState` /
     `ServiceGeneration` newtypes + SimDataplane mirror;
     three DST invariants land RED.
   - 16-02: `CONNTRACK_MAP` declaration + `ConntrackMapHandle`
     userspace + Earned Trust probes (per Decision 5).
     Tier 3 probe-failure tests.
   - 16-03: NOTRACK installation in production
     `EbpfDataplane::start` + `ThreeIfaceTopology::setup`. Flips
     S-2.2-17 GREEN as a side-effect (and the in-flight Slice 06
     pivot tactical-bridge stays as the bridge: until 16-03
     lands, the test setup carries the NOTRACK rule directly;
     once 16-03 lands, the production-path installs it and the
     test-setup duplicate is deleted).
   - 16-04: XDP forward path conntrack lookup-write +
     `ServiceGeneration` increment in `Dataplane::
     update_service`. ESR pair `ConntrackPinsBackendUnderRotation`
     + `ConntrackEventuallyConvergesAfterRotation` go GREEN.
   - 16-05: TC reverse path conntrack `flow_state` promotion
     (SynOnly → Established).
   - 16-06: Tier 4 verifier-regress baseline update + xdp-perf
     baseline update (conntrack adds two map lookups to the
     forward path; perf delta must be measured and gated).

Six steps, well within the 3–5 step suggestion's range when the
pure-Rust newtype work (16-01) is folded into the kernel-side
work — but five-or-six is the credible decomposition; padding
down to three would coarsen the mutation-target boundary and
hide failure modes.

## Cross-references

- **Whitepaper § 7** — *eBPF Dataplane / XDP — Fast Path Packet
  Processing*. The conntrack table is a kernel-side concern that
  composes with the existing SERVICE_MAP / BACKEND_MAP / MAGLEV_MAP
  / REVERSE_NAT_MAP from ADR-0040 / ADR-0041.
- **Whitepaper § 15** — *Zero Downtime Deployments*. Decision 3's
  generation counter is the load-bearing primitive that makes
  flow-affinity-across-rotation structural rather than statistical.
- **CLAUDE.md § Principle 12** — *Earned Trust*. Decision 5
  enumerates the three substrate dependencies + the probes that
  guarantee the dataplane refuses to start if any of them lies.
- **Research** — `docs/research/networking/xdp-service-load-
  balancing-research.md` § 4 (Connection-tracking strategy);
  `docs/research/dataplane/aya-rs-usage-comprehensive-research.md`
  § A.1 (per-CPU LRU typed-wrapper status).
- **ADR-0040 / ADR-0041 / ADR-0042 / ADR-0043** — companion ADRs;
  this ADR composes with all four without superseding any.
- **`.claude/rules/development.md`** — § Newtypes — STRICT
  (FlowKey/FlowEntry/FlowState/ServiceGeneration discipline);
  § Persist inputs, not derived state (Decision 3 rationale on
  `last_seen_secs`); § Production code is not shaped by
  simulation (Decision 6 rationale); § Ordered-collection choice
  (Decision 6's `BTreeMap`-keyed mirror).
- **`.claude/rules/testing.md`** — § Tier 1 (DST invariants per
  Decision 7); § Tier 2 (PROG_TEST_RUN triptych for the lookup-
  chain); § Tier 3 (real-veth integration on Lima / ubuntu-latest);
  § Tier 4 (verifier-regress + xdp-perf baseline update).

## Changelog

| Date | Change |
|---|---|
| 2026-05-06 | Initial draft. Six decisions; four alternatives considered; six-step roadmap-impact statement. — Morgan. |
