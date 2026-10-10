# P2 — Guest-flow owner: control session, flow admission, pairing, launch attribution

> **DISTILL status (2026-10-10, second increment after DG-1..DG-16).** Every
> round-1 `UI-*` item is closed by a DG decision (UI-C1 by DG-2, UI-C2 by DG-3
> / DG-4, UI-BEACON-1 by DG-7). One round-2 item bears on P2: the two guest
> `owner_flows` drivers (P2-S15) are authored, but their model fidelity is
> open (UI-R2-5, HIGH) — a correct implementation may fail them until the
> model decision. DG-2
> sorts each scenario into one of two authoring times:
>
> - **Authored now** (additive surface only; compiled bodies with a reasoned
>   `#[ignore]`): P2-S1 and P2-S2 (`overdrive-core` `vsock_wire_codec`); the
>   guest halves of P2-S10 and P2-S12 (`overdrive-init`
>   `guest_flow::tests::{tcp_accept, tcp_connect, half_close, session}`); the
>   additive half of P2-S14 (`overdrive-worker` `vsock_beacon_intake`, plus the
>   `SimBeaconIo` unit tests in `overdrive-sim`); the guest side of P2-S15
>   (`overdrive-init` drivers `quint_owner_flows.rs` and `quint_flows_half.rs`).
> - **Authored at the attachment cut** (bodies that name a DG-2 row-1..18
>   replacement): P2-S3..S9, P2-S11, P2-S13 and the host halves of P2-S10 /
>   P2-S12 (DG-2 rows 1-3, 7-9); the attribution cells of P2-S14 (rows 13, 15);
>   the READY `launch=` parse cells of P2-S1 (row 13, DG-7); the host side,
>   `flows_half_host` and `flows_beacon` instances of P2-S15. Their plan is the
>   feature-delta § *Attachment-cut DISTILL increment*.
>
> The 91f9436ff drafts `overdrive-control-plane/tests/acceptance/vsock_owner_flow_admission.rs`,
> `vsock_owner_datagram_assoc.rs` and `overdrive-worker/tests/acceptance/vsock_beacon_attribution.rs`
> no longer exist; the AC table names them as starting drafts. Per-scenario
> **Status** lines below give the exact tests and ignore reasons. Classes:
> `red-classification.md`.

Scope: the node-scoped guest-flow owner's control session (D18a), out-of-band
flow admission + install-at-establishment + pairing (D18, G-V6), launch
attribution by peer-CID + `LaunchToken` against the **live** launch (ATTR-TOKEN,
ATTR-LIVE-CLAIM), the flow-request / control-message / token wire codec, and
the cross-boundary half-close handshake codec (HALFCLOSE-SIGNAL). Mesh vs
non-mesh TCP resolution-at-pairing (D9, D14) and the no-owner-deadline non-mesh
connect bound (D15-R3). Oracle: Quint `owner_flows` (invariants 1, 3, 4, 4+, 9,
13). See [shared-test-infrastructure.md](shared-test-infrastructure.md).

Owns: D7, D9, D14, D17, D18, D18a, D15-R1, D15-R3, ATTR-TOKEN, ATTR-LIVE-CLAIM,
U-1, U-5. Gates G-V6 (TCP flow pairing) + G-V3 (the Active/session precondition,
listeners themselves in P4). Quint `owner_flows`. R5-9, R5-16, R5-17, R5-18,
R5-20, R5-26, R5-28, R5-31, R5-41, R5-45 (codec + owner-seam). Boundary-table
rows for G-V6 + G-V3's session/attribution clauses.

Binding rules: no `.feature` files; GWT scenarios are spec-only here; the
crafter translates each to a Rust `#[test]`/`#[tokio::test]`
(`.claude/rules/testing.md`). PBT is `proptest` (Rust). Layer-dependent PBT
mode (Mandate 9): PBT at the pure codec layer; example-based at the seeded-sim
/ conformance layer (every sad path enumerated, never PBT-generated).

## Files owned (disjoint)

Authored now:

| File | Lane | Layer | Scenarios |
|---|---|---|---|
| `crates/overdrive-core/tests/acceptance/vsock_wire_codec.rs` | default lane, unit + `proptest` | 1 (pure function) | P2-S1, P2-S2 |
| `crates/overdrive-init/src/guest_flow/tests/{tcp_accept,tcp_connect,half_close,session}.rs` (source-local `#[cfg(test)]`; binary crate, no library target, DG-5) | default lane, over `SimGuestOwnerIo` + `SimGuestClock` (`guest_flow::sim`) | 2–3 (example-based) | guest halves of P2-S10, P2-S12 |
| `crates/overdrive-worker/tests/acceptance/vsock_beacon_intake.rs` | default lane, over `SimBeaconIo` + `SimVmm` | 2–3 (example-based) | P2-S14 additive half |
| `crates/overdrive-sim/src/adapters/beacon_io.rs` unit tests | default lane | the sim's own semantics | P2-S14 seam (arbiter of the beacon accept queue) |
| `crates/overdrive-init/src/guest_flow/quint_owner_flows.rs`, `quint_flows_half.rs` (`#[cfg(all(test, feature = "integration-tests"))]`) | quint-connect, Lima, `integration-tests` | conformance | P2-S15 guest side (`flows_ok`, `flows_half_guest`) |

Authored at the attachment cut (DG-2), source-local beside the owner:

| Item | Crate | Scenarios | DG-2 rows |
|---|---|---|---|
| host owner admission / pairing / attribution / session tests, over `HostSharedGuestNetworkOwner::with_flow_io` + `SimGuestFlowIo` | `overdrive-control-plane` (owner of `HostSharedGuestNetworkOwner`) | P2-S3..S9, S11, S13; host halves of S10, S12 | 1-3, 7-9 (+ new-step 10 for datagrams) |
| beacon attribution cells, `VmDriver::new(.., BeaconIntake::bind(SimBeaconIo::stepped(vmm)))` | `overdrive-worker` | P2-S14 attribution half | 13, 15 |
| READY `launch=` parse cells | `overdrive-core` | P2-S1 READY half | 13 (DG-7) |
| `owner_flows` host side (`flows_ok`), `flows_half_host` drivers | `overdrive-control-plane` | P2-S15 | 2, 3, 7, 8, 9 |
| `owner_flows` `flows_beacon` driver | `overdrive-worker` | P2-S15 | 13, 15 |

The wire codec is **the one codec home** for the flow request, control message,
control open and `LaunchToken` (the datagram-frame codec is P3's
`vsock_datagram_frame.rs`; the beacon-line codec stays in
`overdrive_core::vm::beacon`, exercised here only for the `launch` field).

## Shared dependencies (seams and sims; pinned by DG-3, DG-4, DG-5, QPROJ-7)

