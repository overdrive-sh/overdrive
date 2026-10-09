# P2 — Guest-flow owner: control session, flow admission, pairing, launch attribution

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

| File | Lane | Layer |
|---|---|---|
| `crates/overdrive-core/tests/acceptance/vsock_wire_codec.rs` | unit + `proptest` | 1 (pure function) |
| `crates/overdrive-control-plane/tests/acceptance/vsock_owner_flow_admission.rs` | seeded `overdrive-sim` through the production owner's private seam | 2–3 (in-memory acceptance, example-based) |
| `crates/overdrive-worker/tests/acceptance/vsock_beacon_attribution.rs` | seeded `overdrive-sim` through the production `VmDriver` beacon seam | 2–3 (example-based) |
| `tests/conformance/tests/vsock_owner_flows_conformance.rs` | quint-connect `owner_flows` driver | conformance (Lima, `integration-tests`) |

The wire codec is **the one codec home** for the flow request, control message,
control open and `LaunchToken` (the datagram-frame codec is P3's
`vsock_datagram_frame.rs`; the beacon-line codec stays in
`overdrive_core::vm::beacon`, exercised here only for the `launch` field).

## Shared dependencies (NOT edited here — reported as gaps)

- `overdrive-sim/src/adapters/guest_flow.rs` — `SimGuestFlowForwarder` (shared,
  first of P2/P3/P6). The seeded-sim admission tests install/route/abort/drain
  through it.
- `overdrive-sim` `SimSharedGuestNetworkOwner` reshape, `SimMtlsResolve`,
  `SimGuestCidClaim` (P1-owned), `SimEntropy` — the owner's driven ports.
- The production owner's **private flow-admission test seam** (DISTILL-shaped
  test-support surface, CLAUDE.md §"Implement to the design" — test-support is
  DISTILL's to shape) that delivers an accepted guest-opened connection with a
  peer CID and a buffered first record, and exposes the owner's observable
  session/flow surface for the before/after snapshot oracle. Created in DELIVER
  with the owner step.
- The production `VmDriver` beacon **accept seam** (same discipline) that
  delivers an accepted beacon connection with a peer CID and a buffered first
  line.
- `tests/conformance/src/vsock_harness.rs` (`boot_vsock_node` + `ConformanceNode`
  observable surface) and `tests/conformance/src/quint_driver.rs` (quint-connect
  wiring base) — shared, first-needer creates (§2, §4 of the shared doc).

Each module file is registered (`mod <file>;` in the crate's inline
`tests/acceptance.rs` / standalone conformance binary) **by the author when the
compiling body co-arrives with the DELIVER production scaffold it calls**
(shared doc §6). The planner left the committed entrypoints untouched; these
bodies are therefore authored now and **not yet registered** — they are dangling
until the DELIVER roadmap-reconciliation step lands the production scaffolds.

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

### Owner flow admission / pairing / attribution — seeded sim (`vsock_owner_flow_admission.rs`)

Driven through the production `SharedGuestNetworkOwner` private seam with
`SimGuestFlowForwarder` at the forwarder port and `SimMtlsResolve` at
resolution; example-based (layer 3+, Mandate 9/11 — every sad path enumerated,
never PBT-generated). The oracle is a before/after **observable** snapshot (the
Rust universe-guard adaptation of Mandate 8): owner CID-table state, the
`ClaimSet` claim holder, control messages emitted per direction, the
`guest_flow.*` / `vm_beacon.*` counters, and `FlowCloseCause` — never private
owner fields. Each scenario asserts the expected delta AND that every unrelated
observable slot is unchanged.

**P2-S3 acceptor reads request before installing its vsock (R5-9, inv 3)**
`@contract-shape:bounded-change`
- **Outcome**: no flow request is ever lost; the owner never installs a flow
  socket before its 32-byte request has been fully read (H-1), under a
  server-first destination (writes a banner first) and a client-first inbound.
- **Contract**: `FlowRequest` fully read in userspace within
  `FLOW_REQUEST_DEADLINE` before any `PairInstall` effect (per-kind total
  order, TcpConnect step 3; inv 3 `NoInstallBeforeRead`).
