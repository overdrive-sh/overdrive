# RCA: gateway early-401 answered 502, and truncated 1 MiB bodies, under CPU load

Branch base: `marcus-sa/research-ingress-gateway` (286b45ebf). Substrate: native metal host, Linux 7.0.0-38 guest kit,
test `overdrive-system-conformance::public_ingress_gateway_end_to_end` in guest group `cgroup-default`.

## Problem statement

`native_metal_kvm_public_ingress_gateway_lets_the_client_read_the_response_before_a_clean_close` returns 502 instead of
401 for the early-401 case in 40-75% of iterations under CPU load. The `..._128_exchange_bursts` test fails the same way
under load (clients see `Eof` with status 200 and a short body; the load child reports `DONE 127`, `DONE 126`, ...).
The origin (`pig_guest.rs` `/early`) writes the 67-byte 401 and calls `shutdown(Write)` at once.

## Step 1 verdict: case 2 (lost response bytes), 19 of 19 failing requests; case 1 (readiness 502) 0 of 19

Temporary probes at the gateway, the agent relay, the guest origin and the guest console (12 stressed iterations
per run, 3 runs, spinner threads on every CPU):

| Run | failing early-401 | `connect_failed` (case 1) | `response_head_read_failed: Truncated` after a relayed request (case 2) | `forwarded_head_write_failed` |
|---|---|---|---|---|
| 1 | 5 / 12 | 0 | 5 | 0 |
| 2 | 9 / 12 | 0 | 9 | 0 |
| 3 | 5 / 12 | 0 | 5 | 0 |

Every failing request connected, relayed its 172-byte head (`relay client Ok(172)`), and then read EOF with `head_len=0`
from the guest stream (`bad_gateway at exchange.rs:731`). None failed at the dial. The origin was already answering:
in the failing iteration I read in full (run 1) the `/ready` probe was served 75 ms before `POST /early`, and no failing
request in any run failed at the dial, so none was in the startup window of ADR-0242 R-2.

Console evidence from one failing run (run 1, ForwardId(2)): the origin logged `early write Ok(()) shutdown Ok(())`
(436 us), the guest forwarder reported `Completed, moved_ba: 67` (all 67 response bytes accepted by the vsock), the host
relay logged `guest Ok(0) end=Some(HalfClose)` with no data before it, and the origin kept draining 1.7 MiB of request
body afterwards. The guest half therefore did its job; the bytes were lost on the host.

## 5 Whys

PROBLEM: the client receives 502 (or a short body) although the origin sent a complete response before half-closing.

WHY 1: the gateway's response half got EOF with no response head (`read_head Data(0) head_len=0`, then
`response_head_read_failed: Truncated`, `exchange.rs:731`). [Evidence: 19/19 failing runs; 0 `connect_failed`.]

WHY 2: the agent relay's `guest.read()` returned `Ok(0)` (`HalfClose`) while the guest's 67 bytes were still unread in
the host-side vsock `V_h` receive queue. [Evidence: probe in `HostGuestStream::read` that peeked `V_h` at the moment it
returned `Ok(0)` from the pre-check: `queued_peek=Ok(67)` for exactly the 5 failing iterations of run 3, `Ok(0)` for the
normal `/ready` ends; in the burst test `queued_peek=Ok(4096)` 166 times, matching the truncated 1 MiB bodies.]

WHY 3: `HostGuestStream::read` consulted the watch record before reading the socket.
`crates/overdrive-control-plane/src/guest_network/flow_io/host.rs`, top of the `read` loop:
`if let Some(end) = self.read_end() { return end; }`. `read_end()` maps a queued `GuestHalfClose`/`Completed` record to
`Ok(0)` (and `Aborted` to `ConnectionReset`). The guest's SHUTDOWN (flags 0x2) arrives right behind its data, and the
forwarder's callback queues the record as soon as the SHUTDOWN is processed. When the relay task runs late (CPU load),
the record is already resolved at the next `read`, so the pre-check pre-empts `recv`. [Evidence: the fix below removes
the failures; the probe shows the record was resolved with bytes queued.]

WHY 4: the stream treated the watch record as the end of the stream instead of as the classification of an end that
`read` has observed. The documented contract says the opposite: "Every end the stream observes on `V_h` - a `read` of 0,
any `read` error - is resolved from the watch", "`Ok(0)`: the guest direction ended with HalfClose **and every byte
before it has been read**" (`overdrive-core/src/guest_transport/dial.rs`, `GuestStreamIo`/`GuestStream`). The Quint model
encodes it: `ReadEof` requires `gh.hobs != "none"`, and `holderReadable` yields `eof` only when `dir.gh.q1` is empty
(`specs/quint/guest-flow-owner/fwd_pair.qnt`, `canReadEof`, `holderReadable`); the only pre-emption the model allows is
the owner's own end (`strmAbort`). The production `read` allowed record-based pre-emption too.