- `overdrive-control-plane` `guest_network::flow_io` — the host owner effect
  seam `GuestFlowIo` (DG-3) with its production adapter `HostGuestFlowIo` and
  the `#[cfg(test)]` sim `flow_io::sim::{SimGuestFlowIo, SimGuestNode}` (read
  surfaces `socket_observation`, `steering_observation`, `steering_effects`;
  scripted outcomes such as `set_route_type`). The owner is built with
  `HostSharedGuestNetworkOwner::with_flow_io` over `GuestFlowOwnerPorts`
  (`clock`, `resolve` — `SimMtlsResolve` under test —, `mtls`, `vip_range`,
  `wildcard_tcp_ports`, `node_prefix`, …). The owner's observable surface is
  `flow_observation()` (QPROJ-3) and `lease_observation()` (QPROJ-2), never
  private fields. This replaces the earlier `SimGuestFlowForwarder` and
  "private flow-admission test seam" shapes, which do not exist.
- `GuestAddressPool` over `SimGuestCidClaim` and `SimEntropy` (P1-owned;
  pool construction and `assign` are DG-2 rows 1, 2).
- The beacon port `BeaconIo` (`overdrive-core::vm::beacon`, with `BeaconSocket`
  / `BeaconConnKey`), `BeaconIntake::bind` and `VmDriver::beacon_observation`
  (QPROJ-4) in `overdrive-worker`, and the sim `SimBeaconIo` (`overdrive-sim`,
  over `SimVmm`; read surface `beacon_socket_observation`) — DG-4.
- The guest owner seam `GuestOwnerIo` with `guest_flow::sim::{SimGuestOwnerIo,
  SimGuestClock}` (`overdrive-init`, `#[cfg(test)]`; read surface
  `guest_socket_observation`) and the owner accessor `guest_observation`
  (QPROJ-5) — DG-5.

Every authored file is registered and compiles now (DG-2 forbids an
unregistered, uncompiled or always-false-`cfg` file). Bodies that name a
replaced signature are not in the tree until the attachment cut, where DISTILL
writes them first, in the cut's working tree.

## Scenarios (GWT, spec-only)

