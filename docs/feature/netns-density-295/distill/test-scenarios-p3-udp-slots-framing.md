# DISTILL — P3: datagram associations, UDP slots, framing, cells

> **DISTILL status (2026-10-10, second increment after DG-1..DG-16).** Every
> round-1 `UI-*` item P3 depended on is closed: the Rust frame codec by DG-8
> (`DatagramFrame` / `DatagramFrameError`), the guest owner by DG-5 / QPROJ-5
> (`overdrive-init` `guest_flow` module, source-local tests), the host owner's
> seam by DG-3 / QPROJ-1 (`GuestFlowOwnerPorts`, `with_flow_io`,
> `SimGuestFlowIo`). What is authored now and what is authored at the
> attachment cut (DG-2) is recorded per scenario below and summarised in §1.
> Authored now: P3-S1 (`overdrive-core` `vsock_datagram_frame`), the guest
> halves of P3-S2, S4, S5, S6, S9, S10 (`overdrive-init`
> `guest_flow::tests::udp_slots`) and the guest side of the P3-S8 driver
> (`overdrive-init` `guest_flow::quint_udp_slots`; model fidelity open,
> UI-R2-5 — a correct implementation may fail it until the model decision). Authored at the attachment
> cut: the host halves (P3-S6 owner refusal, P3-S7, P3-S9 host half; DG-2 rows
> 2, 3, 7, 9, new-step 10), the `udp_slots` host-side driver, and the
> real-guest-kernel cells (P3-S3, the Tier-3 parts of S5, S6, S7, S10) inside
> the P6-S13 walking skeleton. The frame's kernel behaviour is exercised by
> P5's `vsock_egress_unframe.rs` / `vsock_fragment_map.rs`.

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

## 0. Authoring times (DG-2)

DG-2 splits P3 by the surface each body names:

- **Authored now** — bodies that name only additive surface: the core frame
  codec and constants (`overdrive_core::guest_transport::wire::DatagramFrame`,
  DG-8; `GUEST_UDP_SLOTS`, `GUEST_DATAGRAM_SLOT_IDLE`, `guest_reserved_ports()`),
  and the `overdrive-init` `guest_flow` module with its seam sim
  `SimGuestOwnerIo` + `SimGuestClock` (`#[cfg(test)] guest_flow::sim`, read
  surface `guest_socket_observation`) and the owner's QPROJ-5
  `guest_observation()`. Each body is complete, compiles, and carries
  `#[ignore = "pending DELIVER vsock new-step N (…)"]`.
- **Authored at the attachment cut** — bodies that must call a DG-2 row-1..18
  replacement: the host owner's datagram-association side (`pool.assign` →
  `provision` → `activate` over `SimGuestFlowIo`, rows 2, 3, 7, 9, plus
  new-step 10 for datagrams) and the `udp_slots` host-side driver. These cannot
  compile before the cut and are not in the tree; their plan is the
  feature-delta § *Attachment-cut DISTILL increment*.
- **Real guest kernel** — the `EADDRINUSE` bind (P3-S3), the capture program's
  `EPERM` (P3-S10), the concurrent no-loss cell (P3-S7) and the Tier-3 shapes of
  S5 / S6 are cells of the P6-S13 walking skeleton, at the cut (rows 1-6).

Validation items P3 rests on have all run: V-5(c) (P-41, CELL-RECREATE),
K-B2 / K-B3 (audit, recreate-vs-empty). Where the design leaves a value a body
needs unpinned, it is recorded as a round-2 upstream item (UI-R2-5), not
invented; the golden payload is DISTILL-owned fixture data (UI-R2-4,
reclassified).

---

## 1. File ownership and nextest group