WHY 5 (root cause): the production `HostGuestStream` is not driven by any conformance driver or integration test that
has bytes and the end record both pending. The Quint `HolderEnd`/`ReadEof` drivers run against `SimSocket` (which
follows the recvmsg order, `ba1efeda7`), so the model and the Sim agree, and the one place that diverges from both - the
production read loop - is checked by nothing. The ordering only shows when the relay task is slower than the guest's
write-then-shutdown, which a fast idle host never produces. [Evidence: grep for `HostGuestStream` finds only `host.rs`;
no test in `quint_stream_conformance`/`owner_acceptance` constructs it.]

ROOT CAUSE: `HostGuestStream::read` (`host.rs`, pre-fix line ~1079) returns `Ok(0)` for a queued `GuestHalfClose` /
`Completed` watch record before draining `V_h`, so it drops the guest's last bytes. It breaks the forwarder invariant
("a byte leaves its source only after its target accepts it; a clean half-close follows the last accepted byte",
ADR-0197, `networking-without-a-host-interface-per-vm.mdx:171`) at the host end of the guest-to-host direction: the
in-VM forwarder accepted all 67 bytes and then half-closed (correct), and the host stream let the half-close overtake
the accepted bytes.

Components cleared: the guest origin (`write Ok`, `shutdown Ok`), `ovd_fwd.c` and the in-VM owner (`moved_ba: 67`,
`Completed`), vsock and vhost (the 67 bytes were present in `V_h`'s queue), the gateway (it correctly rejects a
truncated head with 502).

## Explaining the earlier observations (one bug, not two)

- No `shutdown`: no SHUTDOWN, so no record is ever queued and the pre-check never fires. Passed 6/6.
- 100 ms sleep before `shutdown`: the relay reads the 67 bytes before the SHUTDOWN exists. Passed 8/8.
- Readiness 502 (ADR-0242 R-2) is a different, decided defect and did not occur in any failing run.
- `DONE 127` burst flake: same cause. The burst test failed 3 of 3 baseline iterations under the same spinner load with
  `body read failed: Eof, status 200, partial_body` clients and 166 pre-checks with queued bytes; with the fix it passed
  2 of 2 (see below). Without load the window is rarely hit, hence the occasional single short client.

## Validation (backward chain and fix)

Prediction (before the fix): every 502 coincides with `queued_peek > 0` at the pre-check, and removing the record-based
pre-emption removes the failures. Falsification: a 502 with `queued_peek = 0`, or failures persisting after the fix.
Observed: 5 of 5 failures had `queued_peek=Ok(67)`; no failure had `queued_peek=0`.

| Run (spinner load, same host) | Before | After |
|---|---|---|
| early-401 test, `--stress-count` | 5/12, 9/12, 5/12 failed | 0/30 failed |
| 128-exchange burst test | 3/3 failed (truncated bodies) | 0/2 failed |
| `overdrive-control-plane` guest_network / quint_stream / flow_io tests | - | 277/277 passed |

Fix (`host.rs`, `HostGuestStream::read`): only the owner's own end (`StreamResolution::OwnerAbort`, a pre-existing
`Err(ConnectionAborted)`) pre-empts a read. Record-based ends are resolved by the existing post-`recv` branches (a read
of 0 or a non-`WouldBlock` error), which already call `read_end()` and wait for an in-flight record. No new API, no
change to the modelled protocol: the model already forbids the old behaviour. The test, the origin and the gateway are
unchanged.

## Existing issues

`gh issue list --state all --search` for half-close, ovd_fwd, early response, lost bytes, guest flow FIN, 502 gateway,
gateway flaky, pig_guest, shutdown write found nothing covering this. #341 item 3 (the `BACKEND_MAP.healthy` 502) is the
readiness case and is not this bug; #339 (5-byte TLS-record splice loss) is a different mechanism. No issue was created.

## Prevention (proposals, not implemented)

1. Cover the production `HostGuestStream::read` with a deterministic case where the guest's bytes and its SHUTDOWN are
   both pending before the first `read` (the model's `HolderEnd` after non-empty `q1`). Requires owner approval: it
   adds a test (testing.md, "agents propose, the user adds"). Cost: one real-vsock or seam-level test in the default or
   `integration-tests` lane; cheaper alternative: none that reaches the production read loop, since Sim stream tests
   already agree with the model.
2. The record-based pre-emption pattern exists for `write_end`/`shutdown_write` too; those paths do not return data and
   were not changed.

## Not verified

- Linux v7.0 source was fetched into a temp directory (`vendors/linux` is an unpopulated submodule) only to rule out a
  vsock-transport loss; the finding rests on the peek probe, not on a kernel claim. No kernel behaviour is asserted.
- Whether `Aborted`-record ends should also deliver already-queued bytes first is not specified by the spec text I read;
  after the fix they do (bytes, then `ConnectionReset`), and the 277 stream/owner tests pass.
- The slow-start window of ADR-0242 R-2 was ruled out per request by timestamps (`/ready` served first, 0 dial
  failures), not by correlating with the serve log's `Stable`/first-`pass` lines.