Every scenario carries a `@contract-shape:` tag (2026-05-15 mandate). The
Outcome Elevator Pitch uses ubiquitous-language verbs (no "returns 200", "exit
code zero").

### Wire codec — unit + proptest (`vsock_wire_codec.rs`)

**P2-S1 codec roundtrip + reject-all-zero** `@contract-shape:pure-function`
- **Outcome**: every flow-request kind (`TcpConnect`, `DatagramStream`,
  `DatagramSeqpacket`, `TcpAccept`), `ControlOpen`, every `ControlMessage` type
  and `LaunchToken` survives encode→decode carrying its launch token unchanged;
  an all-zero token is refused.
- **Contract**: `FlowRequest::decode(encode(x)) == x` ∀ valid `x`;
  `decode` accepts only canonical encodings; all-zero `LaunchToken` →
  `FlowWireError::InvalidField { field: WireField::LaunchToken }`;
  `LaunchToken::from_bytes([0;16]) == Err(Zero)`;
  `LaunchToken` `FromStr`/`Display` round-trips 32 lowercase hex and rejects
  upper case, wrong length and all zero (R5-41 codec, D18, D18a, ATTR-TOKEN).
- **Invariant**: round-trip equality over every valid value and every accepted
  byte string (D5/D18 "decode accepts only canonical encodings").
- **Oracle**: `proptest` equality + rejection; `@example`-pinned golden bytes
  for each request kind, the control open, each control-message type
  (`HalfClose`/`HalfCloseAck` incl.), each refusal code and the kernel
  command-line token form (`overdrive.transport=vsock2,...,launch=<hex>`).
- **Fault stimulus**: bad magic, unsupported version, unknown kind,
  origin/kind mismatch, all-zero token, non-zero `ControlOpen` reserved byte,
  unknown control type, unknown refusal code — each → its named `FlowWireError`.
- **Evidence tier**: unit + proptest (layer 1).
- **Status**: authored: `overdrive-core` `vsock_wire_codec`::{`every_flow_request_round_trips`, `every_accepted_request_is_canonical`, `every_mutated_request_is_canonical_or_refused`, `control_open_round_trips_and_is_canonical`, `launch_token_round_trips`, `golden_flow_requests_pin_every_kind`, `golden_control_open`, `an_all_zero_launch_token_is_refused_everywhere`, `the_flow_id_carries_its_origin_in_the_top_bit`, `malformed_flow_requests_are_refused_with_their_cause`, `malformed_control_opens_are_refused_with_their_cause`, `launch_token_text_is_exactly_32_lowercase_hex_characters`} — ignored `pending DELIVER vsock new-step 2 (core types and wire codec)`. The kernel command-line token form (`overdrive.transport=vsock2,…,launch=<hex>`) is pinned by the `overdrive-init` `transport::tests` (`well_formed_token_parses_and_other_parameters_are_ignored`, `token_rendering_roundtrips` and siblings, ignored `pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)`; scenario P6-S11), which pass the command line as a string. Attachment cut (DG-2 row 13, DG-7): `BeaconMessage::parse` of the 3-field READY line asserts `FieldCount { expected: 3 }`, `MissingKey { field: "launch" }` and `InvalidLaunchToken`.

**P2-S2 control-message half-close codec** `@contract-shape:pure-function`
- **Outcome**: the half-close signalling messages `HalfClose` / `HalfCloseAck`
  (and every other `ControlMessage` type, each `FlowRefusal`) encode/decode
  losslessly; an unknown control type or refusal code is refused (R5-45 codec,
  HALFCLOSE-SIGNAL).
- **Contract**: `ControlMessage::decode(encode(x)) == x`; `FlowRefusal::as_u8`
  is injective and every code `1..=12` decodes; type `0`/`7`+ out of range →
  `UnknownControlType`; refusal code `0`/`13`+ → `UnknownRefusal`.
- **Oracle**: `proptest` over all 6 message types × flow ids / refusals /
  listen ports; golden-byte pins.
- **Evidence tier**: unit + proptest (layer 1).
- **Status**: authored: `overdrive-core` `vsock_wire_codec`::{`every_control_message_round_trips`, `every_accepted_control_message_is_canonical`, `every_reserved_port_in_a_listen_state_is_refused` (DG-12 reserved `ListenState` port), `golden_control_messages_pin_every_type_and_refusal_code`, `malformed_control_messages_are_refused_with_their_cause`, `kinds_and_refusals_have_distinct_labels`} — ignored `pending DELIVER vsock new-step 2 (core types and wire codec)`.

### Owner flow admission / pairing / attribution — seeded sim (host halves at the attachment cut in `overdrive-control-plane`; guest halves authored in `overdrive-init`)

Host halves are driven through the production `HostSharedGuestNetworkOwner`
built with `with_flow_io` over `SimGuestFlowIo` (DG-3) and `SimMtlsResolve`
at the `resolve` port; guest halves through `GuestFlowOwner::dispatch_next`
over `SimGuestOwnerIo` + `SimGuestClock` (DG-5); example-based (layer 3+, Mandate 9/11 — every sad path enumerated,
never PBT-generated). The oracle is a before/after **observable** snapshot (the
Rust universe-guard adaptation of Mandate 8): host side O2
`lease_observation()`, O3 `flow_observation()` (`session`, `unread`,
`guest_flows`, `host_flows`, `datagrams`), the sim's `closed_by_owner` and
`outbox`, and `FlowCloseCause`; guest side `guest_observation()` (QPROJ-5) and
`guest_socket_observation()` — never private owner fields. The guest owner's
counters have no accessor (UI-R2-6), so guest-side discard counts are not
asserted. Each scenario asserts the expected delta AND that every unrelated
observable slot is unchanged.

**P2-S3 no request is lost when the server speaks first (R5-9, inv 3)**
`@contract-shape:bounded-change`
- **Outcome**: no flow request is ever lost; the owner never installs a flow
  socket before its 32-byte request has been fully read (H-1), under a
  server-first destination (writes a banner first) and a client-first inbound.
- **Contract**: the acceptor reads the request before installing its vsock —
  `FlowRequest` fully read in userspace within
  `FLOW_REQUEST_DEADLINE` before any `PairInstall` effect (per-kind total
  order, TcpConnect step 3; inv 3 `NoInstallBeforeRead`).
- **Owner path**: `owner` accepts V_h from `SimGuestFlowIo` → `HostReadReq` →
  `HostInstall`; the sim records the install ordered after the read.
- **Oracle**: the seam's recorded operation order shows `read(flow)`
  strictly before `install(flow)`; the banner bytes reach the opener after
  `Paired` + cell route, never before. *(G-V6 row 1)*
- **Evidence tier**: seeded sim (owner seam) + conformance (`owner_flows`
  `WitnessNoPaired` family); native Tier-3 is R5-9's own lane (P6 not P2).
- **Status**: attachment cut (DG-2 rows 1-3, 7-9): source-local `overdrive-control-plane` test driving `pool.assign` → `provision` → queued 1240 / 1241 / 1243 connections with buffered records on `SimGuestFlowIo` → `activate`; asserts O3 `flow_observation()` (`session`, `unread`, `guest_flows`, `host_flows`, `datagrams`), the sim's `closed_by_owner` and `outbox`, and `FlowCloseCause`. Starting draft: `vsock_owner_flow_admission.rs` @91f9436ff.

**P2-S4 `Paired` only after peer install (G-V6 row 1, inv 3)**
`@contract-shape:bounded-change`
- **Outcome**: the opener releases its parked bytes only on a `Paired` that
  follows the acceptor's peer socket install + route; a server-first banner is
  never stranded.
- **Contract**: `Paired` is sent only after `HostInstall` + `HostStartConnect`
  + route `V_h → D` (steps 5–6); sending `Paired` in-band on the flow
  connection strands a banner (counter-ex P-23, 18/100).
- **Oracle**: seam op order `install → route → Paired`; the `Paired`
  travels on the **control session**, never the flow vsock; the opener's parked
  cell is routed only after `Paired`.
- **Evidence tier**: seeded sim + conformance (`WitnessNoPaired`).
- **Status**: attachment cut (DG-2 rows 1-3, 7-9): source-local `overdrive-control-plane` test driving `pool.assign` → `provision` → queued 1240 / 1241 / 1243 connections with buffered records on `SimGuestFlowIo` → `activate`; asserts O3 `flow_observation()` (`session`, `unread`, `guest_flows`, `host_flows`, `datagrams`), the sim's `closed_by_owner` and `outbox`, and `FlowCloseCause`. Starting draft: `vsock_owner_flow_admission.rs` @91f9436ff.

**P2-S5 admission racing quiesce / teardown / session-loss (R5-26, U-1, inv 4)**
`@contract-shape:unbounded-preservation`
- **Outcome**: no flow is registered on a check that quiescence, teardown or
  session-loss has made false; a non-blocking connect that completes after its
  flow was aborted sends no `Paired` and closes its socket.
- **Contract**: check-and-register are **one step** relative to quiesce /
  teardown / session loss (U-1); a continuation that finds the flow `Closed`
  (`HostConnectDiscard`) closes D and sends nothing (inv 4 `NoAdmitWhenClosed`).
- **Fault stimulus**: `Quiesce`, `Teardown`, `HostSessionLoss` interleaved
  between `HostCheckFlow` and `HostConnectDone`.
- **Oracle**: no new flow-table entry survives the false check; no `Paired`
  emitted; the seam socket for that flow is closed; CID-table state and
  every other flow unchanged.
- **Evidence tier**: seeded sim + conformance (`owner_flows` safety +
  `WitnessNoQuiesceAbort`). *(G-V6 rows 2/5)*
- **Status**: attachment cut (DG-2 rows 1-3, 7-9): source-local `overdrive-control-plane` test driving `pool.assign` → `provision` → queued 1240 / 1241 / 1243 connections with buffered records on `SimGuestFlowIo` → `activate`; asserts O3 `flow_observation()` (`session`, `unread`, `guest_flows`, `host_flows`, `datagrams`), the sim's `closed_by_owner` and `outbox`, and `FlowCloseCause`. Starting draft: `vsock_owner_flow_admission.rs` @91f9436ff.

**P2-S6 flow from CID without a live session, and stale-token flow (R5-16, inv 4)**
`@contract-shape:unbounded-preservation`
- **Outcome**: a flow connection from a CID with no live control session is
  closed, counted, never paired; a flow whose request carries another launch's
  token is closed, unregistered, counted `StaleLaunch`, with no control message
  on the session (ATTR-TOKEN).
- **Contract**: `NoControlSession` close (M-7) vs `StaleLaunch` close
  (ATTR-TOKEN): the launch token is compared against the CID's **live** launch
  token before any effect (ATTR-LIVE-CLAIM); a mismatch sends nothing — its
  flow id belongs to another launch's id space.
- **Oracle**: `guest_flow.connection_closed_unattributed { cause:
  NoControlSession | StaleLaunch }` emitted; zero control messages on the
  session; no flow registered; no resolution/connect/`Paired`. *(G-V6 row 5)*
- **Evidence tier**: seeded sim + Quint `owner_flows` (`CurrentGeneration`,
  `NoCrossLaunchMsg`).
- **Status**: attachment cut (DG-2 rows 1-3, 7-9): source-local `overdrive-control-plane` test driving `pool.assign` → `provision` → queued 1240 / 1241 / 1243 connections with buffered records on `SimGuestFlowIo` → `activate`; asserts O3 `flow_observation()` (`session`, `unread`, `guest_flows`, `host_flows`, `datagrams`), the sim's `closed_by_owner` and `outbox`, and `FlowCloseCause`. Starting draft: `vsock_owner_flow_admission.rs` @91f9436ff.

**P2-S7 datagram request pair mismatch (R5-17)** `@contract-shape:unbounded-preservation`
- **Outcome**: two datagram requests matching on `(CID, flow)` but differing in
  destination, or both the same kind, → `Refused(Malformed)`, both sockets
  closed, counted `DatagramRequestMismatch`.
- **Contract**: the host matches `DatagramStream` + `DatagramSeqpacket` on
  `(CID, flow)` within `FLOW_REQUEST_DEADLINE`; a mismatch is `Refused(Malformed)`
  (L-4). *(This is the owner-seam view; the association state machine is P3.)*
- **Oracle**: one `Refused(Malformed)` control message per the opener's flow,
  both seam sockets closed, `FlowCloseCause::DatagramRequestMismatch`.
- **Evidence tier**: seeded sim. *(G-V6 row 2)*
- **Status**: attachment cut (DG-2 rows 1-3, 7-9; the datagram association behaviour lands in new-step 10): source-local `overdrive-control-plane` test driving `pool.assign` → `provision` → queued 1240 / 1241 / 1243 connections with buffered records on `SimGuestFlowIo` → `activate`; asserts O3 `flow_observation()` (`session`, `unread`, `guest_flows`, `host_flows`, `datagrams`), the sim's `closed_by_owner` and `outbox`, and `FlowCloseCause`. Starting drafts: `vsock_owner_flow_admission.rs`, `vsock_owner_datagram_assoc.rs` @91f9436ff.

**P2-S8 two concurrent sessions / beacons, current token (R5-18, inv 1)**
`@contract-shape:bounded-change`
- **Outcome**: two concurrent control sessions (and two beacon sessions) from
  one CID carrying the current launch's token — exactly one is accepted
  (`SessionHeld` closes the second); a queued session/beacon of the CID's
  **previous** launch, delivered before/after/between the current launch's own,
  never takes the claim and releases nothing (ATTR-TOKEN).
- **Contract**: `ClaimSet<GuestCid>` — check-and-claim are one atomic step
  (inv 1 `OneLiveSession`); a held claim → `SessionHeld` (L-3); a stale token →
  `StaleLaunch`, no claim.
- **Oracle**: exactly one session `live` for the CID; the second close counted
  `SessionHeld`; the stale one counted `StaleLaunch`; the claim holder is the
  current launch's connection; `heldByOther`/`crossMsg` ghosts stay false.
  *(G-V3 row 5)*
- **Evidence tier**: seeded sim + Quint `owner_flows`
  (`NoCrossLaunchSessionHeld`, `OneLiveSession`).
- **Status**: attachment cut (DG-2 rows 1-3, 7-9): source-local `overdrive-control-plane` test driving `pool.assign` → `provision` → queued 1240 / 1241 / 1243 connections with buffered records on `SimGuestFlowIo` → `activate`; asserts O3 `flow_observation()` (`session`, `unread`, `guest_flows`, `host_flows`, `datagrams`), the sim's `closed_by_owner` and `outbox`, and `FlowCloseCause`. Starting draft: `vsock_owner_flow_admission.rs` @91f9436ff. The two-beacon cell is P2-S14's attachment-cut half (DG-2 rows 13, 15).

**P2-S9 stale-launch attribution end-to-end (R5-41, inv 4+)**
`@contract-shape:unbounded-preservation`
- **Outcome**: a dead launch's queued connections (control session,
  beacon+READY, every flow kind) accepted under the CID's **next** holder are
  closed unattributed (`StaleLaunch` / `NoLiveLaunch`): no claim, no READY
  release, no flow beyond its check, no control message, no resolution / connect
  / `Paired`; the new launch's own session / flows / READY are accepted; a
  4-tuple a stale entry holds is refused until that entry is closed, then
  succeeds on retry.