| File | Crate / layer | Scenarios | Status |
|---|---|---|---|
| `crates/overdrive-core/tests/acceptance/vsock_datagram_frame.rs` (registered in `tests/acceptance.rs`) | `overdrive-core`, unit / `proptest` (layer 1) | P3-S1 | authored; ignored `pending DELIVER vsock new-step 2 (core types and wire codec)`; one default-lane GUARD (`refusals_render_their_pinned_messages`) |
| `crates/overdrive-init/src/guest_flow/tests/udp_slots.rs` (source-local `#[cfg(test)]`; binary crate, no library target, DG-5) | `overdrive-init`, guest-owner acceptance over `SimGuestOwnerIo` | P3-S2, S4, S5, S6, S9 guest half, S10 guest half | authored; ignored `pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)` |
| `crates/overdrive-init/src/guest_flow/quint_udp_slots.rs` (in-crate conformance module) | `overdrive-init`, `quint-connect` driver | P3-S8 guest side (`slots_ok`, `slots_live`) | authored; model fidelity open (UI-R2-5) — a correct implementation may fail it until the model decision; ignored `pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init); model fidelity open, upstream UI-R2-5` |
| `crates/overdrive-core/tests/acceptance/vsock_guest_transport_vocabulary.rs` (P1's file) | `overdrive-core`, unit | P3-S3 pure range fact | authored (P1); `the_reserved_range_holds_the_tcp_intake_and_every_udp_slot_above_the_ephemeral_range` ignored `pending DELIVER vsock new-step 5 (guest intakes, D25)` |
| host datagram-association tests, source-local beside the owner in `overdrive-control-plane` (starting draft: `overdrive-control-plane/tests/acceptance/vsock_owner_datagram_assoc.rs` @91f9436ff, removed) | `overdrive-control-plane`, seam acceptance over `SimGuestFlowIo` | P3-S6 owner half, P3-S7 owner half, P3-S9 host half, §6 row 6 | attachment cut (DG-2 rows 2, 3, 7, 9; new-step 10) |
| `udp_slots` host-side driver, in-crate in `overdrive-control-plane` | `quint-connect` driver | P3-S8 host side (`slots_ok` / `slots_live`) | attachment cut (DG-2 rows 2, 3, 7, 8, 9) |
| P6-S13 walking-skeleton cells (native metal, `run_server` + public API) | Tier-3 | P3-S3, Tier-3 parts of P3-S5, S6, S7, S10, S9b / S9c wire | attachment cut (DG-2 rows 1-6) |

The files `overdrive-init/tests/acceptance/vsock_guest_udp_slots.rs`,
`overdrive-control-plane/tests/acceptance/vsock_owner_datagram_assoc.rs` and
`tests/conformance/tests/vsock_udp_slots_conformance.rs` named by the previous
pass do not exist; `overdrive-init` has no `tests/` entrypoint (its tests are
source-local), and no shared `tests/conformance/src/quint_driver.rs` base
exists — each driver is an in-crate module of its owner.

**nextest group.** P3's authored tests touch no node-global kernel state (the
frame codec is pure; the guest-owner tests and driver run over the in-memory
`SimGuestOwnerIo`). They stay in the default lane and add no
`host-kernel-shared` entry. The real-guest-kernel cells run inside the P6-S13
walking skeleton, whose group assignment is P6's.

---

## 2. Mandate-12 domain vocabulary (Rust adaptation)

Domain concepts are the `overdrive-core` newtypes the design pins (no
`domain_types.py`): `GuestCid`, `LaunchToken`, `FlowId` / `FlowKind` /
`FlowOrigin`, `FlowRequest`, `ControlMessage` / `FlowRefusal`, `ListenPort`,
the frame codec `DatagramFrame` / `DatagramFrameError` (DG-8), and the
datagram constants `DATAGRAM_FRAME_MAGIC` / `DATAGRAM_FRAME_HEADER_LEN`
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

### P3-S1 — A guest datagram of any size crosses the transport frame unchanged `@property @driving_port @contract-shape:pure-function`

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
  `DatagramFrame::{new, payload, encode, decode}` and `DatagramFrameError::{Truncated,
  BadMagic, Oversize, LengthMismatch}` (DG-8; the "Rust codec" the golden-bytes
  test pins across, L18174).
- **Golden frames:** empty frame `OVD1 00 00 00 00`; maximum frame header
  `OVD1 00 00 E6 78` (BE 59_000) followed by a payload whose byte `i` is
  `i mod 251`. The payload is DISTILL-owned fixture data (UI-R2-4,
  reclassified): `i mod 251` is the golden, and the host and guest BPF-side
  goldens adopt the same bytes when they are authored.
