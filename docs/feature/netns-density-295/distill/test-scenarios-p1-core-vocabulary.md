# P1 — Core vocabulary, wire codec and the sim forwarder (netns-density-295, EVMOD)

Owners: `overdrive-core::guest_transport::{forwarder, wire, dial, mod}`,
`overdrive-core::aggregate::workload_spec` (the two deploy ingresses), and
`overdrive-sim::adapters::flow_forwarder` (`SimFlowForwarder`,
`SimForwarderDevice`, `SimSocket`).

Layers. Every core cell is a pure / unit layer-1 cell: no Quint driver, the
oracle is the typed result, the `Display` text or the golden bytes;
proptest for every unbounded input domain (Mandate 9). Every sim cell is a
layer-2 in-memory cell over `SimFlowForwarder`, the named arbiter of the
forwarder's kernel facts (QPROJ-8); its Tier-3 pair is the `overdrive-fwd`
module suite (P2: V-EV-END, V-EV-CTL, V-EV-PAIR, V-EV-DGRAM, P-69..P-72,
and `adapter_equivalence.rs` for R5-57).

Markers: core cells `pending DELIVER step 2` (core types and wire codec);
the D25 reserved-port and D-UP-12 probe-cap ingress cells are attributed to
step 2 as well (core spec parsing — the Roadmap impact names neither; see
`upstream-issues.md` O-1); sim cells `pending DELIVER step 4` (forwarder
module and adapter, which lands `SimFlowForwarder`).

## `overdrive-core/tests/acceptance/`

| File | Cells (contract → oracle) | Active cells |
|---|---|---|
| `vsock_forwarder_vocabulary.rs` (new) | `ForwardId` / `ForwarderGeneration` round trip through `Display` / `FromStr` / serde (proptest); ordering by value; `Zero` / `NotDecimal` refusals; `FIRST = 1`, `next()` = +1, `next()` at `u64::MAX` = `None`; `PairSide` labels distinct; `AbortCause::as_str` one distinct label per cause class, independent of side / errno (SSOT); `HalfEnded` `Display` names side and errno | default inventory all zero; every `ForwardIdError` / `ForwarderError` `Display` text; `Probe` / `Io` keep their source chain; `FlowForwarder<dyn AsFd>` is object-safe |
| `vsock_wire_codec.rs` (rewritten) | every `FlowRequest` kind 1 / 2 / 4 with the launch token round-trips and decodes canonically; golden bytes per kind; kind 3 is `UnknownKind`; `ControlOpen` round trip, golden bytes and every refusal; an all-zero token refused everywhere; the flow-id origin bit; every malformed request / open / control message refused with its cause; `ControlMessage` `Paired` / `Refused { errno }` / `TimedOut { errno }` / `Abort` golden bytes for every type and refusal code 1–12; READY `launch=` golden and `FieldCount` / `MissingKey` / `InvalidLaunchToken` (DG-7); `KernelErrno` round trip, 0 refused, named errnos; distinct kind / refusal labels | wire error texts; `GUEST_FLOW_MAX_PAIRS`; the deadline values and `DIAL > INBOUND_CONNECT` |
| `vsock_guest_transport_vocabulary.rs` (rewritten) | CID derivation per lease offset round-trips and is unique (R5-55a); out of span and the kernel-reserved CIDs are `OutOfRange`; `LaunchToken` round trip, entropy-drawn, rejects zero / upper case / wrong length; the handoff record; a new launch is a new handoff; `ListenPort` / `InterfaceIndex` round trip and refusals; the reserved range above the ephemeral range | span `/16`; vsock ports; intake constants; quota and `GUEST_PROBE_SLOTS = 8` (const asserts); `GUEST_FORWARDER_MODULE_PATH` |
| `vsock_datagram_frame.rs` (rewritten, R5-51) | every payload `0..=MAX` round-trips; accepted bytes are canonical; non-canonical frames refused; golden empty and maximum frames; spike sizes; the empty datagram is a header-only frame; a frame-shaped payload is stripped of exactly one outer frame; framed sequences keep order and every empty; oversize refused by `new` and by `decode`; `Truncated`, `BadMagic`, `LengthMismatch` | `MAX_FRAME = MAX_PAYLOAD + header`; refusal texts |
| `vsock_guest_stream_dial_vocabulary.rs` (EVMOD-STREAM-END / -ABORT) | bytes in order then `Ok(0)` on `HalfClose`; a guest abort is `ConnectionReset`; writes and `shutdown_write` forwarded; an owner abort reaches every call as `ConnectionAborted`; distinct `DialPurpose` / `GuestStreamEnd` / `GuestDialError` labels; errno-carrying error texts | `target()`; no-errno error texts; the dial is a shareable trait object |
| `vsock_deploy_validation.rs` (R5-21, R5-60) | a reserved listener port refused at the TOML ingress (`ListenerPortReserved`) and at the API ingress (`Validation { field: "listeners[].port" }`); proptest: refusal iff the port is in the range; 8 probes accepted in any role shape, 9 refused `ProbeCountExceeded { count: 9, max: 8 }` (the inferred startup probe counts) and `Validation { field: "probes" }` at the API; proptest: refusal iff count > cap | — |
| `vm_beacon_roundtrip.rs` | READY round trip, wire shape, trailing newline, missing key prefix, non-integer pid (step 2) | `EXIT` / `SHUTDOWN` / `EXEC` cells |