- **Contract**: attribution is peer CID **and** the CID's live launch token
  (ATTR-TOKEN); a queued dead-launch connection delivers its buffered first
  record but fails the live-token compare (P-37). A queued connection with no
  buffered bytes fails its first read.
- **ATTR-LIVE-CLAIM regression (round-9 counterexample)**: a dead launch's
  control (and beacon-with-buffered-READY) connection accepted while the CID is
  still Provisioned (launch live) and read **only after** `driver.stop` /
  teardown — which found no session to close — is closed at its read, takes no
  claim; after `pool.release` the next lease's session takes the claim and
  activation runs on that live session; no `Abort` or other message crosses
  launches.
- **Oracle**: for every stale connection, zero claims, zero control messages,
  zero `Paired`; `StaleLaunch`/`NoLiveLaunch` counted; the new launch's claim
  is taken and its activation proceeds. *(G-V2/G-V3/G-V6 row 5)*
- **Evidence tier**: seeded sim (production owner + VM driver, stale queued
  connections scripted at the seam) + Quint `owner_flows`
  (`WitnessNoReadAfterTeardown`, `CurrentGeneration`) + the codec proptest
  (P2-S1). Native kill-and-reuse through `serve`+`deploy` is R5-41's Tier-3
  lane (P6).
- **Status**: attachment cut (DG-2 rows 1-3, 7-9): source-local `overdrive-control-plane` test driving `pool.assign` → `provision` → queued 1240 / 1241 / 1243 connections with buffered records on `SimGuestFlowIo` → `activate`; asserts O3 `flow_observation()` (`session`, `unread`, `guest_flows`, `host_flows`, `datagrams`), the sim's `closed_by_owner` and `outbox`, and `FlowCloseCause`. Starting draft: `vsock_owner_flow_admission.rs` @91f9436ff.

**P2-S10 a connection the host aborted before the guest accepted it never reaches the application (R5-28, R5-31, U-5, K-A4)**
`@contract-shape:unbounded-preservation`
- **Outcome**: a `Paired` from the guest for a host-aborted `TcpAccept` is
  discarded + counted, no reply; an `Abort` the guest receives for a flow it
  does not yet hold is discarded + counted, no reply; the guest half ends
  through the host's close of V_h — the request read succeeds, then EOF,
  `POLLRDHUP`, first write `EPIPE` — and no byte of the aborted flow reaches the
  application (H-1).
- **Contract**: the `TcpAccept` guest vsock V_g ends on the host's close —
  U-5 discard (`guest_flow.control_discarded`); K-A4 — the host's
  close of V_h ends G's half; no byte on V_h after the request before `Paired`
  (P-38).
- **Oracle**: the `Paired`/`Abort` for the closed/unknown flow is discarded and
  counted; the guest owner ends the flow (no C, or C reset); zero application
  bytes delivered. The seam models V_g's EOF-after-request. *(G-V6 row 5; the
  production sockmap V_g→C route is P6's step-5 Tier-3)*
- **Evidence tier**: seeded sim + Quint `owner_flows`
  (`WitnessNoAcceptPaired`, `WitnessNoQueueReset`). Native V-24 shape is P6.