- **Oracle:** universe = { decoded payload bytes, decode result variant };
  `encode(p) |> decode == Ok(p)` for every `p`; empty → header-only (len = 8);
  malformed (bad magic, length > 59_000, truncated body) → typed `Err`, and the
  partial-decode slot is never a payload (fail-closed). Plus the C6 closed-set
  assertion: no decode error escapes the declared set.
- **Evidence tier:** layer-1 unit, `proptest` full (`st.vec(any::<u8>(), 0..=59_000)`
  with pinned `@example` for 0, 1, 1_431, 59_000). Boundary C1b: 59_000 (max),
  59_001 (max+1 → `Err`), and (if the codec validates on encode) encode of
  > 59_000 refused.
- **Status — authored:** `overdrive-core` `tests/acceptance/vsock_datagram_frame.rs`::
  `every_payload_up_to_the_maximum_round_trips`, `every_accepted_byte_string_is_canonical`,
  `every_non_canonical_frame_is_refused`, `golden_empty_frame_is_the_header_alone`,
  `golden_maximum_frame_carries_the_inclusive_bound`, `the_spike_sizes_round_trip`,
  `a_payload_over_the_maximum_is_refused_oversize_by_new`,
  `a_declared_length_over_the_maximum_is_refused_oversize_by_decode`,
  `fewer_bytes_than_the_header_are_refused_truncated`, `a_wrong_magic_is_refused_bad_magic`,
  `a_body_shorter_or_longer_than_declared_is_refused_length_mismatch` — ignored
  `pending DELIVER vsock new-step 2 (core types and wire codec)`.
  `refusals_render_their_pinned_messages` is a default-lane GUARD (not ignored)
  over the pinned `Display` texts.

### P3-S2 — One UDP slot per (application socket, destination) `@driving_port @contract-shape:bounded-change`

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
- **Production-owner path:** the `overdrive-init` guest owner `GuestFlowOwner`
  (DG-5) driven over the seam sim `SimGuestOwnerIo` (the guest kernel's capture
  program, slot map and cells); slot ownership observed through QPROJ-5
  `guest_observation().slots` and the sim's `guest_socket_observation`. The full
  slot state machine is also replayed by the Quint `udp_slots` conformance
  (P3-S8).
- **Oracle:** universe = { owned-slot count for the socket, the set of
  (socket,dest) tags over owned slots, slot reuse on repeat destination };
  delta = N owned slots, all tags distinct, repeat send adds 0 slots.
- **Evidence tier:** guest-owner acceptance over `SimGuestOwnerIo` + Quint
  conformance (P3-S8). PBT: `parametrize` N ∈ {0, 1, 2, 1_024} (C3
  cardinality) — not generative (finite pool).
- **Status — authored:** `overdrive-init` `guest_flow::tests::udp_slots`::
  `one_slot_per_socket_destination_pair` — ignored `pending DELIVER vsock
  new-step 5 (guest adaptation in overdrive-init)`.

### P3-S3 — A reserved intake port rejects an application bind of the same protocol `@real-io @contract-shape:unbounded-preservation`

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
  without `SO_REUSEADDR`/`SO_REUSEPORT`). The pure range fact
  (`guest_reserved_ports()` = `61_000..=62_024`, TCP intake at the start, every
  UDP slot inside, the guest ephemeral range ending at 60_999) is P1's
  `vsock_guest_transport_vocabulary` test. The bind refusal itself needs a
  real guest kernel: it is a cell of the **P6-S13 walking skeleton** at the
  attachment cut (R5-15 lane). The design pins no host-side per-(port, protocol)
  bind-decision function, so no host predicate test is authored for it.
