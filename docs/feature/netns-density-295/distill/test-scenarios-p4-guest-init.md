# P4 — Guest `overdrive-init` (netns-density-295, EVMOD)

Owner: the guest flow owner `GuestFlowOwner` (`overdrive-init::guest_flow`,
driving port `dispatch_next`) over the DG-5 seam sim `SimGuestOwnerIo` /
`SimGuestClock` (`guest_flow::sim`), whose forwarder half is
`SimFlowForwarder` (QPROJ-8 arbiter); the transport setup
(`transport::{parse_transport_token, configure_guest_transport}`,
`KernelRelease`); `main` wiring. Binary crate, so every test is
source-local. Markers: `pending DELIVER step 5` (guest adaptation; it
follows step 4, which lands `SimFlowForwarder`), `attachment-cut step (5)`
for the `main` cells. Semantic RED: `GuestFlowOwner` / `SocketCookie` /
`KernelGuestOwnerIo` / `parse_transport_token` / `configure_guest_transport`
/ `KernelRelease` scaffolds, or the sim forwarder's, panic
`Not yet implemented -- RED scaffold`.

## Example cells (`src/guest_flow/tests/`)

| File | Scenarios |
|---|---|
| `session.rs` | `guest_owner_fatal_renders_its_pinned_display_texts` (**active**: `EventWait`, `ListenerFailed`, `Forwarder`); the session opens with a `ControlOpen` carrying the guest's own token (QPROJ-5 edge cases, D-UP-7 `next_deadline = now`, ATTR-TOKEN `ControlOpen` first); the audit tick is fixed-rate from the first dispatch (DG-5-A); session loss aborts every flow in the observing dispatch before any reconnect (DG-5-F over a paired dial, an accepted dial, paired and pending `TcpConnect`, an attached association; `abort_forward` per pair; far ends `Reset`; reconnect at `CONTROL_RECONNECT_BACKOFF`); the host's end of the session observed as EOF or read hang-up (K-A5; no `ECONNRESET` / `EPOLLERR` asserted); a host owner restart resets flows and reopens the session without a fatal; every session-loss shape is recovered, not fatal (H-4); the guest holds one session and reopens only after observing its loss (D-UP-8); a message for an unknown flow changes nothing (U-5); only `EventWait` and listener failures are fatal |
| `tcp_accept.rs` (INB-1) | a dial with the guest's token attaches at connect and then pairs (C bound to `gateway:0` → `workload_addr:port`; read < end check < connect < attach < `Paired`; `V_g` half A); a paired dial forwards both ways; another launch's token closes `V_g` silently (ATTR-TOKEN); a `V_g` the host closed after the request ends with no C and no message (TCPACCEPT-END); a refused application connect answers `Refused { errno }`; past the inbound timeout `TimedOut { ETIMEDOUT }` once; an accepted `V_g` closed by the host ends silently and an error on it aborts with `Abort` (DG-5-E); a queued `V_g` closed by the host never reaches the guest and its abort is discarded (K-A4, R5-28 guest half); an abort of a paired dial aborts its pair and resets the application; an application reset before the attach is refused and answered with its errno, a `V_g` ended before the attach is refused and ends with no message (EVMOD-ATTACH-END, `forwarder_attach_ended` +1); control messages for closed dials discarded (U-5); a connect completing after session loss sends nothing (U-1); accept-path outcomes by first record (R5-59 (b) sim half) |
| `tcp_connect.rs` | a captured connect writes the request once and attaches on `Paired` (`V_g` half B); an application write and FIN before `Paired` attach in `CLOSE_WAIT` and complete (EVMOD-PAIR, CLEAN-FULL-END, linger cleared before close, the application reads EOF); an intake child reset before `Paired` is refused `HalfEnded` and the flow aborts; a `V_g` reset before `Paired` is refused `HalfEnded` and I is reset (I closed with linger, `V_g` closed, `forwarder_attach_ended` +1); past `GUEST_TCP_PENDING_MAX` reset locally; a refused `TcpConnect` resets the application (D20); an application reset before `Paired` aborts and sends `Abort`; the host's close of an unattached `V_g` resets I with no message (DG-5-E); a paired `TcpConnect` forwards both ways |
| `flow_end.rs` (EVMOD-REC) | a clean end both ways completes and the owner closes on the record (one record; no drain step); a `PeerAbort` record closes the flow and the application reads a reset; a half-close then the peer's close after both ends is clean; an application reset while paired is `SocketError { A, ECONNRESET }`; a FIN then a reset from the application aborts the pair; an `Abort` for a paired flow aborts its pair (`abort_forward` → `Detached`); an `Abort` for a flow the guest does not hold is discarded (R5-28 guest half); a slow reader holds the flow without any deadline (ADR-0183); a trailing reset after completion is inert (END-IDEMPOTENT) |
| `udp_slots.rs` | a first datagram claims a slot, associates, attaches and forwards (S half A); slots per socket and destination; a socket close releases every slot it owns in the release order (abort → discard + disconnect → release); a lost release report recovered by the audit tick (K-B2); an idle slot released after the idle period (120 s); a released burst discarded, the next owner gets none of it (R5-14, P-72); a refused association discards its queue and retries only on a new datagram (U-2, R5-27); an abort of a live association aborts, discards, disconnects and keeps the slot (EVMOD-DGRAM); a `PeerAbort` record ends the association and a late `Abort` is discarded; the pairing deadline aborts and a late `Paired` is discarded; a late `Paired` for a released slot is answered with `Abort` (SLOT-ABORT); a refused destination gets at most one attempt per round trip under a 500-send burst (R5-27) |
| `newtypes.rs` | `SocketCookie` round trip; non-decimal, zero, overflow and empty refused |