- **Owner path**: `owner` accepts V_h via the private seam → `HostReadReq` →
  `HostInstall`; `SimGuestFlowForwarder` records the install ordered after the
  read.
- **Oracle**: the forwarder's recorded operation order shows `read(flow)`
  strictly before `install(flow)`; the banner bytes reach the opener after
  `Paired` + cell route, never before. *(G-V6 row 1)*
- **Evidence tier**: seeded sim (owner seam) + conformance (`owner_flows`
  `WitnessNoPaired` family); native Tier-3 is R5-9's own lane (P6 not P2).

**P2-S4 `Paired` only after peer install (G-V6 row 1, inv 3)**
`@contract-shape:bounded-change`
- **Outcome**: the opener releases its parked bytes only on a `Paired` that
  follows the acceptor's peer socket install + route; a server-first banner is
  never stranded.
- **Contract**: `Paired` is sent only after `HostInstall` + `HostStartConnect`
  + route `V_h → D` (steps 5–6); sending `Paired` in-band on the flow
  connection strands a banner (counter-ex P-23, 18/100).
- **Oracle**: forwarder op order `install → route → Paired`; the `Paired`
  travels on the **control session**, never the flow vsock; the opener's parked
  cell is routed only after `Paired`.
- **Evidence tier**: seeded sim + conformance (`WitnessNoPaired`).

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
  emitted; the forwarder's socket for that flow is closed; CID-table state and
  every other flow unchanged.
- **Evidence tier**: seeded sim + conformance (`owner_flows` safety +
  `WitnessNoQuiesceAbort`). *(G-V6 rows 2/5)*

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

**P2-S7 datagram request pair mismatch (R5-17)** `@contract-shape:unbounded-preservation`
- **Outcome**: two datagram requests matching on `(CID, flow)` but differing in
  destination, or both the same kind, → `Refused(Malformed)`, both sockets
  closed, counted `DatagramRequestMismatch`.
- **Contract**: the host matches `DatagramStream` + `DatagramSeqpacket` on
  `(CID, flow)` within `FLOW_REQUEST_DEADLINE`; a mismatch is `Refused(Malformed)`
  (L-4). *(This is the owner-seam view; the association state machine is P3.)*
- **Oracle**: one `Refused(Malformed)` control message per the opener's flow,
  both forwarder sockets closed, `FlowCloseCause::DatagramRequestMismatch`.
- **Evidence tier**: seeded sim. *(G-V6 row 2)*

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

**P2-S10 `TcpAccept` V_g end on host close (R5-28, R5-31, U-5, K-A4)**
`@contract-shape:unbounded-preservation`
- **Outcome**: a `Paired` from the guest for a host-aborted `TcpAccept` is
  discarded + counted, no reply; an `Abort` the guest receives for a flow it
  does not yet hold is discarded + counted, no reply; the guest half ends
  through the host's close of V_h — the request read succeeds, then EOF,
  `POLLRDHUP`, first write `EPIPE` — and no byte of the aborted flow reaches the
  application (H-1).
- **Contract**: U-5 discard (`guest_flow.control_discarded`); K-A4 — the host's
  close of V_h ends G's half; no byte on V_h after the request before `Paired`
  (P-38).
- **Oracle**: the `Paired`/`Abort` for the closed/unknown flow is discarded and
  counted; the guest owner ends the flow (no C, or C reset); zero application
  bytes delivered. The seam models V_g's EOF-after-request. *(G-V6 row 5; the
  production sockmap V_g→C route is P6's step-5 Tier-3)*
- **Evidence tier**: seeded sim + Quint `owner_flows`
  (`WitnessNoAcceptPaired`, `WitnessNoQueueReset`). Native V-24 shape is P6.

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

**P2-S12 half-close owner seam (R5-45, inv 13, HALFCLOSE-SIGNAL)**
`@contract-shape:bounded-change`
- **Outcome**: each owner shuts its flow vsock for writing only after
  `HalfCloseAck`; a flow vsock that ends with no acknowledged `HalfClose` (while
  the session is live) aborts the flow at once (`UnannouncedVsockEnd`) and every
  far end on that side is reset, never EOF; an announcing side whose peer's
  session is lost, or which receives `Abort`, before the ack aborts; a
  write-then-half-close before `Paired` is carried at `Paired`, a reset before
  `Paired` aborts; `HalfClose`/`HalfCloseAck` for an unknown/closed flow are
  discarded + counted (U-5).
