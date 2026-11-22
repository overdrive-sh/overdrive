# RCA: `an_outbound_connection_names_its_pump_threads_by_role` flakes (~30% on Metal)

Test: `overdrive-dataplane::integration mtls_guardrails::an_outbound_connection_names_its_pump_threads_by_role`
(`crates/overdrive-dataplane/tests/integration/mtls_guardrails.rs`, assertion at the `mtls-out-ret` count).
Failure shapes: `left: 2, right: 1` and `left: 0, right: 1` for `mtls-out-ret`.

## Verdict

**TEST defect, two independent causes. No production defect found.** Both are in the test fixture / assertion timing.
Two minor production observations are recorded at the end (neither causes this flake).

| Branch | Cause | Where | Frequency (guest, stress run) |
|---|---|---|---|
| A | The fixture `OutboundPeer` leaves TLS 1.3 session tickets ON. A `NewSessionTicket` arrives on the agent's kTLS-RX socket; `splice` returns `EINVAL`; the return pump dies as `TransportDeath` and the reaper thread tears the connection down. | test peer, `outbound_peer_serve` | 2/20 in the first run (both showed `errno=22`) |
| B | The assertion reads `/proc/self/task/*/comm` immediately after `enforce` returns. `std::thread::Builder::name` names the thread from inside the new thread, so the freshly spawned return pump still carries its spawner's comm (`tokio-rt-worker`) for a moment. Count of `mtls-out-ret` is 0. | test assertion | 7/60, 4/60, 7/60 with tickets already off |

The earlier "ticket-off edit still failed 1/5" is explained by branch B: removing A left B untouched.

## Method

Probes ran on Metal (guest `mtls-relay`, `--stress-count N`, `-E 'test(...)'`). Diagnostics were temporary `eprintln!` in `splice.rs` and in the test peer, since reverted.
Runs: `386c497e4714b464aa728eca` (errno capture), `8eedba319f9cd787a804eb9d` (tickets off), `4b6a41605eb823385fc155e8` and `4e0dc00c631f918835d9f9a2` (peer + pump lifecycle), `d0ecf847cfc95d3e555d234f` (fix, 60/60).

## 5 Whys

### Branch A: return pump killed by a TLS ticket

