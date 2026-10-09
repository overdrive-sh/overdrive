# DISTILL — P3: datagram associations, UDP slots, framing, cells

Piece P3 of the parallel DISTILL for the vsock attachment replacement
(netns-density-295). Scope: the guest datagram path — the datagram **frame**
(D5), the per-VM **UDP slot** pool (D25), the **association** lifecycle and the
owner's view of it (D4, D10, U-2, SLOT-ABORT), and **cell recreation** on every
end of association (CELL-RECREATE). Authoritative design:
`feature-delta.md` §"vsock Attachment Replacement DESIGN" (decisions D4 L16998,
D5 L16999, D10 L17012, D25 L17031, D25-BIND / U-2 L16757, CELL-RECREATE
L17040; wire contracts L17971; per-kind total orders L18254; guest adaptation
contract L19593; owner surface L19062; G-V6 L20308). Quint oracle:
`specs/quint/guest-flow-owner/udp_slots.qnt` (invariants 2, 14 + U-2;
`checks.toml` `udp-slots-*`). Shared test-support contract:
[`shared-test-infrastructure.md`](shared-test-infrastructure.md).

Binding rules (project): **no `.feature` files**; GWT below is spec only; the
crafter translates each to a Rust `#[test]` / `#[tokio::test]`
(`.claude/rules/testing.md` §"Testing"). PBT layer discipline (Mandate 9 /
`.claude/rules/testing.md`): `proptest` at the pure/unit layer (the frame
codec, the reserved-port function, the slot/overlap predicates); **example-based
only** at Tier-3 / integration / seeded-sim; Quint conformance replays the model
through the production owner.

---

## 0. Load-bearing sequencing constraint — what is authorable now

The replacement production surface P3's bodies call —
`overdrive_core::guest_transport::wire` (the datagram frame codec, `FlowRequest`,
`ControlMessage`), the guest intake constants + `guest_reserved_ports()`, the
`overdrive-init` slot model, the `overdrive-control-plane` owner's datagram-flow
seam — **does not exist yet** (verified 2026-10-09: `guest_transport` /
`DATAGRAM_FRAME_MAGIC` / `GUEST_UDP_SLOTS` absent from `overdrive-core`;
`overdrive-init` is a single `main.rs` with no testable component). DELIVER is
stopped at old-model 08-04; the **roadmap reconciliation is the first DELIVER
activity** (feature-delta L16554).

Consequence (shared-infra §0): this scenario doc is the real, authorable-now
DISTILL product. The four Rust files are authored as complete RED scaffolds
against the exact design-pinned API shapes, every test marked
`#[ignore = "pending DELIVER …"]`. They **do not compile until** DELIVER
reconciliation creates the production scaffolds (the `[REF] Interface contracts`
shapes) and registers the module in its entrypoint — registration + compiling
body co-arrive with that scaffold (shared-infra §6). Nothing P3 rests on is
blocked on a validation item: V-5(c) (P-41, CELL-RECREATE), K-B2/K-B3 (audit,
recreate-vs-empty) have all run; R5-14/R5-15/R5-27 are DELIVER-step example
bodies marked `#[ignore]`, not "blocked". Per CLAUDE.md §"Implement to the
design", where the design does not pin the exact testable shape a scenario
needs, §9 records it as a **testability gap** — no production API is invented
to make a body compile.

---

## 1. File ownership (disjoint) + entrypoint registration + nextest group