- **Contract**: inv 13 `NoEofOnPeerLoss` + `FinOnlyAfterAck` — a far end
  observes EOF for a direction only if that direction's application/remote
  half-closed it, never because an owner died; no owner shuts a flow vsock
  before the peer acknowledged that direction's `HalfClose`.
- **Oracle**: ordered seam ops — `drained → HalfClose → (ack) → shutdown_write`;
  on unannounced vsock end, `abort` with `FlowCloseCause::UnannouncedVsockEnd`
  and linger-reset of outer legs; discards counted. Both owners (guest + host).
- **Evidence tier**: seeded sim (owner seam) + Quint `owner_flows` (`flows_half`
  — `NoEofOnPeerLoss`, `FinOnlyAfterAck`, `EarlyEofCarried`,
  `WitnessNoCleanEof`, `WitnessNoEarlyEof`). Tier-3 cell is P6 inside R5-10.

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

### Beacon attribution — seeded sim (`vsock_beacon_attribution.rs`)

Driven through the production `VmDriver` beacon accept seam with
`SimGuestCidClaim` supplying CIDs and `SimEntropy` the launch tokens.

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

### Conformance — `vsock_owner_flows_conformance.rs`

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
  projecting `State` from the owner's **observable** session/flow surface
  (`ConformanceNode`), and compares state after every step. `#[quint_test]`
  pins the named `checks.toml` runs (safety TLC/Apalache; the witnesses; the
  attribution hazards `peer-cid-only`, `claim-before-token-held`,
  `no-claim-recheck-*`; the beacon hazards; the half-close hazards
  `rdhup-is-halfclose`, `no-ack*`, `ack-waits`, `early-eof-aborts`);
  `#[quint_run]` runs randomized simulation with the seed printed on failure.
  The spec is referenced **in place** (`specs/quint/guest-flow-owner/owner_flows.qnt`),
  never copied; `checks.toml` is not edited (it is hashed into the evidence).
- **Evidence tier**: conformance, Lima, `integration-tests`, `host-kernel-shared`
  nextest group (spawns the `quint` CLI + drives the composed owner).

## Boundary-scenario placement (feature-delta §"Required boundary scenarios")

| Gate | Row 1 available | Row 2 typed failure | Row 3 unrelated unchanged | Row 4 late success | Row 5 disconnect/reconnect | Row 6 feature disabled |
|---|---|---|---|---|---|---|
| **G-V6** | P2-S3/S4 (`Paired`; early bytes flow) | P2-S13 (`Refused`/`PairingTimeout`/`DrainTimeout`) | P2-S13 (Running/Stable) | P2-S13 (`Paired` after Retiring → aborted) | P2-S10/S12 (opener disconnect; reset→abort; session loss; stale queued request) | P2 n/a (sim `transport: None` — P6 owns the fixture) |
| **G-V3** | P4 (`activate` → listeners); P2-S8 (session precondition) | P2-S13 (`activate` after `teardown` → `LeaseRetiring`) | P2-S13 (READY unchanged) | P2-S13 (`activate` after `teardown`) | P2-S8/S9 (stale queued session never counts live) | P6 fixture |

## Quint `owner_flows` module → check → scenario map (P2-S15)

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
| C5a mode-flag combinations | PASS | mesh vs non-mesh resolution (S11 + P5/P4 for VIP); live vs stale token × session/beacon/flow (S6/S8/S9/S14) |
| C5b flag orthogonality | PASS | per-flow refusal never changes allocation state (S13) |
| C6a malformed value per channel | PASS | bad magic/version/kind/origin/reserved/type/refusal (S1/S2) |
| C6b each declared error triggered | PASS | every `FlowRefusal` (S2) + `FlowCloseCause` cause enumerated across S5–S13 |
| C6c closed error set | PASS | `decode` accepts only canonical encodings (S1); no cause outside `FlowCloseCause` (S5–S13) |
| C7a degraded resource | PASS | quota/cell exhaustion (S11 `Capacity`) |
| C7b interruption mid-op | PASS | quiesce/teardown/session-loss mid-admission (S5); owner death mid-flow seam (S12) |
| C7c concurrent actors | PASS | two concurrent sessions/beacons (S8); both applications half-close concurrently (S12) |

