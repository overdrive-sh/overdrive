# P2 — The `ovd_fwd` module through `KernelFlowForwarder` (netns-density-295, EVMOD, Tier 3)

Owner: the new crate `overdrive-fwd` (`crate_class = "adapter-host"`,
`KernelFlowForwarder`, the UAPI mirror in `src/uapi.rs`, the probe) driving
the module `ovd_fwd` (`kmod/ovd_fwd/`, DELIVER step 4). Tier 3 on the host
role of the Lima VM (Linux 7.0), run as root through `cargo xtask lima run
--`, gated by `integration-tests`; the whole `overdrive-fwd` integration
binary is in the `host-kernel-shared` nextest group with ten slow periods
(cases unload and reload `vsock_loopback`, bind-mount over `/proc/modules`
and `/dev` in thread-private namespaces, install nft tables in a private
netns, call `delete_module` and measure node-wide `SUnreclaim`).
Example-based (Mandate 9, layer 3+); each sad path a named test (Mandate 11).

Every case is `#[ignore = "pending DELIVER step 4"]`. Semantic RED on
activation: `KernelFlowForwarder::open` (the first call of every case) and
the adapter bodies panic `Not yet implemented -- RED scaffold` until step 4
lands the module and the adapter.

Prerequisites (DEVOPS, land with step 4): `ovd_fwd` built for the VM's
running 7.0 release and loaded once per boot (EVMOD-LOAD); `vsock_loopback`
loadable; the `tls` module available (EVMOD-KTLS refusal case); `nft` and
`ip` on PATH.

## Cases (`crates/overdrive-fwd/tests/integration/`)

| File (cases) | Contract | Cases |
|---|---|---|
| `probe_gold.rs` (6) | the probe (V-EV-CTL, EVMOD-SAFE) | passes with no residue; absent device → `open` `DeviceAbsent` and probe refused at `DeviceOpen` / `ContextRelease`; a non-forwarder device (`/dev/null` bind) → `Version`; (MODULE-REFCOUNT: no permanence check, so the `/proc/modules` cells are withdrawn) `FD_CLOEXEC` cleared → `ProbeMismatch { Version, ContextInheritable }`; `vsock_loopback` unloaded → `StreamRoundTrip`; FIN rejected with reset in a private netns → `StreamRoundTrip` / `HalfClose` (never a pass) |
| `attach_watch.rs` (16) | attach order, accepted states, refusal precedence (EVMOD-PAIR, EVMOD-ATTACH-END, EVMOD-KTLS) | ESTABLISHED halves accepted and pre-attach bytes forwarded; TCP `CLOSE_WAIT` accepted and its FIN carried out (h_strict); vsock with peer `0x2` classified clean at attach (h_noseed); `AF_INET6` TCP; TCP reset before attach → `HalfEnded { B, ECONNRESET }`; reset in `CLOSE_WAIT` → `HalfEnded { B, EPIPE }`; vsock `CLOSING` → `HalfEnded { A, ECONNRESET }`; vsock `0x3` with bytes queued accepted then `PeerAbort { A }` with 0 moved; LISTEN / unconnected → `NotConnected` naming the side; own `SHUT_WR` → `NotConnected`; wrong kinds → `UnsupportedSocket`; kTLS ULP → `UpperLayerProtocol`; hooked here or in another context → `AlreadyHooked`; a 32 KiB datagram vsock → `BufferTooSmall`; precedence `ENOTSOCK` → `EINVAL` → `ESOCKTNOSUPPORT` → `EPROTONOSUPPORT` → state (A then B) → `EBUSY` → `EMSGSIZE`; a refused attach consumes an id, ids strictly increase across pairs and watches |
| `stream_battery.rs` (12) | the evmod battery shapes (V-EV-END, P-67, P-68) | eof, tiny, bidir 64 MiB × 2, long 512 MiB, tight 64 KiB buffers, burst 10k small writes, slow 4 KiB/ms reader, a stall blocks the sender and resumes byte-exact, 1,000 cycles with one record each, cut (close with unread → `SocketError { B }` + guest abort), rst (`ECONNRESET`), `kill -9` of the vsock holder (`PeerAbort { A }` + TCP far reset) |
| `stream_end.rs` (9) | R5-45 (a)–(c), CLEAN-FULL-END, END-IDEMPOTENT | `0x2` forwarded as FIN after the last byte, with `pair_state`; TCP FIN → guest half-close; `0x3` with a direction open → `PeerAbort` + far reset, never EOF; half-close then close while sending → abort; FIN then reset → `SocketError { B, EPIPE }` via `sk_error_report` (hazard 16); reset with both open → `ECONNRESET`; 300 CLEAN-FULL-END rounds all `Completed` (h_lateshut); a trailing reset after completion adds nothing; an abort over a 6 MiB stalled pipeline forwards no byte past the abort |
| `paused_and_stall.rs` (7) | R5-49, R5-62 (ADR-0183, VALVE-IN-295) | paused TCP and vsock readers never see the half-close before the last byte; a vsock `SHUT_RD` target aborts; a paused flow is held with no deadline while another proceeds; a 10 s stall within the per-flow bound with `SUnreclaim` flat (≤ 1 MiB rise); a TCP-source stall bounded; a `SHUT_RD`-only peer holds until close |
| `datagram.rs` (14) | R5-52, R5-51 (EVMOD-DGRAM) | mix 64–59,000 B and mixs 64–2,047 B at 256 KiB and 64 KiB delivered once, whole, in order; a stalled vsock reader → drops only at the UDP source, partial frames resumed with no duplicate; empties both ways once; a frame-shaped payload stripped of exactly one frame; oversize dropped and counted `dgram_oversize`; bad magic / oversized length → `FrameInvalid`; a truncated frame at a clean `0x2` end → `FrameInvalid` after the whole frames (trunc_fix=0 control); a clean end after whole frames → `Completed`; an abort-class end with a partial frame → `PeerAbort`; an `ECONNREFUSED` destination → `SocketError { B }` |
| `watch.rs` (7) | ADR-0216, WATCH-ABORT | a guest half-close is recorded and the host direction stays with the holder; a guest close → `Aborted { PeerAbort }`; an already-ended guest direction records at once; 400 rounds: the record is queued before the holder's read ends (hazards 4 / 8); an abort records `Detached` before return, sends nothing, leaves the socket unchanged both ways, re-watchable, the stale id `AlreadyEnded`; `abort_all` counts and records every pair and watch; the context release shuts the watched `V_h` (the holder reads 0 or `ENOTCONN`, a write `EPIPE`, F11) and queues no record |
| `context_release.rs` (4) | R5-61, EVMOD-CTL | the release of 48 pairs + 16 watches resets every TCP far end and every vsock peer classifies abort; one context's release leaves another's pairs forwarding; a 1,000-pair reset storm → exactly one record per id; (the permanence cell is withdrawn: V-EV-UNLOAD below) a child spawned with the context open holds no `/dev/ovd_fwd` descriptor and keeps nothing alive |
| `end_record_abi.rs` (2) | EVMOD-REC mirror round trip | a raw read of `readiness_fd`: a buffer below 32 B is `EINVAL` and consumes nothing; 100 B returns 3 whole records at the pinned offsets / values, then `EAGAIN` and no `EPOLLIN`; `moved_ab` / `moved_ba` equal the bytes each target accepted |
| `adapter_equivalence.rs` (2) | R5-57 real half | the same step sequence (attach, watch, `0x2`, `0x3`, FIN, reset incl. FIN-then-reset, owner abort incl. of an ended id, `abort_all`; a 64-flow teardown) against `KernelFlowForwarder` and `SimFlowForwarder` yields equal inventory, live `pair_state` and end records after every step |