Deleted with the redirect era: the `FlowEvent` / `FlowCell` / `CellEpoch` /
`WireField::Event*` cells and the empty-notice cells, and the stale
`*.proptest-regressions` files of `vsock_wire_codec`,
`vsock_guest_transport_vocabulary`, `vsock_datagram_frame` and
`vsock_deploy_validation`.

## `overdrive-sim/tests/acceptance/sim_flow_forwarder.rs` (new, all step 4)

| Cell | Contract | Oracle |
|---|---|---|
| a stream pair forwards both ways and completes on two clean ends | EVMOD-PAIR, CLEAN-FULL-END | `moved_ab` / `moved_ba`, unhook, inventory |
| vsock `0x3` with a direction open is `PeerAbort` and resets the TCP far end | EVMOD-END | end record, `peer_end() == Reset` |
| a TCP RST after FIN aborts with `SocketError { EPIPE }` | hazard 16 | end record |
| ends after a full clean end change nothing | END-IDEMPOTENT | unbounded-preservation |
| a target that stops reading blocks the sender until it reads again | backpressure (P-67, P-68) | queue facts |
| unknown or other-context ids give `AlreadyEnded` / `pair_state None` and touch nothing | EVMOD-CTL | unbounded-preservation |
| `take_end` returns each record once, oldest first; a second abort is `AlreadyEnded` | EVMOD-REC | record order |
| `abort_all` queues one `Detached` record per live pair and watch | EVMOD-AB | records |
| context release aborts everything (TCP reset, vsock `0x3`, watched vsock shut), queues no record, leaves another context alone | EVMOD-CTL, WATCH-ABORT | context table |
| `HalfEnded` for a reset TCP half (`ECONNRESET` / `EPIPE` in `CLOSE_WAIT`), a `CLOSING` vsock and a swept `sk_err`, changing nothing | EVMOD-ATTACH-END | typed refusal; the other half stays attachable |
| ends present at attach are carried out by the first run | EVMOD-END | record |
| owner defects refused with their typed cause | `UnsupportedSocket` / `UpperLayerProtocol` / `NotConnected` / `AlreadyHooked` | typed error |
| a watch on an already-ended vsock queues its record at once; a watch record is queued by the classifying callback; aborting a watch changes no socket fact | ADR-0216, WATCH-ABORT | records, socket facts |
| a datagram vsock half below `MAX_GUEST_DATAGRAM_FRAME` is `BufferTooSmall` | EVMOD-DGRAM | typed refusal |
| datagrams cross in order, empty ones included; oversize counted `dgram_oversize`; a bad frame aborts `FrameInvalid` | EVMOD-DGRAM, R5-51 | delivered datagrams, inventory |
| proptest: `live_pairs + live_watches = made − ended`, `queued_ends = ended − read`, no record read twice | C4 inventory invariant | inventory |

The sim's stepping contract (test support, pinned in the rustdoc): one
`run(id, source)` call is one `fwd_pair` hop — one stream byte or one
datagram frame item, or, once the source queue is empty, the classified end
(never a unit and the end in one call); `peer_resume_reading` delivers
everything queued toward the far reader in one call; `local_read` on UDP
returns one datagram per call; `received_shutdown` reads back the peer's
SHUTDOWN flags (`vs(s).ps`, QPROJ-8).

## Not authored

Nothing in P1 waits on DESIGN. The redirect-era production items still in
`wire.rs` / `mod.rs` (`FlowKind::DatagramSeqpacket`, `FlowEvent`,
`FlowCell`, `CellEpoch`, `FlowEventKind`, `FlowEventDirection`,
`FLOW_EVENT_LEN`, `WireField::Event*`, `FlowWireError::UnknownEventKind`,
`GUEST_FLOW_DATAGRAM_PORT`, the intake steering / host-port constants) are
undefended by any test; other partitions' unmigrated production code still
uses them, and DELIVER deletes them with list E (`upstream-issues.md` O-2).
