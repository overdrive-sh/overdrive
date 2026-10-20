# P3a — Host guest-flow owner (netns-density-295, EVMOD)

Owner: the production host owner in `overdrive-control-plane::guest_network`
(`HostSharedGuestNetworkOwner`, its `GuestStreamDial` implementation) over
the DG-3 seam sim `SimGuestFlowIo` (`guest_network::flow_io::sim`,
`pub(crate)`), whose forwarder half is `SimFlowForwarder` (QPROJ-8 arbiter).
Source-local tests (testing.md § "Quint conformance": the seam is
crate-private). Seeded where ordering matters, the seed printed. Example
cells for the unique invariants; the drivers carry the interleavings.

Markers name the Roadmap impact: `attachment-cut step (7)` owner flows,
`(8)` mTLS registration, `(9)` host platform clients dial through the owner,
`step 11` VIP placement, `step 13` guest-prefix boot checks, `step 4` the
sim forwarder. Semantic RED: the owner scaffold
(`crates/overdrive-control-plane/src/guest_network.rs:854`, `assign` /
`quiesce` / the dial) or the `SimFlowForwarder` scaffold panics
`Not yet implemented -- RED scaffold`.

## Example cells (`src/guest_network/owner_acceptance/`)

| File (cells) | Scenarios | Pending |
|---|---|---|
| `admission.rs` (16) | R5-9 / H-1 attach only after the request is fully read; `Paired` after attach with a server-first banner; R5-26 quiescence, teardown and session loss on a connecting flow and on an unread registration; R5-16 `NoControlSession` / `StaleLaunch`; R5-17 malformed `DatagramStream` → `Refused(Malformed)`; R5-18 racing sessions and the previous launch's queued session; R5-41 sim (stale session / flow / datagram / empty); R5-20 no owner deadline; G-V3 rows 1, 2, 4; flow-local refusal | (7) |
| `flow_end.rs` (15) | EVMOD-REC completed both ways (`ClearLinger` before `Close`, `PeerEof`); a guest half-close needs no owner step; guest `0x3` / RST → `PeerReset` with no `Abort` message; a destination reset after FIN; the `Unclassified` alarm (`unclassified_eof`); `HalfEnded` at attach (`attach_ended` +1, `Abort` sent, no `Paired`); guest close before attach; an end before attach carried by the attach-time run; END-IDEMPOTENT; stalled-reader backpressure; quiescence aborts a stalled flow (`Quiesced`); owner death; VM end through device release; T1–T6 | (7) |
| `quota.rs` (5) | the quota boundary; a half-closed flow holds its place until its record; an abort releases at the record (EVMOD-AB); D-UP-17 `held_flows` mix; R5-20 releases only the allocation's own quota | (7), (9) |
| `datagrams.rs` (9) | concurrent associations; order with the empty datagram; gateway:53 vs prefix policy; VIP `connect4` + kernel-peer policy; R5-7 `EPERM` → `DestinationUnreachable`, then `HostInternal`; R5-27 host half; slot release → `PeerReset`; a bad frame → `FrameInvalid`; U-5 discard | (7), step 11 |
| `dial.rs` (25) | the watch installed before the request is written; `Ok` + drop aborts the watch then closes; `Refused` / `TimedOut` / `PairingDeadline`, a late `Paired` discarded; `NotServing`, `Provisioned`, `Quiesced`, `NoSession`, `Unreachable` (`ENODEV` / `ECONNRESET`); in-flight teardown / quiesce / session loss; cancellation; the `Capacity` relay vs the probe exemption; R5-63 (a)–(d), (f) (the `Forwarder` recovery ends streams, then reopens the context at the next generation; a context release reads as the owner's abort); R5-28 and R5-31 host halves | (9) |
| `policy.rs` (7) | R5-6 classes and policy order; R5-7 TCP `DestinationUnreachable`; D14 marked lingering socket; R5-8 mesh leg-F unmarked loopback bind; `MeshUnreachable` / `ResolveFailed`; `NotInterceptLive` → `IntakeUnavailable` | (7), (8) |
| `errors.rs` (6) | the C6c taxonomy incl. `Forwarder`; `probe_startup` refusals; forwarder open / probe failures typed `Forwarder { source }` with nothing bound; listener bind failure; the GW-LOCAL route written last, a failed write, left behind, idempotent; `PostconditionMismatch` | (7), step 13 |

`flow_io/sim/tests.rs` (10): the seam's self-tests, seeded. The two
gateway-route tests are active; the arbiter tests are pending step 4: a
destination reset gives `next_end` `SocketError { B }` and far end `Reset`;
`close_forwarder` releases its context row; watching a swept `V_h` is
refused `HalfEnded { ECONNRESET }`; the D-UP-19 accept order; the QPROJ-7
`socket_observation`. Test-support additions to the seam: `vsock_of_opt`,
`peer_of_opt`, `latest_peer_socket` (the far end's facts before attach) and
`caused()` (wakes the owner after a cause scripted on a `SimSocket`).

## Conformance drivers (source-local, `integration-tests`)

| Driver | Spec · main | Named runs (`max_samples = 1`) + `#[quint_run]` | Projection | Pending |
|---|---|---|---|---|
| `guest_network::quint_owner_flows` | `owner_flows.qnt` · `flows_conf` (`"flows_conf::owner_flows::conf_step"`) | 45 + 1 | `flow_observation()` (QPROJ-3) by qualified names; arbiter reads marked | (7) |
| `guest_network::quint_fwd_pair` | `fwd_pair.qnt` · `pair_conf` | 37 + 1 (the retired `OwnerAbortOfWatchShutsVh…` / `OwnerAbortedWatchHolderReadsNotConnected…` runs are gone) | `ctx` / `oldCtx` (context table, arbiter), `gen` (`forwarder_generation`), `pr(h).ph` / `cgen` / `rec`, `attEnded`, `strmAbort`, `hres` (production `GuestStream`), `tcpErr`, `vs(h).err` / `.st`, `gh.unclassified` | (7) |
| `guest_network::quint_flow_quota` | `flow_quota.qnt` · `quota_conf` | 5 + 1 | quota observation; `Q` / `CAP` realised with background flows | (7) |
| `guest_network::quint_udp_slots` | `udp_slots.qnt` · `slots_conf` (host side) | 12 + 1 | `host(i).st` ← `datagrams[i].phase` plus `closed_by_owner`; `RouteType` held per association | (7) |

`fwd_pair` host: every spec action is mapped; six hazard-only actions return
errors; the guest side is replayed as packet stimulus.
`ForwarderRecoveryAbortsThenReopens` drives quiesce → `audit_shared` damage →
`converge_shared` with `open()` held → `Reopen`. `udp_slots` host:
`HostRefuse` via a scripted `Local` route class; `HostAbortAssoc` via `EPERM`
on H; `HostRecvEnd` via the guest's `Abort` (unpaired) or a vsock close
(paired); `AppPick` / `AppTake` hazard-only.

All 11 drivers were activated one `#[quint_test]` each in this integration
pass: each replays to the semantic RED (`red-classification.md`).

## Deleted

`quint_flow_end.rs` and `quint_udp_empty.rs` (their specs `flow_end.qnt` /
`udp_empty.qnt` were removed by EVMOD-ADOPT / EVMOD-DGRAM).

## Not authored / partial (`upstream-issues.md`)

R5-59 (a), flow seam, is authored source-local in
`guest_network/flow_io/accept_path_tier3.rs` (`kvm-tests`, pending
attachment-cut (7)): a guest-kit microVM at a band CID opens connections to
the stream and control listeners' kernel-assigned ports (read back from the
binds, PORTS-CONFIG) from chosen source ports, with or without a record, compared per
connection with `SimGuestFlowIo` (identity, first record, no extra accept).
`flow_quota` reads end records in queue order and projects `endq` (D-12,
S-5). `fwd_pair` (host) projects `strmAbort` under the dial-entry lifetime
rule (D-3), `pr(h).rec` / `nrec` across contexts from `queued_ends`,
`.closed`, `.byRel` and `attRes(h)` from the cumulative counters
(`forwarder_attached`, `forwarder_attach_ended`) after the flow left the
table, and all of `vs(h)` through `vsock_half`; `CtxRelease(h)` has two
mappings by state (owner death, S-3; the recovery's close); two runs added
(`HostOwnerDeathIsFinal`, `RecoveryEndsHeldStreamWhoseWatchEnded`). Open:
`pr(h).out` / `.cause` / `.errno` (D-11). Every control-plane driver
overrides `from_spec`.