- **Oracle (pure part):** `guest_reserved_ports()` is `61_000..=62_024`
  inclusive, starting at `GUEST_TCP_INTAKE_PORT` and ending at
  `GUEST_UDP_SLOT_BASE + GUEST_UDP_SLOTS - 1`. **Oracle (Tier-3, P6-S13):**
  real `bind()` errno == `EADDRINUSE` on the reserved/protocol pairs and `Ok`
  on the free pairs; the TCP intake absent from two `NETLINK_SOCK_DIAG`
  `TCPF_LISTEN` dumps.
- **Evidence tier:** range fact layer-1 (P1's file); bind refusal Tier-3
  example-based in the P6-S13 walking skeleton (R5-15).
- **Status:** range fact authored in `overdrive-core`
  `vsock_guest_transport_vocabulary`::`the_reserved_range_holds_the_tcp_intake_and_every_udp_slot_above_the_ephemeral_range`
  — ignored `pending DELIVER vsock new-step 5 (guest intakes, D25)`.
  **Attachment cut (DG-2 rows 1-6):** the P6-S13 walking-skeleton cell drives
  `run_server` + the public API on native metal and asserts the real `bind()`
  outcomes and the `TCPF_LISTEN` dumps above.

### P3-S4 — An idle datagram association is released after 120 s; a lost release report is recovered by the audit `@driving_port @contract-shape:bounded-change`

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
- **Production-owner path:** `overdrive-init` `GuestFlowOwner` idle release
  (nothing parked) and audit (orphan slots whose cookie is gone), driven over
  `SimGuestOwnerIo` with the injected `SimGuestClock`.
- **Oracle:** universe = { owned-slot set, pool-free count, `toHostEnd` (the
  host observes the association's end) }; delta = idle slot moves owned→free
  after the idle deadline; orphan slot (socket closed, report lost) moves
  owned→free after the audit; the ended slot's `routed`/`fid`/`parked` cleared.
- **Evidence tier:** layer-2 seeded (clock injected), example-based (not PBT:
  the lane is layer 3+ / seeded-sim per Mandate 11); Quint conformance covers
  the liveness (P3-S8, `udp-slots-release-liveness`). Real idle + real audit at
  density is the P6 Tier-3 / V-9 cell.
- **Status — authored:** `overdrive-init` `guest_flow::tests::udp_slots`::
  `an_idle_association_is_released_and_a_busy_one_kept`,
  `a_lost_release_report_is_recovered_by_the_audit`,
  `a_release_report_releases_every_slot_of_the_socket` — ignored `pending
  DELIVER vsock new-step 5 (guest adaptation in overdrive-init)`. The audit's
  period is not pinned; the tests advance the clock to the owner's next
  deadline.

### P3-S5 — Ending an association recreates its cells and discards parked frames; no stale-generation event starts an association `@driving_port @property @contract-shape:bounded-change`

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
- **Production-owner path:** `overdrive-init` `GuestFlowOwner`
  end-of-association + readiness dispatch (by (slot, `CellGeneration`)); the
  cell recreation is a seam effect observed on `SimGuestOwnerIo`.
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
  `fd-dispatch`, `fd-dispatch-generation`) + guest-owner example over
  `SimGuestOwnerIo` + P6-S13 Tier-3 (the P-41 increment-e shape through
  `serve` + `deploy`). PBT only at the Quint/model layer; Tier-3 is
  example-based (Mandate 11).
- **Status — authored:** `overdrive-init` `guest_flow::tests::udp_slots`::
  `recreated_cells_discard_parked_frames_and_stale_generation_events`,
  `a_cell_creation_failure_quarantines_the_slot_until_the_audit_retry` — ignored
  `pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)`.
  **Attachment cut (DG-2 rows 1-6):** the reply cut by the association's end at
  the real application, and the burst on real cells, are P6-S13 walking-skeleton
  cells. The `guest_udp_slot_quarantined` counter has no accessor (UI-R2-6); the
  test asserts the quarantine through the slot state, not the counter.

### P3-S6 — A refused or aborted association keeps its slot, discards parked frames, and re-associates only on the next application datagram `@driving_port @contract-shape:bounded-change`

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
- **Production-owner path:** two seams. (a) **Host owner side:** the host
  owner refusing a datagram association (`FlowRefusal::Policy` /
  `HostInternal`), driven through `HostSharedGuestNetworkOwner::with_flow_io`
  over `SimGuestFlowIo` (`overdrive-control-plane` `#[cfg(test)]
  guest_network::flow_io::sim`). (b) **Guest side:** `GuestFlowOwner` receiving
  `Refused` / `Abort` or reaching `FLOW_PAIRING_DEADLINE` keeps the slot,
  discards parked frames, does not re-associate by itself — over
  `SimGuestOwnerIo` + `SimGuestClock`, and in the Quint `udp_slots` replay.
- **Fault stimulus:** destination refused every attempt (500-send burst);
  connected socket aborted after `Paired`.
- **Oracle:** universe = { slot ownership, parked set, association count per
  send, `toHostAbort`/`toGuestRefused` }; delta = slot stays owned, parked = ∅,
  exactly one association per application send, re-association on the next
  datagram of a connected socket.
- **Evidence tier:** guest-owner example + host-owner seam example + Quint
  conformance (P3-S8, hazard `keep-frames-on-end`) + P6-S13 Tier-3 (the P-41
  refused-burst shape). Example-based at Tier-3 (Mandate 11).
- **Status — authored (guest half):** `overdrive-init`
  `guest_flow::tests::udp_slots`::`a_refused_burst_keeps_the_slot_and_starts_one_association_per_send`,
  `an_aborted_paired_association_reassociates_only_on_the_next_datagram`,
  `the_pairing_deadline_aborts_the_association_and_keeps_the_slot` — ignored
  `pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)`.
- **Attachment cut (DG-2 rows 2, 3, 7, 9; new-step 10) — host half:** source-local
  beside the owner in `overdrive-control-plane` (starting draft
  `vsock_owner_datagram_assoc.rs` @91f9436ff): `pool.assign` → `provision` →
  queued 1240 / 1241 datagram connections with buffered requests on
  `SimGuestFlowIo` → `activate`; asserts O3 `flow_observation().datagrams`, the
  sim's `outbox` (`Refused(Policy | HostInternal)`) and `closed_by_owner`, with
  `FlowCloseCause`. The real refused burst is a P6-S13 cell.

### P3-S7 — Concurrent datagram associations lose no datagram; every loss is attributed to a stage `@real-io @contract-shape:unbounded-preservation`

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
- **Evidence tier:** Tier-3 example-based (P6-S13 walking skeleton); the
  model's concurrency is the Quint `udp_slots` interleaving (P3-S8). P3 records
  the obligation; the executable cell is P6's.
- **Status — attachment cut (DG-2 rows 2, 3, 7, 9; new-step 10):** no test is
  authored now. The host owner's concurrent-association half is part of the
  owner-flow seam tests in `overdrive-control-plane` (several queued datagram
  associations on `SimGuestFlowIo` → `activate`; asserts O3
  `flow_observation().datagrams` and the sim's `socket_observation()`). The
  no-loss count over a real guest kernel is a P6-S13 walking-skeleton cell
  (rows 1-6) on native metal.

### P3-S8 — The guest-flow owner conforms to the `udp_slots` Quint model `@property @contract-shape:bounded-change`

> **Driver status (2026-10-10).** Placement, `State` projection and action
> map are pinned by QPROJ-1, QPROJ-5, QPROJ-7 and DG-15. Guest side **authored**:
> `crates/overdrive-init/src/guest_flow/quint_udp_slots.rs`, `#[quint_run]`
> instances `quint_udp_slots_guest_slots_ok` (`slots_ok`) and
> `quint_udp_slots_guest_slots_live` (`slots_live`) — ignored `pending DELIVER
> vsock new-step 5 (guest adaptation in overdrive-init); model fidelity open,
> upstream UI-R2-5`. Authored; model fidelity open (UI-R2-5) — a correct
> implementation may fail them until the model decision. Host side: **attachment
> cut** (DG-2 rows 2, 3, 7, 8, 9), an in-crate driver in `overdrive-control-plane`
> over the QPROJ host-side action map on `SimGuestFlowIo`, projecting O2, O3 and
> S `socket_observation()`.

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
- **Production-owner path (guest side):** the driver holds the production
  `GuestFlowOwner` over `SimGuestOwnerIo` + `SimGuestClock` and plays the host
  owner and the applications as stimulus. Actions: `AppClaim` / `AppSendOnSlot`
  → `app_send_on`; `GuestReady(s, cur)` → the seam releases the readiness
  event(s), then both leg connects complete; `GuestRecvPaired` / `GuestRecvEnd`
  → `host_control`; `GuestPairingTimeout`, `GuestAudit`, `IdleRelease` →
  `SimGuestClock`; `SockClose` / `SockCloseReportLost` → `app_close_socket`;
  `GuestReleaseSock` → `release_socket_report`; host actions are stimulus;
  `AppPick`, `AppTake`, `Refill` are hazard-only and return a typed error.
  **State** is projected from O5 `GuestFlowOwner::guest_observation().slots`
  (`slots(s).owned / sock / dest / fid / routed`) and from the QPROJ-7 arbiter
  `SimGuestOwnerIo::guest_socket_observation` (`parked`, `ready` — current iff
  O5's generation —, `releasePending`, `toHostAbort` ← `outbox`, `toHostEnd` ←
  `legs_closed_unobserved`); ghost and hazard-only variables are not compared.
  Spec referenced **in place** (`udp_slots.qnt`), never copied.
- **Model bound (UI-R2-5(b)):** the model bounds association ids by `MAXFID`
  (3 for `slots_ok`, 2 for `slots_live`); the production owner has no such
  bound. A `GuestReady` that would open an association past `MAXFID` returns
  `DriverError::ModelBound`, so a trace is not replayed past that point.
- **Production-owner path (host side, attachment cut):** the in-crate
  `overdrive-control-plane` driver over `HostSharedGuestNetworkOwner::with_flow_io`
  and `SimGuestFlowIo`.
- **Modes:** `#[quint_run]` randomized traces only, with the seed printed on
  failure. The specs define no Quint `run`, so there is no `#[quint_test]` pin
  (UI-R2-1); the `checks.toml` `udp-slots-*` entries are invariant, witness and
  hazard checks of the model, run by `cargo xtask quint check`, not replayed
  against code.
- **Evidence tier:** conformance (spawns `quint`; `cargo xtask lima run --`),
  an in-crate module of the owner (`overdrive-init` guest side,
  `overdrive-control-plane` host side). Complementary to the guest-owner
  examples, never a substitute (`.claude/rules/testing.md`).

### P3-S9 — A late `Paired` for a released slot is answered with `Abort`; the datagram direction + DNS obligations `@driving_port @contract-shape:bounded-change`

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
  feature-delta L18334) — the structural rule that no read or write exists for
  a `PairSocket` of `GuestFlowIo` (DG-3; enforced by the seam signatures, no
  `trybuild` fixture, since the types are crate-private);
  S9c the owner's datagram association to `gateway:53` resolving to the
  `DnsResponder`. Guest half (S9a) over `SimGuestOwnerIo`; host half (S9a host
  side, S9b, S9c) over `SimGuestFlowIo` at the attachment cut.
- **Oracle:** S9a universe = { `toHostAbort` (Abort emitted), routed-frame set,
  `control_discarded` counter }; delta = late-`Paired`-for-released → +1 Abort,
  0 frames routed; unknown/closed flow → +1 discarded, 0 reply; live-assoc
  `Paired` → routed. S9b universe = { leg kind per direction, host writes on Q };
  delta = host writes on Q == 0. S9c universe = { the datagram association's
  destination, refusal }; delta = `gateway:53` paired (not refused), any other
  guest-prefix UDP destination → `Refused(Policy)`.
- **Evidence tier:** guest-owner example (S9a guest half) + host-owner seam
  example at the cut + Quint conformance (S9a = `LatePairedAborted`, hazard
  `ignore-late-paired` / `late-paired-by-slot-*`); S9b the seam signatures
  (DG-3, reviewer-checked) + Tier-3 wire observation in P6-S13; S9c unit (resolve branch) + P6 Tier-3 (real DNS, the
  P-29 `dig @1.1.1.1` shape). Example-based at Tier-3 (Mandate 11).
- **Status — authored (guest half):** `overdrive-init`
  `guest_flow::tests::udp_slots`::`a_late_paired_for_a_released_slot_is_answered_with_abort`
  — ignored `pending DELIVER vsock new-step 5 (guest adaptation in
  overdrive-init)`. The guest's U-5 discard of a control message for an
  unknown or closed flow is P2's
  `guest_flow::tests::tcp_accept::control_messages_for_unknown_or_closed_flows_are_discarded_without_reply`
  (same ignore reason). The `control_discarded` counter has no guest accessor
  (UI-R2-6).
- **Attachment cut (DG-2 rows 2, 3, 7, 9; new-step 10) — host half:** owner-flow
  seam tests in `overdrive-control-plane` (queued 1240 / 1241 connections on
  `SimGuestFlowIo` → `activate`) assert O3 `flow_observation().datagrams`, the
  sim's `outbox` and `closed_by_owner` with `FlowCloseCause`: a late `Paired`
  path routes nothing, and a datagram association to `gateway:53` pairs while
  any other guest-prefix UDP destination is `Refused(Policy)` (S9c). S9b's
  "host never writes on SEQPACKET" is structural (no write exists for a
  `PairSocket` of `GuestFlowIo`, DG-3); its wire observation is a P6-S13 cell.
  The `gateway:53` end-to-end accept is a P6-S13 cell (UI-R2-14).

### P3-S10 — Slot exhaustion refuses a new destination with `EPERM` `@driving_port @contract-shape:unbounded-preservation`

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
  kernel) — the executable cell is a **P6-S13 walking-skeleton cell**; the
  guest-owner part (every slot owned, a new destination refused, existing
  associations unaffected) runs over `SimGuestOwnerIo`, where the sim plays the
  capture program. C1b boundary (1_024 ok, 1_025 refused).
- **Oracle:** universe = { free-slot count, the new send's result, existing
  associations }; delta = at 0 free slots the new-destination send → `EPERM`;
  existing associations unchanged.
- **Evidence tier:** guest-owner example over `SimGuestOwnerIo`; Tier-3
  example in the P6-S13 walking skeleton.
- **Status — authored (guest half):** `overdrive-init`
  `guest_flow::tests::udp_slots`::`exhaustion_refuses_a_new_destination_and_leaves_existing_associations`
  — ignored `pending DELIVER vsock new-step 5 (guest adaptation in
  overdrive-init)`. **Attachment cut (DG-2 rows 1-6):** the real `EPERM` from
  the guest capture program and the `guest_udp_slot_exhausted` count are a
  P6-S13 walking-skeleton cell.

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
| 1 Available | `Paired`; frames flow in order after routing; early parked bytes included | P3-S6 (happy edge), P3-S8 | guest-owner example (authored) + Quint + P6-S13 Tier-3 (cut) |
| 2 Typed failure in owner domain | `Refused(Policy/HostInternal/Capacity/Malformed)`; `PairingTimeout` after `FLOW_PAIRING_DEADLINE` | P3-S6, P3-S9c; P3-S10 (`EPERM`) | guest-owner example (authored); host-owner seam example and Tier-3 (cut) |
| 3 Unrelated unchanged | other associations / allocations stay Running | P3-S6, P3-S7 | guest-owner example (authored, P3-S10 existing associations); host-owner seam (cut) |
| 4 Late success can't resurrect | **SLOT-ABORT**: late `Paired` for a released slot → `Abort`, routes nothing (P3-S9a); late `Paired` for a still-closed flow → discarded (U-5) | P3-S9a | guest-owner example (authored) + Quint (`LatePairedAborted`) |
| 5 Disconnect / reconnect | control-session loss aborts live associations; a reconnect does not resurrect them; parked frames discarded by recreation | P3-S6 (abort path), cross-ref P2 session loss (`guest_flow::tests::session`) | guest-owner example (authored) |
| 6 Feature disabled | sim-driver fixture `transport: None` behaves as today's `network: None` — no owner, no slot pool | one fixture row of the host datagram-association tests at the attachment cut (`AllocationSpec.transport` is DG-2 row 4) | attachment cut |

Rows 2/4/5 run through the production owner path (the guest owner over
`SimGuestOwnerIo` now, the host owner over `SimGuestFlowIo` at the cut); the
kernel-effect rows (1 at the wire, P3-S7) are native Tier-3 through `serve` +
`deploy` in the P6-S13 walking skeleton.

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

**Count: 15/15 applicable PASS (C5b N/A documented) → COMPLETE** (≥13/15)
over the P3 scenario set. Over what is authored now, the C5a disable row (§6
row 6) and the host halves are `AT_GAP_IN_DELIVERY_SCOPE`, authored at the
attachment cut (feature-delta § *Prerequisites and Completeness*: C5a, C5b, C6c
owed to the cut bodies); **zero `SPECIFICATION_AMBIGUITY`** (no upstream C2/C5/C6/C7
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
| Guest-owner examples (S2/S4/S5/S6/S9/S10) and host-owner seam examples at the cut | example-based | seam-sim layer — sad paths enumerated (Mandate 11) |
| Quint conformance (P3-S8) | model-generated traces (`#[quint_run]` randomized only; no `#[quint_test]`, UI-R2-1) | the model is the generator; production replays |
| Concurrency (P3-S7) | example-based Tier-3 | real guest kernel, Mandate 11 |

---

## 9. Testability gaps + coordination

Round-1 gaps, all closed:

- **G-1 (frame codec type)** — closed by DG-8: `DatagramFrame::{new, payload,
  encode, decode}`, `DatagramFrameError`.
- **G-2 (host-testable guest slot pool)** — closed by DG-5 / QPROJ-5 /
  QPROJ-7: the `overdrive-init` `guest_flow` module (`GuestFlowOwner`,
  `guest_observation()`) with the seam sim `SimGuestOwnerIo` + `SimGuestClock`;
  P3-S2, S4, S5, S6, S9 and S10 guest halves are authored over it.
- **G-3 (owner datagram-flow seam, shared `quint_driver` base)** — closed by
  DG-3 / QPROJ-1: the host owner's seam is `GuestFlowOwnerPorts` /
  `with_flow_io` with `SimGuestFlowIo`; conformance drivers are in-crate
  modules of their owner; no shared base or `SimGuestFlowForwarder` exists.
- **G-4 (`quint-connect` API)** — closed: `quint-connect` is a dev-dependency
  of `overdrive-init` and the guest driver compiles against it; drivers use
  `#[quint_run]` only (UI-R2-1).
- **G-5 (real-guest-kernel cells → P6)** — coordination, unchanged in
  substance: the `EADDRINUSE` bind, the concurrent no-loss count, the
  refused-burst shape and the exhaustion `EPERM` are P6-S13 walking-skeleton
  cells, authored at the attachment cut (DG-2 rows 1-6). P3 owns their spec.
- **Entrypoint ownership** — obsolete: `overdrive-init` tests are source-local
  (DG-5); `vsock_datagram_frame` is registered in `overdrive-core`'s
  `tests/acceptance.rs`.

Round-2 items affecting P3 (`upstream-issues.md` round 2):

- **UI-R2-4** — reclassified as DISTILL-owned fixture data: byte `i` =
  `i mod 251` behind the pinned header `OVD1 00 00 E6 78` is the golden; the
  BPF-side goldens adopt it.
- **UI-R2-5** — the guest driver cannot faithfully replay some model
  situations; for `udp_slots`, (b) the model's `MAXFID` bound has no production
  counterpart, so the driver stops with `DriverError::ModelBound`; one
  `SimGuestClock` drives every guest timer (pairing deadline, idle, audit).
- **UI-R2-6** — guest counters (`guest_udp_slot_quarantined`,
  `guest_udp_slot_exhausted`, `guest_flow.control_discarded`) have no accessor;
  the authored tests assert the state they count (or, for exhaustion, the
  sim's own `exhausted_sends()` count of refused sends), not the counters.