- **WHY 1.** The assertion sees 0 or 2 `mtls-out-ret` threads. [Evidence: run 386c...: `left: 0` and `left: 2`; the 2-case list shows `"mtls-out-fwd", "mtls-out-ret", "mtls-out-ret"`.]
- **WHY 2.** The return pump `run_decrypt_pump` exited within ms via the `TransportDeath` splice-error arm, which fires the self-teardown reaper (`mod.rs` ~:304). The reaper is an unnamed `std::thread::spawn` from the pump thread, so it inherits the comm `mtls-out-ret` (the 2-case); once it has joined the pump, the count is 0. [Evidence: `DIAG decrypt pump splice errno=22 thread=Some("mtls-out-ret")` in both captured failures; `splice.rs` splice-error arm `break PumpExit::TransportDeath`.]
- **WHY 3.** `splice(kTLS-RX socket -> pipe)` returned `EINVAL` because the next record was not application data. `tls_sw_splice_read` returns `-EINVAL` when `tlm->control != TLS_RECORD_TYPE_DATA` ("splice does not support reading control messages", `net/tls/tls_sw.c:2310-2312`; read from a `vendors/linux` checkout of another workspace, as this worktree's submodule is not populated. I did not verify its tag is v7.0; the guest kernel is `7.0.0-38`). The only control record a healthy server sends here is the TLS 1.3 `NewSessionTicket`. [Evidence: `errno=22` captured; branch disappears-in-kind when tickets are off (no `DIAG ... splice errno` lines in 120 later runs).]
- **WHY 4.** The fixture peer's `ServerConfig` keeps rustls' default `send_tls13_tickets` (non-zero), so it sends a ticket after the handshake. Every production server config and the other test peers set `send_tls13_tickets = 0` precisely for this ("raw kTLS-RX hits EIO on a post-handshake ticket"): `tls_config.rs:337`, `outbound.rs:529`, `helpers/relay_peers.rs:253`. [Evidence: `mtls_guardrails.rs` `outbound_peer_serve` builds `ServerConfig` with no such line.]
- **WHY 5 (root).** The fixture was written as a "best-effort" loopback server without inheriting the production server-leg invariant, and nothing in the test asserts the invariant. The ticket send is a scheduling-dependent race (it appears only when the record is delivered before the test's assertion), so it surfaced as a flake rather than a deterministic failure.

### Branch B: thread comm not yet set when counted

- **WHY 1.** With tickets off and no pump exit, the `mtls-out-ret` count is still 0 in ~7-12% of runs. The thread list in those failures contains an extra `tokio-rt-worker` where the return pump should be. [Evidence: run 4b6a...: 4 failures / 60, run 8eed...: 1 / 40; all with `left: 0` and a trailing `"tokio-rt-worker"`.]
- **WHY 2.** The return pump has not yet executed the first line of its body. In every such failure the `DIAG decrypt pump start` line is printed *after* the assertion panic message; the peer log shows the handshake finished and the reply written (so no peer-side early close). [Evidence: run 4e0d...: lines `DIAG write_leading ... thread=Some("mtls-out-fwd")` and `DIAG decrypt pump start` appear after `left: 0`; in the passing path they precede it.]
- **WHY 3.** The thread's name is applied by the child after `clone`, so before that the kernel comm is the parent's (`tokio-rt-worker`). `PumpHandle::spawn_decrypt` uses `std::thread::Builder::name(...).spawn(...)` (`splice.rs:235-237`); std applies the name inside the new thread. `enforce` returns as soon as `spawn_decrypt` returns (`outbound.rs:161-166`), and the return pump is the last thing spawned, so the test thread can win the race. [Evidence: above; forward pump, spawned earlier and having just done a blocking prelude write, never races.]
- **WHY 4.** The test reads `/proc/self/task/*/comm` once, with no allowance for the spawn-to-name window. The test file's own comment already anticipates a proc-vs-join ordering caveat for teardown and says to poll if it ever fails, but the activation applied the same single-read assumption to the *start* side.
- **WHY 5 (root).** Thread names are an eventually-consistent observation (set from the child), but the test asserted them as if they were synchronous with `enforce`. The race was latent in a loaded guest and invisible on a quiet host.

### Cross-validation
- A alone explains `left: 0`/`left: 2` with `errno=22`; B alone explains `left: 0` with a live pump and a trailing `tokio-rt-worker`. They are independent and additive: first run (tickets on) 2/20 all A; tickets off 12% B.
- Prior hypothesis "pump dies on a ticket" is confirmed for branch A, but it is a fixture misconfiguration, not production (see below).
- Not found: any case with a production-relevant pump death on a healthy connection.

## Production assessment

- Outbound peers in production are Overdrive agents' leg-C servers, which set `send_tls13_tickets = 0` (`tls_config.rs:337`). A healthy production peer does not send tickets, so the return pump cannot die this way. [Reachability: mesh peers are the only outbound targets (`CLAUDE.md` mesh model).]
- Latent hardening note (no change proposed): the outbound client config does not defend against a non-Overdrive server that sends tickets; that server would kill the return pump with `EINVAL`. It is unreachable on the appliance mesh path, so per `design.md` "Correct, and no more than correct" no mechanism is proposed.
- Observation: the self-teardown reaper (`mod.rs` ~:304) is an unnamed thread, so it shows up as `mtls-out-ret` / `mtls-out-fwd` in a CPU profile during a transport death. Cosmetic, only on a death path; not the cause here. Naming it (`mtls-reaper`) would be a separate decision; I did not change it.

## Fix (minimal, test-only) and verification

File: `crates/overdrive-dataplane/tests/integration/mtls_guardrails.rs` only.
1. `outbound_peer_serve`: `let Ok(mut cfg) = ...; cfg.send_tls13_tickets = 0;` (applies to both tests that use `OutboundPeer`).
2. In the test, replace the single `live_thread_names()` read with a bounded (1 s, 5 ms step) poll until `mtls-out-fwd == 1 && mtls-out-ret == 1`, then keep the two exact-count assertions. A duplicate or missing thread still fails; only the spawn-to-name window is tolerated.

Verification (Metal, `--stress-count 60`, run `d0ecf847cfc95d3e555d234f`): **60/60 passed**. Before: 2/20 and 7/60, 4/60 (with tickets off only) failed; same host, same guest.
The fix is left in the working tree, uncommitted; diagnostics are reverted (`splice.rs` is identical to HEAD).

## Risk
- Low: test-only. The poll adds at most 1 s on failure and ~0 on success.
- Residual: the final "after teardown" assertion still reads comm once; its caveat is documented in the test and the join happens-before it, so it was not observed to fail (60/60).
- No issue created. GitHub search (`pump thread names`, `NewSessionTicket kTLS`) found no existing issue for this flake.

## Early detection
A fixture rule worth recording in review: any in-test TLS server whose peer is a kTLS-RX socket must set `send_tls13_tickets = 0` (the production invariant), and thread-name assertions must tolerate the child-side naming window.