`support.rs` holds the shared fixtures (`open_context`, `tcp_pair`,
`vsock_pair`, `udp_pair`, `attach_stream`, `reset_close`, `wait_end_of`,
`close_watched` — a live watch holds a file reference, so a test closing a
watched far end aborts its watch first, P-70 § 4.4).

The negative controls of the spikes (h_lateshut, h_noseed, h_strict, hazards
1 / 2 / 4 / 8 / 16, trunc_fix=0, no-discard) are falsification notes in
`tests/integration.rs`, not module variants.

## Default-lane unit cases (`src/uapi.rs`, pending step 4)

Every pinned end-record shape decodes to its `ForwardEnd`; an out-of-range
field is refused `EndDecode` naming the field (`ForwardRecordField`); the
`Version` verdict accepts only this ABI (MODULE-REFCOUNT: `OVD_FWD_VERSION`
returns `abi_version` only; no permanence flag). They call the crate-private
helpers DISTILL names (`decode_end_record(&[u8; 32])`,
`version_verdict(OvdFwdVersion)`) — internal structure, the crafter's to
shape; if it differs, the cases route back to DISTILL (the crafter does not
rewrite them). Their Tier-3 counterparts are `probe_gold` and
`end_record_abi`.

## Unload safety (`tests/integration/unload.rs`, V-EV-UNLOAD, `kvm-tests`)

Native metal under its lease only (never shared Lima), serialized by
`host-kernel-shared`; each case requires `refcnt` 0 and this workspace's
build loaded, and leaves it loaded.

| Cell | Step | Oracle |
|---|---|---|
| `an_open_context_refuses_the_unload_and_its_close_lets_it_succeed` | K | context open: `refcnt` 1, `delete_module(O_NONBLOCK)` `EWOULDBLOCK`, module present; closed: `refcnt` 0, unload `0`, `/sys/module/ovd_fwd` gone; reload of this workspace's `.ko`, a new context opens (`refcnt` 1) |
| `hooked_sockets_hold_the_module_until_their_free_work_and_a_reload_then_serves` | 4 | pair `refcnt` 3, watch 4, refused at each; `ABORT_ALL` + records read → 1 within the bounded wait; a live pair at context close → 0; unload succeeds, reload, the forwarder probe passes |

The ULP negative control and the KASAN stress need a kernel boot of their
own and are not authored (upstream issues, U-EV-UNLOAD).

## Guest side of a real VM boundary (`tests/integration/guest_boundary.rs`, `kvm-tests`)

On the guest-kit microVM fixture (user ruling T-1, 2026-10-18; native
x86_64 metal, `cargo xtask metal run --`), pending step 4 (it also needs
step 1, the fork's RESET-EXIT):

- the device-release orphan sweep (`drivers/vhost/vsock.c:721-750 @ v7.0`):
  a live pair aborts `SocketError { A, ECONNRESET }` and its TCP far end
  resets when the VMM exits; a half swept before attach is refused
  `HalfEnded { A, ECONNRESET }`; a watch's record is queued before its
  holder sees `ECONNRESET`;
- the battery across the VM boundary (host pair ↔ vhost ↔ virtio-vsock ↔
  guest pair): 2 MiB each way with clean ends completing both pairs, a guest
  application reset, a host application reset, a VMM exit mid-transfer.

These are the real-adapter pairing of `SimSocket::{set_sk_err,
report_error}` (QPROJ-7). The host-role suite above stays on Lima and also
runs on the metal host's x86_64 7.0 release (`upstream-issues.md` O-7).