`transport::tests` (kept from the earlier pass): every setup error's pinned
`Display` and the lower-snake-case labels (**active**); the token grammar
with `launch=`; `configure` runs every step in the pinned order (… programs,
module, context, inbound listener); it stops at the first failure with that
step's variant; each forwarder-module failure is a pre-READY start failure
of its own kind (`ForwarderModuleMissing` / `ForwarderModuleRelease` /
`ForwarderModuleLoad`); the `KernelRelease` codec. `main.rs`:
`the_guest_flow_owner_runs_on_its_own_thread_before_ready`,
`ready_carries_the_launch_token_of_the_transport_token`,
`resolver_write_failure_suppresses_ready_and_exec` (attachment-cut (5));
`a_setup_failure_is_one_console_line_and_starts_neither_owner_nor_ready`
and `a_guest_owner_failure_is_one_console_line` (**active**).

## Conformance drivers (source-local, `integration-tests`, all step 5)

| Driver | Spec · main | Named runs + `#[quint_run]` | Projection |
|---|---|---|---|
| `guest_flow::quint_owner_flows` | `owner_flows.qnt` · `flows_conf` | 45 + 1 | `gf(h).st` and `gSess` by qualified names (`Qualified`); `guest_observation()`; the accept-queue tear-down read through the seam arbiter (marked, Tier-3 V-24 / P-38) |
| `guest_flow::quint_fwd_pair` | `fwd_pair.qnt` · `pair_conf` | 37 + 1 | guest `gen` / `cgen` constant 0 ↔ `ForwarderGeneration::FIRST`; `pr(g)`, `ctx(g)`, `oldCtx(g)` (context table, arbiter), `vs(g).ps` / `.sent` / `.cls` / `.st` / `.sshut` / `.err`, `dir(*)` queues and `pair_state`, `tcpErr(g)`, `dg(hg).got` (each `SimSocket` / `SimFlowForwarder` read marked with its Tier-3 pair); host-side actions are stimulus |
| `guest_flow::quint_udp_slots` | `udp_slots.qnt` · `slots_conf` (guest side) | 12 + 1 | slot table and per-origin queues (seam arbiter, Tier-3 R5-14 / V-EV-DGRAM) |

Every action is mapped; hazard and other-instance actions return errors;
`Config::nondet` is the single element `"<main>::<module>::conf_step"`.
Activated once each in this integration pass: RED on the
`SimFlowForwarder` scaffold (`fwd_pair`, `udp_slots`) and on the wire-codec
scaffold (`owner_flows`); see `red-classification.md`.

## In-guest real-kernel cells (`kvm-tests`)

`transport::real_kernel` (setup postconditions, R5-15, the module release
check) and `guest_flow::real_kernel` (R5-59 (b) kernel half over vsock
loopback), gated `cfg(all(test, feature = "integration-tests", feature =
"kvm-tests"))`. They fail closed outside a guest and never run in plain Lima.
The guest lane (user ruling T-2, 2026-10-18) is
`tests/integration/in_guest_lane.rs` (`kvm-tests`, native metal): it boots a
guest-kit microVM with the production `overdrive-init` as PID 1, plays the
node beacon and the host control session, and runs each in-guest cell with
`--exact` as the guest's operator command (a cell passes only on "1
passed"); it runs the four existing in-guest cells plus a missing-module
cell (EVMOD-GUEST-KIT: the guest refuses before READY when `ovd_fwd.ko` is
absent). Pending step 5. R5-59 (b)'s kernel
half is placed in-guest over vsock loopback (D-UP-18: the lane is DISTILL's),
not on host-opened connections as the design text suggests.

## Deleted

`guest_flow/quint_flows_half.rs`, `guest_flow/tests/half_close.rs` (list C),
the redirect-era rows (drain, cells, parking, `CellGeneration`, routed /
installed phases, SEQPACKET), and the untracked `overdrive-bpf`
`maps/guest_flow.rs` drain-cell / `FLOW_EVENT_RING` scaffold.