| File | Crate / layer | Scenarios | Registration (co-arrives DELIVER) |
|---|---|---|---|
| `crates/overdrive-core/tests/acceptance/vsock_datagram_frame.rs` | overdrive-core, unit/proptest (layer 1) | P3-S1 | `mod vsock_datagram_frame;` in `tests/acceptance.rs` |
| `crates/overdrive-init/tests/acceptance/vsock_guest_udp_slots.rs` | overdrive-init, acceptance (host-runnable parts) | P3-S2, S3(pure), S4(model), S5(model), plus D25 reserved/token/exhaustion | P3 **creates** `crates/overdrive-init/tests/acceptance.rs` (distinct from P6's `tests/integration.rs`); `mod vsock_guest_udp_slots;` |
| `crates/overdrive-control-plane/tests/acceptance/vsock_owner_datagram_assoc.rs` | overdrive-control-plane, seeded-sim acceptance (default lane) | P3-S6, S9, D4 direction, D10 | `mod vsock_owner_datagram_assoc;` in `tests/acceptance.rs` |
| `tests/conformance/tests/vsock_udp_slots_conformance.rs` | root conformance, standalone binary (`integration-tests`) | P3-S8 (all udp_slots invariants) | standalone binary — no `mod` line; needs shared `tests/conformance/src/quint_driver.rs` (shared layer) |

Entrypoints are NOT pre-created now (shared-infra §6: pre-creating stubs is
premature churn before the reconciliation reshapes the step→crate→file map and
retires the old-model modules). The three `tests/acceptance/*.rs` subfiles sit
**unregistered** until DELIVER (Cargo does not compile an unregistered
`tests/acceptance/<file>.rs`, so the tree stays green). `overdrive-init` has no
test entrypoint today: P3 owns creating `tests/acceptance.rs`; P6 owns
`tests/integration.rs` — distinct files, no collision.

**nextest group (`.config/nextest.toml`, `host-kernel-shared`, `max-threads=1`,
matched by MODULE).** P3's **host-runnable acceptance tests touch no node-global
kernel state** (the datagram frame codec and reserved-port function are pure;
the owner seeded-sim runs through the private seam with `Sim*` at driven ports;
the overdrive-init slot-model acceptance tests use in-memory doubles for the
kernel-map port). They stay in the default lane and add **no** group entry. The
**Tier-3 guest-kernel cells** (R5-15 real `EADDRINUSE` bind, R5-14/R5-27 real
capture+pairing, R5-14 concurrent-no-loss) run **inside the P6 walking
skeleton** (shared-infra §6: guest-only real-kernel behaviour is a Tier-3 cell
in the WS, not a standalone host test); P6 owns their `host-kernel-shared`
module entries (process-global `AF_VSOCK` 1240/1241, the guest's own kernel).
P3 adds no `host-kernel-shared` entry of its own.

---

## 2. Mandate-12 domain vocabulary (Rust adaptation)

Domain concepts are the `overdrive-core` newtypes the design pins (no
`domain_types.py`): `GuestCid`, `LaunchToken`, `FlowId` / `FlowKind` /
`FlowOrigin`, `FlowRequest`, `ControlMessage` / `FlowRefusal`, `ListenPort`,
and the datagram constants `DATAGRAM_FRAME_MAGIC` / `DATAGRAM_FRAME_HEADER_LEN`
/ `MAX_GUEST_DATAGRAM_PAYLOAD`, the slot constants `GUEST_TCP_INTAKE_PORT` /
`GUEST_UDP_SLOT_BASE` / `GUEST_UDP_SLOTS` / `GUEST_DATAGRAM_SLOT_IDLE` /
`GUEST_TCP_PARK_CELLS` and `guest_reserved_ports()`. Step bodies invoke
production services (the pool, the owner's provisioner surface, the datagram
codec) and never inline business logic. Newtype completeness
(Display/FromStr/serde/proptest roundtrip) is the mandatory proptest call site
(`.claude/rules/testing.md` §"Property-based testing").

---

## 3. Coverage maps

**Decision → scenario.** D4 → P3-S9b (direction/SEQPACKET-only-from-guest). D5
→ P3-S1 (frame codec). D10 → P3-S9c (DNS as datagram association). D25 → P3-S2
(slot identity), P3-S3 (D25-BIND bind refusal), P3-S4 (idle), P3-S10 (slot
exhaustion `EPERM`). D25-BIND → P3-S3. D15-R2 (guest bind/pool/idle residual) →
P3-S3/S4/S10 (each cross-references D15 re-derivation row). CELL-RECREATE →
P3-S5 (recreate+generation dispatch). U-2 → P3-S6.

**R5 → scenario.** R5-14 → P3-S4 (idle 120 s), P3-S5 (burst parked discarded on
recreate; reply cut by assoc end never reaches app with frame; generation
dispatch; cell-creation-failure quarantine), P3-S7 (concurrent no-loss, loss
attributed to a stage), and the lost-report audit recovery row P3-S4b. R5-15 →
P3-S3 (reserved-port bind `EADDRINUSE`, Tier-3 guest). R5-27 → P3-S6
(refused-dest keeps slot, no self-retry, re-associate on next datagram).

**Quint `udp_slots` invariant → conformance run (P3-S8).**

| Invariant (udp_slots.qnt) | Defends | `checks.toml` run(s) replayed |
|---|---|---|
| `SlotHeldOnce` (inv 2a) | one slot per (socket,dest); holders ≤ 1 | `udp-slots-safety-tlc`, `udp-slots-safety-apalache`; hazard `udp-slots-hazard-claim-split` |
| `ReleasedSlotCarriesNoData` (inv 2b) | released slot empty; owned carries only its owner; no misdelivery | safety runs; hazards `keep-frames`, `late-paired-by-slot-data` |
| `RouteCurrent` | a routed slot routes to its own current association | safety; hazard `late-paired-by-slot-route` |
| `LatePairedAborted` (inv 2c, SLOT-ABORT) | late `Paired` for a released slot → `Abort`, never ignored/routed | safety; hazard `ignore-late-paired` |
| `NoReassociationWithoutNewDatagram` (U-2) | ended assoc re-associates only on a new datagram | safety; hazards `keep-frames-on-end`, `cells-emptied-only`, `fd-dispatch` |
| `NoStaleGenerationStart` (inv 14, CELL-RECREATE) | no assoc starts from an earlier-generation readiness event | `udp-slots-safety-*` (inside `Safety`); hazards `fd-dispatch-generation`, `cells-emptied-only` |
| witnesses (non-vacuity) | `WitnessNoLatePaired` / `NoRouted` / `NoTwoSlotsOneSocket` / `NoReassociation` / `NoStaleEvent` | `udp-slots-witness-*` |
| `ReleasedSlotBecomesFree` (progress) | a slot of a released socket returns to the pool | `udp-slots-release-liveness` |
| hazard teeth | each design rule off → expected `violation` | `udp-slots-hazard-*` incl. `no-audit-fairness` |

**Gate → scenario.** G-V6 (per-flow pairing, datagram acceptor) → P3-S6, S9,
and the datagram rows of the 6-row boundary table (§6). P3's datagram
obligations are the subset of G-V6 the owner holds for datagram associations;
the TcpAccept / TcpConnect halves of G-V6 are P2/P5.

---

## 4. Scenarios

Each scenario records: **contract/invariant**, **production-owner path**,
**fault stimulus** (where applicable), **oracle** (observable universe + delta,
Mandate 8 spirit via the project's native before/after capture — port-exposed
names only, never private fields), **evidence tier**, **PBT layer**, and its
`#[ignore]` DELIVER marker. Target ≥40% error/edge (P3: S3, S4b, S5c, S6, S9a,
S10 + the boundary rows 2/4/5 are failure/edge — well over 40%).

### P3-S1 — A guest datagram of any size crosses the transport frame unchanged `@property @driving_port`

```gherkin
Property: the datagram frame carries any logical payload and restores it exactly
  Given the guest transport frames a datagram (magic "OVD1" + BE u32 length)
  When a payload of any length 0..=59_000 is framed and then unframed
  Then the restored payload is byte-identical to the original
    And an empty datagram is an 8-byte header with no payload
    And a payload of 59_000 bytes frames and restores (the inclusive bound)
    And a byte string whose length prefix or magic is malformed fails to decode
        with a typed error, never a panic and never a truncated payload
```

- **Contract/invariant:** D5 — `DATAGRAM_FRAME_MAGIC = *b"OVD1"`,
  `DATAGRAM_FRAME_HEADER_LEN = 8`, `MAX_GUEST_DATAGRAM_PAYLOAD = 59_000`;
  canonical-encoding round-trip holds over every valid value (symmetric
  property, Hebert §3); the golden-bytes pin for an empty frame and a maximum
  frame (feature-delta L18170).
- **Production-owner path:** `overdrive_core::guest_transport::wire` frame codec
  (the "Rust codec" the golden-bytes test pins across, L18174). **Testability
  gap G-1:** the design pins the three constants but not the exact codec
  type/method — §9.
- **Oracle:** universe = { decoded payload bytes, decode result variant };
  `encode(p) |> decode == Ok(p)` for every `p`; empty → header-only (len = 8);
  malformed (bad magic, length > 59_000, truncated body) → typed `Err`, and the
  partial-decode slot is never a payload (fail-closed). Plus the C6 closed-set
  assertion: no decode error escapes the declared set.
- **Evidence tier:** layer-1 unit, `proptest` full (`st.vec(any::<u8>(), 0..=59_000)`
  with pinned `@example` for 0, 1, 1_431, 59_000). Boundary C1b: 59_000 (max),
  59_001 (max+1 → `Err`), and (if the codec validates on encode) encode of
  > 59_000 refused.
- `#[ignore = "pending DELIVER: datagram frame codec in overdrive-core guest_transport::wire (D5)"]`

### P3-S2 — One UDP slot per (application socket, destination) `@driving_port`

```gherkin
Property: a socket sending to N destinations holds exactly N slots, each distinct
  Given a guest with a pool of GUEST_UDP_SLOTS (1_024) datagram slots
  When one application socket sends a first datagram to each of N distinct destinations
  Then the slot-ownership table holds exactly N owned slots for that socket
    And no two owned slots share the same (socket, destination)
    And a second datagram to a destination it already owns reuses that slot (claims none)
```

- **Contract/invariant:** D25 "Slot identity" — one slot per (socket cookie,
  original destination); Quint `SlotHeldOnce` (holders ≤ 1; distinct
  (sock,dest)); the claim is one atomic step (Quint `AppClaim`, not
  `AppPick`/`AppTake`).
- **Production-owner path:** `overdrive-init`'s slot-ownership table keyed by
  socket cookie, driven through its kernel-slot-map read port (in-memory double
  at the acceptance layer). **Testability gap G-2:** the design describes the
  table as `overdrive-init` internal state read from the kernel slot map; it
  does not pin a pure, host-testable slot-table component — §9. The full slot
  state machine is covered by the Quint `udp_slots` conformance (P3-S8); this
  acceptance scenario asserts the (socket,dest)→slot allocation decision if a
  pure component is exposed, else it collapses into P3-S8 + the P6 Tier-3 cell.
- **Oracle:** universe = { owned-slot count for the socket, the set of
  (socket,dest) tags over owned slots, slot reuse on repeat destination };
  delta = N owned slots, all tags distinct, repeat send adds 0 slots.
- **Evidence tier:** layer-2 acceptance with in-memory kernel-map double (if
  G-2 resolved), else Quint conformance + P6 Tier-3. PBT: `parametrize` N ∈
  {0, 1, 2, 1_024} (C3 cardinality) — not generative (finite pool).
- `#[ignore = "pending DELIVER: overdrive-init slot-ownership table (D25)"]`

### P3-S3 — A reserved intake port rejects an application bind of the same protocol `@real-io`

```gherkin
Scenario: an application bind on a reserved intake port fails EADDRINUSE
  Given a booted guest with the TCP intake on 61_000 and UDP slots on 61_001..=62_024
  When the application binds TCP 61_000 on workload_addr or 0.0.0.0
  Then the bind fails EADDRINUSE
  When the application binds UDP 61_001 or UDP 62_024 on workload_addr or 0.0.0.0
  Then the bind fails EADDRINUSE
  When the application binds TCP 61_001 (no TCP intake there) or UDP 61_000 (no UDP intake there)
  Then the bind succeeds
    And no ephemeral port in 61_000..=62_024 is ever handed out
    And the TCP intake never appears in a ListenState report
```

- **Contract/invariant:** D25-BIND — `EADDRINUSE` deterministically **only** on
  a port holding an intake of **that** protocol; the reserved range
  (`guest_reserved_ports() = 61_000..=62_024`) is set in
  `net.ipv4.ip_local_reserved_ports` so no reserved port is ephemeral; the TCP
  intake is excluded from `ListenState` (D23). D15-R2 (reserved-range residual).
- **Production-owner path:** `overdrive-init` boot (intakes bound before READY
  without `SO_REUSEADDR`/`SO_REUSEPORT`). The **host-runnable** part of this
  scenario in `vsock_guest_udp_slots.rs` is the pure predicate: `guest_reserved_ports()`
  bounds, and the per-(port,protocol) bind-decision (TCP 61_000 / UDP 61_001..=62_024
  reserved; TCP 61_001 / UDP 61_000 free). The **real `EADDRINUSE` on a live
  guest kernel is Tier-3** → a cell inside the **P6 walking skeleton** (R5-15
  lane; shared-infra §6). Cross-reference recorded as coordination gap §9 G-5.
- **Oracle (host part):** `guest_reserved_ports()` is `61_000..=62_024`
  inclusive; `reserved(port, Tcp) ⇔ port == 61_000`;
  `reserved(port, Udp) ⇔ 61_001 ≤ port ≤ 62_024`. **Oracle (Tier-3, P6):**
  real `bind()` errno == `EADDRINUSE` on the reserved/protocol pairs and `Ok`
  on the free pairs; the TCP intake absent from two `NETLINK_SOCK_DIAG`
  `TCPF_LISTEN` dumps.
- **Evidence tier:** host part layer-1 `proptest` over `u16` (refusal iff
  reserved-and-matching-protocol; C6 robustness: wrong-protocol bind on a
  reserved port succeeds); Tier-3 example-based in P6 WS (R5-15).
- `#[ignore = "pending DELIVER: guest_reserved_ports + reserved-port bind decision (D25-BIND); Tier-3 EADDRINUSE in P6 walking skeleton (R5-15)"]`

### P3-S4 — An idle datagram association is released after 120 s; a lost release report is recovered by the audit `@driving_port`

```gherkin
Scenario: a datagram association idle in both directions for 120 s is released
  Given an owned slot whose association has carried no datagram either way
  When GUEST_DATAGRAM_SLOT_IDLE (120 s) elapses with nothing parked
  Then the association ends in the end-of-association order and the slot returns to the pool
    And a slot whose sock_release report was lost is found by the level-triggered audit
        (its socket cookie no longer exists) and released
```

- **Contract/invariant:** D25 "Slot release" — idle 120 s (both directions,
  per-slot counters); K-B2 — a lost `sock_release` report is recovered by the
  `sock_diag`-based audit; Quint progress `ReleasedSlotBecomesFree` (weak
  fairness of `releaseAny` + `GuestAudit`). D15-R2 (120 s idle residual).
- **Production-owner path:** `overdrive-init` slot model: `IdleRelease`
  (nothing parked) and `GuestAudit` (orphan slots whose cookie is gone), driven
  with an injected clock (`SimClock`) and in-memory kernel-map double.
- **Oracle:** universe = { owned-slot set, pool-free count, `toHostEnd` (the
  host observes the association's end) }; delta = idle slot moves owned→free
  after the idle deadline; orphan slot (socket closed, report lost) moves
  owned→free after the audit; the ended slot's `routed`/`fid`/`parked` cleared.
- **Evidence tier:** layer-2 seeded (clock injected), example-based (not PBT:
  the lane is layer 3+ / seeded-sim per Mandate 11); Quint conformance covers
  the liveness (P3-S8, `udp-slots-release-liveness`). Real idle + real audit at
  density is the P6 Tier-3 / V-9 cell.
- `#[ignore = "pending DELIVER: overdrive-init idle release + sock_release audit (D25, K-B2, R5-14)"]`

### P3-S5 — Ending an association recreates its cells and discards parked frames; no stale-generation event starts an association `@driving_port @property`

```gherkin
Scenario: a burst parked in a slot is discarded by cell recreation, not re-delivered
  Given a slot with a burst parked in its framing cell (200 x 1_000 B against 4 KiB buffers)
  When the association ends in the end-of-association order (cut reassembly output → close legs →
       recreate both cells → disconnect slot socket → remove the guest-lo unframe tuple)
  Then the parked frames are discarded (cells recreated, not emptied)
    And the slot's next association to another destination receives none of the old frames
    And a host→guest reply cut by the association's end never reaches the application with a frame header
    And a readiness event registered for an earlier cell generation never starts an association
    And a slot whose new cell cannot be created is quarantined (no association; the next send to it
        takes a free slot or is refused EPERM at exhaustion; it returns to the pool after the audit retries)
```

- **Contract/invariant:** CELL-RECREATE — end of association recreates framing +
  reassembly cells (never empties: an emptied cell's sender-side queue / psock
  backlog residue refills it, P-41 / K-B3); dispatch by (slot index, cell
  generation), never raw descriptor; Quint `NoStaleGenerationStart` (inv 14) +
  `ReleasedSlotCarriesNoData`; R5-14 (burst discarded; reply cut never reaches
  app; generation dispatch; quarantine). End-of-association order (feature-delta
  L18369).
- **Production-owner path:** `overdrive-init` end-of-association + readiness
  dispatch (by (slot, generation)); the guest cell framing/reassembly recreate.
- **Fault stimulus:** (a) burst parked then association ended; (b) scripted
  cell-creation failure (`ENOMEM`/`EMFILE`/sockmap insert) → quarantine.
- **Oracle:** universe = { parked-frame set after recreate, associations started
  after end, late-reply framing at the app, cell generation of a started
  association, quarantined-slot state }; delta = parked = ∅ after recreate; 0
  associations from old-generation events; 0 unstripped late replies; a started
  association's generation == the slot's current; quarantined slot owns no
  association and is not in the pool until the audit's retry succeeds (counter
  `guest_udp_slot_quarantined`).
- **Evidence tier:** Quint conformance (P3-S8, hazards `cells-emptied-only`,
  `fd-dispatch`, `fd-dispatch-generation`) + seeded-sim example (guest-owner
  seam) + P6 Tier-3 (the P-41 increment-e shape through `serve` + `deploy`).
  PBT only at the Quint/model layer; Tier-3 is example-based (Mandate 11).
- `#[ignore = "pending DELIVER: overdrive-init cell recreation + generation dispatch (CELL-RECREATE, R5-14)"]`

### P3-S6 — A refused or aborted association keeps its slot, discards parked frames, and re-associates only on the next application datagram `@driving_port`

```gherkin
Scenario: a destination refused Policy/HostInternal keeps the slot and does not self-retry
  Given a datagram association whose first forwarded datagram is refused Policy or HostInternal
  When the Refused (or Abort, or the FLOW_PAIRING_DEADLINE) ends the association
  Then the slot's parked frames are discarded by recreating the cells
    And the slot stays owned by its (application socket, destination)
    And no association is retried until the application sends again
    And each later application send causes at most one pairing attempt per round trip
        (a 500-send burst refused every time starts exactly one association per send, never from backlog)
    And a connected socket aborted after Paired re-associates on its next datagram
```

- **Contract/invariant:** U-2 — after `Refused`/`Abort`/pairing-deadline the
  slot's parked frames are discarded (by cell recreation) and the slot does not
  re-associate for that association; only a new application datagram opens a new
  one. Quint `NoReassociationWithoutNewDatagram`; R5-27 (at most one attempt per
  round trip; cells recreated so backlog never starts an association;
  re-associate on next datagram). CELL-RECREATE.
- **Production-owner path:** two seams. (a) **Owner side** (`vsock_owner_datagram_assoc.rs`):
  the host owner refusing a datagram association (`FlowRefusal::Policy` /
  `HostInternal`) and the guest receiving `Refused`/`Abort` — driven through the
  seeded-sim **private seam** (`SimGuestFlowForwarder` + the owner's flow-request
  delivery seam). (b) **Guest side** (model/Quint): `GuestRecvEnd` /
  `GuestPairingTimeout` keep the slot, discard parked, no self-reassoc.
- **Fault stimulus:** destination refused every attempt (500-send burst);
  connected socket aborted after `Paired`.
- **Oracle:** universe = { slot ownership, parked set, association count per
  send, `toHostAbort`/`toGuestRefused` }; delta = slot stays owned, parked = ∅,
  exactly one association per application send, re-association on the next
  datagram of a connected socket.
- **Evidence tier:** seeded-sim example (guest-owner seam) + Quint conformance
  (P3-S8, hazard `keep-frames-on-end`) + P6 Tier-3 (the P-41 refused-burst
  shape). Example-based at Tier-3 (Mandate 11).
- `#[ignore = "pending DELIVER: owner datagram refusal seam + guest keep-slot (U-2, R5-27)"]`

### P3-S7 — Concurrent datagram associations lose no datagram; every loss is attributed to a stage `@real-io`

```gherkin
Scenario: 4 senders x 250 datagrams through the production owner lose nothing unattributed
  Given 4 application sockets each sending 250 datagrams (small and fragmented, the V-18 shape)
  When the production guest-flow owner captures, pairs and parks every datagram concurrently
  Then every datagram is delivered or its loss is attributed to a named stage
    And no datagram is lost in capture, pairing or parking that recreation did not account for
```

- **Contract/invariant:** R5-14 concurrency clause — under concurrent
  associations no datagram is lost in capture/pairing/parking; every loss is
  attributed to a stage (the ~0.05% V-18 upstream residue is hypothesised cell
  residue that recreation removes, P-45). C7c (concurrent actors).
- **Production-owner path:** real `overdrive serve` + `overdrive deploy` guest
  datagram path (owner + guest programs + cells). **Tier-3 only** — a cell
  inside the **P6 walking skeleton** (shared-infra §6: real guest kernel).
- **Oracle:** universe = { per-sender delivered count, per-stage loss counters
  (capture / pairing / parking) }; delta = Σ delivered + Σ attributed losses ==
  Σ sent; 0 unattributed loss. The abort-to-reset latency is reported p50/p99,
  not asserted (R5-14 pattern).
- **Evidence tier:** Tier-3 example-based (P6 WS); the model's concurrency is
  the Quint `udp_slots` interleaving (P3-S8). P3 records the obligation; the
  executable cell is P6's.
- `#[ignore = "pending DELIVER: concurrent-no-loss Tier-3 cell inside P6 walking skeleton (R5-14)"]`

### P3-S8 — The guest-flow owner conforms to the `udp_slots` Quint model `@property`

```gherkin
Property: the production datagram-slot path follows the udp_slots model along generated traces
  Given the udp_slots Quint module (specs/quint/guest-flow-owner/udp_slots.qnt, referenced in place)
  When each generated trace step is replayed against the production guest datagram-slot path
  Then the projected state matches the model's state after every step
    And every udp_slots invariant has at least one conformance path
```

- **Contract/invariant:** every `udp_slots` invariant (SlotHeldOnce,
  ReleasedSlotCarriesNoData, RouteCurrent, LatePairedAborted,
  NoReassociationWithoutNewDatagram, NoStaleGenerationStart) + the
  `ReleasedSlotBecomesFree` liveness; the witnesses pin non-vacuity. (`.claude/rules/testing.md`
  §"Quint conformance".)
- **Production-owner path:** the conformance driver drives the production guest
  datagram-slot path (the owner's datagram-association seam + the guest slot
  model) through its driving ports with `Sim*` at driven ports; **State is
  projected from the observable surface** (owner's datagram-association table
  via its public/observable accessors; the slot pool's observable snapshot) —
  never private fields. **Testability gap G-3:** the design pins no public "step
  the datagram slot machine" surface; the driver depends on the shared
  `tests/conformance/src/quint_driver.rs` base (shared layer, first-needer
  creates — P3 does not edit it; reported §9) and on the owner's seeded-sim
  seam. Spec referenced **in place** (`udp_slots.qnt`), never copied.
- **Modes:** `#[quint_test]` for the named `checks.toml` runs
  (`udp-slots-safety-tlc`, `udp-slots-safety-apalache`,
  `udp-slots-release-liveness`, each `udp-slots-witness-*`); `#[quint_run]`
  randomized with the seed printed on failure. Hazard runs are the model's own
  teeth (not replayed against code — they prove the invariants bite).
- **Evidence tier:** conformance (spawns `quint`; `integration-tests`; `cargo
  xtask lima run --`; placed in `tests/conformance`). Complementary to
  seeded-sim, never a substitute (`.claude/rules/testing.md`).
- `#[ignore = "pending DELIVER: udp_slots quint-connect driver + shared quint_driver base + production owner seam"]`

### P3-S9 — A late `Paired` for a released slot is answered with `Abort`; the datagram direction + DNS obligations `@driving_port`

```gherkin
Scenario (S9a, SLOT-ABORT): a late Paired for a released slot is answered with Abort, routing nothing
  Given a datagram association whose slot was released (socket closed, or idle, or ended by U-2)
  When a Paired for that flow id arrives late
  Then the owner-side guest answers Abort and routes no frame to any association
    And a Paired for a still-live association of the same socket/destination is routed normally
    And a late Refused/Abort for an unknown or closed flow id is discarded and counted, no reply

Scenario (S9b, D4): the association uses STREAM toward the guest and SEQPACKET from the guest
  Given a paired datagram association
  Then host→guest datagrams travel on the STREAM leg (T) and guest→host on SEQPACKET (Q)
    And the host never writes on the SEQPACKET leg (the guest reassembles STREAM pieces of <= 4 KiB)

Scenario (S9c, D10): guest DNS travels as an ordinary datagram association to the node DnsResponder
  Given the guest resolver's nameserver is the guest gateway
  When the application resolves a name
  Then the DNS query travels as an ordinary datagram association whose destination is gateway:53
    And gateway:53 is the one guest-prefix UDP destination not refused Policy (#310)
```

- **Contract/invariant:** SLOT-ABORT (Quint `LatePairedAborted`; `GuestRecvPaired`
  answers `Abort` when the slot was released); U-5 (control message for an
  unknown/closed flow discarded + counted `guest_flow.control_discarded`, except
  SLOT-ABORT); D4 (STREAM toward guest, SEQPACKET from guest, host never writes
  SEQPACKET; guest reassembles ≤4 KiB pieces, G-MECH); D10 (DNS → gateway, as a
  datagram association to the node `DnsResponder`); the `Policy` refusal for a
  UDP guest-prefix destination other than `gateway:53` (feature-delta L18104).
- **Production-owner path:** S9a guest-side late-`Paired` handling (owner's
  datagram-association seam + `ControlMessage::Paired`/`Abort`); S9b the owner's
  datagram association install order (`T_h`/`Q_h`, per-kind total order step 4,
  feature-delta L18334) — the structural `PairedSocket` read-only proof that no
  `Read`/`AsyncRead` exists on the SEQPACKET-fed socket (trybuild-adjacent);
  S9c the owner's datagram association to `gateway:53` resolving to the
  `DnsResponder`. Driven through the seeded-sim private seam.
- **Oracle:** S9a universe = { `toHostAbort` (Abort emitted), routed-frame set,
  `control_discarded` counter }; delta = late-`Paired`-for-released → +1 Abort,
  0 frames routed; unknown/closed flow → +1 discarded, 0 reply; live-assoc
  `Paired` → routed. S9b universe = { leg kind per direction, host writes on Q };
  delta = host writes on Q == 0. S9c universe = { the datagram association's
  destination, refusal }; delta = `gateway:53` paired (not refused), any other
  guest-prefix UDP destination → `Refused(Policy)`.
- **Evidence tier:** seeded-sim example (owner seam) + Quint conformance (S9a =
  `LatePairedAborted`, hazard `ignore-late-paired` / `late-paired-by-slot-*`);
  S9b the structural `PairedSocket` test (compile-fail-adjacent) + Tier-3 wire
  observation in P6 WS; S9c unit (resolve branch) + P6 Tier-3 (real DNS, the
  P-29 `dig @1.1.1.1` shape). Example-based at Tier-3 (Mandate 11).
- `#[ignore = "pending DELIVER: owner datagram SLOT-ABORT + direction + DNS association (D4, D10, SLOT-ABORT, U-5)"]`

### P3-S10 — Slot exhaustion refuses a new destination with `EPERM` `@driving_port`

```gherkin
Scenario: a socket opening a GUEST_UDP_SLOTS+1-th destination is refused EPERM
  Given all GUEST_UDP_SLOTS (1_024) datagram slots are owned
  When the application sends a first datagram to a new destination
  Then the capture program refuses the sendmsg() with EPERM, counted guest_udp_slot_exhausted
    And existing associations are unaffected
```

- **Contract/invariant:** D25 "Exhaustion" — no free slot → `EPERM` on the new
  destination, counted `guest_udp_slot_exhausted`; existing flows unaffected
  (the per-allocation bulkhead, ADR-0147). D15-R2 (pool-size residual).
- **Production-owner path:** the guest capture program's refusal (Tier-3, guest
  kernel) — the executable cell is a **P6 walking-skeleton Tier-3 cell**; the
  host-runnable part is the pool-accounting invariant (1_024 slots, exhaustion
  at the 1_025-th distinct destination) if a pure slot-pool component is exposed
  (G-2). C1b boundary (1_024 ok, 1_025 refused).
- **Oracle:** universe = { free-slot count, the new send's result, existing
  associations }; delta = at 0 free slots the new-destination send → `EPERM`;
  existing associations unchanged.
- **Evidence tier:** host part layer-2 (if G-2 resolved) `parametrize` boundary;
  Tier-3 example in P6 WS.
- `#[ignore = "pending DELIVER: guest slot exhaustion EPERM (D25); Tier-3 in P6 walking skeleton"]`

---

## 5. Per-kind total-order obligations carried (datagram association, D18/G-V6)

The datagram-association total order (feature-delta L18310) is the owner-side
order P3-S6/S9 rest on; the acceptance bodies assert the observable effects, not
the internal step sequence:

1. guest `sendto` → slot intake U; verdict frames into the framing cell.
2. guest: framing-cell readiness → claim `FlowId`; connect `T_g` (STREAM 1240)
   + `Q_g` (SEQPACKET 1241); write `DatagramStream` on T_g,
   `DatagramSeqpacket` on Q_g.
3. host: accept both; read each request before installing either; match on
   (CID, flow) within `FLOW_REQUEST_DEADLINE`; mismatch kind/destination →
   `Refused(Malformed)` (**R5-17 is P2** — P3 does not own the mismatch body,
   only observes the refusal where S9 needs it).
4. host: policy + D26; create UDP `H` (`SO_NO_CHECK`), connect to `dst` (VIP →
   `connect4` rewrite, D24); `getpeername` guest-prefix check → `Refused(Policy)`
   unless `gateway:53`; register the unframe tuple from the kernel peer; install
   `T_h` target, `H→T_h` (framed), `Q_h→H` (hybrid strip, D5a); `Paired`.
5. guest: route framing cell → `Q_g`.

**End-of-association order** (CELL-RECREATE) is the P3-S5/S6 oracle:
cut reassembly output → close legs → recreate cells → disconnect slot socket →
remove guest-`lo` unframe tuple.

---

## 6. G-V6 datagram boundary scenarios (6-row table, feature-delta L20411)

| # | Datagram-association row | Scenario | Lane |
|---|---|---|---|
| 1 Available | `Paired`; frames flow in order after routing; early parked bytes included | P3-S6 (happy edge), P3-S8 | seeded-sim + Quint + P6 Tier-3 |
| 2 Typed failure in owner domain | `Refused(Policy/HostInternal/Capacity/Malformed)`; `PairingTimeout` after `FLOW_PAIRING_DEADLINE` | P3-S6, P3-S9c; P3-S10 (`EPERM`) | seeded-sim + Tier-3 |
| 3 Unrelated unchanged | other associations / allocations stay Running | P3-S6, P3-S7 | seeded-sim |
| 4 Late success can't resurrect | **SLOT-ABORT**: late `Paired` for a released slot → `Abort`, routes nothing (P3-S9a); late `Paired` for a still-closed flow → discarded (U-5) | P3-S9a | seeded-sim + Quint (`LatePairedAborted`) |
| 5 Disconnect / reconnect | control-session loss aborts live associations; a reconnect does not resurrect them; parked frames discarded by recreation | P3-S6 (abort path), cross-ref P2 session loss | seeded-sim |
| 6 Feature disabled | sim-driver fixture `transport: None` behaves as today's `network: None` — no owner, no slot pool | one fixture row in `vsock_owner_datagram_assoc.rs` | seeded-sim |

Rows 2/4/5 use a seeded `overdrive-sim` invariant through the production owner
path; the kernel-effect rows (1 at the wire, P3-S7) are native Tier-3 through
`serve` + `deploy` in the P6 walking skeleton.

---

## 7. Self-completeness audit (`nw-at-completeness-check`, 15-item)

| Item | Verdict | Evidence |
|---|---|---|
| C1a empty/zero/min | PASS | P3-S1 empty frame (0 bytes); P3-S2 N=0 slots; P3-S10 free-slot=0 |
| C1b boundary | PASS | P3-S1 59_000/59_001; P3-S10 1_024/1_025 |
| C2a state machine in docstring | PASS | §5 datagram total order + end-of-assoc order; the slot state machine is the `udp_slots.qnt` model (P3-S8) |
| C2b illegal-event-per-state | PASS | P3-S9a late `Paired` on a released slot (SLOT-ABORT); U-5 unknown-flow discard; P3-S5 old-generation readiness discarded |
| C3 count 0/1/N | PASS | P3-S2 N∈{0,1,2,1_024}; P3-S7 concurrent N |
| C4a idempotency / repeat | PASS | P3-S2 repeat send reuses slot (0 claims); P3-S6 at-most-one-attempt-per-send |
| C4b inverse without prerequisite | PASS | P3-S4 release of an idle/orphan slot; P3-S9a `Paired` with no slot |
| C5a mode combinations | PASS (N/A-documented) | P3 has no mode flags of its own; row 6 `transport: None` is the one disable flag |
| C5b flag orthogonality | N/A | no P3 mode flags beyond disable |
| C6a malformed input | PASS | P3-S1 bad magic / overlong length / truncated body → typed `Err` |
| C6b each declared error triggered | PASS | `Refused` codes Policy/HostInternal/Capacity/Malformed (P3-S6/S9/S10); frame decode errors (P3-S1) |
| C6c closed error set | PASS | P3-S1 asserts no decode error escapes the declared set |
| C7a degraded resource | PASS | P3-S5 cell-creation failure → quarantine; P3-S10 slot exhaustion |
| C7b interruption mid-flow | PASS | P3-S6 abort mid-association; P3-S4 lost report + audit |
| C7c concurrent actors | PASS | P3-S7 (4 senders); P3-S8 Quint interleavings |

**Count: 15/15 applicable PASS (C5b N/A documented) → COMPLETE** (≥13/15).
All gaps are `AT_GAP_IN_DELIVERY_SCOPE` (filled by DELIVER co-arrival) or the
testability gaps §9; **zero `SPECIFICATION_AMBIGUITY`** (no upstream C2/C5/C6/C7
artifact is absent — the design pins the datagram state machine in `udp_slots.qnt`,
the error contract in the wire tables, and the slot pool in D25). No
`CLARIFICATION_NEEDED` upstream routing.

Wave-Decision reconciliation (single authoritative design in `feature-delta.md`,
Accepted 2026-10-08): no contradiction detected within P3's decisions
(D4/D5/D10/D25/D25-BIND/U-2/CELL-RECREATE are mutually consistent and consistent
with the Quint `udp_slots` model). The full cross-piece reconciliation + the
four-reviewer Final Wave Review Gate run over the assembled six-piece set
(shared-infra §8).

---

## 8. PBT / parametrize density (EXPAND)

| Surface | Paradigm | Rationale |
|---|---|---|
| Datagram frame codec (P3-S1) | `proptest` full (`st.vec(u8, 0..=59_000)`) + pinned `@example` | unbounded payload domain, symmetric roundtrip invariant (Mandate 9 layer 1) |
| `guest_reserved_ports` / reserved-protocol predicate (P3-S3) | `proptest` over `u16` | unbounded port domain, boolean invariant |
| Slot cardinality N (P3-S2), pool boundary (P3-S10) | `parametrize` {0,1,2,1_024,1_025} | finite closed domain (falsifier-gate: parametrize, not PBT) |
| Owner seeded-sim (S4/S5/S6/S9) | example-based | layer 3+ / seeded-sim — sad paths enumerated (Mandate 11) |
| Quint conformance (P3-S8) | model-generated traces (`#[quint_run]` randomized + `#[quint_test]` named) | the model is the generator; production replays |
| Concurrency (P3-S7) | example-based Tier-3 | real guest kernel, Mandate 11 |

---

## 9. Testability gaps + coordination (reported, not invented)

- **G-1 (frame codec type).** D5 pins `DATAGRAM_FRAME_MAGIC`,
  `DATAGRAM_FRAME_HEADER_LEN`, `MAX_GUEST_DATAGRAM_PAYLOAD` and that a "Rust
  codec" exists (golden-bytes, L18174) but **not** the exact codec type/method
  (unlike `FlowRequest`/`ControlMessage`, which have `encode`/`decode`). P3-S1
  is authored against the design-implied `overdrive_core::guest_transport::wire`
  frame-codec shape; DELIVER pins the exact signature. **Not a DESIGN blocker**
  (internal structure per CLAUDE.md); a note for the DELIVER reviewer.
- **G-2 (pure guest slot-table / pool component).** D25 describes the
  slot-ownership table + dispatch + pool as `overdrive-init` internal state read
  from the kernel slot map; it pins no **pure, host-testable** slot-table/pool
  component. P3-S2/S4/S5/S10's host-runnable halves need one; absent it, those
  halves collapse into the Quint `udp_slots` conformance (P3-S8) + the P6 Tier-3
  cells. Test-support surface is DISTILL's to shape (CLAUDE.md) — recommend
  DELIVER expose a pure `SlotTable` component with a kernel-map read port; if it
  does not, the obligation is discharged by P3-S8 + P6 Tier-3 and this is
  documented, not a blocker.
- **G-3 (owner datagram-flow seam + shared quint_driver base).** The owner's
  flow acceptor is internal (D7). The seeded-sim lane and the Quint conformance
  driver both depend on (a) the owner's private flow-request-delivery seam
  (`SimGuestFlowForwarder`, shared-infra §3 — **shared**, first of P2/P3/P6
  creates; P3 does not edit it) and (b) `tests/conformance/src/quint_driver.rs`
  (shared base, first-needer creates; P3 does not edit it). P3's conformance
  file specialises that base. **Reported, not edited** (task: do not edit shared
  files).
- **G-4 (`quint-connect` 0.1.2 API).** The exact `quint-connect` driver API
  (trace ingest, per-step dispatch trait, `#[quint_test]` / `#[quint_run]`
  macro surface) is not vendored locally (offline). P3-S8 is authored against
  the shape `.claude/rules/testing.md` §"Quint conformance" + shared-infra §4
  describe; DELIVER pins it when the shared base lands.
- **G-5 (R5-15 / R5-14 / R5-27 / P3-S7 / P3-S10 Tier-3 cells → P6).** The real
  guest-kernel behaviours (EADDRINUSE bind, concurrent capture-no-loss,
  refused-burst re-association, slot-exhaustion EPERM) run as cells **inside the
  P6 walking skeleton** (shared-infra §6). P3 owns their **spec**; P6 owns their
  executable cells + the `host-kernel-shared` nextest entries. Coordination
  recorded; no P3 file change.
- **Entrypoint ownership.** P3 owns creating `crates/overdrive-init/tests/acceptance.rs`
  at DELIVER co-arrival (distinct from P6's `tests/integration.rs`); the
  `overdrive-core` and `overdrive-control-plane` entrypoints already exist —
  P3's `mod` lines are added at co-arrival, not now (shared-infra §6).