**Verdict: 15/15 → COMPLETE.** Zero `SPECIFICATION_AMBIGUITY` (C2/C5/C6 all
resolved upstream: the state machine in D18/D18a, the mode flags in
ATTR-TOKEN/ATTR-LIVE-CLAIM/D9/D14, the error contract in the typed taxonomy §).

## RED / blocked notes

All bodies are authored to the **design-pinned** interface contracts
(feature-delta §"[REF] Interface contracts") and marked
`#[ignore = "pending DELIVER …"]`. They compile — and are registered in their
crate's `tests/acceptance.rs` / conformance entrypoint — only once the DELIVER
roadmap-reconciliation creates the production scaffolds they call:

- `vsock_wire_codec.rs`: `overdrive_core::guest_transport::{wire::*, LaunchToken,
  GuestCid, FlowId, FlowOrigin, FlowKind, FlowRefusal}` + `overdrive_core::net::ListenPort`.
- `vsock_owner_flow_admission.rs`: the reshaped `SharedGuestNetworkOwner` +
  `GuestNetworkProvisioner`, `GuestTransportAssignment`, the owner's private
  flow-admission test seam, `SimGuestFlowForwarder` (shared), `SimMtlsResolve`,
  `SimGuestCidClaim` (P1), `SimEntropy`.
- `vsock_beacon_attribution.rs`: the `VmDriver` beacon accept seam,
  `BeaconMessage::Ready { launch }`, `SimGuestCidClaim`, `SimEntropy`.
- `vsock_owner_flows_conformance.rs`: `tests/conformance/src/{vsock_harness,quint_driver}.rs`
  (shared base) + the owner's observable session/flow surface on `ConformanceNode`.

Nothing is blocked on a validation item (V-22 done P-37; V-24 done P-38; the
ATTR-LIVE-CLAIM and HALFCLOSE-SIGNAL decisions ARE the design and the Quint
round-9 re-run is recorded). The gating above is the normal RED-scaffold
co-sequencing with DELIVER (shared doc §0), not a contract-shaping blocker.

## Testability gaps routed to DELIVER / shared authors (not fixed here)

1. **Owner private flow-admission test seam** — DISTILL-shaped test-support the
   DELIVER owner step must expose: inject an accepted guest-opened connection
   `(peer_cid, buffered_first_record, has_buffered_bytes)`; drive
   `provision/activate/teardown/quiesce/restore`, `ListenState`, session
   open/loss; read the observable `(cid_state, claim_holder, control_messages,
   flow_counters, FlowCloseCause)` for the before/after snapshot. The owner must
   NOT expose private fields — only this observable surface
   (`.claude/rules/testing.md` §"Quint conformance").
2. **`VmDriver` beacon accept seam** — same shape for the beacon listener
   (inject `(peer_cid, buffered_first_line, has_buffered_bytes)`; observe claim
   holder + `vm_beacon.*` counters).
3. **`SimGuestFlowForwarder`** (shared) — install/route/abort/drain with an
   ordered operation log so S3/S4/S12 can assert op ordering.
4. **`tests/conformance/src/vsock_harness.rs` + `quint_driver.rs`** (shared
   base) — `boot_vsock_node` + `ConformanceNode` observable surface + the
   quint-connect `Driver`/`State` wiring S15 specialises.
5. **Wire-type module path** — the design leaves the exact module for
   `FlowId`/`FlowKind`/`FlowRequest`/`ControlMessage` to the crafter ("picks the
   module that satisfies the crate classes; the values exist exactly once").
   The bodies import from `overdrive_core::guest_transport::wire`; the crafter
   finalises the path (an internal-structure note for the DELIVER reviewer, not
   a DESIGN finding).