- **Status**: guest half authored: `overdrive-init` `guest_flow::tests::tcp_accept`::{`tcp_accept_with_the_guests_token_pairs_in_the_pinned_order`, `tcp_accept_with_another_launchs_token_closes_v_g_silently`, `refused_application_connect_answers_application_not_listening`, `a_v_g_the_host_closed_after_the_request_ends_with_no_c_and_no_message`, `a_queued_v_g_closed_by_the_host_never_reaches_the_guest_and_its_abort_is_discarded`, `control_messages_for_unknown_or_closed_flows_are_discarded_without_reply`, `abort_of_a_routed_flow_resets_the_application_leg`, `a_connect_completing_after_session_loss_sends_nothing_and_closes_c`} — ignored `pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)`. The discard *count* is not asserted (guest counters have no accessor, UI-R2-6). Host half: attachment cut (DG-2 rows 1-3, 7-9), its drain-dependent interleavings (R5-45 (a)) pinned to the UI-R2-5 (d) decision — the body cannot be finalised until DESIGN pins how an owner learns a half-closed direction has drained (the guest half above is affected the same way at activation): source-local `overdrive-control-plane` test driving `pool.assign` → `provision` → queued 1240 / 1241 / 1243 connections with buffered records on `SimGuestFlowIo` → `activate`; asserts O3 `flow_observation()` (`session`, `unread`, `guest_flows`, `host_flows`, `datagrams`), the sim's `closed_by_owner` and `outbox`, and `FlowCloseCause`. Starting draft: `vsock_owner_flow_admission.rs` @91f9436ff.

**P2-S11 non-mesh connect has no owner deadline (R5-20, D15-R3)**
`@contract-shape:unbounded-preservation`
- **Outcome**: a TcpConnect to an unresponsive non-mesh destination completes
  with no `PairingTimeout`; it holds one quota slot + one guest cell until the
  host kernel's SYN retries give up, then the application sees a reset; a
  refused destination resets at once; other flows and `ListenState` of the VM
  proceed throughout.
- **Contract**: the TcpConnect opener holds no pairing deadline (D15-R3 = (b));
  the host's connect is bounded only by `net.ipv4.tcp_syn_retries`; refused /
  ICMP-unreachable → `Refused(DestinationUnreachable)` at once; no answer →
  `Refused(DestinationUnreachable)` when the kernel gives up.
- **Fault stimulus**: `SimMtlsResolve` returns `NonMesh`; the sim connect
  outcome is scripted (pending-forever, refused-at-once) with the host-SYN-retry
  duration as a sim parameter.
- **Oracle**: no `PairingTimeout` `FlowCloseCause`; exactly one quota slot + one
  cell held while pending; at give-up the flow closes
  `DestinationUnreachable`; a concurrent flow and a `ListenState` of the same VM
  are serviced while the connect is pending; 4,096 such connects exhaust only
  that VM's quota (`Capacity` / `guest_park_exhausted`), affecting no other VM.
- **Evidence tier**: seeded sim (scripted connect outcome). Tier-3 V-9 density
  is P6. *(G-V6)*
- **Status**: attachment cut (DG-2 rows 1-3, 7-9): source-local `overdrive-control-plane` test driving `pool.assign` → `provision` → queued 1240 / 1241 / 1243 connections with buffered records on `SimGuestFlowIo` → `activate`; asserts O3 `flow_observation()` (`session`, `unread`, `guest_flows`, `host_flows`, `datagrams`), the sim's `closed_by_owner` and `outbox`, and `FlowCloseCause`. Starting draft: `vsock_owner_flow_admission.rs` @91f9436ff. The non-mesh decision is scripted through `SimMtlsResolve` (`NonMesh`) and the sim's connect outcome; the owner `NonMesh` policy cells are P5's.

**P2-S12 a far end sees end-of-stream only for a direction its peer closed, never because an owner failed (R5-45, inv 13, HALFCLOSE-SIGNAL)**
`@contract-shape:bounded-change`
- **Outcome**: each owner shuts its flow vsock for writing only after
  `HalfCloseAck`; a flow vsock that ends with no acknowledged `HalfClose` (while
  the session is live) aborts the flow at once (`UnannouncedVsockEnd`) and every
  far end on that side is reset, never EOF; an announcing side whose peer's
  session is lost, or which receives `Abort`, before the ack aborts; a
  write-then-half-close before `Paired` is carried at `Paired`, a reset before
  `Paired` aborts; `HalfClose`/`HalfCloseAck` for an unknown/closed flow are
  discarded + counted (U-5).
- **Contract**: the half-close owner seam — inv 13 `NoEofOnPeerLoss` + `FinOnlyAfterAck` — a far end
  observes EOF for a direction only if that direction's application/remote
  half-closed it, never because an owner died; no owner shuts a flow vsock
  before the peer acknowledged that direction's `HalfClose`.
- **Oracle**: ordered seam ops — `drained → HalfClose → (ack) → shutdown_write`;
  on unannounced vsock end, `abort` with `FlowCloseCause::UnannouncedVsockEnd`
  and linger-reset of outer legs; discards counted. Both owners (guest + host).
- **Evidence tier**: seeded sim (owner seam) + Quint `owner_flows` (`flows_half`
  — `NoEofOnPeerLoss`, `FinOnlyAfterAck`, `EarlyEofCarried`,
  `WitnessNoCleanEof`, `WitnessNoEarlyEof`). Tier-3 cell is P6 inside R5-10.
- **Status**: guest half authored: `overdrive-init` `guest_flow::tests::half_close`::{`the_flow_vsock_is_shut_for_writing_only_after_half_close_ack`, `a_peer_half_close_is_acked_and_the_outer_leg_shut_after_the_vsock_eof`, `an_unannounced_vsock_end_aborts_and_resets_the_application`, `a_drain_that_misses_the_deadline_aborts_the_flow`, `a_clean_close_clears_linger_before_closing`} and `guest_flow::tests::tcp_connect`::{`an_application_reset_before_paired_aborts_and_sends_abort`, `an_early_application_eof_is_carried_out_at_paired`} — ignored `pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)`. The inventory groups the remaining `tcp_connect` tests (`a_captured_connect_opens_v_g_writes_the_request_once_and_routes_on_paired`, `a_capture_without_a_parking_cell_is_reset_locally`, `a_refused_tcp_connect_resets_the_application`) and `guest_flow::tests::session` (session loss and host-owner restart; also P6-S11) with these guest halves, same ignore reason; `guest_owner_fatal_renders_its_pinned_display_texts` runs in the default lane. The drain timeout sends `Abort` (UI-R2-12, closed: `owner_flows.qnt` `HcFlowError`, QPROJ, DG-15). Host half: attachment cut (DG-2 rows 1-3, 7-9): source-local `overdrive-control-plane` test driving `pool.assign` → `provision` → queued 1240 / 1241 / 1243 connections with buffered records on `SimGuestFlowIo` → `activate`; asserts O3 `flow_observation()` (`session`, `unread`, `guest_flows`, `host_flows`, `datagrams`), the sim's `closed_by_owner` and `outbox`, and `FlowCloseCause`. Starting draft: `vsock_owner_flow_admission.rs` @91f9436ff.

**P2-S13 G-V6 / G-V3 boundary rows** `@contract-shape:unbounded-preservation`
- **Outcome**: (row 2) a refusal path answers `Refused(code)` / `PairingTimeout`
  (datagram/TcpAccept) / `DrainTimeout`, flow closed; (row 3) the allocation
  stays Running/Stable and every unrelated flow is unchanged; (row 4) a `Paired`
  that lands after the CID moved to Retiring is aborted, never resurrecting the
  flow (inv 9 `ClosedIsTerminal`); for G-V3, `activate` after `teardown` →
  `LeaseRetiring`.
- **Contract**: per-flow failures never delay/advance/revoke allocation
  `Running`/READY/intercept-live/Service `Stable` (taxonomy "nobody in the
  allocation lifecycle branches").
- **Oracle**: the refusal/abort is flow-local; allocation state slot unchanged;
  the late `Paired` discarded (`resurrected` ghost stays false).
- **Evidence tier**: seeded sim + Quint `owner_flows`
  (`WitnessNoLateCompletion`, `ClosedIsTerminal`).
- **Status**: attachment cut (DG-2 rows 1-3, 7-9): source-local `overdrive-control-plane` test driving `pool.assign` → `provision` → queued 1240 / 1241 / 1243 connections with buffered records on `SimGuestFlowIo` → `activate`; asserts O3 `flow_observation()` (`session`, `unread`, `guest_flows`, `host_flows`, `datagrams`), the sim's `closed_by_owner` and `outbox`, and `FlowCloseCause`. Starting draft: `vsock_owner_flow_admission.rs` @91f9436ff.

### Beacon attribution — seeded sim (`overdrive-worker`: `vsock_beacon_intake` now, attribution cells at the attachment cut)

Driven through the production `BeaconIntake` / `VmDriver` over the `BeaconIo`
port with `SimBeaconIo` (DG-4, over `SimVmm`), `SimGuestCidClaim` supplying
CIDs and `SimEntropy` the launch tokens.

**P2-S14 beacon keyed by peer-CID + live launch token (D17, R5-18, ATTR-LIVE-CLAIM)**
`@contract-shape:bounded-change`
- **Outcome**: READY is attributed by peer CID **and** the launch token its line
  carries, compared against the CID's live launch (one per-CID step); a beacon
  connection the CID's previous launch left queued (buffered READY) is closed
  unattributed (`StaleLaunch`) and releases nothing; a stopped launch's
  connection read after `driver.stop` is `NoLiveLaunch`; a second concurrent
  connection of the live launch is `SessionHeld`.
- **Contract**: the beacon `ClaimSet<GuestCid>` claim is taken in the same step
  as the live-launch + token compare after the first line is read; the
  accept-time live-launch check is only an early close (ATTR-LIVE-CLAIM); the
  line gains the `launch` field (`BeaconMessage::Ready { pid, port, launch }`).
- **Oracle**: the live launch's READY takes the claim exactly once; every stale
  / stopped / held connection is closed with
  `vm_beacon.connection_closed_unattributed { cause }` and releases no READY;
  the claim holder and `readyCross`/`bHeldByOther` ghosts assert the invariant.
  *(G-V2 row 5)*
- **Evidence tier**: seeded sim (VM driver beacon seam) + Quint `owner_flows`
  (`flows_beacon` — `BeaconCurrentLaunch`, `BeaconNoCrossLaunchReady`,
  `BeaconNoCrossLaunchHeld`, `ReadyReleased`, `WitnessNoReady`,
  `WitnessNoStaleBeaconRead`) + the READY-line codec (P2-S1 launch-field
  roundtrip). Native kill-and-reuse is R5-41 Tier-3 (P6).
- **Status**: additive half authored: `overdrive-worker` `vsock_beacon_intake`::{`binding_the_intake_binds_the_beacon_listener`, `a_bind_failure_is_typed_and_binds_nothing`, `a_fresh_driver_reports_an_empty_beacon_observation`} — ignored `pending DELIVER vsock attachment cut (new-step 6, beacon relocation, D17)`; `the_bind_refusal_renders_its_pinned_message` runs in the default lane. The seam's own semantics are the `overdrive-sim` `adapters::beacon_io` unit tests: `bind_is_idempotent`, `a_scripted_bind_failure_binds_nothing_and_is_one_shot`, `accept_before_bind_is_refused`, `a_handle_another_adapter_issued_is_unreachable` in the default lane; `a_guest_cannot_connect_before_the_listener_is_bound`, `a_queued_ready_is_observed_then_accepted_and_read`, `a_queued_connection_with_nothing_buffered_or_no_valid_token_projects_none` and the other connection-lifecycle tests through `stepped_mode_holds_every_delivery_until_granted`, ignored `pending DELIVER vsock new-step 2 (core types and wire codec)`. Attachment cut (DG-2 rows 13, 15): `VmDriver::new(.., BeaconIntake::bind(SimBeaconIo::stepped(vmm)))` with READY lines carrying `launch=`; asserts O4 `beacon_observation()` (`live`, `claims`, `unread`) and the sim's `closed_by_driver`: the live READY claims; a stale queued READY (P-37) closes; a read after `stop` takes nothing; a second connection is closed `SessionHeld`; no buffered bytes → EOF close; a non-READY first line closes. Starting draft: `vsock_beacon_attribution.rs` @91f9436ff. `SimBeaconIo`'s `Option<LaunchToken>` projection reads READY `launch=` (UI-R2-15); the production `VsockBeaconIo` constructor is unpinned (UI-R2-2) and is exercised at P6-S13.

### Conformance — `owner_flows`

> **QPROJ status (2026-10-10).** Placement, `State` projection and action map
> are pinned by QPROJ-1 to QPROJ-8 ([`test-scenarios.md` § Quint conformance drivers](test-scenarios.md#quint-conformance-drivers-qproj-1-to-qproj-8-approved-2026-10-09)).
> Drivers are in-crate modules in their owner's crate. Authored now:
> `crates/overdrive-init/src/guest_flow/quint_owner_flows.rs` (`flows_ok` guest
> side) and `quint_flows_half.rs` (`flows_half_guest`). At the attachment cut:
> the `flows_ok` host side and `flows_half_host` in `overdrive-control-plane`
> (over `SimGuestFlowIo`), and `flows_beacon` in `overdrive-worker` (over
> `SimBeaconIo`). There is no `tests/conformance/tests/vsock_owner_flows_conformance.rs`,
> no shared `quint_driver` base and no `ConformanceNode`.

**P2-S15 `owner_flows` conformance** `@contract-shape:bounded-change`
- **Outcome**: the production guest-flow owner + VM-driver beacon conform to the
  Quint `owner_flows` protocol along generated traces.
- **Contract / invariants defended**: 1 (one live session per CID), 3 (no
  install before read), 4 (no admit/`Paired` while quiesced / no live session /
  not Active), 4+ (every live session & flow belongs to the current lease; no
  claim held by a non-live launch), 9 (U-5 — a closed flow never pairs again),
  13 (HALFCLOSE-SIGNAL — EOF only for an announced, acknowledged direction).
- **Oracle**: quint-connect replays each `flows_ok` / `flows_beacon` /
  `flows_half` trace step through the production owner (Sim at driven ports),
  projecting `State` from the owner's **observable** surface — guest side
  `guest_observation()` (QPROJ-5) and `SimGuestOwnerIo`'s
  `guest_socket_observation()` (S); host side O2 `lease_observation()`, O3
  `flow_observation()` and `SimGuestFlowIo`'s `socket_observation()`;
  `flows_beacon` O4 `beacon_observation()` and `SimBeaconIo`'s
  `beacon_socket_observation()` — and compares state after every step. Every
  driver runs `#[quint_run]` randomized traces with the seed printed on
  failure. There is no `#[quint_test]`: the specs define no Quint `run`
  (UI-R2-1). The `checks.toml` entries (safety TLC/Apalache, the witnesses,
  the attribution hazards `peer-cid-only`, `claim-before-token-held`,
  `no-claim-recheck-*`, the beacon hazards, the half-close hazards
  `rdhup-is-halfclose`, `no-ack*`, `ack-waits`, `early-eof-aborts`) are
  invariant and witness checks of the model run by `cargo xtask quint check`,
  not traces a driver pins.
  The spec is referenced **in place** (`specs/quint/guest-flow-owner/owner_flows.qnt`),
  never copied; `checks.toml` is not edited (it is hashed into the evidence).
- **Evidence tier**: conformance, Lima, `integration-tests` (spawns the `quint`
  CLI and drives the owner through its seam sim).
- **Status**: guest side authored: `overdrive-init`
  `guest_flow::quint_owner_flows`::`quint_owner_flows_ok_guest` (`flows_ok`) and
  `guest_flow::quint_flows_half`::`quint_owner_flows_half_guest`
  (`flows_half_guest`), both `#[quint_run]`, ignored `pending DELIVER vsock
  new-step 5 (guest adaptation in overdrive-init); model fidelity open,
  upstream UI-R2-5`. Authored; model fidelity open (UI-R2-5) — a correct
  implementation may fail them until the model decision. The open items:
  one guest clock for all timers, the MAXFID model bound, `HcCleanClose` as a
  separate model step where production clears linger and closes in the
  dispatch that completes the second half-close
  (`a_clean_close_clears_linger_before_closing`), how an owner learns a
  half-closed direction has drained (unpinned on both sides), an error on an
  unrouted V_g unobservable, and some `flows_ok` orders not producible. Attachment cut: `flows_ok` host side and `flows_half_host` (DG-2
  rows 2, 3, 7, 8, 9; `overdrive-control-plane`, QPROJ host-side action maps;
  asserts O2, O3, S `socket_observation()`), and `flows_beacon` (rows 13, 15;
  `overdrive-worker`; `VmStart` `VmDriver::start`, `VmStop` `SimVmm` exit,
  `DriverStop` `VmDriver::stop`, beacon connections over `SimBeaconIo`;
  asserts O4 `beacon_observation`, S `beacon_socket_observation`).

## Boundary-scenario placement (feature-delta §"Required boundary scenarios")

| Gate | Row 1 available | Row 2 typed failure | Row 3 unrelated unchanged | Row 4 late success | Row 5 disconnect/reconnect | Row 6 feature disabled |
|---|---|---|---|---|---|---|
| **G-V6** | P2-S3/S4 (`Paired`; early bytes flow) | P2-S13 (`Refused`/`PairingTimeout`/`DrainTimeout`) | P2-S13 (Running/Stable) | P2-S13 (`Paired` after Retiring → aborted) | P2-S10/S12 (opener disconnect; reset→abort; session loss; stale queued request) | P6-S12b (attachment cut) |
| **G-V3** | P4 (`activate` → listeners); P2-S8 (session precondition) | P2-S13 (`activate` after `teardown` → `LeaseRetiring`) | P2-S13 (READY unchanged) | P2-S13 (`activate` after `teardown`) | P2-S8/S9 (stale queued session never counts live) | P6-S12b (attachment cut) |

## Quint `owner_flows` module → check → scenario map (P2-S15)

The families below are `checks.toml` invariant / witness / hazard checks of the
model (`cargo xtask quint check`); the drivers replay `#[quint_run]` traces of
the named instance and do not pin these entries (UI-R2-1).

| `checks.toml` run family | Instance | Defends | P2 scenario |
|---|---|---|---|
| `owner-flows-safety-{tlc,apalache,apalache-deep}` | `flows_ok` | inv 1/3/4/4+/9 | S3–S10, S13 |
| `owner-flows-witness-{paired,reuse,quiesce-abort,late-completion,accept-paired,phantom-connect,queue-reset,stale-accept,read-after-teardown}` | `flows_ok` | reachability of each good outcome | S3/S4/S5/S9/S10/S13 |
| `owner-flows-current-launch`, `-no-cross-launch-message`, `-no-cross-launch-session-held` | `flows_ok` | 4+ / ATTR-TOKEN / ATTR-LIVE-CLAIM | S6/S8/S9 |
| `owner-flows-hazard-{split-claim,install-early,admit-split,paired-no-recheck,no-abort-on-loss,teardown-skips-flows,late-paired-resurrects,peer-cid-only*,claim-before-token-held,no-claim-recheck-*}` | hazards | teeth for each rule | S3/S5/S6/S8/S9/S13 |
| `owner-flows-beacon-*` + `-hazard-beacon-*` | `flows_beacon` | D17 / ATTR-LIVE-CLAIM beacon | S14 |
| `owner-flows-half-*` + `-hazard-{rdhup-is-halfclose,no-ack*,ack-waits,early-eof-aborts}` | `flows_half` | inv 13 / HALFCLOSE-SIGNAL | S12 |

## R5-* obligation → scenario

| R5 | Scenario |
|---|---|
| R5-9 acceptor reads before install | P2-S3 |
| R5-16 no live session / stale token flow | P2-S6 |
| R5-17 datagram request mismatch | P2-S7 |
| R5-18 two concurrent sessions/beacons, current token | P2-S8 (+ P2-S14 beacon) |
| R5-20 non-mesh no owner deadline | P2-S11 |
| R5-26 admission racing quiesce/teardown/loss | P2-S5 |
| R5-28 `Paired`/`Abort` for closed/unheld `TcpAccept` discarded; K-A4 | P2-S10 |
| R5-31 host aborts `TcpAccept` before guest accepts V_g | P2-S10 (owner-seam half); Tier-3 is P6 |
| R5-41 launch attribution end-to-end + ATTR-LIVE-CLAIM regression | P2-S9 (+ P2-S1 codec, P2-S14 beacon) |
| R5-45 half-close across the VM boundary | P2-S12 (codec P2-S2); Tier-3 cell is P6 inside R5-10 |

## Lanes & PBT mode (Mandate 9)

- **Layer 1 (pure codec)** — `proptest` full: roundtrip ∀ valid value, reject
  ∀ malformed; `@example`-pinned golden bytes. `vsock_wire_codec.rs`.
- **Layer 2–3 (seeded sim through the production owner seam)** — **example-based
  only**; every sad path enumerated (S5–S10, S12, S13). No `proptest` generation
  at this layer (Mandate 9/11). Before/after observable snapshot oracle (the
  Rust universe-guard adaptation of Mandate 8): assert the delta + unrelated
  slots unchanged.
- **Conformance** — quint-connect generated traces; the spec is the oracle, not
  a hand-written property.

No Tier-3 owned here (the crash-matrix Tier-3 is P6). ≥40% sad paths:
S5, S6, S7, S9, S10, S12, S13 (7 of 15 scenarios) + the codec-reject arms of
S1/S2 + the hazard families of S15 — well above 40%.

## AT-completeness audit (nw-at-completeness-check, 15-item)

| Item | Status | Where |
|---|---|---|
| C1a empty/min input | PASS | codec all-zero token / empty listen-port fields (S1); no-session flow (S6) |
| C1b partition boundary | PASS | `FlowId` seq `2^31-1`/`2^31` boundary; refusal codes `1`/`12`/`13` (S1/S2) |
| C2a state machine documented | PASS | owner `Requested→Paired→HalfClosing→HalfClosed→Closed` + session `Provisioned/Active/Retiring` in each module docstring |
| C2b illegal-event-from-state | PASS | `Paired` after Retiring (S13); flow from CID without session (S6); `activate` after teardown (S13) |
| C3 count 0/1/N | PASS | 0 sessions (S6), 1 (S8), N concurrent (S8/S9); 0/1/4096 connects (S11) |
| C4a apply-twice idempotency | PASS | second concurrent session → `SessionHeld` (S8); duplicate `Paired` discarded (S10/S13) |
| C4b inverse without prerequisite | PASS | `HalfClose`/`Abort`/`Paired` for a flow never opened (S10/S12 U-5) |
| C5a mode-flag combinations | specified; owed to the cut | mesh vs non-mesh resolution (S11 + P5/P4 for VIP); live vs stale token × session/beacon/flow (S6/S8/S9/S14) |
| C5b flag orthogonality | specified; owed to the cut | per-flow refusal never changes allocation state (S13) |
| C6a malformed value per channel | PASS | bad magic/version/kind/origin/reserved/type/refusal (S1/S2) |
| C6b each declared error triggered | PASS | every `FlowRefusal` (S2) + `FlowCloseCause` cause enumerated across S5–S13 |
| C6c closed error set | specified; owed to the cut | `decode` accepts only canonical encodings (S1); no cause outside `FlowCloseCause` (S5–S13) |
| C7a degraded resource | PASS | quota/cell exhaustion (S11 `Capacity`) |
| C7b interruption mid-op | PASS | quiesce/teardown/session-loss mid-admission (S5); owner death mid-flow seam (S12) |
| C7c concurrent actors | PASS | two concurrent sessions/beacons (S8); both applications half-close concurrently (S12) |

**Verdict: specification 15/15.** Over what is authored now, the host-owner
evidence for C5b and C6c (S5–S13) and the feature-level C5a / C5b cell
(`AllocationSpec.transport` `None` vs `Some`, P6-S12b) are attachment-cut
bodies, as the feature-delta records. Zero `SPECIFICATION_AMBIGUITY` (C2/C5/C6 all
resolved upstream: the state machine in D18/D18a, the mode flags in
ATTR-TOKEN/ATTR-LIVE-CLAIM/D9/D14, the error contract in the typed taxonomy §).

## RED notes

Every authored body compiles against the design-pinned interface contracts,
is registered in its crate, and carries the ignore reason given in its
scenario's **Status** line; plain-data GUARD tests (`Display` texts, label
enums) run in the default lane. The fail-for-the-right-reason run classified
every authored ignored test with 0 BROKEN (`red-classification.md`).

- `vsock_wire_codec.rs`: `overdrive_core::guest_transport::{wire::*,
  LaunchToken, GuestCid, FlowId, FlowOrigin, FlowKind, FlowRefusal}` +
  `overdrive_core::net::ListenPort` (DG-11).
- `overdrive-init` `guest_flow::tests::*` and drivers: `GuestFlowOwner`,
  `GuestOwnerIo`, `guest_flow::sim::{SimGuestOwnerIo, SimGuestClock}` (DG-5,
  QPROJ-5, QPROJ-7); not called by `main` until the attachment cut (row 16).
- `vsock_beacon_intake.rs`: `BeaconIntake`, `BeaconIntakeError`,
  `VmDriver::beacon_observation`, `SimBeaconIo` (DG-4, QPROJ-4).
- Host-side bodies and drivers: written at the attachment cut against
  `HostSharedGuestNetworkOwner::new(ports)` / `with_flow_io`,
  `GuestAddressPool::new` / `assign`, `ForwardingActivation`,
  `VmDriver::new(.., beacon)` and `BeaconMessage::Ready { launch }` (DG-2 rows
  1-3, 7-9, 13, 15).

Nothing is blocked on a validation item (V-22 done P-37; V-24 done P-38; the
ATTR-LIVE-CLAIM and HALFCLOSE-SIGNAL decisions ARE the design and the Quint
round-9 re-run is recorded).

## Testability gaps (closed)

1. **Owner flow-admission seam** — closed by DG-3: the `GuestFlowIo` seam,
   `SimGuestFlowIo` / `SimGuestNode`, `HostSharedGuestNetworkOwner::with_flow_io`,
   and the observation accessors QPROJ-2 / QPROJ-3.
2. **`VmDriver` beacon accept seam** — closed by DG-4: the core `BeaconIo` port,
   `SimBeaconIo`, `BeaconIntake`, and `VmDriver::beacon_observation` (QPROJ-4).
3. **`SimGuestFlowForwarder` with an ordered op log** — superseded by DG-3:
   `SimGuestFlowIo` records the effect order (`steering_effects`,
   `socket_observation`).
4. **`tests/conformance/src/vsock_harness.rs` + `quint_driver.rs`** — not
   created: QPROJ-1 places each driver in its owner's crate as an in-crate
   module over the seam sim.
5. **Wire-type module path** — settled: the types live in
   `overdrive_core::guest_transport::wire` (authored).
6. **READY `launch=` error shape** (UI-BEACON-1) — closed by DG-7
   (`BeaconParseError::InvalidLaunchToken`); the parse cells are authored at
   the attachment cut (row 13).

Open round-2 items that touch P2: UI-R2-1 (no `#[quint_test]`, HIGH),
UI-R2-2 (`VsockBeaconIo` constructor, before the cut), UI-R2-5 (guest driver
model fidelity, HIGH; includes the drain-complete decision), UI-R2-6 (guest
counters have no accessor), UI-R2-7 (`FlowCloseCause::as_str` strings
unpinned; operator-visible), UI-R2-15 (`SimBeaconIo` token projection, note).
Closed: UI-R2-12 (the drain deadline sends `Abort`; `owner_flows.qnt`,
QPROJ, DG-15).
